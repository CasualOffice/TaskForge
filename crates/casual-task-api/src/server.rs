//! The router, the middleware, and shutdown.
//!
//! # Health endpoints are two questions, not one (`docs/46`)
//!
//! `/health/live` answers "is this process wedged?" and touches nothing else.
//! A liveness probe that checks the database restarts every API instance during
//! a database outage — removing the only thing that could still serve cached
//! reads, and adding a thundering herd of reconnects to an already struggling
//! server.
//!
//! `/health/ready` answers "should traffic come here?" and *does* check the
//! database, because an instance that cannot reach it should leave the load
//! balancer rotation without dying.
//!
//! # Shutdown (D-041)
//!
//! `SIGTERM` stops accepting connections and lets in-flight requests finish,
//! bounded. Unbounded would mean one slow request holds the deploy open until
//! the orchestrator `SIGKILL`s the process mid-write.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{MatchedPath, State};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use casual_task_observability::labels::{LabelSet, keys};
use casual_task_observability::metrics::{HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_TOTAL};
use casual_task_observability::recorder::Recorder;
use sqlx::PgPool;

use crate::error::ApiError;

/// The header carrying a request id in and out.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// The request id, put into extensions so handlers and extractors can put the
/// **same** value in an error body that the response header carries.
///
/// It existed only as a header before, so every error body carried a hardcoded
/// literal — `"auth"`, `"login"` — while the header carried a real id. `docs/05`
/// promises "a `request_id` the user can quote to support"; two different values
/// for one request makes that promise false in the exact situation it is for.
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

impl RequestId {
    /// The id, or a marker when the observability layer did not run — which
    /// happens only in a test that builds a bare handler.
    #[must_use]
    pub fn of(parts: &axum::http::request::Parts) -> String {
        parts
            .extensions
            .get::<Self>()
            .map_or_else(|| "unknown".to_owned(), |id| id.0.clone())
    }

    /// The same, from headers alone — for a handler that took `HeaderMap`
    /// rather than `Parts`.
    #[must_use]
    pub fn of_parts(headers: &axum::http::HeaderMap) -> String {
        headers
            .get(REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty() && v.len() <= 128)
            .map_or_else(|| "unknown".to_owned(), ToOwned::to_owned)
    }

    /// The same, from a whole request.
    #[must_use]
    pub fn of_request(request: &Request<axum::body::Body>) -> String {
        request
            .extensions()
            .get::<Self>()
            .map_or_else(|| "unknown".to_owned(), |id| id.0.clone())
    }
}

/// Extracted so a handler can build an error body carrying the **same** id the
/// response header will.
///
/// Reading it from the request headers instead — as the login handler does —
/// finds an id only when the client happened to send one, because `observe`
/// puts the minted id in extensions and on the *response*. A handler that took
/// the header would therefore report `"unknown"` to every user who did not
/// already know their own request id.
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for RequestId {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<Self>()
            .cloned()
            .unwrap_or_else(|| Self("unknown".to_owned())))
    }
}

/// How long shutdown waits for in-flight requests.
///
/// Shorter than Kubernetes' default 30-second `SIGKILL` grace, because being
/// killed mid-request is what this exists to avoid.
pub const DRAIN: Duration = Duration::from_secs(20);

/// Shared, immutable per-request state.
#[derive(Clone)]
#[allow(missing_debug_implementations)]
pub struct AppState {
    pub pool: PgPool,
    /// Live-update fan-out (C-015). A trait object rather than the concrete hub
    /// so `docs/48`'s Redis implementation, when it exists, replaces it without
    /// touching a handler — and so a test can supply its own.
    pub broadcast: Arc<dyn casual_task_infra::broadcast::Broadcast>,
    pub metrics: Arc<Recorder>,
    /// `TF_SECRET_KEY`. Used for the CSRF binding and nothing else — ADR-032:
    /// "TF_SECRET_KEY is not a cookie signature."
    pub secret_key: Arc<str>,
    /// `TF_PUBLIC_URL`. `docs/48`: "used in emails and OIDC redirects". The
    /// reset link is built from it, so a deployment that sets it wrongly sends
    /// links to the wrong host rather than merely rendering one oddly.
    pub public_url: Arc<str>,
    /// Where attachment bytes live. `Arc<dyn ObjectStore>` for the reason
    /// `Mailer` is one: `TF_STORAGE_BACKEND` picks the backend once at startup
    /// and no handler branches on it again, so the filesystem profile runs the
    /// identical handshake S3 does (`docs/28` §Local deployment).
    pub storage: Arc<dyn casual_task_infra::ObjectStore>,
    /// Where outbound mail goes. `Arc<dyn Mailer>` and not an `Option`: an
    /// empty `TF_SMTP_HOST` selects the no-op implementation at startup
    /// (`docs/48`, D-046), so no handler has to ask whether email is on.
    pub mailer: Arc<dyn casual_task_infra::Mailer>,
}

include!("server_routes.rs");
include!("server_runtime.rs");
#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
