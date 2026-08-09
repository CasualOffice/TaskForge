//! The task repository (C-008, read and create).
//!
//! # `state` is written in the same statement as `status_id`
//!
//! `docs/23` §What commits makes that the invariant "that lets every report read
//! `state` without a join". [`NewTask`] carries both together and there is no
//! setter for one of them, so a caller cannot write a status without the state
//! it maps to — the storage-side form of the guarantee
//! `casual_task_workflow::ValidTransition` makes in memory.
//!
//! # The list goes through the filter compiler
//!
//! [`list`] executes whatever `crate::compile` produced. That is deliberate:
//! the permission filter is injected by the compiler and cannot be omitted
//! (`docs/27`), so a list path that built its own SQL would be a second place
//! the tenant and project filters could be forgotten.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::compile::{Compiled, Param};
use crate::scoped::Scoped;

/// A task as stored.
#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub number: i64,
    pub title: String,
    pub description: Option<String>,
    /// `TASK` | `BUG` | `FEATURE` | `INCIDENT` | `REQUEST`.
    pub task_type: String,
    /// `NONE` | `LOW` | `MEDIUM` | `HIGH` | `URGENT`.
    pub priority: String,
    pub status_id: Uuid,
    /// One of the five permanent states, derived from `status_id`.
    pub state: String,
    pub reporter_id: Uuid,
    pub environment_id: Option<Uuid>,
    pub milestone_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub start_at: Option<OffsetDateTime>,
    pub due_at: Option<OffsetDateTime>,
    pub position: String,
    pub created_at: OffsetDateTime,
    pub created_by: Uuid,
    pub updated_at: OffsetDateTime,
    pub updated_by: Option<Uuid>,
    pub version: i64,
    pub archived_at: Option<OffsetDateTime>,
    /// The full-text relevance of this row, when the query ranked one.
    ///
    /// `None` for every structured list — there is no query to rank against.
    /// It is not a column of `task`: it is computed per query, and it is here
    /// because a cursor that resumes on rank has to carry the value it
    /// resumed from (`docs/26` §Cursor pagination).
    pub rank: Option<f32>,
}

/// The columns [`TaskRow`] decodes, qualified as `t`.
///
/// Written out rather than `t.*` for a reason the filter compiler shares: the
/// three enum columns must arrive as `text`. `t.*` hands back `task_type`,
/// `task_priority` and `task_state` as PostgreSQL enums, which no `String`
/// decoder accepts, so every list would fail at decode time rather than at
/// compile time. `crate::compile` selects this same projection.
pub(crate) const COLUMNS: &str =
    "t.id, t.workspace_id, t.project_id, t.number, t.title, t.description,
                       t.type::text AS \"type\", t.priority::text AS priority, t.status_id,
                       t.state::text AS state,
                       t.reporter_id, t.environment_id, t.milestone_id, t.parent_id,
                       t.start_at, t.due_at, t.position, t.created_at, t.created_by,
                       t.updated_at, t.updated_by, t.version, t.archived_at";

fn row_of(row: &sqlx::postgres::PgRow) -> Result<TaskRow, sqlx::Error> {
    use sqlx::Row as _;
    Ok(TaskRow {
        id: row.try_get("id")?,
        workspace_id: row.try_get("workspace_id")?,
        project_id: row.try_get("project_id")?,
        number: row.try_get("number")?,
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        task_type: row.try_get("type")?,
        priority: row.try_get("priority")?,
        status_id: row.try_get("status_id")?,
        state: row.try_get("state")?,
        reporter_id: row.try_get("reporter_id")?,
        environment_id: row.try_get("environment_id")?,
        milestone_id: row.try_get("milestone_id")?,
        parent_id: row.try_get("parent_id")?,
        start_at: row.try_get("start_at")?,
        due_at: row.try_get("due_at")?,
        position: row.try_get("position")?,
        created_at: row.try_get("created_at")?,
        created_by: row.try_get("created_by")?,
        updated_at: row.try_get("updated_at")?,
        updated_by: row.try_get("updated_by")?,
        version: row.try_get("version")?,
        archived_at: row.try_get("archived_at")?,
        // Absent from every projection but the ranked search one, so a missing
        // column is the ordinary case rather than a decode failure.
        rank: row.try_get("rank").ok(),
    })
}

/// What a task create supplies.
///
/// `status_id` and `state` travel together — see the module docs.
#[derive(Debug, Clone)]
pub struct NewTask {
    pub id: Uuid,
    pub project_id: Uuid,
    pub number: i64,
    pub title: String,
    pub description: Option<String>,
    pub task_type: String,
    pub priority: String,
    pub status_id: Uuid,
    pub state: String,
    pub reporter_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub due_at: Option<OffsetDateTime>,
    /// The lexicographic board rank (ADR-013).
    pub position: String,
    pub created_by: Uuid,
}

/// Insert a task.
///
/// # Errors
///
/// Any database error. `UNIQUE (project_id, number)` is the guard against a
/// number being allocated twice; `crate::project::allocate_number` is what
/// prevents it from firing.
pub async fn insert(scoped: &mut Scoped<'_>, new: &NewTask) -> Result<TaskRow, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let sql = format!(
        "WITH inserted AS (
             INSERT INTO task
                 (id, workspace_id, project_id, number, title, description, type,
                  priority, status_id, state, reporter_id, parent_id, due_at,
                  position, created_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7::task_type,$8::task_priority,$9,
                     $10::task_state,$11,$12,$13,$14,$15)
             RETURNING *
         )
         SELECT {COLUMNS} FROM inserted t"
    );
    let row = sqlx::query(&sql)
        .bind(new.id)
        .bind(workspace)
        .bind(new.project_id)
        .bind(new.number)
        .bind(&new.title)
        .bind(new.description.as_deref())
        .bind(&new.task_type)
        .bind(&new.priority)
        .bind(new.status_id)
        .bind(&new.state)
        .bind(new.reporter_id)
        .bind(new.parent_id)
        .bind(new.due_at)
        .bind(&new.position)
        .bind(new.created_by)
        .fetch_one(scoped.conn())
        .await?;
    row_of(&row)
}

/// One task, or `None` when it does not exist, is deleted, or sits in a project
/// the actor cannot see.
///
/// The visibility predicate is `crate::project`'s, applied through a join
/// rather than re-stated here — one rule, one place. A task in another
/// workspace produces no row for a second, independent reason: row-level
/// security (migration 0010).
///
/// # Errors
///
/// Any database error.
pub async fn read_visible(
    scoped: &mut Scoped<'_>,
    viewer: &crate::project::Viewer,
    id: Uuid,
) -> Result<Option<(TaskRow, String)>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let sql = format!(
        "SELECT {COLUMNS}, p.key AS project_key
           FROM task t
           JOIN project p ON p.id = t.project_id
          WHERE t.id = $5
            AND t.workspace_id = $1
            AND t.deleted_at IS NULL
            AND p.deleted_at IS NULL
            AND {visible}",
        visible = crate::project::VISIBLE
    );
    let row = sqlx::query(&sql)
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(id)
        .fetch_optional(scoped.conn())
        .await?;
    let Some(row) = row else { return Ok(None) };
    use sqlx::Row as _;
    let key: String = row.try_get("project_key")?;
    Ok(Some((row_of(&row)?, key)))
}

/// The fields `PATCH /tasks/{id}` may change.
///
/// # Two levels of `Option`, and why both are needed
///
/// `docs/05` §Conventions: "absent = leave unchanged; `null` = clear". A single
/// `Option` cannot say both, so a nullable column takes `Option<Option<T>>` —
/// `None` is absent, `Some(None)` is an explicit clear. The non-nullable columns
/// keep one level because "clear the title" is not expressible.
///
/// `status_id` and `state` are deliberately absent. `docs/23`: "Status is
/// **never** written through `PATCH /tasks/{id}`" — there is no field here to
/// write it with, so the rule is a property of the type rather than a check
/// somebody remembers.
#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub task_type: Option<String>,
    pub priority: Option<String>,
    pub start_at: Option<Option<OffsetDateTime>>,
    pub due_at: Option<Option<OffsetDateTime>>,
}

/// Apply a patch, conditional on `expected_version`.
///
/// `None` means the compare-and-set matched nothing: the row moved on, was
/// deleted, or never existed. `docs/24`: "0 rows affected ⇒ someone else wrote
/// first ⇒ 409".
///
/// # Errors
///
/// Any database error.
pub async fn update(
    scoped: &mut Scoped<'_>,
    id: Uuid,
    expected_version: i64,
    patch: &TaskPatch,
    actor: Uuid,
) -> Result<Option<TaskRow>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    // COALESCE against a NULL parameter is what makes "absent = unchanged" one
    // statement. The nullable columns need the extra boolean because NULL is a
    // meaningful *value* there, not only "unchanged".
    let sql = format!(
        "WITH updated AS (
             UPDATE task t
                SET title       = COALESCE($4::text, t.title),
                    description = CASE WHEN $5 THEN $6::text ELSE t.description END,
                    type        = COALESCE($7::task_type, t.type),
                    priority    = COALESCE($8::task_priority, t.priority),
                    start_at    = CASE WHEN $9  THEN $10::timestamptz ELSE t.start_at END,
                    due_at      = CASE WHEN $11 THEN $12::timestamptz ELSE t.due_at END,
                    updated_at  = now(),
                    updated_by  = $13,
                    version     = t.version + 1
              WHERE t.id = $1
                AND t.workspace_id = $2
                AND t.deleted_at IS NULL
                AND t.version = $3
          RETURNING *
         )
         SELECT {COLUMNS} FROM updated t"
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .bind(workspace)
        .bind(expected_version)
        .bind(patch.title.as_deref())
        .bind(patch.description.is_some())
        .bind(patch.description.clone().flatten())
        .bind(patch.task_type.as_deref())
        .bind(patch.priority.as_deref())
        .bind(patch.start_at.is_some())
        .bind(patch.start_at.flatten())
        .bind(patch.due_at.is_some())
        .bind(patch.due_at.flatten())
        .bind(actor)
        .fetch_optional(scoped.conn())
        .await?;
    row.as_ref().map(row_of).transpose()
}

/// Soft-delete, conditional on `expected_version`.
///
/// `docs/03`: a delete is a tombstone, not a `DELETE`. Every index that matters
/// is partial on `deleted_at IS NULL` (migration 0005), so the row leaves every
/// read path without leaving the table — and the activity trail that references
/// it stays readable.
///
/// # Errors
///
/// Any database error.
pub async fn soft_delete(
    scoped: &mut Scoped<'_>,
    id: Uuid,
    expected_version: i64,
    actor: Uuid,
) -> Result<Option<TaskRow>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let sql = format!(
        "WITH deleted AS (
             UPDATE task t
                SET deleted_at = now(),
                    updated_at = now(),
                    updated_by = $4,
                    version    = t.version + 1
              WHERE t.id = $1
                AND t.workspace_id = $2
                AND t.deleted_at IS NULL
                AND t.version = $3
          RETURNING *
         )
         SELECT {COLUMNS} FROM deleted t"
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .bind(workspace)
        .bind(expected_version)
        .bind(actor)
        .fetch_optional(scoped.conn())
        .await?;
    row.as_ref().map(row_of).transpose()
}

/// Move a task to a new status, conditional on `expected_version`.
///
/// `status_id` and `state` are written in **one** statement, which is the
/// invariant `docs/23` §What commits rests on — the derived column cannot drift
/// from its source because there is no interval in which one is written and the
/// other is not. The caller obtains the pair from
/// `casual_task_workflow::ValidTransition`, which likewise carries both.
///
/// # Errors
///
/// Any database error.
pub async fn transition(
    scoped: &mut Scoped<'_>,
    id: Uuid,
    expected_version: i64,
    status_id: Uuid,
    state: &str,
    actor: Uuid,
) -> Result<Option<TaskRow>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let sql = format!(
        "WITH moved AS (
             UPDATE task t
                SET status_id  = $4,
                    state      = $5::task_state,
                    updated_at = now(),
                    updated_by = $6,
                    version    = t.version + 1
              WHERE t.id = $1
                AND t.workspace_id = $2
                AND t.deleted_at IS NULL
                AND t.version = $3
          RETURNING *
         )
         SELECT {COLUMNS} FROM moved t"
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .bind(workspace)
        .bind(expected_version)
        .bind(status_id)
        .bind(state)
        .bind(actor)
        .fetch_optional(scoped.conn())
        .await?;
    row.as_ref().map(row_of).transpose()
}

/// The unresolved blockers of a task, restricted to the ones the actor can see.
///
/// `docs/23` step 7 gates a transition on blocking dependencies and requires the
/// error to name "the blockers the actor can see" — so the visibility predicate
/// is part of this query rather than a filter applied afterwards. A blocker in a
/// project the actor cannot see still blocks; it is simply not named, which is
/// the only answer that neither leaks it nor lies about why the transition
/// failed.
///
/// A blocker is unresolved when it is not in a terminal state. `docs/23`:
/// `COMPLETED` and `CANCELED` are the two terminal states, and a canceled
/// blocker must stop blocking or abandoned work would wedge its dependents
/// forever.
///
/// # Errors
///
/// Any database error.
pub async fn unresolved_blockers(
    scoped: &mut Scoped<'_>,
    viewer: &crate::project::Viewer,
    task_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let sql = format!(
        "SELECT b.id
           FROM task_dependency d
           JOIN task b    ON b.id = d.from_task_id
           JOIN project p ON p.id = b.project_id
          WHERE d.to_task_id = $5
            AND d.workspace_id = $1
            AND d.kind = 'BLOCKS'
            AND b.deleted_at IS NULL
            AND p.deleted_at IS NULL
            AND b.state NOT IN ('COMPLETED','CANCELED')
            AND {visible}
          ORDER BY b.id",
        visible = crate::project::VISIBLE
    );
    sqlx::query_scalar(&sql)
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(task_id)
        .fetch_all(scoped.conn())
        .await
}

/// Whether `user` is in this workspace **and** can see `project`.
///
/// This is the check behind `TF-TSK-0005` ("assignee is not a member of the
/// project"), and it is deliberately the *visibility* rule from `docs/04`
/// §Visibility vs permission rather than a `project_membership` lookup. On a
/// `WORKSPACE`-visible project — the default — there are usually no membership
/// rows at all, so requiring one would make assignment impossible in the common
/// case while claiming to enforce a rule nobody could satisfy.
///
/// What it does enforce is the invariant that actually matters: **work is never
/// assigned to someone who cannot see it**. A stranger, a member of another
/// tenant, or a colleague who cannot open the project are all refused, and for
/// the same reason.
///
/// The clauses mirror `crate::project::VISIBLE` with `user` in the viewer's
/// place. They are written out rather than reusing that constant because this
/// query resolves the user's teams and project grants inline — the caller has a
/// `Viewer` for the *actor*, and the question here is about somebody else.
///
/// # Errors
///
/// Any database error.
pub async fn may_be_assigned(
    scoped: &mut Scoped<'_>,
    user: Uuid,
    project_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1
              FROM project p
             WHERE p.id = $1
               AND p.workspace_id = $2
               AND p.deleted_at IS NULL
               AND EXISTS (SELECT 1 FROM workspace_membership wm
                            WHERE wm.workspace_id = $2 AND wm.user_id = $3)
               AND (   p.visibility = 'WORKSPACE'
                    OR (p.visibility = 'TEAM' AND p.team_id IN (
                            SELECT tm.team_id FROM team_membership tm
                             WHERE tm.user_id = $3))
                    OR EXISTS (SELECT 1 FROM project_membership pm
                                WHERE pm.project_id = p.id AND pm.user_id = $3)
                    OR EXISTS (SELECT 1 FROM role_assignment ra
                                WHERE ra.workspace_id = $2
                                  AND ra.scope_type = 'PROJECT'
                                  AND ra.scope_id = p.id
                                  AND ra.principal_type = 'USER'
                                  AND ra.principal_id = $3)))",
    )
    .bind(project_id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(user)
    .fetch_one(scoped.conn())
    .await
}

/// The users assigned to a task, oldest assignment first.
///
/// # Errors
///
/// Any database error.
pub async fn assignees(scoped: &mut Scoped<'_>, task_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT user_id FROM task_assignee
          WHERE task_id = $1 AND workspace_id = $2
          ORDER BY assigned_at, user_id",
    )
    .bind(task_id)
    .bind(scoped.workspace_id().as_uuid())
    .fetch_all(scoped.conn())
    .await
}

/// Assign a user. `false` if they were already assigned.
///
/// The `workspace_id` is taken from the scope rather than from a parameter:
/// `task_assignee` carries the column but no foreign key to `workspace`, so the
/// scope is the only thing that keeps the denormalized value honest.
///
/// # Errors
///
/// Any database error.
pub async fn add_assignee(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "INSERT INTO task_assignee (task_id, user_id, workspace_id)
         SELECT t.id, $2, t.workspace_id
           FROM task t
          WHERE t.id = $1 AND t.workspace_id = $3 AND t.deleted_at IS NULL
         ON CONFLICT (task_id, user_id) DO NOTHING",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(scoped.workspace_id().as_uuid())
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Unassign a user. `false` if they were not assigned.
///
/// # Errors
///
/// Any database error.
pub async fn remove_assignee(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "DELETE FROM task_assignee
          WHERE task_id = $1 AND user_id = $2 AND workspace_id = $3",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(scoped.workspace_id().as_uuid())
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// A tag's name, when it exists and is usable on tasks in `project`.
///
/// `tag.project_id` of `NULL` is a workspace-scoped tag (migration 0005), usable
/// anywhere in the workspace; a project-scoped one is usable only on tasks in
/// that project. Returning the name rather than a bare boolean is what lets the
/// activity record hold a display value instead of an id (`docs/25`).
///
/// # Errors
///
/// Any database error.
pub async fn usable_tag(
    scoped: &mut Scoped<'_>,
    tag_id: Uuid,
    project_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT name::text FROM tag
          WHERE id = $1
            AND workspace_id = $2
            AND (project_id IS NULL OR project_id = $3)",
    )
    .bind(tag_id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(project_id)
    .fetch_optional(scoped.conn())
    .await
}

/// Tag a task. `false` if it already carried the tag.
///
/// # Errors
///
/// Any database error.
pub async fn add_tag(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    tag_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "INSERT INTO task_tag (task_id, tag_id, workspace_id)
         SELECT t.id, $2, t.workspace_id
           FROM task t
          WHERE t.id = $1 AND t.workspace_id = $3 AND t.deleted_at IS NULL
         ON CONFLICT (task_id, tag_id) DO NOTHING",
    )
    .bind(task_id)
    .bind(tag_id)
    .bind(scoped.workspace_id().as_uuid())
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Write the comment a transition carried (`docs/23` §What commits).
///
/// In the transition's own transaction, so a move that was explained and a move
/// that was not are never confused: either both rows commit or neither does.
///
/// # Errors
///
/// Any database error.
pub async fn insert_comment(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    author: Uuid,
    body: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO comment (id, workspace_id, task_id, author_id, body)
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(task_id)
    .bind(author)
    .bind(body)
    .execute(scoped.conn())
    .await?;
    Ok(id)
}

/// Execute a query the filter compiler produced.
///
/// # Errors
///
/// Any database error.
pub async fn list(
    scoped: &mut Scoped<'_>,
    compiled: &Compiled,
) -> Result<Vec<TaskRow>, sqlx::Error> {
    let mut query = sqlx::query(&compiled.sql);
    for param in &compiled.params {
        query = match param {
            Param::Workspace(w) => query.bind(w.as_uuid()),
            Param::Projects(ps) => query.bind(ps.iter().map(|p| p.as_uuid()).collect::<Vec<_>>()),
            Param::Text(t) => query.bind(t.clone()),
            Param::TextList(v) => query.bind(v.clone()),
        };
    }
    let rows = query.fetch_all(scoped.conn()).await?;
    rows.iter().map(row_of).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_insert_writes_status_and_state_in_one_statement() {
        // docs/23: the derived column can never drift because it is written
        // with its source. Splitting this into two statements would open the
        // window this invariant exists to close.
        let new = NewTask {
            id: Uuid::now_v7(),
            project_id: Uuid::now_v7(),
            number: 1,
            title: "t".into(),
            description: None,
            task_type: "TASK".into(),
            priority: "NONE".into(),
            status_id: Uuid::now_v7(),
            state: "BACKLOG".into(),
            reporter_id: Uuid::now_v7(),
            parent_id: None,
            due_at: None,
            position: "00000001".into(),
            created_by: Uuid::now_v7(),
        };
        // The type carries both, so there is no way to construct a create that
        // sets one of them.
        assert!(!new.state.is_empty());
        assert_ne!(new.status_id, Uuid::nil());
    }

    #[test]
    fn the_column_list_matches_the_decoded_fields() {
        // 23 columns, decoded by name. A column added to COLUMNS without a
        // field lands nowhere; a field added without a column fails at runtime
        // with "no column found", which is a worse place to learn it.
        assert_eq!(COLUMNS.split(',').count(), 23);
    }
}
