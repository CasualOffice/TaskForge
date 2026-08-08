//! Authentication, workspace resolution, and CSRF enforcement.
//!
//! # Two questions, not one (`docs/40` §Workspace-level SSO and MFA step-up)
//!
//! "Signed in" and "may enter this workspace" are separate, because the browser
//! session is **user**-scoped — `user_account` is the one table without a
//! `workspace_id`, since a person spans workspaces — while membership, SSO
//! enforcement and MFA policy are per workspace.
//!
//! So there are two extractors, and the split is the design:
//!
//! - [`Authenticated`] — a live session or token. Knows *who*, not *where*.
//! - [`WorkspaceMember`] — the above, plus a workspace the actor is a member
//!   of, validated on this request. It is the **only** thing in this crate that
//!   mints an [`AuthContext`], which is what makes `WorkspaceScope` unforgeable
//!   everywhere else (`docs/32`).
//!
//! A handler that takes `Authenticated` can never reach tenant data, because it
//! has no `AuthContext` to build a scope from. That is a compile-time property,
//! not a review note.

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use casual_task_identity::credential;
use casual_task_model::{ActorType, AuthContext, UserId, WorkspaceId};
use casual_task_persistence::{auth as token_auth, identity};

use crate::auth::session_selector;
use crate::csrf;
use crate::error::ApiError;
use crate::server::AppState;

/// The header carrying the workspace a request is for.
///
/// `docs/05` §Authentication: "Workspace is determined by the path or an
/// `X-Workspace-Id` header, and is validated against membership on every
/// request — never trusted from the client."
pub const WORKSPACE_HEADER: &str = "x-workspace-id";

/// A caller with a live credential. Knows **who**, not **where**.
#[derive(Debug, Clone, Copy)]
pub struct Authenticated {
    pub actor_id: UserId,
    pub actor_type: ActorType,
    /// The session row, when the credential was a session cookie. `None` for a
    /// bearer token, which has no session.
    pub session_id: Option<uuid::Uuid>,
}

/// A caller who is a member of a specific workspace.
///
/// Holds the only [`AuthContext`] this crate produces.
///
/// `Clone` but not `Copy`: `AuthContext` is deliberately not `Copy` so that a
/// scope cannot be duplicated implicitly by a stray dereference.
#[derive(Debug, Clone)]
pub struct WorkspaceMember {
    pub context: AuthContext,
}

impl FromRequestParts<AppState> for Authenticated {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let request_id = "auth";
        let mut conn = state
            .pool
            .acquire()
            .await
            .map_err(|_| ApiError::unavailable(request_id, 5))?;

        // A session cookie first, then a bearer token. Both fail the same way:
        // 401 with one shape, so a caller cannot learn which credential type
        // the server recognised.
        if let Some(selector) = session_selector(&parts.headers)
            && let Some(session) =
                identity::live_session(&mut conn, &selector)
                    .await
                    .map_err(|error| {
                        tracing::error!(%error, "session lookup failed");
                        ApiError::internal(request_id)
                    })?
        {
            {
                let presented = presented_session(&parts.headers).unwrap_or_default();
                let (_, verifier) = credential::split(&presented)
                    .map_err(|_| ApiError::unauthenticated(request_id))?;
                if credential::verify(verifier, &session.verifier_hash) {
                    // Best effort: a failed `last_seen_at` update must not fail
                    // an otherwise valid request.
                    let _ = identity::touch_session(&mut conn, session.id).await;
                    return Ok(Self {
                        actor_id: UserId::from_uuid(session.user_id),
                        actor_type: ActorType::User,
                        session_id: Some(session.id),
                    });
                }
            }
        }

        if let Some(presented) = bearer(&parts.headers) {
            let (selector, verifier) =
                credential::split(&presented).map_err(|_| ApiError::unauthenticated(request_id))?;
            // Through the pre-workspace seam: a token carries a workspace, but
            // the request has no scope yet to read it with (ADR-032).
            if let Some(token) = token_auth::lookup_token(&mut conn, selector)
                .await
                .map_err(|error| {
                    tracing::error!(%error, "token lookup failed");
                    ApiError::internal(request_id)
                })?
            {
                let stored = token_auth::lookup_token_verifier(&mut conn, selector)
                    .await
                    .map_err(|error| {
                        tracing::error!(%error, "token verifier lookup failed");
                        ApiError::internal(request_id)
                    })?
                    .unwrap_or_default();
                if credential::verify(verifier, &stored) {
                    return Ok(Self {
                        actor_id: UserId::from_uuid(token.principal_id),
                        actor_type: match token.principal_type.as_str() {
                            "SERVICE_ACCOUNT" => ActorType::ServiceAccount,
                            _ => ActorType::User,
                        },
                        session_id: None,
                    });
                }
            }
        }

        Err(ApiError::unauthenticated(request_id))
    }
}

impl FromRequestParts<AppState> for WorkspaceMember {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let request_id = "auth";
        let actor = Authenticated::from_request_parts(parts, state).await?;

        let workspace = parts
            .headers
            .get(WORKSPACE_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<uuid::Uuid>().ok())
            // A missing or malformed workspace is 404, not 400: docs/04
            // requires "absent" and "invisible" to be indistinguishable, and a
            // 400 here would confirm the header is the way in.
            .ok_or_else(|| ApiError::not_found(request_id))?;

        let mut conn = state
            .pool
            .acquire()
            .await
            .map_err(|_| ApiError::unavailable(request_id, 5))?;

        let member = identity::is_workspace_member(&mut conn, actor.actor_id.as_uuid(), workspace)
            .await
            .map_err(|error| {
                tracing::error!(%error, "membership check failed");
                ApiError::internal(request_id)
            })?;

        if !member {
            // 404, not 403. An authenticated stranger must not be able to
            // discover which workspace ids exist by probing this header
            // (`docs/04`).
            return Err(ApiError::not_found(request_id));
        }

        Ok(Self {
            context: AuthContext::authenticated(
                actor.actor_id,
                WorkspaceId::from_uuid(workspace),
                actor.actor_type,
            ),
        })
    }
}

/// Reject unsafe methods without a valid CSRF token (`docs/05`, `docs/40`).
///
/// Applied as a layer rather than per handler, because "every unsafe method"
/// means the guard has to be somewhere a new route cannot be added *beside*.
///
/// Only session-authenticated requests are checked. A bearer token is not sent
/// automatically by a browser, so it cannot be cross-site forged — and requiring
/// a CSRF token from a service account would be asking a machine to defend
/// against an attack that needs a browser.
pub async fn csrf_guard(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if !csrf::requires_token(request.method()) {
        return next.run(request).await;
    }

    let Some(selector) = session_selector(request.headers()) else {
        // No session cookie: nothing for a browser to forge with.
        return next.run(request).await;
    };

    let presented = request
        .headers()
        .get(csrf::CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if csrf::verify(&state.secret_key, &selector, presented) {
        return next.run(request).await;
    }

    // 403 rather than 401: the caller IS authenticated. Retrying with the same
    // credential and a correct token succeeds, which a 401 would not suggest.
    ApiError::forbidden("csrf").into_response()
}

/// The whole presented session credential, not just the selector.
fn presented_session(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == crate::auth::SESSION_COOKIE)
        .map(|(_, value)| value.to_owned())
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .map(ToOwned::to_owned)
}

/// `GET /api/v1/auth/session` — who the caller is.
///
/// Takes [`Authenticated`], not [`WorkspaceMember`]: answering "who am I" must
/// not require choosing a workspace first, and this endpoint is what a client
/// calls before it knows which workspaces exist.
pub async fn whoami(actor: Authenticated) -> Response {
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "actor_id": actor.actor_id.as_uuid(),
            "actor_type": actor.actor_type.as_audit_str(),
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn a_bearer_token_is_read_from_the_authorization_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc.def"),
        );
        assert_eq!(bearer(&headers).as_deref(), Some("abc.def"));
    }

    #[test]
    fn other_authorization_schemes_are_ignored() {
        // Basic auth reaching the token path would compare a base64 blob
        // against a selector and produce confusing failures.
        for value in ["Basic dXNlcjpwdw==", "bearer lowercase", "abc.def", ""] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::AUTHORIZATION,
                HeaderValue::from_str(value).expect("valid"),
            );
            assert_eq!(bearer(&headers), None, "accepted {value:?}");
        }
    }
}
