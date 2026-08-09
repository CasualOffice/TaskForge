//! Writing a workflow: its statuses, and the migration that moves work off
//! one. Its edges are `workflow_edge`.
//!
//! # Why this is not in `workflow.rs`
//!
//! That module changes when the *default* workflow does — the six statuses and
//! nine edges `docs/23` draws, and the provisioning race a fresh workspace runs.
//! This one changes when the **authoring contract** does. They share a table and
//! nothing else: reading a workflow is a two-query board dependency on the hot
//! path, and editing one is a rare administrative write that moves in-flight
//! work between statuses.
//!
//! # The one thing this module exists to make impossible
//!
//! A task on a status that no longer exists. `docs/23` §Deleting a status
//! rejects both cheap answers — silently orphaning tasks, and lazily remapping
//! them on next read — because each produces "tasks whose history does not
//! explain their status". So [`migrate_tasks_off_status`] writes the activity
//! rows **before** it moves anything, in the same transaction as the move and
//! the `DELETE`, and there is no path here that removes a status without going
//! through it.
//!
//! # Why the migration is set-based and not a loop
//!
//! `docs/23`'s acceptance gate migrates 50,000 tasks. A per-task round trip
//! would hold a write transaction open for minutes over rows that other people
//! are trying to edit. Every statement here touches the whole affected set at
//! once — including the per-task activity rows, which are one `INSERT … SELECT`
//! rather than 50,000 inserts. The activity stream is identical either way; the
//! lock duration is not.

use uuid::Uuid;

use crate::scoped::Scoped;
use crate::workflow::StatusRow;

/// A name collision on a `UNIQUE` the caller can do something about.
///
/// Separated from a generic database error because the two lead to different
/// responses: `409` naming the field, versus `500`.
#[derive(Debug)]
pub enum WriteError {
    /// `workflow_status (workflow_id, name)` or
    /// `workflow_transition (workflow_id, from_status_id, to_status_id)`.
    Duplicate,
    /// A `required_permission` that is not in the `permission` table — the
    /// registry is closed (`docs/04`), so this is a caller error, not a fault.
    UnknownReference,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for WriteError {
    fn from(error: sqlx::Error) -> Self {
        match &error {
            sqlx::Error::Database(db) if db.is_unique_violation() => Self::Duplicate,
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => Self::UnknownReference,
            _ => Self::Db(error),
        }
    }
}

/// Take the workflow's version, or report that someone else moved first.
///
/// **Every** authoring write goes through this, and it is the reason two admins
/// editing one workflow cannot interleave into something neither asked for —
/// "delete Blocked, migrating to In Progress" racing "delete In Progress" would
/// otherwise both read a consistent workflow and both commit.
///
/// The workflow is the aggregate: statuses and transitions carry no version of
/// their own, and giving them one would let those same two operations pass each
/// other because they touched different rows.
///
/// Returns the new version, or `None` when `expected` is stale (`docs/24`:
/// "0 rows affected ⇒ someone else wrote first ⇒ 409").
///
/// # Errors
///
/// Any database error.
pub async fn claim_workflow(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
    expected: i64,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "UPDATE workflow SET version = version + 1
          WHERE id = $1 AND version = $2
      RETURNING version",
    )
    .bind(workflow)
    .bind(expected)
    .fetch_optional(scoped.conn())
    .await
}

/// One status of one workflow, or `None`.
///
/// Takes the workflow as well as the status so "belongs to a different
/// workflow" is answered by the query rather than by a comparison a caller
/// could forget — `TF-WFL-0008` exists because the id in a `migrate_to` is
/// attacker-supplied.
///
/// # Errors
///
/// Any database error.
pub async fn status_in(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
    status: Uuid,
) -> Result<Option<StatusRow>, sqlx::Error> {
    let row: Option<(Uuid, String, String, i32, bool)> = sqlx::query_as(
        "SELECT id, name, state::text, position, is_initial
           FROM workflow_status
          WHERE workflow_id = $1 AND id = $2",
    )
    .bind(workflow)
    .bind(status)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(
        row.map(|(id, name, state, position, is_initial)| StatusRow {
            id,
            name,
            state,
            position,
            is_initial,
        }),
    )
}

/// How many tasks hold this status.
///
/// Counts soft-deleted rows too. They still carry the foreign key, so a
/// migration that skipped them would leave the `DELETE` failing on a constraint
/// violation the admin has no way to interpret — and the count shown in the
/// delete dialog would be smaller than the number of rows that actually move.
///
/// # Errors
///
/// Any database error.
pub async fn count_tasks_on(scoped: &mut Scoped<'_>, status: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM task WHERE status_id = $1")
        .bind(status)
        .fetch_one(scoped.conn())
        .await
}

/// The task count for every status of one workflow, as `(status_id, count)`.
///
/// One grouped query rather than one per status: the settings screen shows the
/// count beside every status, and N statuses meant N round trips before anyone
/// pressed Delete.
///
/// # Errors
///
/// Any database error.
pub async fn counts_by_status(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
) -> Result<Vec<(Uuid, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT s.id, count(t.id)
           FROM workflow_status s
           LEFT JOIN task t ON t.status_id = s.id
          WHERE s.workflow_id = $1
       GROUP BY s.id",
    )
    .bind(workflow)
    .fetch_all(scoped.conn())
    .await
}

/// Append a status to a workflow.
///
/// `is_initial` is deliberately not a parameter: exactly one status per workflow
/// is initial (`migrations/0004`'s partial unique index), so making it settable
/// at creation would mean every create either collides or silently demotes the
/// existing one. Designating a different initial status is [`set_initial`],
/// which is one operation that does both halves.
///
/// # Errors
///
/// [`WriteError::Duplicate`] when the name is taken in this workflow.
pub async fn insert_status(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
    name: &str,
    state: &str,
) -> Result<StatusRow, WriteError> {
    let workspace = scoped.workspace_id().as_uuid();
    let id = Uuid::now_v7();
    // Appended after the last status. `COALESCE(max, 0) + 1` rather than a
    // count, because a workflow that has had a status deleted has gaps and a
    // count would collide with a position already in use.
    let row: (Uuid, String, String, i32, bool) = sqlx::query_as(
        "INSERT INTO workflow_status
             (id, workflow_id, workspace_id, name, state, position, is_initial)
         SELECT $1, $2, $3, $4, $5::task_state,
                COALESCE(max(position), 0) + 1, false
           FROM workflow_status WHERE workflow_id = $2
      RETURNING id, name, state::text, position, is_initial",
    )
    .bind(id)
    .bind(workflow)
    .bind(workspace)
    .bind(name)
    .bind(state)
    .fetch_one(scoped.conn())
    .await?;
    Ok(StatusRow {
        id: row.0,
        name: row.1,
        state: row.2,
        position: row.3,
        is_initial: row.4,
    })
}

/// Rename a status.
///
/// # Errors
///
/// [`WriteError::Duplicate`] when the name is taken in this workflow.
pub async fn rename_status(
    scoped: &mut Scoped<'_>,
    status: Uuid,
    name: &str,
) -> Result<(), WriteError> {
    sqlx::query("UPDATE workflow_status SET name = $2 WHERE id = $1")
        .bind(status)
        .bind(name)
        .execute(scoped.conn())
        .await?;
    Ok(())
}

/// Remap a status onto a different permanent state, and recompute every task
/// on it in the same statement pair.
///
/// `docs/23` §Changing a status's state mapping: "permitted, and **retroactive
/// by construction**: `task.state` is recomputed for every task on that status
/// in the same transaction". Retroactive is the whole point — `task.state` is
/// the column every report reads without a join, so a status whose meaning
/// changed and whose tasks did not would make the derived column a lie for
/// exactly the rows that moved.
///
/// Returns how many tasks were recomputed.
///
/// # Errors
///
/// Any database error.
pub async fn remap_status_state(
    scoped: &mut Scoped<'_>,
    status: Uuid,
    state: &str,
    actor: Uuid,
) -> Result<u64, sqlx::Error> {
    sqlx::query("UPDATE workflow_status SET state = $2::task_state WHERE id = $1")
        .bind(status)
        .bind(state)
        .execute(scoped.conn())
        .await?;

    // `state <> $2` keeps the version bump off rows that did not change: a task
    // already carrying the target state has nothing to recompute, and bumping
    // it would invalidate an ETag a client holds for no reason.
    let recomputed = sqlx::query(
        "UPDATE task
            SET state = $2::task_state,
                version = version + 1,
                updated_at = now(),
                updated_by = $3
          WHERE status_id = $1 AND state <> $2::task_state",
    )
    .bind(status)
    .bind(state)
    .bind(actor)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(recomputed)
}

/// Make `status` the workflow's initial status, demoting the current one.
///
/// Both halves in the order the partial unique index requires: the demotion
/// first, or the promotion collides with the status it is replacing.
///
/// # Errors
///
/// Any database error.
pub async fn set_initial(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
    status: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE workflow_status SET is_initial = false WHERE workflow_id = $1 AND is_initial",
    )
    .bind(workflow)
    .execute(scoped.conn())
    .await?;
    sqlx::query("UPDATE workflow_status SET is_initial = true WHERE id = $1")
        .bind(status)
        .execute(scoped.conn())
        .await?;
    Ok(())
}

/// Rewrite every position of a workflow from a complete ordering.
///
/// Takes the **whole** order rather than one status and a target index, because
/// `workflow_status` has no unique constraint on `(workflow_id, position)` —
/// so a partial reorder can leave two statuses sharing a position, and a board
/// whose column order then depends on which row the planner returns first. A
/// complete permutation cannot express that state.
///
/// # Errors
///
/// Any database error.
pub async fn reorder_statuses(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
    order: &[Uuid],
) -> Result<(), sqlx::Error> {
    // One statement over an ordinality-joined array: N round trips would each
    // pass through a state where two statuses share a position, which is
    // visible to a concurrent reader in READ COMMITTED.
    sqlx::query(
        "UPDATE workflow_status s
            SET position = o.ordinality
           FROM unnest($2::uuid[]) WITH ORDINALITY AS o(id, ordinality)
          WHERE s.id = o.id AND s.workflow_id = $1",
    )
    .bind(workflow)
    .bind(order)
    .execute(scoped.conn())
    .await?;
    Ok(())
}

/// Every task on a status, as `(task_id, project_id)`.
///
/// Read before the migration so the activity rows can be written with ids this
/// process minted. The alternative — `INSERT … SELECT gen_random_uuid()` — is
/// one fewer round trip and produces v4 ids in a table whose every other row is
/// UUIDv7 (`docs/05`), which would make the activity stream's own ordering key
/// stop being time-ordered for exactly the rows a migration produced.
///
/// Bounded by the caller against [`MIGRATION_LIMIT`] before it is used, so the
/// vector is never larger than the request is allowed to move.
///
/// # Errors
///
/// Any database error.
pub async fn tasks_on(
    scoped: &mut Scoped<'_>,
    status: Uuid,
) -> Result<Vec<(Uuid, Uuid)>, sqlx::Error> {
    sqlx::query_as("SELECT id, project_id FROM task WHERE status_id = $1")
        .bind(status)
        .fetch_all(scoped.conn())
        .await
}

/// The most tasks one request may migrate.
///
/// `docs/23`: "bulk moves above 10,000 tasks run as a tracked background job
/// with progress, not a request." The job does not exist (D-063), so this is
/// the boundary at which the API refuses rather than quietly doing the thing
/// the design record says a request must not do.
pub const MIGRATION_LIMIT: i64 = 10_000;

/// Move every task off `from` and onto `to`, writing one activity row each.
///
/// The activity rows are written **first**, while the tasks still carry the old
/// status. Reversing the two would produce events saying a task moved from the
/// status it is already on.
///
/// `reason: "workflow_migration"` is in every row, and the actor is the admin
/// who pressed Delete — `docs/23` requires both, so that a reader years later
/// can tell an ordinary transition from a configuration change that moved their
/// work without them touching it.
///
/// Returns how many tasks moved.
///
/// # Errors
///
/// Any database error.
pub async fn migrate_tasks_off_status(
    scoped: &mut Scoped<'_>,
    from: &StatusRow,
    to: &StatusRow,
    tasks: &[(Uuid, Uuid)],
    actor: Uuid,
) -> Result<u64, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    // Display VALUES, not ids (`docs/25`): the status this names is about to
    // stop existing, so an id would render as a dangling reference forever.
    let changes = serde_json::json!({
        "status": { "from": from.name, "to": to.name },
        "reason": "workflow_migration",
    });

    if !tasks.is_empty() {
        let event_ids: Vec<Uuid> = (0..tasks.len()).map(|_| Uuid::now_v7()).collect();
        let task_ids: Vec<Uuid> = tasks.iter().map(|(id, _)| *id).collect();
        let project_ids: Vec<Uuid> = tasks.iter().map(|(_, project)| *project).collect();
        sqlx::query(
            "INSERT INTO activity_event
                 (id, workspace_id, project_id, aggregate_type, aggregate_id,
                  event_type, actor_id, changes)
             SELECT e.id, $1, e.project_id, 'task', e.task_id,
                    'task.status.changed', $2, $3
               FROM unnest($4::uuid[], $5::uuid[], $6::uuid[])
                    AS e(id, task_id, project_id)",
        )
        .bind(workspace)
        .bind(actor)
        .bind(&changes)
        .bind(&event_ids)
        .bind(&task_ids)
        .bind(&project_ids)
        .execute(scoped.conn())
        .await?;
    }

    // `state` is written in the SAME statement as `status_id` — `docs/23`: that
    // is the invariant which lets every report read `state` without a join, and
    // a migration is exactly the bulk write that would break it if it drifted.
    let moved = sqlx::query(
        "UPDATE task
            SET status_id = $2,
                state = $3::task_state,
                version = version + 1,
                updated_at = now(),
                updated_by = $4
          WHERE status_id = $1",
    )
    .bind(from.id)
    .bind(to.id)
    .bind(&to.state)
    .bind(actor)
    .execute(scoped.conn())
    .await?
    .rows_affected();

    Ok(moved)
}

/// Remove a status and every edge that mentions it.
///
/// The edges go first: `workflow_transition` references `workflow_status`
/// without `ON DELETE`, so a status still named by an arrow cannot be removed.
/// Returns how many edges went with it — the caller reports that number,
/// because an admin who deleted one status and silently lost four transitions
/// has been surprised.
///
/// # Errors
///
/// Any database error.
pub async fn delete_status(scoped: &mut Scoped<'_>, status: Uuid) -> Result<u64, sqlx::Error> {
    let edges = sqlx::query(
        "DELETE FROM workflow_transition WHERE from_status_id = $1 OR to_status_id = $1",
    )
    .bind(status)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    sqlx::query("DELETE FROM workflow_status WHERE id = $1")
        .bind(status)
        .execute(scoped.conn())
        .await?;
    Ok(edges)
}

/// Every project on this workflow, as `(project_id, team_ids, actor_is_member)`.
///
/// The authority question for a workflow edit is asked once per project in this
/// list — see `crates/casual-task-api/src/workflows/guard.rs` for why one
/// project's grant is not enough. Membership comes back in the same row because
/// `is_project_member` is one of `docs/04`'s constraints, and resolving it with
/// a query per project would be the per-row resolution `docs/04` §The list
/// problem forbids.
///
/// # Errors
///
/// Any database error.
pub async fn projects_on(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
    actor: Uuid,
) -> Result<Vec<(Uuid, Vec<Uuid>, bool)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id,
                ARRAY(SELECT pt.team_id FROM project_team pt
                       WHERE pt.project_id = p.id ORDER BY pt.team_id),
                EXISTS (SELECT 1 FROM project_membership pm
                         WHERE pm.project_id = p.id AND pm.user_id = $2)
           FROM project p
          WHERE p.workflow_id = $1 AND p.deleted_at IS NULL
       ORDER BY p.id",
    )
    .bind(workflow)
    .bind(actor)
    .fetch_all(scoped.conn())
    .await
}
