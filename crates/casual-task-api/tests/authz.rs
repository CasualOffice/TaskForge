//! Authentication, workspace resolution and CSRF, end to end.
//!
//! Everything here is a refusal. That is the point: the interesting behaviour
//! of an auth layer is what it *stops*, and a test suite that only proves the
//! happy path proves the layer exists rather than that it works.

mod schema_harness;

use std::sync::Arc;

use anyhow::Result;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{Request, StatusCode, header};
use axum::response::IntoResponse;
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::middleware::{WORKSPACE_HEADER, WorkspaceMember};
use casual_task_api::server::{AppState, router};
use casual_task_identity::password;
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";
const SECRET: &str = "a-test-secret-key-long-enough-for-hmac";

fn state(pool: sqlx::PgPool) -> AppState {
    AppState {
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
        mailer: std::sync::Arc::new(casual_task_infra::mail::LoggingMailer),
    }
}

/// A router with one extra route that requires workspace membership.
///
/// Added here rather than in the server, because no product endpoint exists yet
/// and inventing one to make a test pass would put an endpoint in the API that
/// `docs/05` does not describe. The extractor is public, so exercising it
/// through a route is exactly how a real handler will use it.
fn app_with_protected_route(pool: sqlx::PgPool) -> axum::Router {
    let state = state(pool);
    router(state.clone()).route(
        "/test/workspace",
        axum::routing::get(async |State(_): State<AppState>, member: WorkspaceMember| {
            (
                StatusCode::OK,
                member.context.scope().id().as_uuid().to_string(),
            )
                .into_response()
        })
        .with_state(state),
    )
}

async fn seed(pool: &sqlx::PgPool, email: &str) -> Result<Uuid> {
    let id = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        id,
        email,
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
    Ok(id)
}

/// Log in and return `(session cookie, csrf token)`.
async fn login(app: &axum::Router, email: &str) -> Result<(String, String)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "email": email, "password": PASSWORD }).to_string(),
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
        .expect("session cookie")
        .to_owned();

    let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    let token = body["csrf_token"].as_str().expect("csrf token").to_owned();
    Ok((cookie, token))
}

#[path = "authz/part1.rs"]
mod part1;
#[path = "authz/part2.rs"]
mod part2;
