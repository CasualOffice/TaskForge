//! How long a task spent in each state (`docs/38` §Where the numbers come
//! from).
//!
//! # Why a projection and not a query
//!
//! Cycle time, lead time, time-in-state and throughput all ask the same
//! question, and none of them can be answered from `task` — which holds only
//! where the work is now. Answering them by replaying the event stream per
//! request is the unbounded query `docs/38` exists to prevent: an aggregate over
//! every status change a workspace has ever made, run by everyone at 9am.
//!
//! # Why the rebuild is the whole maintenance path
//!
//! Outbox delivery is at-least-once (`docs/25`), so a consumer that *appended*
//! an interval per event would double a task's history the first time an event
//! was redelivered — and nothing on screen would say so. [`rebuild`] derives a
//! task's entire series from the audit stream and replaces it, which is
//! idempotent by construction: the same event delivered five times produces the
//! same rows.
//!
//! It also means the repair path and the steady-state path are the same code.
//! A rebuild that is only exercised during an incident is a rebuild that does
//! not work during an incident.
//!
//! # Why the source is `audit_event` and not `activity_event`
//!
//! `docs/38` says "rebuildable from `activity_event`", and in this
//! implementation that is not possible: `docs/25` requires the activity stream
//! to carry **display values** — the status *names* — precisely so an entry
//! stays readable after a status is renamed. The audit stream carries the ids
//! and the state on both sides of every transition, which is what an interval
//! needs. The design's intent holds; the table named in it does not.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// The event types that move a task between states.
///
/// `task.created` opens the first interval. `task.status.changed` and
/// `task.reopened` are the two spellings a transition takes — the second is
/// emitted when a task comes back out of a terminal state, and it moves the
/// task exactly as the first does.
const MOVES: [&str; 3] = ["task.created", "task.status.changed", "task.reopened"];

/// Derive one task's state intervals again, replacing whatever is there.
///
/// # Errors
///
/// Any database error.
pub async fn rebuild(scoped: &mut Scoped<'_>, task_id: Uuid) -> Result<(), sqlx::Error> {
    // The task's own row supplies the tenant and project. A task that is gone
    // leaves no intervals — the delete below has already removed them.
    let owner: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT workspace_id, project_id FROM task WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(task_id)
    .fetch_optional(scoped.conn())
    .await?;

    sqlx::query("DELETE FROM task_state_interval WHERE task_id = $1")
        .bind(task_id)
        .execute(scoped.conn())
        .await?;

    let Some((workspace_id, project_id)) = owner else {
        return Ok(());
    };

    // Every move, oldest first. `changes -> 'after'` carries the status and
    // state the task landed in; the row that opens an interval is the one that
    // closes the interval before it.
    let moves: Vec<(Option<String>, Option<String>, OffsetDateTime)> = sqlx::query_as(
        "SELECT changes -> 'after' ->> 'status_id',
                changes -> 'after' ->> 'state',
                occurred_at
           FROM audit_event
          WHERE target_id = $1
            AND event_type = ANY($2)
          ORDER BY occurred_at, id",
    )
    .bind(task_id)
    .bind(MOVES.map(str::to_owned).as_slice())
    .fetch_all(scoped.conn())
    .await?;

    // An event whose `after` carries no status is one this projection cannot
    // place — an older row written before the shape settled, or a create that
    // recorded no status. Skipped rather than guessed: an interval at the wrong
    // status is worse than a missing one, because it is counted.
    let placed: Vec<(Uuid, String, OffsetDateTime)> = moves
        .into_iter()
        .filter_map(|(status, state, at)| {
            let status = status?.parse::<Uuid>().ok()?;
            Some((status, state?, at))
        })
        .collect();

    for (index, (status_id, state, entered_at)) in placed.iter().enumerate() {
        // The next move is this interval's end; the last one is still open.
        let exited_at = placed.get(index + 1).map(|next| next.2);
        sqlx::query(
            "INSERT INTO task_state_interval
                 (task_id, workspace_id, project_id, state, status_id, entered_at, exited_at)
             VALUES ($1,$2,$3,$4::task_state,$5,$6,$7)
             ON CONFLICT (task_id, entered_at) DO UPDATE
                 SET state = EXCLUDED.state,
                     status_id = EXCLUDED.status_id,
                     exited_at = EXCLUDED.exited_at",
        )
        .bind(task_id)
        .bind(workspace_id)
        .bind(project_id)
        .bind(state)
        .bind(status_id)
        .bind(entered_at)
        .bind(exited_at)
        .execute(scoped.conn())
        .await?;
    }

    Ok(())
}

/// One task's intervals, oldest first. For tests and for the repair path.
///
/// # Errors
///
/// Any database error.
pub async fn for_task(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
) -> Result<Vec<IntervalRow>, sqlx::Error> {
    let rows: Vec<(String, Uuid, OffsetDateTime, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT state::text, status_id, entered_at, exited_at
           FROM task_state_interval
          WHERE task_id = $1
          ORDER BY entered_at",
    )
    .bind(task_id)
    .fetch_all(scoped.conn())
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| IntervalRow {
            state: row.0,
            status_id: row.1,
            entered_at: row.2,
            exited_at: row.3,
        })
        .collect())
}

/// One occupancy.
#[derive(Debug, Clone)]
pub struct IntervalRow {
    pub state: String,
    pub status_id: Uuid,
    pub entered_at: OffsetDateTime,
    /// `None` while the task is still in this state.
    pub exited_at: Option<OffsetDateTime>,
}
