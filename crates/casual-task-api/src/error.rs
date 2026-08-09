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

    /// Adopt a code the model layer already produced.
    ///
    /// `casual-task-search` reports refusals as
    /// [`casual_task_model::ErrorCode`], which is the same registry
    /// (`docs/20`) behind a different newtype — the model crate cannot depend
    /// on this one. Both wrap a `&'static str` from the registry, so carrying
    /// it across is the identity, and doing it here keeps the API crate from
    /// re-deciding what a filter error is called.
    #[must_use]
    pub fn from_registry(code: casual_task_model::ErrorCode) -> Self {
        Self(code.as_str())
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

    /// No credential, or one that is not valid.
    pub const UNAUTHENTICATED: Code = Code::new("TF-AUT-0001");
    /// The credential is valid and is not the kind this endpoint accepts.
    ///
    /// A bearer token is "scoped to one workspace" (`docs/40`), so using one on
    /// a route that is *about* choosing a workspace is outside the contract it
    /// was issued under. Not `TF-AZN-0001`: the fix is a different credential,
    /// not a different role.
    pub const WRONG_CREDENTIAL_TYPE: Code = Code::new("TF-AUT-0013");
    /// The CSRF token was missing or did not verify.
    pub const CSRF: Code = Code::new("TF-AUT-0008");
    /// Absent or invisible — never disambiguated (`docs/04`).
    ///
    /// The generic form of `TF-PRJ-0001` and `TF-TSK-0001`, for the resources
    /// that have no code of their own.
    pub const NOT_FOUND: Code = Code::new("TF-AZN-0008");
    /// The last grant carrying `workspace.owner` cannot be removed or
    /// downgraded (`docs/04` control 4, migration 0021).
    pub const LAST_OWNER: Code = Code::new("TF-AZN-0005");
    /// Too many requests. Always carries `Retry-After`.
    pub const RATE_LIMITED: Code = Code::new("TF-LIM-0001");
    /// The service is temporarily unable to answer. Always carries
    /// `Retry-After`.
    pub const UNAVAILABLE: Code = Code::new("TF-SYS-0002");
    /// Anything unhandled.
    pub const INTERNAL: Code = Code::new("TF-SYS-0001");

    // ---------------------------------------------------------------------
    // C-006 / C-008. Every code below is copied from `docs/20`, area and
    // number, rather than invented — a code that is not in that registry is a
    // code no client can look up and the `docs` URL in the envelope 404s.
    // ---------------------------------------------------------------------

    /// Malformed request body.
    pub const MALFORMED_BODY: Code = Code::new("TF-VAL-0001");
    /// Unknown field in request. `docs/05`: "silently ignoring a typo'd field
    /// is how clients ship bugs that look like server bugs".
    pub const UNKNOWN_FIELD: Code = Code::new("TF-VAL-0002");
    /// Required field missing.
    pub const MISSING_FIELD: Code = Code::new("TF-VAL-0003");
    /// Field value out of range.
    pub const OUT_OF_RANGE: Code = Code::new("TF-VAL-0004");
    /// Invalid enum value.
    pub const INVALID_ENUM: Code = Code::new("TF-VAL-0005");
    /// Referenced entity not found.
    pub const REFERENCE_NOT_FOUND: Code = Code::new("TF-VAL-0007");

    /// Permission denied — no grant carried it.
    pub const NO_GRANT: Code = Code::new("TF-AZN-0001");
    /// Permission denied — a grant carried it, but not for this object.
    pub const CONSTRAINT_UNSATISFIED: Code = Code::new("TF-AZN-0002");

    /// Invalid or expired cursor.
    pub const BAD_CURSOR: Code = Code::new("TF-QRY-0006");
    /// Page size over limit.
    pub const PAGE_TOO_LARGE: Code = Code::new("TF-QRY-0007");
    /// Unknown filter field.
    pub const UNKNOWN_FILTER_FIELD: Code = Code::new("TF-QRY-0001");
    /// Unknown or unsortable sort field.
    pub const UNSORTABLE_FIELD: Code = Code::new("TF-QRY-0002");
    /// Operator not valid for this field type.
    pub const BAD_OPERATOR: Code = Code::new("TF-QRY-0003");
    /// Too many filter clauses.
    pub const TOO_MANY_CLAUSES: Code = Code::new("TF-QRY-0004");
    /// Filter nesting too deep.
    pub const FILTER_TOO_DEEP: Code = Code::new("TF-QRY-0005");
    /// Search query too long.
    pub const SEARCH_TOO_LONG: Code = Code::new("TF-QRY-0008");
    /// A symbol (`@me`, `+7d`) this server does not know.
    ///
    /// `docs/20` has no code for it, so it reports as the operator/value code:
    /// an unrecognised symbol is a malformed value for the field it was written
    /// on. Recorded in `docs/14` as a registry gap rather than a new area.
    pub const UNKNOWN_SYMBOL: Code = Code::new("TF-QRY-0003");

    /// Project not found or not visible — never disambiguated.
    pub const PROJECT_NOT_FOUND: Code = Code::new("TF-PRJ-0001");
    /// Project key already in use.
    pub const PROJECT_KEY_TAKEN: Code = Code::new("TF-PRJ-0002");
    /// Project key is immutable (ADR-007).
    pub const PROJECT_KEY_IMMUTABLE: Code = Code::new("TF-PRJ-0003");
    /// Project key format invalid.
    pub const PROJECT_KEY_FORMAT: Code = Code::new("TF-PRJ-0004");

    /// Task not found or not visible — never disambiguated.
    pub const TASK_NOT_FOUND: Code = Code::new("TF-TSK-0001");
    /// Assignee is not a member of the project.
    pub const ASSIGNEE_NOT_PROJECT_MEMBER: Code = Code::new("TF-TSK-0005");
    /// Parent task must be in the same project (ADR-018).
    pub const PARENT_OUT_OF_PROJECT: Code = Code::new("TF-TSK-0006");

    /// Status cannot be set directly — use a transition (`docs/23`).
    pub const STATUS_NOT_DIRECTLY_WRITABLE: Code = Code::new("TF-WFL-0001");
    /// No such transition in this workflow.
    pub const NO_SUCH_TRANSITION: Code = Code::new("TF-WFL-0002");
    /// The transition requires a permission the actor lacks.
    pub const TRANSITION_PERMISSION: Code = Code::new("TF-WFL-0003");
    /// Required fields missing for the target status.
    pub const TRANSITION_FIELDS_MISSING: Code = Code::new("TF-WFL-0004");
    /// Blocking dependencies unresolved.
    pub const BLOCKED_BY_DEPENDENCIES: Code = Code::new("TF-WFL-0005");

    /// Version conflict.
    pub const VERSION_CONFLICT: Code = Code::new("TF-CNC-0001");
    /// `If-Match` required.
    pub const IF_MATCH_REQUIRED: Code = Code::new("TF-CNC-0002");
    /// Malformed `If-Match`.
    pub const IF_MATCH_MALFORMED: Code = Code::new("TF-CNC-0003");

    /// A request with this idempotency key is already in progress.
    pub const IDEMPOTENCY_IN_PROGRESS: Code = Code::new("TF-IDM-0001");
    /// Idempotency key reused with a different body.
    pub const IDEMPOTENCY_BODY_CHANGED: Code = Code::new("TF-IDM-0002");
    /// Idempotency key required.
    pub const IDEMPOTENCY_REQUIRED: Code = Code::new("TF-IDM-0003");

    /// A workspace would lose its last member.
    pub const LAST_MEMBER: Code = Code::new("TF-PRJ-0006");
    /// The slug is taken by another workspace.
    pub const SLUG_TAKEN: Code = Code::new("TF-PRJ-0007");
    /// The team name is taken inside this workspace.
    pub const TEAM_NAME_TAKEN: Code = Code::new("TF-PRJ-0008");

    /// Every code this binary can emit.
    ///
    /// The registry gate walks this list, so a code missing from it is a
    /// code whose `docs` URL is never checked against `docs/20`.
    pub const ALL: &[Code] = &[
        UNAUTHENTICATED,
        WRONG_CREDENTIAL_TYPE,
        CSRF,
        NOT_FOUND,
        LAST_OWNER,
        RATE_LIMITED,
        UNAVAILABLE,
        INTERNAL,
        MALFORMED_BODY,
        UNKNOWN_FIELD,
        MISSING_FIELD,
        OUT_OF_RANGE,
        INVALID_ENUM,
        REFERENCE_NOT_FOUND,
        NO_GRANT,
        CONSTRAINT_UNSATISFIED,
        BAD_CURSOR,
        PAGE_TOO_LARGE,
        UNKNOWN_FILTER_FIELD,
        UNSORTABLE_FIELD,
        BAD_OPERATOR,
        TOO_MANY_CLAUSES,
        FILTER_TOO_DEEP,
        SEARCH_TOO_LONG,
        PROJECT_NOT_FOUND,
        PROJECT_KEY_TAKEN,
        PROJECT_KEY_IMMUTABLE,
        PROJECT_KEY_FORMAT,
        TASK_NOT_FOUND,
        ASSIGNEE_NOT_PROJECT_MEMBER,
        PARENT_OUT_OF_PROJECT,
        STATUS_NOT_DIRECTLY_WRITABLE,
        NO_SUCH_TRANSITION,
        TRANSITION_PERMISSION,
        TRANSITION_FIELDS_MISSING,
        BLOCKED_BY_DEPENDENCIES,
        VERSION_CONFLICT,
        IF_MATCH_REQUIRED,
        IF_MATCH_MALFORMED,
        IDEMPOTENCY_IN_PROGRESS,
        IDEMPOTENCY_BODY_CHANGED,
        IDEMPOTENCY_REQUIRED,
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
    ///
    /// The code is a parameter because 403 is not one answer. `docs/20` gives
    /// a CSRF failure, a wrong credential type and a missing grant separate
    /// codes, and they lead a user to three different actions: retry with a
    /// token, use a different credential, ask an admin. One shared code sends
    /// all three to the same documentation page — and the one this used to
    /// send them to was `TF-AUT-0002`, "session expired", which is a fourth
    /// thing none of them is.
    #[must_use]
    pub fn forbidden(code: Code, request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            code,
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

    /// 429 — rate limited. `docs/05` requires `Retry-After` on every one, so
    /// `retry_after_seconds` is a parameter and not a builder step: a call site
    /// cannot produce a 429 without saying when to come back.
    ///
    /// The message names no limit and no address. A refusal that told an
    /// attacker which bucket they exhausted, or how many attempts remained,
    /// would be a tuning aid — the numbers a legitimate client needs are in the
    /// `RateLimit-*` headers, which are on successes too.
    #[must_use]
    pub fn too_many_requests(request_id: impl Into<String>, retry_after_seconds: u32) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            codes::RATE_LIMITED,
            "Too many requests",
            request_id,
        );
        error.retry_after = Some(retry_after_seconds);
        error
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

    /// 400 with a registry code — malformed body, unknown field, bad cursor.
    #[must_use]
    pub fn bad_request(
        code: Code,
        message: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message, request_id)
    }

    /// 404 with the resource's own registry code.
    ///
    /// The **code** differs per resource; the shape does not, and neither does
    /// the answer for "absent" and "invisible" — `docs/04` requires those two
    /// to be indistinguishable, and they are, because one handler returns this
    /// for both.
    #[must_use]
    pub fn missing(code: Code, request_id: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, "Not found", request_id)
    }

    /// 403 for an authorization denial on a resource the actor **can** see.
    ///
    /// `docs/20`: `TF-AZN-0001` and `-0002` are distinct on purpose — "you were
    /// never given this" and "you have it, but not for this object" lead a user
    /// to different actions.
    #[must_use]
    pub fn denied(code: Code, request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            code,
            "You do not have permission to do that",
            request_id,
        )
    }

    /// 422 — valid syntax, violates a domain rule.
    #[must_use]
    pub fn unprocessable(
        code: Code,
        message: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, message, request_id)
    }

    /// 409 — a conflict with the resource's current state.
    #[must_use]
    pub fn conflict(code: Code, message: impl Into<String>, request_id: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message, request_id)
    }

    /// 428 — `If-Match` is required.
    ///
    /// `docs/05`: "a client that forgets `If-Match` has a bug, and failing
    /// loudly in development is better than losing a user's edit in
    /// production." There is deliberately no way to perform an unconditional
    /// update, so this is not a mode anything can opt out of.
    #[must_use]
    pub fn precondition_required(request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::PRECONDITION_REQUIRED,
            codes::IF_MATCH_REQUIRED,
            "If-Match is required for this request",
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

        assert_eq!(json["error"]["code"], "TF-AZN-0008");
        assert_eq!(json["error"]["request_id"], "018f2c");
        assert_eq!(json["error"]["details"]["a"], 1);
        assert_eq!(
            json["error"]["docs"],
            "https://docs.taskforge.dev/errors/TF-AZN-0008"
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
    fn a_rate_limit_refusal_always_carries_retry_after() {
        // docs/05: "429 | rate limited (`Retry-After` always present)". The
        // constructor takes the value, so there is no path to a 429 without it —
        // a client told to back off with no idea for how long retries
        // immediately, which is the flood the limiter was added to stop.
        let response = ApiError::too_many_requests("r", 6).into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok()),
            Some("6")
        );
    }

    #[tokio::test]
    async fn a_rate_limit_refusal_names_no_limit_and_no_address() {
        // The body is reconnaissance if it says what was exceeded. The numbers a
        // legitimate client needs are the RateLimit-* headers.
        let json = body_of(ApiError::too_many_requests("r", 6)).await;
        assert_eq!(json["error"]["code"], "TF-LIM-0001");
        assert_eq!(json["error"]["message"], "Too many requests");
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

    #[test]
    fn the_documented_status_codes_are_used() {
        assert_eq!(
            ApiError::unauthenticated("r").status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            ApiError::forbidden(codes::CSRF, "r").status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(ApiError::not_found("r").status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn every_code_this_binary_emits_is_in_the_registry() {
        // docs/20 is what the `docs` URL in every error body points at. A code
        // that is not there is a link to a 404 in the exact moment a user is
        // trying to understand a failure.
        //
        // There is deliberately NO exception list. C-002 shipped this gate with
        // four — TF-REQ-0001, TF-REQ-0004, TF-SRV-0001, TF-SRV-0003, in two
        // areas the registry does not define — and opened D-055 rather than
        // resolving it. D-055 is now resolved: the four were retired in favour
        // of registry codes, which was safe for a reason that will not be true
        // again — none of them had ever been released. An exception list is how
        // a gate stops holding, one entry at a time, so this one has nowhere to
        // put the next one.
        //
        // (This test also went missing: it exists on `feat/c002-workspaces` and
        // was dropped by the merge into `feat/phase-1`. Restored here.)
        let registry = include_str!("../../../docs/20-ERROR-CODE-REGISTRY.md");
        for code in codes::ALL {
            assert!(
                registry.contains(code.as_str()),
                "{code:?} is emitted by this binary and absent from docs/20"
            );
        }
    }

    #[test]
    fn the_registry_gate_can_fail() {
        // A gate nobody has watched fail is a gate nobody knows works. The
        // retired codes are the values the check above would have to reject.
        let registry = include_str!("../../../docs/20-ERROR-CODE-REGISTRY.md");
        for retired in ["TF-REQ-0001", "TF-REQ-0004", "TF-SRV-0001", "TF-SRV-0003"] {
            assert!(
                !registry.contains(retired),
                "{retired} is back in the registry, so the gate above would \
                 pass for a code that should not exist"
            );
            assert!(
                !codes::ALL.iter().any(|c| c.as_str() == retired),
                "{retired} is emitted again"
            );
        }
    }

    #[test]
    fn the_area_of_every_code_is_one_the_registry_declares() {
        // Stronger than containment: `TF-XYZ-0001` would pass the test above if
        // the string happened to appear anywhere in the prose. The registry
        // declares its areas in one table, and a code outside them is a code in
        // an area nobody defined — which is exactly what TF-REQ-* and TF-SRV-*
        // were.
        let areas = [
            "AUT", "AZN", "VAL", "QRY", "WFL", "TSK", "PRJ", "CNC", "IDM", "ATT", "PLG", "AUM",
            "LIM", "SYS",
        ];
        for code in codes::ALL {
            let area = code.as_str().split('-').nth(1).unwrap_or_default();
            assert!(
                areas.contains(&area),
                "{code:?} is in area {area}, which docs/20 does not declare"
            );
        }
    }

    #[test]
    fn every_code_this_crate_emits_is_in_the_registry() {
        // docs/20 is what the `docs` URL in every error body points at. A code
        // that is not there is a link to a 404 in the exact moment a user is
        // trying to understand a failure.
        //
        // There is deliberately no exception list. One existed while four codes
        // were drifting from the registry; D-055 corrected the codes instead,
        // and an exception list that outlives its exceptions is a gate with a
        // hole in it.
        //
        // This test was dropped once already, by a merge that resolved two
        // versions of the enclosing module by keeping matching lines. Losing it
        // is silent — the codes keep working, and only their documentation
        // links rot.
        let registry = include_str!("../../../docs/20-ERROR-CODE-REGISTRY.md");
        for code in codes::ALL {
            assert!(
                registry.contains(code.as_str()),
                "{code:?} is emitted by this crate and absent from docs/20"
            );
        }
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
