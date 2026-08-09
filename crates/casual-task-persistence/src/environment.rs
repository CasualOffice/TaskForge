//! `project_environment` — the deployment target a task is about.
//!
//! # Why this is not just another task field
//!
//! `scope_type` includes `ENVIRONMENT` (`migrations/0001`) and `docs/04`'s
//! closed constraint set includes `environment_in`, so a grant can be scoped to
//! an environment and a grant can be *narrowed* to one. That makes an
//! environment row part of the permission model: creating one creates a scope
//! somebody can be granted authority in, and deleting one silently voids every
//! grant that named it.
//!
//! Which is why deletion here works the way a status deletion does in
//! `docs/23` — the caller states where the tasks go, and the move is in the
//! same transaction as the delete. `TF-PRJ-0005` exists for exactly that.
//!
//! # Ordering
//!
//! `position` rather than name, for the same reason a workflow's statuses have
//! one: `dev → staging → production` is a pipeline, and alphabetical order puts
//! production in the middle of it.

use uuid::Uuid;

use crate::scoped::Scoped;

/// A `project_environment` row.
#[derive(Debug, Clone)]
pub struct EnvironmentRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub position: i32,
}

/// A name collision on `UNIQUE (project_id, name)`.
#[derive(Debug)]
pub enum WriteError {
    Duplicate,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for WriteError {
    fn from(error: sqlx::Error) -> Self {
        match &error {
            sqlx::Error::Database(db) if db.is_unique_violation() => Self::Duplicate,
            _ => Self::Db(error),
        }
    }
}

/// Every environment of one project, in pipeline order.
///
/// Not paged. `docs/21` bounds every list, and this one is bounded by
/// [`MAX_PER_PROJECT`] at the write end instead — a keyset cursor over at most
/// fifty rows would be ceremony that the settings screen then has to implement.
///
/// # Errors
///
/// Any database error.
pub async fn list(
    scoped: &mut Scoped<'_>,
    project: Uuid,
) -> Result<Vec<EnvironmentRow>, sqlx::Error> {
    let rows: Vec<(Uuid, Uuid, String, i32)> = sqlx::query_as(
        "SELECT id, project_id, name, position
           FROM project_environment
          WHERE project_id = $1
       ORDER BY position, name",
    )
    .bind(project)
    .fetch_all(scoped.conn())
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, project_id, name, position)| EnvironmentRow {
            id,
            project_id,
            name,
            position,
        })
        .collect())
}

/// The most environments one project may have.
///
/// A bound, not a guess: `docs/21` requires every list to have one, and an
/// environment is a *grant scope* — an unbounded set of them is an unbounded
/// set of scopes the permission resolver has to consider.
pub const MAX_PER_PROJECT: i64 = 50;

/// One environment, or `None` — including when it belongs to another tenant,
/// which row-level security makes indistinguishable from absent.
///
/// # Errors
///
/// Any database error.
pub async fn read(
    scoped: &mut Scoped<'_>,
    environment: Uuid,
) -> Result<Option<EnvironmentRow>, sqlx::Error> {
    let row: Option<(Uuid, Uuid, String, i32)> = sqlx::query_as(
        "SELECT id, project_id, name, position FROM project_environment WHERE id = $1",
    )
    .bind(environment)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(row.map(|(id, project_id, name, position)| EnvironmentRow {
        id,
        project_id,
        name,
        position,
    }))
}

/// Append an environment to a project.
///
/// # Errors
///
/// [`WriteError::Duplicate`] when the name is taken in this project.
pub async fn insert(
    scoped: &mut Scoped<'_>,
    project: Uuid,
    name: &str,
) -> Result<EnvironmentRow, WriteError> {
    let workspace = scoped.workspace_id().as_uuid();
    let id = Uuid::now_v7();
    let row: (Uuid, Uuid, String, i32) = sqlx::query_as(
        "INSERT INTO project_environment (id, project_id, workspace_id, name, position)
         SELECT $1, $2, $3, $4, COALESCE(max(position), 0) + 1
           FROM project_environment WHERE project_id = $2
      RETURNING id, project_id, name, position",
    )
    .bind(id)
    .bind(project)
    .bind(workspace)
    .bind(name)
    .fetch_one(scoped.conn())
    .await?;
    Ok(EnvironmentRow {
        id: row.0,
        project_id: row.1,
        name: row.2,
        position: row.3,
    })
}

/// Rename an environment. Returns `false` when it does not exist here.
///
/// # Errors
///
/// [`WriteError::Duplicate`] when the name is taken in this project.
pub async fn rename(
    scoped: &mut Scoped<'_>,
    environment: Uuid,
    name: &str,
) -> Result<bool, WriteError> {
    let affected = sqlx::query("UPDATE project_environment SET name = $2 WHERE id = $1")
        .bind(environment)
        .bind(name)
        .execute(scoped.conn())
        .await?
        .rows_affected();
    Ok(affected == 1)
}

/// How many tasks name this environment.
///
/// Served by `task_env_ix` — `(project_id, environment_id)`, which is why this
/// takes the project as well as the environment even though the environment id
/// alone is unique. Without the leading column the predicate scans `task`.
///
/// # Errors
///
/// Any database error.
pub async fn count_tasks_on(
    scoped: &mut Scoped<'_>,
    project: Uuid,
    environment: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM task WHERE project_id = $1 AND environment_id = $2")
        .bind(project)
        .bind(environment)
        .fetch_one(scoped.conn())
        .await
}

/// How many environments this project already has.
///
/// # Errors
///
/// Any database error.
pub async fn count_in(scoped: &mut Scoped<'_>, project: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM project_environment WHERE project_id = $1")
        .bind(project)
        .fetch_one(scoped.conn())
        .await
}

/// Move every task off `from` onto `to` (or to no environment), then delete it.
///
/// `to` of `None` is "clear it", which is a real answer here and is not one for
/// a workflow status: `task.environment_id` is nullable and `task.status_id` is
/// not. The caller must still say so explicitly — `TF-PRJ-0005` refuses a
/// delete that named no target at all, because "untag 4,000 tasks" is a
/// decision and not a default.
///
/// Returns how many tasks moved.
///
/// # Errors
///
/// Any database error.
pub async fn delete_with_migration(
    scoped: &mut Scoped<'_>,
    environment: Uuid,
    to: Option<Uuid>,
    actor: Uuid,
) -> Result<u64, sqlx::Error> {
    let moved = sqlx::query(
        "UPDATE task
            SET environment_id = $2,
                version = version + 1,
                updated_at = now(),
                updated_by = $3
          WHERE environment_id = $1",
    )
    .bind(environment)
    .bind(to)
    .bind(actor)
    .execute(scoped.conn())
    .await?
    .rows_affected();

    sqlx::query("DELETE FROM project_environment WHERE id = $1")
        .bind(environment)
        .execute(scoped.conn())
        .await?;
    Ok(moved)
}

/// Set (or clear) a task's environment, guarded by its version.
///
/// Returns the new version, or `None` when `expected` is stale — `docs/24`:
/// "0 rows affected ⇒ someone else wrote first ⇒ 409".
///
/// The environment is **not** written through `PATCH /tasks/{id}`'s general
/// field path: it is a foreign key into a per-project table, so setting it
/// needs the target project checked against the task's own, and folding that
/// into the generic patch would put a cross-table check in a handler that has
/// no reason to load a project.
///
/// # Errors
///
/// Any database error.
pub async fn set_on_task(
    scoped: &mut Scoped<'_>,
    task: Uuid,
    environment: Option<Uuid>,
    expected: i64,
    actor: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "UPDATE task
            SET environment_id = $2,
                version = version + 1,
                updated_at = now(),
                updated_by = $3
          WHERE id = $1 AND version = $4 AND deleted_at IS NULL
      RETURNING version",
    )
    .bind(task)
    .bind(environment)
    .bind(actor)
    .bind(expected)
    .fetch_optional(scoped.conn())
    .await
}
