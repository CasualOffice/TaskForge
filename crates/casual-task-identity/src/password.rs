//! Password hashing, and the lockout that stops it being a free oracle.
//!
//! Argon2id, and only here. A password is a **low-entropy secret chosen by a
//! human**: the search space is a dictionary, not 2^192, so the slow KDF is the
//! only thing standing between a database dump and an offline attack. Every
//! other credential in this crate is a random 192-bit value where a slow hash
//! buys nothing and costs latency on every request
//! ([`credential`](crate::credential) says so at the other end).
//!
//! # The lockout is a counter and a time, never a flag
//!
//! `docs/40` §Acceptance gates: "brute force triggers exponential backoff
//! **without locking a legitimate user out permanently**". A boolean `locked`
//! column is a denial of service anyone can trigger by typing a stranger's
//! email address wrongly enough times, and the victim cannot clear it without
//! support. Backoff that expires on its own has no such lever.

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use time::{Duration, OffsetDateTime};

/// Failures a caller must distinguish.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasswordError {
    /// The stored hash could not be parsed — a damaged row, not a wrong
    /// password. Kept separate so a migration bug cannot be silently reported
    /// to users as "wrong password" forever.
    #[error("the stored password hash is malformed")]
    MalformedHash,
    /// Hashing itself failed.
    #[error("hashing failed: {0}")]
    Hashing(String),
}

/// Hash a password for storage.
///
/// Returns a PHC string, so the parameters travel with the hash and raising the
/// cost later does not invalidate existing passwords.
///
/// # Errors
///
/// [`PasswordError::Hashing`] if the KDF fails.
pub fn hash(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| PasswordError::Hashing(e.to_string()))
}

/// Whether `password` matches `stored`.
///
/// # Errors
///
/// [`PasswordError::MalformedHash`] if `stored` is not a PHC string. A wrong
/// password is `Ok(false)`, not an error — the two are different questions and
/// only one of them is a bug.
pub fn verify(password: &str, stored: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(stored).map_err(|_| PasswordError::MalformedHash)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// The backoff ladder, indexed by consecutive failures.
///
/// Doubling from one second, capped at fifteen minutes. The cap matters: an
/// uncapped ladder reaches "locked until next Tuesday" after about twenty
/// attempts, which is the permanent lockout `docs/40` forbids wearing a
/// different name.
pub const LOCKOUT_LADDER: &[Duration] = &[
    Duration::seconds(0),
    Duration::seconds(0),
    Duration::seconds(0),
    Duration::seconds(1),
    Duration::seconds(5),
    Duration::seconds(30),
    Duration::minutes(2),
    Duration::minutes(15),
];

/// Failures allowed before any delay at all.
///
/// Three, because typos exist and the first three attempts of a real brute
/// force are indistinguishable from a person who changed their password last
/// month. The cost of the fourth attempt starting to hurt is one second.
pub const FREE_ATTEMPTS: usize = 3;

/// When the next attempt may be made, given consecutive failures.
///
/// `None` means "now". Returned rather than stored as a boolean so the caller
/// writes a timestamp that expires on its own.
#[must_use]
pub fn locked_until(failed_attempts: u32, now: OffsetDateTime) -> Option<OffsetDateTime> {
    let index = usize::try_from(failed_attempts).unwrap_or(usize::MAX);
    let delay = LOCKOUT_LADDER
        .get(index)
        .copied()
        .unwrap_or_else(|| *LOCKOUT_LADDER.last().expect("the ladder is not empty"));
    if delay.is_zero() {
        return None;
    }
    Some(now + delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_verifies_against_its_own_hash() {
        let stored = hash("correct horse battery staple").expect("hashes");
        assert!(verify("correct horse battery staple", &stored).expect("parses"));
        assert!(!verify("Correct horse battery staple", &stored).expect("parses"));
    }

    #[test]
    fn the_stored_hash_does_not_contain_the_password() {
        let stored = hash("hunter2").expect("hashes");
        assert!(!stored.contains("hunter2"));
    }

    #[test]
    fn the_same_password_hashes_differently_each_time() {
        // Per-hash salt. Without it, a dump shows which accounts share a
        // password — which is most of the value of a dump.
        let a = hash("same").expect("hashes");
        let b = hash("same").expect("hashes");
        assert_ne!(a, b);
        assert!(verify("same", &a).expect("parses"));
        assert!(verify("same", &b).expect("parses"));
    }

    #[test]
    fn the_hash_carries_its_parameters() {
        // A PHC string, so raising the cost later does not invalidate every
        // existing password.
        let stored = hash("x").expect("hashes");
        assert!(stored.starts_with("$argon2id$"), "{stored}");
        assert!(stored.contains("m="), "no memory parameter: {stored}");
    }

    #[test]
    fn a_damaged_row_is_distinguishable_from_a_wrong_password() {
        assert_eq!(
            verify("x", "not-a-phc-string"),
            Err(PasswordError::MalformedHash)
        );
        assert_eq!(verify("x", ""), Err(PasswordError::MalformedHash));
    }

    #[test]
    fn the_first_few_failures_cost_nothing() {
        let now = OffsetDateTime::UNIX_EPOCH;
        for attempts in 0..FREE_ATTEMPTS {
            assert_eq!(
                locked_until(u32::try_from(attempts).expect("small"), now),
                None,
                "attempt {attempts} was delayed; typos exist"
            );
        }
    }

    #[test]
    fn the_delay_grows_and_then_stops_growing() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let fourth = locked_until(3, now).expect("delayed");
        let seventh = locked_until(6, now).expect("delayed");
        assert!(seventh > fourth);

        // The cap is what keeps this from being a permanent lockout under
        // another name. A thousand failures is still fifteen minutes.
        let capped = locked_until(1_000, now).expect("delayed");
        assert_eq!(capped, now + Duration::minutes(15));
    }

    #[test]
    fn the_ladder_never_reaches_a_permanent_lockout() {
        // docs/40 §Acceptance gates, stated as a property: no rung of the
        // ladder may exceed an interval a person would experience as being
        // locked out.
        for delay in LOCKOUT_LADDER {
            assert!(
                *delay <= Duration::minutes(15),
                "{delay:?} is long enough to be a lockout"
            );
        }
    }
}
