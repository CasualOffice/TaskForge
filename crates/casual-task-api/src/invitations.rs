//! Invitations (C-001, `docs/40` §Invitations).
//!
//! > "Invite by email, single-use, 7-day expiry, tied to the address."
//!
//! # The invite response is constant, and that closes the enumeration gate
//!
//! `docs/40` §Acceptance gates requires login, reset **and invite** responses
//! to be indistinguishable for existing and non-existing accounts. The first
//! two were closed by `auth` and `password_reset`; this is the third, and until
//! it existed the gate could not close.
//!
//! `docs/40` §Invitations states the rule directly: "The response is identical
//! whether or not the address has an account — **only the delivered email
//! differs**." So [`create`] returns `202` with a fixed body on every path —
//! new address, existing account, already invited. Nothing about the response
//! varies with anything the caller could learn from.
//!
//! **The cost is stated:** an inviter does not get the invitation id back and
//! must `GET` the list to find it. That is a real ergonomic loss, taken because
//! the alternative — returning the created row — makes the endpoint's response
//! a function of state the caller is otherwise asking about, and every future
//! edit to that body becomes a chance to reintroduce the oracle. A constant is
//! the only shape that cannot drift into one.
//!
//! Delivery is off the request path for the same reason it is in
//! [`crate::password_reset`]: an SMTP handshake on one branch and not the other
//! is a timing oracle wide enough to read with a stopwatch.
//!
//! # Tied to the address
//!
//! An invitation is **not a bearer token for whoever holds the link**. Accepting
//! while signed in as an account whose email differs from the invited address is
//! refused. Without that check, forwarding the email — which people do, in good
//! faith — hands workspace membership to the wrong person, and the audit trail
//! records it as the invitation being used correctly.
//!
//! # What is NOT enforced here yet, stated plainly
//!
//! **Issuing an invitation requires workspace membership and nothing more.**
//! That matches what `crate::workspaces` already does for adding a member
//! directly, and for the same recorded reason: **D-054** — "which permission
//! governs workspace membership and team management" — is `Open`, the closed
//! registry has no invitation permission, and `docs/04` names none. Gating this
//! on an invented mapping would settle D-054 in an implementation, which
//! `AGENTS.md` forbids. Inviting is adding a member by another route, so it
//! inherits that decision rather than pre-empting it.
//!
//! **Inviting *with a role* is gated, because that half is not open.**
//! `docs/04` §Grant ceilings is explicit that `role.assign` "grants an existing
//! role at workspace scope", and an invitation carrying a `role_id` is a
//! deferred grant of exactly that shape. So [`create`] requires
//! `role.assign` **and** applies control 1 — "you cannot grant what you do not
//! hold" — permission by permission. Without it, inviting would be a way to
//! hand out a role the inviter does not hold, which is the escalation hole
//! D-049 split `role.assign` from `role.manage` to prevent.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_infra::{Mailer, Message};
use casual_task_model::{AuthContext, WorkspaceId, permission};
use casual_task_persistence::identity;
use casual_task_persistence::invitation as repo;
use casual_task_persistence::workspace as workspace_repo;
use casual_task_persistence::{Change, Provenance, Scoped, UnitOfWork};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::json::ValidJson;
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;

/// The event schema version this module emits.
const SCHEMA_VERSION: i32 = 1;

/// The path the emailed link points at, under `TF_PUBLIC_URL`.
pub const ACCEPT_PATH: &str = "/accept-invitation";

/// The subject line. ASCII and constant — `casual-task-infra` refuses anything
/// else, and a workspace name here would put tenant content in a subject
/// delivered to an address nobody has yet proved they control.
pub const INVITE_SUBJECT: &str = "You have been invited to a TaskForge workspace";

/// The largest page [`list`] will serve.
const MAX_PAGE: u32 = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInvitation {
    pub email: String,
    /// The role the invitee receives on acceptance. Optional: an invitation
    /// with no role adds membership and nothing else.
    #[serde(default)]
    pub role_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptInvitation {
    pub token: String,
    /// Used only when the invitation creates an account. Ignored when one
    /// already exists — an invitation must not be able to rename a stranger.
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paging {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct InvitationBody {
    pub id: Uuid,
    pub email: String,
    pub role_id: Option<Uuid>,
    pub invited_by: Option<Uuid>,
    pub expires_at: String,
    pub created_at: String,
}

/// The one response [`create`] can produce.
///
/// A struct with one constant field, so the shape is a *type*: a future edit
/// that wanted to return the invitation for a new address has to change this
/// declaration, in view of the module docs, rather than add a branch.
#[derive(Debug, Serialize)]
pub struct Accepted {
    pub message: &'static str,
}

impl Accepted {
    const TEXT: &'static str = "If that address can be invited, an invitation is on its way.";
}

#[derive(Debug, Serialize)]
pub struct AcceptedInvitation {
    pub workspace_id: Uuid,
    pub user_id: Uuid,
}

/// `POST /api/v1/workspaces/{workspace_id}/invitations`.
///
/// # Errors
///
/// `403` when a role is requested and the caller may not grant it, `422` for a
/// role that does not exist in this workspace, `400` for a malformed address,
/// or a database failure.
pub async fn create(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<CreateInvitation>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let email = valid_email(&body.email, &request_id)?;

    let accepted = (
        StatusCode::ACCEPTED,
        axum::Json(Accepted {
            message: Accepted::TEXT,
        }),
    )
        .into_response();

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    if let Some(role_id) = body.role_id {
        authorize_role_grant(&mut scoped, &ctx, role_id, &request_id).await?;
    }

    // Supersede any live invitation for this address. Someone re-inviting
    // because the first email was lost must not leave two working links in one
    // inbox — the same rule password reset applies, and the partial unique
    // index on (workspace_id, email) requires it anyway.
    if let Some(existing) = repo::live_for_email(&mut scoped, email)
        .await
        .map_err(|error| internal(&error, "checking for a live invitation", &request_id))?
    {
        repo::revoke(&mut scoped, existing)
            .await
            .map_err(|error| internal(&error, "superseding an invitation", &request_id))?;
    }

    let minted = casual_task_identity::credential::mint().map_err(|error| {
        tracing::error!(%error, "the randomness source failed");
        ApiError::internal(&request_id)
    })?;
    let (selector, _) = casual_task_identity::credential::split(&minted.presented)
        .map_err(|_| ApiError::internal(&request_id))?;

    let created = repo::insert(
        &mut scoped,
        email,
        body.role_id,
        ctx.actor.as_uuid(),
        selector,
        &minted.verifier_hash,
        OffsetDateTime::now_utc() + repo::INVITATION_LIFETIME,
    )
    .await
    .map_err(|error| internal(&error, "creating the invitation", &request_id))?;

    // docs/40 §What is audited: `user.invited`.
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "workspace".to_owned(),
            aggregate_id: member.context.scope().id().as_uuid(),
            project_id: None,
            event_type: "user.invited".to_owned(),
            // The ADDRESS, not the token. An activity stream is rendered in a
            // browser and exported to a CSV; a working invitation link in
            // either is a credential in a place nobody is guarding.
            activity_changes: serde_json::json!({
                "email": created.email, "role_id": created.role_id,
            }),
            audit_changes: serde_json::json!({
                "before": serde_json::Value::Null,
                "after": { "email": created.email, "role_id": created.role_id },
            }),
            payload: serde_json::json!({
                "workspace_id": member.context.scope().id().as_uuid(),
                "invitation_id": created.id,
                "email": created.email,
            }),
            schema_version: SCHEMA_VERSION,
        },
        &provenance_of(&ctx),
    )
    .await
    .map_err(|error| internal(&error, "recording the invitation", &request_id))?;

    unit::commit(tx, &request_id).await?;

    // After the commit. An email promising a link that a rolled-back
    // transaction never created is worse than a slow one.
    deliver(
        state.mailer.clone(),
        Message::new(
            created.email,
            INVITE_SUBJECT,
            invite_body(&state.public_url, &minted.presented),
        ),
    );

    Ok(accepted)
}

/// `GET /api/v1/workspaces/{workspace_id}/invitations` — the live ones.
///
/// # Errors
///
/// `400` on a bad page request, or a database failure.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    Query(paging): Query<Paging>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let limit = match paging.limit {
        None => 50,
        Some(n) if (1..=MAX_PAGE).contains(&n) => n,
        Some(_) => {
            return Err(ApiError::bad_request(
                codes::OUT_OF_RANGE,
                "limit must be between 1 and 100",
                &request_id,
            )
            .with_details(serde_json::json!({ "max": MAX_PAGE })));
        }
    };

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;

    // One more than asked for, so "is there another page" needs no count.
    let mut rows = repo::list_live(&mut scoped, paging.cursor, limit + 1)
        .await
        .map_err(|error| internal(&error, "listing invitations", &request_id))?;
    unit::commit(tx, &request_id).await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = has_more.then(|| rows.last().map(|r| r.id)).flatten();

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "data": rows.iter().map(body_of).collect::<Vec<_>>(),
            "next_cursor": next,
        })),
    )
        .into_response())
}

/// `DELETE /api/v1/workspaces/{workspace_id}/invitations/{id}` — revoke.
///
/// # Errors
///
/// `404` if it is not a live invitation in this workspace, or a database
/// failure.
pub async fn revoke(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    headers: HeaderMap,
    Path((_workspace, id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let revoked = repo::revoke(&mut scoped, id)
        .await
        .map_err(|error| internal(&error, "revoking an invitation", &request_id))?;
    if !revoked {
        // 404 and not 403: `docs/04` requires absent and invisible to be
        // indistinguishable, and an invitation in another tenant is invisible.
        return Err(ApiError::not_found(&request_id));
    }

    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "workspace".to_owned(),
            aggregate_id: member.context.scope().id().as_uuid(),
            project_id: None,
            event_type: "user.invitation.revoked".to_owned(),
            activity_changes: serde_json::json!({ "invitation_id": id }),
            audit_changes: serde_json::json!({
                "before": { "invitation_id": id, "state": "PENDING" },
                "after": { "invitation_id": id, "state": "REVOKED" },
            }),
            payload: serde_json::json!({
                "workspace_id": member.context.scope().id().as_uuid(),
                "invitation_id": id,
            }),
            schema_version: SCHEMA_VERSION,
        },
        &provenance_of(&ctx),
    )
    .await
    .map_err(|error| internal(&error, "recording the revocation", &request_id))?;

    unit::commit(tx, &request_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

include!("invitations_accept.rs");
#[cfg(test)]
#[path = "invitations_tests.rs"]
mod tests;
