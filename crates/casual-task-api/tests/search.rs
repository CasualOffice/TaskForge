//! Full-text search and the filter grammar, end to end (C-013).
//!
//! The test that matters most here is the permission one. `docs/26`
//! §Acceptance gates asks for it in exactly these words — "search never returns
//! a task from an inaccessible project, **including for tasks whose text
//! matches strongly**" — because the classic failure is to search first and
//! filter afterwards, which collapses page sizes, breaks cursors, and leaks the
//! existence of matching work.
//!
//! The projection is populated through `test_support::index_task` rather than
//! by running a dispatch loop: the subject of these tests is the query path.
//! The consumer that keeps the projection current has its own test in
//! `casual-task-worker`.

mod schema_harness;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::middleware::WORKSPACE_HEADER;
use casual_task_api::server::{AppState, router};
use casual_task_identity::password;
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";
const SECRET: &str = "a-test-secret-key-long-enough-for-hmac";

const MEMBER: &[&str] = &["project.create", "task.create", "task.read", "task.update"];

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
    user: Uuid,
    pool: sqlx::PgPool,
}

impl Caller {
    async fn get(&self, uri: &str) -> Result<(StatusCode, serde_json::Value)> {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::COOKIE, &self.cookie)
                    .header(WORKSPACE_HEADER, self.workspace.to_string())
                    .body(Body::empty())?,
            )
            .await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        Ok((status, body))
    }

    async fn post(&self, uri: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::COOKIE, &self.cookie)
                    .header("x-csrf-token", &self.csrf)
                    .header(WORKSPACE_HEADER, self.workspace.to_string())
                    .header("idempotency-key", Uuid::now_v7().to_string())
                    .body(Body::from(body.to_string()))?,
            )
            .await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        anyhow::ensure!(status == StatusCode::CREATED, "create failed: {value}");
        Ok(value)
    }

    /// Create a task and put it in the search projection, as the worker would.
    async fn indexed_task(&self, project: Uuid, title: &str, description: &str) -> Result<Uuid> {
        let body = self
            .post(
                &format!("/api/v1/projects/{project}/tasks"),
                &serde_json::json!({ "title": title, "description": description }),
            )
            .await?;
        let id: Uuid = body["id"].as_str().context("task id")?.parse()?;
        anyhow::ensure!(
            test_support::index_task(&self.pool, self.workspace, id).await?,
            "the task was not indexed"
        );
        Ok(id)
    }

    async fn project(&self, key: &str, visibility: &str) -> Result<Uuid> {
        let body = self
            .post(
                "/api/v1/projects",
                &serde_json::json!({ "key": key, "name": key, "visibility": visibility }),
            )
            .await?;
        Ok(body["id"].as_str().context("project id")?.parse()?)
    }
}

fn state(pool: sqlx::PgPool) -> AppState {
    AppState {
        storage: std::sync::Arc::new(casual_task_infra::FilesystemStore::new(
            std::env::temp_dir().join("tf-test-objects"),
            "https://files.example.test".to_owned(),
            "test-object-signing-secret".to_owned(),
        )),
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: SECRET.into(),
        broadcast: casual_task_api::sse::local_hub(),
        public_url: "https://tasks.example.test".into(),
        mailer: Arc::new(casual_task_infra::mail::LoggingMailer),
    }
}

async fn signed_in(
    pool: &sqlx::PgPool,
    email: &str,
    workspace: Uuid,
    permissions: &[&str],
) -> Result<Caller> {
    let user = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        user,
        email,
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
    test_support::add_workspace_member(pool, workspace, user).await?;
    if !permissions.is_empty() {
        test_support::grant_at_workspace(pool, workspace, user, permissions).await?;
    }

    let app = router(state(pool.clone()));
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
    anyhow::ensure!(response.status() == StatusCode::OK, "login failed");
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with(SESSION_COOKIE))
        .and_then(|c| c.split(';').next())
        .context("session cookie")?
        .to_owned();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    let csrf = body["csrf_token"]
        .as_str()
        .context("csrf token")?
        .to_owned();

    Ok(Caller {
        app,
        cookie,
        csrf,
        workspace,
        user,
        pool: pool.clone(),
    })
}

async fn fresh(pool: &sqlx::PgPool, email: &str, slug: &str) -> Result<Caller> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    signed_in(pool, email, workspace, MEMBER).await
}

/// The task ids a response returned, in order.
fn ids(body: &serde_json::Value) -> Vec<String> {
    body["data"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["id"].as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------

#[path = "search/part1.rs"]
mod part1;
