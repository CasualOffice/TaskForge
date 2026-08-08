//! Login end to end, against a real PostgreSQL (C-001, `docs/40`).
//!
//! The properties asserted here cannot be checked anywhere else:
//!
//! - **Enumeration.** `docs/40` §Acceptance gates: login responses are
//!   indistinguishable for existing and non-existing accounts "in body, status,
//!   and timing envelope". Body and status are compared byte for byte; the
//!   timing envelope is checked as an order-of-magnitude property, because a
//!   tight bound on a shared CI runner is a flaky test rather than a stronger
//!   one.
//! - **Revocation is immediate.** The reason `docs/40` rejects JWTs. A logged-out
//!   session must be dead on the next request, not at expiry.
//! - **The cookie flags.** `HttpOnly` on the session cookie is what keeps an XSS
//!   bug short of account takeover, and it is one word from being absent.

mod schema_harness;

use std::sync::Arc;

use anyhow::Result;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::server::{AppState, router};
use casual_task_identity::password;
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";

/// Through casual-task-persistence, not raw SQL: docs/19 puts every query in
/// that crate and casual-task-lint enforces it, including in tests.
async fn seed_user(pool: &sqlx::PgPool, email: &str) -> Result<Uuid> {
    let id = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        id,
        email,
        &password::hash(PASSWORD).expect("hashes"),
    )
    .await?;
    Ok(id)
}

fn app(pool: sqlx::PgPool) -> axum::Router {
    router(AppState {
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: "a-test-secret-key-long-enough-for-hmac".into(),
    })
}

fn login_request(email: &str, password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "email": email, "password": password }).to_string(),
        ))
        .expect("request")
}

async fn parts(response: axum::response::Response) -> (StatusCode, Vec<String>, String) {
    let status = response.status();
    let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(ToOwned::to_owned))
        .collect();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    (
        status,
        cookies,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_correct_password_creates_a_session() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    seed_user(&db.pool, "user@example.com").await?;

    let (status, cookies, body) = parts(
        app(db.pool.clone())
            .oneshot(login_request("user@example.com", PASSWORD))
            .await?,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let session = cookies
        .iter()
        .find(|c| c.starts_with(SESSION_COOKIE))
        .expect("no session cookie");

    // docs/40 §Browser sessions, flag for flag. HttpOnly is the one that keeps
    // an XSS bug short of account takeover.
    assert!(session.contains("HttpOnly"), "{session}");
    assert!(session.contains("Secure"), "{session}");
    assert!(session.contains("SameSite=Lax"), "{session}");
    assert!(session.contains("Path=/"), "{session}");

    // The CSRF cookie is deliberately NOT HttpOnly — the client must read it to
    // echo it back — and that is only safe while the session cookie is.
    let csrf = cookies
        .iter()
        .find(|c| c.starts_with("tf_csrf"))
        .expect("no csrf cookie");
    assert!(!csrf.contains("HttpOnly"), "{csrf}");

    assert_eq!(test_support::live_session_count(&db.pool).await?, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unknown_account_and_a_wrong_password_are_indistinguishable() -> Result<()> {
    // docs/40 §Acceptance gates, the enumeration test. Body and status compared
    // exactly: any difference at all is the oracle.
    let db = schema_harness::TestDatabase::start().await?;
    seed_user(&db.pool, "real@example.com").await?;

    let (unknown_status, unknown_cookies, unknown_body) = parts(
        app(db.pool.clone())
            .oneshot(login_request("nobody@example.com", PASSWORD))
            .await?,
    )
    .await;
    let (wrong_status, wrong_cookies, wrong_body) = parts(
        app(db.pool.clone())
            .oneshot(login_request("real@example.com", "the wrong password"))
            .await?,
    )
    .await;

    assert_eq!(unknown_status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        unknown_status, wrong_status,
        "an unknown account and a wrong password return different statuses"
    );
    assert_eq!(
        unknown_body, wrong_body,
        "the two failures have different bodies, which is the enumeration oracle"
    );
    assert!(
        unknown_cookies.is_empty() && wrong_cookies.is_empty(),
        "a failed login set a cookie"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_failed_login_costs_the_same_order_of_magnitude_either_way() -> Result<()> {
    // The timing half of the enumeration gate. Asserted as an order of
    // magnitude, not a tight bound: a shared CI runner cannot support a tight
    // one, and a flaky security test gets deleted.
    //
    // Without the dummy verification the unknown-account path returns in
    // microseconds against ~100 ms for a real one — a ratio of thousands, which
    // this catches easily.
    let db = schema_harness::TestDatabase::start().await?;
    seed_user(&db.pool, "real@example.com").await?;

    // One warm-up each: the lazily initialised dummy hash and the connection
    // pool both cost more on first use, and neither is what is being measured.
    let _ = app(db.pool.clone())
        .oneshot(login_request("nobody@example.com", PASSWORD))
        .await?;
    let _ = app(db.pool.clone())
        .oneshot(login_request("real@example.com", "wrong"))
        .await?;

    let unknown = std::time::Instant::now();
    let _ = app(db.pool.clone())
        .oneshot(login_request("nobody@example.com", PASSWORD))
        .await?;
    let unknown = unknown.elapsed();

    let wrong = std::time::Instant::now();
    let _ = app(db.pool.clone())
        .oneshot(login_request("real@example.com", "the wrong password"))
        .await?;
    let wrong = wrong.elapsed();

    let ratio = unknown.as_secs_f64().max(wrong.as_secs_f64())
        / unknown
            .as_secs_f64()
            .min(wrong.as_secs_f64())
            .max(f64::EPSILON);
    assert!(
        ratio < 10.0,
        "unknown account took {unknown:?}, wrong password took {wrong:?} — a ratio of {ratio:.1}. \
         The two paths must do the same work, or the endpoint enumerates accounts."
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn logging_out_revokes_the_session_immediately() -> Result<()> {
    // The reason docs/40 rejects JWTs. Not "at expiry" — now.
    let db = schema_harness::TestDatabase::start().await?;
    seed_user(&db.pool, "user@example.com").await?;

    let (_, cookies, _) = parts(
        app(db.pool.clone())
            .oneshot(login_request("user@example.com", PASSWORD))
            .await?,
    )
    .await;
    let session_cookie = cookies
        .iter()
        .find(|c| c.starts_with(SESSION_COOKIE))
        .and_then(|c| c.split(';').next())
        .expect("session cookie")
        .to_owned();
    // The CSRF token, because logout is a state-changing method and the guard
    // added with the auth middleware rejects one without it. This test caught
    // that the moment the layer landed, which is the layer working.
    let csrf_token = cookies
        .iter()
        .find(|c| c.starts_with("tf_csrf"))
        .and_then(|c| c.split(';').next())
        .and_then(|c| c.split_once('='))
        .map(|(_, value)| value.to_owned())
        .expect("csrf cookie");

    let response = app(db.pool.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header(header::COOKIE, &session_cookie)
                .header("x-csrf-token", &csrf_token)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        test_support::live_session_count(&db.pool).await?,
        0,
        "the session survived logout"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn logging_out_without_a_session_is_not_an_error() -> Result<()> {
    // Reporting one would tell a caller whether a stolen cookie was still live.
    let db = schema_harness::TestDatabase::start().await?;
    let response = app(db.pool.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn repeated_failures_back_off_without_locking_the_account_forever() -> Result<()> {
    // docs/40 §Acceptance gates: "brute force triggers exponential backoff
    // without locking a legitimate user out permanently."
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed_user(&db.pool, "user@example.com").await?;

    for _ in 0..5 {
        let _ = app(db.pool.clone())
            .oneshot(login_request("user@example.com", "wrong"))
            .await?;
    }

    let state = test_support::lockout_state(&db.pool, user).await?;
    assert!(state.locked, "five failures did not produce any backoff");

    // FEWER than five, and that is the point. Once the backoff starts, further
    // attempts are refused WITHOUT counting — otherwise anyone could hold a
    // victim's account locked indefinitely by guessing at it forever, which is
    // the denial of service docs/40 §Acceptance gates rules out ("without
    // locking a legitimate user out permanently"). The counter advances only on
    // attempts the server actually evaluated.
    assert!(
        state.failed_attempts < 5,
        "attempts made during a backoff window still counted ({}), so an attacker can \
         extend a stranger's lockout at will",
        state.failed_attempts
    );
    assert!(
        state.failed_attempts >= 4,
        "the backoff started too late ({})",
        state.failed_attempts
    );

    // The lock expires on its own — it is a timestamp, never a flag. A boolean
    // would be a denial of service anyone could trigger against a stranger.
    assert!(
        !state.locked_beyond_an_hour,
        "the account is locked for more than an hour"
    );

    // And clearing the backoff lets the real password through again.
    test_support::clear_lockout(&db.pool, user).await?;
    let (status, _, _) = parts(
        app(db.pool.clone())
            .oneshot(login_request("user@example.com", PASSWORD))
            .await?,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a legitimate user could not get back in"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unknown_field_in_the_login_body_is_rejected() -> Result<()> {
    // docs/05 §Conventions: unknown request fields are rejected with 400 —
    // "silently ignoring a typo'd field is how clients ship bugs that look like
    // server bugs".
    let db = schema_harness::TestDatabase::start().await?;
    let response = app(db.pool.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "email": "user@example.com",
                        "password": "x",
                        "remember_me": true
                    })
                    .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    Ok(())
}
