//! Who holds a task, where it has reached, and whether it passed
//! (`docs/45-DEVELOPMENT-LIFECYCLE-AND-CUSTODY.md`).
//!
//! # Why these four live together
//!
//! Transfer, promotion and verification are one story told three ways: the chain
//! of custody. They are written by different handlers and read by one panel, and
//! every one of them is append-only — migration 0031 grants no `DELETE` on any
//! of these tables, because a custody chain you can edit is not a custody chain.
//!
//! # The two clocks, in code
//!
//! `task.status_id` is what state the work is in and belongs to
//! [`crate::task`]. `task.environment_id` is where it has *reached*.
//!
//! **The invariant is not "one writer" but "no silent move":** every change to
//! that column leaves a promotion row. Two functions satisfy it —
//! [`promote`], which sets the column and logs in one statement pair, and
//! [`record_promotion`], which logs a move another module already made under its
//! own optimistic-concurrency check (`crate::environment::set_on_task`, behind
//! `PUT /tasks/{id}/environment`). Advancing the second clock without a row is
//! the failure this module exists to prevent, and either function prevents it.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// The tuples these rows decode into, before they become the structs above.
///
/// Named because six anonymous columns say nothing about which is which, and
/// clippy is right that the inline form is unreadable at three call sites.
type TransferTuple = (
    Uuid,
    Option<Uuid>,
    Uuid,
    Uuid,
    OffsetDateTime,
    Option<String>,
);
type PromotionTuple = (Uuid, Uuid, Option<Uuid>, Uuid, OffsetDateTime);
type VerificationTuple = (Uuid, Uuid, String, Uuid, OffsetDateTime, Option<String>);

/// One hand-off between teams.
#[derive(Debug, Clone)]
pub struct TransferRow {
    pub id: Uuid,
    /// `None` for the first assignment out of triage — a transfer from nobody.
    pub from_team_id: Option<Uuid>,
    pub to_team_id: Uuid,
    pub moved_by: Uuid,
    pub moved_at: OffsetDateTime,
    pub note: Option<String>,
}

/// One step along the second clock.
#[derive(Debug, Clone)]
pub struct PromotionRow {
    pub id: Uuid,
    pub environment_id: Uuid,
    /// Set when it moved as part of a release, `None` when a developer promoted
    /// it themselves at resolve time. Both happen, at different moments.
    pub release_id: Option<Uuid>,
    pub promoted_by: Uuid,
    pub promoted_at: OffsetDateTime,
}

/// One verdict, on one environment.
#[derive(Debug, Clone)]
pub struct VerificationRow {
    pub id: Uuid,
    pub environment_id: Uuid,
    /// `PASS` or `FAIL` — the database enum, as text.
    pub verdict: String,
    pub verified_by: Uuid,
    pub verified_at: OffsetDateTime,
    pub note: Option<String>,
}

/// Why a custody write could not be applied.
#[derive(Debug)]
pub enum CustodyError {
    /// The team is not on the task's project. `docs/45`: a task owned by people
    /// who cannot see it is not a hand-off, it is a disappearance.
    TeamNotOnProject,
    /// The environment belongs to a different project than the task.
    EnvironmentNotOnProject,
    /// The task is already owned by that team.
    AlreadyThere,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for CustodyError {
    fn from(error: sqlx::Error) -> Self {
        Self::Db(error)
    }
}

impl std::fmt::Display for CustodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TeamNotOnProject => f.write_str("that team is not on this task's project"),
            Self::EnvironmentNotOnProject => {
                f.write_str("that environment is not on this task's project")
            }
            Self::AlreadyThere => f.write_str("the task is already owned by that team"),
            Self::Db(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CustodyError {}

/// Hand a task to a team, clearing its assignees and logging the move.
///
/// # What "clearing the assignees" is for
///
/// `docs/45`: the task lands **unassigned** in the receiving team's queue, and
/// their process picks it up. Keeping the previous developer attached leaves the
/// receiving team with nothing in any queue to notice, which is the failure
/// teams complain about most. It also makes "unassigned, owned by my team" the
/// triage list a lead actually opens.
///
/// # Why the guard is a subquery and not a read
///
/// The receiving team must be on the task's project. Checked inside the same
/// statement that writes, so two concurrent requests — one removing the team
/// from the project, one transferring to it — cannot interleave into a task
/// owned by a team that has just lost its reach.
///
/// # Errors
///
/// [`CustodyError::TeamNotOnProject`], [`CustodyError::AlreadyThere`], or any
/// database error.
pub async fn transfer(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    to_team: Uuid,
    moved_by: Uuid,
    note: Option<&str>,
) -> Result<TransferRow, CustodyError> {
    let workspace = scoped.workspace_id().as_uuid();

    let current: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT team_id FROM task WHERE id = $1 AND deleted_at IS NULL")
            .bind(task_id)
            .fetch_optional(scoped.conn())
            .await?;
    let from_team = current.ok_or(CustodyError::Db(sqlx::Error::RowNotFound))?;
    if from_team == Some(to_team) {
        return Err(CustodyError::AlreadyThere);
    }

    let moved = sqlx::query(
        "UPDATE task SET team_id = $2, updated_at = now(), updated_by = $3
          WHERE id = $1
            AND deleted_at IS NULL
            AND EXISTS (
                SELECT 1 FROM project_team pt
                 WHERE pt.project_id = task.project_id AND pt.team_id = $2)",
    )
    .bind(task_id)
    .bind(to_team)
    .bind(moved_by)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    if moved == 0 {
        return Err(CustodyError::TeamNotOnProject);
    }

    // The queue is the point of the transfer, so this is not optional cleanup.
    sqlx::query("DELETE FROM task_assignee WHERE task_id = $1 AND workspace_id = $2")
        .bind(task_id)
        .bind(workspace)
        .execute(scoped.conn())
        .await?;

    let row: (
        Uuid,
        Option<Uuid>,
        Uuid,
        Uuid,
        OffsetDateTime,
        Option<String>,
    ) = sqlx::query_as(
        "INSERT INTO task_team_transfer
             (id, workspace_id, task_id, from_team_id, to_team_id, moved_by, note)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         RETURNING id, from_team_id, to_team_id, moved_by, moved_at, note",
    )
    .bind(Uuid::now_v7())
    .bind(workspace)
    .bind(task_id)
    .bind(from_team)
    .bind(to_team)
    .bind(moved_by)
    .bind(note)
    .fetch_one(scoped.conn())
    .await?;

    Ok(TransferRow {
        id: row.0,
        from_team_id: row.1,
        to_team_id: row.2,
        moved_by: row.3,
        moved_at: row.4,
        note: row.5,
    })
}

/// Move a task to an environment, recording how it got there.
///
/// Sets the column and writes the log together, so the second clock cannot
/// advance without leaving a trace of the move — see the module docs for the one
/// other path that satisfies the same invariant.
///
/// Idempotent by nature rather than by trick: promoting to the environment a
/// task is already on writes another row, because a redeploy to staging *is* a
/// second event and a log that swallowed it would understate the work.
///
/// # Errors
///
/// [`CustodyError::EnvironmentNotOnProject`] or any database error.
pub async fn promote(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    environment_id: Uuid,
    release_id: Option<Uuid>,
    promoted_by: Uuid,
) -> Result<PromotionRow, CustodyError> {
    let workspace = scoped.workspace_id().as_uuid();

    let moved = sqlx::query(
        "UPDATE task SET environment_id = $2, updated_at = now(), updated_by = $3
          WHERE id = $1
            AND deleted_at IS NULL
            AND EXISTS (
                SELECT 1 FROM project_environment e
                 WHERE e.id = $2 AND e.project_id = task.project_id)",
    )
    .bind(task_id)
    .bind(environment_id)
    .bind(promoted_by)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    if moved == 0 {
        return Err(CustodyError::EnvironmentNotOnProject);
    }

    let row: PromotionTuple = sqlx::query_as(
        "INSERT INTO task_environment_promotion
             (id, workspace_id, task_id, environment_id, release_id, promoted_by)
         VALUES ($1,$2,$3,$4,$5,$6)
         RETURNING id, environment_id, release_id, promoted_by, promoted_at",
    )
    .bind(Uuid::now_v7())
    .bind(workspace)
    .bind(task_id)
    .bind(environment_id)
    .bind(release_id)
    .bind(promoted_by)
    .fetch_one(scoped.conn())
    .await?;

    Ok(PromotionRow {
        id: row.0,
        environment_id: row.1,
        release_id: row.2,
        promoted_by: row.3,
        promoted_at: row.4,
    })
}

include!("custody_history.rs");
