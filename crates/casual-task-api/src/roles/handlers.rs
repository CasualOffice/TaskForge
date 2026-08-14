//! The five handlers.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_app::{Principal, ProposedAssignment, Refusal, ResourceScopes, Scope};
use casual_task_model::{
    EnvironmentId, Permission, ProjectId, TeamId, UserId, WorkspaceId, permission,
};
use casual_task_persistence::role_edit::{self, RoleError, RoleRow};
use casual_task_persistence::{Change, UnitOfWork};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::wire::{
    AssignRequest, AssignmentQuery, AssignmentView, CreateRoleRequest, PatchRoleRequest, RoleView,
};
use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::wire::Body;
use crate::{etag, unit};

/// `docs/21` bounds every page. Same numbers as every other list here.
const DEFAULT_ASSIGNMENT_PAGE: u32 = 50;
const MAX_ASSIGNMENT_PAGE: u32 = 100;

fn view(row: RoleRow) -> RoleView {
    RoleView {
        id: row.id,
        name: row.name,
        is_template: row.is_template,
        permissions: row.permissions,
        created_at: row.created_at.format(&Rfc3339).unwrap_or_default(),
        updated_at: row.updated_at.format(&Rfc3339).unwrap_or_default(),
        version: row.version,
    }
}

/// Parse permission keys, refusing anything outside the registry.
///
/// The schema would refuse an unknown key too — `role_permission.permission` is
/// a foreign key to `permission(key)` — but a foreign-key violation reaches the
/// caller as a 500 unless it is caught, and "`task.updat` is not a permission"
/// is the sentence an admin can act on.
fn parse_permissions(keys: &[String], request_id: &str) -> Result<Vec<Permission>, ApiError> {
    keys.iter()
        .map(|key| {
            permission::ALL
                .iter()
                .copied()
                .find(|p| p.as_str() == key)
                .ok_or_else(|| {
                    ApiError::bad_request(
                        codes::INVALID_ENUM,
                        format!("`{key}` is not a permission this build knows"),
                        request_id,
                    )
                })
        })
        .collect()
}

/// Turn a ceiling refusal into the code that names the rule.
///
/// One code per control, because `docs/04` gives each its own number and
/// "denied" on its own does not tell an admin which rule they hit.
fn refused(refusal: &Refusal, request_id: &str) -> ApiError {
    match refusal {
        Refusal::ExceedsGrantCeiling { missing } => ApiError::unprocessable(
            codes::GRANT_CEILING,
            format!(
                "You cannot grant `{}` because you do not hold it at that scope",
                missing.as_str()
            ),
            request_id,
        )
        .with_details(serde_json::json!({ "missing": missing.as_str() })),
        Refusal::ExceedsScopeCeiling => ApiError::unprocessable(
            codes::SCOPE_CEILING,
            "You cannot assign at that scope",
            request_id,
        ),
        Refusal::RoleEditingIsWorkspaceScoped => ApiError::unprocessable(
            codes::SCOPE_CEILING,
            "`role.manage` exists only at workspace scope",
            request_id,
        ),
        Refusal::SelfElevation { missing } => ApiError::unprocessable(
            codes::SELF_ELEVATION,
            format!(
                "You cannot give yourself `{}` — you do not already hold it",
                missing.as_str()
            ),
            request_id,
        )
        .with_details(serde_json::json!({ "missing": missing.as_str() })),
    }
}

fn write_error(error: &RoleError, name: &str, request_id: &str) -> ApiError {
    match error {
        RoleError::NameTaken => ApiError::conflict(
            codes::ROLE_NAME_TAKEN,
            "That role name is already in use in this workspace",
            request_id,
        )
        .with_details(serde_json::json!({ "name": name })),
        RoleError::UnknownPermission => ApiError::bad_request(
            codes::INVALID_ENUM,
            "The role names a permission this build does not have",
            request_id,
        ),
        RoleError::VersionMismatch => ApiError::conflict(
            codes::VERSION_CONFLICT,
            "The role changed since you read it",
            request_id,
        ),
        RoleError::Database(error) => {
            tracing::error!(%error, "writing the role failed");
            ApiError::internal(request_id)
        }
    }
}

/// `GET /api/v1/roles`.
///
/// Readable by anyone who may assign or author, because a picker that cannot
/// list roles cannot offer one. It carries no grants, only what each role
/// *would* grant, which is workspace configuration rather than tenant content.
///
/// # Errors
///
/// `403` without `role.assign` or `role.manage`, `500` on a database failure.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    if !ctx
        .authority
        .may_in_workspace(permission::ROLE_ASSIGN)
        .is_allowed()
    {
        unit::authorized(
            ctx.authority.may_in_workspace(permission::ROLE_MANAGE),
            &request_id,
        )?;
    }

    let rows = role_edit::list(&mut scoped).await.map_err(|error| {
        tracing::error!(%error, "listing roles failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok(axum::Json(serde_json::json!({
        "data": rows.into_iter().map(view).collect::<Vec<_>>()
    }))
    .into_response())
}

/// `GET /api/v1/role-assignments` — who holds what, and where.
///
/// # Why this read had to exist
///
/// The grant set was write-only. `POST` created a grant and `DELETE` needed its
/// id, and that id appeared exactly once — in the response to the call that
/// made it. An admin who closed the tab could never take a permission back, and
/// no screen could answer "who can do this here?" without one request per
/// member. `docs/04`'s whole model is that authority is legible; a set nobody
/// can read is not.
///
/// # Authorization
///
/// The same pair as [`list`] — `role.assign` or `role.manage`. Reading the
/// grants is what makes assigning them safe: an admin choosing a role needs to
/// see what the person already has, and a permission that let you grant but not
/// look would push them to guess.
///
/// # Errors
///
/// `400` for a bad `limit`, `403` without either permission, `500` on a
/// database failure.
pub async fn list_assignments(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    filters: Result<Query<AssignmentQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let Query(filters) = filters.map_err(|rejection| {
        ApiError::bad_request(codes::MALFORMED_BODY, rejection.body_text(), &request_id)
    })?;
    let limit = match filters.limit {
        None => DEFAULT_ASSIGNMENT_PAGE,
        // Not clamped: a silently smaller page is one the client cannot notice
        // it asked for, and `docs/20` gives the refusal a code.
        Some(limit) if limit == 0 || limit > MAX_ASSIGNMENT_PAGE => {
            return Err(ApiError::bad_request(
                codes::PAGE_TOO_LARGE,
                format!("limit must be between 1 and {MAX_ASSIGNMENT_PAGE}"),
                &request_id,
            ));
        }
        Some(limit) => limit,
    };

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    if !ctx
        .authority
        .may_in_workspace(permission::ROLE_ASSIGN)
        .is_allowed()
    {
        unit::authorized(
            ctx.authority.may_in_workspace(permission::ROLE_MANAGE),
            &request_id,
        )?;
    }

    let filter = role_edit::AssignmentFilter {
        principal_id: filters.principal_id,
        role_id: filters.role_id,
        scope_id: filters.scope_id,
    };
    // One more than asked for: whether another page exists is a fact about the
    // data, and a client that had to issue an empty request to find out would
    // pay for it on every list that happens to end on a boundary.
    let mut rows =
        role_edit::list_assignments(&mut scoped, filter, filters.cursor, i64::from(limit) + 1)
            .await
            .map_err(|error| {
                tracing::error!(%error, "listing grants failed");
                ApiError::internal(&request_id)
            })?;
    unit::commit(tx, &request_id).await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = rows.last().map(|row| row.id);

    Ok(axum::Json(serde_json::json!({
        "data": rows.iter().map(assignment_view).collect::<Vec<_>>(),
        "page": { "next_cursor": next, "has_more": has_more },
    }))
    .into_response())
}

/// `POST /api/v1/roles` — author a role.
///
/// # Errors
///
/// `400` for an unknown permission key, `403` without `role.manage`, `409` for
/// a duplicate name, `422` when the role would carry a permission the author
/// does not hold.
pub async fn create(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Body(body): Body<CreateRoleRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(ApiError::bad_request(
            codes::MISSING_FIELD,
            "name must not be empty",
            &request_id,
        ));
    }
    let wanted = parse_permissions(&body.permissions, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // Control 3 — authoring is workspace-scoped, and D-049 makes it a different
    // permission from assigning.
    unit::authorized(
        ctx.authority.may_in_workspace(permission::ROLE_MANAGE),
        &request_id,
    )?;

    // Control 1, at authoring time. `docs/04` checks the grant ceiling on
    // assignment *and* on edit; authoring is the first edit, and a role nobody
    // could ever assign is a trap rather than a feature.
    ceiling_over(&ctx, &wanted, Scope::Workspace(ctx.workspace), &request_id)?;

    let row = role_edit::create(&mut scoped, &name, &body.permissions)
        .await
        .map_err(|error| write_error(&error, &name, &request_id))?;

    let rendered = view(row);
    let payload = serde_json::to_value(&rendered).unwrap_or(serde_json::Value::Null);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "role".to_owned(),
            aggregate_id: rendered.id,
            project_id: None,
            event_type: "role.created".to_owned(),
            activity_changes: serde_json::json!({ "name": rendered.name }),
            audit_changes: serde_json::json!({ "before": null, "after": payload }),
            payload: payload.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the role create failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::CREATED,
        [(header::ETAG, etag::tag(rendered.version))],
        axum::Json(payload),
    )
        .into_response())
}

/// `PATCH /api/v1/roles/{id}` — rename, or replace the permission set.
///
/// # Errors
///
/// `400`, `403` without `role.manage`, `404`, `409` for a stale `If-Match` or a
/// duplicate name, `422` when the new set exceeds what the editor holds, `428`
/// without `If-Match`.
pub async fn update(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(role_id): Path<Uuid>,
    Body(body): Body<PatchRoleRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let expected = etag::if_match(&headers, &request_id)?;
    let wanted = match &body.permissions {
        Some(keys) => Some(parse_permissions(keys, &request_id)?),
        None => None,
    };

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    unit::authorized(
        ctx.authority.may_in_workspace(permission::ROLE_MANAGE),
        &request_id,
    )?;

    let before = role_edit::read(&mut scoped, role_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the role failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, &request_id))?;

    // Control 1 on edit. `docs/04`: "editing a role you granted cannot smuggle
    // in new permissions" — so the ceiling is re-checked against the NEW set,
    // and the repository replaces the set wholesale rather than merging.
    if let Some(wanted) = &wanted {
        ceiling_over(&ctx, wanted, Scope::Workspace(ctx.workspace), &request_id)?;
    }

    let name = body.name.as_deref().map(str::trim);
    let row = role_edit::update(
        &mut scoped,
        role_id,
        name,
        body.permissions.as_deref(),
        expected,
    )
    .await
    .map_err(|error| write_error(&error, name.unwrap_or(&before.name), &request_id))?;

    let rendered = view(row);
    let payload = serde_json::to_value(&rendered).unwrap_or(serde_json::Value::Null);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "role".to_owned(),
            aggregate_id: rendered.id,
            project_id: None,
            event_type: "role.updated".to_owned(),
            activity_changes: serde_json::json!({ "name": rendered.name }),
            audit_changes: serde_json::json!({
                "before": { "name": before.name, "permissions": before.permissions },
                "after": payload,
            }),
            payload: payload.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the role update failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        [(header::ETAG, etag::tag(rendered.version))],
        axum::Json(payload),
    )
        .into_response())
}

include!("assignments.rs");
#[cfg(test)]
#[path = "handlers_tests.rs"]
mod tests;
