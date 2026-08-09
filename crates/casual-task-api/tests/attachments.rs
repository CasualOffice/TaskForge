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

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn html_uploaded_as_a_png_is_rejected_at_commit() -> Result<()> {
    // docs/28 §Acceptance gates, the type-confusion test. This is the
    // stored-XSS vector the whole sniffing step exists for.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    let html = b"<html><script>alert(document.cookie)</script></html>";
    let id = presign(
        &caller,
        task,
        "innocent.png",
        "image/png",
        html.len() as i64,
    )
    .await?;
    caller.upload(caller.workspace, task, id, html).await?;

    let (status, body) = caller
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "markup was accepted: {body}"
    );
    assert_eq!(body["error"]["code"], "TF-ATT-0002");

    // The object is gone: leaving it would leave a reachable file no row
    // explains.
    let path = caller
        .root
        .join(caller.workspace.to_string())
        .join(task.to_string())
        .join(id.to_string());
    assert!(!path.exists(), "the refused object was left on disk");

    // And it never became visible.
    let (_, listed) = caller
        .get(&format!("/api/v1/tasks/{task}/attachments"))
        .await?;
    assert_eq!(listed["data"].as_array().map(Vec::len), Some(0), "{listed}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_declared_type_that_contradicts_the_bytes_is_rejected() -> Result<()> {
    // The other half of docs/28 §Validation: not markup, but still a lie. A PDF
    // declared as a PNG is refused, because the declaration is what pinned the
    // upload policy.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    let pdf = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n";
    let id = presign(&caller, task, "shot.png", "image/png", pdf.len() as i64).await?;
    caller.upload(caller.workspace, task, id, pdf).await?;

    let (status, body) = caller
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-ATT-0003");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_uncommitted_attachment_is_absent_from_every_read_path() -> Result<()> {
    // docs/28 §The invariant and its acceptance gate. The row exists from
    // pre-sign; nothing may see it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    let id = presign(&caller, task, "chart.png", "image/png", png.len() as i64).await?;

    // The row is there.
    assert!(
        test_support::attachment_exists(&db.pool, id).await?,
        "the pre-sign did not reserve a row"
    );
    // And it is in no read path.
    let (_, listed) = caller
        .get(&format!("/api/v1/tasks/{task}/attachments"))
        .await?;
    assert_eq!(listed["data"].as_array().map(Vec::len), Some(0), "{listed}");

    let (status, _) = caller
        .get(&format!("/api/v1/attachments/{id}/download"))
        .await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an uncommitted file was reachable"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_committed_file_is_not_downloadable_until_a_scan_clears_it() -> Result<()> {
    // D-062, the fail-closed default. Commit verifies; it does not make the file
    // available. Without a scanner the attachment stays PENDING forever, and
    // PENDING is a 409 rather than a 404 so the uploader is told to wait.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRxxxx";
    let id = presign(&caller, task, "chart.png", "image/png", png.len() as i64).await?;
    caller.upload(caller.workspace, task, id, png).await?;

    let (status, body) = caller
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["scan_status"], "PENDING");
    // The stored type came from the bytes, not the declaration.
    assert_eq!(body["content_type"], "image/png");

    // Still not downloadable, and still not listed.
    let (status, refused) = caller
        .get(&format!("/api/v1/attachments/{id}/download"))
        .await?;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an unscanned file was served: {refused}"
    );
    assert_eq!(refused["error"]["code"], "TF-ATT-0007");

    let (_, listed) = caller
        .get(&format!("/api/v1/tasks/{task}/attachments"))
        .await?;
    assert_eq!(listed["data"].as_array().map(Vec::len), Some(0), "{listed}");

    // A CLEAN verdict is what makes it visible — the transition only
    // `mark_scanned` can perform.
    test_support::set_scan_verdict(&db.pool, caller.workspace, id, "CLEAN").await?;

    let (_, listed) = caller
        .get(&format!("/api/v1/tasks/{task}/attachments"))
        .await?;
    assert_eq!(listed["data"][0]["id"], id.to_string(), "{listed}");

    let response = caller
        .raw_get(&format!("/api/v1/attachments/{id}/download"))
        .await?;
    assert_eq!(response.status(), StatusCode::FOUND);
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .context("no redirect")?;
    // docs/28: the separate origin is "the single most important control here".
    assert!(
        location.starts_with("https://files.example.test/"),
        "the download was served from the application origin: {location}"
    );
    assert!(location.contains("signature="), "{location}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_infected_file_is_never_served() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;
    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRxxxx";
    let id = presign(&caller, task, "chart.png", "image/png", png.len() as i64).await?;
    caller.upload(caller.workspace, task, id, png).await?;
    caller
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;

    for (verdict, code) in [("INFECTED", "TF-ATT-0006"), ("FAILED", "TF-ATT-0010")] {
        test_support::set_scan_verdict(&db.pool, caller.workspace, id, verdict).await?;
        let (status, body) = caller
            .get(&format!("/api/v1/attachments/{id}/download"))
            .await?;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{verdict} was served: {body}"
        );
        assert_eq!(body["error"]["code"], code);
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_attachment_in_another_workspace_is_404_and_never_403() -> Result<()> {
    // docs/28 §Acceptance gates, the cross-tenant test: a pre-signed URL for
    // workspace A cannot be minted or used by a member of workspace B.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = fresh(&db.pool, "owner@example.test", "acme").await?;
    let stranger = fresh(&db.pool, "stranger@example.test", "other").await?;
    let task = a_task(&owner, "WR").await?;

    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRxxxx";
    let id = presign(&owner, task, "chart.png", "image/png", png.len() as i64).await?;
    owner.upload(owner.workspace, task, id, png).await?;
    owner
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    test_support::set_scan_verdict(&db.pool, owner.workspace, id, "CLEAN").await?;

    // The owner can reach it, so the fixture is real.
    let response = owner
        .raw_get(&format!("/api/v1/attachments/{id}/download"))
        .await?;
    assert_eq!(response.status(), StatusCode::FOUND);

    // A member of another workspace cannot — and cannot tell it exists.
    for uri in [
        format!("/api/v1/attachments/{id}/download"),
        format!("/api/v1/attachments/{}/download", Uuid::now_v7()),
    ] {
        let (status, body) = stranger.get(&uri).await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
    }

    // Nor can they mint an upload against the other tenant's task.
    let (status, body) = stranger
        .post(
            &format!("/api/v1/tasks/{task}/attachments"),
            &serde_json::json!({
                "filename": "x.png", "content_type": "image/png",
                "byte_size": 10, "checksum": SHA,
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_traversing_filename_cannot_reach_the_object_key() -> Result<()> {
    // The key is three UUIDs, so a filename cannot address storage at all — and
    // the filename is refused separately, so both are true.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    for filename in ["../../etc/passwd", "a/b.png", "..", "with\\slash.png"] {
        let (status, body) = caller
            .post(
                &format!("/api/v1/tasks/{task}/attachments"),
                &serde_json::json!({
                    "filename": filename, "content_type": "image/png",
                    "byte_size": 10, "checksum": SHA,
                }),
            )
            .await?;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "accepted {filename:?}: {body}"
        );
    }

    // An accepted upload's key is the three ids and nothing else.
    let id = presign(&caller, task, "ordinary.png", "image/png", 10).await?;
    let key = test_support::attachment_object_key(&db.pool, id).await?;
    assert_eq!(key, format!("{}/{task}/{id}", caller.workspace));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn commit_refuses_a_size_that_does_not_match_and_an_upload_that_never_happened() -> Result<()>
{
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    // Declared 999, uploaded 16.
    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    let id = presign(&caller, task, "chart.png", "image/png", 999).await?;
    caller.upload(caller.workspace, task, id, png).await?;
    let (status, body) = caller
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-ATT-0009");

    // Committing something that was never uploaded.
    let missing = presign(&caller, task, "ghost.png", "image/png", 10).await?;
    let (status, body) = caller
        .post(
            &format!("/api/v1/attachments/{missing}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "TF-ATT-0005");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_attachment_routes_sit_inside_the_csrf_guard() -> Result<()> {
    // A route registered after `.layer()` escapes the guard, and nothing about
    // a handler would show it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    for (uri, body) in [
        (
            format!("/api/v1/tasks/{task}/attachments"),
            r#"{"filename":"a.png","content_type":"image/png","byte_size":1,"checksum":""#
                .to_owned()
                + SHA
                + r#""}"#,
        ),
        (
            format!("/api/v1/attachments/{}/commit", Uuid::now_v7()),
            "{}".to_owned(),
        ),
    ] {
        let response = caller
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&uri)
                    .header(header::COOKIE, &caller.cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(WORKSPACE_HEADER, caller.workspace.to_string())
                    .body(Body::from(body))?,
            )
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{uri} accepted a state change with no CSRF token"
        );
    }
    Ok(())
}
