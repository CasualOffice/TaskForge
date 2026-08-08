//! A JSON body extractor that refuses what `docs/05` says must be refused.
//!
//! # Why not `axum::Json`
//!
//! Two reasons, both visible from outside:
//!
//! 1. **The status is wrong.** `axum::Json`'s deserialization rejection is
//!    `422 Unprocessable Entity`. `docs/05` §Errors is explicit that a
//!    malformed body or an unknown field is `400`, and reserves `422` for
//!    "valid syntax, violates a domain rule". A client switching on the status
//!    would treat a typo'd field name as a business-rule violation.
//! 2. **The body is wrong.** Its rejection renders a bare text string, not the
//!    documented envelope, so the one field `docs/05` promises is always
//!    present — the `request_id` a user can quote to support — is missing from
//!    exactly the responses users complain about.
//!
//! # Unknown fields are the caller's bug, and are reported as such
//!
//! `docs/05` §Conventions: unknown request fields are "**rejected** with `400` —
//! silently ignoring a typo'd field is how clients ship bugs that look like
//! server bugs". The request types carry `#[serde(deny_unknown_fields)]`; this
//! extractor is what turns the resulting serde error into the envelope, with
//! the offending field named in `details` so the client does not have to guess
//! which of its fields was wrong.

use axum::extract::{FromRequest, Request};
use axum::http::header;
use serde::de::DeserializeOwned;

use crate::error::{ApiError, codes};
use crate::server::RequestId;

/// `T`, deserialized from a JSON body, or an [`ApiError`] in the documented
/// shape.
#[derive(Debug, Clone, Copy)]
pub struct ValidJson<T>(pub T);

impl<T, S> FromRequest<S> for ValidJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = RequestId::of_request(&request);

        // The content type is checked rather than assumed. A cross-site form
        // post cannot set `application/json`, so requiring it is a second lock
        // on the door the CSRF guard already holds — and it costs a client
        // nothing, because `docs/05` §Conventions names it as the only content
        // type this API accepts.
        let declared = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !declared
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("application/json")
        {
            return Err(ApiError::bad_request(
                codes::MALFORMED_BODY,
                "The request body must be application/json",
                &request_id,
            ));
        }

        let bytes = axum::body::Bytes::from_request(request, state)
            .await
            .map_err(|_| {
                ApiError::bad_request(
                    codes::MALFORMED_BODY,
                    "The request body could not be read",
                    &request_id,
                )
            })?;

        serde_json::from_slice(&bytes).map(Self).map_err(|error| {
            let message = error.to_string();
            // serde's own wording is the only place the offending field name
            // exists; `deny_unknown_fields` reports it as
            // `unknown field \`foo\`, expected ...`.
            if let Some(field) = unknown_field(&message) {
                ApiError::bad_request(
                    codes::UNKNOWN_FIELD,
                    "The request contains a field this endpoint does not accept",
                    &request_id,
                )
                .with_details(serde_json::json!({ "unknown_fields": [field] }))
            } else {
                ApiError::bad_request(codes::MALFORMED_BODY, "Malformed request body", &request_id)
            }
        })
    }
}

/// The field name out of serde's `unknown field \`x\`, expected ...` message.
fn unknown_field(message: &str) -> Option<&str> {
    let rest = message.strip_prefix("unknown field `")?;
    rest.split('`').next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offending_field_is_named() {
        // Without this the client is told "one of your fields is wrong" and has
        // to bisect its own request body to find out which.
        assert_eq!(
            unknown_field("unknown field `naem`, expected `name` or `slug`"),
            Some("naem")
        );
    }

    #[test]
    fn other_serde_errors_are_not_mistaken_for_unknown_fields() {
        assert_eq!(
            unknown_field("missing field `name` at line 1 column 2"),
            None
        );
        assert_eq!(unknown_field("expected value at line 1 column 1"), None);
    }
}
