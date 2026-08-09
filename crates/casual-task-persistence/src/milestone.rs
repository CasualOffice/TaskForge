//! Milestones — the `milestone` table (migration 0004) and its progress count.
//!
//! # The failure this module prevents
//!
//! A rollup that is mistaken for a rule. `docs/03` settles the same question for
//! subtasks — "Rollup is displayed (`3/5 done`), never enforced" — and a
//! milestone is the same shape of thing one level up. So the only thing here
//! that touches a task is [`progress`], which **counts** and never writes: there
//! is deliberately no function in this module that closes a milestone's tasks,
//! and closing a milestone is a write to one row of `milestone` and to nothing
//! else.
//!
//! The absence is the design. A repository that offered `complete_with_tasks`
//! would be one call site away from a product where closing a milestone
//! silently completes work nobody looked at.
//!
//! # Why progress is one query and not one per milestone
//!
//! A project's milestone list renders every bar at once. Counting per milestone
//! is N+1 by construction, and the N is under the control of whoever creates
//! milestones. The `GROUP BY` below is served by `task_milestone_ix`
//! (migration 0005), which is partial on `milestone_id IS NOT NULL` — so the
//! scan is over tasks that *have* a milestone rather than over the table.

use std::collections::HashMap;

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// The most milestones one project may hold.
///
/// `docs/21` bounds every input, and this list is returned whole rather than
/// paged: a milestone list is project configuration, like statuses, and a
/// cursor over a control panel is ceremony. The bound is what keeps "returned
/// whole" honest — past it the create is refused rather than the list truncated,
/// because a truncated configuration list is the kind of wrong that looks right.
pub const MAX_PER_PROJECT: i64 = 200;

/// A milestone as stored.
#[derive(Debug, Clone)]
pub struct MilestoneRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub due_at: Option<OffsetDateTime>,
    /// `Some` once closed. `docs/03`'s rollup rule means this says nothing
    /// about the state of the milestone's tasks.
    pub completed_at: Option<OffsetDateTime>,
}

const COLUMNS: &str = "id, workspace_id, project_id, name, due_at, completed_at";

type MilestoneTuple = (
    Uuid,
    Uuid,
    Uuid,
    String,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
);

fn row_of(t: MilestoneTuple) -> MilestoneRow {
    MilestoneRow {
        id: t.0,
        workspace_id: t.1,
        project_id: t.2,
        name: t.3,
        due_at: t.4,
        completed_at: t.5,
    }
}

/// How much of a milestone is done, and how much there is.
///
/// Both numbers, never a percentage: a bar labelled `58%` hides whether it is
/// 7 of 12 or 700 of 1200, and the two are different conversations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    /// Tasks in a `COMPLETED` state. `CANCELED` is **not** counted as done —
    /// cancelled work was not delivered, and folding it in makes a milestone
    /// look finished because somebody gave up.
    pub done: i64,
    pub total: i64,
}

/// Every milestone in a project, open first, then by due date.
///
/// Ordered so that the list answers "what is next" before "what happened":
/// incomplete milestones first, then the soonest due, then a stable id
/// tiebreaker so two milestones sharing a due date do not swap places between
/// renders.
///
/// # Errors
///
/// Any database error.
pub async fn list_for_project(
    scoped: &mut Scoped<'_>,
    project_id: Uuid,
) -> Result<Vec<MilestoneRow>, sqlx::Error> {
    let sql = format!(
        "SELECT {COLUMNS} FROM milestone
          WHERE workspace_id = $1 AND project_id = $2
          ORDER BY (completed_at IS NOT NULL), due_at NULLS LAST, id
          LIMIT {MAX_PER_PROJECT}"
    );
    let rows: Vec<MilestoneTuple> = sqlx::query_as(&sql)
        .bind(scoped.workspace_id().as_uuid())
        .bind(project_id)
        .fetch_all(scoped.conn())
        .await?;
    Ok(rows.into_iter().map(row_of).collect())
}

/// One milestone, by id, within the caller's workspace.
///
/// Visibility is the **project's**, not this row's, and is resolved by the
/// caller: nothing here can see whether the actor may read the project, and a
/// repository that guessed would be a second, weaker copy of `docs/04`.
///
/// # Errors
///
/// Any database error.
pub async fn read(scoped: &mut Scoped<'_>, id: Uuid) -> Result<Option<MilestoneRow>, sqlx::Error> {
    let sql = format!("SELECT {COLUMNS} FROM milestone WHERE id = $1 AND workspace_id = $2");
    let row: Option<MilestoneTuple> = sqlx::query_as(&sql)
        .bind(id)
        .bind(scoped.workspace_id().as_uuid())
        .fetch_optional(scoped.conn())
        .await?;
    Ok(row.map(row_of))
}

/// How many milestones a project already holds.
///
/// # Errors
///
/// Any database error.
pub async fn count_in_project(
    scoped: &mut Scoped<'_>,
    project_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM milestone WHERE workspace_id = $1 AND project_id = $2")
        .bind(scoped.workspace_id().as_uuid())
        .bind(project_id)
        .fetch_one(scoped.conn())
        .await
}

/// What a milestone create supplies.
#[derive(Debug, Clone)]
pub struct NewMilestone {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub due_at: Option<OffsetDateTime>,
}

/// Create a milestone.
///
/// `None` when the `(project_id, name)` unique constraint refuses it. Reported
/// as an absent row rather than as an error so the caller answers "that name is
/// taken" without matching on a driver error string.
///
/// # Errors
///
/// Any database error other than the unique violation.
pub async fn insert(
    scoped: &mut Scoped<'_>,
    new: &NewMilestone,
) -> Result<Option<MilestoneRow>, sqlx::Error> {
    let sql = format!(
        "INSERT INTO milestone (id, workspace_id, project_id, name, due_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (project_id, name) DO NOTHING
         RETURNING {COLUMNS}"
    );
    let row: Option<MilestoneTuple> = sqlx::query_as(&sql)
        .bind(new.id)
        .bind(scoped.workspace_id().as_uuid())
        .bind(new.project_id)
        .bind(&new.name)
        .bind(new.due_at)
        .fetch_optional(scoped.conn())
        .await?;
    Ok(row.map(row_of))
}

/// What a milestone update may change.
///
/// `completed` is a `bool` and not a timestamp: *when* a milestone closed is the
/// server's answer, and letting a client post a `completed_at` would let it
/// backdate one.
#[derive(Debug, Clone, Default)]
pub struct MilestonePatch {
    pub name: Option<String>,
    /// `Some(None)` clears the due date; `None` leaves it alone.
    pub due_at: Option<Option<OffsetDateTime>>,
    /// `Some(true)` closes it, `Some(false)` reopens it, `None` leaves it.
    pub completed: Option<bool>,
}

/// Apply a patch to a milestone.
///
/// `None` when no such row exists in this workspace, or when the new name
/// collides — the same two-answers-one-shape as [`insert`], for the same reason.
///
/// # Errors
///
/// Any database error other than the unique violation.
pub async fn update(
    scoped: &mut Scoped<'_>,
    id: Uuid,
    patch: &MilestonePatch,
) -> Result<Option<MilestoneRow>, sqlx::Error> {
    // COALESCE for "absent = unchanged"; the explicit boolean for the two
    // columns where NULL is a value rather than a silence.
    let sql = format!(
        "UPDATE milestone
            SET name         = COALESCE($3::text, name),
                due_at       = CASE WHEN $4 THEN $5::timestamptz ELSE due_at END,
                completed_at = CASE
                                 WHEN $6 IS NULL       THEN completed_at
                                 WHEN $6               THEN COALESCE(completed_at, now())
                                 ELSE NULL
                               END
          WHERE id = $1 AND workspace_id = $2
      RETURNING {COLUMNS}"
    );
    let row: Option<MilestoneTuple> = sqlx::query_as(&sql)
        .bind(id)
        .bind(scoped.workspace_id().as_uuid())
        .bind(patch.name.as_deref())
        .bind(patch.due_at.is_some())
        .bind(patch.due_at.flatten())
        .bind(patch.completed)
        .fetch_optional(scoped.conn())
        .await?;
    Ok(row.map(row_of))
}

/// Done-and-total per milestone, for every milestone in a project.
///
/// Counts **only tasks the statement can see**: RLS scopes it to the workspace,
/// and the caller has already established that the actor may read the project.
/// Deleted and archived tasks are excluded, because a rollup that counted
/// tombstones would drift upward forever.
///
/// # Errors
///
/// Any database error.
pub async fn progress(
    scoped: &mut Scoped<'_>,
    project_id: Uuid,
) -> Result<HashMap<Uuid, Progress>, sqlx::Error> {
    let rows: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        "SELECT t.milestone_id,
                count(*)                                        AS total,
                count(*) FILTER (WHERE t.state = 'COMPLETED')   AS done
           FROM task t
          WHERE t.workspace_id = $1
            AND t.project_id = $2
            AND t.milestone_id IS NOT NULL
            AND t.deleted_at IS NULL
            AND t.archived_at IS NULL
          GROUP BY t.milestone_id",
    )
    .bind(scoped.workspace_id().as_uuid())
    .bind(project_id)
    .fetch_all(scoped.conn())
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, total, done)| (id, Progress { done, total }))
        .collect())
}

/// Whether a milestone may be set on a task in `project_id`.
///
/// A milestone belongs to exactly one project (migration 0004), so this is the
/// milestone counterpart of `task::usable_tag`. Returns the name, which is what
/// lets the activity record hold a display value rather than an id (`docs/25`).
///
/// # Errors
///
/// Any database error.
pub async fn usable(
    scoped: &mut Scoped<'_>,
    milestone_id: Uuid,
    project_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT name FROM milestone
          WHERE id = $1 AND workspace_id = $2 AND project_id = $3",
    )
    .bind(milestone_id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(project_id)
    .fetch_optional(scoped.conn())
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The needles are assembled at runtime so this file's own assertions do
    /// not match themselves — a source-text gate that trips on its own text is
    /// a gate nobody can keep.
    fn needle(head: &str, tail: &str) -> String {
        format!("{head} {tail}")
    }

    #[test]
    fn nothing_here_writes_to_a_task() {
        // The invariant this module exists to hold, asserted against its own
        // source: `docs/03`'s rollup rule is that progress is displayed and
        // never enforced, and the way that stays true is that no statement in
        // this file writes the `task` table. A future `complete_with_tasks`
        // would fail here before it reached a handler.
        let source = include_str!("milestone.rs").to_uppercase();
        for banned in [needle("UPDATE", "TASK"), needle("INSERT INTO", "TASK")] {
            assert!(
                !source.contains(&banned),
                "milestone.rs contains `{banned}`; progress is displayed, never enforced"
            );
        }
    }

    #[test]
    fn canceled_work_is_not_counted_as_done() {
        // A milestone that looks finished because somebody gave up is the
        // failure this filter prevents. Pinned against the SQL text because the
        // predicate is the entire rule.
        let source = include_str!("milestone.rs");
        assert!(source.contains("FILTER (WHERE t.state = 'COMPLETED')"));
        assert!(
            !source.contains(&needle("t.state =", "'CANCELED'")),
            "CANCELED must not enter the done count"
        );
    }

    #[test]
    fn progress_reports_both_numbers() {
        let p = Progress { done: 7, total: 12 };
        assert_eq!((p.done, p.total), (7, 12));
        // Default is the answer for a milestone with no tasks at all, which is
        // `0/0` and not a division by zero.
        assert_eq!(Progress::default(), Progress { done: 0, total: 0 });
    }
}
