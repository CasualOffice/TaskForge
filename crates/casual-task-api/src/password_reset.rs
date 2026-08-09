//! Password reset by email (C-001, `docs/40` §Local authentication).
//!
//! > "Reset tokens: single-use, 1 h, hashed at rest, invalidated by password
//! > change."
//!
//! # Enumeration through reset is the same bug as through login
//!
//! `docs/40` §Acceptance gates names the three endpoints together: "login,
//! reset, and invite responses are indistinguishable for existing and
//! non-existing accounts, in body, status, and timing envelope".
//!
//! Body and status are trivial to hold — [`request`] has exactly one success
//! path and it does not branch on whether the account was found. **Timing is
//! the half that is easy to lose**, and this module loses it in one specific
//! way if the mail is sent inline: an SMTP handshake to a real relay is tens to
//! hundreds of milliseconds, and the unknown-address path skips it entirely.
//! That difference is readable with a stopwatch and it is a complete account
//! oracle.
//!
//! So **delivery happens off the request path**. The handler mints, stores, and
//! answers; a spawned task talks to the relay. Two things fall out of that
//! besides the timing envelope: the response is not held open by a slow relay,
//! and a relay that is down cannot be used to stall request-handling tasks. The
//! cost is stated: a send failure reaches the log, not the caller, which is
//! correct here — the caller must not learn whether an address was deliverable
//! either.
//!
//! # The token never reaches a log, and never reaches the table
//!
//! `credential::mint` returns the presented value once. It goes into the email
//! body and nowhere else: [`casual_task_infra::Message`] keeps the body out of
//! `Debug`, the row stores a selector and a **salted hash** of the verifier, and
//! no `tracing` call in this module takes the token. `docs/40`'s token-hash
//! gate — "a database dump contains no usable credential" — holds for reset
//! links because of the second of those, and an integration test reads the
//! stored columns back to prove it.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_identity::{credential, password};
use casual_task_infra::{Mailer, Message};
use casual_task_persistence::identity;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::{ApiError, codes};
use crate::server::AppState;

/// The path the emailed link points at, under `TF_PUBLIC_URL`.
///
/// A constant rather than a format string at the call site: the webapp route
/// and the link in the email are the same fact, and two copies of it produce a
/// link that 404s for every user at once.
pub const RESET_PATH: &str = "/reset-password";

/// The subject line. ASCII and constant — `casual-task-infra` refuses anything
/// else, because encoding it safely needs an RFC 2047 encoder that is not in
/// the dependency set.
pub const RESET_SUBJECT: &str = "Reset your TaskForge password";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestBody {
    pub email: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmBody {
    pub token: String,
    pub password: String,
}

/// The one response [`request`] can produce.
///
/// A struct with one constant field rather than a `StatusCode` alone, so that
/// the shape is a *type* and a future edit that wanted to say "we sent it" for
/// a real account would have to change this declaration, in view of the module
/// docs, instead of adding a branch.
#[derive(Debug, Serialize)]
pub struct Accepted {
    pub message: &'static str,
}

impl Accepted {
    /// Deliberately says nothing about whether the address exists.
    const TEXT: &'static str = "If that address has an account, a reset link is on its way.";
}

/// `POST /api/v1/auth/password-reset` — ask for a link.
///
/// Always 202 with the same body. See the module docs.
///
/// # Errors
///
/// [`ApiError`] only when the database is unreachable — which is not an
/// enumeration signal, because it does not depend on the address.
pub async fn request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RequestBody>,
) -> Result<Response, ApiError> {
    let request_id = crate::server::RequestId::of_parts(&headers);
    let accepted = (
        StatusCode::ACCEPTED,
        Json(Accepted {
            message: Accepted::TEXT,
        }),
    )
        .into_response();

    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(&request_id, 5))?;

    // Recorded for an unknown address too. docs/40 §What is audited: a burst of
    // these is the signal of an attack, and an attacker guessing addresses
    // produces exactly the rows with no user_id.
    let ip = client_ip(&headers);
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok());

    let found = identity::credential_for_email(&mut conn, &body.email)
        .await
        .map_err(|error| {
            tracing::error!(%error, "credential lookup failed");
            ApiError::internal(&request_id)
        })?;

    if let Err(error) = identity::record_auth_event(
        &mut conn,
        found.as_ref().map(|c| c.user_id),
        Some(&body.email),
        "password.reset.requested",
        ip.as_deref(),
        agent,
    )
    .await
    {
        tracing::error!(%error, "the authentication trail was not written");
    }

    let Some(credential) = found else {
        // No account. The same response, and no further work — which is safe
        // only because the work that was skipped is off the request path for
        // the other branch too. See the module docs.
        return Ok(accepted);
    };

    // Any earlier link this person holds stops working now. Asking twice must
    // not leave two live tokens in one inbox.
    if let Err(error) = identity::invalidate_reset_tokens(&mut conn, credential.user_id).await {
        tracing::error!(%error, "superseding earlier reset tokens failed");
        return Err(ApiError::internal(&request_id));
    }

    let minted = credential::mint().map_err(|error| {
        tracing::error!(%error, "the randomness source failed");
        ApiError::internal(&request_id)
    })?;
    let (selector, _) = credential::split(&minted.presented).map_err(|_| {
        // Unreachable: the value was just minted in the shape `split` parses.
        ApiError::internal(&request_id)
    })?;

    identity::create_reset_token(
        &mut conn,
        credential.user_id,
        selector,
        &minted.verifier_hash,
        OffsetDateTime::now_utc() + identity::RESET_LIFETIME,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "storing the reset token failed");
        ApiError::internal(&request_id)
    })?;
    drop(conn);

    let message = Message::new(
        body.email,
        RESET_SUBJECT,
        reset_body(&state.public_url, &minted.presented),
    );
    deliver(state.mailer.clone(), message);

    Ok(accepted)
}

/// Hand the message to the relay, off the request path.
///
/// Split into its own function so the reason survives a refactor: see the
/// module docs on the timing envelope. The result is logged and dropped — the
/// caller must not learn whether an address was deliverable.
fn deliver(mailer: std::sync::Arc<dyn Mailer>, message: Message) {
    tokio::spawn(async move {
        if let Err(error) = mailer.send(&message).await {
            // `message` is safe to log: its Debug redacts the body, which is
            // the half that carries the token.
            tracing::error!(%error, ?message, "a password-reset email was not delivered");
        }
    });
}

/// The email body. `docs/29` §Email content, and what it must **not** contain.
///
/// A link and instructions. No password — not the old one, which is not
/// recoverable, and not a new one, which would put a working credential in an
/// inbox — and no task content, because a reset email is the one message this
/// system sends to an address before anyone has proved they control it.
#[must_use]
pub fn reset_body(public_url: &str, token: &str) -> String {
    format!(
        "Someone asked to reset the TaskForge password for this address.\n\
         \n\
         Open this link to choose a new one:\n\
         {}{RESET_PATH}?token={token}\n\
         \n\
         The link works once and expires in one hour.\n\
         \n\
         If you did not ask for this, nothing has changed and you can ignore\n\
         this message.\n",
        public_url.trim_end_matches('/')
    )
}

/// `POST /api/v1/auth/password-reset/confirm` — spend the link.
///
/// # Errors
///
/// 400 when the new password is below the twelve-character minimum, 401 when
/// the token is unknown, expired, or already spent, 503 when the database is
/// unreachable.
pub async fn confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ConfirmBody>,
) -> Result<Response, ApiError> {
    let request_id = crate::server::RequestId::of_parts(&headers);

    // Parsed before anything else: a malformed token must not reach a query as
    // a parameter, and it fails with the same 401 a wrong one does.
    let Ok((selector, verifier)) = credential::split(body.token.trim()) else {
        return Ok(ApiError::unauthenticated(&request_id).into_response());
    };

    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(&request_id, 5))?;
    let token = identity::live_reset_token(&mut conn, selector)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reset token lookup failed");
            ApiError::internal(&request_id)
        })?;
    // RELEASED before the Argon2 hash below. The hash is ~100 ms of pure CPU;
    // holding a pooled connection across it pins one of a bounded set (D-039)
    // for the whole time — the same reason login drops its connection.
    drop(conn);

    let Some(token) = token.filter(|t| credential::verify(verifier, &t.verifier_hash)) else {
        // Unknown selector, expired, already spent, or a forged verifier
        // against a real selector: one refusal, so none of them is
        // distinguishable from the others.
        return Ok(ApiError::unauthenticated(&request_id).into_response());
    };

    // `hash_chosen_async`, never `hash_generated`: this is a password a human
    // typed, so the twelve-character minimum in `docs/40` applies, and it is
    // enforced by the only function that can hash a chosen password rather than
    // by a check this endpoint could forget.
    let hash = match password::hash_chosen_async(&body.password).await {
        Ok(hash) => hash,
        Err(password::PasswordError::TooShort { minimum }) => {
            // The token is deliberately NOT spent. A person who typed a short
            // password should try again with the same link, not go back to
            // their inbox for a new one.
            return Ok(ApiError::new(
                StatusCode::BAD_REQUEST,
                codes::OUT_OF_RANGE,
                format!("A password must be at least {minimum} characters"),
                &request_id,
            )
            .with_details(serde_json::json!({ "field": "password", "min_length": minimum }))
            .into_response());
        }
        Err(error) => {
            tracing::error!(%error, "hashing the new password failed");
            return Err(ApiError::internal(&request_id));
        }
    };

    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(&request_id, 5))?;

    // Burn first. `consume_reset_token` updates only a row that is still
    // unused, so of two concurrent confirmations exactly one proceeds — and the
    // loser changes nothing, rather than both setting a password and the second
    // silently winning.
    let spent = identity::consume_reset_token(&mut conn, token.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "spending the reset token failed");
            ApiError::internal(&request_id)
        })?;
    if !spent {
        return Ok(ApiError::unauthenticated(&request_id).into_response());
    }

    identity::set_password(&mut conn, token.user_id, &hash)
        .await
        .map_err(|error| {
            tracing::error!(%error, "setting the new password failed");
            ApiError::internal(&request_id)
        })?;

    // `docs/40` §Local authentication: a password change invalidates sessions.
    // `set_password` moves `changed_at`, which `live_session` already checks —
    // this is the explicit half, so the sessions are *revoked* rather than
    // merely refused, and a user reading their session list sees them gone.
    let revoked = identity::revoke_all_sessions(&mut conn, token.user_id, None)
        .await
        .map_err(|error| {
            tracing::error!(%error, "revoking sessions after a password change failed");
            ApiError::internal(&request_id)
        })?;

    if let Err(error) = identity::record_auth_event(
        &mut conn,
        Some(token.user_id),
        None,
        "password.changed",
        client_ip(&headers).as_deref(),
        headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok()),
    )
    .await
    {
        tracing::error!(%error, "the authentication trail was not written");
    }

    tracing::info!(
        user_id = %token.user_id,
        revoked_sessions = revoked,
        "a password was reset"
    );

    // 204, with no session. Resetting a password does not sign anyone in: the
    // person who reset it may not be the person holding the browser, and a
    // reset that hands out a session turns a compromised mailbox into an
    // immediate takeover with one fewer step.
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The client IP from `X-Forwarded-For`, or `None`.
///
/// The same narrow reading as the one in `crate::auth`: a hint for the audit
/// trail, never an identity, and parsed rather than passed through — the raw
/// header is a proxy *chain*, which is not an `inet` and used to fail the
/// insert.
fn client_ip(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("x-forwarded-for")?.to_str().ok()?;
    raw.split(',')
        .next()?
        .trim()
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| ip.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_email_carries_the_link_and_nothing_else_sensitive() {
        // docs/29 and the C-001 brief: no password, no task content. The
        // positive half matters as much — a body without the token is an email
        // that wastes everyone's time.
        let body = reset_body("https://tasks.example.com", "abc.def");
        assert!(body.contains("https://tasks.example.com/reset-password?token=abc.def"));
        assert!(
            body.contains("once"),
            "the single-use property is not stated"
        );
        assert!(body.contains("one hour"), "the expiry is not stated");
        assert!(!body.to_lowercase().contains("your password is"));
    }

    #[test]
    fn a_trailing_slash_on_the_public_url_does_not_double() {
        // `https://x.com//reset-password` is a different path to most routers,
        // and TF_PUBLIC_URL is written by hand in an env file.
        let body = reset_body("https://tasks.example.com/", "abc.def");
        assert!(
            body.contains("https://tasks.example.com/reset-password?"),
            "{body}"
        );
        assert!(!body.contains("com//"), "{body}");
    }

    #[test]
    fn the_subject_is_something_casual_task_infra_will_accept() {
        // It refuses a non-ASCII or multi-line subject rather than mangling it,
        // so a subject that fails is an email nobody receives.
        assert!(RESET_SUBJECT.is_ascii());
        assert!(!RESET_SUBJECT.chars().any(|c| c.is_ascii_control()));
    }

    #[test]
    fn the_acceptance_message_reveals_nothing() {
        // The body is byte-identical for a real and an imaginary address, and
        // this is the string that has to stay conditional-free to keep it so.
        assert!(Accepted::TEXT.starts_with("If that address"));
    }
}
