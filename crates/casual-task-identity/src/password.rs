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

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use time::{Duration, OffsetDateTime};

/// Memory cost, in KiB. `docs/40` §Local authentication: **64 MB**.
pub const MEMORY_KIB: u32 = 64 * 1024;
/// Time cost (passes). `docs/40`: **t=3**.
pub const TIME_COST: u32 = 3;
/// Parallelism (lanes). `docs/40`: **p=4**.
pub const PARALLELISM: u32 = 4;

/// The minimum length of a **human-chosen** password. `docs/40`: "No
/// composition rules beyond a 12-character minimum. Rules produce
/// `Password1!`; length and a breach check produce better passwords."
///
/// Enforced by [`hash_chosen`], which is the only way a chosen password enters
/// the system. A rule checked at each call site is a rule the next endpoint
/// forgets — and this constant existed for a while enforcing nothing at all,
/// which is how that starts.
pub const MIN_LENGTH: usize = 12;

/// The configured hasher.
///
/// **Not `Argon2::default()`.** The crate's defaults are 19 MiB, t=2, p=1 —
/// which is what this module used until it was checked against `docs/40`, and
/// the difference is not cosmetic: memory cost is the parameter that makes
/// GPU and ASIC attacks expensive, and 19 MiB against 64 MB is roughly a
/// threefold discount to an attacker with a dump.
///
/// The parameters are stored in each PHC string, so raising them later does not
/// invalidate existing passwords.
fn hasher() -> Argon2<'static> {
    let params = Params::new(MEMORY_KIB, TIME_COST, PARALLELISM, None)
        .expect("the parameters above are within Argon2's accepted ranges");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

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
    /// A chosen password is shorter than [`MIN_LENGTH`].
    #[error("a password must be at least {minimum} characters")]
    TooShort { minimum: usize },
}

/// Hash a password a **human chose**, enforcing [`MIN_LENGTH`].
///
/// Use this for anything a person types in. [`hash_generated`] is for secrets
/// this system produced, where the length rule is meaningless — a ten-character
/// recovery code from a CSPRNG has more entropy than a twelve-character
/// password from a person, and rejecting it would be enforcing the rule against
/// the wrong thing.
///
/// Two functions rather than one with a flag: a boolean argument at a call site
/// is a decision nobody reads, and this one decides whether a policy applies.
///
/// # Errors
///
/// [`PasswordError::TooShort`] below [`MIN_LENGTH`], or
/// [`PasswordError::Hashing`] if the KDF fails.
pub fn hash_chosen(password: &str) -> Result<String, PasswordError> {
    // Characters, not bytes. `password.len()` is bytes, so a twelve-character
    // passphrase in a non-Latin script would pass a byte check trivially while
    // a shorter one in Latin script would not — the rule would then mean
    // different things in different languages.
    if password.chars().count() < MIN_LENGTH {
        return Err(PasswordError::TooShort {
            minimum: MIN_LENGTH,
        });
    }
    hash_generated(password)
}

/// Hash a secret **this system generated** — a recovery code, a seeded
/// credential. No length policy: see [`hash_chosen`].
///
/// Returns a PHC string, so the parameters travel with the hash and raising the
/// cost later does not invalidate existing passwords.
///
/// # Errors
///
/// [`PasswordError::Hashing`] if the KDF fails.
pub fn hash_generated(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    hasher()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| PasswordError::Hashing(e.to_string()))
}

/// Whether `password` matches `stored`, **off the async runtime**.
///
/// Argon2id at 64 MB and t=3 is ~100 ms of pure CPU with no I/O. Run inline on
/// a tokio worker thread it blocks that thread for the whole time: with the
/// default worker count, a handful of concurrent logins stalls *every* task on
/// the runtime, including health checks and requests that touch nothing.
///
/// # Errors
///
/// [`PasswordError::MalformedHash`], or the blocking pool failing.
pub async fn verify_async(password: &str, stored: &str) -> Result<bool, PasswordError> {
    let password = password.to_owned();
    let stored = stored.to_owned();
    tokio::task::spawn_blocking(move || verify(&password, &stored))
        .await
        .unwrap_or(Err(PasswordError::Hashing(
            "the blocking pool failed".into(),
        )))
}

/// Hash a chosen password off the async runtime. See [`verify_async`].
///
/// # Errors
///
/// [`PasswordError::TooShort`], [`PasswordError::Hashing`].
pub async fn hash_chosen_async(password: &str) -> Result<String, PasswordError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || hash_chosen(&password))
        .await
        .unwrap_or(Err(PasswordError::Hashing(
            "the blocking pool failed".into(),
        )))
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
    // Verification uses the parameters stored in `stored`, not the ones above,
    // which is what lets the cost be raised without invalidating old hashes.
    Ok(hasher()
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
    fn a_chosen_password_below_the_minimum_is_refused() {
        // docs/40's 12-character minimum. It was a constant nothing referenced
        // until this test existed, which is the same as not having the rule.
        assert_eq!(
            hash_chosen("short").err(),
            Some(PasswordError::TooShort {
                minimum: MIN_LENGTH
            })
        );
        assert_eq!(
            hash_chosen(&"a".repeat(MIN_LENGTH - 1)).err(),
            Some(PasswordError::TooShort {
                minimum: MIN_LENGTH
            })
        );
        assert!(hash_chosen(&"a".repeat(MIN_LENGTH)).is_ok());
    }

    #[test]
    fn the_minimum_counts_characters_not_bytes() {
        // Twelve characters of a multi-byte script is 24+ bytes. A byte check
        // would accept four characters of it while rejecting eleven Latin ones,
        // so the rule would mean something different per language.
        let eleven = "ありがとうございます!";
        assert_eq!(eleven.chars().count(), 11);
        assert!(eleven.len() > MIN_LENGTH, "the byte length is not the test");
        assert!(
            hash_chosen(eleven).is_err(),
            "an 11-character password passed because its BYTE length was over 12"
        );
    }

    #[test]
    fn a_generated_secret_has_no_length_policy() {
        // A recovery code is ten characters from a CSPRNG — more entropy than a
        // twelve-character human password. Applying the rule here would enforce
        // it against the wrong thing.
        assert!(hash_generated("SHORT12345").is_ok());
    }

    #[test]
    fn a_password_verifies_against_its_own_hash() {
        let stored = hash_chosen("correct horse battery staple").expect("hashes");
        assert!(verify("correct horse battery staple", &stored).expect("parses"));
        assert!(!verify("Correct horse battery staple", &stored).expect("parses"));
    }

    #[test]
    fn the_stored_hash_does_not_contain_the_password() {
        let stored = hash_generated("hunter2").expect("hashes");
        assert!(!stored.contains("hunter2"));
    }

    #[test]
    fn the_same_password_hashes_differently_each_time() {
        // Per-hash salt. Without it, a dump shows which accounts share a
        // password — which is most of the value of a dump.
        let a = hash_generated("same").expect("hashes");
        let b = hash_generated("same").expect("hashes");
        assert_ne!(a, b);
        assert!(verify("same", &a).expect("parses"));
        assert!(verify("same", &b).expect("parses"));
    }

    #[test]
    fn the_parameters_are_the_ones_the_design_record_specifies() {
        // docs/40 §Local authentication: Argon2id, 64 MB, t=3, p=4. This module
        // used Argon2::default() — 19 MiB, t=2, p=1 — until it was checked.
        // Memory cost is what makes GPU attacks expensive, so the difference is
        // roughly a threefold discount to an attacker holding a dump.
        assert_eq!(MEMORY_KIB, 65_536, "docs/40 says 64 MB");
        assert_eq!(TIME_COST, 3);
        assert_eq!(PARALLELISM, 4);

        let stored = hash_chosen("a password long enough").expect("hashes");
        assert!(stored.contains("m=65536"), "{stored}");
        assert!(stored.contains("t=3"), "{stored}");
        assert!(stored.contains("p=4"), "{stored}");
    }

    #[test]
    fn a_hash_made_with_older_parameters_still_verifies() {
        // The reason parameters live in the PHC string. Raising the cost must
        // not lock every existing user out of their account.
        let weak = Params::new(19 * 1024, 2, 1, None).expect("valid");
        let salt = SaltString::generate(&mut OsRng);
        let old = Argon2::new(Algorithm::Argon2id, Version::V0x13, weak)
            .hash_password(b"a password long enough", &salt)
            .expect("hashes")
            .to_string();

        assert!(
            verify("a password long enough", &old).expect("parses"),
            "a password hashed with the previous parameters stopped working"
        );
    }

    #[test]
    fn the_hash_carries_its_parameters() {
        // A PHC string, so raising the cost later does not invalidate every
        // existing password.
        let stored = hash_generated("x").expect("hashes");
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
