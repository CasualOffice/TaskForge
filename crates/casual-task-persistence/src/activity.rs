//! Reading the activity stream (`docs/25` §The three streams, C-011).
//!
//! # The failure this module prevents
//!
//! History that exists and cannot be read. Every change has written an
//! `activity_event` in the same transaction as the change itself since C-011
//! (ADR-006) — the task drawer's History tab had nothing to call, so the data
//! accumulated and was invisible. This is the read half.
//!
//! # Why the cursor carries `occurred_at` and not just an id
//!
//! `activity_event` is **partitioned by `occurred_at`** (migration 0007), and
//! its primary key is `(id, occurred_at)` for that reason. A cursor keyed on
//! the id alone could not be resumed without scanning every partition to find
//! which one holds it. Carrying the timestamp lets the planner eliminate
//! partitions before it reads anything, and it is the leading column of
//! `activity_stream_ix` — `tests/explain/queries/13` asserts the plan.
//!
//! # This module does not decide who may read
//!
//! Visibility is the *task's* (`docs/25`: activity "must be readable by anyone
//! who can see the task") and the permission is `task.history.read`. Both are
//! the caller's to establish; a repository that answered them would be a second
//! place the rule lived.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// One entry in a task's history.
#[derive(Debug, Clone)]
pub struct ActivityRow {
    pub id: Uuid,
    pub event_type: String,
    /// `None` for a system-generated change (a retention sweep, an automation
    /// running as nobody).
    pub actor_id: Option<Uuid>,
    /// Display **values**, not ids. `docs/25`: the stream is rendered years
    /// later, possibly after a status was renamed or deleted, and must still
    /// read correctly.
    pub changes: serde_json::Value,
    pub occurred_at: OffsetDateTime,
}

/// Where a page resumes: `(occurred_at, id)`, both descending.
pub type ActivityCursor = (OffsetDateTime, Uuid);

/// One page of a task's history, newest first.
///
/// `limit` is the page size; **one more row than that is fetched**, so "is
/// there a next page" costs no second query (`docs/05` §Pagination).
///
/// # Errors
///
/// Any database error.
pub async fn for_task(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    after: Option<ActivityCursor>,
    limit: u32,
) -> Result<Vec<ActivityRow>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    // Row-value comparison rather than the expanded `a < x OR (a = x AND ...)`
    // form: `docs/26` — PostgreSQL drives a composite index from the row-value
    // form and often cannot from the expanded one. Here the composite is
    // activity_stream_ix (workspace_id, aggregate_id, occurred_at DESC).
    let rows: Vec<(
        Uuid,
        String,
        Option<Uuid>,
        serde_json::Value,
        OffsetDateTime,
    )> = sqlx::query_as(
        "SELECT id, event_type, actor_id, changes, occurred_at
               FROM activity_event
              WHERE workspace_id = $1
                AND aggregate_id = $2
                AND ($3::timestamptz IS NULL
                     OR (occurred_at, id) < ($3::timestamptz, $4::uuid))
              ORDER BY occurred_at DESC, id DESC
              LIMIT $5",
    )
    .bind(workspace)
    .bind(task_id)
    .bind(after.map(|c| c.0))
    .bind(after.map(|c| c.1))
    .bind(i64::from(limit).saturating_add(1))
    .fetch_all(scoped.conn())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, event_type, actor_id, changes, occurred_at)| ActivityRow {
                id,
                event_type,
                actor_id,
                changes,
                occurred_at,
            },
        )
        .collect())
}

/// The display names of the actors in a page, for rendering "Sarah moved this".
///
/// Read once per page rather than once per row — the same argument `docs/04`
/// §The list problem makes about authorization, applied to a join the stream
/// would otherwise do per entry.
///
/// # Errors
///
/// Any database error.
pub async fn actor_names(
    scoped: &mut Scoped<'_>,
    actors: &[Uuid],
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    if actors.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as("SELECT id, display_name FROM user_account WHERE id = ANY($1)")
        .bind(actors)
        .fetch_all(scoped.conn())
        .await
}
