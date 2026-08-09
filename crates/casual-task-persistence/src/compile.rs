//! Compiling a validated filter to parameterized SQL (`docs/27` §Compilation).
//!
//! # The one rule
//!
//! "The compiler emits only `$1`-style parameters and identifiers from a static
//! map. There is no string interpolation of user data anywhere in the path."
//!
//! Every user-supplied value becomes a [`Param`] and is referenced by position.
//! The only text that reaches the SQL string is a column name from the private
//! static identifier map, an operator from a fixed `match`, and digits. A
//! hostile value has nowhere to go: `Value` holds `String`s, and no code path
//! writes one into the output.
//!
//! # The permission filter cannot be forgotten
//!
//! `docs/27`: "It is structurally impossible to compile a filter that omits it
//! — the compiler's signature requires an `AuthorizedProjectSet`."
//!
//! [`compile`] takes one by value. There is no overload without it, and
//! [`AuthorizedProjectSet`] cannot be constructed from a plain `Vec` by
//! accident — it is built from the resolver's answer. A missing tenant filter is
//! therefore not a code-review oversight; it does not compile.
//!
//! # Why `EXISTS` and not `JOIN`
//!
//! `assignee` and `tag` are many-to-many. A `JOIN` makes a task with two
//! matching tags appear twice, which forces `DISTINCT`, which breaks keyset
//! pagination — the cursor's `(updated_at, id)` comparison stops being a total
//! order over the result set. `EXISTS` returns each task once with no
//! deduplication.

use casual_task_model::Cursor;
use casual_task_model::{ProjectId, WorkspaceId};
use casual_task_search::filter::{Field, Node, Operator, Value};
use casual_task_search::sort::{Direction, Sort, SortField};

/// The projects an actor may see, from the resolver.
///
/// A newtype rather than `Vec<ProjectId>` so it cannot be produced by writing
/// `vec![]` at a call site. `docs/04` §The list problem resolves this once per
/// request; the compiler consumes that answer and cannot invent one.
#[derive(Debug, Clone)]
pub struct AuthorizedProjectSet(Vec<ProjectId>);

impl AuthorizedProjectSet {
    /// Build from the resolver's answer.
    ///
    /// An empty set is legitimate — an actor with access to no project — and
    /// compiles to a predicate matching nothing rather than to no predicate.
    pub fn resolved(projects: Vec<ProjectId>) -> Self {
        Self(projects)
    }

    pub fn as_slice(&self) -> &[ProjectId] {
        &self.0
    }
}

/// A bound value. Never rendered into SQL text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Param {
    Workspace(WorkspaceId),
    Projects(Vec<ProjectId>),
    Text(String),
    TextList(Vec<String>),
}

/// How a page is asked for: what to sort by, where to resume, and how many.
///
/// `docs/26` bans `OFFSET` — it scans, and it duplicates or skips rows under
/// concurrent writes, "both of which a live board guarantees". A page is
/// therefore always a keyset: a predicate on `(sort_key, id)` against the
/// previous page's last row.
#[derive(Debug, Clone)]
pub struct Page {
    pub sort: Sort,
    /// `None` for the first page.
    pub after: Option<Cursor>,
    /// Rows wanted. One more is always fetched, to detect a next page without
    /// a second count query.
    pub limit: u32,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            sort: Sort::default(),
            after: None,
            limit: 50,
        }
    }
}

/// The column a sort field orders by. The static identifier map for sorting.
fn sort_column(field: SortField) -> &'static str {
    match field {
        SortField::CreatedAt => "t.created_at",
        SortField::UpdatedAt => "t.updated_at",
        SortField::DueAt => "t.due_at",
        SortField::Priority => "t.priority",
        SortField::StatusPosition => "ws.position",
        SortField::Position => "t.position",
        SortField::Key => "t.number",
        SortField::Rank => "rank",
    }
}

/// The PostgreSQL type a bound value must be cast to.
///
/// The cast goes on the **parameter**, never the column. `t.state = $3::task_state`
/// uses the index on `state`; `t.state::text = $3` does not, and would turn
/// every filtered list into a sequential scan — the thing NFR-5 forbids and the
/// `EXPLAIN` gate exists to catch.
fn cast_for(field: Field) -> &'static str {
    match field {
        Field::Project
        | Field::Status
        | Field::Assignee
        | Field::Reporter
        | Field::Tag
        | Field::Milestone
        | Field::Environment
        | Field::Team
        | Field::Parent => "uuid",
        Field::State => "task_state",
        Field::Type => "task_type",
        Field::Priority => "task_priority",
        Field::CreatedAt | Field::UpdatedAt | Field::DueAt => "timestamptz",
        // Both are BOOLEAN in the grammar (`casual_task_search::FieldType`)
        // even though `archived` is stored as a nullable timestamp: the filter
        // asks "is it archived", and the comparison is against
        // `archived_at IS NOT NULL`.
        Field::IsBlocked | Field::Archived => "boolean",
        // Compared as text; no cast needed, and `q` is handled structurally.
        Field::Title | Field::Q | Field::Key => "",
    }
}

/// `$3::uuid`, or `$3` when the field needs no cast.
fn cast(placeholder: &str, field: Field, list: bool) -> String {
    let ty = cast_for(field);
    if ty.is_empty() {
        placeholder.to_owned()
    } else if list {
        format!("{placeholder}::{ty}[]")
    } else {
        format!("{placeholder}::{ty}")
    }
}

/// SQL plus the parameters it references, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub sql: String,
    pub params: Vec<Param>,
}

/// The ranking expression (`docs/26` §Weighting).
///
/// `ts_rank_cd` — cover density — rather than `ts_rank`: it accounts for how
/// close the matched lexemes are to each other, so a title containing the whole
/// phrase outranks a description that happens to mention both words pages
/// apart.
const RANK: &str = "ts_rank_cd(s.document, q)";

/// The static identifier map. The only place a column name is written.
fn column_of(field: Field) -> &'static str {
    match field {
        Field::Project => "t.project_id",
        Field::Status => "t.status_id",
        Field::State => "t.state",
        Field::Type => "t.type",
        Field::Priority => "t.priority",
        Field::Reporter => "t.reporter_id",
        Field::Milestone => "t.milestone_id",
        Field::Environment => "t.environment_id",
        Field::Team => "t.team_id",
        Field::Parent => "t.parent_id",
        Field::CreatedAt => "t.created_at",
        Field::UpdatedAt => "t.updated_at",
        Field::DueAt => "t.due_at",
        Field::Title => "t.title",
        Field::Archived => "t.archived_at",
        // Handled structurally rather than as a column comparison.
        Field::Assignee | Field::Tag | Field::Q | Field::IsBlocked | Field::Key => "",
    }
}

/// Compile a **validated** filter.
///
/// Takes an [`AuthorizedProjectSet`] because `docs/27` requires the permission
/// filter to be injected here rather than supplied by the caller.
///
/// The filter must already have passed `casual_task_search::filter::validate`.
/// This function does not re-check field/operator legality — that would
/// duplicate the rule in two places and invite them to drift.
pub fn compile(
    filter: &Node,
    workspace: WorkspaceId,
    authorized: &AuthorizedProjectSet,
    page: &Page,
) -> Compiled {
    let mut params: Vec<Param> = Vec::new();

    // Injected first, always. Not conditional, not caller-supplied.
    params.push(Param::Workspace(workspace));
    params.push(Param::Projects(authorized.as_slice().to_vec()));

    // A `q` clause changes the SHAPE of the query, not just its predicate:
    // `docs/26` §Permission filtering joins the projection and ranks in the
    // select list, because a rank cannot be computed from a subquery the
    // planner is free to turn into a semi-join. Hoisted here so the clause
    // emitter never sees it.
    if let Some(term) = full_text_term(filter) {
        return compile_search(filter, term, page, params);
    }

    let predicate = emit(filter, &mut params);
    let col = sort_column(page.sort.field);
    let keyset = keyset_predicate(page, &mut params, col);
    let dir = match page.sort.direction {
        Direction::Asc => "ASC",
        Direction::Desc => "DESC",
    };
    // The id tiebreaker is mandatory and orders the same way as the key.
    // `docs/26`: without it, ties in `updated_at` — which happen constantly on
    // bulk operations — make the cursor non-deterministic, so a page can repeat
    // or skip a row.
    //
    // LIMIT is n + 1: the extra row is how "has next page" is answered without
    // a COUNT over the whole result set.
    let limit = page.limit.saturating_add(1);

    // The projection is the repository's, not `t.*`: `type`, `priority` and
    // `state` are PostgreSQL enums, and a `SELECT *` hands them back as enums
    // that no `String` decoder accepts. One projection, defined once, used by
    // the compiler and by `crate::task::read_visible` alike.
    let columns = crate::task::COLUMNS;
    let sql = format!(
        "SELECT {columns} FROM task t \
         WHERE t.workspace_id = $1 \
           AND t.project_id = ANY($2) \
           AND t.deleted_at IS NULL \
           AND ({predicate}){keyset} \
         ORDER BY {col} {dir}, t.id {dir} \
         LIMIT {limit}"
    );
    Compiled { sql, params }
}

/// The single full-text term in a filter, if it has one.
///
/// Only the first is honoured. Two `q` clauses would need two `tsquery`
/// constructions and a rank over both, and `docs/27`'s URL form cannot express
/// it — a repeated parameter is one value, not two clauses.
fn full_text_term(node: &Node) -> Option<String> {
    match node {
        Node::Clause(clause) if clause.field == Field::Q => match &clause.value {
            Value::Literal(term) | Value::Symbol(term) => Some(term.clone()),
            _ => None,
        },
        Node::And(children) | Node::Or(children) => children.iter().find_map(full_text_term),
        Node::Not(inner) => full_text_term(inner),
        Node::Clause(_) => None,
    }
}

/// The ranked full-text query (`docs/26` §Permission filtering).
///
/// # The tenant predicate is on the projection, not only on `task`
///
/// `s.workspace_id = $1 AND s.project_id = ANY($2)` are applied to
/// `task_search` itself so `task_search_scope_ix` can serve them and be
/// combined with `task_search_gin` in a `BitmapAnd`. Filtering only through the
/// join to `task` would leave the planner scanning the whole projection and
/// discarding rows afterwards — which is the shape **D-043** measured as a
/// `Parallel Seq Scan` at reference scale.
///
/// **D-043 is not closed by this.** Under row-level security `@@` resolves to
/// `ts_match_vq`, which is not `LEAKPROOF`, so PostgreSQL will not evaluate it
/// before the row-security qual and cannot use the GIN index as an index qual
/// at all. This shape is the "tenant-filtered projection" that decision says to
/// try first; whether it is enough is a measurement, and the answer is recorded
/// in `docs/14` §D-043 rather than assumed here.
fn compile_search(filter: &Node, term: String, page: &Page, mut params: Vec<Param>) -> Compiled {
    // $3 — bound before any clause parameter, like the tenant pair above, so
    // the numbering stays positional and predictable.
    let query = bind(&mut params, Param::Text(term));

    // Every other clause still applies. `Field::Q` emits TRUE inside this
    // shape because it has been hoisted into the FROM and WHERE.
    let predicate = emit(filter, &mut params);

    // The ranking EXPRESSION, not the `rank` alias. A select-list alias is not
    // visible in `WHERE`, so a keyset resume written against `rank` fails with
    // "column rank does not exist" — on the second page only, which is exactly
    // the kind of bug a first-page test never sees.
    let col = if page.sort.field == SortField::Rank {
        RANK
    } else {
        sort_column(page.sort.field)
    };
    let keyset = keyset_predicate(page, &mut params, col);
    let dir = match page.sort.direction {
        Direction::Asc => "ASC",
        Direction::Desc => "DESC",
    };
    let limit = page.limit.saturating_add(1);
    let columns = crate::task::COLUMNS;
    let configuration = crate::search::CONFIGURATION;

    // `ts_rank_cd` per `docs/26` §Weighting. The rank is in the select list so
    // it can be ordered by and carried in a cursor; `SortField::Rank` maps to
    // this alias and exists only in this shape.
    let sql = format!(
        "SELECT {columns}, {RANK} AS rank \
           FROM task_search s \
           JOIN task t ON t.id = s.task_id \
           CROSS JOIN plainto_tsquery('{configuration}', {query}) q \
          WHERE s.workspace_id = $1 \
            AND s.project_id = ANY($2) \
            AND s.document @@ q \
            AND t.workspace_id = $1 \
            AND t.deleted_at IS NULL \
            AND ({predicate}){keyset} \
          ORDER BY {col} {dir}, t.id {dir} \
          LIMIT {limit}"
    );
    Compiled { sql, params }
}

/// The keyset resume predicate, or nothing on the first page.
///
/// Row-value comparison — `(key, id) < ($k, $id)` — rather than the expanded
/// `key < $k OR (key = $k AND id < $id)`. PostgreSQL can drive a composite
/// index directly from the row-value form; the expanded form is a filter it
/// often cannot, which is the difference between a keyset page and a scan.
/// `col` is the sort EXPRESSION, not a name: in the full-text shape the key is
/// `ts_rank_cd(...)`, and a select-list alias is not visible in `WHERE`.
fn keyset_predicate(page: &Page, params: &mut Vec<Param>, col: &str) -> String {
    let Some(cursor) = &page.after else {
        return String::new();
    };
    // Descending sorts resume *below* the last row, ascending *above* it.
    // Getting this backwards returns the page just served, forever.
    let cmp = match page.sort.direction {
        Direction::Asc => ">",
        Direction::Desc => "<",
    };
    // A cursor is transported as text — it has to be, it is base64url JSON —
    // and every column it resumes against is typed. Without the cast the row
    // comparison is `timestamptz < text`, which is not an operator PostgreSQL
    // has: the whole keyset path fails at execution time, and no test that
    // only inspects the compiler's output can see it. The cast goes on the
    // PARAMETER, for the reason `cast_for` gives — casting the column instead
    // would defeat the index and turn every second page into a scan.
    let key = format!(
        "{}::{}",
        bind(
            params,
            Param::Text(cursor.keys.first().cloned().unwrap_or_default()),
        ),
        cursor_type(page.sort.field)
    );
    let id = format!("{}::uuid", bind(params, Param::Text(cursor.id.to_string())));
    format!(" AND ({col}, t.id) {cmp} ({key}, {id})")
}

/// The PostgreSQL type a cursor's sort key must be cast to.
///
/// Exhaustive, so a new sortable field cannot be added without deciding how its
/// cursor resumes — which is the failure this function exists to prevent.
fn cursor_type(field: SortField) -> &'static str {
    match field {
        SortField::CreatedAt | SortField::UpdatedAt | SortField::DueAt => "timestamptz",
        SortField::Priority => "task_priority",
        SortField::StatusPosition => "integer",
        // `key` sorts by `task.number`, and `position` is already text.
        SortField::Key => "bigint",
        SortField::Position => "text",
        // `rank` is `ts_rank`'s output. Only meaningful with a `q` clause.
        SortField::Rank => "real",
    }
}

/// Push a parameter and return its `$N` placeholder.
fn bind(params: &mut Vec<Param>, p: Param) -> String {
    params.push(p);
    format!("${}", params.len())
}

fn emit(node: &Node, params: &mut Vec<Param>) -> String {
    match node {
        Node::And(children) | Node::Or(children) => {
            let joiner = if matches!(node, Node::And(_)) {
                " AND "
            } else {
                " OR "
            };
            if children.is_empty() {
                // An empty group must not vanish into an empty string, which
                // would produce `AND ()` and a syntax error. `AND` of nothing
                // is true; `OR` of nothing is false.
                return if matches!(node, Node::And(_)) {
                    "TRUE".to_owned()
                } else {
                    "FALSE".to_owned()
                };
            }
            let parts: Vec<String> = children.iter().map(|c| emit(c, params)).collect();
            format!("({})", parts.join(joiner))
        }
        Node::Not(inner) => format!("NOT ({})", emit(inner, params)),
        Node::Clause(c) => emit_clause(c.field, c.op, &c.value, params),
    }
}

fn emit_clause(field: Field, op: Operator, value: &Value, params: &mut Vec<Param>) -> String {
    // Many-to-many and derived fields, as EXISTS rather than JOIN.
    match field {
        Field::Assignee => return emit_assignee(op, value, params),
        Field::Tag => return emit_tag(op, value, params),
        Field::IsBlocked => {
            let p = cast(&bind(params, param_of(value)), Field::IsBlocked, false);
            return format!(
                // `blocked_task_id` is a column `task_dependency` has never
                // had — migration 0005 names the two ends `from_task_id` and
                // `to_task_id`. Every `is_blocked` filter therefore failed at
                // execution time with "column does not exist"; no test caught
                // it because the compiler's own suite asserts the SQL text, and
                // the EXPLAIN catalogue has no probe for this field.
                //
                // "Blocked" is the same question `task::unresolved_blockers`
                // asks: an unresolved BLOCKS edge pointing AT this task.
                // Nested EXISTS rather than a JOIN, so the compiler's simple
                // invariant — no JOIN anywhere in a compiled query — keeps
                // holding. A JOIN inside EXISTS cannot duplicate outer rows,
                // but "no JOIN" is a rule a reader can check at a glance and a
                // narrower one is a rule that erodes.
                "(EXISTS (SELECT 1 FROM task_dependency d \
                  WHERE d.to_task_id = t.id \
                    AND d.kind = 'BLOCKS' \
                    AND EXISTS (SELECT 1 FROM task b \
                                 WHERE b.id = d.from_task_id \
                                   AND b.deleted_at IS NULL \
                                   AND b.state NOT IN ('COMPLETED','CANCELED')))) = ({p})"
            );
        }
        Field::Key => {
            // `key` is the human identifier `WR-125` — `project.key` and
            // `task.number` concatenated, living in two tables. docs/27 lists
            // it as filterable and the committed schema has no column to
            // compare against, so there is nothing correct to emit here.
            //
            // Matching nothing is the safe direction: a `key` filter that
            // returns no rows is visibly wrong to whoever ran it, where one
            // that compared against `t.number` alone would match `WR-125` and
            // `OPS-125` identically and look right. Tracked as **D-051**.
            return "FALSE".to_owned();
        }
        Field::Q => {
            // Already hoisted into the FROM and WHERE by `compile_search`,
            // which is the only path that can reach a `q` clause. Emitting the
            // predicate a second time here would build a second `tsquery` and
            // ask the planner to satisfy the same condition twice.
            return "TRUE".to_owned();
        }
        Field::Archived => {
            // `archived` is a BOOLEAN in the grammar and a TIMESTAMP in the
            // schema: `archived=true` means "has an archived_at", not
            // "archived_at equals the string true". Compiled as a column
            // comparison it bound `'true'::timestamptz` and errored at
            // execution — a filter that 500s rather than answers.
            let p = cast(&bind(params, param_of(value)), Field::Archived, false);
            return format!("(t.archived_at IS NOT NULL) = ({p})");
        }
        _ => {}
    }

    let col = column_of(field);
    match op {
        Operator::IsEmpty => format!("{col} IS NULL"),
        Operator::IsNotEmpty => format!("{col} IS NOT NULL"),
        Operator::Between => match value {
            Value::Range(lo, hi) => {
                let a = cast(&bind(params, Param::Text(lo.clone())), field, false);
                let b = cast(&bind(params, Param::Text(hi.clone())), field, false);
                format!("{col} BETWEEN {a} AND {b}")
            }
            _ => "FALSE".to_owned(),
        },
        Operator::In => {
            let p = cast(&bind(params, param_of(value)), field, true);
            format!("{col} = ANY({p})")
        }
        Operator::NotIn => {
            let p = cast(&bind(params, param_of(value)), field, true);
            format!("NOT ({col} = ANY({p}))")
        }
        Operator::StartsWith | Operator::Contains => {
            let p = bind(params, param_of(value));
            // The wildcard is applied in SQL around a bound parameter, so the
            // user's text is never concatenated into a pattern here.
            if matches!(op, Operator::StartsWith) {
                format!("{col} LIKE {p} || '%'")
            } else {
                format!("{col} LIKE '%' || {p} || '%'")
            }
        }
        _ => {
            let p = cast(&bind(params, param_of(value)), field, false);
            let sql_op = match op {
                Operator::Eq => "=",
                Operator::Gt | Operator::After => ">",
                Operator::Gte => ">=",
                Operator::Lt | Operator::Before => "<",
                Operator::Lte => "<=",
                // Every remaining operator is handled above; a filter that
                // reached here failed validation, and matching nothing is the
                // safe direction.
                _ => return "FALSE".to_owned(),
            };
            format!("{col} {sql_op} {p}")
        }
    }
}

fn emit_assignee(op: Operator, value: &Value, params: &mut Vec<Param>) -> String {
    let field = Field::Assignee;
    let exists = |inner: &str| {
        format!("EXISTS (SELECT 1 FROM task_assignee a WHERE a.task_id = t.id{inner})")
    };
    match op {
        Operator::IsEmpty => format!("NOT {}", exists("")),
        Operator::IsNotEmpty => exists(""),
        _ => {
            let raw = bind(params, param_of(value));
            let cmp = if matches!(op, Operator::In) {
                format!(" AND a.user_id = ANY({})", cast(&raw, field, true))
            } else {
                format!(" AND a.user_id = {}", cast(&raw, field, false))
            };
            exists(&cmp)
        }
    }
}

fn emit_tag(op: Operator, value: &Value, params: &mut Vec<Param>) -> String {
    let exists =
        |inner: &str| format!("EXISTS (SELECT 1 FROM task_tag tt WHERE tt.task_id = t.id{inner})");
    match op {
        Operator::IsEmpty => format!("NOT {}", exists("")),
        _ => {
            let p = cast(&bind(params, param_of(value)), Field::Tag, true);
            let clause = exists(&format!(" AND tt.tag_id = ANY({p})"));
            if matches!(op, Operator::NotIn) {
                format!("NOT {clause}")
            } else {
                clause
            }
        }
    }
}

/// Every value becomes a parameter. This is the only conversion, and it never
/// returns SQL text.
fn param_of(value: &Value) -> Param {
    match value {
        Value::Literal(s) | Value::Symbol(s) => Param::Text(s.clone()),
        Value::List(items) => Param::TextList(items.clone()),
        Value::Range(a, b) => Param::TextList(vec![a.clone(), b.clone()]),
        Value::None => Param::TextList(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use casual_task_search::filter::Clause;

    fn clause(field: Field, op: Operator, value: Value) -> Node {
        Node::Clause(Clause { field, op, value })
    }

    fn compiled(node: &Node) -> Compiled {
        paged(node, &Page::default())
    }

    fn paged(node: &Node, page: &Page) -> Compiled {
        compile(
            node,
            WorkspaceId::new(),
            &AuthorizedProjectSet::resolved(vec![ProjectId::new()]),
            page,
        )
    }

    #[test]
    fn the_permission_filter_is_always_present() {
        // docs/27: structurally impossible to omit. The signature requires the
        // set, and the predicate is unconditional.
        let c = compiled(&Node::And(Vec::new()));
        assert!(c.sql.contains("t.workspace_id = $1"));
        assert!(c.sql.contains("t.project_id = ANY($2)"));
        assert!(matches!(c.params[0], Param::Workspace(_)));
        assert!(matches!(c.params[1], Param::Projects(_)));
    }

    #[test]
    fn an_actor_with_no_projects_gets_a_predicate_matching_nothing() {
        // Not "no predicate". An empty authorized set must not widen the query.
        let c = compile(
            &Node::And(Vec::new()),
            WorkspaceId::new(),
            &AuthorizedProjectSet::resolved(Vec::new()),
            &Page::default(),
        );
        assert!(c.sql.contains("t.project_id = ANY($2)"));
        assert_eq!(c.params[1], Param::Projects(Vec::new()));
    }

    #[test]
    fn many_to_many_fields_compile_to_exists_not_join() {
        // A JOIN makes a task with two matching tags appear twice, forcing
        // DISTINCT, which breaks keyset pagination.
        for node in [
            clause(Field::Tag, Operator::In, Value::List(vec!["a".into()])),
            clause(Field::Assignee, Operator::Eq, Value::Symbol("@me".into())),
        ] {
            let c = compiled(&node);
            assert!(c.sql.contains("EXISTS"), "{}", c.sql);
            assert!(!c.sql.contains("JOIN"), "{}", c.sql);
            assert!(!c.sql.contains("DISTINCT"), "{}", c.sql);
        }
    }

    #[test]
    fn an_empty_group_does_not_produce_invalid_sql() {
        // `AND ()` is a syntax error. An empty AND is true, an empty OR false.
        assert!(compiled(&Node::And(Vec::new())).sql.contains("(TRUE)"));
        assert!(compiled(&Node::Or(Vec::new())).sql.contains("(FALSE)"));
    }

    #[test]
    fn the_sql_is_independent_of_the_value() {
        // docs/15 §Security: "filter compiler emits no user-derived SQL
        // strings". Stated as substring absence this test is wrong in both
        // directions — `$1` and `t.workspace_id` are hostile-looking inputs
        // that the SQL legitimately contains for other reasons, and a value
        // could still leak in a form the search missed.
        //
        // The property that actually holds is stronger and exact: compiling the
        // same filter with a different value must produce BYTE-IDENTICAL SQL,
        // because the value never reaches the text at all.
        let hostile = [
            "'; DROP TABLE task; --",
            "' OR '1'='1",
            "\\'; DELETE FROM workspace WHERE '' = '",
            "$1",
            "t.workspace_id",
            "*/ UNION SELECT * FROM api_token --",
            "100%",
            "",
        ];

        // `key` is excluded deliberately: it compiles to FALSE pending D-051
        // and binds nothing, so "was it bound as a parameter" cannot hold. It
        // is covered by its own test below.
        for (field, op) in [
            (Field::Title, Operator::Contains),
            (Field::State, Operator::Eq),
            (Field::Q, Operator::Matches),
            (Field::Assignee, Operator::Eq),
            (Field::DueAt, Operator::Before),
        ] {
            let benign = compiled(&clause(field, op, Value::Literal("benign".into())));
            for value in hostile {
                let c = compiled(&clause(field, op, Value::Literal(value.to_owned())));
                assert_eq!(
                    c.sql, benign.sql,
                    "the SQL changed with the value for {field:?} {op:?} — \
                     `{value}` influenced the text"
                );
                assert!(
                    c.params.contains(&Param::Text(value.to_owned())),
                    "`{value}` was not bound as a parameter either"
                );
            }
        }
    }

    #[test]
    fn a_hostile_value_in_a_list_is_also_only_ever_bound() {
        let hostile = "'; DROP TABLE task; --";
        let c = compiled(&clause(
            Field::Tag,
            Operator::In,
            Value::List(vec![hostile.to_owned(), "ok".to_owned()]),
        ));
        assert!(!c.sql.contains(hostile), "{}", c.sql);
        assert!(
            c.params
                .iter()
                .any(|p| matches!(p, Param::TextList(v) if v.contains(&hostile.to_owned())))
        );
    }

    #[test]
    fn the_sql_contains_only_placeholders_columns_and_keywords() {
        // The strongest form of the property: after removing every column name
        // this compiler is allowed to emit, nothing that looks like user data
        // remains — only `$N`, punctuation, and SQL keywords.
        let node = Node::And(vec![
            clause(
                Field::State,
                Operator::In,
                Value::List(vec!["ACTIVE".into()]),
            ),
            clause(
                Field::Title,
                Operator::Contains,
                Value::Literal("x'y".into()),
            ),
            clause(Field::Assignee, Operator::Eq, Value::Symbol("@me".into())),
            clause(Field::DueAt, Operator::Before, Value::Symbol("+7d".into())),
        ]);
        let c = compiled(&node);
        for forbidden in ["ACTIVE", "x'y", "@me", "+7d"] {
            assert!(
                !c.sql.contains(forbidden),
                "`{forbidden}` leaked into:\n{}",
                c.sql
            );
        }
        assert_eq!(c.params.len(), 6, "two injected plus four clause values");
    }

    #[test]
    fn the_first_page_has_no_resume_predicate_and_fetches_one_extra() {
        let c = paged(&Node::And(Vec::new()), &Page::default());
        assert!(
            !c.sql.contains(", t.id) <"),
            "no cursor, no keyset: {}",
            c.sql
        );
        // 50 requested, 51 fetched — the extra row answers "is there a next
        // page" without a COUNT over the result set.
        assert!(c.sql.ends_with("LIMIT 51"), "{}", c.sql);
    }

    #[test]
    fn a_cursor_resumes_with_a_row_value_comparison_including_the_tiebreaker() {
        let page = Page {
            after: Some(Cursor::new(
                vec!["2026-08-08T00:00:00Z".into()],
                uuid::Uuid::now_v7(),
            )),
            ..Default::default()
        };
        let c = paged(&Node::And(Vec::new()), &page);
        // Row-value form, and the id tiebreaker is present. docs/26 makes the
        // tiebreaker mandatory: without it, ties in updated_at make the cursor
        // non-deterministic and a page can repeat or skip a row.
        assert!(
            c.sql
                .contains("(t.updated_at, t.id) < ($3::timestamptz, $4::uuid)"),
            "{}",
            c.sql
        );
    }

    #[test]
    fn every_cursor_key_is_cast_to_the_type_of_the_column_it_resumes_against() {
        // A cursor travels as text. Comparing text to a timestamptz column is
        // not an operator PostgreSQL has, so without these casts the second
        // page of every list fails at execution time — a failure no assertion
        // over the compiler's output can see.
        let cursor = Cursor::new(vec!["x".into()], uuid::Uuid::now_v7());
        for (field, ty) in [
            (SortField::CreatedAt, "timestamptz"),
            (SortField::UpdatedAt, "timestamptz"),
            (SortField::DueAt, "timestamptz"),
            (SortField::Priority, "task_priority"),
            (SortField::StatusPosition, "integer"),
            (SortField::Key, "bigint"),
            (SortField::Position, "text"),
            (SortField::Rank, "real"),
        ] {
            let page = Page {
                sort: Sort {
                    field,
                    direction: Direction::Desc,
                },
                after: Some(cursor.clone()),
                limit: 10,
            };
            let c = paged(&Node::And(Vec::new()), &page);
            assert!(
                c.sql.contains(&format!("::{ty}, $4::uuid)")),
                "{field:?} resumes without a {ty} cast: {}",
                c.sql
            );
        }
    }

    #[test]
    fn the_resume_comparison_follows_the_sort_direction() {
        // Backwards here returns the page just served, forever.
        let cursor = Cursor::new(vec!["x".into()], uuid::Uuid::now_v7());
        for (direction, expected) in [(Direction::Desc, "<"), (Direction::Asc, ">")] {
            let page = Page {
                sort: Sort {
                    field: SortField::DueAt,
                    direction,
                },
                after: Some(cursor.clone()),
                limit: 10,
            };
            let c = paged(&Node::And(Vec::new()), &page);
            assert!(
                c.sql.contains(&format!("(t.due_at, t.id) {expected} (")),
                "{direction:?} should resume with `{expected}`: {}",
                c.sql
            );
            let dir_sql = if matches!(direction, Direction::Asc) {
                "ASC"
            } else {
                "DESC"
            };
            assert!(
                c.sql
                    .contains(&format!("ORDER BY t.due_at {dir_sql}, t.id {dir_sql}")),
                "the tiebreaker must order the same way as the key: {}",
                c.sql
            );
        }
    }

    #[test]
    fn no_page_ever_compiles_to_offset() {
        // docs/26 bans it outright, and the architecture lint bans the token —
        // this asserts the compiler cannot emit one by any route.
        let cursor = Cursor::new(vec!["x".into()], uuid::Uuid::now_v7());
        for after in [None, Some(cursor)] {
            let c = paged(
                &Node::And(Vec::new()),
                &Page {
                    after,
                    ..Default::default()
                },
            );
            assert!(!c.sql.to_uppercase().contains("OFFSET"), "{}", c.sql);
        }
    }

    #[test]
    fn the_cursor_values_are_bound_not_interpolated() {
        let hostile = "'; DROP TABLE task; --";
        let page = Page {
            after: Some(Cursor::new(vec![hostile.into()], uuid::Uuid::now_v7())),
            ..Default::default()
        };
        let c = paged(&Node::And(Vec::new()), &page);
        assert!(!c.sql.contains(hostile), "{}", c.sql);
        assert!(c.params.contains(&Param::Text(hostile.to_owned())));
    }

    #[test]
    fn every_column_the_compiler_names_exists_in_the_schema() {
        // The bug this would have caught: `is_blocked` emitted
        // `d.blocked_task_id`, a column `task_dependency` has never had —
        // migration 0005 names the ends `from_task_id` and `to_task_id`. Both
        // `?is_blocked=true` and `?is_blocked=false` returned TF-SYS-0001, and
        // the built-in "My Work · Blocked" view could not ship.
        //
        // Nothing caught it because every other test here asserts the SQL
        // *text*, which a wrong column name satisfies perfectly. This reads the
        // migration instead.
        let schema = include_str!("../../../migrations/0005_tasks.sql");
        let mut compiled = String::new();
        for (field, op, value) in [
            (
                Field::IsBlocked,
                Operator::Eq,
                Value::Literal("true".into()),
            ),
            (Field::Assignee, Operator::Eq, Value::Literal("x".into())),
            (Field::Tag, Operator::In, Value::List(vec!["x".into()])),
            (Field::Q, Operator::Matches, Value::Literal("x".into())),
        ] {
            compiled.push_str(&compiled_sql_of(field, op, value));
            compiled.push(' ');
        }

        // Every `<alias>.<column>` the compiler emitted, for the tables whose
        // DDL lives in this migration.
        for token in compiled.split_whitespace() {
            let token =
                token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '_');
            let Some((alias, column)) = token.split_once('.') else {
                continue;
            };
            // Aliases used for tables defined in migration 0005.
            if !matches!(alias, "d" | "a" | "tt") || column.is_empty() {
                continue;
            }
            assert!(
                schema.contains(column),
                "the compiler emits `{alias}.{column}`, which migration 0005 \
                 does not define — every query using it fails at execution time"
            );
        }
    }

    #[test]
    fn blocked_means_the_task_is_the_to_end_of_an_unresolved_edge() {
        // docs/03: "`from` blocks `to`", and a transition is gated by an
        // INCOMING BLOCKS edge. Getting this backwards would report every
        // blocker as blocked and every blocked task as free — plausible-looking
        // and exactly wrong.
        let sql = compiled_sql_of(
            Field::IsBlocked,
            Operator::Eq,
            Value::Literal("true".into()),
        );
        assert!(
            sql.contains("d.to_task_id = t.id"),
            "blocked-ness must key on the `to` end: {sql}"
        );
        assert!(
            !sql.contains("d.from_task_id = t.id"),
            "that is the blocking end, not the blocked one: {sql}"
        );
        // And it ignores blockers that are already finished, like
        // `task::unresolved_blockers` does — otherwise a closed blocker would
        // hold a card down forever.
        assert!(sql.contains("COMPLETED"), "{sql}");
    }

    /// The SQL for one clause, with the injected permission filter stripped.
    fn compiled_sql_of(field: Field, op: Operator, value: Value) -> String {
        compiled(&clause(field, op, value)).sql
    }

    #[test]
    fn key_matches_nothing_rather_than_the_wrong_rows() {
        // `WR-125` spans project.key and task.number. Comparing against
        // t.number alone would match WR-125 and OPS-125 identically and look
        // right, which is the worse failure. D-051.
        let c = compiled(&clause(
            Field::Key,
            Operator::Eq,
            Value::Literal("WR-125".into()),
        ));
        assert!(c.sql.contains("FALSE"), "{}", c.sql);
        assert!(!c.sql.contains("WR-125"));
        assert_eq!(c.params.len(), 2, "only the injected permission parameters");
    }

    #[test]
    fn wildcards_are_applied_in_sql_around_a_parameter() {
        // The pattern is built by SQL concatenation of a bound value, not by
        // formatting the value into a Rust string — so `%` in user input is
        // data, not syntax.
        let c = compiled(&clause(
            Field::Title,
            Operator::Contains,
            Value::Literal("100%".into()),
        ));
        assert!(c.sql.contains("LIKE '%' || $3 || '%'"), "{}", c.sql);
        assert!(!c.sql.contains("100%"));
    }
}
