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

    fn body(&self) -> Body<'_> {
        Body {
            code: self.code.as_str(),
            message: &self.message,
            details: &self.details,
            request_id: &self.request_id,
            docs: format!("https://docs.taskforge.dev/errors/{}", self.code.as_str()),
        }
    }

    /// This error as the object a normal response would carry under `error`.
    ///
    /// `207 Multi-Status` reports many independent outcomes in one body, so a
    /// failure there cannot become the response — but a client should not need
    /// a second renderer for it. It is the same object, built from the same
    /// place, so the two can never drift.
    ///
    /// # Panics
    ///
    /// Never: the shape is fixed and `details` is already a `Value`.
    #[must_use]
    pub fn envelope(&self) -> serde_json::Value {
        serde_json::to_value(self.body()).expect("the error body is always serialisable")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(Envelope { error: self.body() });

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
