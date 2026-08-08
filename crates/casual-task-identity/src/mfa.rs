//! TOTP (RFC 6238) and recovery codes.
//!
//! `docs/40` §MFA. Two things here are security decisions rather than
//! implementation detail, and both are the kind that look like nothing:
//!
//! 1. **A used code cannot be replayed.** RFC 6238 §5.2 requires it: a code is
//!    valid for a whole time step, so an attacker who observes one — over the
//!    user's shoulder, in a phishing proxy — can use it themselves within the
//!    same window. The check is not in this module because it needs storage;
//!    [`Totp::verify`] returns *which* step matched precisely so the caller can
//!    refuse a step it has already accepted, and its documentation says so.
//!
//! 2. **The skew window is ±1 step, not more.** Every extra step of tolerance
//!    multiplies the number of simultaneously valid codes. One step either side
//!    covers ordinary clock drift; three would cover a badly set clock and
//!    quadruple the guessing surface.

use hmac::{Hmac, Mac};
use rand::TryRngCore;
use sha1::Sha1;
use subtle::ConstantTimeEq;
use time::OffsetDateTime;

use crate::password;

/// Seconds per code. Thirty is the RFC default and what every authenticator
/// app assumes; it is not configurable because a server and a phone that
/// disagree produce codes that never match and an error message that says
/// "invalid code".
pub const STEP_SECONDS: i64 = 30;

/// Digits in a code.
pub const DIGITS: u32 = 6;

/// Time steps of tolerance either side of now.
///
/// One. See the module docs: each extra step multiplies the number of valid
/// codes at any instant.
pub const SKEW_STEPS: i64 = 1;

/// Recovery codes issued at enrolment.
///
/// Ten is enough that losing a phone is not an incident and few enough that the
/// list stays something a person will actually store somewhere safe.
pub const RECOVERY_CODE_COUNT: usize = 10;

/// A TOTP factor: the shared secret, and nothing else.
#[derive(Clone)]
pub struct Totp {
    secret: Vec<u8>,
}

// Hand-written so a secret cannot reach a log through `{:?}` on a struct that
// happens to contain one. `docs/46` has a Redacted<T> for values that are
// customer content; this is the same idea for the one plaintext secret in the
// schema.
impl std::fmt::Debug for Totp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Totp(<secret>)")
    }
}

impl Totp {
    /// Generate a new factor.
    ///
    /// # Errors
    ///
    /// If the operating system's randomness source fails.
    pub fn generate() -> Result<Self, rand::rand_core::OsError> {
        // 20 bytes: the RFC 4226 recommendation and the length every
        // authenticator app handles without complaint.
        let mut secret = vec![0u8; 20];
        rand::rngs::OsRng.try_fill_bytes(&mut secret)?;
        Ok(Self { secret })
    }

    /// Reconstruct from the stored base32 secret.
    ///
    /// # Errors
    ///
    /// [`MalformedSecret`] if it is not valid base32.
    pub fn from_base32(encoded: &str) -> Result<Self, MalformedSecret> {
        base32::decode(base32::Alphabet::Rfc4648 { padding: false }, encoded)
            .map(|secret| Self { secret })
            .ok_or(MalformedSecret)
    }

    /// The secret as stored and as shown to an authenticator app.
    #[must_use]
    pub fn to_base32(&self) -> String {
        base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &self.secret)
    }

    /// Verify `code`, returning the time step it matched.
    ///
    /// **The caller must reject a step it has already accepted.** A code is
    /// valid for a whole 30-second window, so without that check an observed
    /// code can be replayed within the window by whoever saw it (RFC 6238
    /// §5.2). Returning the step rather than a bool is what makes the check
    /// possible; a `bool` here would quietly make replay protection impossible
    /// to add later without changing every caller.
    #[must_use]
    pub fn verify(&self, code: &str, now: OffsetDateTime) -> Option<i64> {
        let current = now.unix_timestamp() / STEP_SECONDS;
        (current - SKEW_STEPS..=current + SKEW_STEPS).find(|step| {
            // Constant-time: a code is only six digits, and a comparison that
            // returns early on the first wrong digit narrows the search space
            // by a factor of ten per digit.
            self.code_at(*step).as_bytes().ct_eq(code.as_bytes()).into()
        })
    }

    /// The code for a given time step. RFC 4226 dynamic truncation.
    fn code_at(&self, step: i64) -> String {
        let mut mac =
            Hmac::<Sha1>::new_from_slice(&self.secret).expect("HMAC accepts any key size");
        mac.update(&step.to_be_bytes());
        let digest = mac.finalize().into_bytes();

        let offset = (digest[digest.len() - 1] & 0x0f) as usize;
        let binary = u32::from_be_bytes([
            digest[offset] & 0x7f,
            digest[offset + 1],
            digest[offset + 2],
            digest[offset + 3],
        ]);
        format!(
            "{:0width$}",
            binary % 10_u32.pow(DIGITS),
            width = DIGITS as usize
        )
    }
}

/// The stored secret is not valid base32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the stored MFA secret is malformed")]
pub struct MalformedSecret;

/// One recovery code: what to show once, and what to store.
#[derive(Debug, Clone)]
pub struct RecoveryCode {
    /// Shown once, at enrolment. Never recoverable afterwards.
    pub presented: String,
    /// Stored. Hashed with Argon2id, like a password.
    pub hash: String,
}

/// Issue a fresh set of recovery codes.
///
/// Hashed with **Argon2id**, not SHA-256 — unlike the 192-bit verifiers in
/// [`credential`](crate::credential). A recovery code is short enough for a
/// human to copy off a screen and type back in, which puts it in the same
/// low-entropy category as a password, and a dump of these is a dump of MFA
/// bypasses.
///
/// # Errors
///
/// If randomness or hashing fails.
pub fn issue_recovery_codes() -> Result<Vec<RecoveryCode>, RecoveryError> {
    // Crockford-ish: no I, L, O, U — the characters people mistype when reading
    // a code off a screen, which is the only way a recovery code is ever used.
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTVWXYZ23456789";
    let mut codes = Vec::with_capacity(RECOVERY_CODE_COUNT);

    for _ in 0..RECOVERY_CODE_COUNT {
        let mut raw = [0u8; 10];
        rand::rngs::OsRng
            .try_fill_bytes(&mut raw)
            .map_err(|_| RecoveryError::Entropy)?;
        let presented: String = raw
            .iter()
            .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
            .collect();
        let hash = password::hash_generated(&presented).map_err(|_| RecoveryError::Hashing)?;
        codes.push(RecoveryCode { presented, hash });
    }
    Ok(codes)
}

/// Issuing recovery codes failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryError {
    #[error("the randomness source failed")]
    Entropy,
    #[error("hashing failed")]
    Hashing,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(unix: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(unix).expect("valid")
    }

    #[test]
    fn a_generated_factor_verifies_its_own_current_code() {
        let totp = Totp::generate().expect("entropy");
        let now = at(1_760_000_000);
        let code = totp.code_at(now.unix_timestamp() / STEP_SECONDS);
        assert!(totp.verify(&code, now).is_some());
    }

    #[test]
    fn a_secret_survives_the_base32_round_trip() {
        // The stored column is base32; a factor that could not be reconstructed
        // from it would fail every verification after a restart.
        let totp = Totp::generate().expect("entropy");
        let restored = Totp::from_base32(&totp.to_base32()).expect("valid base32");
        let now = at(1_760_000_000);
        let code = totp.code_at(now.unix_timestamp() / STEP_SECONDS);
        assert!(restored.verify(&code, now).is_some());
    }

    #[test]
    fn one_step_of_drift_is_tolerated_and_two_is_not() {
        let totp = Totp::generate().expect("entropy");
        let now = at(1_760_000_000);
        let step = now.unix_timestamp() / STEP_SECONDS;

        assert!(
            totp.verify(&totp.code_at(step - 1), now).is_some(),
            "a phone one step slow was refused"
        );
        assert!(
            totp.verify(&totp.code_at(step + 1), now).is_some(),
            "a phone one step fast was refused"
        );
        // Each extra tolerated step multiplies the number of simultaneously
        // valid codes, so the boundary is a security property, not a default.
        assert!(totp.verify(&totp.code_at(step - 2), now).is_none());
        assert!(totp.verify(&totp.code_at(step + 2), now).is_none());
    }

    #[test]
    fn verify_returns_the_step_so_replay_can_be_refused() {
        // RFC 6238 §5.2. This module cannot enforce it — that needs storage —
        // so the API makes it possible instead of pretending it is handled.
        let totp = Totp::generate().expect("entropy");
        let now = at(1_760_000_000);
        let step = now.unix_timestamp() / STEP_SECONDS;
        assert_eq!(totp.verify(&totp.code_at(step), now), Some(step));
        assert_eq!(totp.verify(&totp.code_at(step - 1), now), Some(step - 1));
    }

    #[test]
    fn a_wrong_code_is_refused() {
        let totp = Totp::generate().expect("entropy");
        let now = at(1_760_000_000);
        assert!(totp.verify("000000", now).is_none() || totp.verify("111111", now).is_none());
        assert!(totp.verify("", now).is_none());
        assert!(totp.verify("not a code", now).is_none());
    }

    #[test]
    fn codes_are_six_digits_including_leading_zeros() {
        // A code formatted without zero padding is five characters some of the
        // time, and every authenticator app shows six. The mismatch would look
        // like an intermittently broken factor.
        let totp = Totp::generate().expect("entropy");
        for step in 0..500 {
            let code = totp.code_at(step);
            assert_eq!(code.len(), DIGITS as usize, "step {step} produced {code}");
            assert!(code.bytes().all(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn the_secret_does_not_appear_in_debug_output() {
        let totp = Totp::generate().expect("entropy");
        let rendered = format!("{totp:?}");
        assert_eq!(rendered, "Totp(<secret>)");
        assert!(!rendered.contains(&totp.to_base32()));
    }

    #[test]
    fn recovery_codes_are_distinct_and_stored_hashed() {
        let codes = issue_recovery_codes().expect("issued");
        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);

        let mut seen = std::collections::HashSet::new();
        for code in &codes {
            assert!(seen.insert(code.presented.clone()), "a code repeated");
            assert!(
                !code.hash.contains(&code.presented),
                "the code is recoverable from its stored hash"
            );
            assert!(password::verify(&code.presented, &code.hash).expect("parses"));
        }
    }

    #[test]
    fn recovery_codes_avoid_characters_people_mistype() {
        // I/L/O/U are excluded because the only way a recovery code is ever
        // used is by being read off a screen and typed back in.
        let codes = issue_recovery_codes().expect("issued");
        for code in &codes {
            for banned in ['I', 'L', 'O', 'U'] {
                assert!(
                    !code.presented.contains(banned),
                    "{} contains {banned}",
                    code.presented
                );
            }
        }
    }

    #[test]
    fn a_malformed_stored_secret_is_reported_not_panicked() {
        assert_eq!(
            Totp::from_base32("!!!not base32!!!").err(),
            Some(MalformedSecret)
        );
    }
}
