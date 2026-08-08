//! Selector/verifier credentials — sessions, API tokens, invitations, resets.
//!
//! `docs/40` §Credential lookup, ADR-032. A presented credential is
//! `<selector>.<verifier>`:
//!
//! - **selector** — non-secret, uniquely indexed. Finds the row in one index
//!   read, before any secret is examined.
//! - **verifier** — 192 bits of randomness, stored only as a salted hash.
//!
//! # Why not one hashed column with a unique index
//!
//! That was the shape `api_token` had, and it forces a choice between two bad
//! options. Salting the hash makes the row unfindable without already knowing
//! which row it is; not salting it makes the column a rainbow-table target and
//! leaks equality between rows — two users with the same token would be
//! visible as such in a dump.
//!
//! Splitting the credential removes the choice: the lookup key is public, and
//! the secret is salted per row.
//!
//! # Why not a keyed HMAC under a server pepper
//!
//! Proposed and rejected in ADR-032. It makes a secret outside the database
//! load-bearing for **every** authentication: lose it and every session and
//! token dies at once; rotate it and they die unless a versioning window
//! exists, which forces a key id onto two tables and a key custody procedure
//! into the runbooks. Selector/verifier buys the same property — a dump
//! contains no usable credential — for a longer token and no key.
//!
//! # Why SHA-256 here and Argon2id on passwords
//!
//! A verifier is 192 bits of output from a CSPRNG. There is no dictionary to
//! attack and no meaningful search space, so a slow KDF buys nothing while
//! costing latency on every authenticated request — which `docs/21` budgets. A
//! password is a low-entropy secret chosen by a human, and there the slow KDF
//! is the only thing between a dump and an offline attack. Both facts are in
//! [`password`](crate::password).

use rand::TryRngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Bytes of randomness in a verifier.
///
/// 24 bytes = 192 bits. `docs/40` says "~190 bits"; this is the nearest whole
/// number of bytes at or above it. Well beyond brute force, and the reason no
/// slow hash is needed on this path.
pub const VERIFIER_BYTES: usize = 24;

/// Bytes of randomness in a selector.
///
/// Shorter than the verifier because it is not a secret — it only has to be
/// unique. 12 bytes makes an accidental collision impossible in practice
/// (birthday bound ~2^48) without inviting anyone to treat it as one.
pub const SELECTOR_BYTES: usize = 12;

/// The argument for using SHA-256 on verifiers instead of Argon2id rests
/// entirely on this number, so it is checked at compile time rather than by a
/// test that could be deleted. `docs/40` says "~190 bits".
const _: () = assert!(
    VERIFIER_BYTES * 8 >= 190,
    "a verifier shorter than ~190 bits needs a slow KDF; see the module docs"
);

/// The separator between the two halves in the presented string.
///
/// `.` rather than `:` — the credential travels in an `Authorization` header
/// and in a cookie value, and `:` is meaningful in the first.
const SEPARATOR: char = '.';

/// Something went wrong parsing or verifying a presented credential.
///
/// Deliberately coarse. `docs/40` §Acceptance gates requires login responses to
/// be indistinguishable for existing and non-existing accounts; an error type
/// that distinguished "no such selector" from "wrong verifier" would be an
/// enumeration oracle the moment any caller logged or returned it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the credential is not valid")]
pub struct Invalid;

/// A freshly minted credential: what to store, and what to show once.
///
/// The plaintext is in `presented` and is **never** recoverable afterwards —
/// only its hash is stored. That is what makes a database dump not a
/// credential dump.
#[derive(Debug, Clone)]
pub struct Minted {
    /// Store this. Non-secret, uniquely indexed.
    pub selector: String,
    /// Store this. Salted hash of the verifier.
    pub verifier_hash: String,
    /// Show this once, to the user or the client. Not stored anywhere.
    pub presented: String,
}

/// Mint a new credential.
///
/// # Errors
///
/// If the operating system's randomness source fails. Propagated rather than
/// unwrapped: a credential minted from a degraded entropy source is exactly the
/// failure that must not be papered over with a fallback.
pub fn mint() -> Result<Minted, rand::rand_core::OsError> {
    let mut selector_bytes = [0u8; SELECTOR_BYTES];
    let mut verifier_bytes = [0u8; VERIFIER_BYTES];
    rand::rngs::OsRng.try_fill_bytes(&mut selector_bytes)?;
    rand::rngs::OsRng.try_fill_bytes(&mut verifier_bytes)?;

    let selector = hex(&selector_bytes);
    let verifier = hex(&verifier_bytes);
    let mut salt = [0u8; 16];
    rand::rngs::OsRng.try_fill_bytes(&mut salt)?;

    Ok(Minted {
        verifier_hash: hash_verifier(&verifier, &hex(&salt)),
        presented: format!("{selector}{SEPARATOR}{verifier}"),
        selector,
    })
}

/// Split a presented credential into its selector and verifier.
///
/// # Errors
///
/// [`Invalid`] if it is not `<selector>.<verifier>` with both halves the
/// expected length and both halves hex. Checked here so that a malformed
/// credential is rejected before it reaches a database query or a hash.
///
/// **Both** halves, not only the selector. [`mint`] emits hex on both sides, so
/// anything else is malformed by construction — and accepting it means 48 bytes
/// of arbitrary input reach [`verify`], and through it a hash of whatever an
/// unauthenticated caller sent.
pub fn split(presented: &str) -> Result<(&str, &str), Invalid> {
    let (selector, verifier) = presented.split_once(SEPARATOR).ok_or(Invalid)?;
    if selector.len() != SELECTOR_BYTES * 2 || verifier.len() != VERIFIER_BYTES * 2 {
        return Err(Invalid);
    }
    if !is_hex(selector) || !is_hex(verifier) {
        return Err(Invalid);
    }
    Ok((selector, verifier))
}

fn is_hex(half: &str) -> bool {
    half.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether `verifier` matches `stored`.
///
/// Constant-time in the hash comparison. Comparing with `==` returns as soon as
/// two bytes differ, which leaks how much of a guess was correct — the one
/// place in this codebase where that matters enough to reach for `subtle`.
#[must_use]
pub fn verify(verifier: &str, stored: &str) -> bool {
    let Some((salt, _)) = stored.split_once('$') else {
        // A stored value that is not `salt$hash` cannot match anything. Returns
        // false rather than erroring: a malformed row is an authentication
        // failure, not a different kind of answer the caller must handle.
        return false;
    };
    let computed = hash_verifier(verifier, salt);
    computed.as_bytes().ct_eq(stored.as_bytes()).into()
}

/// `salt$hex(sha256(salt || verifier))`.
///
/// The salt travels with the hash, as it does in a PHC string, so a row is
/// self-describing and rotating the salt is a per-row operation.
fn hash_verifier(verifier: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(verifier.as_bytes());
    format!("{salt}${}", hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_minted_credential_verifies_against_what_was_stored() {
        let minted = mint().expect("entropy");
        let (selector, verifier) = split(&minted.presented).expect("well formed");
        assert_eq!(selector, minted.selector);
        assert!(verify(verifier, &minted.verifier_hash));
    }

    #[test]
    fn the_stored_form_contains_no_usable_credential() {
        // docs/40 §Acceptance gates, "token-hash test": a database dump is not a
        // credential dump. The two stored columns are the selector and the
        // hash; neither may contain the verifier.
        let minted = mint().expect("entropy");
        let (_, verifier) = split(&minted.presented).expect("well formed");

        assert!(
            !minted.verifier_hash.contains(verifier),
            "the verifier is recoverable from the stored hash"
        );
        assert!(
            !minted.selector.contains(verifier),
            "the verifier is recoverable from the selector"
        );
        // And the presented form is not stored anywhere in the struct's stored
        // half — checked explicitly because a future refactor that "helpfully"
        // kept it would be silent.
        assert_ne!(minted.verifier_hash, minted.presented);
    }

    #[test]
    fn a_wrong_verifier_is_refused() {
        let minted = mint().expect("entropy");
        let other = mint().expect("entropy");
        let (_, wrong) = split(&other.presented).expect("well formed");
        assert!(!verify(wrong, &minted.verifier_hash));
    }

    #[test]
    fn two_credentials_with_the_same_verifier_hash_to_different_values() {
        // Per-row salt, asserted. Without it, equal secrets produce equal
        // hashes and a dump reveals which accounts share a credential.
        let salt_a = "aaaa";
        let salt_b = "bbbb";
        let verifier = "the same secret";
        assert_ne!(
            hash_verifier(verifier, salt_a),
            hash_verifier(verifier, salt_b)
        );
    }

    #[test]
    fn selectors_do_not_repeat() {
        // Not a proof, a smoke test: the column is UNIQUE, so a generator with
        // a narrow range would surface as insertion failures in production
        // rather than here.
        let mut seen = HashSet::new();
        for _ in 0..1_000 {
            assert!(
                seen.insert(mint().expect("entropy").selector),
                "a selector repeated within 1000 mints"
            );
        }
    }

    #[test]
    fn malformed_credentials_are_refused_before_any_lookup() {
        // Each of these would otherwise reach the database as a query
        // parameter. Rejecting them here keeps a malformed credential from
        // being indistinguishable from a wrong one in the logs.
        for bad in [
            "",
            "no-separator",
            ".",
            "short.short",
            &format!("{}.{}", "0".repeat(24), "0".repeat(47)), // verifier one short
            &format!("{}.{}", "0".repeat(23), "0".repeat(48)), // selector one short
            &format!("{}.{}", "z".repeat(24), "0".repeat(48)), // selector not hex
            // The verifier half, which was length-checked but never validated:
            // without the check these reached `verify` as-is, so an
            // unauthenticated caller chose 48 bytes of hash input.
            &format!("{}.{}", "0".repeat(24), "z".repeat(48)),
            &format!("{}.{}", "0".repeat(24), "0".repeat(47) + " "),
            &format!("{}.{}", "0".repeat(24), "0".repeat(47) + "%"),
        ] {
            assert_eq!(split(bad), Err(Invalid), "accepted {bad:?}");
        }
    }

    #[test]
    fn a_well_formed_credential_still_splits() {
        // The companion to the refusals above: a validator that rejected
        // everything would satisfy that test and break every login.
        let minted = mint().expect("entropy");
        let (selector, verifier) = split(&minted.presented).expect("well formed");
        assert_eq!(selector.len(), SELECTOR_BYTES * 2);
        assert_eq!(verifier.len(), VERIFIER_BYTES * 2);
        // Upper-case hex is accepted too: a proxy or client that normalised the
        // case would otherwise turn a valid credential into a parse failure.
        assert!(split(&minted.presented.to_uppercase()).is_ok());
    }

    #[test]
    fn a_malformed_stored_hash_fails_closed() {
        // A row damaged by a bad migration must refuse authentication, not
        // panic and not accept.
        assert!(!verify("anything", ""));
        assert!(!verify("anything", "no-dollar-sign"));
    }
}
