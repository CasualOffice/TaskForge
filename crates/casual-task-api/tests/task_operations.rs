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

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_patch_updates_fields_clears_with_null_and_moves_the_version() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}");

    let (status, body, next) = caller
        .patch(
            &uri,
            &serde_json::json!({
                "title": "Ship it properly",
                "description": "the long version",
                "priority": "HIGH",
                "type": "BUG",
            }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["title"], "Ship it properly");
    assert_eq!(body["priority"], "HIGH");
    assert_eq!(body["type"], "BUG");
    let next = next.context("no ETag on a successful patch")?;
    assert_ne!(next, etag, "the version did not move");

    // docs/05 §Conventions: `null` clears, absent leaves alone. Both in one
    // request, so a handler that collapsed them would fail here.
    let (status, body, _) = caller
        .patch(
            &uri,
            &serde_json::json!({ "description": null }),
            Some(&next),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["description"].is_null(), "null did not clear: {body}");
    assert_eq!(
        body["title"], "Ship it properly",
        "an absent field was not left alone"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_patch_without_if_match_is_428_and_a_stale_one_is_409() -> Result<()> {
    // docs/05 §Concurrency. A client that forgets If-Match has a bug, and
    // failing loudly in development beats losing a user's edit in production.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}");
    let rename = serde_json::json!({ "title": "renamed" });

    let (status, body, _) = caller.patch(&uri, &rename, None).await?;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["error"]["code"], "TF-CNC-0002");

    let (status, body, _) = caller.patch(&uri, &rename, Some("\"nonsense\"")).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-CNC-0003");

    // Win once, then replay the same stale tag.
    let (status, _, _) = caller.patch(&uri, &rename, Some(&etag)).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, body, _) = caller.patch(&uri, &rename, Some(&etag)).await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "TF-CNC-0001");
    // docs/24: the loser is told what it lost to, so it can merge.
    assert!(
        body["error"]["details"]["current_version"].is_number(),
        "{body}"
    );
    assert!(body["error"]["details"]["current"].is_object(), "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_patch_naming_a_status_is_sent_to_the_transition_endpoint() -> Result<()> {
    // docs/23: "Status is never written through PATCH /tasks/{id}. Attempting it
    // returns 400 TF-WFL-0001." The field is DECLARED so the error can say that,
    // rather than deny_unknown_fields calling it a field nobody has heard of.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}");

    for body in [
        serde_json::json!({ "status_id": Uuid::now_v7() }),
        serde_json::json!({ "state": "COMPLETED" }),
    ] {
        let (status, answer, _) = caller.patch(&uri, &body, Some(&etag)).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        assert_eq!(
            answer["error"]["code"], "TF-WFL-0001",
            "a status write was not pointed at the transition endpoint: {answer}"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// DELETE
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_delete_is_a_tombstone_and_the_task_stops_being_readable() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(
        &db.pool,
        "dev@example.test",
        "acme",
        &[MEMBER, &["task.delete"]].concat(),
    )
    .await?;
    let (_, task, etag) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}");

    let (status, body, _) = caller.delete(&uri, None).await?;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");

    let (status, body, _) = caller.delete(&uri, Some(&etag)).await?;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // docs/03: a delete is a tombstone, not a DELETE. The row survives and the
    // read path does not see it.
    assert!(
        test_support::task_is_deleted(&db.pool, task).await?,
        "the row was hard-deleted"
    );
    let (status, _, _) = caller.get(&uri).await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a deleted task stayed readable"
    );

    let types = test_support::outbox_event_types(&db.pool, task).await?;
    assert!(
        types.contains(&"task.deleted".to_owned()),
        "the delete wrote no event: {types:?}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Transitions — docs/23
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_transition_moves_the_task_and_writes_its_history() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;
    let todo = status_ids["Todo"];

    let (status, body, next) = caller
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": todo, "comment": "starting" }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status_id"], todo.to_string());
    // docs/23: state is written in the same statement as status_id, so the two
    // can never disagree. Read from the row, not from the response.
    let (stored_status, stored_state) = test_support::task_status_and_state(&db.pool, task).await?;
    assert_eq!(stored_status, todo);
    assert_eq!(stored_state, "PLANNED", "the derived state drifted");
    assert_eq!(body["state"], "PLANNED");
    assert_ne!(next.context("etag")?, etag);

    // ADR-006: the domain change and all three history rows commit together.
    let (activity, audit, outbox, deliveries) =
        test_support::history_counts(&db.pool, task).await?;
    assert_eq!((activity, audit, outbox), (2, 2, 2), "create + transition");
    assert_eq!(deliveries, 2 * 6, "one delivery row per consumer per event");
    assert_eq!(
        test_support::outbox_event_types(&db.pool, task).await?,
        vec!["task.created".to_owned(), "task.status.changed".to_owned()]
    );
    // docs/23 §What commits lists the comment among the rows one transaction
    // writes.
    assert_eq!(test_support::comment_count(&db.pool, task).await?, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_status_with_no_edge_from_here_is_refused() -> Result<()> {
    // docs/23 step 4, TF-WFL-0002. The default workflow has no Backlog -> Done
    // edge; work has to pass through Todo and In Progress.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;

    let (status, body, _) = caller
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0002");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn closing_needs_task_close_and_reopening_needs_task_reopen() -> Result<()> {
    // docs/23 §Closing and reopening: closing "requires task.close AND a valid
    // transition edge; both, not either". The default workflow carries the
    // permission on the edge, so this exercises step 5 — TF-WFL-0003, a 403 and
    // not a 422, because the answer is "you may not", not "that is impossible".
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;
    let uri = format!("/api/v1/tasks/{task}");
    let transitions = format!("/api/v1/tasks/{task}/transitions");

    // Backlog -> Todo -> In Progress, neither of which needs a permission.
    let mut tag = etag;
    for name in ["Todo", "In Progress"] {
        let (status, body, next) = caller
            .post_conditional(
                &transitions,
                &serde_json::json!({ "to_status_id": status_ids[name] }),
                Some(&tag),
            )
            .await?;
        assert_eq!(status, StatusCode::OK, "moving to {name}: {body}");
        tag = next.context("etag")?;
    }

    let (status, body, _) = caller
        .post_conditional(
            &transitions,
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            Some(&tag),
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0003");
    assert_eq!(
        body["error"]["details"]["required_permission"],
        "task.close"
    );

    // With the permission, the same move succeeds — so the refusal above was
    // the permission and not the edge.
    let closer = member_of(
        &db.pool,
        "closer@example.test",
        caller.workspace,
        &[MEMBER, &["task.close", "task.reopen"]].concat(),
    )
    .await?;
    let (_, current, tag) = closer.get(&uri).await?;
    assert_eq!(current["status_id"], status_ids["In Progress"].to_string());
    let (status, body, next) = closer
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            tag.as_deref(),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "COMPLETED");

    // And reopening writes a DISTINCT event — docs/23: "how often does work
    // come back?" is a question a generic status-change event cannot serve.
    let (status, body, _) = closer
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["In Progress"] }),
            next.as_deref(),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let types = test_support::outbox_event_types(&db.pool, task).await?;
    assert_eq!(
        types.last().map(String::as_str),
        Some("task.reopened"),
        "leaving a terminal state wrote a generic event: {types:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_move_to_the_status_it_already_has_is_a_no_op() -> Result<()> {
    // docs/23 §Concurrency: "moving to a status the task is already in is a
    // no-op that returns 200 without writing an event. This makes client
    // retries safe without an idempotency key."
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;

    let before = test_support::history_counts(&db.pool, task).await?;
    let (status, body, next) = caller
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Backlog"] }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        next.as_deref(),
        Some(etag.as_str()),
        "a no-op moved the version"
    );
    assert_eq!(
        test_support::history_counts(&db.pool, task).await?,
        before,
        "a no-op transition wrote history"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_blocked_task_cannot_move_until_the_blocker_resolves_or_is_overridden() -> Result<()> {
    // docs/23 step 7, TF-WFL-0005. The error names the blockers the actor can
    // see, which is what makes it actionable rather than merely a refusal.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (project, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;

    let (_, blocker_body, _) = caller
        .post(
            &format!("/api/v1/projects/{project}/tasks"),
            &serde_json::json!({ "title": "Do this first" }),
            Some(&key()),
        )
        .await?;
    let blocker: Uuid = blocker_body["id"].as_str().context("blocker id")?.parse()?;
    test_support::add_blocker(&db.pool, caller.workspace, blocker, task).await?;

    let transitions = format!("/api/v1/tasks/{task}/transitions");
    let (status, body, _) = caller
        .post_conditional(
            &transitions,
            &serde_json::json!({ "to_status_id": status_ids["Todo"] }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0005");
    assert_eq!(
        body["error"]["details"]["blocked_by"][0],
        blocker.to_string(),
        "the blocker was not named: {body}"
    );

    // Cancel is the wildcard edge and opts out of dependency gating entirely,
    // so a blocked task can still be abandoned.
    let (status, body, _) = caller
        .post_conditional(
            &transitions,
            &serde_json::json!({ "to_status_id": status_ids["Canceled"] }),
            Some(&etag),
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "cancel is gated by a blocker: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_first_failure_in_the_documented_order_is_the_one_reported() -> Result<()> {
    // docs/23: "the first failure is the one reported — so the error a user sees
    // is the most actionable one, not whichever check happened to run first."
    //
    // Each request below violates SEVERAL rules at once and must report the
    // earliest. A handler that checked permission before version, or version
    // before visibility, would pass every single-violation test and fail here.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;

    // Step 1 beats everything: an invisible task with a stale version and an
    // unreachable target is a 404, not a 409 or a 422.
    let stranger = signed_in(&db.pool, "other@example.test", "other", MEMBER).await?;
    let (status, body, _) = stranger
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            Some("\"999\""),
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "step 1 did not win: {body}");

    // Step 2 beats step 4: a stale version AND an unreachable status is a 409.
    let (status, body, _) = caller
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            Some("\"999\""),
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "step 2 did not beat step 4: {body}"
    );

    // Step 3 beats step 4: no task.transition AND an unreachable status is a
    // 403 naming the missing grant, not a 422 about the edge.
    let onlooker = member_of(
        &db.pool,
        "onlooker@example.test",
        caller.workspace,
        &["task.read"],
    )
    .await?;
    let (status, body, _) = onlooker
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            Some(&etag),
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "step 3 did not beat step 4: {body}"
    );
    assert_eq!(body["error"]["code"], "TF-AZN-0001");
    Ok(())
}

// ---------------------------------------------------------------------------
// Assignees and tags
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn assigning_is_idempotent_and_unassigning_removes_it() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, _) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}/assignees");
    let body = serde_json::json!({ "user_id": caller.user });

    let (status, answer, _) = caller.post(&uri, &body, None).await?;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    assert_eq!(answer["assignees"][0], caller.user.to_string());

    // A retry of a request whose response was never seen is doing the right
    // thing; an error there makes correct behaviour look broken.
    let (status, answer, _) = caller.post(&uri, &body, None).await?;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        test_support::task_assignees(&db.pool, task).await?.len(),
        1,
        "a retry assigned the same person twice"
    );

    let (status, answer, _) = caller
        .delete(&format!("{uri}/{}", caller.user), None)
        .await?;
    assert_eq!(status, StatusCode::NO_CONTENT, "{answer}");
    assert!(
        test_support::task_assignees(&db.pool, task)
            .await?
            .is_empty()
    );

    // Unassigning someone who is not assigned is a 404, not a silent success.
    let (status, _, _) = caller
        .delete(&format!("{uri}/{}", caller.user), None)
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn work_cannot_be_assigned_to_someone_who_cannot_see_the_project() -> Result<()> {
    // TF-TSK-0005. The invariant that matters is not "has a membership row" —
    // a WORKSPACE-visible project usually has none — but "can see it at all".
    // A stranger and another tenant's user are refused for the same reason.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let outsider = signed_in(&db.pool, "outsider@example.test", "other", MEMBER).await?;
    let (_, task, _) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}/assignees");

    for (label, user) in [
        ("another tenant's member", outsider.user),
        ("nobody at all", Uuid::now_v7()),
    ] {
        let (status, body, _) = caller
            .post(&uri, &serde_json::json!({ "user_id": user }), None)
            .await?;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{label} was assignable: {body}"
        );
        assert_eq!(body["error"]["code"], "TF-TSK-0005");
    }
    assert!(
        test_support::task_assignees(&db.pool, task)
            .await?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_task_can_be_tagged_and_an_unusable_tag_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, _) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}/tags");

    let tag = test_support::insert_tag(&db.pool, caller.workspace, None, "security").await?;
    let (status, body, _) = caller
        .post(&uri, &serde_json::json!({ "tag_id": tag }), None)
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["name"], "security");

    // Idempotent, like assigning.
    let (status, _, _) = caller
        .post(&uri, &serde_json::json!({ "tag_id": tag }), None)
        .await?;
    assert_eq!(status, StatusCode::OK);

    // The activity stream holds the tag's NAME, not its id: docs/25 wants a
    // stream that still reads correctly after the tag is renamed or deleted.
    let types = test_support::outbox_event_types(&db.pool, task).await?;
    assert!(types.contains(&"task.tagged".to_owned()), "{types:?}");

    // A tag from another workspace is refused — and so is one that does not
    // exist, with the same answer.
    let elsewhere = signed_in(&db.pool, "elsewhere@example.test", "other", MEMBER).await?;
    let foreign = test_support::insert_tag(&db.pool, elsewhere.workspace, None, "security").await?;
    for id in [foreign, Uuid::now_v7()] {
        let (status, body, _) = caller
            .post(&uri, &serde_json::json!({ "tag_id": id }), None)
            .await?;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"]["code"], "TF-VAL-0007");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_new_route_sits_inside_the_csrf_guard() -> Result<()> {
    // The rule is about the ROUTER, not the handlers: a route registered after
    // `.layer()` escapes both the CSRF guard and the request id, and nothing
    // about a handler would show it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;

    for (method, uri, body) in [
        ("PATCH", format!("/api/v1/tasks/{task}"), "{}"),
        ("DELETE", format!("/api/v1/tasks/{task}"), ""),
        (
            "POST",
            format!("/api/v1/tasks/{task}/transitions"),
            "{\"to_status_id\":\"00000000-0000-7000-8000-000000000001\"}",
        ),
        (
            "POST",
            format!("/api/v1/tasks/{task}/assignees"),
            "{\"user_id\":\"00000000-0000-7000-8000-000000000001\"}",
        ),
        (
            "DELETE",
            format!("/api/v1/tasks/{task}/assignees/{}", caller.user),
            "",
        ),
        (
            "POST",
            format!("/api/v1/tasks/{task}/tags"),
            "{\"tag_id\":\"00000000-0000-7000-8000-000000000001\"}",
        ),
    ] {
        // Everything a real request has, except the CSRF token.
        let request = Request::builder()
            .method(method)
            .uri(&uri)
            .header(header::COOKIE, &caller.cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_MATCH, &etag)
            .header(WORKSPACE_HEADER, caller.workspace.to_string())
            .body(Body::from(body))?;
        let response = caller.app.clone().oneshot(request).await?;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{method} {uri} accepted a state change with no CSRF token"
        );
        assert!(
            response.headers().contains_key("x-request-id"),
            "{method} {uri} is outside the observability layer too"
        );
    }
    Ok(())
}
