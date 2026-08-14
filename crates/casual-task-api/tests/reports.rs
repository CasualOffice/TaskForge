//! Running a report (`docs/38`, ADR-027).
//!
//! # What is worth asserting here
//!
//! Not that a count is a number. The properties below are the ones a report is
//! trusted for, and each is one a plausible implementation gets wrong:
//!
//! - it groups by the dimension asked for, and a **null group is a real
//!   answer** — untriaged work is the slice a lead is looking for, and a report
//!   that dropped it would hide exactly that;
//! - the **permission filter is the list query's**, so a viewer who cannot see
//!   a project does not see its tasks in a total — the failure here is silent
//!   and the number still looks plausible;
//! - a **measure that is designed but unbuilt is refused by name**, because
//!   answering a request for `p50 cycle_time` with a count gives someone a
//!   figure that is wrong in a way nothing on the page reveals;
//! - the **dimension set is closed**, so `group_by` can never become a SQL
//!   fragment.

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

const MEMBER: &[&str] = &["project.create", "task.create", "task.read", "task.update"];

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
}

impl Caller {
    async fn send_json(
        &self,
        method: &str,
        uri: &str,
        body: &serde_json::Value,
    ) -> Result<(StatusCode, serde_json::Value)> {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, &self.cookie)
            .header("x-csrf-token", &self.csrf)
            .header(WORKSPACE_HEADER, self.workspace.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .body(Body::from(body.to_string()))?;
        let response = self.app.clone().oneshot(request).await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
        let parsed = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        Ok((status, parsed))
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

async fn signed_in(
    pool: &sqlx::PgPool,
    email: &str,
    workspace: Uuid,
    grants: &[&str],
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
    test_support::grant_at_workspace(pool, workspace, user, grants).await?;

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
    })
}

/// The count for one group key, or 0 when the report did not name it.
fn total_for(body: &serde_json::Value, key: Option<&str>) -> i64 {
    body["groups"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .find(|group| group["key"].as_str() == key)
        .and_then(|group| group["total"].as_i64())
        .unwrap_or(0)
}

#[path = "reports/part1.rs"]
mod part1;
