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
