//! Workspaces, membership and teams, end to end (C-002).
//!
//! Everything here goes through `router()` rather than through a handler
//! directly, because half of what is being asserted lives in the layers: the
//! CSRF guard, the request id, and the workspace resolution that happens in the
//! extractor before a handler runs. A test that called a handler would prove
//! the handler works and nothing about whether the route is reachable the way a
//! client reaches it.

mod schema_harness;

use std::sync::Arc;

use anyhow::{Context, Result};
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
const SECRET: &str = "a-test-secret-key-long-enough-for-hmac";

/// A signed-in caller: everything a request needs to be accepted.
#[derive(Debug, Clone)]
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
        broadcast: casual_task_api::sse::local_hub(),
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: SECRET.into(),
        public_url: "https://tasks.example.test".into(),
        mailer: Arc::new(casual_task_infra::mail::LoggingMailer),
    })
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
    let csrf = body["csrf_token"]
        .as_str()
        .context("csrf token")?
        .to_owned();

    Ok(Caller {
        user_id,
        cookie,
        csrf,
    })
}

/// A request builder pre-loaded with the caller's credentials.
fn request(caller: &Caller, method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, &caller.cookie)
        .header("x-csrf-token", &caller.csrf)
}

async fn send(app: &axum::Router, request: Request<Body>) -> Result<axum::response::Response> {
    Ok(app.clone().oneshot(request).await?)
}

async fn json_body(response: axum::response::Response) -> Result<Value> {
    let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Create a workspace and return `(id, etag)`.
async fn create_workspace(
    app: &axum::Router,
    caller: &Caller,
    slug: &str,
) -> Result<(Uuid, String)> {
    let response = send(
        app,
        request(caller, "POST", "/api/v1/workspaces")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": format!("Workspace {slug}"), "slug": slug }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CREATED, "create failed");
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .context("no ETag on a created workspace")?
        .to_owned();
    let body = json_body(response).await?;
    let id = body["id"].as_str().context("id")?.parse()?;
    Ok((id, etag))
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// D-054 — a workspace acquires an owner when it is created
// ---------------------------------------------------------------------------

#[path = "workspaces/part1.rs"]
mod part1;
#[path = "workspaces/part2.rs"]
mod part2;
#[path = "workspaces/part3.rs"]
mod part3;
