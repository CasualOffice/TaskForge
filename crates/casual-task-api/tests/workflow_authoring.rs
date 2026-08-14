//! Workflow authoring and environments, end to end (C-007).
//!
//! The status-delete path is why this file exists. `docs/23` §"Editing a
//! workflow" settles what the old drafts left open: a status holding tasks
//! **cannot** be deleted, the admin must supply a migration target, and then
//! every task moves in one transaction with an activity event attributed to
//! them. "Silently orphaning tasks, or lazily remapping them on next read, are
//! both rejected — they produce tasks whose history does not explain their
//! status."
//!
//! That is three assertions that only a database can make: the refusal, the
//! move, and the history. None of them is observable from inside a handler.
//!
//! The harness is `relations.rs`'s, for the same reason it borrowed
//! `comments.rs`'s: a real `role_assignment` row behind every permission, and
//! visibility resolved through the project.

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
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";
const SECRET: &str = "a-test-secret-key-long-enough-for-hmac";

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
}

impl Caller {
    async fn get(&self, uri: &str) -> Result<(StatusCode, serde_json::Value)> {
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
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.body_request("POST", uri, body, None).await
    }

    async fn patch(
        &self,
        uri: &str,
        body: &serde_json::Value,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.body_request("PATCH", uri, body, None).await
    }

    async fn put_at(
        &self,
        uri: &str,
        body: &serde_json::Value,
        version: i64,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.body_request("PUT", uri, body, Some(version)).await
    }

    /// The same, carrying the workflow's current version.
    ///
    /// Workflow authoring is guarded by optimistic concurrency: `docs/24` puts
    /// `If-Match` on every write to a versioned aggregate, and the endpoints
    /// answer `428` without one. Two admins editing one workflow is exactly the
    /// case it exists for.
    async fn post_at(
        &self,
        uri: &str,
        body: &serde_json::Value,
        version: i64,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.body_request("POST", uri, body, Some(version)).await
    }

    async fn patch_at(
        &self,
        uri: &str,
        body: &serde_json::Value,
        version: i64,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.body_request("PATCH", uri, body, Some(version)).await
    }

    async fn delete_at(&self, uri: &str, version: i64) -> Result<(StatusCode, serde_json::Value)> {
        self.send(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .header(header::COOKIE, &self.cookie)
                .header("x-csrf-token", &self.csrf)
                .header(header::IF_MATCH, format!("\"{version}\""))
                .header(WORKSPACE_HEADER, self.workspace.to_string())
                .body(Body::empty())?,
        )
        .await
    }

    async fn delete(&self, uri: &str) -> Result<(StatusCode, serde_json::Value)> {
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

    async fn body_request(
        &self,
        method: &str,
        uri: &str,
        body: &serde_json::Value,
        version: Option<i64>,
    ) -> Result<(StatusCode, serde_json::Value)> {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, &self.cookie)
            .header("x-csrf-token", &self.csrf)
            .header("idempotency-key", Uuid::now_v7().to_string())
            .header(WORKSPACE_HEADER, self.workspace.to_string());
        if let Some(version) = version {
            request = request.header(header::IF_MATCH, format!("\"{version}\""));
        }
        self.send(request.body(Body::from(body.to_string()))?).await
    }

    async fn send(&self, request: Request<Body>) -> Result<(StatusCode, serde_json::Value)> {
        let response = self.app.clone().oneshot(request).await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        Ok((status, body))
    }
}

fn state(pool: sqlx::PgPool) -> AppState {
    AppState {
        storage: Arc::new(casual_task_infra::FilesystemStore::new(
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
    }
}

/// The permissions this endpoint family needs.
///
/// `project.workflow.manage` is the authoring permission; the rest exist so the
/// test can put a task on a status and then watch it move.
const AUTHOR: &[&str] = &[
    "project.create",
    "project.update",
    "project.workflow.manage",
    "task.create",
    "task.read",
    "task.update",
    "task.history.read",
];

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

/// A grant is a real `role_assignment` row: migration 0003 says it is the only
/// source of authority, and a test that set a flag instead would prove the
/// handler works and nothing about whether the resolver is consulted.
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

/// The workflow's current version, for `If-Match`.
///
/// Read immediately before each write rather than cached: every authoring call
/// bumps it, so a test that reused one would be asserting the concurrency guard
/// rather than the behaviour it means to.
async fn version_of(caller: &Caller, workflow: &str) -> Result<i64> {
    let (status, body) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    Ok(body["version"].as_i64().expect("workflow version"))
}

/// A project, and the workflow it was created with.
async fn a_project(caller: &Caller, key_prefix: &str) -> Result<(String, String)> {
    let (status, project) = caller
        .post(
            "/api/v1/projects",
            &json!({ "name": format!("Project {key_prefix}"), "key": key_prefix }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    Ok((
        project["id"].as_str().expect("project id").to_owned(),
        project["workflow_id"]
            .as_str()
            .expect("workflow id")
            .to_owned(),
    ))
}

async fn a_task(caller: &Caller, project: &str, title: &str) -> Result<String> {
    let (status, task) = caller
        .post(
            &format!("/api/v1/projects/{project}/tasks"),
            &json!({ "title": title }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    Ok(task["id"].as_str().expect("task id").to_owned())
}

/// A task's version, for `If-Match`. Setting an environment writes the *task*,
/// which is a versioned aggregate; the environment itself is not.
async fn task_version(caller: &Caller, task: &str) -> Result<i64> {
    let (status, body) = caller.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    Ok(body["version"].as_i64().expect("task version"))
}

/// The status a task currently sits on, read back through the API.
async fn status_of(caller: &Caller, task: &str) -> Result<(String, String)> {
    let (status, body) = caller.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    Ok((
        body["status_id"].as_str().expect("status_id").to_owned(),
        body["state"].as_str().expect("state").to_owned(),
    ))
}

/// Move the initial flag off `status`, so a delete can reach its own rules.
///
/// A workflow must keep exactly one initial status (`TF-WFL-0007`), and that
/// check fires **before** the holds-tasks and wrong-workflow ones. A new task
/// lands on the initial status, so without this every delete test would assert
/// the initial-status rule instead of the rule it names.
async fn demote_initial(caller: &Caller, workflow: &str, keep_off: &str) -> Result<()> {
    let (_, view) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    let other = view["statuses"]
        .as_array()
        .expect("statuses")
        .iter()
        .find(|s| s["id"].as_str() != Some(keep_off))
        .expect("a second status")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let version = version_of(caller, workflow).await?;
    let (status, body) = caller
        .patch_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{other}"),
            &json!({ "is_initial": true }),
            version,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    Ok(())
}

fn status_named<'v>(workflow: &'v serde_json::Value, name: &str) -> &'v serde_json::Value {
    workflow["statuses"]
        .as_array()
        .expect("statuses")
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("no status named {name} in {workflow}"))
}

// ── Statuses ────────────────────────────────────────────────────────────────

// ── Deleting a status — the part docs/23 exists to settle ───────────────────

// ── Authority ───────────────────────────────────────────────────────────────

// ── Transitions ─────────────────────────────────────────────────────────────

// ── Environments ────────────────────────────────────────────────────────────

#[path = "workflow_authoring/part1.rs"]
mod part1;
