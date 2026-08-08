//! Login, logout, and reading the current session (`docs/40`).
//!
//! # The login endpoint is the one most often shipped broken
//!
//! `docs/40`: "Login responses are constant-shape and constant-ish time whether
//! or not the account exists. Account enumeration through the login endpoint is
//! the most commonly shipped auth bug."
//!
//! Two things make that true here rather than aspirational:
//!
//! 1. **One failure response.** Unknown account, wrong password, locked account
//!    and tombstoned account all return the identical body and status. There is
//!    no branch that returns something else, because there is no other failure
//!    value to return — [`LoginOutcome`] has one failure variant, so a future
//!    edit cannot add a distinguishable case without changing the type.
//!
//! 2. **The work is always done.** An unknown address still performs an Argon2
//!    verification against a fixed dummy hash. Skipping it returns in
//!    microseconds where a real account takes ~100 ms at the parameters
//!    `docs/40` sets — a timing oracle wide enough to read with a stopwatch.

use std::sync::LazyLock;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_identity::{credential, password};
use casual_task_persistence::identity;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::csrf;
use crate::error::ApiError;
use crate::server::AppState;

/// The session cookie (`docs/40` §Browser sessions).
pub const SESSION_COOKIE: &str = "tf_session";

/// How long a session lives without being refreshed.
pub const SESSION_TTL: Duration = Duration::days(14);

/// A hash to verify against when the account does not exist.
///
/// Computed once, lazily, from a value no one can log in with. Its only purpose
/// is to make the failing path cost the same as the succeeding one — see the
/// module docs. Computing it per request would double the cost of every failed
/// login and make the endpoint a cheap denial-of-service amplifier.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    password::hash_generated("this password authenticates nobody").unwrap_or_else(|_| {
        "$argon2id$v=19$m=65536,t=3,p=4$AAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAA".into()
    })
});

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// Echoed so a client can put it in the header without reading the cookie
    /// — useful for non-browser clients, and harmless for browsers, which have
    /// the cookie anyway.
    pub csrf_token: String,
}

/// What a login attempt produced.
///
/// **One failure variant, deliberately.** See the module docs: a type with
/// `NoSuchAccount` and `WrongPassword` beside each other is an enumeration
/// oracle waiting for someone to map them onto different responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    Authenticated { user_id: uuid::Uuid },
    Refused,
}

/// `POST /api/v1/auth/login`.
///
/// # Errors
///
/// Returns an [`ApiError`] on a database failure. An authentication failure is
/// **not** an error variant — it is a 401 with the same shape as every other
/// refusal.
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let request_id = crate::server::RequestId::of_parts(&headers);
    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(&request_id, 5))?;

    let found = identity::credential_for_email(&mut conn, &body.email)
        .await
        .map_err(|error| {
            tracing::error!(%error, "credential lookup failed");
            ApiError::internal(&request_id)
        })?;

    // The connection is RELEASED before hashing. Argon2id at 64 MB is ~100 ms
    // of pure CPU with no I/O; holding a pooled connection across it pins one
    // of a bounded set (D-039) for the whole time, so a burst of logins
    // exhausts the pool with work that needs no database at all.
    drop(conn);

    let outcome = authenticate(found.as_ref(), &body.password, OffsetDateTime::now_utc()).await;

    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(&request_id, 5))?;

    let outcome = record_outcome(
        &mut conn,
        found.as_ref(),
        outcome,
        OffsetDateTime::now_utc(),
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording a login attempt failed");
        ApiError::internal(&request_id)
    })?;

    let LoginOutcome::Authenticated { user_id } = outcome else {
        // The single refusal. docs/40: constant shape, whatever the reason.
        return Ok(ApiError::unauthenticated(&request_id).into_response());
    };

    let minted = credential::mint().map_err(|error| {
        tracing::error!(%error, "the randomness source failed");
        ApiError::internal(&request_id)
    })?;
    let (selector, _) = credential::split(&minted.presented).map_err(|_| {
        // Unreachable: the value was just minted in the shape `split` parses.
        ApiError::internal(&request_id)
    })?;

    identity::create_session(
        &mut conn,
        user_id,
        selector,
        &minted.verifier_hash,
        "password",
        OffsetDateTime::now_utc() + SESSION_TTL,
        client_ip(&headers).as_deref(),
        header_str(&headers, header::USER_AGENT.as_str()),
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "creating the session failed");
        ApiError::internal(&request_id)
    })?;

    let csrf_token = csrf::token_for(&state.secret_key, selector);
    let mut response = (
        StatusCode::OK,
        Json(LoginResponse {
            csrf_token: csrf_token.clone(),
        }),
    )
        .into_response();

    for cookie in [
        // HttpOnly: the session value must never be readable by script, which
        // is what makes an XSS bug fall short of account takeover.
        format!(
            "{SESSION_COOKIE}={}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
            minted.presented,
            SESSION_TTL.whole_seconds()
        ),
        // NOT HttpOnly: the client has to read this one to echo it back. Safe
        // because it is useless without the session cookie beside it.
        format!(
            "{}={csrf_token}; Secure; SameSite=Lax; Path=/; Max-Age={}",
            csrf::CSRF_COOKIE,
            SESSION_TTL.whole_seconds()
        ),
    ] {
        if let Ok(value) = cookie.parse() {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    Ok(response)
}

/// The decision, separated from HTTP so it can be tested for the properties
/// that matter: one refusal, and the same work on every path.
///
/// # Errors
///
/// Any database error while recording the attempt.
pub async fn authenticate(
    found: Option<&identity::Credential>,
    presented: &str,
    now: OffsetDateTime,
) -> LoginOutcome {
    let Some(credential) = found else {
        // No account. The hash still runs — see the module docs.
        let _ = password::verify_async(presented, &DUMMY_HASH).await;
        return LoginOutcome::Refused;
    };

    if credential.locked_until.is_some_and(|until| until > now) {
        // Backing off. The hash still runs, so a locked account is not
        // detectable by how fast it refuses.
        let _ = password::verify_async(presented, &DUMMY_HASH).await;
        return LoginOutcome::Refused;
    }

    if password::verify_async(presented, &credential.password_hash)
        .await
        .unwrap_or(false)
    {
        LoginOutcome::Authenticated {
            user_id: credential.user_id,
        }
    } else {
        LoginOutcome::Refused
    }
}

/// Persist what the attempt implies, in a second short transaction.
///
/// Split from [`authenticate`] so the decision needs no database connection and
/// the connection is not held across the hash.
///
/// # Errors
///
/// Any database error.
async fn record_outcome(
    conn: &mut sqlx::PgConnection,
    found: Option<&identity::Credential>,
    outcome: LoginOutcome,
    now: OffsetDateTime,
) -> Result<LoginOutcome, sqlx::Error> {
    let Some(credential) = found else {
        return Ok(outcome);
    };
    match outcome {
        LoginOutcome::Authenticated { .. } => {
            identity::clear_failures(conn, credential.user_id).await?;
        }
        LoginOutcome::Refused => {
            // Only when the account was not already backing off: counting
            // attempts made during a lock would let anyone hold a stranger's
            // account locked indefinitely.
            if credential.locked_until.is_none_or(|until| until <= now) {
                let attempts = u32::try_from(credential.failed_attempts).unwrap_or(u32::MAX);
                let locked_until = password::locked_until(attempts.saturating_add(1), now);
                identity::record_failure(conn, credential.user_id, locked_until).await?;
            }
        }
    }
    Ok(outcome)
}

/// `POST /api/v1/auth/logout`.
///
/// Revokes the session row, so revocation is immediate — `docs/40` rejects JWTs
/// for exactly this reason. Clearing the cookie alone would leave a credential
/// that still works if it was captured.
///
/// # Errors
///
/// [`ApiError`] on a database failure.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = crate::server::RequestId::of_parts(&headers);
    if let Some(selector) = session_selector(&headers) {
        let mut conn = state
            .pool
            .acquire()
            .await
            .map_err(|_| ApiError::unavailable(&request_id, 5))?;
        if let Some(session) =
            identity::live_session(&mut conn, &selector)
                .await
                .map_err(|error| {
                    tracing::error!(%error, "session lookup failed");
                    ApiError::internal(&request_id)
                })?
        {
            identity::revoke_session(&mut conn, session.id)
                .await
                .map_err(|error| {
                    tracing::error!(%error, "revoking the session failed");
                    ApiError::internal(&request_id)
                })?;
        }
    }

    // 204 whether or not there was a session. Logging out of nothing is not an
    // error, and reporting one would tell a caller whether a stolen cookie was
    // still live.
    let mut response = StatusCode::NO_CONTENT.into_response();
    for cookie in [
        format!("{SESSION_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"),
        format!(
            "{}=; Secure; SameSite=Lax; Path=/; Max-Age=0",
            csrf::CSRF_COOKIE
        ),
    ] {
        if let Ok(value) = cookie.parse() {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    Ok(response)
}

/// The session selector from the cookie, if there is one that parses.
#[must_use]
pub fn session_selector(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    let presented = cookies
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value)?;
    let (selector, _) = credential::split(presented).ok()?;
    Some(selector.to_owned())
}

fn header_str<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// The client IP from `X-Forwarded-For`, or `None`.
///
/// Two things this must not do, and previously did both:
///
/// - **Pass the raw header to an `inet` column.** A normal two-hop proxy chain
///   sends `X-Forwarded-For: 203.0.113.9, 198.51.100.4`; that string is not an
///   inet, so the insert failed and *login returned 500* for every client
///   behind two proxies. The header is attacker-controlled, so it was also a
///   trivial way to make anyone's login fail.
/// - **Trust it as an identity.** It is a hint for the audit trail only. It is
///   never used for authorisation, and the first hop is taken because that is
///   the convention, not because it is verified.
fn client_ip(headers: &HeaderMap) -> Option<String> {
    let raw = header_str(headers, "x-forwarded-for")?;
    let first = raw.split(',').next()?.trim();
    first
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| ip.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn the_outcome_type_has_exactly_one_failure() {
        // The enumeration guard, as a type. A variant added beside `Refused`
        // would be a distinguishable case, and this test is where that is
        // noticed — the doc comment alone would not survive a refactor.
        let refused = LoginOutcome::Refused;
        match refused {
            LoginOutcome::Authenticated { .. } | LoginOutcome::Refused => {}
        }
    }

    #[test]
    fn a_session_selector_is_read_from_the_cookie() {
        let minted = credential::mint().expect("entropy");
        let (selector, _) = credential::split(&minted.presented).expect("well formed");

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("other=1; {SESSION_COOKIE}={}", minted.presented))
                .expect("valid"),
        );
        assert_eq!(session_selector(&headers).as_deref(), Some(selector));
    }

    #[test]
    fn a_malformed_session_cookie_yields_nothing() {
        // It reaches a database query as a parameter otherwise.
        for value in ["tf_session=", "tf_session=nonsense", "unrelated=1", ""] {
            let mut headers = HeaderMap::new();
            headers.insert(header::COOKIE, HeaderValue::from_str(value).expect("valid"));
            assert_eq!(session_selector(&headers), None, "accepted {value:?}");
        }
    }

    #[test]
    fn the_session_ttl_is_finite_and_not_absurd() {
        // A session that never expires is a credential with no end. Two weeks
        // is the documented default in docs/40's cookie example.
        assert!(SESSION_TTL > Duration::days(1));
        assert!(SESSION_TTL <= Duration::days(30));
    }

    #[test]
    fn the_dummy_hash_is_a_real_argon2_hash_at_the_configured_cost() {
        // If it were not, the equalising verification would return early and
        // the timing oracle would be back.
        assert!(DUMMY_HASH.starts_with("$argon2id$"), "{}", *DUMMY_HASH);
        assert!(DUMMY_HASH.contains("m=65536"), "{}", *DUMMY_HASH);
    }
}
