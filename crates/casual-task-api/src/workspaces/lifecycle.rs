//! Creating, listing, reading and updating a workspace.
//!
//! `create` is the one endpoint in the product that mints authority — it grants
//! the creator Owner in the same transaction (D-054). That is why it lives
//! beside the other lifecycle operations and not with membership: the grant is
//! part of the workspace coming into existence, not a membership change.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use casual_task_model::{AuthContext, WorkspaceId, permission};
use casual_task_persistence::workspace as repo;
use casual_task_persistence::{Change, Scoped, UnitOfWork};

use super::*;
use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::json::ValidJson;
use crate::middleware::{Authenticated, WorkspaceMember};
use crate::server::{AppState, RequestId};

/// `POST /api/v1/workspaces` — create, and make the creator a member.
///
/// # The one place an `AuthContext` is minted for a workspace nobody is yet a
/// member of
///
/// Everywhere else, membership is checked and *then* a context is minted. Here
/// the order is inverted, because the workspace does not exist until this
/// request creates it. What makes it sound is that the membership row is
/// written in the same transaction as the workspace row: either both commit, or
/// neither does, so there is no committed state in which the scope this handler
/// used was not backed by a membership.
///
/// The scope is also load-bearing rather than ceremonial. `workspace_membership`
/// carries a row-level-security policy whose `WITH CHECK` defaults to its
/// `USING` clause, so the `INSERT` below is refused outright unless
/// `taskforge.workspace_id` already names the workspace being created.
///
/// # Errors
///
/// [`ApiError`] for a taken slug (409), a bad name or slug (400), or a database
/// failure.
pub async fn create(
    State(state): State<AppState>,
    actor: Authenticated,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<CreateWorkspace>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    only_a_person(&actor, &request_id)?;
    let name = valid_name(&body.name, &request_id)?;
    let slug = valid_slug(&body.slug, &request_id)?;

    let workspace = WorkspaceId::new();
    // See the note above: minted before the membership exists, and made true by
    // the transaction that follows.
    let context = AuthContext::authenticated(actor.actor_id, workspace, actor.actor_type);
    let scope = context.scope();

    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = Scoped::apply(&mut tx, &scope)
        .await
        .map_err(|error| internal(&error, "applying the tenant scope", &request_id))?;

    let created = repo::insert(&mut scoped, name, slug)
        .await
        .map_err(|error| {
            if unique_violation(&error) {
                // 409 and not 404: the caller can see that the name is taken, which
                // is not a fact about any tenant's data — `slug` is a global
                // namespace by construction (`UNIQUE` in migration 0002).
                ApiError::conflict(
                    codes::SLUG_TAKEN,
                    "That workspace slug is already in use",
                    &request_id,
                )
            } else {
                internal(&error, "creating the workspace", &request_id)
            }
        })?;

    repo::insert_member(&mut scoped, actor.actor_id.as_uuid(), "MEMBER")
        .await
        .map_err(|error| internal(&error, "adding the creator", &request_id))?;

    // D-054. `repo::insert` returned an `Unowned`, and this is the only thing
    // that opens it — so the workspace row and the grant that makes it usable
    // are not two steps one of which can be forgotten. It seeds `docs/04`'s
    // five role templates into the workspace and assigns the creator the one
    // carrying `workspace.owner`, at WORKSPACE scope, in this transaction.
    //
    // Without it the workspace committed with no `role_assignment` row at all,
    // and since that table is the only source of authority (migration 0003),
    // its creator could read it and never write to it.
    let (created, bootstrap) =
        casual_task_persistence::role::bootstrap(&mut scoped, created, actor.actor_id.as_uuid())
            .await
            .map_err(|error| internal(&error, "granting the workspace owner", &request_id))?;

    let who = provenance(&actor, &request_id, &headers);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "workspace".to_owned(),
            aggregate_id: created.id,
            project_id: None,
            event_type: "workspace.created".to_owned(),
            // Display values, not ids (`docs/25`): the stream is rendered years
            // later, and "granted Owner to the creator" has to read correctly
            // after the role has been renamed or deleted.
            activity_changes: serde_json::json!({
                "name": created.name,
                "slug": created.slug,
                "roles_seeded": bootstrap.template_names(),
                "owner_granted_to": actor.actor_id.as_uuid(),
            }),
            // `docs/04` control 7: "Every grant, revoke, role edit, and consent
            // writes an `audit_event` with before/after." The owner grant is
            // made in this transaction, so it is audited in this record rather
            // than in one of its own — `docs/25` lists `role.assigned` as
            // audit-only, and `UnitOfWork` has no audit-only path yet (D-053).
            audit_changes: serde_json::json!({
                "before": serde_json::Value::Null,
                "after": {
                    "name": created.name,
                    "slug": created.slug,
                    "role_assignment": {
                        "id": bootstrap.assignment,
                        "principal_id": actor.actor_id.as_uuid(),
                        "principal_type": "USER",
                        "role_id": bootstrap.owner_role,
                        "role_name": casual_task_model::template::owner().name,
                        "scope_type": "WORKSPACE",
                        "scope_id": created.id,
                    },
                },
            }),
            payload: serde_json::json!({
                "workspace_id": created.id,
                "name": created.name,
                "slug": created.slug,
                "owner_id": actor.actor_id.as_uuid(),
            }),
            schema_version: SCHEMA_VERSION,
        },
        &who,
    )
    .await
    .map_err(|error| internal(&error, "recording the workspace creation", &request_id))?;

    commit(tx, &request_id).await?;

    Ok(with_etag(
        StatusCode::CREATED,
        created.version,
        workspace_body(&created),
    ))
}

/// `GET /api/v1/workspaces` — the workspaces the caller belongs to.
///
/// # Errors
///
/// [`ApiError`] on a bad page request or a database failure.
pub async fn list(
    State(state): State<AppState>,
    actor: Authenticated,
    request_id: RequestId,
    paging: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    only_a_person(&actor, &request_id)?;
    let (limit, after) = page_request(paging, &request_id)?;

    let mut conn = acquire(&state, &request_id).await?;
    let mut found = repo::list_for_user(
        &mut conn,
        actor.actor_id.as_uuid(),
        after,
        i64::from(limit) + 1,
    )
    .await
    .map_err(|error| internal(&error, "listing workspaces", &request_id))?;

    let has_more = truncate(&mut found, limit);
    let next = found.last().map(|w| cursor_for(w.id));
    Ok(page(
        found.iter().map(workspace_body).collect(),
        has_more,
        next,
    ))
}

/// `GET /api/v1/workspaces/{workspace_id}` — read one.
///
/// A non-member never reaches this handler: `WorkspaceMember` has already
/// answered 404 (`docs/04`).
///
/// # Errors
///
/// [`ApiError`] 404 if the workspace is absent or soft-deleted, or a database
/// failure.
pub async fn read(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = scope_of(&mut tx, &member, &request_id).await?;

    let found = repo::read(&mut scoped)
        .await
        .map_err(|error| internal(&error, "reading the workspace", &request_id))?
        .ok_or_else(|| ApiError::not_found(&request_id))?;
    commit(tx, &request_id).await?;

    Ok(with_etag(
        StatusCode::OK,
        found.version,
        workspace_body(&found),
    ))
}

/// `PATCH /api/v1/workspaces/{workspace_id}` — update name and/or appearance.
///
/// `If-Match` is required (`docs/05` §Concurrency): a client that forgets it
/// gets `428`, not a silently applied unconditional write.
///
/// # Errors
///
/// [`ApiError`] 428 without `If-Match`, 409 against a stale version, 404 if the
/// workspace is gone, 400 for a bad name, or a database failure.
pub async fn update(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<UpdateWorkspace>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let expected = if_match(&headers, &request_id)?;
    let name = body
        .name
        .as_deref()
        .map(|value| valid_name(value, &request_id))
        .transpose()?;
    let primary_color = body
        .appearance
        .as_ref()
        .map(|appearance| valid_primary_color(&appearance.primary_color, &request_id))
        .transpose()?;
    if name.is_none() && primary_color.is_none() {
        return Err(ApiError::bad_request(
            codes::MISSING_FIELD,
            "Provide name, appearance, or both",
            &request_id,
        )
        .with_details(serde_json::json!({ "required_any": ["name", "appearance"] })));
    }

    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = scope_of(&mut tx, &member, &request_id).await?;
    let context = Context::load(&mut scoped, &member, &headers, &request_id).await?;
    crate::unit::authorized(
        context
            .authority
            .may_in_workspace(permission::WORKSPACE_MANAGE),
        &request_id,
    )?;

    let before = repo::read(&mut scoped)
        .await
        .map_err(|error| internal(&error, "reading the workspace", &request_id))?
        .ok_or_else(|| ApiError::not_found(&request_id))?;

    let Some(after) = repo::update(&mut scoped, name, primary_color.as_deref(), expected)
        .await
        .map_err(|error| internal(&error, "updating the workspace", &request_id))?
    else {
        // It was there a statement ago, so this is a version conflict rather
        // than a disappearance. `docs/24` requires the loser to be told which
        // version it lost to, so the client can re-read and merge.
        return Err(ApiError::conflict(
            codes::VERSION_CONFLICT,
            "The workspace has changed since you read it",
            &request_id,
        )
        .with_details(serde_json::json!({
            "your_version": expected,
            "current_version": before.version,
        })));
    };

    let before_primary = workspace_body(&before).appearance.primary_color;
    let after_primary = workspace_body(&after).appearance.primary_color;
    let mut activity = serde_json::Map::new();
    if name.is_some() {
        activity.insert(
            "name".to_owned(),
            serde_json::json!({ "from": before.name, "to": after.name }),
        );
    }
    if primary_color.is_some() {
        activity.insert(
            "appearance.primary_color".to_owned(),
            serde_json::json!({ "from": before_primary, "to": after_primary }),
        );
    }

    let who = provenance_member(&member, &request_id, &headers);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "workspace".to_owned(),
            aggregate_id: after.id,
            project_id: None,
            event_type: "workspace.updated".to_owned(),
            activity_changes: serde_json::Value::Object(activity.clone()),
            audit_changes: serde_json::json!({
                "before": {
                    "name": before.name,
                    "appearance": { "primary_color": before_primary },
                },
                "after": {
                    "name": after.name,
                    "appearance": { "primary_color": after_primary },
                },
            }),
            payload: serde_json::json!({
                "workspace_id": after.id,
                "changed_fields": activity.keys().collect::<Vec<_>>(),
            }),
            schema_version: SCHEMA_VERSION,
        },
        &who,
    )
    .await
    .map_err(|error| internal(&error, "recording the workspace update", &request_id))?;

    commit(tx, &request_id).await?;
    Ok(with_etag(
        StatusCode::OK,
        after.version,
        workspace_body(&after),
    ))
}

// ---------------------------------------------------------------------------
// Handlers — workspace membership
// ---------------------------------------------------------------------------
