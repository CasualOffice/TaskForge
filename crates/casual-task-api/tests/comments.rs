//! Projects and tasks, end to end, against a real PostgreSQL (C-006, C-008).
//!
//! Two kinds of test live here and both are necessary. The happy path proves a
//! user can now do the thing the product exists for — create a project, create
//! a task in it, read both back. Everything else proves a refusal: another
//! workspace's project is a `404` and not a `403`, a write without `If-Match`
//! is refused rather than silently accepted, and a stale `If-Match` loses
//! rather than overwriting.
//!
//! Every test here fails without the code it covers. That is the bar, and it is
//! why the history assertions count rows rather than asserting a `201`: a create
//! that returned `201` and wrote no audit record would pass a status-code test
//! and violate ADR-006.

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
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: SECRET.into(),
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
const COMMENTER: &[&str] = &["project.create", "task.create", "task.read", "task.comment"];

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

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_comment_can_be_posted_and_read_back_in_the_thread() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", COMMENTER).await?;
    let (project, task) = a_task(&caller).await?;

    let (status, comment, etag) = caller
        .post(
            &format!("/api/v1/tasks/{task}/comments"),
            &serde_json::json!({ "body": "Looks right to me" }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{comment}");
    assert_eq!(comment["body"], "Looks right to me");
    assert_eq!(comment["version"], 1);
    assert_eq!(etag.as_deref(), Some("\"1\""));

    let (status, page, _) = caller
        .get(&format!("/api/v1/tasks/{task}/comments"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["data"][0]["id"], comment["id"]);
    assert_eq!(page["page"]["has_more"], false);
    let _ = project;
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn commenting_needs_the_permission_and_a_grant_is_the_only_source() -> Result<()> {
    // Membership is not authority (migration 0003). A member without
    // `task.comment` must be refused even though they can read the task.
    let db = schema_harness::TestDatabase::start().await?;
    let author = caller(&db.pool, "author@example.com", "acme", COMMENTER).await?;
    let (_, task) = a_task(&author).await?;

    let reader = member_of(
        &db.pool,
        "reader@example.com",
        author.workspace,
        &["task.read"],
    )
    .await?;
    let (status, body, _) = reader
        .post(
            &format!("/api/v1/tasks/{task}/comments"),
            &serde_json::json!({ "body": "let me in" }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_task_in_another_workspace_is_404_and_never_403() -> Result<()> {
    // docs/04: absent and invisible are indistinguishable. A comment endpoint
    // that answered differently would leak which task ids exist.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = caller(&db.pool, "owner@example.com", "acme", COMMENTER).await?;
    let (_, task) = a_task(&owner).await?;

    let outsider = caller(&db.pool, "outsider@example.com", "other", COMMENTER).await?;
    let (real, body, _) = outsider
        .post(
            &format!("/api/v1/tasks/{task}/comments"),
            &serde_json::json!({ "body": "hello" }),
            None,
        )
        .await?;
    let (imaginary, _, _) = outsider
        .post(
            &format!("/api/v1/tasks/{}/comments", Uuid::now_v7()),
            &serde_json::json!({ "body": "hello" }),
            None,
        )
        .await?;
    assert_eq!(real, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        real, imaginary,
        "a real task in another workspace is distinguishable from one that does \
         not exist"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn threading_is_one_level_deep() -> Result<()> {
    // The schema permits arbitrary depth; docs/06 says one level. A reply to a
    // reply produces a thread nobody can render and a table nobody can migrate.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", COMMENTER).await?;
    let (_, task) = a_task(&caller).await?;

    let (_, top, _) = caller
        .post(
            &format!("/api/v1/tasks/{task}/comments"),
            &serde_json::json!({ "body": "top level" }),
            None,
        )
        .await?;
    let top_id = top["id"].as_str().expect("id").to_owned();

    let (status, reply, _) = caller
        .post(
            &format!("/api/v1/tasks/{task}/comments"),
            &serde_json::json!({ "body": "a reply", "parent_comment_id": top_id }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{reply}");
    let reply_id = reply["id"].as_str().expect("id").to_owned();

    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{task}/comments"),
            &serde_json::json!({ "body": "too deep", "parent_comment_id": reply_id }),
            None,
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a reply to a reply was accepted: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn only_the_author_can_edit_and_a_stale_version_is_409() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let author = caller(&db.pool, "author@example.com", "acme", COMMENTER).await?;
    let (_, task) = a_task(&author).await?;
    let (_, comment, _) = author
        .post(
            &format!("/api/v1/tasks/{task}/comments"),
            &serde_json::json!({ "body": "mine" }),
            None,
        )
        .await?;
    let id = comment["id"].as_str().expect("id").to_owned();

    // Without If-Match: 428, never a silent overwrite.
    let (status, _, _) = author
        .patch(
            &format!("/api/v1/comments/{id}"),
            &serde_json::json!({ "body": "edited" }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);

    // A stale version is a conflict, not a lost edit.
    let (status, _, _) = author
        .patch(
            &format!("/api/v1/comments/{id}"),
            &serde_json::json!({ "body": "edited" }),
            Some("\"99\""),
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, edited, _) = author
        .patch(
            &format!("/api/v1/comments/{id}"),
            &serde_json::json!({ "body": "edited" }),
            Some("\"1\""),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["body"], "edited");
    assert_eq!(edited["version"], 2);

    // Someone else holding the same permission still cannot rewrite it, and is
    // told 404 rather than 403 — the comment is not theirs to know about.
    let other = member_of(&db.pool, "other@example.com", author.workspace, COMMENTER).await?;
    let (status, _, _) = other
        .patch(
            &format!("/api/v1/comments/{id}"),
            &serde_json::json!({ "body": "not yours" }),
            Some("\"2\""),
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another member edited someone else's comment"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_thread_pages_by_cursor_without_repeating_or_skipping() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", COMMENTER).await?;
    let (_, task) = a_task(&caller).await?;

    for n in 0..5 {
        let (status, body, _) = caller
            .post(
                &format!("/api/v1/tasks/{task}/comments"),
                &serde_json::json!({ "body": format!("comment {n}") }),
                None,
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    let mut seen: Vec<String> = Vec::new();
    let mut uri = format!("/api/v1/tasks/{task}/comments?limit=2");
    loop {
        let (status, page, _) = caller.get(&uri).await?;
        assert_eq!(status, StatusCode::OK, "{page}");
        for item in page["data"].as_array().expect("data") {
            seen.push(item["id"].as_str().expect("id").to_owned());
        }
        match page["page"]["next_cursor"].as_str() {
            Some(cursor) => {
                uri = format!("/api/v1/tasks/{task}/comments?limit=2&cursor={cursor}");
            }
            None => break,
        }
    }

    assert_eq!(seen.len(), 5, "paging lost or repeated rows: {seen:?}");
    let unique: std::collections::HashSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 5, "a row appeared on two pages");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_empty_body_and_an_unknown_field_are_both_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", COMMENTER).await?;
    let (_, task) = a_task(&caller).await?;

    let (status, _, _) = caller
        .post(
            &format!("/api/v1/tasks/{task}/comments"),
            &serde_json::json!({ "body": "   " }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "whitespace is not a body");

    // docs/05: a typo'd field is rejected, never ignored.
    let (status, _, _) = caller
        .post(
            &format!("/api/v1/tasks/{task}/comments"),
            &serde_json::json!({ "body": "ok", "reply_to": "oops" }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    Ok(())
}
