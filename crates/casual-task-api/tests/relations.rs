//! Task activity and dependencies, end to end (C-011, C-008).
//!
//! The cycle tests are the reason this file exists. A reachability check is
//! easy to write and easy to write *wrongly* — off by one hop, or checking the
//! edge being inserted rather than the one that would close the loop — and both
//! mistakes produce a graph that looks correct until something walks it.
//!
//! The harness is `comments.rs`'s, because these endpoints share its shape:
//! visibility resolved through the task, 404 for anything invisible, and a real
//! `role_assignment` row behind every permission.

mod schema_harness;

use std::sync::Arc;

use anyhow::Result;
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

/// Everything a request needs to be accepted: a signed-in user, a workspace
/// they belong to, and a CSRF token bound to the session.
struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
}

impl Caller {
    async fn get(&self, uri: &str) -> Result<(StatusCode, serde_json::Value, Option<String>)> {
        self.send(
            Request::builder()
                .uri(uri)
                .header(header::COOKIE, &self.cookie)
                .header(WORKSPACE_HEADER, self.workspace.to_string())
                .body(Body::empty())?,
        )
        .await
    }

    async fn post(
        &self,
        uri: &str,
        body: &serde_json::Value,
        idempotency_key: Option<&str>,
    ) -> Result<(StatusCode, serde_json::Value, Option<String>)> {
        let mut request = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, &self.cookie)
            .header("x-csrf-token", &self.csrf)
            .header(WORKSPACE_HEADER, self.workspace.to_string());
        if let Some(key) = idempotency_key {
            request = request.header("idempotency-key", key);
        }
        self.send(request.body(Body::from(body.to_string()))?).await
    }

    async fn delete(&self, uri: &str) -> Result<(StatusCode, serde_json::Value, Option<String>)> {
        self.send(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .header(header::COOKIE, &self.cookie)
                .header("x-csrf-token", &self.csrf)
                .header(WORKSPACE_HEADER, self.workspace.to_string())
                .body(Body::empty())?,
        )
        .await
    }

    async fn patch(
        &self,
        uri: &str,
        body: &serde_json::Value,
        if_match: Option<&str>,
    ) -> Result<(StatusCode, serde_json::Value, Option<String>)> {
        let mut request = Request::builder()
            .method("PATCH")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, &self.cookie)
            .header("x-csrf-token", &self.csrf)
            .header(WORKSPACE_HEADER, self.workspace.to_string());
        if let Some(tag) = if_match {
            request = request.header(header::IF_MATCH, tag);
        }
        self.send(request.body(Body::from(body.to_string()))?).await
    }

    async fn send(
        &self,
        request: Request<Body>,
    ) -> Result<(StatusCode, serde_json::Value, Option<String>)> {
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

/// A signed-in member of a fresh workspace, holding `permissions`.
async fn caller(
    pool: &sqlx::PgPool,
    email: &str,
    slug: &str,
    permissions: &[&str],
) -> Result<Caller> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    member_of(pool, email, workspace, permissions).await
}

/// A signed-in member of an **existing** workspace, holding `permissions`.
///
/// The grant is a real `role_assignment` row, not a flag: migration 0003 says
/// "role_assignment is the ONLY source of authority in the system", and a test
/// that bypassed it would prove the handler works and nothing about whether the
/// resolver is consulted.
async fn member_of(
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
    let csrf = body["csrf_token"].as_str().expect("csrf token").to_owned();

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

/// The permissions this endpoint family needs.
const RELATER: &[&str] = &[
    "project.create",
    "task.create",
    "task.read",
    "task.update",
    "task.history.read",
];

/// A project and a task in it, returning both ids.
async fn a_task(caller: &Caller) -> Result<(String, String)> {
    let (status, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().expect("id").to_owned();

    let (status, task, _) = caller
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "Ship the thing" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    Ok((project_id, task["id"].as_str().expect("id").to_owned()))
}

/// A second task in the same project, so a dependency has two ends.
async fn another_task(caller: &Caller, project_id: &str, title: &str) -> Result<String> {
    let (status, task, _) = caller
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": title }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    Ok(task["id"].as_str().expect("id").to_owned())
}

// ---------------------------------------------------------------------------
// Activity — C-011
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Dependencies — C-008
// ---------------------------------------------------------------------------

// ── Removing a dependency, which used to be impossible ──────────────────────

#[path = "relations/part1.rs"]
mod part1;
#[path = "relations/part2.rs"]
mod part2;
