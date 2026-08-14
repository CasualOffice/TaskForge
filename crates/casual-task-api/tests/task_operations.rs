//! Update, delete, transitions, assignees and tags, end to end (C-008).
//!
//! The transition tests are the ones that matter most, and they are written
//! against `docs/23` §Validation order rather than against the implementation:
//! the order is a *specification*, the first failure is the one a user sees, and
//! a handler that ran the same checks in a different sequence would report a
//! misleading error while passing every test that only asserted "it refused".
//!
//! Every test here fails without the code it covers. The history assertions
//! count rows rather than asserting a `200`, because a transition that returned
//! `200` and wrote no audit record would pass a status-code test and violate
//! ADR-006.

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

/// What a task operation needs to be permitted at all.
const MEMBER: &[&str] = &[
    "project.create",
    "task.create",
    "task.read",
    "task.update",
    "task.transition",
    "task.assign",
];

/// A member who may make the exceptional dependency-bypass decision.
const OVERRIDER: &[&str] = &[
    "project.create",
    "task.create",
    "task.read",
    "task.update",
    "task.transition",
    "task.assign",
    "task.dependency.override",
];

type Answer = (StatusCode, serde_json::Value, Option<String>);

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
    user: Uuid,
}

impl Caller {
    async fn get(&self, uri: &str) -> Result<Answer> {
        self.send(
            self.base("GET", uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::empty())?,
        )
        .await
    }

    async fn post(&self, uri: &str, body: &serde_json::Value, key: Option<&str>) -> Result<Answer> {
        let mut request = self
            .base("POST", uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        self.send(request.body(Body::from(body.to_string()))?).await
    }

    /// A `POST` carrying `If-Match` — what a transition sends.
    async fn post_conditional(
        &self,
        uri: &str,
        body: &serde_json::Value,
        if_match: Option<&str>,
    ) -> Result<Answer> {
        let mut request = self
            .base("POST", uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(tag) = if_match {
            request = request.header(header::IF_MATCH, tag);
        }
        self.send(request.body(Body::from(body.to_string()))?).await
    }

    async fn patch(
        &self,
        uri: &str,
        body: &serde_json::Value,
        if_match: Option<&str>,
    ) -> Result<Answer> {
        let mut request = self
            .base("PATCH", uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(tag) = if_match {
            request = request.header(header::IF_MATCH, tag);
        }
        self.send(request.body(Body::from(body.to_string()))?).await
    }

    async fn delete(&self, uri: &str, if_match: Option<&str>) -> Result<Answer> {
        let mut request = self.base("DELETE", uri);
        if let Some(tag) = if_match {
            request = request.header(header::IF_MATCH, tag);
        }
        self.send(request.body(Body::empty())?).await
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

async fn signed_in(
    pool: &sqlx::PgPool,
    email: &str,
    slug: &str,
    permissions: &[&str],
) -> Result<Caller> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    member_of(pool, email, workspace, permissions).await
}

/// A signed-in member of an existing workspace holding `permissions`.
///
/// The grant is a real `role_assignment` row: migration 0003 says
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
    })
}

fn key() -> String {
    Uuid::now_v7().to_string()
}

/// A project and one task in it. Returns `(project_id, task_id, task etag)`.
///
/// The project is **`WORKSPACE`-visible on purpose**. The default is `TEAM`
/// (migration 0004) and these projects have no team, so under the default only
/// the creator could see them — through the `project_membership` row a create
/// writes for itself. Several tests below need a *second* member of the same
/// workspace to reach the task, and a fixture that quietly made them all
/// invisible would turn every one of those into a `404` that looks like the
/// refusal being asserted.
async fn a_task(caller: &Caller) -> Result<(Uuid, Uuid, String)> {
    let (status, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "project create: {project}");
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;

    let (status, task, etag) = caller
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "Ship it" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "task create: {task}");
    let task_id: Uuid = task["id"].as_str().context("task id")?.parse()?;
    Ok((project_id, task_id, etag.context("task etag")?))
}

/// The default workflow's status ids, by name.
async fn statuses(
    pool: &sqlx::PgPool,
    workspace: Uuid,
) -> Result<std::collections::HashMap<String, Uuid>> {
    Ok(test_support::default_status_ids(pool, workspace)
        .await?
        .into_iter()
        .collect())
}

// ---------------------------------------------------------------------------
// PATCH
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// DELETE
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Transitions — docs/23
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Assignees and tags
// ---------------------------------------------------------------------------

#[path = "task_operations/part1.rs"]
mod part1;
#[path = "task_operations/part2.rs"]
mod part2;
