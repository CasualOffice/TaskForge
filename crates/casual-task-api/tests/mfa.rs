//! MFA end to end, against a real PostgreSQL (C-001, `docs/40` §MFA).
//!
//! Four properties here cannot be observed from inside a handler, and each is
//! the kind that fails silently:
//!
//! - **An unconfirmed factor satisfies nothing.** A user who scanned the QR
//!   code and closed the tab must not be locked out by a factor they cannot
//!   produce codes for. Migration 0016's own comment names this failure.
//! - **A code cannot be replayed** (RFC 6238 §5.2). `Totp::verify` returns the
//!   matched step for exactly this reason; until now nothing used it.
//! - **Step-up happens at workspace resolution**, not at login, because the
//!   session is user-scoped and the policy is per workspace (`docs/40`
//!   §Workspace-level SSO and MFA step-up).
//! - **Break-glass works and is audited.** `docs/40` §Acceptance gates requires
//!   it, and this test runs the real binary rather than the functions behind
//!   it — a documented path that nobody executes is a path that has rotted.

mod schema_harness;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::server::{AppState, router};
use casual_task_identity::mfa::{STEP_SECONDS, Totp};
use casual_task_identity::password;
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use serde_json::{Value, json};
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";

struct Caller {
    user_id: Uuid,
    cookie: String,
    csrf: String,
}

fn app(pool: sqlx::PgPool) -> axum::Router {
    router(AppState {
        storage: std::sync::Arc::new(casual_task_infra::FilesystemStore::new(
            std::env::temp_dir().join("tf-test-objects"),
            "https://files.example.test".to_owned(),
            "test-object-signing-secret".to_owned(),
        )),
        pool,
        broadcast: casual_task_api::sse::local_hub(),
        metrics: Arc::new(Recorder::new()),
        secret_key: "a-test-secret-key-long-enough-for-hmac".into(),
        public_url: "https://tasks.example.test".into(),
        mailer: Arc::new(casual_task_infra::mail::LoggingMailer),
    })
}

async fn json_body(response: axum::response::Response) -> Result<Value> {
    let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Through casual-task-persistence, not raw SQL: `docs/19` puts every query in
/// that crate and `casual-task-lint` enforces it, **including in tests**.
async fn sign_up(app: &axum::Router, pool: &sqlx::PgPool, email: &str) -> Result<Caller> {
    let user_id = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        user_id,
        email,
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
    let (cookie, csrf) = login(app, email).await?;
    Ok(Caller {
        user_id,
        cookie,
        csrf,
    })
}

async fn login(app: &axum::Router, email: &str) -> Result<(String, String)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "email": email, "password": PASSWORD }).to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "login failed");
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with(SESSION_COOKIE))
        .and_then(|c| c.split(';').next())
        .context("session cookie")?
        .to_owned();
    let body = json_body(response).await?;
    let csrf = body["csrf_token"].as_str().context("csrf")?.to_owned();
    Ok((cookie, csrf))
}

fn request(caller: &Caller, method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, &caller.cookie)
        .header("x-csrf-token", &caller.csrf)
}

fn json_request(caller: &Caller, method: &str, uri: &str, body: &Value) -> Result<Request<Body>> {
    Ok(request(caller, method, uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))?)
}

/// Begin enrolment and return the factor, reconstructed from the secret the
/// endpoint handed back — which is how an authenticator app gets it.
async fn begin_enrolment(app: &axum::Router, caller: &Caller) -> Result<Totp> {
    let response = app
        .clone()
        .oneshot(request(caller, "POST", "/api/v1/auth/mfa/enrolment").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "enrolment did not begin");
    let body = json_body(response).await?;
    let secret = body["secret"].as_str().context("secret")?;
    assert!(
        body["provisioning_uri"]
            .as_str()
            .context("uri")?
            .starts_with("otpauth://totp/"),
        "no provisioning uri"
    );
    Ok(Totp::from_base32(secret).expect("valid base32"))
}

/// The code for the current step, exactly as a phone would show it.
fn code_now(totp: &Totp) -> String {
    code_at(
        totp,
        OffsetDateTime::now_utc().unix_timestamp() / STEP_SECONDS,
    )
}

/// The code for a given step, from the generator itself.
///
/// This used to brute-force the six-digit space against `verify`, to avoid a
/// second RFC 4226 implementation in the test. It was both — half a million
/// iterations averaged **seconds**, and the 30-second TOTP window moved on
/// before the request landed, so `confirm` intermittently returned 401 and the
/// test looked like a replay bug. `Totp::code_at` is public for exactly this.
fn code_at(totp: &Totp, step: i64) -> String {
    totp.code_at(step)
}

/// Enrol fully, returning the factor and the recovery codes.
async fn enrol(app: &axum::Router, caller: &Caller) -> Result<(Totp, Vec<String>)> {
    let totp = begin_enrolment(app, caller).await?;
    let response = app
        .clone()
        .oneshot(json_request(
            caller,
            "POST",
            "/api/v1/auth/mfa/enrolment/confirm",
            &json!({ "code": code_now(&totp) }),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "confirmation failed");
    let body = json_body(response).await?;
    let codes: Vec<String> = body["recovery_codes"]
        .as_array()
        .context("recovery codes")?
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_owned())
        .collect();
    Ok((totp, codes))
}

async fn create_workspace(app: &axum::Router, caller: &Caller, slug: &str) -> Result<Uuid> {
    let response = app
        .clone()
        .oneshot(json_request(
            caller,
            "POST",
            "/api/v1/workspaces",
            &json!({ "name": format!("Workspace {slug}"), "slug": slug }),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED, "create failed");
    let body = json_body(response).await?;
    Ok(body["id"].as_str().context("id")?.parse()?)
}

/// Any workspace-scoped read, used to observe whether resolution let us in.
async fn enter_workspace(
    app: &axum::Router,
    caller: &Caller,
    workspace: Uuid,
) -> Result<axum::response::Response> {
    Ok(app
        .clone()
        .oneshot(
            request(
                caller,
                "GET",
                &format!("/api/v1/workspaces/{workspace}/members"),
            )
            .body(Body::empty())?,
        )
        .await?)
}

#[path = "mfa/part1.rs"]
mod part1;
#[path = "mfa/part2.rs"]
mod part2;
