//! The wire conventions every endpoint shares (`docs/05` §Conventions).
//!
//! - `snake_case` field names, which serde gives us because the Rust fields are
//!   already snake_case.
//! - RFC 3339 timestamps, **always UTC, always `Z`** — see [`timestamp`].
//! - Unknown request fields **rejected**, with `400 TF-VAL-0002`.
//! - Cursor pagination only, in the documented envelope.
//!
//! # Why the body is not `axum::Json`
//!
//! `Json<T>` rejects a bad body with `422` and a plain-text reason. `docs/05`
//! says an unknown field is `400`, and `docs/20` says every error carries a
//! registry code and a `request_id`. [`Body`] reads the bytes itself so the
//! rejection is the product's error envelope rather than the framework's.

use axum::extract::{FromRequest, Request};
use axum::http::header;
use serde::Serialize;
use serde::de::DeserializeOwned;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::{ApiError, codes};
use crate::server::RequestId;

/// The largest JSON body any endpoint in this module accepts.
///
/// `docs/21` bounds every input. A task description is capped at 64 KiB by the
/// schema, so 256 KiB leaves room for the rest of the object and nothing like
/// enough for an attack.
pub const MAX_BODY: usize = 256 * 1024;

/// A JSON request body that refuses what it does not recognise.
#[derive(Debug)]
pub struct Body<T>(pub T);

impl<S, T> FromRequest<S> for Body<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = RequestId::of_request(&request);

        // A missing or wrong content type is a malformed request, not an
        // unknown field: saying so points the client at the right line.
        let json = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"));
        if !json {
            return Err(ApiError::bad_request(
                codes::MALFORMED_BODY,
                "Content-Type must be application/json",
                &request_id,
            ));
        }

        let bytes = axum::body::Bytes::from_request(request, state)
            .await
            .map_err(|_| {
                ApiError::bad_request(codes::MALFORMED_BODY, "Unreadable body", &request_id)
            })?;
        if bytes.len() > MAX_BODY {
            return Err(ApiError::new(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                codes::OUT_OF_RANGE,
                "Request body too large",
                &request_id,
            ));
        }

        serde_json::from_slice::<T>(&bytes)
            .map(Self)
            .map_err(|error| classify(&error, &request_id))
    }
}

/// Map a deserialization failure onto the registry.
///
/// The serde message is included in `details` on purpose: it names the field,
/// which is exactly what `docs/05` means by "silently ignoring a typo'd field
/// is how clients ship bugs that look like server bugs". It describes the
/// *request*, so it leaks nothing about the server or another tenant.
fn classify(error: &serde_json::Error, request_id: &str) -> ApiError {
    let text = error.to_string();
    let code = if text.starts_with("unknown field") {
        codes::UNKNOWN_FIELD
    } else if text.starts_with("missing field") {
        codes::MISSING_FIELD
    } else {
        // Syntax, EOF, and every other data error. `docs/20` has finer codes
        // for range and enum violations; those are checked by the handlers,
        // which know the field, rather than guessed from a serde message.
        codes::MALFORMED_BODY
    };
    ApiError::bad_request(code, "The request body was not accepted", request_id)
        .with_details(serde_json::json!({ "reason": text }))
}

/// RFC 3339, UTC, with `Z` — `docs/05` §Conventions.
///
/// `time`'s RFC 3339 writes `+00:00` for UTC, which is valid RFC 3339 and not
/// what the contract says. Two clients that string-compare timestamps would
/// disagree about two representations of the same instant, so the offset is
/// normalized here rather than at each call site.
#[must_use]
pub fn timestamp(at: OffsetDateTime) -> String {
    at.to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .unwrap_or_default()
        .replace("+00:00", "Z")
}

/// `docs/05` §Pagination: `limit` default 50, max 100.
pub const DEFAULT_LIMIT: u32 = 50;
/// The hard ceiling. `docs/26` §Query limits: "bounds work per request".
pub const MAX_LIMIT: u32 = 100;

/// The page envelope every list endpoint returns.
#[derive(Debug, Serialize)]
pub struct Page {
    /// Opaque. `docs/05`: "clients must not parse it".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

/// `{ "data": [...], "page": { ... } }`.
#[derive(Debug, Serialize)]
pub struct Paged<T> {
    pub data: Vec<T>,
    pub page: Page,
}

/// Validate a requested page size.
///
/// # Errors
///
/// `400 TF-QRY-0007` when it exceeds [`MAX_LIMIT`]. Over-limit is refused
/// rather than clamped: a client asking for 500 and silently receiving 100
/// concludes there were only 100.
pub fn limit(requested: Option<u32>, request_id: &str) -> Result<u32, ApiError> {
    match requested {
        None => Ok(DEFAULT_LIMIT),
        Some(0) => Err(ApiError::bad_request(
            codes::PAGE_TOO_LARGE,
            "limit must be at least 1",
            request_id,
        )),
        Some(n) if n > MAX_LIMIT => Err(ApiError::bad_request(
            codes::PAGE_TOO_LARGE,
            format!("limit must be at most {MAX_LIMIT}"),
            request_id,
        )),
        Some(n) => Ok(n),
    }
}

/// Decode a cursor, or refuse it with the registry code.
///
/// # Errors
///
/// `400 TF-QRY-0006`.
pub fn cursor(
    raw: Option<&str>,
    request_id: &str,
) -> Result<Option<casual_task_model::Cursor>, ApiError> {
    let Some(raw) = raw.filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    casual_task_model::Cursor::decode(raw)
        .map(Some)
        .map_err(|_| {
            ApiError::bad_request(codes::BAD_CURSOR, "Malformed pagination cursor", request_id)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn timestamps_are_utc_with_a_z() {
        // docs/05: "RFC 3339, always UTC, always `Z`". `time`'s own RFC 3339
        // writes +00:00, which is valid and is not the contract.
        assert_eq!(
            timestamp(datetime!(2026-08-08 10:14:22 UTC)),
            "2026-08-08T10:14:22Z"
        );
        // A non-UTC input is converted, not relabelled.
        assert_eq!(
            timestamp(datetime!(2026-08-08 12:14:22 +2)),
            "2026-08-08T10:14:22Z"
        );
    }

    #[test]
    fn an_over_limit_page_is_refused_rather_than_clamped() {
        // Clamping is the friendly-looking answer and it lies: a client that
        // asked for 500 and received 100 concludes there were only 100.
        assert_eq!(limit(None, "r").ok(), Some(DEFAULT_LIMIT));
        assert_eq!(limit(Some(100), "r").ok(), Some(100));
        assert_eq!(
            limit(Some(101), "r").err().map(|e| e.code()),
            Some(codes::PAGE_TOO_LARGE)
        );
        assert_eq!(
            limit(Some(0), "r").err().map(|e| e.code()),
            Some(codes::PAGE_TOO_LARGE)
        );
    }

    #[test]
    fn a_garbage_cursor_is_a_400_with_the_registry_code() {
        assert!(cursor(None, "r").expect("none is fine").is_none());
        assert!(cursor(Some(""), "r").expect("empty is none").is_none());
        assert_eq!(
            cursor(Some("!!!not-a-cursor!!!"), "r")
                .err()
                .map(|e| e.code()),
            Some(codes::BAD_CURSOR)
        );
    }

    #[test]
    fn a_cursor_this_module_encodes_is_one_it_accepts() {
        let c = casual_task_model::Cursor::new(
            vec!["2026-08-08T10:14:22Z".into()],
            uuid::Uuid::now_v7(),
        );
        assert_eq!(cursor(Some(&c.encode()), "r").expect("round trip"), Some(c));
    }
}
