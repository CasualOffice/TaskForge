//! The chain of custody, end to end (`docs/45`).
//!
//! # What is worth asserting here
//!
//! Not that a row is written — that is the easy half. The properties below are
//! the ones the lifecycle depends on and that a plausible implementation gets
//! wrong:
//!
//! - a transfer **clears the assignees**, because the task has to land in the
//!   receiving team's queue rather than staying attached to someone who is done
//!   with it;
//! - a task cannot be handed to a team that is **not on its project**, because a
//!   task owned by people who cannot see it is a disappearance, not a hand-off;
//! - a promotion writes a **log row as well as the column**, or "when did this
//!   reach staging" is unanswerable and the column is the only fact left;
//! - the environment endpoint that predates all of this **also** logs, so the
//!   history is complete regardless of which door the task went through;
//! - a verdict is **not** a status change, so a task can fail twice on the same
//!   environment and both survive.

mod schema_harness;

use std::sync::Arc;

use anyhow::{Context as _, Result};
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

/// Everything the custody commands need between them.
const MEMBER: &[&str] = &[
    "project.create",
    "task.create",
    "task.read",
    "task.update",
    "task.assign",
    "task.transition",
    // Authoring an environment is `project.update` — the fixture creates the
    // pipeline these tests promote along, which is a project configuration
    // change and not a task one.
    "project.update",
];

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
    user: Uuid,
}

impl Caller {
    async fn get(&self, uri: &str) -> Result<(StatusCode, serde_json::Value)> {
        self.send(self.base("GET", uri).body(Body::empty())?).await
    }

    async fn send_json(
        &self,
        method: &str,
        uri: &str,
        body: &serde_json::Value,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.send(
            self.base(method, uri)
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", Uuid::now_v7().to_string())
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
    let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    let csrf = body["csrf_token"].as_str().context("csrf")?.to_owned();

    Ok(Caller {
        app,
        cookie,
        csrf,
        workspace,
        user,
    })
}

/// A project, a task in it, and two teams on the project.
struct Fixture {
    project: Uuid,
    task: Uuid,
    android: Uuid,
    backend: Uuid,
}

async fn fixture(pool: &sqlx::PgPool, caller: &Caller) -> Result<Fixture> {
    let (status, project) = caller
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;

    let (status, task) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{project_id}/tasks"),
            &json!({ "title": "Login crashes on rotate" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let task_id: Uuid = task["id"].as_str().context("task id")?.parse()?;

    let android = test_support::insert_team(pool, caller.workspace, "Android").await?;
    let backend = test_support::insert_team(pool, caller.workspace, "Backend").await?;
    for team in [android, backend] {
        test_support::add_project_team(pool, caller.workspace, project_id, team).await?;
    }

    Ok(Fixture {
        project: project_id,
        task: task_id,
        android,
        backend,
    })
}

/// An environment on the project, since only the API can make one.
async fn environment(caller: &Caller, project: Uuid, name: &str) -> Result<Uuid> {
    let (status, env) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{project}/environments"),
            &json!({ "name": name }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{env}");
    Ok(env["id"].as_str().context("environment id")?.parse()?)
}

#[path = "custody/part1.rs"]
mod part1;
