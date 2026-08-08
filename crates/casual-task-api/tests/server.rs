//! The server, exercised through a real router.
//!
//! These are not unit tests of handlers. What is worth asserting here is a
//! *request reaching a response*: the request-id echo, the health split, the
//! metrics body, and the cardinality guard holding on a path an attacker
//! controls. None of that is visible from inside a handler.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use casual_task_api::server::{AppState, REQUEST_ID_HEADER, router};
use casual_task_observability::recorder::Recorder;
use tower::ServiceExt;

/// A router whose pool points nowhere.
///
/// Deliberate: it makes `/health/ready` fail, which is the interesting case —
/// a readiness endpoint that only works when everything works has never been
/// observed doing its job.
fn unreachable_database() -> (AppState, axum::Router) {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy("postgres://nobody:nobody@127.0.0.1:1/nothing")
        .expect("a lazy pool never connects at construction");
    let state = AppState {
        pool,
        metrics: Arc::new(Recorder::new()),
    };
    (state.clone(), router(state))
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

#[tokio::test]
async fn liveness_does_not_touch_the_database() {
    // The whole point. If liveness checked the database, a database outage
    // would restart every API instance — removing the only thing that could
    // still serve anything and adding a reconnect storm to a struggling server.
    let (_, app) = unreachable_database();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "liveness failed with an unreachable database"
    );
}

#[tokio::test]
async fn readiness_reports_503_when_the_database_is_unreachable() {
    // 503, not 500: "do not send me traffic", not "I am broken". A 500 here
    // reads as a bug in the instance rather than a dependency being down.
    let (_, app) = unreachable_database();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("5"),
        "docs/05 requires Retry-After on every 503"
    );

    let body: serde_json::Value =
        serde_json::from_str(&body_string(response).await).expect("json envelope");
    assert_eq!(body["error"]["code"], "TF-SRV-0003");
}

#[tokio::test]
async fn every_response_carries_a_request_id() {
    // docs/05 promises the user something to quote to support, and the
    // responses they need it for are the failing ones — so it is asserted on
    // both a success and a failure.
    for (uri, expected) in [
        ("/health/live", StatusCode::OK),
        ("/health/ready", StatusCode::SERVICE_UNAVAILABLE),
        ("/nothing-here", StatusCode::NOT_FOUND),
    ] {
        let (_, app) = unreachable_database();
        let response = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), expected, "{uri}");
        let id = response
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(!id.is_empty(), "{uri} returned no request id");
    }
}

#[tokio::test]
async fn an_inbound_request_id_is_echoed_so_a_trace_spans_a_proxy() {
    let (_, app) = unreachable_database();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .header(REQUEST_ID_HEADER, "from-the-proxy")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(
        response
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok()),
        Some("from-the-proxy")
    );
}

#[tokio::test]
async fn an_absurd_inbound_request_id_is_replaced_rather_than_echoed() {
    // The header is attacker-controlled and ends up in logs. An unbounded one
    // is a log-flooding primitive.
    let (_, app) = unreachable_database();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .header(REQUEST_ID_HEADER, "x".repeat(4096))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    let echoed = response
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(echoed.len() <= 128, "echoed {} bytes", echoed.len());
}

#[tokio::test]
async fn metrics_are_served_in_the_prometheus_exposition_format() {
    // F-009's serving half. The recorder produced the body from the first day;
    // this is the endpoint that had to wait for an HTTP crate to exist.
    let (state, app) = unreachable_database();

    // One request, so there is something to report.
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/plain; version=0.0.4; charset=utf-8")
    );

    let body = body_string(response).await;
    assert!(
        body.contains("# TYPE http_requests_total counter"),
        "no request counter in the scrape:\n{body}"
    );
    assert!(
        body.contains(r#"route="/health/live""#),
        "the route template is missing:\n{body}"
    );
    drop(state);
}

#[tokio::test]
async fn an_unrouted_path_creates_no_metric_series() {
    // docs/46 §Cardinality discipline. The path is attacker-controlled: without
    // interning against the router's own table, every 404 to a random URL would
    // permanently add a time series.
    let (_, app) = unreachable_database();

    for path in ["/../../etc/passwd", "/api/v1/tasks/018f2c", "/wp-admin"] {
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let body = body_string(response).await;

    for path in ["etc/passwd", "018f2c", "wp-admin"] {
        assert!(
            !body.contains(path),
            "{path} reached the metrics body as a label:\n{body}"
        );
    }
}
