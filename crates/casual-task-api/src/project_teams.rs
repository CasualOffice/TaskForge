//! `/api/v1/projects/{id}/teams` — which teams a project involves.
//!
//! # The failure this module prevents
//!
//! Adding a team to a project hands every member of that team, and every grant
//! scoped to it, reach into the project's tasks. Removing one takes that reach
//! away. Both are **authorization changes**, and `docs/03` §Teams on a project
//! says so in as many words: they bump `workspace.authz_epoch` so open SSE
//! streams revalidate instead of continuing to deliver on stale authority.
//!
//! Kept out of [`crate::projects`] because the two change for different
//! reasons: that module changes when a project's own fields or its
//! representation do, this one when the reach of a team grant does. A file that
//! held both would let a routine field addition sit beside the epoch bump, and
//! the epoch bump is the line whose omission nobody notices until a revoked
//! reader keeps receiving events.
//!
//! # Authority
//!
//! `project.member.manage` — the existing permission for "who belongs to this
//! project", which is exactly the question a team on a project answers. No new
//! permission key: `docs/17` makes a new user-facing noun an ADR trigger, and
//! there is no new noun here.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_model::{ProjectId, permission};
use casual_task_persistence::project::ProjectRow;
use casual_task_persistence::{Change, Scoped, UnitOfWork, project, project_team, workspace};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::wire::{Body, Page, Paged};
use crate::{unit, wire};

/// One team on a project.
#[derive(Debug, Serialize)]
pub struct ProjectTeamView {
    pub team_id: Uuid,
    pub name: String,
    pub added_at: String,
    pub added_by: Option<Uuid>,
}

/// `POST /api/v1/projects/{id}/teams`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddTeamRequest {
    pub team_id: Uuid,
}

/// `GET /api/v1/projects/{id}/teams`.
///
/// Not paginated by cursor: the set is bounded by the number of teams in a
/// workspace and is read whole by every caller. `docs/05`'s envelope is kept so
/// a client does not have to special-case one collection, and `has_more` is
/// always `false` because there is never a second page.
///
/// # Errors
///
/// `404` when the project does not exist or the caller cannot see it.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let row = visible(&mut scoped, &ctx, project_id, &request_id).await?;
    let teams = project_team::list(&mut scoped, row.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "listing the project's teams failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    Ok(axum::Json(Paged {
        data: teams
            .iter()
            .map(|t| ProjectTeamView {
                team_id: t.team_id,
                name: t.name.clone(),
                added_at: wire::timestamp(t.added_at),
                added_by: t.added_by,
            })
            .collect::<Vec<_>>(),
        page: Page {
            next_cursor: None,
            has_more: false,
        },
    })
    .into_response())
}

/// `POST /api/v1/projects/{id}/teams`.
///
/// `201` when the team was added, `200` when it was already on the project.
/// Re-adding is not an error — it is the state the caller asked for — but it
/// writes no history and bumps no epoch, because nothing about who can reach
/// this project changed.
///
/// # Errors
///
/// `404` for an invisible project, `403` without `project.member.manage`, `422`
/// when `team_id` does not name a team in this workspace.
pub async fn add(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Body(body): Body<AddTeamRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let row = visible(&mut scoped, &ctx, project_id, &request_id).await?;
    authorize(&mut scoped, &ctx, &row, &request_id).await?;

    // Asked before the insert so a team from another tenant is a 422 naming the
    // problem, rather than an insert that matches no row and reports success.
    if !project_team::team_exists(&mut scoped, body.team_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the team failed");
            ApiError::internal(&request_id)
        })?
    {
        return Err(ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "team_id does not name a team in this workspace",
            &request_id,
        ));
    }

    let added = project_team::add(&mut scoped, row.id, body.team_id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "adding the team to the project failed");
            ApiError::internal(&request_id)
        })?;

    if added {
        bump(&state, &mut scoped, &request_id).await?;
        record(
            &mut scoped,
            &ctx,
            &row,
            "project.team.added",
            body.team_id,
            &request_id,
        )
        .await?;
    }
    unit::commit(tx, &request_id).await?;

    Ok(if added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    }
    .into_response())
}

/// `DELETE /api/v1/projects/{id}/teams/{team_id}`.
///
/// `204` when the team was on the project, `404` when it was not — the same
/// answer a team that never existed gets, because the caller has no business
/// distinguishing them and neither answer is actionable.
///
/// # Errors
///
/// `404` for an invisible project or a team not on it, `403` without
/// `project.member.manage`.
pub async fn remove(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path((project_id, team_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let row = visible(&mut scoped, &ctx, project_id, &request_id).await?;
    authorize(&mut scoped, &ctx, &row, &request_id).await?;

    if !project_team::remove(&mut scoped, row.id, team_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "removing the team from the project failed");
            ApiError::internal(&request_id)
        })?
    {
        return Err(ApiError::not_found(&request_id));
    }

    // Removing reach is the direction that matters. `docs/03`: "Removing a team
    // from a project removes reach, and that is the point." The bump and the
    // delete are in ONE transaction, so there is no interval in which a stream
    // has been told nothing changed and the row is already gone.
    bump(&state, &mut scoped, &request_id).await?;
    record(
        &mut scoped,
        &ctx,
        &row,
        "project.team.removed",
        team_id,
        &request_id,
    )
    .await?;
    unit::commit(tx, &request_id).await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Attach the teams a create request named, and report the ids that took.
///
/// Lives here rather than in [`crate::projects`] so there is one place that
/// turns "these teams" into rows, and the create path cannot grow a second,
/// laxer one.
///
/// # Errors
///
/// `422` when a team is not in this workspace, `500` on a database failure.
pub async fn attach_all(
    scoped: &mut Scoped<'_>,
    project: Uuid,
    actor: Uuid,
    teams: &[Uuid],
    request_id: &str,
) -> Result<Vec<Uuid>, ApiError> {
    let mut attached: Vec<Uuid> = Vec::new();
    for team in teams {
        if !project_team::team_exists(scoped, *team)
            .await
            .map_err(|error| {
                tracing::error!(%error, "reading the team failed");
                ApiError::internal(request_id)
            })?
        {
            return Err(ApiError::unprocessable(
                codes::REFERENCE_NOT_FOUND,
                "team_ids names a team that is not in this workspace",
                request_id,
            )
            .with_details(serde_json::json!({ "team_id": team })));
        }
        project_team::add(scoped, project, *team, actor)
            .await
            .map_err(|error| {
                tracing::error!(%error, "attaching a team to the project failed");
                ApiError::internal(request_id)
            })?;
        if !attached.contains(team) {
            attached.push(*team);
        }
    }
    attached.sort_unstable();
    Ok(attached)
}

/// Read a project the caller may see, or refuse with `404`.
async fn visible(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    id: Uuid,
    request_id: &str,
) -> Result<ProjectRow, ApiError> {
    project::read_visible(scoped, &ctx.viewer, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::PROJECT_NOT_FOUND, request_id))
}

/// `project.member.manage` on this project, scoped through its current teams.
///
/// The project's *existing* teams, deliberately: whether you may change the set
/// is decided by the authority you already hold over the project, never by the
/// team you are about to add. Evaluating against the incoming team would let
/// anyone with a grant on a team add that team to any project they can see.
async fn authorize(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    row: &ProjectRow,
    request_id: &str,
) -> Result<(), ApiError> {
    let is_member = project::is_member(scoped, row.id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading project membership failed");
            ApiError::internal(request_id)
        })?;
    unit::authorized(
        ctx.authority.may_in_project(
            permission::PROJECT_MEMBER_MANAGE,
            ProjectId::from_uuid(row.id),
            &row.teams(),
            &ctx.facts_in_project(is_member),
        ),
        request_id,
    )
}

/// Bump `workspace.authz_epoch` and count it (`docs/46` §Domain metrics).
async fn bump(state: &AppState, scoped: &mut Scoped<'_>, request_id: &str) -> Result<(), ApiError> {
    workspace::bump_authz_epoch(scoped).await.map_err(|error| {
        tracing::error!(%error, "bumping authz_epoch failed");
        ApiError::internal(request_id)
    })?;
    // A metric failure must never fail an authorization change.
    let _ = state.metrics.increment(
        casual_task_observability::metrics::AUTHZ_EPOCH_BUMPS_TOTAL,
        &casual_task_observability::labels::LabelSet::for_metric(
            casual_task_observability::metrics::AUTHZ_EPOCH_BUMPS_TOTAL,
        ),
        1,
    );
    Ok(())
}

/// One history record, in the caller's transaction (ADR-006).
async fn record(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    row: &ProjectRow,
    event_type: &str,
    team_id: Uuid,
    request_id: &str,
) -> Result<(), ApiError> {
    let added = event_type.ends_with("added");
    UnitOfWork::record(
        scoped,
        &Change {
            aggregate_type: "project".to_owned(),
            aggregate_id: row.id,
            project_id: Some(row.id),
            event_type: event_type.to_owned(),
            // Display values, not ids (`docs/25`): the stream is rendered years
            // later and must still read correctly.
            activity_changes: serde_json::json!({ "project": row.name, "team_id": team_id }),
            audit_changes: if added {
                serde_json::json!({ "before": null, "after": { "team_id": team_id } })
            } else {
                serde_json::json!({ "before": { "team_id": team_id }, "after": null })
            },
            payload: serde_json::json!({ "project_id": row.id, "team_id": team_id }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the project team change failed");
        ApiError::internal(request_id)
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_field_does_not_deserialize() {
        // docs/05: unknown request fields are rejected. A typo silently ignored
        // is a client bug that looks like a server bug.
        assert!(serde_json::from_str::<AddTeamRequest>(r#"{"team":"x"}"#).is_err());
    }

    #[test]
    fn both_mutations_bump_the_epoch() {
        // docs/03: adding or removing a team is an authorization change. The
        // bump is one line, and its omission is invisible until a revoked
        // reader keeps receiving events — so it is asserted over the source
        // rather than left to review.
        let source = include_str!("project_teams.rs");
        let add = source
            .split("pub async fn add(")
            .nth(1)
            .and_then(|s| s.split("pub async fn remove(").next())
            .expect("add() is defined here");
        let remove = source
            .split("pub async fn remove(")
            .nth(1)
            .and_then(|s| s.split("pub async fn attach_all(").next())
            .expect("remove() is defined here");
        assert!(add.contains("bump(&state"), "add does not bump authz_epoch");
        assert!(
            remove.contains("bump(&state"),
            "remove does not bump authz_epoch"
        );
    }
}
