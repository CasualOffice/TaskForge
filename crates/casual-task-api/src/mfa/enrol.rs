//! Enrolling a factor, and removing one (`docs/40` §MFA).
//!
//! # The ceremony is two steps because one step locks people out
//!
//! `begin` stores an **unconfirmed** factor and hands back the secret.
//! `confirm` takes a code computed from that secret and only then sets
//! `confirmed_at`. Until it does, the factor satisfies nothing.
//!
//! Migration 0016's own comment says why: "a user who lost the enrolment
//! halfway would otherwise be locked out by a factor they do not have." Someone
//! who scans a QR code and closes the tab, or whose phone clock is wrong, must
//! end up exactly where they started — not holding a factor they cannot produce
//! codes for.
//!
//! # The secret is returned once and wrapped everywhere else
//!
//! `begin` is the only code path in this crate that returns the shared secret.
//! Every other use wraps it in `Redacted`, whose `Debug`, `Display` and
//! `Serialize` all print `<redacted>` — so the leak `docs/46` cares about is
//! prevented by the type rather than by remembering.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_identity::mfa::{DIGITS, STEP_SECONDS, Totp, issue_recovery_codes};
use casual_task_observability::Redacted;
use casual_task_persistence::{identity, mfa as repo};

use crate::error::{ApiError, codes};
use crate::json::ValidJson;
use crate::middleware::Authenticated;
use crate::server::{AppState, RequestId};

use super::policy::internal;
use super::wire::{
    ConfirmEnrolment, EnrolmentStarted, MfaStatus, RecoveryCodesIssued, provisioning_uri,
};

/// What an authenticator app shows as the account issuer.
const ISSUER: &str = "TaskForge";

/// `GET /api/v1/auth/mfa` — what this account currently has.
///
/// # Errors
///
/// A database failure.
pub async fn status(
    State(state): State<AppState>,
    actor: Authenticated,
    request_id: RequestId,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(&request_id, 5))?;

    let user = actor.actor_id.as_uuid();
    let enrolled = repo::has_confirmed_factor(&mut conn, user)
        .await
        .map_err(|error| internal(&error, "reading the factor", &request_id))?;
    let pending = repo::pending_factor(&mut conn, user)
        .await
        .map_err(|error| internal(&error, "reading the pending factor", &request_id))?
        .is_some();
    let remaining = repo::remaining_recovery_codes(&mut conn, user)
        .await
        .map_err(|error| internal(&error, "counting recovery codes", &request_id))?;

    Ok((
        StatusCode::OK,
        axum::Json(MfaStatus {
            enrolled,
            pending,
            recovery_codes_remaining: remaining,
            session_satisfied: actor.mfa_satisfied_at.is_some(),
        }),
    )
        .into_response())
}

/// `POST /api/v1/auth/mfa/enrolment` — begin.
///
/// Refuses when a **confirmed** factor already exists: re-enrolling over a
/// working factor without proving control of the current one would make this
/// endpoint a way to displace it, and displacing it is what an attacker holding
/// a stolen session would want.
///
/// # Errors
///
/// `409` when a confirmed factor exists, `503` when the database is
/// unreachable, `500` when the randomness source fails.
pub async fn begin(
    State(state): State<AppState>,
    actor: Authenticated,
    request_id: RequestId,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(&request_id, 5))?;
    let user = actor.actor_id.as_uuid();

    if repo::has_confirmed_factor(&mut conn, user)
        .await
        .map_err(|error| internal(&error, "checking for an existing factor", &request_id))?
    {
        return Err(ApiError::conflict(
            codes::MFA_ALREADY_ENROLLED,
            "Multi-factor authentication is already enabled for this account",
            &request_id,
        ));
    }

    let totp = Totp::generate().map_err(|error| {
        tracing::error!(%error, "the randomness source failed");
        ApiError::internal(&request_id)
    })?;
    // Wrapped the moment it exists. Everything below passes the wrapper around
    // and calls `expose()` exactly where the value is genuinely needed — which
    // is the response body and the database, and nowhere else.
    let secret = Redacted::new(totp.to_base32());

    repo::begin_enrolment(&mut conn, user, secret.expose())
        .await
        .map_err(|error| internal(&error, "storing the pending factor", &request_id))?;

    let email = identity::email_of(&mut conn, user)
        .await
        .map_err(|error| internal(&error, "reading the account address", &request_id))?
        .unwrap_or_else(|| user.to_string());

    // Recorded, because starting enrolment is a change to how an account
    // authenticates and `docs/40` §What is audited wants those. The secret is
    // NOT in the event — the whole point of the wrapper.
    audit(
        &mut conn,
        user,
        "mfa.enrolment.started",
        &headers,
        &request_id,
    )
    .await;

    Ok((
        StatusCode::OK,
        axum::Json(EnrolmentStarted {
            provisioning_uri: provisioning_uri(
                ISSUER,
                &email,
                secret.expose(),
                DIGITS,
                STEP_SECONDS,
            ),
            secret: secret.into_inner(),
            period_seconds: STEP_SECONDS,
            digits: DIGITS,
        }),
    )
        .into_response())
}

/// `POST /api/v1/auth/mfa/enrolment/confirm` — prove it, and get the codes.
///
/// The code is verified against the **pending** factor, and the step it matched
/// is stored as `last_step` in the same statement that confirms it — so the
/// code used to enrol cannot immediately be replayed as a step-up.
///
/// # Errors
///
/// `404` when no enrolment is pending, `401` when the code does not verify.
pub async fn confirm(
    State(state): State<AppState>,
    actor: Authenticated,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<ConfirmEnrolment>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(&request_id, 5))?;
    let user = actor.actor_id.as_uuid();

    let Some(factor) = repo::pending_factor(&mut conn, user)
        .await
        .map_err(|error| internal(&error, "reading the pending factor", &request_id))?
    else {
        // 404 rather than 409: from the caller's side there is nothing to
        // confirm, and saying "you already confirmed it" would be a different
        // fact they can get from `GET /api/v1/auth/mfa`.
        return Err(ApiError::not_found(&request_id));
    };

    let secret = Redacted::new(
        repo::factor_secret(&mut conn, factor.id)
            .await
            .map_err(|error| internal(&error, "reading the factor secret", &request_id))?
            .unwrap_or_default(),
    );
    let Ok(totp) = Totp::from_base32(secret.expose()) else {
        // A row damaged by a bad migration. Fails closed and loudly, rather
        // than reporting "wrong code" forever to a user whose code is right.
        tracing::error!(factor = %factor.id, "the stored MFA secret is malformed");
        return Err(ApiError::internal(&request_id));
    };

    let Some(step) = totp.verify(body.code.trim(), time::OffsetDateTime::now_utc()) else {
        audit(
            &mut conn,
            user,
            "mfa.enrolment.failed",
            &headers,
            &request_id,
        )
        .await;
        return Err(bad_code(&request_id));
    };

    if !repo::confirm_factor(&mut conn, factor.id, step)
        .await
        .map_err(|error| internal(&error, "confirming the factor", &request_id))?
    {
        // Something confirmed it between the read and the write. The caller's
        // factor is enrolled either way, which is what they asked for.
        return Err(ApiError::conflict(
            codes::MFA_ALREADY_ENROLLED,
            "Multi-factor authentication is already enabled for this account",
            &request_id,
        ));
    }

    // `docs/40` §MFA: "10 single-use recovery codes shown once."
    //
    // OFF THE ASYNC RUNTIME. Each code is hashed with Argon2id at 64 MB, t=3 —
    // ~100 ms of pure CPU with no I/O — and there are ten of them. Run inline on
    // a tokio worker that is one second of a blocked thread, which is the exact
    // failure `password::verify_async` and `hash_chosen_async` were written to
    // avoid: with the default worker count, a handful of concurrent enrolments
    // stalls every task on the runtime, including health checks.
    let codes = tokio::task::spawn_blocking(issue_recovery_codes)
        .await
        .unwrap_or(Err(casual_task_identity::mfa::RecoveryError::Hashing))
        .map_err(|error| {
            tracing::error!(%error, "issuing recovery codes failed");
            ApiError::internal(&request_id)
        })?;
    let hashes: Vec<String> = codes.iter().map(|c| c.hash.clone()).collect();
    repo::replace_recovery_codes(&mut conn, user, &hashes)
        .await
        .map_err(|error| internal(&error, "storing recovery codes", &request_id))?;

    // The session that enrolled has, by construction, just proved the factor.
    // Marking it here means a user who enrols in order to enter a workspace
    // that demands MFA is not immediately asked for a second code.
    if let Some(session) = actor.session_id {
        repo::mark_session_satisfied(&mut conn, session, time::OffsetDateTime::now_utc())
            .await
            .map_err(|error| internal(&error, "marking the session", &request_id))?;
    }

    audit(&mut conn, user, "mfa.enrolled", &headers, &request_id).await;

    Ok((
        StatusCode::OK,
        axum::Json(RecoveryCodesIssued {
            recovery_codes: codes.into_iter().map(|c| c.presented).collect(),
        }),
    )
        .into_response())
}

/// `DELETE /api/v1/auth/mfa` — remove the factor and its recovery codes.
///
/// # Requires a current code, not merely a session
///
/// `docs/40` §MFA lists "managing MFA" among the actions that need
/// re-authentication rather than a valid session. Removing a second factor with
/// nothing but a stolen cookie is the single most useful thing an attacker
/// could do with one, so the caller proves the factor they are removing — with
/// a TOTP code or a recovery code, the same two proofs the step-up accepts.
///
/// # Errors
///
/// `404` when there is no factor, `401` when the proof does not verify.
pub async fn disable(
    State(state): State<AppState>,
    actor: Authenticated,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<super::wire::StepUp>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(&request_id, 5))?;
    let user = actor.actor_id.as_uuid();

    if repo::confirmed_factor(&mut conn, user)
        .await
        .map_err(|error| internal(&error, "reading the factor", &request_id))?
        .is_none()
    {
        return Err(ApiError::not_found(&request_id));
    }

    super::challenge::prove(&mut conn, user, &body, &request_id).await?;

    repo::disable(&mut conn, user)
        .await
        .map_err(|error| internal(&error, "removing the factor", &request_id))?;

    audit(&mut conn, user, "mfa.removed", &headers, &request_id).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The refusal for a code that did not verify.
///
/// One shape for every reason — wrong code, replayed code, wrong recovery code
/// — so a caller cannot learn which of them applied. A "that code was already
/// used" message would tell an attacker holding an observed code that they had
/// the right one and were merely late.
pub(crate) fn bad_code(request_id: &str) -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        codes::MFA_CODE_INVALID,
        "That code is not valid",
        request_id,
    )
}

/// Write an `auth_event`. Best effort at the call site is not acceptable —
/// `docs/40` §What is audited lists `auth.mfa.enrolled` and `auth.mfa.removed`
/// — so a failure is logged loudly rather than discarded silently.
pub(crate) async fn audit(
    conn: &mut sqlx::PgConnection,
    user: uuid::Uuid,
    event: &str,
    headers: &HeaderMap,
    request_id: &str,
) {
    if let Err(error) = identity::record_auth_event(
        conn,
        Some(user),
        None,
        event,
        crate::auth::client_ip(headers).as_deref(),
        headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok()),
    )
    .await
    {
        tracing::error!(%error, event, request_id, "the authentication trail was not written");
    }
}
