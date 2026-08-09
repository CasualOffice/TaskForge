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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

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

/// `POST /api/v1/auth/invitations/accept`.
///
/// **Unauthenticated by design.** The invitee may have no account, which is the
/// point of inviting by email. The invitation token is the authority, and it is
/// checked the same way every other credential in this system is: a selector
/// finds the row in one indexed read, a constant-time comparison verifies the
/// secret.
///
/// # Errors
///
/// `401` for an unknown, expired, revoked or already-accepted token, `403` when
/// a signed-in caller's address does not match the invited one, or a database
/// failure.
pub async fn accept(
    State(state): State<AppState>,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<AcceptInvitation>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;

    // Parsed before anything else: a malformed token must not reach a query as
    // a parameter, and it fails with the same 401 a wrong one does.
    let Ok((selector, verifier)) = casual_task_identity::credential::split(body.token.trim())
    else {
        return Ok(ApiError::unauthenticated(&request_id).into_response());
    };

    let mut tx = unit::begin(&state, &request_id).await?;

    // Through the ADR-032 seam: unscoped by necessity, because the workspace is
    // not known until this returns it.
    let pending = repo::find_pending(tx.as_mut(), selector)
        .await
        .map_err(|error| internal(&error, "looking up an invitation", &request_id))?;
    let stored = repo::pending_verifier(tx.as_mut(), selector)
        .await
        .map_err(|error| internal(&error, "reading the invitation verifier", &request_id))?
        .unwrap_or_default();

    let Some(pending) =
        pending.filter(|_| casual_task_identity::credential::verify(verifier, &stored))
    else {
        // Unknown, expired, revoked, already accepted, or a forged verifier
        // against a real selector: one refusal, so none is distinguishable.
        return Ok(ApiError::unauthenticated(&request_id).into_response());
    };

    // Who is accepting? A signed-in session, if there is one; otherwise the
    // address the invitation names.
    let signed_in = current_user(tx.as_mut(), &headers).await?;
    let user_id = match signed_in {
        Some((user_id, email)) => {
            // TIED TO THE ADDRESS (docs/40 §Invitations). An invitation is not
            // a bearer token for whoever holds the link — forwarding the email
            // must not hand membership to the wrong person.
            if !email.eq_ignore_ascii_case(&pending.email) {
                return Err(ApiError::denied(codes::FORBIDDEN, &request_id));
            }
            user_id
        }
        None => {
            match repo::user_by_email(tx.as_mut(), &pending.email)
                .await
                .map_err(|error| internal(&error, "finding the invitee", &request_id))?
            {
                Some(existing) => existing,
                None => {
                    let display = body
                        .display_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| local_part(&pending.email));
                    repo::insert_user(tx.as_mut(), &pending.email, display)
                        .await
                        .map_err(|error| {
                            internal(&error, "creating the invited account", &request_id)
                        })?
                }
            }
        }
    };

    // Burn FIRST, inside the transaction. `consume_invitation` updates only a
    // row that is still pending, so two concurrent acceptances both find a live
    // invitation and exactly one proceeds — and the loser changes nothing
    // rather than both adding a membership and the second silently winning.
    let burned = repo::consume(tx.as_mut(), pending.id)
        .await
        .map_err(|error| internal(&error, "spending the invitation", &request_id))?;
    if !burned {
        return Ok(ApiError::unauthenticated(&request_id).into_response());
    }

    // The scope is minted here, before the membership exists, and made true by
    // this transaction — the same bootstrap `crate::workspaces::create` uses
    // for the creator's own membership. The invitation is the authority: it was
    // issued by someone already inside, and it has just been verified.
    let workspace = WorkspaceId::from_uuid(pending.workspace_id);
    let context = AuthContext::authenticated(
        casual_task_model::UserId::from_uuid(user_id),
        workspace,
        casual_task_model::ActorType::User,
    );
    let scope = context.scope();
    let mut scoped = Scoped::apply(&mut tx, &scope)
        .await
        .map_err(|error| internal(&error, "applying the tenant scope", &request_id))?;

    workspace_repo::insert_member(&mut scoped, user_id, "MEMBER")
        .await
        .map_err(|error| internal(&error, "adding the member", &request_id))?;

    if let Some(role_id) = pending.role_id {
        // `granted_by` is the INVITER, read back from the row. The audit
        // question is "who gave them this authority", and the answer is never
        // "they did" — recording the acceptor would read, years later, as a
        // self-grant. It falls back to the acceptor only if the inviter's
        // account has since been deleted, which `invited_by`'s nullable
        // foreign key permits.
        let inviter = repo::inviter_of(&mut scoped, pending.id)
            .await
            .map_err(|error| internal(&error, "reading the inviter", &request_id))?
            .unwrap_or(user_id);
        repo::assign_role(&mut scoped, user_id, role_id, inviter)
            .await
            .map_err(|error| internal(&error, "assigning the invited role", &request_id))?;
    }

    // docs/04 §Caching: bumped in the same transaction as the change, so a
    // stale permission-cache entry cannot be read — the key simply misses.
    workspace_repo::bump_authz_epoch(&mut scoped)
        .await
        .map_err(|error| internal(&error, "bumping authz_epoch", &request_id))?;

    let who = Provenance {
        actor: Some(casual_task_model::UserId::from_uuid(user_id)),
        actor_type: casual_task_model::ActorType::User,
        request_id: Uuid::parse_str(&request_id)
            .ok()
            .map(casual_task_model::RequestId::from_uuid),
        correlation_id: None,
        ip: crate::auth::client_ip(&headers),
        user_agent: headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned),
    };
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "workspace".to_owned(),
            aggregate_id: pending.workspace_id,
            project_id: None,
            event_type: "user.invitation.accepted".to_owned(),
            activity_changes: serde_json::json!({ "user_id": user_id }),
            audit_changes: serde_json::json!({
                "before": serde_json::Value::Null,
                "after": { "user_id": user_id, "role_id": pending.role_id },
            }),
            payload: serde_json::json!({
                "workspace_id": pending.workspace_id,
                "user_id": user_id,
                "invitation_id": pending.id,
            }),
            schema_version: SCHEMA_VERSION,
        },
        &who,
    )
    .await
    .map_err(|error| internal(&error, "recording the acceptance", &request_id))?;

    unit::commit(tx, &request_id).await?;

    // No session is issued. Accepting proves control of a mailbox, not of a
    // password — signing the caller in here would turn a forwarded email into
    // an authenticated session, which is the attack the address check above
    // exists to stop. The client sends them to sign in, or to set a password
    // through the reset flow if the account was just created.
    Ok((
        StatusCode::OK,
        axum::Json(AcceptedInvitation {
            workspace_id: pending.workspace_id,
            user_id,
        }),
    )
        .into_response())
}

/// Require `role.assign`, then control 1 of `docs/04`'s grant ceiling.
///
/// Split out so the two halves are visible as two rules rather than one
/// condition: the first is "may you grant roles at all", the second is "may you
/// grant *this* one".
async fn authorize_role_grant(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    role_id: Uuid,
    request_id: &str,
) -> Result<(), ApiError> {
    // Control 2 — the scope-appropriate assign permission. An invitation
    // carrying a role creates a WORKSPACE-scope grant on acceptance, and
    // `docs/04` names `role.assign` for exactly that.
    unit::authorized(
        ctx.authority.may_in_workspace(permission::ROLE_ASSIGN),
        request_id,
    )?;

    if !repo::role_exists(scoped, role_id)
        .await
        .map_err(|error| internal(&error, "checking the role", request_id))?
    {
        // 422, not 404: the id is well formed, it names nothing here. Refused
        // at invite time so a bad role is not discovered at acceptance time,
        // when the invitee would join with no role and nobody would know why.
        return Err(ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "No such role in this workspace",
            request_id,
        ));
    }

    let held = repo::role_permissions(scoped, role_id)
        .await
        .map_err(|error| internal(&error, "reading the role's permissions", request_id))?;

    // Control 1 — you cannot grant what you do not hold, checked permission by
    // permission so the refusal names the one that failed.
    for key in &held {
        // Against the CLOSED registry: a permission string in the database that
        // is not in `Permission::ALL` fails closed rather than being waved
        // through, because an unknown authority is one nobody has reasoned about.
        let Some(known) = permission::ALL.iter().find(|p| p.as_str() == key) else {
            tracing::error!(
                permission = key,
                "a role carries a permission not in the registry"
            );
            return Err(ApiError::denied(codes::NO_GRANT, request_id));
        };
        unit::authorized(ctx.authority.may_in_workspace(*known), request_id)?;
    }
    Ok(())
}

/// Hand the message to the relay, off the request path.
fn deliver(mailer: std::sync::Arc<dyn Mailer>, message: Message) {
    tokio::spawn(async move {
        if let Err(error) = mailer.send(&message).await {
            // `message` is safe to log: its Debug redacts the body, which is
            // the half that carries the token.
            tracing::error!(%error, ?message, "an invitation email was not delivered");
        }
    });
}

/// The email body. A link and nothing else sensitive.
///
/// No workspace name and no inviter name: this is delivered to an address
/// nobody has yet proved they control, so it must not reveal who is working
/// with whom. `docs/29` §Email content governs notification mail; the same
/// reasoning applies harder here.
#[must_use]
pub fn invite_body(public_url: &str, token: &str) -> String {
    format!(
        "You have been invited to a workspace on TaskForge.\n\
         \n\
         Open this link to accept:\n\
         {}{ACCEPT_PATH}?token={token}\n\
         \n\
         The link works once and expires in seven days. It is tied to this\n\
         email address and cannot be used with a different account.\n\
         \n\
         If you were not expecting this, you can ignore this message.\n",
        public_url.trim_end_matches('/')
    )
}

/// The signed-in user, if the request carries a live session.
///
/// Read directly rather than through the [`crate::middleware::Authenticated`]
/// extractor, because this endpoint must work **without** a credential and an
/// extractor that rejects cannot express "optional".
async fn current_user(
    conn: &mut sqlx::PgConnection,
    headers: &HeaderMap,
) -> Result<Option<(Uuid, String)>, ApiError> {
    let Some(selector) = crate::auth::session_selector(headers) else {
        return Ok(None);
    };
    let Some(session) = identity::live_session(conn, &selector)
        .await
        .map_err(|error| {
            tracing::error!(%error, "session lookup failed");
            ApiError::internal("invitation")
        })?
    else {
        return Ok(None);
    };
    let email = identity::email_of(conn, session.user_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the signed-in address failed");
            ApiError::internal("invitation")
        })?;
    Ok(email.map(|email| (session.user_id, email)))
}

/// The part before the `@`, as a default display name.
fn local_part(email: &str) -> &str {
    email.split('@').next().unwrap_or(email)
}

/// Reject an address this system cannot send to.
///
/// Deliberately minimal — one `@`, no whitespace, no control characters, and a
/// length bound. A full RFC 5322 validator rejects addresses that work, and the
/// real test of an address is whether mail reaches it. What this **must** catch
/// is the newline that would let an address carry its own headers; that is also
/// refused by `casual-task-infra`, and refusing it twice is cheaper than
/// deciding which layer owns it.
fn valid_email<'a>(email: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = email.trim();
    let refuse = || {
        ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "That is not an email address this system can send to",
            request_id,
        )
    };
    if trimmed.is_empty() || trimmed.len() > 320 {
        return Err(refuse());
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(refuse());
    }
    let mut parts = trimmed.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(refuse());
    };
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Err(refuse());
    }
    Ok(trimmed)
}

fn provenance_of(ctx: &Context) -> Provenance {
    ctx.provenance.clone()
}

fn body_of(record: &repo::InvitationRecord) -> InvitationBody {
    InvitationBody {
        id: record.id,
        email: record.email.clone(),
        role_id: record.role_id,
        invited_by: record.invited_by,
        expires_at: record
            .expires_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| String::new()),
        created_at: record
            .created_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| String::new()),
    }
}

fn internal(error: &sqlx::Error, doing: &str, request_id: &str) -> ApiError {
    tracing::error!(%error, doing, "invitation request failed");
    ApiError::internal(request_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_email_carries_the_link_and_nothing_about_the_workspace() {
        // Delivered to an address nobody has proved they control. A workspace
        // name here reveals who is working with whom to whoever holds the
        // mailbox — or to whoever the mail was forwarded to.
        let body = invite_body("https://tasks.example.com", "abc.def");
        assert!(body.contains("https://tasks.example.com/accept-invitation?token=abc.def"));
        assert!(
            body.contains("once"),
            "the single-use property is not stated"
        );
        assert!(body.contains("seven days"), "the expiry is not stated");
        assert!(
            body.contains("tied to this"),
            "the address binding is not stated"
        );
    }

    #[test]
    fn a_trailing_slash_on_the_public_url_does_not_double() {
        let body = invite_body("https://tasks.example.com/", "abc.def");
        assert!(body.contains("com/accept-invitation?"), "{body}");
        assert!(!body.contains("com//"), "{body}");
    }

    #[test]
    fn the_subject_is_something_casual_task_infra_will_accept() {
        assert!(INVITE_SUBJECT.is_ascii());
        assert!(!INVITE_SUBJECT.chars().any(|c| c.is_ascii_control()));
    }

    #[test]
    fn the_acceptance_message_reveals_nothing() {
        // Byte-identical for an address with an account and one without, which
        // is the docs/40 enumeration gate this endpoint had to close.
        assert!(Accepted::TEXT.starts_with("If that address"));
    }

    #[test]
    fn an_address_that_could_carry_a_header_is_refused() {
        // The injection case. casual-task-infra refuses it too; refusing twice
        // is cheaper than deciding which layer owns it.
        for bad in [
            "user@example.com\r\nBcc: attacker@example.com",
            "user@example.com\nBcc: x@y.com",
            "user @example.com",
            "",
            "no-at-sign",
            "a@b@c.com",
            "@example.com",
            "user@",
            "user@nodot",
        ] {
            assert!(valid_email(bad, "r").is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn an_ordinary_address_is_accepted() {
        // The companion: a validator that refused everything would satisfy the
        // test above and break every invitation.
        for good in [
            "user@example.com",
            "first.last+tag@sub.example.co.uk",
            "  spaced@example.com  ",
        ] {
            assert_eq!(valid_email(good, "r").expect("accepted"), good.trim());
        }
    }

    #[test]
    fn a_display_name_falls_back_to_the_local_part() {
        assert_eq!(local_part("ada@example.com"), "ada");
        assert_eq!(local_part("malformed"), "malformed");
    }
}
