//! Role authoring and grant creation, end to end (C-003, `docs/04`).
//!
//! `docs/04` §Acceptance gates asks for an **escalation suite** — "one test per
//! control above, each *attempting* the exploit and asserting rejection". That
//! is what most of this file is. A test that only proves the happy path proves
//! the endpoint exists; these prove it cannot be turned into a way to become an
//! owner.
//!
//! The controls, and where each is enforced:
//!
//! | Control | Enforced by | Tested here |
//! | --- | --- | --- |
//! | 1 — grant ceiling | `casual_task_authz::ceiling` | yes, on assign *and* on role edit |
//! | 2 — scope ceiling | same | yes |
//! | 3 — authoring is workspace-scoped | same | yes |
//! | 4 — last owner | migration 0021's trigger | yes |
//! | 5 — self-elevation | `casual_task_authz::ceiling` | yes |
//! | 6 — plugin ceiling | nothing yet | **no — see below** |
//! | 7 — everything audited | `UnitOfWork::record` | yes |
//!
//! Control 6 is the one row here that is not covered, and it is listed rather
//! than skipped: a table that counts 1, 2, 3, 4, 5, 7 tells a reader nothing
//! about whether 6 was forgotten. It cannot be tested yet — a plugin's ceiling
//! is the intersection of its granted scopes with the installing admin's
//! permissions, and there are no plugins to install until P-001. The test lands
//! with the registry, not before it.

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
    user: Uuid,
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
        self.with_body("POST", uri, body, None).await
    }

    async fn patch(
        &self,
        uri: &str,
        body: &serde_json::Value,
        version: i64,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.with_body("PATCH", uri, body, Some(version)).await
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

    async fn with_body(
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
        user,
    })
}

/// Everything an admin needs to author and grant, and the workspace to do it in.
const ADMIN: &[&str] = &[
    "role.manage",
    "role.assign",
    "task.read",
    "task.create",
    "task.update",
    "project.create",
];

async fn admin(pool: &sqlx::PgPool, slug: &str) -> Result<Caller> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    member_of(pool, "admin@example.com", workspace, ADMIN).await
}

// ── The happy path, so the refusals mean something ──────────────────────────

// ── The escalation suite ────────────────────────────────────────────────────

// ── Reading the grant set, which is what makes revoking possible ────────────

#[path = "roles/part1.rs"]
mod part1;
#[path = "roles/part2.rs"]
mod part2;
