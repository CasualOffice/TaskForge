//! Workspaces, membership and teams (C-002, `docs/05` §Projects, workflow,
//! admin).
//!
//! # Two extractors, and which one each route may use
//!
//! Every route here that touches a tenant row takes [`WorkspaceMember`], which
//! is the only thing in this crate that mints an `AuthContext` and therefore
//! the only route to a `WorkspaceScope` (`docs/32`). A handler holding only
//! [`Authenticated`] cannot reach a tenant row, because it has no scope to ask
//! for one with — a compile-time property, not a review note.
//!
//! Exactly two routes take `Authenticated`, and both are the same question:
//!
//! - `POST /api/v1/workspaces` — there is no workspace to be a member of yet.
//!   The scope it eventually uses is minted for the workspace it is *creating*,
//!   in the same transaction that makes the creator a member of it, so the
//!   membership the scope claims is true by the time the transaction commits.
//! - `GET /api/v1/workspaces` — "which workspaces do I belong to" is
//!   cross-tenant by construction. `docs/32` §The `user_account` exception is
//!   the same observation: a person spans workspaces, and their own membership
//!   index is their own data. It reads through the migration-0019 seam, which
//!   is filtered by user and cannot be pointed at anyone else.
//!
//! # Absent and invisible are the same answer
//!
//! `docs/04`: an invisible resource returns **404, not 403**, and the two are
//! never disambiguated. Non-membership is handled for free — `WorkspaceMember`
//! rejects with 404 before a handler runs — and the handlers keep the property
//! for everything below it: a team in another tenant is hidden by the policy,
//! read back as `None`, and returned as the same 404 as a team id that was
//! never allocated.
//!
//! # What is NOT enforced here yet, stated plainly
//!
//! **Membership is the only authority C-002 applies.** Any member of a
//! workspace can rename it, add and remove members, and create teams. That is
//! not the intended end state — `docs/04` gives Member "no config" — but the
//! machinery that would express it does not exist yet: `role_assignment` is the
//! only source of authority (migration 0003), no built-in role template has
//! been authored, and the golden matrix that fixes each template's permission
//! set is explicitly still missing (`docs/14` §C-004). Enforcing an invented
//! mapping here would be settling that in an implementation, which AGENTS.md
//! forbids. Tracked as **D-054**.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_model::{ActorType, AuthContext, WorkspaceId};
use casual_task_observability::labels::LabelSet;
use casual_task_observability::metrics::AUTHZ_EPOCH_BUMPS_TOTAL;
use casual_task_persistence::workspace as repo;
use casual_task_persistence::{Change, Provenance, Scoped, UnitOfWork};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::error::{ApiError, codes};
use crate::json::ValidJson;
use crate::middleware::{Authenticated, WorkspaceMember};
use crate::server::{AppState, RequestId};

/// `docs/05` §Pagination: "limit default 50, max 100".
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 100;

/// The event schema version carried by every event this module emits.
const SCHEMA_VERSION: i32 = 1;

/// Bounds on the two free-text fields, so no input is unbounded (AGENTS.md
/// §Engineering priorities 4).
const MAX_NAME: usize = 200;
const MAX_SLUG: usize = 64;

// ---------------------------------------------------------------------------
// Representations
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct WorkspaceBody {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct MemberBody {
    pub user_id: Uuid,
    pub display_name: String,
    /// `null` once the account is anonymized (ADR-026).
    pub email: Option<String>,
    pub member_type: String,
    pub joined_at: String,
}

#[derive(Debug, Serialize)]
pub struct TeamBody {
    pub id: Uuid,
    pub name: String,
    pub created_at: String,
}

/// The documented list envelope (`docs/05` §Pagination).
#[derive(Debug, Serialize)]
pub struct PageBody<T> {
    pub data: Vec<T>,
    pub page: PageInfo,
}

#[derive(Debug, Serialize)]
pub struct PageInfo {
    /// Opaque. `docs/05`: "clients must not parse it".
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateWorkspace {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameWorkspace {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddMember {
    pub user_id: Uuid,
    /// Absent means `MEMBER`. `docs/04` §Built-in role templates gives GUEST a
    /// narrower shape, so making it the explicit choice keeps the wider one
    /// from being granted by omission.
    pub member_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTeam {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddTeamMember {
    pub user_id: Uuid,
}

/// Cursor pagination parameters.
///
/// `deny_unknown_fields` here as well as on bodies: `docs/05` says unknown
/// request fields are rejected, and a mistyped `?limti=200` that is silently
/// ignored produces the same class of client bug as a mistyped body field.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paging {
    pub limit: Option<u32>,
    /// The opaque `next_cursor` from a previous page.
    pub cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers — workspaces
// ---------------------------------------------------------------------------

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

/// `PATCH /api/v1/workspaces/{workspace_id}` — rename.
///
/// `If-Match` is required (`docs/05` §Concurrency): a client that forgets it
/// gets `428`, not a silently applied unconditional write.
///
/// # Errors
///
/// [`ApiError`] 428 without `If-Match`, 409 against a stale version, 404 if the
/// workspace is gone, 400 for a bad name, or a database failure.
pub async fn rename(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<RenameWorkspace>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let expected = if_match(&headers, &request_id)?;
    let name = valid_name(&body.name, &request_id)?;

    let mut tx = begin(&state, &request_id).await?;
    let mut scoped = scope_of(&mut tx, &member, &request_id).await?;

    let before = repo::read(&mut scoped)
        .await
        .map_err(|error| internal(&error, "reading the workspace", &request_id))?
        .ok_or_else(|| ApiError::not_found(&request_id))?;

    let Some(after) = repo::rename(&mut scoped, name, expected)
        .await
        .map_err(|error| internal(&error, "renaming the workspace", &request_id))?
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

    let who = provenance_member(&member, &request_id, &headers);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "workspace".to_owned(),
            aggregate_id: after.id,
            project_id: None,
            event_type: "workspace.renamed".to_owned(),
            activity_changes: serde_json::json!({ "name": { "from": before.name, "to": after.name } }),
            audit_changes: serde_json::json!({
                "before": { "name": before.name },
                "after": { "name": after.name },
            }),
            payload: serde_json::json!({ "workspace_id": after.id, "name": after.name }),
            schema_version: SCHEMA_VERSION,
        },
        &who,
    )
    .await
    .map_err(|error| internal(&error, "recording the rename", &request_id))?;

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

/// `/workspaces/{workspace_id}/members/{user_id}`.
///
/// `workspace_id` is declared even though the handler reads the workspace from
/// the scope: it is the same parameter `WorkspaceMember` resolved the tenant
/// from, and naming every captured segment keeps this type a faithful
/// description of the route it is attached to.
#[derive(Debug, Deserialize)]
pub struct MemberPath {
    pub workspace_id: Uuid,
    pub user_id: Uuid,
}

/// `/teams/{team_id}/members`.
#[derive(Debug, Deserialize)]
pub struct TeamPath {
    pub team_id: Uuid,
}

/// `/teams/{team_id}/members/{user_id}`.
#[derive(Debug, Deserialize)]
pub struct TeamMemberPath {
    pub team_id: Uuid,
    pub user_id: Uuid,
}

// ---------------------------------------------------------------------------
// Shared plumbing
// ---------------------------------------------------------------------------

/// Refuse a non-person actor on the two pre-workspace routes.
///
/// A bearer token is "scoped to one workspace" (`docs/40`), so using one to
/// create a *different* workspace, or to enumerate the workspaces of the person
/// it was issued against, is outside the contract the token was issued under.
/// 403 rather than 404: the endpoint is not hidden, the credential is simply
/// not the right kind.
fn only_a_person(actor: &Authenticated, request_id: &str) -> Result<(), ApiError> {
    if matches!(actor.actor_type, ActorType::User) {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            codes::WRONG_CREDENTIAL_TYPE,
            request_id,
        ))
    }
}

async fn begin(
    state: &AppState,
    request_id: &str,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, ApiError> {
    state
        .pool
        .begin()
        .await
        .map_err(|_| ApiError::unavailable(request_id, 5))
}

async fn acquire(
    state: &AppState,
    request_id: &str,
) -> Result<sqlx::pool::PoolConnection<sqlx::Postgres>, ApiError> {
    state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(request_id, 5))
}

async fn scope_of<'t>(
    tx: &'t mut sqlx::Transaction<'static, sqlx::Postgres>,
    member: &WorkspaceMember,
    request_id: &str,
) -> Result<Scoped<'t>, ApiError> {
    Scoped::apply(tx, &member.context.scope())
        .await
        .map_err(|error| internal(&error, "applying the tenant scope", request_id))
}

async fn commit(
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
    request_id: &str,
) -> Result<(), ApiError> {
    tx.commit()
        .await
        .map_err(|error| internal(&error, "committing", request_id))
}

/// Bump `authz_epoch` and count it (`docs/46` §Domain metrics).
async fn bump_epoch(
    state: &AppState,
    scoped: &mut Scoped<'_>,
    request_id: &str,
) -> Result<(), ApiError> {
    repo::bump_authz_epoch(scoped)
        .await
        .map_err(|error| internal(&error, "bumping authz_epoch", request_id))?;
    // A metric failure must never fail a membership change.
    let _ = state.metrics.increment(
        AUTHZ_EPOCH_BUMPS_TOTAL,
        &LabelSet::for_metric(AUTHZ_EPOCH_BUMPS_TOTAL),
        1,
    );
    Ok(())
}

fn provenance(actor: &Authenticated, request_id: &str, headers: &HeaderMap) -> Provenance {
    Provenance {
        actor: Some(actor.actor_id),
        actor_type: actor.actor_type,
        request_id: Uuid::parse_str(request_id)
            .ok()
            .map(casual_task_model::RequestId::from_uuid),
        correlation_id: None,
        ip: crate::auth::client_ip(headers),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned),
    }
}

fn provenance_member(
    member: &WorkspaceMember,
    request_id: &str,
    headers: &HeaderMap,
) -> Provenance {
    Provenance {
        actor: Some(member.context.actor_id()),
        actor_type: member.context.actor_type(),
        request_id: Uuid::parse_str(request_id)
            .ok()
            .map(casual_task_model::RequestId::from_uuid),
        correlation_id: None,
        ip: crate::auth::client_ip(headers),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned),
    }
}

/// `docs/24`: the version is exposed as an `ETag` and required back as
/// `If-Match`.
fn with_etag(status: StatusCode, version: i64, body: WorkspaceBody) -> Response {
    let mut response = (status, axum::Json(body)).into_response();
    if let Ok(value) = format!("\"{version}\"").parse() {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

/// The version from `If-Match`, or the documented refusal.
fn if_match(headers: &HeaderMap, request_id: &str) -> Result<i64, ApiError> {
    let raw = headers
        .get(header::IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::precondition_required(request_id))?;

    raw.trim()
        .trim_start_matches("W/")
        .trim_matches('"')
        .parse::<i64>()
        .map_err(|_| {
            ApiError::bad_request(
                codes::IF_MATCH_MALFORMED,
                "If-Match is not an ETag this server issued",
                request_id,
            )
        })
}

/// Validate the page request, returning `(limit, after_id)`.
fn page_request(
    paging: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
    request_id: &str,
) -> Result<(u32, Option<Uuid>), ApiError> {
    let paging = query(paging, request_id)?;
    let limit = limit_of(&paging, request_id)?;
    let after = match paging.cursor.as_deref() {
        None => None,
        Some(raw) => Some(decode_cursor(raw, request_id)?.id),
    };
    Ok((limit, after))
}

/// The same, for a list keyed by a text column rather than by id.
fn page_request_text(
    paging: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
    request_id: &str,
) -> Result<(u32, Option<String>), ApiError> {
    let paging = query(paging, request_id)?;
    let limit = limit_of(&paging, request_id)?;
    let after = match paging.cursor.as_deref() {
        None => None,
        Some(raw) => decode_cursor(raw, request_id)?.keys.into_iter().next(),
    };
    Ok((limit, after))
}

fn query(
    paging: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
    request_id: &str,
) -> Result<Paging, ApiError> {
    // A rejection here is an unknown or unparseable query parameter, which
    // `docs/05` makes a 400 rather than something silently ignored.
    paging.map(|Query(paging)| paging).map_err(|_| {
        ApiError::bad_request(
            codes::UNKNOWN_FIELD,
            "Unknown or malformed query parameter",
            request_id,
        )
    })
}

fn limit_of(paging: &Paging, request_id: &str) -> Result<u32, ApiError> {
    match paging.limit {
        None => Ok(DEFAULT_LIMIT),
        // Clamping silently would return a page the client did not ask for and
        // has no way to notice; `docs/20` has a code for it (`TF-QRY-0007`),
        // which is a decision that it is an error rather than a courtesy.
        Some(limit) if limit == 0 || limit > MAX_LIMIT => Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "limit must be between 1 and 100",
            request_id,
        )),
        Some(limit) => Ok(limit),
    }
}

fn decode_cursor(raw: &str, request_id: &str) -> Result<casual_task_model::Cursor, ApiError> {
    casual_task_model::Cursor::decode(raw).map_err(|_| {
        ApiError::bad_request(
            codes::MALFORMED_BODY,
            "Malformed pagination cursor",
            request_id,
        )
    })
}

fn cursor_for(id: Uuid) -> String {
    casual_task_model::Cursor::new(Vec::new(), id).encode()
}

fn encode_cursor(key: &str, id: Uuid) -> String {
    casual_task_model::Cursor::new(vec![key.to_owned()], id).encode()
}

/// The id immediately below `id`, so a keyset walk starting `> after` includes
/// `id` itself.
///
/// Used to read one specific row back through the same paged query rather than
/// adding a second statement that could drift from it.
fn previous_uuid(id: Uuid) -> Option<Uuid> {
    let n = id.as_u128();
    n.checked_sub(1).map(Uuid::from_u128)
}

/// Drop the probe row fetched to detect a next page, reporting whether it was
/// there.
fn truncate<T>(rows: &mut Vec<T>, limit: u32) -> bool {
    let limit = limit as usize;
    if rows.len() > limit {
        rows.truncate(limit);
        true
    } else {
        false
    }
}

fn page<T: Serialize>(data: Vec<T>, has_more: bool, next: Option<String>) -> Response {
    (
        StatusCode::OK,
        axum::Json(PageBody {
            data,
            page: PageInfo {
                next_cursor: if has_more { next } else { None },
                has_more,
            },
        }),
    )
        .into_response()
}

fn valid_name<'a>(name: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_NAME {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "name must be between 1 and 200 characters",
            request_id,
        ));
    }
    Ok(trimmed)
}

/// A slug is URL-visible, so its character set is bounded rather than trusted.
fn valid_slug<'a>(slug: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let ok = !slug.is_empty()
        && slug.len() <= MAX_SLUG
        && slug.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(slug)
    } else {
        Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "slug must be 1-64 characters of a-z, 0-9 and -, starting with a letter or digit",
            request_id,
        ))
    }
}

fn workspace_body(record: &repo::WorkspaceRecord) -> WorkspaceBody {
    WorkspaceBody {
        id: record.id,
        name: record.name.clone(),
        slug: record.slug.clone(),
        created_at: rfc3339(record.created_at),
    }
}

fn member_body(record: &repo::MemberRecord) -> MemberBody {
    MemberBody {
        user_id: record.user_id,
        display_name: record.display_name.clone(),
        email: record.email.clone(),
        member_type: record.member_type.clone(),
        joined_at: rfc3339(record.joined_at),
    }
}

fn team_body(record: &repo::TeamRecord) -> TeamBody {
    TeamBody {
        id: record.id,
        name: record.name.clone(),
        created_at: rfc3339(record.created_at),
    }
}

/// `docs/05` §Conventions: RFC 3339, always UTC, always `Z`.
fn rfc3339(at: OffsetDateTime) -> String {
    at.to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .unwrap_or_default()
}

fn unique_violation(error: &sqlx::Error) -> bool {
    sqlstate(error).as_deref() == Some("23505")
}

fn foreign_key_violation(error: &sqlx::Error) -> bool {
    sqlstate(error).as_deref() == Some("23503")
}

fn sqlstate(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
}

/// Log the cause, return the opaque envelope.
///
/// The detail belongs in the log correlated by `request_id`; in the response it
/// is reconnaissance (`docs/05`).
fn internal(error: &sqlx::Error, doing: &str, request_id: &str) -> ApiError {
    tracing::error!(%error, doing, "workspace request failed");
    ApiError::internal(request_id)
}

fn internal_message(doing: &str, request_id: &str) -> ApiError {
    tracing::error!(doing, "workspace request failed");
    ApiError::internal(request_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_cannot_carry_anything_a_url_would_have_to_escape() {
        for bad in [
            "",
            "-leading",
            "Upper",
            "with space",
            "with/slash",
            "with.dot",
            "..",
            "a", // fine — kept below as the positive case
        ] {
            let result = valid_slug(bad, "r");
            if bad == "a" {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err(), "accepted slug {bad:?}");
            }
        }
        assert!(valid_slug(&"a".repeat(MAX_SLUG), "r").is_ok());
        assert!(valid_slug(&"a".repeat(MAX_SLUG + 1), "r").is_err());
    }

    #[test]
    fn a_name_is_bounded_and_not_blank() {
        assert!(valid_name("  ", "r").is_err());
        assert!(valid_name(&"x".repeat(MAX_NAME + 1), "r").is_err());
        assert_eq!(valid_name("  Acme  ", "r").expect("valid"), "Acme");
    }

    #[test]
    fn if_match_is_required_and_then_parsed() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            if_match(&headers, "r").expect_err("required").status(),
            StatusCode::PRECONDITION_REQUIRED
        );

        headers.insert(header::IF_MATCH, "\"7\"".parse().expect("valid"));
        assert_eq!(if_match(&headers, "r").expect("parsed"), 7);

        // Weak validators are what a caching proxy may rewrite an ETag into.
        headers.insert(header::IF_MATCH, "W/\"7\"".parse().expect("valid"));
        assert_eq!(if_match(&headers, "r").expect("parsed"), 7);

        headers.insert(header::IF_MATCH, "\"nonsense\"".parse().expect("valid"));
        assert_eq!(
            if_match(&headers, "r").expect_err("malformed").status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn the_page_limit_is_bounded_at_the_documented_values() {
        // docs/05: default 50, max 100. A client asking for 10_000 is refused
        // rather than quietly served 100, because a silently clamped page is
        // indistinguishable to the client from a short last page.
        let with = |limit| Paging {
            limit,
            cursor: None,
        };
        assert_eq!(limit_of(&with(None), "r").expect("default"), DEFAULT_LIMIT);
        assert_eq!(limit_of(&with(Some(100)), "r").expect("max"), MAX_LIMIT);
        assert!(limit_of(&with(Some(0)), "r").is_err());
        assert!(limit_of(&with(Some(101)), "r").is_err());
    }

    #[test]
    fn the_probe_row_is_dropped_and_reported() {
        let mut rows = vec![1, 2, 3];
        assert!(truncate(&mut rows, 2));
        assert_eq!(rows, vec![1, 2]);

        let mut rows = vec![1, 2];
        assert!(!truncate(&mut rows, 2));
        assert_eq!(rows, vec![1, 2]);
    }

    #[test]
    fn a_cursor_round_trips_and_is_not_an_offset() {
        let id = Uuid::now_v7();
        let encoded = cursor_for(id);
        assert_eq!(
            casual_task_model::Cursor::decode(&encoded)
                .expect("decodes")
                .id,
            id
        );
        assert!(!encoded.contains('='), "cursors are base64url, unpadded");
    }

    #[test]
    fn timestamps_are_utc_with_a_z() {
        // docs/05 §Conventions: "RFC 3339, always UTC, always Z". An
        // OffsetDateTime carrying +05:30 formats as +05:30 unless converted.
        let at = OffsetDateTime::from_unix_timestamp(1_767_225_600)
            .expect("valid")
            .to_offset(time::UtcOffset::from_hms(5, 30, 0).expect("valid"));
        assert!(rfc3339(at).ends_with('Z'), "{}", rfc3339(at));
    }
}
