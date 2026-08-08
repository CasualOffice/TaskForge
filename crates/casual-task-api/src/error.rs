//! The error envelope (`docs/05` §Errors, `docs/20` error-code registry).
//!
//! ```json
//! { "error": { "code": "TF-WFL-0004", "message": "...", "details": {...},
//!              "request_id": "018f2c...", "docs": "https://..." } }
//! ```
//!
//! # Two properties this type exists to keep
//!
//! **A `request_id` is always present.** `docs/05`: "a `request_id` the user can
//! quote to support". It is a required field of the constructor rather than an
//! `Option`, so an error cannot be built without one.
//!
//! **404 and 403 are not disambiguated.** `docs/04`: an absent resource and an
//! invisible one return the same thing. That is a decision the *caller* can
//! still get wrong by choosing [`ApiError::forbidden`] for a resource the actor
//! cannot see, so the two constructors say which is which in their own
//! documentation.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// A stable error code from `docs/20`.
///
/// `&'static str` and not a `String`: a code is a compile-time constant from
/// the registry, and one built at runtime is one nobody documented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Code(&'static str);

impl Code {
    /// Declare a code. Only this module's constants call it.
    const fn new(code: &'static str) -> Self {
        Self(code)
    }

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// The codes this crate can currently produce.
///
/// `docs/20` is the registry; these are the subset the server emits before any
/// domain endpoint exists. A code used here that is not in that document is a
/// code no client can look up.
pub mod codes {
    use super::Code;

    /// Malformed request, unknown field, bad filter.
    pub const BAD_REQUEST: Code = Code::new("TF-REQ-0001");
    /// No credential, or one that is not valid.
    pub const UNAUTHENTICATED: Code = Code::new("TF-AUT-0001");
    /// Authenticated, not permitted, **on a resource the actor can see**.
    pub const FORBIDDEN: Code = Code::new("TF-AUT-0002");
    /// Absent or invisible — never disambiguated (`docs/04`).
    pub const NOT_FOUND: Code = Code::new("TF-REQ-0004");
    /// The server is shedding load. Always carries `Retry-After`.
    pub const UNAVAILABLE: Code = Code::new("TF-SRV-0003");
    /// Anything unhandled.
    pub const INTERNAL: Code = Code::new("TF-SRV-0001");

    /// Malformed request body.
    pub const MALFORMED_BODY: Code = Code::new("TF-VAL-0001");
    /// A field the request type does not declare (`docs/05`: rejected, never
    /// ignored).
    pub const UNKNOWN_FIELD: Code = Code::new("TF-VAL-0002");
    /// A field value outside its documented bounds.
    pub const OUT_OF_RANGE: Code = Code::new("TF-VAL-0004");
    /// A referenced entity does not exist **in this workspace**.
    pub const REFERENCE_NOT_FOUND: Code = Code::new("TF-VAL-0007");

    /// The aggregate moved on since the caller read it.
    pub const VERSION_CONFLICT: Code = Code::new("TF-CNC-0001");
    /// `If-Match` is required and was not sent.
    pub const IF_MATCH_REQUIRED: Code = Code::new("TF-CNC-0002");
    /// `If-Match` was sent and is not an ETag this server issued.
    pub const IF_MATCH_MALFORMED: Code = Code::new("TF-CNC-0003");

    /// A workspace would lose its last member.
    pub const LAST_MEMBER: Code = Code::new("TF-PRJ-0006");
    /// The slug is taken by another workspace.
    pub const SLUG_TAKEN: Code = Code::new("TF-PRJ-0007");
    /// The team name is taken inside this workspace.
    pub const TEAM_NAME_TAKEN: Code = Code::new("TF-PRJ-0008");

    /// Every code this crate can emit.
    ///
    /// Listed once so the tests that check the format and the registry cannot
    /// drift from the set they are checking — a code added above and forgotten
    /// here would be a code no gate looks at.
    pub const ALL: &[Code] = &[
        BAD_REQUEST,
        UNAUTHENTICATED,
        FORBIDDEN,
        NOT_FOUND,
        UNAVAILABLE,
        INTERNAL,
        MALFORMED_BODY,
        UNKNOWN_FIELD,
        OUT_OF_RANGE,
        REFERENCE_NOT_FOUND,
        VERSION_CONFLICT,
        IF_MATCH_REQUIRED,
        IF_MATCH_MALFORMED,
        LAST_MEMBER,
        SLUG_TAKEN,
        TEAM_NAME_TAKEN,
    ];
}

/// An error on its way to a client.
#[derive(Debug, Clone)]
pub struct ApiError {
    status: StatusCode,
    code: Code,
    message: String,
    details: Option<serde_json::Value>,
    request_id: String,
    /// Seconds. Present only where `docs/05` requires it.
    retry_after: Option<u32>,
}

#[derive(Serialize)]
struct Envelope<'a> {
    error: Body<'a>,
}

#[derive(Serialize)]
struct Body<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: &'a Option<serde_json::Value>,
    request_id: &'a str,
    docs: String,
}

impl ApiError {
    /// Build an error. `request_id` is required, not optional — see the module
    /// docs.
    #[must_use]
    pub fn new(
        status: StatusCode,
        code: Code,
        message: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: None,
            request_id: request_id.into(),
            retry_after: None,
        }
    }

    /// Machine-readable violations. `docs/05`: return **all** of them at once,
    /// "never the first one — a form that reveals missing fields one round-trip
    /// at a time is a bad form".
    #[must_use]
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// 401 — no credential, or one that is not valid.
    #[must_use]
    pub fn unauthenticated(request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            codes::UNAUTHENTICATED,
            "Authentication is required",
            request_id,
        )
    }

    /// 403 — **only** for a resource the actor can already see.
    ///
    /// If the actor cannot see it, use [`Self::not_found`]: `docs/04` requires
    /// absent and invisible to be indistinguishable, and a 403 here tells an
    /// attacker the resource exists.
    #[must_use]
    pub fn forbidden(request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            codes::FORBIDDEN,
            "You do not have permission to do that",
            request_id,
        )
    }

    /// 404 — absent **or** invisible.
    #[must_use]
    pub fn not_found(request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            codes::NOT_FOUND,
            "Not found",
            request_id,
        )
    }

    /// 503 — shedding load. `docs/05` requires `Retry-After` to be present, so
    /// this constructor sets it rather than trusting a caller to.
    #[must_use]
    pub fn unavailable(request_id: impl Into<String>, retry_after_seconds: u32) -> Self {
        let mut error = Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            codes::UNAVAILABLE,
            "The service is temporarily unable to handle the request",
            request_id,
        );
        error.retry_after = Some(retry_after_seconds);
        error
    }

    /// 400 — malformed, unknown field, or out of range.
    #[must_use]
    pub fn bad_request(
        code: Code,
        message: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message, request_id)
    }

    /// 422 — valid syntax, violates a domain rule (`docs/05`).
    #[must_use]
    pub fn unprocessable(
        code: Code,
        message: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, message, request_id)
    }

    /// 409 — the caller's version is not the current one, or a unique value is
    /// taken.
    #[must_use]
    pub fn conflict(code: Code, message: impl Into<String>, request_id: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message, request_id)
    }

    /// 428 — `If-Match` is required and absent.
    ///
    /// `docs/05` §Concurrency: "428 Precondition Required rather than silently
    /// accepting an unconditional write: a client that forgets `If-Match` has a
    /// bug, and failing loudly in development is better than losing a user's
    /// edit in production."
    #[must_use]
    pub fn precondition_required(request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::PRECONDITION_REQUIRED,
            codes::IF_MATCH_REQUIRED,
            "If-Match is required for this request",
            request_id,
        )
    }

    /// 500 — anything unhandled.
    ///
    /// The message is fixed and generic on purpose: an internal error's detail
    /// belongs in the log, correlated by `request_id`, not in a response where
    /// it becomes reconnaissance.
    #[must_use]
    pub fn internal(request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            codes::INTERNAL,
            "Something went wrong",
            request_id,
        )
    }

    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    #[must_use]
    pub const fn code(&self) -> Code {
        self.code
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(Envelope {
            error: Body {
                code: self.code.as_str(),
                message: &self.message,
                details: &self.details,
                request_id: &self.request_id,
                docs: format!("https://docs.taskforge.dev/errors/{}", self.code.as_str()),
            },
        });

        let mut response = (self.status, body).into_response();
        if let Some(seconds) = self.retry_after {
            // `docs/05`: 429 and 503 always carry Retry-After. Set here rather
            // than at the call site so it cannot be forgotten.
            if let Ok(value) = seconds.to_string().parse() {
                response.headers_mut().insert("retry-after", value);
            }
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_of(error: ApiError) -> serde_json::Value {
        let response = error.into_response();
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    #[tokio::test]
    async fn the_envelope_matches_the_documented_shape() {
        let error = ApiError::not_found("018f2c").with_details(serde_json::json!({"a": 1}));
        let json = body_of(error).await;

        assert_eq!(json["error"]["code"], "TF-REQ-0004");
        assert_eq!(json["error"]["request_id"], "018f2c");
        assert_eq!(json["error"]["details"]["a"], 1);
        assert_eq!(
            json["error"]["docs"],
            "https://docs.taskforge.dev/errors/TF-REQ-0004"
        );
    }

    #[tokio::test]
    async fn details_are_absent_rather_than_null_when_there_are_none() {
        // `null` and "not applicable" are different things to a client that
        // switches on presence.
        let json = body_of(ApiError::unauthenticated("r")).await;
        assert!(
            json["error"].get("details").is_none(),
            "details rendered as null: {json}"
        );
    }

    #[tokio::test]
    async fn an_internal_error_reveals_nothing() {
        // The detail belongs in the log, correlated by request_id. In the
        // response it is reconnaissance.
        let json = body_of(ApiError::internal("r")).await;
        assert_eq!(json["error"]["message"], "Something went wrong");
        assert!(json["error"].get("details").is_none());
    }

    #[test]
    fn service_unavailable_always_carries_retry_after() {
        // docs/05 says "always present" for 429 and 503. The constructor sets
        // it, so a call site cannot omit it.
        let response = ApiError::unavailable("r", 5).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok()),
            Some("5")
        );
    }

    /// The four codes this crate emits that `docs/20` does not define.
    ///
    /// `REQ` and `SRV` are not areas in the registry — the registry's are
    /// `VAL`, `AZN`, `PRJ`, `SYS` and so on — so a client hitting one of these
    /// gets a `docs` URL for a code that is not documented anywhere. They
    /// predate this gate and are named rather than papered over: renaming a
    /// shipped code is a public-contract change (`docs/20` §Rules: codes are
    /// append-only), so it belongs in its own change with a note, not in a list
    /// of new endpoints.
    const NOT_IN_THE_REGISTRY: &[&str] =
        &["TF-REQ-0001", "TF-REQ-0004", "TF-SRV-0001", "TF-SRV-0003"];

    #[test]
    fn every_code_this_crate_emits_is_in_the_registry() {
        // docs/20 is what the `docs` URL in every error body points at. A code
        // that is not there is a link to a 404 in the exact moment a user is
        // trying to understand a failure.
        let registry = include_str!("../../../docs/20-ERROR-CODE-REGISTRY.md");
        for code in codes::ALL {
            if NOT_IN_THE_REGISTRY.contains(&code.as_str()) {
                continue;
            }
            assert!(
                registry.contains(code.as_str()),
                "{code:?} is emitted by this crate and absent from docs/20"
            );
        }
    }

    #[test]
    fn the_documented_status_codes_are_used() {
        assert_eq!(
            ApiError::unauthenticated("r").status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(ApiError::forbidden("r").status(), StatusCode::FORBIDDEN);
        assert_eq!(ApiError::not_found("r").status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn every_code_follows_the_registry_format() {
        // docs/20: TF-XXX-NNNN. A code that does not match is one no client can
        // look up, and the URL in the envelope would 404.
        for code in codes::ALL {
            let text = code.as_str();
            let parts: Vec<_> = text.split('-').collect();
            assert_eq!(parts.len(), 3, "{text}");
            assert_eq!(parts[0], "TF", "{text}");
            assert_eq!(parts[1].len(), 3, "{text}");
            assert!(parts[1].bytes().all(|b| b.is_ascii_uppercase()), "{text}");
            assert_eq!(parts[2].len(), 4, "{text}");
            assert!(parts[2].bytes().all(|b| b.is_ascii_digit()), "{text}");
        }
    }
}
