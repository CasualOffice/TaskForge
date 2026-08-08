//! Double-submit CSRF tokens, bound to the session (`docs/40`, ADR-032).
//!
//! # What `TF_SECRET_KEY` is for
//!
//! ADR-032: "`TF_SECRET_KEY` is not a cookie signature." The session cookie
//! stays opaque and unsigned — a signature over a random value proves nothing
//! the value does not already prove. The key is used **here**, and only here.
//!
//! # Why the token is bound to the session
//!
//! A plain double-submit cookie compares two values a client sent. Any attacker
//! who can set a cookie on the victim's domain — a subdomain takeover, an XSS on
//! a sibling host, a network position on plain HTTP — can set *both* halves and
//! the check passes.
//!
//! Binding the token to the session with a keyed MAC removes that: forging one
//! requires the session selector **and** the server key. The token is
//! `HMAC-SHA256(key, selector)`, so it needs no storage and no expiry of its
//! own — it dies exactly when the session does, which is the correct lifetime
//! and one fewer thing to sweep.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// The header a client returns the token in.
pub const CSRF_HEADER: &str = "x-csrf-token";

/// The cookie the token is also delivered in, for the double submit.
///
/// **Not** `HttpOnly`: the client has to read it to echo it back, which is the
/// entire mechanism. That is safe precisely because the token is useless
/// without the session cookie beside it, and the session cookie *is*
/// `HttpOnly`.
pub const CSRF_COOKIE: &str = "tf_csrf";

/// Derive the CSRF token for a session.
///
/// Deterministic, so it never needs storing and cannot drift from the session
/// it belongs to.
#[must_use]
pub fn token_for(secret_key: &str, session_selector: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret_key.as_bytes()).expect("HMAC accepts any key size");
    mac.update(session_selector.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .fold(String::new(), |mut out, byte| {
            use std::fmt::Write;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Whether `presented` is the token for this session.
///
/// Constant-time: a comparison that returns on the first differing byte lets an
/// attacker recover the token one byte at a time.
#[must_use]
pub fn verify(secret_key: &str, session_selector: &str, presented: &str) -> bool {
    let expected = token_for(secret_key, session_selector);
    expected.as_bytes().ct_eq(presented.as_bytes()).into()
}

/// Whether a method changes state and therefore needs a token.
///
/// `docs/05`: "every unsafe method without a valid token is rejected". The
/// **safe** list is spelled out rather than the unsafe one, so a method nobody
/// anticipated is treated as unsafe by default.
#[must_use]
pub fn requires_token(method: &axum::http::Method) -> bool {
    !matches!(
        *method,
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789012345678901234567890123456789";

    #[test]
    fn a_token_verifies_for_its_own_session() {
        let token = token_for(KEY, "selector-a");
        assert!(verify(KEY, "selector-a", &token));
    }

    #[test]
    fn a_token_from_another_session_is_refused() {
        // The property a plain double-submit cookie does not have. An attacker
        // who can set cookies on the victim's domain still cannot produce a
        // token for the victim's session.
        let token = token_for(KEY, "attacker-session");
        assert!(!verify(KEY, "victim-session", &token));
    }

    #[test]
    fn a_token_from_another_deployment_is_refused() {
        let token = token_for("a-different-server-key-aaaaaaaaaaaaaaaa", "selector-a");
        assert!(!verify(KEY, "selector-a", &token));
    }

    #[test]
    fn the_token_does_not_contain_the_selector_or_the_key() {
        // It travels in a cookie the client can read. It must not carry either
        // secret it was derived from.
        let token = token_for(KEY, "selector-a");
        assert!(!token.contains("selector-a"));
        assert!(!token.contains(KEY));
    }

    #[test]
    fn an_empty_token_is_refused() {
        // The shape a missing header arrives in.
        assert!(!verify(KEY, "selector-a", ""));
    }

    #[test]
    fn every_state_changing_method_requires_a_token() {
        use axum::http::Method;
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(requires_token(&method), "{method} did not require a token");
        }
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(!requires_token(&method), "{method} required a token");
        }
    }

    #[test]
    fn an_unanticipated_method_is_treated_as_unsafe() {
        // The safe list is spelled out, not the unsafe one. A verb nobody
        // thought about must default to needing a token, not to skipping it.
        let odd = axum::http::Method::from_bytes(b"PURGE").expect("valid");
        assert!(requires_token(&odd));
    }
}
