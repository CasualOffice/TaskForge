//! What went out together
//! (`docs/45-DEVELOPMENT-LIFECYCLE-AND-CUSTODY.md` §The two clocks).
//!
//! # Why a release is not a status
//!
//! `task.status_id` says what state work is in; `task.environment_id` says where
//! it has *reached*. A release is a fact about the second clock and about
//! several tasks at once: "these eleven things went to staging on Tuesday, and
//! we called it 2.4.0". A status column cannot hold that — it would say
//! *whether* each task moved but never that they moved *together*, which is the
//! only thing a release conversation is about.
//!
//! # Why the batch is atomic
//!
//! [`crate::custody::promote`] already records one task reaching one
//! environment, and a release is N of those with a name tied around them. It
//! would be simpler to loop and let the failures fall where they may — that is
//! what `POST /tasks/bulk` does, and it is right there, because those tasks have
//! nothing to do with each other.
//!
//! Here they do. A release that recorded nine of eleven tasks is a *worse*
//! answer than no release at all: it reads as complete, and the two missing ones
//! are now invisible in exactly the surface built to find them. So the whole
//! batch commits or none of it does, and a task that is not in the project is a
//! refusal rather than a skipped row.
//!
//! # Append-only, like the rest of the chain
//!
//! Migration 0031 grants no `DELETE` on `release`. A release is a claim about
//! what shipped, and a claim you can quietly retract is not evidence. Correcting
//! one means cutting another.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// One named batch.
#[derive(Debug, Clone)]
pub struct ReleaseRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub note: Option<String>,
    pub created_by: Uuid,
    pub created_at: OffsetDateTime,
}

type ReleaseTuple = (Uuid, Uuid, String, Option<String>, Uuid, OffsetDateTime);

fn row(tuple: ReleaseTuple) -> ReleaseRow {
    ReleaseRow {
        id: tuple.0,
        project_id: tuple.1,
        name: tuple.2,
        note: tuple.3,
        created_by: tuple.4,
        created_at: tuple.5,
    }
}

/// Why a release could not be cut.
#[derive(Debug)]
pub enum ReleaseError {
    /// `UNIQUE (project_id, name)`. Two things called 2.4.0 in one project is
    /// not a release train, it is a question nobody can answer.
    NameTaken,
    /// The environment belongs to another project, or does not exist.
    EnvironmentNotOnProject,
    /// At least one task is not in this project, is deleted, or is invisible.
    /// Deliberately not "which ones": see [`promote_batch`].
    TasksNotInProject,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for ReleaseError {
    fn from(error: sqlx::Error) -> Self {
        // 23505 is `unique_violation`, and the only unique constraint reachable
        // from here is `(project_id, name)`.
        if let sqlx::Error::Database(ref db) = error
            && db.code().as_deref() == Some("23505")
        {
            return Self::NameTaken;
        }
        Self::Db(error)
    }
}

/// Record the release itself.
///
/// # Errors
///
/// [`ReleaseError::NameTaken`], or any database error.
pub async fn create(
    scoped: &mut Scoped<'_>,
    project_id: Uuid,
    name: &str,
    note: Option<&str>,
    created_by: Uuid,
) -> Result<ReleaseRow, ReleaseError> {
    let workspace = scoped.workspace_id().as_uuid();
    let tuple: ReleaseTuple = sqlx::query_as(
        "INSERT INTO release (id, workspace_id, project_id, name, note, created_by)
         VALUES ($1,$2,$3,$4,$5,$6)
         RETURNING id, project_id, name, note, created_by, created_at",
    )
    .bind(Uuid::now_v7())
    .bind(workspace)
    .bind(project_id)
    .bind(name)
    .bind(note)
    .bind(created_by)
    .fetch_one(scoped.conn())
    .await?;
    Ok(row(tuple))
}

/// Move every task in the batch, or none of them.
///
/// # Why the refusal does not name the offending task
///
/// It cannot, safely. "Task X is not in this project" tells a caller that X
/// exists somewhere — and the whole point of scoping a batch to a project is
/// that a caller learns nothing about the tasks outside it. The count is enough
/// to act on: the caller sent the ids, so it knows which set it meant.
///
/// # Errors
///
/// [`ReleaseError::EnvironmentNotOnProject`] when the environment belongs
/// elsewhere, [`ReleaseError::TasksNotInProject`] when any task does, or any
/// database error.
pub async fn promote_batch(
    scoped: &mut Scoped<'_>,
    release_id: Uuid,
    project_id: Uuid,
    environment_id: Uuid,
    task_ids: &[Uuid],
    promoted_by: Uuid,
) -> Result<Vec<Uuid>, ReleaseError> {
    let workspace = scoped.workspace_id().as_uuid();

    // Checked before the update rather than inferred from it: with a bad
    // environment *every* row fails to move, which is indistinguishable from
    // every task being in the wrong project, and the two deserve different
    // answers.
    let known: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM project_environment WHERE id = $1 AND project_id = $2")
            .bind(environment_id)
            .bind(project_id)
            .fetch_optional(scoped.conn())
            .await?;
    if known.is_none() {
        return Err(ReleaseError::EnvironmentNotOnProject);
    }

    let moved: Vec<(Uuid,)> = sqlx::query_as(
        "UPDATE task SET environment_id = $3, updated_at = now(), updated_by = $4
          WHERE id = ANY($1)
            AND project_id = $2
            AND deleted_at IS NULL
          RETURNING id",
    )
    .bind(task_ids)
    .bind(project_id)
    .bind(environment_id)
    .bind(promoted_by)
    .fetch_all(scoped.conn())
    .await?;

    // A duplicate id in the request moves one row and would otherwise read as a
    // missing task, so the comparison is against the distinct set the caller
    // named.
    let mut wanted: Vec<Uuid> = task_ids.to_vec();
    wanted.sort_unstable();
    wanted.dedup();
    if moved.len() != wanted.len() {
        return Err(ReleaseError::TasksNotInProject);
    }

    // One row per task, ids minted here rather than by the database: `now_v7`
    // is time-ordered, which is what the promotion log is read by.
    let promotion_ids: Vec<Uuid> = (0..moved.len()).map(|_| Uuid::now_v7()).collect();
    let moved_ids: Vec<Uuid> = moved.into_iter().map(|row| row.0).collect();
    sqlx::query(
        "INSERT INTO task_environment_promotion
             (id, workspace_id, task_id, environment_id, release_id, promoted_by)
         SELECT p.id, $3, p.task_id, $4, $5, $6
           FROM unnest($1::uuid[], $2::uuid[]) AS p(id, task_id)",
    )
    .bind(&promotion_ids)
    .bind(&moved_ids)
    .bind(workspace)
    .bind(environment_id)
    .bind(release_id)
    .bind(promoted_by)
    .execute(scoped.conn())
    .await?;

    Ok(moved_ids)
}

/// One page of a project's releases, newest first.
///
/// Keyset on `id`, which is `now_v7` and therefore time-ordered: `OFFSET` is
/// banned workspace-wide (ADR-014) and would drift anyway as releases are cut.
///
/// # Errors
///
/// Any database error.
pub async fn list(
    scoped: &mut Scoped<'_>,
    project_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<ReleaseRow>, sqlx::Error> {
    let tuples: Vec<ReleaseTuple> = sqlx::query_as(
        "SELECT id, project_id, name, note, created_by, created_at
           FROM release
          WHERE project_id = $1
            AND ($2::uuid IS NULL OR id < $2)
          ORDER BY id DESC
          LIMIT $3",
    )
    .bind(project_id)
    .bind(after)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;
    Ok(tuples.into_iter().map(row).collect())
}

/// One release, by id.
///
/// # Errors
///
/// Any database error.
pub async fn read(
    scoped: &mut Scoped<'_>,
    release_id: Uuid,
) -> Result<Option<ReleaseRow>, sqlx::Error> {
    let tuple: Option<ReleaseTuple> = sqlx::query_as(
        "SELECT id, project_id, name, note, created_by, created_at
           FROM release WHERE id = $1",
    )
    .bind(release_id)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(tuple.map(row))
}

/// What a task looked like when it went out.
#[derive(Debug, Clone)]
pub struct ReleasedTask {
    pub task_id: Uuid,
    pub task_key: String,
    pub title: String,
    pub promoted_at: OffsetDateTime,
}

/// The tasks a release carried, in the order they were recorded.
///
/// The title is read live rather than copied at promotion time. A release says
/// which tasks went out, not what they were called that day, and a stored copy
/// would slowly become a second, wrong title nobody maintains.
///
/// # Errors
///
/// Any database error.
pub async fn contents(
    scoped: &mut Scoped<'_>,
    release_id: Uuid,
) -> Result<Vec<ReleasedTask>, sqlx::Error> {
    let tuples: Vec<(Uuid, String, String, OffsetDateTime)> = sqlx::query_as(
        "SELECT t.id, p.key || '-' || t.number, t.title, e.promoted_at
           FROM task_environment_promotion e
           JOIN task t ON t.id = e.task_id
           JOIN project p ON p.id = t.project_id
          WHERE e.release_id = $1
            AND t.deleted_at IS NULL
          ORDER BY e.id",
    )
    .bind(release_id)
    .fetch_all(scoped.conn())
    .await?;
    Ok(tuples
        .into_iter()
        .map(|tuple| ReleasedTask {
            task_id: tuple.0,
            task_key: tuple.1,
            title: tuple.2,
            promoted_at: tuple.3,
        })
        .collect())
}
