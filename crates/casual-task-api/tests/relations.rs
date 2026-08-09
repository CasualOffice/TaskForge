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

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_tasks_history_is_readable_and_newest_first() -> Result<()> {
    // Every change has written an activity record in the same transaction as
    // the change since C-011 (ADR-006). Until now nothing read them: the data
    // accumulated and the History tab had nothing to call.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (_, task) = a_task(&caller).await?;

    // A second change, so ordering means something.
    let (status, _, etag) = caller.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, updated, _) = caller
        .patch(
            &format!("/api/v1/tasks/{task}"),
            &serde_json::json!({ "title": "Renamed" }),
            etag.as_deref(),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let (status, page, _) = caller
        .get(&format!("/api/v1/tasks/{task}/activity"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    let entries = page["data"].as_array().expect("data");
    assert!(entries.len() >= 2, "history is missing entries: {page}");

    // Newest first (docs/05: the History tab reads top-down).
    assert_eq!(entries[0]["event_type"], "task.updated");
    assert_eq!(
        entries[entries.len() - 1]["event_type"],
        "task.created",
        "the oldest entry is not the create"
    );
    // The actor is resolved for rendering, not left as a bare id.
    assert!(
        entries[0]["actor_name"].is_string(),
        "no actor name to render: {}",
        entries[0]
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn history_pages_by_cursor_without_repeating_or_skipping() -> Result<()> {
    // docs/26 bans OFFSET. activity_event is partitioned by occurred_at, so the
    // cursor carries the timestamp as well as the id — an id-only cursor cannot
    // be resumed without scanning every partition.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (_, task) = a_task(&caller).await?;

    for n in 0..4 {
        let (_, current, etag) = caller.get(&format!("/api/v1/tasks/{task}")).await?;
        assert!(current["id"].is_string());
        let (status, body, _) = caller
            .patch(
                &format!("/api/v1/tasks/{task}"),
                &serde_json::json!({ "title": format!("Rename {n}") }),
                etag.as_deref(),
            )
            .await?;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let mut seen: Vec<String> = Vec::new();
    let mut next: Option<String> = None;
    for _ in 0..6 {
        let uri = next.map_or_else(
            || format!("/api/v1/tasks/{task}/activity?limit=2"),
            |c| format!("/api/v1/tasks/{task}/activity?limit=2&cursor={c}"),
        );
        let (status, page, _) = caller.get(&uri).await?;
        assert_eq!(status, StatusCode::OK, "{page}");
        for entry in page["data"].as_array().expect("data") {
            seen.push(entry["id"].as_str().expect("id").to_owned());
        }
        next = page["page"]["next_cursor"].as_str().map(ToOwned::to_owned);
        if next.is_none() {
            break;
        }
    }

    let unique: std::collections::HashSet<&String> = seen.iter().collect();
    assert_eq!(
        unique.len(),
        seen.len(),
        "an entry was served twice: {seen:?}"
    );
    assert!(seen.len() >= 5, "paging lost entries: {seen:?}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn history_of_an_invisible_task_is_404_and_never_403() -> Result<()> {
    // docs/04: absent and invisible are one answer. The activity stream is the
    // most attractive read in the product for this mistake — it names actors,
    // statuses and titles, keyed by an id the caller supplies.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = caller(&db.pool, "owner@example.com", "acme", RELATER).await?;
    let (_, task) = a_task(&owner).await?;
    let outsider = caller(&db.pool, "outsider@example.com", "other", RELATER).await?;

    let (real, _, _) = outsider
        .get(&format!("/api/v1/tasks/{task}/activity"))
        .await?;
    let (imaginary, _, _) = outsider
        .get(&format!("/api/v1/tasks/{}/activity", Uuid::now_v7()))
        .await?;
    assert_eq!(real, StatusCode::NOT_FOUND);
    assert_eq!(
        real, imaginary,
        "another tenant's task is distinguishable from one that does not exist"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn history_needs_task_history_read_and_a_grant_is_the_only_source() -> Result<()> {
    // docs/25 §The three streams assigns this read to `task.history.read`, not
    // `audit.read` — gating it on the latter would hide a user's own task
    // history behind an administrator's permission.
    let db = schema_harness::TestDatabase::start().await?;
    let author = caller(&db.pool, "author@example.com", "acme", RELATER).await?;
    let (_, task) = a_task(&author).await?;

    // A colleague who can SEE the task but was never granted history.
    let colleague = member_of(
        &db.pool,
        "colleague@example.com",
        author.workspace,
        &["task.read"],
    )
    .await?;
    let (status, body, _) = colleague
        .get(&format!("/api/v1/tasks/{task}/activity"))
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0001");

    // And the holder is allowed — otherwise the test above passes with the
    // endpoint refusing everyone.
    let (status, page, _) = author
        .get(&format!("/api/v1/tasks/{task}/activity"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Dependencies — C-008
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_dependency_is_added_and_reads_back_from_both_ends() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, first) = a_task(&caller).await?;
    let second = another_task(&caller, &project, "Second").await?;

    // "first blocks second".
    let (status, relations, _) = caller
        .post(
            &format!("/api/v1/tasks/{first}/dependencies"),
            &serde_json::json!({ "blocks": second }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{relations}");
    assert_eq!(relations["blocks"][0]["id"], second);
    assert_eq!(relations["blocks"][0]["key"], "WR-2");
    assert_eq!(relations["blocks"][0]["state"], "BACKLOG");
    assert!(
        relations["blocked_by"]
            .as_array()
            .expect("array")
            .is_empty()
    );

    // The other end sees the mirror image. Getting this backwards would draw
    // every arrow the wrong way and look entirely plausible.
    let (status, theirs, _) = caller
        .get(&format!("/api/v1/tasks/{second}/dependencies"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{theirs}");
    assert_eq!(theirs["blocked_by"][0]["id"], first);
    assert!(theirs["blocks"].as_array().expect("array").is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_dependency_that_would_close_a_loop_is_refused() -> Result<()> {
    // ADR-019, and the part that must be impossible to get wrong. A cycle makes
    // "what is blocking this?" non-terminating, and the transition gate, the
    // board and My Work all walk that graph.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;
    let c = another_task(&caller, &project, "C").await?;

    // A blocks B blocks C.
    for (blocker, blocked) in [(&a, &b), (&b, &c)] {
        let (status, body, _) = caller
            .post(
                &format!("/api/v1/tasks/{blocker}/dependencies"),
                &serde_json::json!({ "blocks": blocked }),
                None,
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    // C blocking A would close the loop, two hops away from the edge itself.
    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{c}/dependencies"),
            &serde_json::json!({ "blocks": a }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-TSK-0003");

    // And nothing was written: the refusal is the whole statement, not a check
    // followed by an insert.
    let (_, relations, _) = caller
        .get(&format!("/api/v1/tasks/{c}/dependencies"))
        .await?;
    assert!(
        relations["blocks"].as_array().expect("array").is_empty(),
        "a refused dependency was stored anyway: {relations}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_shortest_possible_cycle_is_refused_too() -> Result<()> {
    // The one-hop case and the zero-hop case. A reachability check that starts
    // its walk one step too late lets both of these through.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;

    // A task cannot block itself.
    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": a }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-TSK-0003");

    // A blocks B, then B blocks A.
    let (status, _, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{b}/dependencies"),
            &serde_json::json!({ "blocks": a }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-TSK-0003");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_same_dependency_twice_is_not_an_error_and_not_a_duplicate() -> Result<()> {
    // The drawer's button is idempotent, and a duplicate is not a cycle. The
    // second call must not be refused as one.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;

    let (first, _, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(first, StatusCode::CREATED);

    let (second, relations, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(second, StatusCode::OK, "a repeat was refused: {relations}");
    assert_eq!(
        relations["blocks"].as_array().expect("array").len(),
        1,
        "the edge was stored twice"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_dependency_on_a_task_in_another_workspace_is_404() -> Result<()> {
    // Absent and invisible are one answer, so a caller cannot discover task ids
    // by proposing dependencies on them.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = caller(&db.pool, "owner@example.com", "acme", RELATER).await?;
    let (_, theirs) = a_task(&owner).await?;

    let stranger = caller(&db.pool, "stranger@example.com", "other", RELATER).await?;
    let (project, mine) = a_task(&stranger).await?;
    assert!(!project.is_empty());

    let (real, body, _) = stranger
        .post(
            &format!("/api/v1/tasks/{mine}/dependencies"),
            &serde_json::json!({ "blocks": theirs }),
            None,
        )
        .await?;
    let (imaginary, _, _) = stranger
        .post(
            &format!("/api/v1/tasks/{mine}/dependencies"),
            &serde_json::json!({ "blocks": Uuid::now_v7() }),
            None,
        )
        .await?;
    assert_eq!(real, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(real, imaginary, "a foreign task id is distinguishable");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn adding_a_dependency_needs_task_update() -> Result<()> {
    // A dependency changes how a task behaves — it gates its transitions
    // (ADR-019) — so it is a task update. There is no `task.dependency.add` in
    // the closed registry.
    let db = schema_harness::TestDatabase::start().await?;
    let author = caller(&db.pool, "author@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&author).await?;
    let b = another_task(&author, &project, "B").await?;

    let reader = member_of(
        &db.pool,
        "reader@example.com",
        author.workspace,
        &["task.read"],
    )
    .await?;
    let (status, body, _) = reader
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0001");

    // Reading relations only needs to see the task.
    let (status, _, _) = reader
        .get(&format!("/api/v1/tasks/{a}/dependencies"))
        .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_request_naming_neither_direction_or_both_is_refused() -> Result<()> {
    // Picking a direction silently is how a Relations panel ends up drawing the
    // arrow backwards.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;

    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({}),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b, "blocked_by": b }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn adding_a_dependency_writes_its_history_in_the_same_transaction() -> Result<()> {
    // ADR-006, and the join between the two features in this file: the edge and
    // its activity record commit together, so the History tab shows it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;

    let (status, _, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, page, _) = caller.get(&format!("/api/v1/tasks/{a}/activity")).await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    let entries = page["data"].as_array().expect("data");
    assert_eq!(
        entries[0]["event_type"], "task.dependency.added",
        "the dependency left no history: {page}"
    );
    assert_eq!(entries[0]["changes"]["direction"], "blocks");
    Ok(())
}
