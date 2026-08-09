//! Rate limiting, exercised through a real router (`docs/21` §Rate limits).
//!
//! What is worth asserting here and cannot be seen from inside the limiter: that
//! the layer is actually *wired*, that it wraps the route it is supposed to and
//! not the ones it must never touch, and that a refusal comes back as the 429
//! `docs/05` describes rather than as some other error.
//!
//! **No database.** The pool points at a closed port, so a login that gets past
//! the limiter fails fast with a 503. That is the point: these tests are about
//! which requests reach the handler at all, and "not 429" is the assertion. A
//! Docker harness here would make the suite slower without making it stricter.
//!
//! The **window reset** is asserted in the unit tests in `rate_limit.rs`
//! instead, where the clock is a parameter. Waiting out a real six-second window
//! in an HTTP test would add six seconds to every run of the suite, and a test
//! that slow is a test that ends up `#[ignore]`d and then unrun.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use casual_task_api::rate_limit::AUTH;
use casual_task_api::server::{AppState, router};
use casual_task_observability::recorder::Recorder;
use tower::ServiceExt;

/// A router whose pool points nowhere, and the state behind it.
///
/// Each call builds a **new** router, and therefore a new limiter: the limiter's
/// state is owned by the router, so one test cannot spend another's tokens.
fn app() -> (AppState, axum::Router) {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_millis(50))
        .connect_lazy("postgres://nobody:nobody@127.0.0.1:1/nothing")
        .expect("a lazy pool never connects at construction");
    let state = AppState {
        storage: std::sync::Arc::new(casual_task_infra::FilesystemStore::new(
            std::env::temp_dir().join("tf-test-objects"),
            "https://files.example.test".to_owned(),
            "test-object-signing-secret".to_owned(),
        )),
        broadcast: casual_task_api::sse::local_hub(),
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: "test-key-long-enough-for-the-config-check".into(),
        public_url: "https://tasks.example.test".into(),
        mailer: std::sync::Arc::new(casual_task_infra::mail::LoggingMailer),
    };
    (state.clone(), router(state))
}

fn login_from(ip: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .uri("/api/v1/auth/login")
        .method("POST")
        .header("content-type", "application/json");
    if let Some(ip) = ip {
        builder = builder.header("x-forwarded-for", ip);
    }
    builder
        .body(Body::from(
            serde_json::json!({"email": "user@example.test", "password": "not-the-password"})
                .to_string(),
        ))
        .expect("request")
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

async fn scrape(app: &axum::Router) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    body_string(response).await
}

#[tokio::test]
async fn a_burst_from_one_address_is_refused_with_retry_after() {
    // The defect this whole change exists for: before it, this loop could run
    // forever. Every request reached the password check, so an attacker's only
    // cost was bandwidth.
    let (_, app) = app();

    for attempt in 1..=AUTH.burst {
        let response = app
            .clone()
            .oneshot(login_from(Some("203.0.113.9")))
            .await
            .expect("response");
        assert_ne!(
            response.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "attempt {attempt} of a burst of {} was refused; docs/21 allows the burst",
            AUTH.burst
        );
    }

    let refused = app
        .clone()
        .oneshot(login_from(Some("203.0.113.9")))
        .await
        .expect("response");
    assert_eq!(
        refused.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the {}th attempt from one address was admitted: login is unlimited",
        AUTH.burst + 1
    );
    // docs/05: "429 | rate limited (Retry-After always present)".
    assert_eq!(
        refused
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("6"),
        "a 429 without Retry-After tells a client to retry immediately"
    );

    let body: serde_json::Value =
        serde_json::from_str(&body_string(refused).await).expect("json envelope");
    assert_eq!(
        body["error"]["code"], "TF-LIM-0001",
        "docs/20 registers TF-LIM-0001 for a rate limit"
    );
}

#[tokio::test]
async fn a_second_address_is_unaffected_by_the_first() {
    // The property that separates a rate limit from an outage. A limiter keyed
    // on something shared would let one attacker deny login to everyone — the
    // failure docs/21 names for a per-account-only limit, in the other
    // direction.
    let (_, app) = app();

    for _ in 0..AUTH.burst + 2 {
        let _ = app
            .clone()
            .oneshot(login_from(Some("203.0.113.9")))
            .await
            .expect("response");
    }
    let attacker = app
        .clone()
        .oneshot(login_from(Some("203.0.113.9")))
        .await
        .expect("response");
    assert_eq!(attacker.status(), StatusCode::TOO_MANY_REQUESTS);

    let ordinary = app
        .clone()
        .oneshot(login_from(Some("198.51.100.4")))
        .await
        .expect("response");
    assert_ne!(
        ordinary.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "a different address was refused because the first exhausted its bucket; \
         one attacker can lock every user out of logging in"
    );
}

#[tokio::test]
async fn health_endpoints_are_never_rate_limited() {
    // An orchestrator probes these every second or two, from one address —
    // exactly the shape the limiter refuses on login. A 429 on a liveness probe
    // is a container restart, so a limiter that covered these would turn a
    // defence into an outage during the incident it was meant to help with.
    let (_, app) = app();

    for probe in 1..=50 {
        let response = app
            .clone()
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
            "liveness probe {probe} did not return 200"
        );
        assert!(
            response.headers().get("ratelimit-limit").is_none(),
            "an ungoverned route is advertising a rate limit it is not subject to"
        );
    }

    for probe in 1..=20 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "readiness probe {probe} returned something other than the expected 503"
        );
    }
}

#[tokio::test]
async fn a_refusal_increments_rate_limit_hits_total() {
    // docs/46 declares `rate_limit_hits_total` and, until this change, nothing
    // ever wrote to it. A limiter whose refusals are invisible cannot answer the
    // question an operator asks during an attack: is this throttling working,
    // and on whom?
    let (_, app) = app();

    assert!(
        !scrape(&app).await.contains("rate_limit_hits_total"),
        "the counter reported a hit before anything was refused"
    );

    for _ in 0..AUTH.burst + 3 {
        let _ = app
            .clone()
            .oneshot(login_from(Some("203.0.113.9")))
            .await
            .expect("response");
    }

    let body = scrape(&app).await;
    assert!(
        body.contains("# TYPE rate_limit_hits_total counter"),
        "the metric is not in the scrape at all:\n{body}"
    );
    // Three refusals: burst + 3 requests, of which `burst` were admitted.
    assert!(
        body.contains(r#"rate_limit_hits_total{scope_kind="ip"} 3"#),
        "the refusals were not counted under scope_kind=ip:\n{body}"
    );
    // docs/46 §Cardinality discipline — the label must say what kind of scope,
    // never which client.
    assert!(
        !body.contains("203.0.113.9"),
        "the client address reached a metric label:\n{body}"
    );
}

#[tokio::test]
async fn rate_limit_headers_are_returned_on_a_success_too() {
    // docs/05 §Rate limiting: "Returned on success too, so a client can slow
    // down *before* being throttled." Headers only on the 429 would tell a
    // client it is in trouble exactly one request too late.
    let (_, app) = app();

    let response = app
        .clone()
        .oneshot(login_from(Some("203.0.113.9")))
        .await
        .expect("response");
    assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned)
    };
    assert_eq!(
        header("ratelimit-limit"),
        Some(AUTH.sustained_per_minute.to_string()),
        "docs/05 shows RateLimit-Limit on every response"
    );
    assert_eq!(
        header("ratelimit-remaining"),
        Some((AUTH.burst - 1).to_string()),
        "the first request of a burst did not report the rest of it as remaining"
    );
    assert_eq!(
        header("ratelimit-reset"),
        Some("6".to_owned()),
        "one spent token refills in one emission interval"
    );
}

#[tokio::test]
async fn a_request_with_no_forwarded_address_is_still_limited() {
    // The bypass an attacker would try first. Keying on a header means asking
    // what happens when the header is absent, and the answer must not be
    // "unlimited".
    let (_, app) = app();

    for _ in 0..AUTH.burst {
        let response = app
            .clone()
            .oneshot(login_from(None))
            .await
            .expect("response");
        assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
    let refused = app
        .clone()
        .oneshot(login_from(None))
        .await
        .expect("response");
    assert_eq!(
        refused.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "omitting X-Forwarded-For escaped the limiter entirely"
    );
}
