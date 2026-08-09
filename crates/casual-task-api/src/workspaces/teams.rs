//! Teams and their membership.
//!
//! A team is a principal a grant can be assigned to, which is the whole reason
//! it exists. Kept apart from workspace membership because the two answer
//! different questions and share only the workspace they sit in.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_persistence::workspace as repo;
use casual_task_persistence::{Change, UnitOfWork};

use super::*;
use crate::error::{ApiError, codes};
use crate::json::ValidJson;
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};

/// `GET /api/v1/workspaces/{workspace_id}/teams`.
///
/// # Errors
///
/// [`ApiError`] on a bad page request or a database failure.
pub async fn list_teams(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    paging: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let (limit, after) = page_request_text(paging, &request_id)?;

    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = scope_of(&mut tx, &member, &request_id).await?;
    let mut found = repo::list_teams(&mut scoped, after.as_deref(), i64::from(limit) + 1)
        .await
        .map_err(|error| internal(&error, "listing teams", &request_id))?;
    commit(tx, &request_id).await?;

    let has_more = truncate(&mut found, limit);
    let next = found.last().map(|t| encode_cursor(&t.name, t.id));
    Ok(page(found.iter().map(team_body).collect(), has_more, next))
}

/// `POST /api/v1/workspaces/{workspace_id}/teams`.
///
/// # Errors
///
/// [`ApiError`] 409 for a duplicate name, 400 for a bad one, or a database
/// failure.
pub async fn create_team(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<CreateTeam>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let name = valid_name(&body.name, &request_id)?;

    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = scope_of(&mut tx, &member, &request_id).await?;

    let created = repo::insert_team(&mut scoped, name)
        .await
        .map_err(|error| {
            if unique_violation(&error) {
                ApiError::conflict(
                    codes::TEAM_NAME_TAKEN,
                    "A team with that name already exists in this workspace",
                    &request_id,
                )
            } else {
                internal(&error, "creating the team", &request_id)
            }
        })?;

    let who = provenance_member(&member, &request_id, &headers);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "team".to_owned(),
            aggregate_id: created.id,
            project_id: None,
            event_type: "team.created".to_owned(),
            activity_changes: serde_json::json!({ "name": created.name }),
            audit_changes: serde_json::json!({
                "before": serde_json::Value::Null,
                "after": { "name": created.name },
            }),
            payload: serde_json::json!({ "team_id": created.id, "name": created.name }),
            schema_version: SCHEMA_VERSION,
        },
        &who,
    )
    .await
    .map_err(|error| internal(&error, "recording the team", &request_id))?;

    commit(tx, &request_id).await?;
    Ok((StatusCode::CREATED, axum::Json(team_body(&created))).into_response())
}

/// `POST /api/v1/teams/{team_id}/members`.
///
/// The workspace comes from `X-Workspace-Id` — there is no workspace in this
/// path — and the team is then read through the policy, so a team id from
/// another tenant reads back as `None` and is answered 404 exactly like an
/// unallocated one.
///
/// # Errors
///
/// [`ApiError`] 404 for an invisible team, 422 for a user who is not a member
/// of the workspace, or a database failure.
pub async fn add_team_member(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    headers: HeaderMap,
    Path(path): Path<TeamPath>,
    ValidJson(body): ValidJson<AddTeamMember>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = scope_of(&mut tx, &member, &request_id).await?;

    let team = repo::find_team(&mut scoped, path.team_id)
        .await
        .map_err(|error| internal(&error, "reading the team", &request_id))?
        .ok_or_else(|| ApiError::not_found(&request_id))?;

    // `team_membership` carries no workspace_id and therefore no policy of its
    // own (migration 0010). This check is the tenant boundary for that table:
    // without it, any workspace member could put any user id from any tenant
    // into one of their teams, and principal expansion would then carry that
    // person's team grants.
    if !repo::is_member_scoped(&mut scoped, body.user_id)
        .await
        .map_err(|error| internal(&error, "checking workspace membership", &request_id))?
    {
        return Err(ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "That user is not a member of this workspace",
            &request_id,
        ));
    }

    let added = repo::insert_team_member(&mut scoped, team.id, body.user_id)
        .await
        .map_err(|error| internal(&error, "adding a team member", &request_id))?;

    if added {
        bump_epoch(&state, &mut scoped, &request_id).await?;
        let who = provenance_member(&member, &request_id, &headers);
        UnitOfWork::record(
            &mut scoped,
            &Change {
                aggregate_type: "team".to_owned(),
                aggregate_id: team.id,
                project_id: None,
                event_type: "team.member.added".to_owned(),
                activity_changes: serde_json::json!({
                    "team": team.name, "user_id": body.user_id,
                }),
                audit_changes: serde_json::json!({
                    "before": serde_json::Value::Null,
                    "after": { "team_id": team.id, "user_id": body.user_id },
                }),
                payload: serde_json::json!({ "team_id": team.id, "user_id": body.user_id }),
                schema_version: SCHEMA_VERSION,
            },
            &who,
        )
        .await
        .map_err(|error| internal(&error, "recording the team membership", &request_id))?;
    }

    commit(tx, &request_id).await?;
    Ok(if added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    }
    .into_response())
}

/// `DELETE /api/v1/teams/{team_id}/members/{user_id}`.
///
/// # Errors
///
/// [`ApiError`] 404 for an invisible team or a non-member, or a database
/// failure.
pub async fn remove_team_member(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    headers: HeaderMap,
    Path(path): Path<TeamMemberPath>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = scope_of(&mut tx, &member, &request_id).await?;

    let team = repo::find_team(&mut scoped, path.team_id)
        .await
        .map_err(|error| internal(&error, "reading the team", &request_id))?
        .ok_or_else(|| ApiError::not_found(&request_id))?;

    if !repo::delete_team_member(&mut scoped, team.id, path.user_id)
        .await
        .map_err(|error| internal(&error, "removing a team member", &request_id))?
    {
        return Err(ApiError::not_found(&request_id));
    }

    bump_epoch(&state, &mut scoped, &request_id).await?;
    let who = provenance_member(&member, &request_id, &headers);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "team".to_owned(),
            aggregate_id: team.id,
            project_id: None,
            event_type: "team.member.removed".to_owned(),
            activity_changes: serde_json::json!({ "team": team.name, "user_id": path.user_id }),
            audit_changes: serde_json::json!({
                "before": { "team_id": team.id, "user_id": path.user_id },
                "after": serde_json::Value::Null,
            }),
            payload: serde_json::json!({ "team_id": team.id, "user_id": path.user_id }),
            schema_version: SCHEMA_VERSION,
        },
        &who,
    )
    .await
    .map_err(|error| internal(&error, "recording the removal", &request_id))?;

    commit(tx, &request_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Path parameters
// ---------------------------------------------------------------------------
