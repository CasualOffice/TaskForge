//! Proving a factor now: step-up, replay refusal, and recovery codes.
//!
//! # The replay refusal is the reason this module is separate
//!
//! RFC 6238 §5.2: a TOTP code is valid for a whole 30-second step, so an
//! attacker who observes one can present it inside the same window.
//! `casual-task-identity` was built for this — `Totp::verify` returns the
//! matched **step** rather than a bool, and says in its own documentation that
//! the caller must reject a step it has already accepted. `prove` is that
//! caller, and it is the only one, so the check cannot be present on one path
//! and missing on another.
//!
//! The refusal itself is a `WHERE` clause in the repository
//! (`mfa::accept_step`), not a read-then-write here: two requests carrying the
//! same observed code would both pass a read-side check before either wrote.
//!
//! # One refusal, whatever the reason
//!
//! Wrong code, replayed code, wrong recovery code, and an account with no
//! factor at all produce the identical 401. A "that code was already used"
//! message tells an attacker holding an observed code that they had the right
//! one and were merely late — which is precisely the information the replay
//! refusal exists to deny them.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_identity::mfa::Totp;
use casual_task_identity::password;
use casual_task_observability::Redacted;
use casual_task_persistence::mfa as repo;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{ApiError, codes};
use crate::json::ValidJson;
use crate::middleware::Authenticated;
use crate::server::{AppState, RequestId};

use super::enrol::{audit, bad_code};
use super::policy::internal;
use super::wire::StepUp;

/// `POST /api/v1/auth/mfa/step-up` — satisfy this workspace's MFA requirement.
///
/// Marks the **session**, not the user. `docs/40` §Workspace-level SSO and MFA
/// step-up puts the assertion on the session, so a step-up performed in one
/// browser is not inherited by another.
///
/// # Errors
///
/// `400` when neither a code nor a recovery code is supplied, `401` when the
/// proof does not verify, `409` when the caller is not using a session.
pub async fn step_up(
    State(state): State<AppState>,
    actor: Authenticated,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<StepUp>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let Some(session) = actor.session_id else {
        // A bearer token has no session to mark, and `docs/40` scopes MFA to
        // browser sessions. Refusing here is clearer than marking nothing and
        // reporting success to a caller who will be refused again immediately.
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            codes::MFA_REQUIRED,
            "Step-up applies to a browser session, not to a token",
            &request_id,
        ));
    };

    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(&request_id, 5))?;
    let user = actor.actor_id.as_uuid();

    let outcome = prove(&mut conn, user, &body, &request_id).await;
    match outcome {
        Ok(proof) => {
            repo::mark_session_satisfied(&mut conn, session, OffsetDateTime::now_utc())
                .await
                .map_err(|error| internal(&error, "marking the session", &request_id))?;
            audit(&mut conn, user, proof.audit_event(), &headers, &request_id).await;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        Err(error) => {
            // Audited on failure too. `docs/40`: a burst of these is the
            // clearest signal that someone is guessing at a second factor, and
            // it is invisible if only successes are recorded.
            audit(&mut conn, user, "mfa.failed", &headers, &request_id).await;
            Err(error)
        }
    }
}

/// `POST /api/v1/auth/mfa/recovery` — the same, spelled for a recovery code.
///
/// A separate route rather than a flag, because a client showing "lost your
/// device?" is on a different screen and `docs/05` prefers a URL that says what
/// it does. It funnels into the same `prove`.
///
/// # Errors
///
/// As [`step_up`].
pub async fn verify_recovery_code(
    state: State<AppState>,
    actor: Authenticated,
    request_id: RequestId,
    headers: HeaderMap,
    body: ValidJson<StepUp>,
) -> Result<Response, ApiError> {
    step_up(state, actor, request_id, headers, body).await
}

/// Which proof was accepted, for the audit trail.
///
/// An enum rather than a bool: "they used a recovery code" is materially
/// different from "they used their phone" to whoever reads the trail after an
/// incident, and a recovery-code redemption is often the first visible sign
/// that a device was lost or stolen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proof {
    Totp,
    RecoveryCode,
}

impl Proof {
    const fn audit_event(self) -> &'static str {
        match self {
            Self::Totp => "mfa.satisfied",
            Self::RecoveryCode => "mfa.recovery_code.used",
        }
    }
}

/// Verify a TOTP code or a recovery code. **The only place either is checked.**
///
/// # Errors
///
/// `400` when neither field is supplied, `401` for every failure — see the
/// module docs on why they are indistinguishable.
pub(crate) async fn prove(
    conn: &mut sqlx::PgConnection,
    user: Uuid,
    body: &StepUp,
    request_id: &str,
) -> Result<Proof, ApiError> {
    let code = body
        .code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let recovery = body
        .recovery_code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());

    match (code, recovery) {
        (None, None) => Err(ApiError::bad_request(
            codes::MISSING_FIELD,
            "Supply either code or recovery_code",
            request_id,
        )
        .with_details(serde_json::json!({ "one_of": ["code", "recovery_code"] }))),
        // Both supplied is a client bug, and guessing which one they meant
        // would silently ignore the other — including the case where the
        // ignored one was the correct one and the caller cannot see why.
        (Some(_), Some(_)) => Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "Supply either code or recovery_code, not both",
            request_id,
        )),
        (Some(code), None) => verify_totp(conn, user, code, request_id).await,
        (None, Some(recovery)) => redeem(conn, user, recovery, request_id).await,
    }
}

/// A TOTP code, with the replay refusal.
async fn verify_totp(
    conn: &mut sqlx::PgConnection,
    user: Uuid,
    code: &str,
    request_id: &str,
) -> Result<Proof, ApiError> {
    // `confirmed_factor`, never `pending_factor`: an unconfirmed factor must
    // never satisfy MFA. The rule is in the query, so this call site cannot
    // get it wrong.
    let Some(factor) = repo::confirmed_factor(conn, user)
        .await
        .map_err(|error| internal(&error, "reading the factor", request_id))?
    else {
        return Err(bad_code(request_id));
    };

    let secret = Redacted::new(
        repo::factor_secret(conn, factor.id)
            .await
            .map_err(|error| internal(&error, "reading the factor secret", request_id))?
            .unwrap_or_default(),
    );
    let Ok(totp) = Totp::from_base32(secret.expose()) else {
        tracing::error!(factor = %factor.id, "the stored MFA secret is malformed");
        return Err(ApiError::internal(request_id));
    };

    let Some(step) = totp.verify(code, OffsetDateTime::now_utc()) else {
        return Err(bad_code(request_id));
    };

    // RFC 6238 §5.2. `accept_step` refuses a step at or below the highest
    // already accepted, in the UPDATE's own predicate, so an observed code
    // cannot be replayed inside its window even by a concurrent request.
    if !repo::accept_step(conn, factor.id, step)
        .await
        .map_err(|error| internal(&error, "recording the accepted step", request_id))?
    {
        // Deliberately the same refusal as a wrong code. See the module docs.
        return Err(bad_code(request_id));
    }
    Ok(Proof::Totp)
}

/// A recovery code: Argon2 comparison against every unused code, then burn.
///
/// Compared against all of them because the hash is salted per row and the code
/// therefore cannot be looked up by value. At most ten rows, served by the
/// partial index migration 0016 created for exactly this.
async fn redeem(
    conn: &mut sqlx::PgConnection,
    user: Uuid,
    presented: &str,
    request_id: &str,
) -> Result<Proof, ApiError> {
    // Recovery codes only exist alongside a confirmed factor, so this also
    // stops an account that abandoned enrolment from being satisfied by a
    // leftover code.
    if repo::confirmed_factor(conn, user)
        .await
        .map_err(|error| internal(&error, "reading the factor", request_id))?
        .is_none()
    {
        return Err(bad_code(request_id));
    }

    let candidates = repo::unused_recovery_codes(conn, user)
        .await
        .map_err(|error| internal(&error, "reading recovery codes", request_id))?;

    // Normalized the way people type them: recovery codes are read off a screen
    // and typed back, and refusing a correct code because of case or a pasted
    // space is a support ticket, not a security control.
    let presented = presented.replace([' ', '-'], "").to_uppercase();

    // OFF THE ASYNC RUNTIME, for the same reason `password::verify_async`
    // exists: this is up to ten Argon2id verifications at 64 MB, so a second of
    // pure CPU. Run inline it blocks a tokio worker for that whole time, and a
    // burst of redemption attempts stalls every unrelated request on the
    // runtime — which would make this endpoint a cheap denial-of-service lever.
    let matched = tokio::task::spawn_blocking(move || {
        let mut found = None;
        for (id, hash) in &candidates {
            // Every candidate is compared even after a match. Returning early
            // would make the response time a function of the code's position in
            // the list, which leaks how many codes have been used.
            if password::verify(&presented, hash).unwrap_or(false) && found.is_none() {
                found = Some(*id);
            }
        }
        found
    })
    .await
    .map_err(|_| {
        tracing::error!("the blocking pool failed while checking a recovery code");
        ApiError::internal(request_id)
    })?;

    let Some(id) = matched else {
        return Err(bad_code(request_id));
    };

    // Single use as a `WHERE` clause: two requests presenting the same code
    // both find it unused, and exactly one burns it.
    if !repo::redeem_recovery_code(conn, id)
        .await
        .map_err(|error| internal(&error, "redeeming the recovery code", request_id))?
    {
        return Err(bad_code(request_id));
    }
    Ok(Proof::RecoveryCode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recovery_code_use_is_audited_differently_from_a_phone() {
        // Whoever reads the trail after an incident needs these apart: a
        // recovery-code redemption is often the first visible sign that a
        // device was lost or stolen.
        assert_ne!(Proof::Totp.audit_event(), Proof::RecoveryCode.audit_event());
        assert_eq!(Proof::RecoveryCode.audit_event(), "mfa.recovery_code.used");
    }
}
