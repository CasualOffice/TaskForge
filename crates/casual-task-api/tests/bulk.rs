//! `POST /api/v1/tasks/bulk`, end to end (C-008, `docs/05` §Bulk operations).
//!
//! Every test here is about the *seam*, not about the transition rules —
//! `task_operations.rs` owns those, and the handler runs the same code, so
//! re-asserting them here would only prove the call reaches it.
//!
//! What is actually at risk in a bulk endpoint is the isolation: that a refused
//! task leaves the others committed, that the refusal it reports is its own,
//! and that a client can undo what happened when only some of it happened. So
//! the assertions are made against the *database* after the call, not against
//! the status line — a handler that reported `207` with the right counts and
//! rolled everything back would pass a status-code test and be useless.

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
const BULK: &str = "/api/v1/tasks/bulk";

const MEMBER: &[&str] = &[
    "project.create",
    "task.create",
    "task.read",
    "task.update",
    "task.transition",
];

type Answer = (StatusCode, serde_json::Value, Option<String>);

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
}

impl Caller {
    async fn post(&self, uri: &str, body: &serde_json::Value, key: Option<&str>) -> Result<Answer> {
        let mut request = self
            .base("POST", uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        self.send(request.body(Body::from(body.to_string()))?).await
    }

    async fn post_conditional(
        &self,
        uri: &str,
        body: &serde_json::Value,
        if_match: &str,
    ) -> Result<Answer> {
        self.send(
            self.base("POST", uri)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::IF_MATCH, if_match)
                .body(Body::from(body.to_string()))?,
        )
        .await
    }

    fn base(&self, method: &str, uri: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, &self.cookie)
            .header("x-csrf-token", &self.csrf)
            .header(WORKSPACE_HEADER, self.workspace.to_string())
    }

    async fn send(&self, request: Request<Body>) -> Result<Answer> {
        let response = self.app.clone().oneshot(request).await?;
        let status = response.status();
        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        Ok((status, body, etag))
    }
}

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

async fn signed_in(pool: &sqlx::PgPool, email: &str, slug: &str) -> Result<Caller> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    let user = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        user,
        email,
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
    test_support::add_workspace_member(pool, workspace, user).await?;
    test_support::grant_at_workspace(pool, workspace, user, MEMBER).await?;

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
    })
}

fn key() -> String {
    Uuid::now_v7().to_string()
}

/// A `WORKSPACE`-visible project — the default is `TEAM`, and these projects
/// have no team, which would make every task invisible to its own fixture.
async fn a_project(caller: &Caller) -> Result<Uuid> {
    let (status, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "project create: {project}");
    Ok(project["id"].as_str().context("project id")?.parse()?)
}

/// `n` fresh tasks, each on the initial status. Returns `(id, version)`.
async fn tasks(caller: &Caller, project: Uuid, n: usize) -> Result<Vec<(Uuid, i64)>> {
    let mut made = Vec::with_capacity(n);
    for i in 0..n {
        let (status, task, _) = caller
            .post(
                &format!("/api/v1/projects/{project}/tasks"),
                &serde_json::json!({ "title": format!("Task {i}") }),
                Some(&key()),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "task create: {task}");
        made.push((
            task["id"].as_str().context("task id")?.parse()?,
            task["version"].as_i64().context("version")?,
        ));
    }
    Ok(made)
}

async fn statuses(
    pool: &sqlx::PgPool,
    workspace: Uuid,
) -> Result<std::collections::HashMap<String, Uuid>> {
    Ok(test_support::default_status_ids(pool, workspace)
        .await?
        .into_iter()
        .collect())
}

/// The result for one task, by id — the response is ordered, but an assertion
/// that depended on the order would fail confusingly when only the order broke.
fn result_for(body: &serde_json::Value, id: Uuid) -> &serde_json::Value {
    body["results"]
        .as_array()
        .expect("results")
        .iter()
        .find(|r| r["task_id"] == id.to_string())
        .unwrap_or_else(|| panic!("no result for {id} in {body}"))
}

// ---------------------------------------------------------------------------
// Partial success — the whole point of the endpoint
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The envelope — refused whole, because the client could have known
// ---------------------------------------------------------------------------

#[path = "bulk/part1.rs"]
mod part1;
