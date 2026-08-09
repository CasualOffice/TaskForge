//! `/api/v1/me` — a person's own account (C-001, `docs/40`).
//!
//! # Why this is outside the tenant boundary
//!
//! Every other read in the product is scoped to a workspace, because
//! `docs/32` says no query may address data without one. An account is the
//! exception by construction: a person belongs to many workspaces (`docs/03`),
//! and "who am I" has the same answer in all of them. So these handlers take a
//! connection rather than a `Scoped`, and every one of them answers **only
//! about the caller** — there is no `user_id` parameter anywhere in this file,
//! which is what makes the absence of a workspace scope safe.
//!
//! # The time zone is not a preference
//!
//! `casual-task-search`'s resolver takes a `UtcOffset` and has no default,
//! deliberately: `docs/27` says "`due before @today` must mean the same thing
//! to someone in Auckland and someone in Los Angeles. Server-local date
//! boundaries are a classic and extremely confusing bug." Until now the API
//! passed `UtcOffset::UTC` at every call site, because nothing stored a zone —
//! so the type made the mistake impossible to write by accident and the
//! application made it anyway.
//!
//! Storing an IANA name rather than an offset is the point: an offset changes
//! twice a year in most of the world, and a stored one would drift from the
//! user's real day boundary every time it did. Deriving the offset from the
//! name needs a time-zone database, which is **D-065** and not a dependency
//! yet, so evaluation uses the offset the client sends.
//!
//! # Changing a password is not the same as resetting one
//!
//! A reset proves control of an email address. A change proves control of the
//! account **right now**, which is why it requires the current password even
//! though the caller is already signed in — a borrowed laptop is exactly the
//! case it exists for. `docs/40` §Local authentication.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_identity::password;
use casual_task_persistence::identity;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::error::{ApiError, codes};
use crate::middleware::Authenticated;
use crate::server::{AppState, RequestId};
use crate::wire::{self, Body};

/// `PATCH /api/v1/me`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchRequest {
    #[serde(default)]
    pub display_name: Option<String>,
    /// An IANA zone name. `Option<Option<_>>`: absent leaves it alone, `null`
    /// clears it. Those are different — "I have not chosen" and "I no longer
    /// want one set" reach the same state by different routes, and a bare
    /// `Option` cannot tell a missing key from an explicit null.
    #[serde(default, deserialize_with = "wire::double_option")]
    pub time_zone: Option<Option<String>>,
}

/// `POST /api/v1/me/password`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize)]
pub struct MeView {
    pub id: Uuid,
    pub email: Option<String>,
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// `null` means unset, which is **not** UTC. A client that treats it as UTC
    /// is asserting a day boundary nobody chose.
    pub time_zone: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SessionView {
    pub id: Uuid,
    pub auth_method: String,
    pub created_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    /// Whether this is the session making the request. Without it a person
    /// cannot tell which row is the laptop in front of them, and "sign out
    /// everywhere" becomes a guess.
    pub current: bool,
}

/// The longest an IANA zone name may be.
///
/// `America/Argentina/ComodRivadavia` is 32; the longest in the database is
/// under 40. Bounded because every input is (`docs/21`), and because an
/// unbounded string here would be stored on a row read on every request.
const MAX_ZONE: usize = 64;

/// Validate a zone name's *shape*, not its membership.
///
/// Membership needs the tz database this workspace does not have (D-065).
/// Checking the shape still refuses the input that would otherwise be stored
/// and silently never match anything, and refusing a real zone would be worse
/// than accepting a fictional one — so this is deliberately permissive about
/// which names exist and strict about what a name can contain.
fn validated_zone(raw: &str, request_id: &str) -> Result<String, ApiError> {
    let zone = raw.trim();
    if zone.is_empty() {
        return Err(ApiError::bad_request(
            codes::MISSING_FIELD,
            "time_zone must not be empty — send null to clear it",
            request_id,
        ));
    }
    if zone.len() > MAX_ZONE {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            format!("time_zone must be at most {MAX_ZONE} characters"),
            request_id,
        ));
    }
    if !zone
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
    {
        return Err(ApiError::bad_request(
            codes::INVALID_ENUM,
            "time_zone must be an IANA name such as Australia/Sydney",
            request_id,
        ));
    }
    Ok(zone.to_owned())
}

fn view(profile: identity::Profile) -> MeView {
    MeView {
        id: profile.id,
        email: profile.email,
        display_name: profile.display_name,
        avatar_url: profile.avatar_url,
        time_zone: profile.time_zone,
    }
}

/// `GET /api/v1/me`.
///
/// # Errors
///
/// `500` on a database failure. There is no `404`: the caller authenticated, so
/// their account exists by construction.
pub async fn read(
    State(state): State<AppState>,
    caller: Authenticated,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut conn = connection(&state, &request_id).await?;
    let profile = identity::profile(&mut conn, caller.actor_id.as_uuid())
        .await
        .map_err(internal("reading the profile", &request_id))?
        .ok_or_else(|| {
            // The credential resolved a moment ago, so the row is gone or
            // tombstoned underneath us. That is a 500, not a 404: the caller
            // did nothing wrong and there is nothing they can do.
            tracing::error!("an authenticated caller has no account row");
            ApiError::internal(&request_id)
        })?;
    Ok(axum::Json(view(profile)).into_response())
}

/// `PATCH /api/v1/me`.
///
/// # Errors
///
/// `400` for a malformed name or zone, `500` on a database failure.
pub async fn update(
    State(state): State<AppState>,
    caller: Authenticated,
    headers: HeaderMap,
    Body(body): Body<PatchRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    let display_name = match body.display_name.as_deref().map(str::trim) {
        Some("") => {
            return Err(ApiError::bad_request(
                codes::MISSING_FIELD,
                "display_name must not be empty",
                &request_id,
            ));
        }
        other => other,
    };
    let zone = match &body.time_zone {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => Some(Some(validated_zone(raw, &request_id)?)),
    };

    let mut conn = connection(&state, &request_id).await?;
    identity::update_profile(
        &mut conn,
        caller.actor_id.as_uuid(),
        display_name,
        zone.as_ref().map(|z| z.as_deref()),
    )
    .await
    .map_err(internal("updating the profile", &request_id))?;

    let profile = identity::profile(&mut conn, caller.actor_id.as_uuid())
        .await
        .map_err(internal("re-reading the profile", &request_id))?
        .ok_or_else(|| ApiError::internal(&request_id))?;
    Ok(axum::Json(view(profile)).into_response())
}

/// `POST /api/v1/me/password`.
///
/// **Every session ends, including this one.** That is not a choice this
/// handler makes: `user_credential.changed_at` moves to now, and `live_session`
/// refuses any session created before it — migration 0016 calls that "forces
/// re-authentication everywhere on password change". So the caller signs in
/// again, on this device too.
///
/// It is the right stance and worth stating: the reason to change a password is
/// usually that somebody else might know the old one, and "everywhere except
/// where I am standing" is a weaker guarantee than it sounds. The explicit
/// revoke below is belt and braces — it marks the rows revoked rather than
/// leaving them merely unusable, so a session list does not show corpses.
///
/// # Errors
///
/// `400` if the new password is too short, `403` if the current password is
/// wrong, `500` on a database failure.
pub async fn change_password(
    State(state): State<AppState>,
    caller: Authenticated,
    headers: HeaderMap,
    Body(body): Body<ChangePasswordRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let actor = caller.actor_id.as_uuid();
    let mut conn = connection(&state, &request_id).await?;

    let email = identity::email_of(&mut conn, actor)
        .await
        .map_err(internal("reading the account", &request_id))?
        .ok_or_else(|| ApiError::internal(&request_id))?;
    let credential = identity::credential_for_email(&mut conn, &email)
        .await
        .map_err(internal("reading the credential", &request_id))?
        .ok_or_else(|| {
            // An account with no password reached a password change — an SSO
            // account, or one mid-migration. Refusing is right; pretending the
            // current password was wrong would be a lie.
            ApiError::new(
                StatusCode::FORBIDDEN,
                codes::UNAUTHENTICATED,
                "This account does not sign in with a password",
                &request_id,
            )
        })?;

    // Verified off the runtime, like every other Argon2 call: at 64 MiB and
    // t=3 this takes long enough to stall a worker thread, and `docs/30` counts
    // that as a denial-of-service lever rather than a slow request.
    let matches = password::verify_async(&body.current_password, &credential.password_hash)
        .await
        .map_err(|error| {
            tracing::error!(%error, "verifying the current password failed");
            ApiError::internal(&request_id)
        })?;
    if !matches {
        // Deliberately not rate-limited here beyond the edge limiter: this is an
        // authenticated caller proving they are still themselves, not an
        // enumeration surface.
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            codes::UNAUTHENTICATED,
            "The current password is not correct",
            &request_id,
        ));
    }

    let hash = password::hash_chosen_async(&body.new_password)
        .await
        .map_err(|error| match error {
            password::PasswordError::TooShort { minimum } => ApiError::bad_request(
                codes::OUT_OF_RANGE,
                format!("A password must be at least {minimum} characters"),
                &request_id,
            )
            .with_details(serde_json::json!({ "field": "new_password", "min_length": minimum })),
            other => {
                tracing::error!(error = %other, "hashing the new password failed");
                ApiError::internal(&request_id)
            }
        })?;

    identity::set_password(&mut conn, actor, &hash)
        .await
        .map_err(internal("setting the password", &request_id))?;
    identity::revoke_all_sessions(&mut conn, actor, None)
        .await
        .map_err(internal("revoking other sessions", &request_id))?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /api/v1/me/sessions`.
///
/// # Errors
///
/// `500` on a database failure.
pub async fn sessions(
    State(state): State<AppState>,
    caller: Authenticated,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut conn = connection(&state, &request_id).await?;
    let current = caller.session_id;
    let rows = identity::sessions_of(&mut conn, caller.actor_id.as_uuid())
        .await
        .map_err(internal("listing sessions", &request_id))?;

    let data: Vec<SessionView> = rows
        .into_iter()
        .map(|s| SessionView {
            current: Some(s.id) == current,
            id: s.id,
            auth_method: s.auth_method,
            created_at: s.created_at.format(&Rfc3339).unwrap_or_default(),
            last_seen_at: s.last_seen_at.format(&Rfc3339).unwrap_or_default(),
            expires_at: s.expires_at.format(&Rfc3339).unwrap_or_default(),
            ip_address: s.ip_address,
            user_agent: s.user_agent,
        })
        .collect();
    Ok(axum::Json(serde_json::json!({ "data": data })).into_response())
}

/// `DELETE /api/v1/me/sessions/{id}`.
///
/// # Errors
///
/// `404` when the session is not the caller's — never `403`, because telling a
/// caller a session exists but is not theirs is telling them about somebody
/// else's session.
pub async fn revoke_session(
    State(state): State<AppState>,
    caller: Authenticated,
    headers: HeaderMap,
    Path(session_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut conn = connection(&state, &request_id).await?;
    let mine = identity::session_belongs_to(&mut conn, session_id, caller.actor_id.as_uuid())
        .await
        .map_err(internal("checking the session", &request_id))?;
    if !mine {
        return Err(ApiError::missing(codes::NOT_FOUND, &request_id));
    }
    identity::revoke_session(&mut conn, session_id)
        .await
        .map_err(internal("revoking the session", &request_id))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `DELETE /api/v1/me/sessions` — sign out everywhere else.
///
/// The caller's own session survives, which is what `docs/40`'s "sign out
/// everywhere" means in practice: a person doing this because a device was lost
/// does not want to be signed out of the one they are holding.
///
/// # Errors
///
/// `500` on a database failure.
pub async fn revoke_other_sessions(
    State(state): State<AppState>,
    caller: Authenticated,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut conn = connection(&state, &request_id).await?;
    identity::revoke_all_sessions(&mut conn, caller.actor_id.as_uuid(), caller.session_id)
        .await
        .map_err(internal("revoking sessions", &request_id))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn connection(
    state: &AppState,
    request_id: &str,
) -> Result<sqlx::pool::PoolConnection<sqlx::Postgres>, ApiError> {
    state.pool.acquire().await.map_err(|error| {
        tracing::error!(%error, "acquiring a connection failed");
        ApiError::internal(request_id)
    })
}

fn internal(what: &'static str, request_id: &str) -> impl Fn(sqlx::Error) -> ApiError {
    let request_id = request_id.to_owned();
    move |error: sqlx::Error| {
        tracing::error!(%error, what, "a /me request failed");
        ApiError::internal(&request_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zone_name_is_shape_checked_and_bounded() {
        assert_eq!(
            validated_zone("Australia/Sydney", "r").expect("valid"),
            "Australia/Sydney"
        );
        assert_eq!(
            validated_zone("America/Argentina/ComodRivadavia", "r").expect("valid"),
            "America/Argentina/ComodRivadavia"
        );
        // Shape only — membership needs the tz database D-065 records.
        assert!(validated_zone("Mars/Olympus_Mons", "r").is_ok());

        assert!(validated_zone("", "r").is_err());
        assert!(validated_zone("   ", "r").is_err());
        assert!(validated_zone("Europe/London; DROP TABLE", "r").is_err());
        assert!(validated_zone(&"a".repeat(MAX_ZONE + 1), "r").is_err());
    }

    #[test]
    fn nothing_in_this_module_takes_a_user_id_from_the_caller() {
        // The whole reason these handlers may live outside the tenant boundary
        // is that they answer only about the caller. A parameter naming another
        // user would turn every one of them into a directory.
        let source = include_str!("me.rs");
        let signature = format!("Path(user{}id)", "_");
        assert!(
            !source.contains(&signature),
            "a /me handler takes a user id; it must only ever answer about the caller"
        );
    }
}
