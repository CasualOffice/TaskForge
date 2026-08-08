//! `ETag` and `If-Match` (`docs/05` §Concurrency, `docs/24` §Optimistic
//! concurrency).
//!
//! Every mutable aggregate carries `version bigint`, and that number **is** the
//! entity tag: `ETag: "7"`. There is no hashing step, so an `ETag` cannot drift
//! from the row it describes, and the `409` body can quote both versions.
//!
//! # Absent and malformed are different answers
//!
//! `docs/20` gives them separate codes — `TF-CNC-0002` (428) and `TF-CNC-0003`
//! (400) — and the distinction is worth keeping: a missing header is a client
//! that has not implemented concurrency at all, and a malformed one is a client
//! that has, incorrectly. Collapsing them would send the second down the first's
//! documentation.

use axum::http::HeaderMap;
use axum::http::header;

use crate::error::{ApiError, codes};

/// The entity tag for a version. Always a quoted string, per RFC 9110.
#[must_use]
pub fn tag(version: i64) -> String {
    format!("\"{version}\"")
}

/// The version a caller asserts, from `If-Match`.
///
/// # Errors
///
/// - `428 TF-CNC-0002` when the header is absent.
/// - `400 TF-CNC-0003` when it is present and does not parse.
///
/// `If-Match: *` is deliberately **not** accepted. It means "any current
/// representation", which is an unconditional write wearing a conditional
/// header — precisely the silent overwrite `docs/24` exists to prevent.
pub fn if_match(headers: &HeaderMap, request_id: &str) -> Result<i64, ApiError> {
    let Some(raw) = headers.get(header::IF_MATCH) else {
        return Err(ApiError::precondition_required(request_id));
    };
    let malformed = || {
        ApiError::bad_request(
            codes::IF_MATCH_MALFORMED,
            "If-Match must be an entity tag from a previous read, such as \"7\"",
            request_id,
        )
    };
    let value = raw.to_str().map_err(|_| malformed())?;
    parse(value).ok_or_else(malformed)
}

/// `"7"`, `W/"7"`, or `7` → `7`. Anything else → `None`.
///
/// The bare form is tolerated because it is what a hand-written `curl` produces
/// and rejecting it teaches nothing; `*` is not, because accepting it would
/// mean accepting an unconditional write.
fn parse(value: &str) -> Option<i64> {
    let value = value.trim();
    if value == "*" {
        return None;
    }
    let value = value.strip_prefix("W/").unwrap_or(value);
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    value.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(value: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(header::IF_MATCH, HeaderValue::from_str(value).expect("ok"));
        }
        headers
    }

    #[test]
    fn a_tag_round_trips_through_the_header() {
        // The property the whole scheme rests on: what a read returns is what
        // the next write may send back.
        for version in [0_i64, 1, 7, i64::MAX] {
            assert_eq!(parse(&tag(version)), Some(version));
        }
    }

    #[test]
    fn an_absent_header_is_428_and_a_broken_one_is_400() {
        // docs/20 gives them different codes; a client reading the docs URL
        // needs to land on the right page.
        assert_eq!(
            if_match(&headers(None), "r").err().map(|e| e.code()),
            Some(codes::IF_MATCH_REQUIRED)
        );
        for broken in ["", "\"\"", "abc", "\"1.5\"", "7 8"] {
            assert_eq!(
                if_match(&headers(Some(broken)), "r")
                    .err()
                    .map(|e| e.code()),
                Some(codes::IF_MATCH_MALFORMED),
                "{broken:?} was accepted"
            );
        }
    }

    #[test]
    fn a_wildcard_is_refused_rather_than_treated_as_a_match() {
        // `If-Match: *` is an unconditional write wearing a conditional
        // header. Accepting it would reintroduce last-write-wins for any
        // client that sends it, which is the data-loss channel ADR-023 closes.
        assert_eq!(parse("*"), None);
    }

    #[test]
    fn the_weak_and_bare_forms_are_understood() {
        assert_eq!(parse("W/\"7\""), Some(7));
        assert_eq!(parse("7"), Some(7));
        assert_eq!(parse("  \"7\"  "), Some(7));
    }
}
