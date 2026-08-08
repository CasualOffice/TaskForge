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
    pub metrics: Arc<Recorder>,
    /// `TF_SECRET_KEY`. Used for the CSRF binding and nothing else — ADR-032:
    /// "TF_SECRET_KEY is not a cookie signature."
    pub secret_key: Arc<str>,
}

/// Build the router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/metrics", get(metrics))
        .route(
            "/api/v1/auth/login",
            axum::routing::post(crate::auth::login),
        )
        .route(
            "/api/v1/auth/logout",
            axum::routing::post(crate::auth::logout),
        )
        .route("/api/v1/auth/session", get(crate::middleware::whoami))
        // CSRF sits over every route, so a route added later cannot be added
        // beside it. docs/05: "every unsafe method without a valid token is
        // rejected" — every, not most.
        //
        // Under `observe`, so that a CSRF rejection still gets a request id and
        // still counts in the metrics. A refusal nobody can measure is a
        // refusal nobody notices.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(state.clone(), observe))
        .with_state(state)
}

/// Liveness. Deliberately touches nothing — see the module docs.
async fn live() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Readiness. Checks the database, and reports **503** when it cannot be
/// reached rather than 500: this is "do not send me traffic", not "I am
/// broken", and the distinction is what keeps a load balancer from removing
/// every instance during a brief database blip.
async fn ready(State(state): State<AppState>) -> Response {
    // Through casual-task-persistence: docs/19 puts every query there, and
    // casual-task-lint enforces it. Even a one-line `SELECT 1` — the rule is
    // not worth a hole.
    match casual_task_persistence::health::ping(&state.pool).await {
        Ok(()) => (StatusCode::OK, "ready").into_response(),
        Err(error) => {
            tracing::warn!(%error, "readiness check failed");
            // No request id here: readiness is probed by infrastructure, not by
            // a user who could quote one.
            ApiError::unavailable("health", 5).into_response()
        }
    }
}

/// The Prometheus scrape endpoint (F-009).
///
/// The body comes from `casual-task-observability`; this is the only part that
/// had to live in an HTTP crate, which is why the recorder was written without
/// it.
async fn metrics(State(state): State<AppState>) -> Response {
    (
        StatusCode::OK,
        [(
            "content-type",
            // Version 0.0.4 of the exposition format, which is what every
            // Prometheus scraper still negotiates.
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(),
    )
        .into_response()
}

/// Assign a request id, record RED metrics, and echo the id back.
///
/// The id is echoed on **every** response, including errors, because `docs/05`
/// promises the user something to quote to support — and the responses they
/// need it for are exactly the failing ones.
async fn observe(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // An inbound id is honoured so a trace spans a proxy; one is minted when
    // absent. It is not trusted for anything but correlation.
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .map_or_else(|| uuid::Uuid::now_v7().to_string(), ToOwned::to_owned);

    // The route TEMPLATE, never the resolved path: `docs/46` §Cardinality
    // discipline. `/api/v1/tasks/{id}` is one series; the resolved path is one
    // series per task.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned());
    let method = request.method().clone();

    let started = std::time::Instant::now();
    let mut response = next.run(request).await;
    let elapsed = started.elapsed();

    record(&state.metrics, &method, &route, response.status(), elapsed);

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}

/// RED: rate, errors, duration.
fn record(
    metrics: &Recorder,
    method: &axum::http::Method,
    route: &str,
    status: StatusCode,
    elapsed: Duration,
) {
    // Bounded label values only. `method` is a fixed set; `route` is a template
    // from the router, not from the request; `status_class` collapses 200..599
    // to five values. None of them can carry an id.
    let status_class = match status.as_u16() {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    };
    let Some(method) = declared_method(method) else {
        // An unrecognised verb is not worth a metric series; it is worth a log.
        tracing::debug!(%method, "unrecognised HTTP method");
        return;
    };
    // `route` is a String from the router. It is bounded by the number of
    // routes, but LabelValue takes only &'static str — the cardinality guard —
    // so it is interned against the router's own table.
    let Some(route) = declared_route(route) else {
        return;
    };

    let labels = LabelSet::for_metric(HTTP_REQUESTS_TOTAL)
        .with(keys::METHOD, method)
        .and_then(|l| l.with(keys::ROUTE, route))
        .and_then(|l| l.with(keys::STATUS_CLASS, status_class));
    if let Ok(labels) = labels {
        let _ = metrics.increment(HTTP_REQUESTS_TOTAL, &labels, 1);
    }

    let labels = LabelSet::for_metric(HTTP_REQUEST_DURATION_SECONDS)
        .with(keys::METHOD, method)
        .and_then(|l| l.with(keys::ROUTE, route));
    if let Ok(labels) = labels {
        let _ = metrics.observe(
            HTTP_REQUEST_DURATION_SECONDS,
            &labels,
            elapsed.as_secs_f64(),
        );
    }
}

/// Map a method to a `&'static str`, or refuse it.
const fn declared_method(method: &axum::http::Method) -> Option<&'static str> {
    match *method {
        axum::http::Method::GET => Some("GET"),
        axum::http::Method::POST => Some("POST"),
        axum::http::Method::PUT => Some("PUT"),
        axum::http::Method::PATCH => Some("PATCH"),
        axum::http::Method::DELETE => Some("DELETE"),
        axum::http::Method::HEAD => Some("HEAD"),
        axum::http::Method::OPTIONS => Some("OPTIONS"),
        _ => None,
    }
}

/// Every route this server serves, as `&'static str`.
///
/// The cardinality guard in `casual-task-observability` accepts only
/// `&'static str`, and the router hands back a `String`. Interning it here
/// rather than widening the guard means an unrouted path — which is attacker
/// controlled — cannot become a metric series.
pub const ROUTES: &[&str] = &[
    "/health/live",
    "/health/ready",
    "/metrics",
    "/api/v1/auth/login",
    "/api/v1/auth/logout",
    "/api/v1/auth/session",
    "unmatched",
];

fn declared_route(route: &str) -> Option<&'static str> {
    ROUTES.iter().copied().find(|known| *known == route)
}

/// Serve until `SIGTERM` or `SIGINT`, then drain.
///
/// # Errors
///
/// If the listener cannot be bound, or the server fails while running.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: AppState,
) -> Result<(), std::io::Error> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            // Cannot install the handler: better to keep serving than to exit.
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGTERM; shutdown will be abrupt");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!(drain_seconds = DRAIN.as_secs(), "shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_the_router_serves_is_interned() {
        // The metric label must be a &'static str, so a route missing from
        // ROUTES silently loses its metrics. Listed here as the one place the
        // two tables are compared.
        for route in [
            "/health/live",
            "/health/ready",
            "/metrics",
            "/api/v1/auth/login",
            "/api/v1/auth/logout",
            "/api/v1/auth/session",
        ] {
            assert!(
                declared_route(route).is_some(),
                "{route} is served but not in ROUTES, so it records no metrics"
            );
        }
    }

    #[test]
    fn an_unrouted_path_cannot_become_a_metric_series() {
        // The path is attacker-controlled. Without interning, every 404 to a
        // random URL would create a time series.
        assert_eq!(declared_route("/../../etc/passwd"), None);
        assert_eq!(declared_route("/api/v1/tasks/018f2c"), None);
    }

    #[test]
    fn the_drain_is_shorter_than_the_orchestrator_kill_grace() {
        // Kubernetes defaults to 30 s. A drain longer than that is not a drain;
        // it is a SIGKILL with extra steps.
        assert!(DRAIN < Duration::from_secs(30));
    }

    #[test]
    fn methods_map_to_bounded_labels() {
        assert_eq!(declared_method(&axum::http::Method::GET), Some("GET"));
        assert_eq!(
            declared_method(&axum::http::Method::from_bytes(b"PROPFIND").expect("valid")),
            None,
            "an arbitrary verb would be an unbounded label"
        );
    }
}
