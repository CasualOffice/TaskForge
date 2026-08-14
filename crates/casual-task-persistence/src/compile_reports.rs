/// A slice a report groups by (`docs/38` §The report model, `ADR-027`).
///
/// Closed, like everything else the filter grammar touches. A report that needs
/// a sixth part is a signal the model is wrong, not that the report is special —
/// and a user-defined group expression is exactly the unbounded query
/// `docs/38` exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Status,
    State,
    Type,
    Priority,
    Project,
    Team,
    Environment,
    Reporter,
    Milestone,
    /// Needs a join, and a task with two assignees counts in both slices.
    /// That is right for "how much is on each person" and wrong for "how many
    /// tasks are there" — which is why the answer names its dimension.
    Assignee,
}

/// The time grain of a bucketed report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interval {
    Day,
    Week,
    Month,
}

/// Which timestamp the buckets are cut on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketField {
    CreatedAt,
    UpdatedAt,
    DueAt,
}

/// A time series, when a report asks for one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    pub field: BucketField,
    pub interval: Interval,
}

impl Dimension {
    /// The grouping expression, and whether it needs the assignee join.
    ///
    /// A `NULL` group is a real answer — unassigned, untriaged, on no
    /// environment — and is returned rather than filtered out. `docs/45` makes
    /// the triage queue a place; a report that dropped it would hide the one
    /// slice a lead is looking for.
    fn expression(self) -> &'static str {
        match self {
            Self::Status => "t.status_id::text",
            Self::State => "t.state::text",
            Self::Type => "t.type::text",
            Self::Priority => "t.priority::text",
            Self::Project => "t.project_id::text",
            Self::Team => "t.team_id::text",
            Self::Environment => "t.environment_id::text",
            Self::Reporter => "t.reporter_id::text",
            Self::Milestone => "t.milestone_id::text",
            Self::Assignee => "a.user_id::text",
        }
    }

    fn needs_assignees(self) -> bool {
        matches!(self, Self::Assignee)
    }
}

impl Bucket {
    fn expression(self) -> String {
        let grain = match self.interval {
            Interval::Day => "day",
            Interval::Week => "week",
            Interval::Month => "month",
        };
        let column = match self.field {
            BucketField::CreatedAt => "t.created_at",
            BucketField::UpdatedAt => "t.updated_at",
            BucketField::DueAt => "t.due_at",
        };
        format!("date_trunc('{grain}', {column})")
    }
}

/// Compile a **validated** filter into a grouped count (`ADR-027`).
///
/// # Why this lives beside [`compile`] and not in a reporting module
///
/// The tenant predicate and the authorized project set are injected in exactly
/// one place, and this is that place. A reporting module that assembled its own
/// `WHERE` would be a second copy of the rule that keeps a report from
/// answering across a tenant boundary — and the copy that drifts is always the
/// one written second.
///
/// # Why `count` and nothing else, for now
///
/// `docs/38`'s measure set includes `cycle_time`, `lead_time` and `throughput`,
/// and all three read state occupancy — which is a projection
/// (`task_state_interval`) the outbox worker does not maintain yet. Computing
/// them by scanning `activity_event` at query time is precisely the unbounded
/// query that document exists to prevent, so they are absent rather than slow.
///
/// # Ordering
///
/// By count descending, then by the group key, so the answer is deterministic
/// under ties. Without the tiebreaker two runs of the same report can disagree
/// about which slices made the limit.
pub fn compile_group_count(
    filter: &Node,
    workspace: WorkspaceId,
    authorized: &AuthorizedProjectSet,
    group: Dimension,
    bucket: Option<Bucket>,
    limit: u32,
) -> Compiled {
    let mut params: Vec<Param> = Vec::new();
    params.push(Param::Workspace(workspace));
    params.push(Param::Projects(authorized.as_slice().to_vec()));

    let predicate = emit(filter, &mut params);
    let join = if group.needs_assignees() {
        " LEFT JOIN task_assignee a ON a.task_id = t.id"
    } else {
        ""
    };
    let group_expr = group.expression();
    let (bucket_select, bucket_group, bucket_order) = match bucket {
        None => (
            String::from("NULL::timestamptz"),
            String::new(),
            String::new(),
        ),
        Some(b) => {
            let expr = b.expression();
            (expr.clone(), format!(", {expr}"), format!(", {expr}"))
        }
    };

    let sql = format!(
        "SELECT {group_expr} AS group_key, {bucket_select} AS bucket_start, count(*) AS total \
         FROM task t{join} \
         WHERE t.workspace_id = $1 \
           AND t.project_id = ANY($2) \
           AND t.deleted_at IS NULL \
           AND ({predicate}) \
         GROUP BY {group_expr}{bucket_group} \
         ORDER BY count(*) DESC, {group_expr} NULLS LAST{bucket_order} \
         LIMIT {limit}"
    );
    Compiled { sql, params }
}

/// What a duration report reduces a set of per-task durations to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reduce {
    Avg,
    P50,
    P90,
    /// The largest, which is the only one that answers "the oldest open task".
    /// A median age says how old work usually is; the question a standup asks
    /// is what has been sitting longest, and an average hides exactly that.
    Max,
}

impl Reduce {
    /// The aggregate over a column of seconds.
    fn over(self, column: &str) -> String {
        match self {
            Self::Avg => format!("avg({column})"),
            Self::P50 => format!("percentile_cont(0.5) WITHIN GROUP (ORDER BY {column})"),
            Self::P90 => format!("percentile_cont(0.9) WITHIN GROUP (ORDER BY {column})"),
            Self::Max => format!("max({column})"),
        }
    }
}

/// Which span of a task's life a duration measure covers (`docs/38` §Measures).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    /// First entry to an `ACTIVE` state → first entry to `COMPLETED`.
    CycleTime,
    /// `created_at` → first entry to `COMPLETED`.
    LeadTime,
}

/// Compile a **validated** filter into a duration, per group (ADR-027).
///
/// # `CANCELED` is never `COMPLETED`
///
/// `docs/38` is explicit, and `docs/23` keeps the two states apart precisely so
/// this can be true: collapsing them is the most common metric bug in trackers,
/// because abandoned work is fast and makes a team look quick. Only intervals
/// whose state is `COMPLETED` end a span, so a cancelled task contributes
/// nothing rather than contributing a flattering number.
///
/// # Why the durations are computed per task first
///
/// A task can enter `ACTIVE` several times — reopened, sent back by QA — and the
/// span is from the *first* time it started to the *first* time it finished.
/// Aggregating the intervals directly would average a task's separate visits
/// instead of measuring the task, and a task that bounced twice would count
/// three times.
///
/// # Errors
///
/// None. The SQL is assembled, not executed.
pub fn compile_duration(
    filter: &Node,
    workspace: WorkspaceId,
    authorized: &AuthorizedProjectSet,
    group: Dimension,
    span: Span,
    reduce: Reduce,
    limit: u32,
) -> Compiled {
    let mut params: Vec<Param> = Vec::new();
    params.push(Param::Workspace(workspace));
    params.push(Param::Projects(authorized.as_slice().to_vec()));

    let predicate = emit(filter, &mut params);
    let join = if group.needs_assignees() {
        " LEFT JOIN task_assignee a ON a.task_id = t.id"
    } else {
        ""
    };
    let group_expr = group.expression();
    let started = match span {
        // The task's own creation is the start of a lead time, and it is on
        // `task` rather than in the projection — nothing enters a state before
        // it exists.
        Span::LeadTime => "min(t.created_at)".to_owned(),
        Span::CycleTime => "min(i.entered_at) FILTER (WHERE i.state = 'ACTIVE')".to_owned(),
    };
    let seconds = reduce.over("EXTRACT(EPOCH FROM (finished - started))");

    let sql = format!(
        "WITH per_task AS ( \
           SELECT t.id AS task_id, {group_expr} AS group_key, \
                  {started} AS started, \
                  min(i.entered_at) FILTER (WHERE i.state = 'COMPLETED') AS finished \
             FROM task t \
             JOIN task_state_interval i ON i.task_id = t.id{join} \
            WHERE t.workspace_id = $1 \
              AND t.project_id = ANY($2) \
              AND t.deleted_at IS NULL \
              AND ({predicate}) \
            GROUP BY t.id, group_key \
         ) \
         SELECT group_key, NULL::timestamptz AS bucket_start, \
                round({seconds})::bigint AS total \
           FROM per_task \
          WHERE started IS NOT NULL AND finished IS NOT NULL AND finished >= started \
          GROUP BY group_key \
          ORDER BY total DESC, group_key NULLS LAST \
          LIMIT {limit}"
    );
    Compiled { sql, params }
}

/// Compile a **validated** filter into the age of *open* work (`docs/38`).
///
/// # Why this does not touch `task_state_interval`
///
/// Age is `created_at → now`, and both are on the task. Cycle and lead time need
/// the projection because they are bounded by a *transition*; age is bounded by
/// the clock, so joining the projection would only add a way for a task with no
/// intervals yet — one created a second ago — to vanish from a measure that is
/// specifically about work sitting untouched.
///
/// # Why "open" is in the measure and not left to the filter
///
/// `docs/38` defines age as "`created_at` → now, **for open tasks**". The age of
/// a finished task is not a smaller number, it is a meaningless one: it keeps
/// growing after the work stopped. Leaving that to the caller would make the
/// measure mean different things depending on a filter someone else wrote, so
/// the completed states are excluded here. A filter that asks for completed work
/// and this measure returns nothing, which is visibly empty rather than quietly
/// wrong.
///
/// # Errors
///
/// None. The SQL is assembled, not executed.
pub fn compile_age(
    filter: &Node,
    workspace: WorkspaceId,
    authorized: &AuthorizedProjectSet,
    group: Dimension,
    reduce: Reduce,
    limit: u32,
) -> Compiled {
    let mut params: Vec<Param> = Vec::new();
    params.push(Param::Workspace(workspace));
    params.push(Param::Projects(authorized.as_slice().to_vec()));

    let predicate = emit(filter, &mut params);
    let join = if group.needs_assignees() {
        " LEFT JOIN task_assignee a ON a.task_id = t.id"
    } else {
        ""
    };
    let group_expr = group.expression();
    let seconds = reduce.over("EXTRACT(EPOCH FROM (now() - t.created_at))");

    // No `per_task` stage: there is one row per task already, unless the group
    // is by assignee — and there the LEFT JOIN is the point, because a task with
    // two assignees is old for both of them.
    let sql = format!(
        "SELECT {group_expr} AS group_key, NULL::timestamptz AS bucket_start,                 round({seconds})::bigint AS total            FROM task t{join}           WHERE t.workspace_id = $1             AND t.project_id = ANY($2)             AND t.deleted_at IS NULL             AND t.state NOT IN ('COMPLETED', 'CANCELED')             AND ({predicate})           GROUP BY group_key           ORDER BY total DESC, group_key NULLS LAST           LIMIT {limit}"
    );
    Compiled { sql, params }
}

/// Compile a **validated** filter into a count of tasks *entering* `COMPLETED`,
/// per bucket (`docs/38` §Measures — throughput).
///
/// # Why the bucket is the completion, not a column on the task
///
/// Throughput is "how much finished in that week". Bucketing on `created_at`
/// would answer "how much that was raised that week has since finished", which
/// is a different question and a worse one — it moves work into the past as it
/// completes, so last month's number changes every day.
///
/// # Errors
///
/// None. The SQL is assembled, not executed.
/// Compile a **validated** filter into how long tasks spent in one state
/// (`docs/38` §Measures — `time_in_state`).
///
/// # Why the total per task and not per visit
///
/// A task can enter a state several times — sent back by review, reopened — and
/// "how long was this in Code Review" means all of it. Reducing the intervals
/// directly would answer "how long was a typical *visit*", which flatters a task
/// that bounced five times into looking quick five times over.
///
/// # Why an open interval counts up to now
///
/// A task sitting in a state right now is exactly the one the question is
/// usually about. `exited_at` is `NULL` while it is there, so `coalesce(…, now())`
/// makes the open interval contribute its time so far — a measure that ignored
/// it would report the state's cost as zero for the work currently stuck in it.
///
/// # Why a permanent state and not a status
///
/// `docs/38` asks it of "a given state" and the projection carries both. A
/// status is named inside one project's workflow, so at workspace scope two
/// projects can hold two different statuses called "Review" and a report naming
/// one could not say which it meant. The five permanent states are closed and
/// shared, which is what makes the answer comparable across projects.
///
/// # Errors
///
/// None. The SQL is assembled, not executed.
pub fn compile_time_in_state(
    filter: &Node,
    workspace: WorkspaceId,
    authorized: &AuthorizedProjectSet,
    group: Dimension,
    state: &str,
    reduce: Reduce,
    limit: u32,
) -> Compiled {
    let mut params: Vec<Param> = Vec::new();
    params.push(Param::Workspace(workspace));
    params.push(Param::Projects(authorized.as_slice().to_vec()));

    let predicate = emit(filter, &mut params);
    let join = if group.needs_assignees() {
        " LEFT JOIN task_assignee a ON a.task_id = t.id"
    } else {
        ""
    };
    let group_expr = group.expression();
    let seconds = reduce.over("seconds");

    // The state is bound as a parameter and cast, never interpolated: it
    // reaches here from a closed set, and a cast parameter uses `tsi_cycle_ix`
    // where `i.state::text = $n` would not (the lesson from `t.state = $3`).
    params.push(Param::Text(state.to_owned()));
    let state_param = params.len();

    let sql = format!(
        "WITH per_task AS (            SELECT t.id AS task_id, {group_expr} AS group_key,                   sum(EXTRACT(EPOCH FROM (coalesce(i.exited_at, now()) - i.entered_at)))                     AS seconds              FROM task t              JOIN task_state_interval i                ON i.task_id = t.id AND i.state = ${state_param}::task_state{join}             WHERE t.workspace_id = $1               AND t.project_id = ANY($2)               AND t.deleted_at IS NULL               AND ({predicate})             GROUP BY t.id, group_key          )          SELECT group_key, NULL::timestamptz AS bucket_start,                 round({seconds})::bigint AS total            FROM per_task           WHERE seconds IS NOT NULL           GROUP BY group_key           ORDER BY total DESC, group_key NULLS LAST           LIMIT {limit}"
    );
    Compiled { sql, params }
}

/// Compile a **validated** filter into two series per bucket: work raised, and
/// work finished (`docs/38` §Measures — `created_vs_completed`).
///
/// # Why this is one query and not two
///
/// The whole message of the chart is where the lines *cross*. Two separate runs
/// would be two permission resolutions, two cache windows and two moments in
/// time, so a reader comparing them would be comparing answers to slightly
/// different questions — and the crossing point, which is the only thing anyone
/// reads this for, is exactly where that error shows.
///
/// # Why it takes no dimension
///
/// The two series *are* the grouping. `group_key` carries `created` or
/// `completed`, so a caller's `group_by` has nowhere to go — asking for
/// "created vs completed by assignee" is asking for four lines from a chart
/// that draws two. The request field stays required because every other measure
/// needs it, and the response says `"group_by": "series"` rather than echoing a
/// dimension the answer does not contain.
///
/// # Errors
///
/// None. The SQL is assembled, not executed.
pub fn compile_created_vs_completed(
    filter: &Node,
    workspace: WorkspaceId,
    authorized: &AuthorizedProjectSet,
    interval: Interval,
    limit: u32,
) -> Compiled {
    let mut params: Vec<Param> = Vec::new();
    params.push(Param::Workspace(workspace));
    params.push(Param::Projects(authorized.as_slice().to_vec()));

    let predicate = emit(filter, &mut params);
    let grain = match interval {
        Interval::Day => "day",
        Interval::Week => "week",
        Interval::Month => "month",
    };

    // `DISTINCT t.id` on the completed half for the reason throughput needs it:
    // a task that finished, reopened and finished again has two `COMPLETED`
    // intervals, and inside one bucket that is one delivery.
    //
    // `CANCELED` never counts as completed (`docs/38`), which is why the join
    // names the state rather than reading "not open".
    let sql = format!(
        "SELECT 'created' AS group_key,                 date_trunc('{grain}', t.created_at) AS bucket_start,                 count(*)::bigint AS total            FROM task t           WHERE t.workspace_id = $1 AND t.project_id = ANY($2)             AND t.deleted_at IS NULL AND ({predicate})           GROUP BY bucket_start           UNION ALL          SELECT 'completed' AS group_key,                 date_trunc('{grain}', i.entered_at) AS bucket_start,                 count(DISTINCT t.id)::bigint AS total            FROM task t            JOIN task_state_interval i              ON i.task_id = t.id AND i.state = 'COMPLETED'           WHERE t.workspace_id = $1 AND t.project_id = ANY($2)             AND t.deleted_at IS NULL AND ({predicate})           GROUP BY bucket_start           ORDER BY bucket_start, group_key           LIMIT {limit}"
    );
    Compiled { sql, params }
}

pub fn compile_throughput(
    filter: &Node,
    workspace: WorkspaceId,
    authorized: &AuthorizedProjectSet,
    group: Dimension,
    interval: Interval,
    limit: u32,
) -> Compiled {
    let mut params: Vec<Param> = Vec::new();
    params.push(Param::Workspace(workspace));
    params.push(Param::Projects(authorized.as_slice().to_vec()));

    let predicate = emit(filter, &mut params);
    let join = if group.needs_assignees() {
        " LEFT JOIN task_assignee a ON a.task_id = t.id"
    } else {
        ""
    };
    let group_expr = group.expression();
    let grain = match interval {
        Interval::Day => "day",
        Interval::Week => "week",
        Interval::Month => "month",
    };

    // `DISTINCT t.id`: a task that completed, reopened and completed again has
    // two `COMPLETED` intervals, and inside one bucket that is one delivery,
    // not two.
    let sql = format!(
        "SELECT {group_expr} AS group_key, \
                date_trunc('{grain}', i.entered_at) AS bucket_start, \
                count(DISTINCT t.id) AS total \
           FROM task t \
           JOIN task_state_interval i ON i.task_id = t.id AND i.state = 'COMPLETED'{join} \
          WHERE t.workspace_id = $1 \
            AND t.project_id = ANY($2) \
            AND t.deleted_at IS NULL \
            AND ({predicate}) \
          GROUP BY group_key, bucket_start \
          ORDER BY bucket_start, total DESC, group_key NULLS LAST \
          LIMIT {limit}"
    );
    Compiled { sql, params }
}
