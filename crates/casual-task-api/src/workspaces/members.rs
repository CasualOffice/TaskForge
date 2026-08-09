//! Workspace membership.
//!
//! Membership is not authority (migration 0003). These endpoints add and remove
//! the row that makes someone *visible* in a workspace; what they may then do
//! comes from `role_assignment` and nowhere else.

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

/// `GET /api/v1/workspaces/{workspace_id}/members`.
///
/// # Errors
///
/// [`ApiError`] on a bad page request or a database failure.
pub async fn list_members(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    paging: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let (limit, after) = page_request(paging, &request_id)?;

    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = scope_of(&mut tx, &member, &request_id).await?;
    let mut found = repo::list_members(&mut scoped, after, i64::from(limit) + 1)
        .await
        .map_err(|error| internal(&error, "listing members", &request_id))?;
    commit(tx, &request_id).await?;

    let has_more = truncate(&mut found, limit);
    let next = found.last().map(|m| cursor_for(m.user_id));
    Ok(page(
        found.iter().map(member_body).collect(),
        has_more,
        next,
    ))
}

/// `POST /api/v1/workspaces/{workspace_id}/members` — add a member.
///
/// Idempotent: adding someone who is already a member is `200`, not an error.
/// The client that retries a request whose response it never saw is doing the
/// right thing, and an error there would make the correct behaviour look
/// broken.
///
/// # Errors
///
/// [`ApiError`] 422 for an unknown user or an invalid member type, or a
/// database failure.
pub async fn add_member(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<AddMember>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let member_type = body.member_type.as_deref().unwrap_or("MEMBER");
    if !repo::MEMBER_TYPES.contains(&member_type) {
        return Err(
            ApiError::bad_request(codes::OUT_OF_RANGE, "Unknown member type", &request_id)
                .with_details(serde_json::json!({ "member_type": repo::MEMBER_TYPES })),
        );
    }

    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = scope_of(&mut tx, &member, &request_id).await?;

    let added = repo::insert_member(&mut scoped, body.user_id, member_type)
        .await
        .map_err(|error| {
            if foreign_key_violation(&error) {
                // `docs/05`: 422 is "valid syntax, violates a domain rule". The
                // id is well formed; it names nobody.
                ApiError::unprocessable(codes::REFERENCE_NOT_FOUND, "No such user", &request_id)
            } else {
                internal(&error, "adding a member", &request_id)
            }
        })?;

    if added {
        // docs/04 §Caching: the epoch is bumped in the same transaction as the
        // change, so a stale permission-cache entry cannot be read — the key
        // simply misses.
        bump_epoch(&state, &mut scoped, &request_id).await?;

        let who = provenance_member(&member, &request_id, &headers);
        UnitOfWork::record(
            &mut scoped,
            &Change {
                aggregate_type: "workspace".to_owned(),
                aggregate_id: member.context.scope().id().as_uuid(),
                project_id: None,
                event_type: "workspace.member.added".to_owned(),
                activity_changes: serde_json::json!({
                    "user_id": body.user_id, "member_type": member_type,
                }),
                audit_changes: serde_json::json!({
                    "before": serde_json::Value::Null,
                    "after": { "user_id": body.user_id, "member_type": member_type },
                }),
                payload: serde_json::json!({
                    "workspace_id": member.context.scope().id().as_uuid(),
                    "user_id": body.user_id,
                    "member_type": member_type,
                }),
                schema_version: SCHEMA_VERSION,
            },
            &who,
        )
        .await
        .map_err(|error| internal(&error, "recording the membership", &request_id))?;
    }

    let status = if added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    // Read back rather than echo: the display name and join time come from the
    // database, and a caller adding an existing member gets the row as it is,
    // not as they asked for it.
    let found = repo::list_members(&mut scoped, previous_uuid(body.user_id), 1)
        .await
        .map_err(|error| internal(&error, "reading the member back", &request_id))?;
    let representation = found
        .first()
        .filter(|m| m.user_id == body.user_id)
        .map(member_body);
    commit(tx, &request_id).await?;

    representation.map_or_else(
        || {
            Err(internal_message(
                "the member vanished mid-transaction",
                &request_id,
            ))
        },
        |body| Ok((status, axum::Json(body)).into_response()),
    )
}

/// `DELETE /api/v1/workspaces/{workspace_id}/members/{user_id}` — remove.
///
/// # The last member is protected
///
/// A workspace with no members is unreachable forever: nothing can see it, so
/// nothing can add a member back to it. Refusing the removal is the only
/// outcome that does not silently destroy data, and it is decided under the
/// workspace row's write lock so two concurrent removals cannot each believe
/// they are not the last (`docs/04` control 4: inside the transaction, not
/// beside it).
///
/// # Errors
///
/// [`ApiError`] 404 if the target is not a member, 422 if they are the last
/// one, or a database failure.
pub async fn remove_member(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    headers: HeaderMap,
    Path(path): Path<MemberPath>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = scope_of(&mut tx, &member, &request_id).await?;

    repo::lock(&mut scoped)
        .await
        .map_err(|error| internal(&error, "locking the workspace", &request_id))?;

    let count = repo::member_count(&mut scoped)
        .await
        .map_err(|error| internal(&error, "counting members", &request_id))?;

    let members = repo::list_members(&mut scoped, previous_uuid(path.user_id), 1)
        .await
        .map_err(|error| internal(&error, "reading the member", &request_id))?;
    let target = members
        .into_iter()
        .find(|m| m.user_id == path.user_id)
        .ok_or_else(|| ApiError::not_found(&request_id))?;

    if count <= 1 {
        return Err(ApiError::unprocessable(
            codes::LAST_MEMBER,
            "A workspace cannot lose its last member",
            &request_id,
        ));
    }

    if !repo::delete_member(&mut scoped, path.user_id)
        .await
        .map_err(|error| internal(&error, "removing a member", &request_id))?
    {
        return Err(ApiError::not_found(&request_id));
    }

    bump_epoch(&state, &mut scoped, &request_id).await?;

    let who = provenance_member(&member, &request_id, &headers);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "workspace".to_owned(),
            aggregate_id: member.context.scope().id().as_uuid(),
            project_id: None,
            event_type: "workspace.member.removed".to_owned(),
            activity_changes: serde_json::json!({ "user_id": target.user_id }),
            audit_changes: serde_json::json!({
                "before": { "user_id": target.user_id, "member_type": target.member_type },
                "after": serde_json::Value::Null,
            }),
            payload: serde_json::json!({
                "workspace_id": member.context.scope().id().as_uuid(),
                "user_id": target.user_id,
            }),
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
// Handlers — teams
// ---------------------------------------------------------------------------
