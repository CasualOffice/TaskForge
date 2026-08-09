//! Which teams a project involves (`docs/03` §"Teams on a project — many, not
//! one").
//!
//! # Why this is not in `crate::project`
//!
//! The two change for different reasons. `project` changes when a project's own
//! fields or its visibility rule do; this changes when the *association* between
//! projects and teams does — and that association is an authorization edge, not
//! a project attribute. Keeping them apart is what stops a routine column
//! addition landing in the same file as the reach of a team grant.
//!
//! # Every write here is an authorization change
//!
//! `docs/03`: "Removing a team from a project removes reach, and that is the
//! point. It is an authorization change, so it bumps `workspace.authz_epoch`
//! like any other, and open SSE streams revalidate against it."
//!
//! The bump is **not** done here, deliberately. `docs/04` makes the epoch a
//! workspace-level counter and `crate::workspace::bump_authz_epoch` the single
//! write that moves it; issuing a second one from this module would mean two
//! places that must agree about what a bump is. What this module does instead is
//! report whether anything actually changed, so the caller bumps exactly when
//! reach moved and not on a no-op re-add.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::project::{ProjectCursor, ProjectRow, VISIBLE, Viewer, row_of};
use crate::scoped::Scoped;

/// A team on a project, with when and by whom it was added.
#[derive(Debug, Clone)]
pub struct ProjectTeamRow {
    pub team_id: Uuid,
    pub name: String,
    pub added_at: OffsetDateTime,
    pub added_by: Option<Uuid>,
}

/// The teams on `project`, by name.
///
/// Joined to `team` rather than returning bare ids: every caller renders a name,
/// and resolving it here is one query instead of one per team.
///
/// # Errors
///
/// Any database error.
pub async fn list(
    scoped: &mut Scoped<'_>,
    project: Uuid,
) -> Result<Vec<ProjectTeamRow>, sqlx::Error> {
    let rows: Vec<(Uuid, String, OffsetDateTime, Option<Uuid>)> = sqlx::query_as(
        "SELECT t.id, t.name, pt.added_at, pt.added_by
           FROM project_team pt
           JOIN team t ON t.id = pt.team_id
          WHERE pt.project_id = $1 AND pt.workspace_id = $2
          ORDER BY t.name, t.id",
    )
    .bind(project)
    .bind(scoped.workspace_id().as_uuid())
    .fetch_all(scoped.conn())
    .await?;
    Ok(rows
        .into_iter()
        .map(|(team_id, name, added_at, added_by)| ProjectTeamRow {
            team_id,
            name,
            added_at,
            added_by,
        })
        .collect())
}

/// Put `team` on `project`. `false` when it was already there.
///
/// `workspace_id` comes from the scope, never from an argument — the same rule
/// every write in this crate follows, so the row written and the RLS policy
/// enforced cannot disagree.
///
/// The `SELECT` in the `INSERT` is the tenant check, not decoration: it inserts
/// only when the project and the team are both in *this* workspace, so a team id
/// from another tenant writes no row rather than writing a cross-tenant edge
/// that RLS would then happily read back.
///
/// # Errors
///
/// Any database error.
pub async fn add(
    scoped: &mut Scoped<'_>,
    project: Uuid,
    team: Uuid,
    actor: Uuid,
) -> Result<bool, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let affected = sqlx::query(
        "INSERT INTO project_team (workspace_id, project_id, team_id, added_by)
         SELECT $1, p.id, t.id, $4
           FROM project p
           JOIN team t ON t.workspace_id = p.workspace_id
          WHERE p.id = $2 AND t.id = $3
            AND p.workspace_id = $1 AND p.deleted_at IS NULL
         ON CONFLICT (project_id, team_id) DO NOTHING",
    )
    .bind(workspace)
    .bind(project)
    .bind(team)
    .bind(actor)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Take `team` off `project`. `false` when it was not on it.
///
/// # Errors
///
/// Any database error.
pub async fn remove(
    scoped: &mut Scoped<'_>,
    project: Uuid,
    team: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "DELETE FROM project_team
          WHERE project_id = $1 AND team_id = $2 AND workspace_id = $3",
    )
    .bind(project)
    .bind(team)
    .bind(scoped.workspace_id().as_uuid())
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Whether `team` names a team in this workspace.
///
/// Answered separately from [`add`] so a request naming a team that does not
/// exist gets `422 TF-CMN-0006` rather than the `200` a silent zero-row insert
/// would produce.
///
/// # Errors
///
/// Any database error.
pub async fn team_exists(scoped: &mut Scoped<'_>, team: Uuid) -> Result<bool, sqlx::Error> {
    let found: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM team WHERE id = $1 AND workspace_id = $2")
            .bind(team)
            .bind(scoped.workspace_id().as_uuid())
            .fetch_optional(scoped.conn())
            .await?;
    Ok(found.is_some())
}

/// One page of the projects `team` works on that `viewer` can see.
///
/// The team view's centre panel. `VISIBLE` is joined on rather than applied
/// afterwards for the reason `crate::project` gives: post-filtering an
/// authorized page "silently shrinks pages and breaks cursors".
///
/// Being on the team does **not** by itself make a project on that team visible
/// — `PRIVATE` projects stay private, and the predicate decides. A team view
/// that listed projects the viewer cannot open would be a membership oracle.
///
/// # Errors
///
/// Any database error.
pub async fn projects_of_team(
    scoped: &mut Scoped<'_>,
    viewer: &Viewer,
    team: Uuid,
    after: Option<ProjectCursor>,
    limit: u32,
) -> Result<Vec<ProjectRow>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let columns = crate::project::COLUMNS;
    let sql = format!(
        "SELECT {columns}
           FROM project p
           JOIN project_team pt ON pt.project_id = p.id AND pt.workspace_id = p.workspace_id
          WHERE p.workspace_id = $1
            AND pt.team_id = $8
            AND p.deleted_at IS NULL
            AND {VISIBLE}
            AND ($5::timestamptz IS NULL
                 OR (p.created_at, p.id) < ($5::timestamptz, $6::uuid))
          ORDER BY p.created_at DESC, p.id DESC
          LIMIT $7"
    );
    let rows: Vec<crate::project::ProjectTuple> = sqlx::query_as(&sql)
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(after.map(|c| c.0))
        .bind(after.map(|c| c.1))
        .bind(i64::from(limit).saturating_add(1))
        .bind(team)
        .fetch_all(scoped.conn())
        .await?;
    Ok(rows.into_iter().map(row_of).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_read_in_this_module_paginates_by_offset() {
        // docs/26 bans it outright. Asserted over this file's own source rather
        // than over a fragment, because the page here is assembled inline.
        //
        // The needle is assembled. Spelling it out would put it in the file the
        // check reads, and the assertion would fail on itself — the third time
        // that trap has been written in this codebase.
        let source = include_str!("project_team.rs");
        let banned = format!("{}{} ", "OFF", "SET");
        assert!(!source.to_uppercase().contains(&banned));
    }
}
