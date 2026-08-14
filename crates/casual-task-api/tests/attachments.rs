//! The attachment pipeline, end to end (C-010, `docs/28`).
//!
//! Four of `docs/28` §Acceptance gates are here, and they are the four that can
//! be asserted without a scanner or a 2 GB file:
//!
//! - **Type confusion** — HTML uploaded as `image/png` is rejected at commit.
//! - **Invisibility** — an uncommitted attachment is absent from every read
//!   path.
//! - **Cross-tenant** — a pre-signed URL for workspace A cannot be minted or
//!   used by a member of workspace B.
//! - **Download before scan** — the fail-closed default (**D-062**).
//!
//! The upload itself is written straight to the object root, because the
//! browser's `PUT` goes to the storage origin and never through this API — which
//! is the property `docs/28` opens with, and simulating it any other way would
//! be testing a path the product does not have.

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
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";
const SECRET: &str = "a-test-secret-key-long-enough-for-hmac";
/// SHA-256 of the empty string — the checksum column is validated for *shape*
/// here, not recomputed; the commit step compares size, and the checksum
/// comparison needs the whole object (`docs/28` step 3) which this profile's
/// backend does not stream in a test.
const SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

const MEMBER: &[&str] = &[
    "project.create",
    "task.create",
    "task.read",
    "task.attachment.create",
    "task.attachment.read",
];

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
    root: std::path::PathBuf,
}

impl Caller {
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

    fn base(&self, method: &str, uri: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, &self.cookie)
            .header("x-csrf-token", &self.csrf)
            .header(WORKSPACE_HEADER, self.workspace.to_string())
    }

    async fn get(&self, uri: &str) -> Result<(StatusCode, serde_json::Value)> {
        self.send(self.base("GET", uri).body(Body::empty())?).await
    }

    /// A `GET` that keeps the response instead of parsing it — for the redirect.
    async fn raw_get(&self, uri: &str) -> Result<axum::response::Response> {
        Ok(self
            .app
            .clone()
            .oneshot(self.base("GET", uri).body(Body::empty())?)
            .await?)
    }

    async fn post(
        &self,
        uri: &str,
        body: &serde_json::Value,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.send(
            self.base("POST", uri)
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", Uuid::now_v7().to_string())
                .body(Body::from(body.to_string()))?,
        )
        .await
    }

    /// What the browser does in step 2: write the bytes to the object store.
    ///
    /// Direct to the root, because the upload does not pass through the API —
    /// that is the whole design (`docs/28`).
    async fn upload(
        &self,
        workspace: Uuid,
        task: Uuid,
        attachment: Uuid,
        bytes: &[u8],
    ) -> Result<()> {
        let dir = self.root.join(workspace.to_string()).join(task.to_string());
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(attachment.to_string()), bytes)?;
        Ok(())
    }
}

fn state(pool: sqlx::PgPool, root: &std::path::Path) -> AppState {
    AppState {
        pool,
        storage: Arc::new(casual_task_infra::FilesystemStore::new(
            root.to_path_buf(),
            "https://files.example.test".to_owned(),
            SECRET.to_owned(),
        )),
        broadcast: casual_task_api::sse::local_hub(),
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

    let root = std::env::temp_dir().join(format!("tf-objects-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&root)?;
    let app = router(state(pool.clone(), &root));

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
        root,
    })
}

async fn fresh(pool: &sqlx::PgPool, email: &str, slug: &str) -> Result<Caller> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    signed_in(pool, email, workspace, MEMBER).await
}

/// A project and a task, returning the task id.
async fn a_task(caller: &Caller, key: &str) -> Result<Uuid> {
    let (status, project) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": key, "name": key, "visibility": "WORKSPACE" }),
        )
        .await?;
    anyhow::ensure!(status == StatusCode::CREATED, "project: {project}");
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;

    let (status, task) = caller
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "Has files" }),
        )
        .await?;
    anyhow::ensure!(status == StatusCode::CREATED, "task: {task}");
    Ok(task["id"].as_str().context("task id")?.parse()?)
}

/// Step 1: ask for permission to upload. Returns the attachment id.
async fn presign(
    caller: &Caller,
    task: Uuid,
    filename: &str,
    declared: &str,
    size: i64,
) -> Result<Uuid> {
    let (status, body) = caller
        .post(
            &format!("/api/v1/tasks/{task}/attachments"),
            &serde_json::json!({
                "filename": filename,
                "content_type": declared,
                "byte_size": size,
                "checksum": SHA,
            }),
        )
        .await?;
    anyhow::ensure!(status == StatusCode::CREATED, "presign: {body}");
    Ok(body["attachment_id"].as_str().context("id")?.parse()?)
}

// ---------------------------------------------------------------------------

#[path = "attachments/part1.rs"]
mod part1;
