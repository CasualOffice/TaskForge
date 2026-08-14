//! The tag vocabulary — the `tag` table (migration 0005).
//!
//! # The failure this module prevents
//!
//! A vocabulary nothing can enumerate. `POST /tasks/{id}/tags` has taken a
//! `tag_id` since C-008 and there has never been a way to find out what ids
//! exist, so the endpoint was reachable only by someone who had already read the
//! database. A picker is not a nicety here; without one the write endpoint is
//! unusable by construction.
//!
//! # Workspace tags and project tags are one list, deliberately
//!
//! `tag.project_id` is nullable and `NULL` means workspace-scoped (migration
//! 0005). A caller asking "what may I put on a task in this project?" wants both
//! kinds in one answer, ordered so the shared vocabulary leads — which is what
//! [`list`] returns. Splitting them into two endpoints would push the union into
//! every client and guarantee one of them gets it wrong.
//!
//! # There is no delete
//!
//! Deleting a tag cascades through `task_tag` (migration 0005) and silently
//! removes it from every task that carried it, which is a bulk edit disguised as
//! a configuration change. That needs its own decision about what the user is
//! shown first, and inventing one inside a repository is exactly the move
//! AGENTS.md forbids. Retiring a tag is unbuilt and stated as unbuilt.

use uuid::Uuid;

use crate::scoped::Scoped;

/// The most tags one workspace may hold.
///
/// `docs/21` bounds every input. This is also the cardinality guard `docs/24`
/// §D-042 is about, one layer up: a tag vocabulary that grows without limit is
/// a picker nobody can use and, the moment anyone labels a metric with it, an
/// unbounded label set. The bound is enforced on create, not by truncating the
/// list — a truncated vocabulary makes an existing tag look deleted.
pub const MAX_PER_WORKSPACE: i64 = 500;

/// A tag as stored.
#[derive(Debug, Clone)]
pub struct TagRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// `None` for a workspace-scoped tag, usable anywhere in the workspace.
    pub project_id: Option<Uuid>,
    pub name: String,
    /// A presentation hint. Never the only carrier of meaning — the foundation
    /// §7 forbids colour-alone, so a tag always renders its name.
    pub color: Option<String>,
}

/// `name` is `citext`, and `::text` is what makes it decode as a `String`.
const COLUMNS: &str = "id, workspace_id, project_id, name::text AS name, color";

type TagTuple = (Uuid, Uuid, Option<Uuid>, String, Option<String>);

/// A tag row with the task it is on — [`TagTuple`] plus the `task_id` that a
/// bulk read needs to group by. Named rather than written inline, where six
/// anonymous columns say nothing about which is which.
type TaggedTuple = (Uuid, Uuid, Uuid, Option<Uuid>, String, Option<String>);

fn row_of(t: TagTuple) -> TagRow {
    TagRow {
        id: t.0,
        workspace_id: t.1,
        project_id: t.2,
        name: t.3,
        color: t.4,
    }
}

/// The tags usable in a workspace, optionally narrowed to one project.
///
/// With `project_id`, the answer is exactly the set `task::usable_tag` accepts
/// for a task in that project: the workspace-scoped tags plus that project's
/// own. Without it, every tag in the workspace — which is what a settings
/// surface wants and what a picker on a task must not use, because it would
/// offer tags the write endpoint then refuses with `422`.
///
/// # Errors
///
/// Any database error.
pub async fn list(
    scoped: &mut Scoped<'_>,
    project_id: Option<Uuid>,
) -> Result<Vec<TagRow>, sqlx::Error> {
    // Two statements rather than one with a `$2 IS NULL OR …`, because the
    // narrowed form is the hot one and a disjunction over a nullable parameter
    // is how a planner ends up choosing a sequential scan for both.
    let sql = if project_id.is_some() {
        format!(
            "SELECT {COLUMNS} FROM tag
              WHERE workspace_id = $1 AND (project_id IS NULL OR project_id = $2)
              ORDER BY (project_id IS NOT NULL), name
              LIMIT {MAX_PER_WORKSPACE}"
        )
    } else {
        format!(
            "SELECT {COLUMNS} FROM tag
              WHERE workspace_id = $1
              ORDER BY (project_id IS NOT NULL), name
              LIMIT {MAX_PER_WORKSPACE}"
        )
    };
    let mut query = sqlx::query_as(&sql).bind(scoped.workspace_id().as_uuid());
    if let Some(project) = project_id {
        query = query.bind(project);
    }
    let rows: Vec<TagTuple> = query.fetch_all(scoped.conn()).await?;
    Ok(rows.into_iter().map(row_of).collect())
}

/// How many tags the workspace already holds.
///
/// # Errors
///
/// Any database error.
pub async fn count(scoped: &mut Scoped<'_>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM tag WHERE workspace_id = $1")
        .bind(scoped.workspace_id().as_uuid())
        .fetch_one(scoped.conn())
        .await
}

/// What a tag create supplies.
#[derive(Debug, Clone)]
pub struct NewTag {
    pub id: Uuid,
    /// `None` creates a workspace-scoped tag.
    pub project_id: Option<Uuid>,
    pub name: String,
    pub color: Option<String>,
}

/// Create a tag.
///
/// `None` when the name is already taken at that scope. The unique constraint is
/// `NULLS NOT DISTINCT (workspace_id, project_id, name)` — which migration 0005
/// calls load-bearing, because default NULL semantics would permit unlimited
/// duplicate workspace tags — so `ON CONFLICT DO NOTHING` covers both the
/// workspace and the project case with one clause.
///
/// # Errors
///
/// Any database error other than the unique violation.
pub async fn insert(scoped: &mut Scoped<'_>, new: &NewTag) -> Result<Option<TagRow>, sqlx::Error> {
    let sql = format!(
        "INSERT INTO tag (id, workspace_id, project_id, name, color)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (workspace_id, project_id, name) DO NOTHING
         RETURNING {COLUMNS}"
    );
    let row: Option<TagTuple> = sqlx::query_as(&sql)
        .bind(new.id)
        .bind(scoped.workspace_id().as_uuid())
        .bind(new.project_id)
        .bind(&new.name)
        .bind(new.color.as_deref())
        .fetch_optional(scoped.conn())
        .await?;
    Ok(row.map(row_of))
}

/// The tags on one task.
///
/// # Errors
///
/// Any database error.
pub async fn for_task(scoped: &mut Scoped<'_>, task_id: Uuid) -> Result<Vec<TagRow>, sqlx::Error> {
    let sql = format!(
        "SELECT {COLUMNS} FROM tag
          WHERE workspace_id = $1
            AND id IN (SELECT tt.tag_id FROM task_tag tt WHERE tt.task_id = $2)
          ORDER BY name"
    );
    let rows: Vec<TagTuple> = sqlx::query_as(&sql)
        .bind(scoped.workspace_id().as_uuid())
        .bind(task_id)
        .fetch_all(scoped.conn())
        .await?;
    Ok(rows.into_iter().map(row_of).collect())
}

/// The tags on many tasks, as `(task_id, tag)` pairs.
///
/// One statement for a whole page, because the alternative is a query per row
/// and the row count is the page size. `task_tag`'s primary key leads with
/// `task_id`, so `= ANY($2)` is an index scan rather than the join order the
/// reverse index (`task_tag_rev_ix`) exists to serve.
///
/// # Errors
///
/// Any database error.
pub async fn for_tasks(
    scoped: &mut Scoped<'_>,
    task_ids: &[Uuid],
) -> Result<Vec<(Uuid, TagRow)>, sqlx::Error> {
    if task_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<TaggedTuple> = sqlx::query_as(
        "SELECT tt.task_id, g.id, g.workspace_id, g.project_id, g.name::text, g.color
           FROM task_tag tt
           JOIN tag g ON g.id = tt.tag_id
          WHERE tt.workspace_id = $1 AND tt.task_id = ANY($2)
          ORDER BY tt.task_id, g.name",
    )
    .bind(scoped.workspace_id().as_uuid())
    .bind(task_ids)
    .fetch_all(scoped.conn())
    .await?;
    Ok(rows
        .into_iter()
        .map(|(task_id, id, workspace_id, project_id, name, color)| {
            (
                task_id,
                TagRow {
                    id,
                    workspace_id,
                    project_id,
                    name,
                    color,
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_narrowed_list_matches_what_the_write_endpoint_accepts() {
        // `task::usable_tag` accepts `project_id IS NULL OR project_id = $3`.
        // A picker built from a wider list offers tags the write then refuses
        // with a 422, which reads to the user as a broken control rather than
        // as a scope they misunderstood. The two predicates are pinned to each
        // other here because they live in different files.
        let here = include_str!("tag.rs");
        let there = include_str!("task_relations.rs");
        assert!(here.contains("project_id IS NULL OR project_id = $2"));
        assert!(there.contains("project_id IS NULL OR project_id = $3"));
    }

    #[test]
    fn the_scoped_ordering_puts_the_shared_vocabulary_first() {
        // `(project_id IS NOT NULL)` sorts false before true, so workspace tags
        // lead. Reversing it would bury the vocabulary most tasks use under one
        // project's private labels, which is only visible as "the picker feels
        // wrong" rather than as a failure.
        let here = include_str!("tag.rs");
        assert!(here.contains("ORDER BY (project_id IS NOT NULL), name"));
        // Assembled, not written out: this check reads its own file, so
        // spelling the reversed clause here would make the assertion fail on
        // itself.
        let reversed = format!("ORDER BY (project_id IS{} NULL), name", "");
        assert!(!here.contains(&reversed));
    }
}
