//! The request and response shapes for MFA (`docs/05`).
//!
//! Separated from the handlers because the API contract and the behaviour
//! change for different reasons — a field rename is a client-visible break, and
//! a change to how a code is verified is not.
//!
//! # The one type that carries a secret
//!
//! [`EnrolmentStarted`] is the only response in this crate that returns
//! material an attacker could use directly. It is returned **once**, to an
//! already-authenticated caller, over a channel `docs/07` requires to be TLS
//! 1.3 — and it derives `Serialize` but deliberately **not** `Debug`, so it
//! cannot reach a log through `{:?}` on a struct that happens to hold one.

use serde::{Deserialize, Serialize};

/// `POST /api/v1/auth/mfa/enrolment` — what an authenticator app needs.
///
/// **`Debug` is hand-written and redacts everything.** Deriving it would put
/// the shared secret one `{:?}` away from a log line, and `tracing` reaches for
/// `Debug` by default on any field that is not explicitly formatted. Omitting
/// it entirely was the other option and is worse: the workspace lints require
/// one, and a type with no `Debug` invites the next person to add the derive.
#[derive(Serialize)]
pub struct EnrolmentStarted {
    /// Base32, for a user typing it in by hand.
    pub secret: String,
    /// `otpauth://` URI, for a QR code.
    pub provisioning_uri: String,
    /// Seconds per code and digits per code, so a client rendering its own
    /// countdown does not hardcode what the server decided.
    pub period_seconds: i64,
    pub digits: u32,
}

impl std::fmt::Debug for EnrolmentStarted {
    /// Everything a caller could authenticate with is redacted; the two
    /// parameters are printed because they are constants a reader may want.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnrolmentStarted")
            .field("secret", &"<redacted>")
            .field("provisioning_uri", &"<redacted>")
            .field("period_seconds", &self.period_seconds)
            .field("digits", &self.digits)
            .finish()
    }
}

/// `POST /api/v1/auth/mfa/enrolment/confirm`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmEnrolment {
    pub code: String,
}

/// What confirmation returns: the recovery codes, shown once.
///
/// `Debug` redacts, for the same reason as [`EnrolmentStarted`] — these are MFA
/// bypasses in plaintext, and this is the only moment they exist outside a
/// hash.
#[derive(Serialize)]
pub struct RecoveryCodesIssued {
    /// `docs/40`: "10 single-use recovery codes shown once".
    pub recovery_codes: Vec<String>,
}

impl std::fmt::Debug for RecoveryCodesIssued {
    /// The count, never the codes. "Ten were issued" is the operationally
    /// useful fact; the codes themselves are the thing a log must never hold.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryCodesIssued")
            .field(
                "recovery_codes",
                &format_args!("<{} redacted>", self.recovery_codes.len()),
            )
            .finish()
    }
}

/// `POST /api/v1/auth/mfa/step-up` — prove the factor for this session.
///
/// One of `code` or `recovery_code`, never both. Two optional fields rather
/// than an untagged enum so that a client sending neither gets a field-level
/// `400` naming what was missing instead of a deserialization error naming
/// nothing.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepUp {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub recovery_code: Option<String>,
}

/// `GET /api/v1/auth/mfa` — what the account currently has.
#[derive(Debug, Serialize)]
pub struct MfaStatus {
    /// Whether a **confirmed** factor exists. An enrolment left half-finished
    /// reports `false`, because that is what it means for every decision that
    /// consults it.
    pub enrolled: bool,
    /// Whether enrolment has begun and not been confirmed, so a client can
    /// offer to resume rather than starting over silently.
    pub pending: bool,
    /// How many recovery codes are left, so "you have two left" can prompt a
    /// re-issue before the answer is zero.
    pub recovery_codes_remaining: i64,
    /// Whether **this session** has satisfied MFA. Per session, not per user:
    /// `docs/40` puts the assertion on the session.
    pub session_satisfied: bool,
}

/// `PUT /api/v1/workspaces/{workspace_id}/mfa-requirement`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetRequirement {
    pub required: bool,
}

/// The `otpauth://` URI an authenticator app scans.
///
/// The issuer and the account label are both percent-encoded. Without it an
/// address containing `&` or `#` — legal in an email local part — silently
/// truncates the URI at the query separator, and the resulting factor produces
/// codes that never match.
#[must_use]
pub fn provisioning_uri(
    issuer: &str,
    account: &str,
    secret: &str,
    digits: u32,
    period: i64,
) -> String {
    let issuer_enc = percent_encode(issuer);
    let account_enc = percent_encode(account);
    format!(
        "otpauth://totp/{issuer_enc}:{account_enc}?secret={secret}&issuer={issuer_enc}\
         &algorithm=SHA1&digits={digits}&period={period}"
    )
}

/// Percent-encode everything outside the unreserved set (RFC 3986 §2.3).
///
/// Deliberately conservative: encoding a character that did not need it is
/// harmless, and failing to encode one that did breaks the URI.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provisioning_uri_carries_what_an_app_needs() {
        let uri = provisioning_uri("TaskForge", "ada@example.com", "JBSWY3DPEHPK3PXP", 6, 30);
        assert!(uri.starts_with("otpauth://totp/TaskForge:"));
        assert!(uri.contains("secret=JBSWY3DPEHPK3PXP"));
        assert!(uri.contains("digits=6"));
        assert!(uri.contains("period=30"));
        assert!(uri.contains("issuer=TaskForge"));
    }

    #[test]
    fn an_address_that_would_truncate_the_uri_is_encoded() {
        // `&` and `#` are legal in an email local part. Unencoded, the first
        // ends the label early and the second starts a fragment — the app
        // enrols a factor whose codes never match, and the failure looks like
        // a broken authenticator.
        let uri = provisioning_uri("Task&Forge", "a&b#c@example.com", "SECRET", 6, 30);
        assert!(!uri.contains("a&b#c"), "{uri}");
        assert!(uri.contains("a%26b%23c%40example.com"), "{uri}");
        assert!(uri.contains("Task%26Forge"), "{uri}");
        // The separators the format string itself introduces must survive.
        assert_eq!(uri.matches("&issuer=").count(), 1, "{uri}");
    }

    #[test]
    fn a_space_in_the_issuer_does_not_break_the_uri() {
        let uri = provisioning_uri("Acme Tasks", "ada@example.com", "S", 6, 30);
        assert!(!uri.contains("Acme Tasks"), "{uri}");
        assert!(uri.contains("Acme%20Tasks"), "{uri}");
    }
}
