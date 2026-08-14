//! `/api/v1/permissions/*` end to end, against a real PostgreSQL (C-003).
//!
//! `docs/04` calls `/permissions/explain` the answer to "why can't I close
//! this?", and the properties that make that answer worth having cannot be
//! observed from inside a handler:
//!
//! - **A constrained permission is reported, not dropped.** The effective set
//!   distinguishes "you may always" from "you may where the constraint holds",
//!   so the client neither renders a button that 403s nor hides a feature the
//!   actor has.
//! - **A grant can contribute and still not allow.** That pair — named grant,
//!   unsatisfied constraint — is the entire product of the endpoint.
//! - **It is not a permission oracle.** Explaining somebody else's authority
//!   discloses their grants, so it costs `role.manage`. Without that, a member
//!   could enumerate which colleague holds `workspace.delete`.
//! - **A subject from another workspace does not resolve.** Row-level security
//!   confines the reads, and the endpoint turns that into a 404 rather than an
//!   empty answer that reads like "they have nothing".

mod schema_harness;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::middleware::WORKSPACE_HEADER;
use casual_task_api::server::{AppState, router};
use casual_task_identity::password;
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use serde_json::{Value, json};
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
        storage: Arc::new(casual_task_infra::FilesystemStore::new(
            std::env::temp_dir().join("tf-test-objects"),
            "https://files.example.test".to_owned(),
            "test-object-signing-secret".to_owned(),
        )),
        broadcast: casual_task_api::sse::local_hub(),
        pool,
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

async fn sign_up(app: &axum::Router, pool: &sqlx::PgPool, email: &str) -> Result<Caller> {
    let user_id = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        user_id,
        email,
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
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
    Ok(Caller {
        user_id,
        cookie,
        csrf,
    })
}

fn request(
    caller: &Caller,
    workspace: Uuid,
    method: &str,
    uri: &str,
) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, &caller.cookie)
        .header("x-csrf-token", &caller.csrf)
        .header(WORKSPACE_HEADER, workspace.to_string())
}

/// A workspace the caller belongs to, created directly rather than through the
/// API: `POST /workspaces` grants the creator Owner (D-054), which holds every
/// permission and would make every assertion here trivially true.
async fn workspace_with(pool: &sqlx::PgPool, user: Uuid, slug: &str) -> Result<Uuid> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    test_support::add_workspace_member(pool, workspace, user).await?;
    Ok(workspace)
}

async fn effective(app: &axum::Router, caller: &Caller, workspace: Uuid) -> Result<Value> {
    let response = app
        .clone()
        .oneshot(
            request(caller, workspace, "GET", "/api/v1/permissions/effective")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "effective failed");
    json_body(response).await
}

async fn explain(
    app: &axum::Router,
    caller: &Caller,
    workspace: Uuid,
    body: Value,
) -> Result<axum::response::Response> {
    Ok(app
        .clone()
        .oneshot(
            request(caller, workspace, "POST", "/api/v1/permissions/explain")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))?,
        )
        .await?)
}

fn reach_of<'v>(body: &'v Value, permission: &str) -> Option<&'v str> {
    body["permissions"]
        .as_array()?
        .iter()
        .find(|p| p["permission"] == permission)?["reach"]
        .as_str()
}

#[path = "permissions/part1.rs"]
mod part1;
