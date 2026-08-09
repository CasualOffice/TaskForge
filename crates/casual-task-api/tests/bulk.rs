//! `POST /api/v1/tasks/bulk`, end to end (C-008, `docs/05` §Bulk operations).
//!
//! Every test here is about the *seam*, not about the transition rules —
//! `task_operations.rs` owns those, and the handler runs the same code, so
//! re-asserting them here would only prove the call reaches it.
//!
//! What is actually at risk in a bulk endpoint is the isolation: that a refused
//! task leaves the others committed, that the refusal it reports is its own,
//! and that a client can undo what happened when only some of it happened. So
//! the assertions are made against the *database* after the call, not against
//! the status line — a handler that reported `207` with the right counts and
//! rolled everything back would pass a status-code test and be useless.

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
const BULK: &str = "/api/v1/tasks/bulk";

const MEMBER: &[&str] = &[
    "project.create",
    "task.create",
    "task.read",
    "task.update",
    "task.transition",
];

type Answer = (StatusCode, serde_json::Value, Option<String>);

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
}

impl Caller {
    async fn post(&self, uri: &str, body: &serde_json::Value, key: Option<&str>) -> Result<Answer> {
        let mut request = self
            .base("POST", uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        self.send(request.body(Body::from(body.to_string()))?).await
    }

    async fn post_conditional(
        &self,
        uri: &str,
        body: &serde_json::Value,
        if_match: &str,
    ) -> Result<Answer> {
        self.send(
            self.base("POST", uri)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::IF_MATCH, if_match)
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
    })
}

fn key() -> String {
    Uuid::now_v7().to_string()
}

/// A `WORKSPACE`-visible project — the default is `TEAM`, and these projects
/// have no team, which would make every task invisible to its own fixture.
async fn a_project(caller: &Caller) -> Result<Uuid> {
    let (status, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "project create: {project}");
    Ok(project["id"].as_str().context("project id")?.parse()?)
}

/// `n` fresh tasks, each on the initial status. Returns `(id, version)`.
async fn tasks(caller: &Caller, project: Uuid, n: usize) -> Result<Vec<(Uuid, i64)>> {
    let mut made = Vec::with_capacity(n);
    for i in 0..n {
        let (status, task, _) = caller
            .post(
                &format!("/api/v1/projects/{project}/tasks"),
                &serde_json::json!({ "title": format!("Task {i}") }),
                Some(&key()),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "task create: {task}");
        made.push((
            task["id"].as_str().context("task id")?.parse()?,
            task["version"].as_i64().context("version")?,
        ));
    }
    Ok(made)
}

async fn statuses(
    pool: &sqlx::PgPool,
    workspace: Uuid,
) -> Result<std::collections::HashMap<String, Uuid>> {
    Ok(test_support::default_status_ids(pool, workspace)
        .await?
        .into_iter()
        .collect())
}

/// The result for one task, by id — the response is ordered, but an assertion
/// that depended on the order would fail confusingly when only the order broke.
fn result_for(body: &serde_json::Value, id: Uuid) -> &serde_json::Value {
    body["results"]
        .as_array()
        .expect("results")
        .iter()
        .find(|r| r["task_id"] == id.to_string())
        .unwrap_or_else(|| panic!("no result for {id} in {body}"))
}

// ---------------------------------------------------------------------------
// Partial success — the whole point of the endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_stale_task_is_refused_alone_and_the_others_stay_committed() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme").await?;
    let project = a_project(&caller).await?;
    let made = tasks(&caller, project, 3).await?;
    let todo = statuses(&db.pool, caller.workspace).await?["Todo"];
    let (a, b, c) = (made[0], made[1], made[2]);

    let (status, body, _) = caller
        .post(
            BULK,
            &serde_json::json!({
                "operation": "transition",
                "task_ids": [a.0, b.0, c.0],
                "to_status_id": todo,
                "if_match": {
                    a.0.to_string(): a.1,
                    // Someone else moved b between the board loading and this
                    // click. That is the ordinary case, not an outage.
                    b.0.to_string(): b.1 + 99,
                    c.0.to_string(): c.1,
                },
            }),
            None,
        )
        .await?;

    assert_eq!(status, StatusCode::MULTI_STATUS, "{body}");
    assert_eq!(body["succeeded"], 2, "{body}");
    assert_eq!(body["failed"], 1, "{body}");
    assert_eq!(result_for(&body, b.0)["status"], 409);
    assert_eq!(result_for(&body, b.0)["error"]["code"], "TF-CNC-0001");
    assert!(
        result_for(&body, b.0)["task"].is_null(),
        "a refusal carried a task"
    );

    // The database, not the response. `docs/05`: "one bad task does not roll
    // back 99 good ones" — and the inverse, that a refused task is not half
    // written, is the same assertion from the other side.
    for id in [a.0, c.0] {
        let (stored, state) = test_support::task_status_and_state(&db.pool, id).await?;
        assert_eq!(stored, todo, "{id} did not move");
        assert_eq!(state, "PLANNED");
    }
    let (stored, _) = test_support::task_status_and_state(&db.pool, b.0).await?;
    assert_ne!(stored, todo, "the stale task moved anyway");

    // ADR-006: each task that moved wrote its history in its own transaction.
    for id in [a.0, c.0] {
        let (activity, audit, outbox, _) = test_support::history_counts(&db.pool, id).await?;
        assert_eq!((activity, audit, outbox), (2, 2, 2), "create + transition");
    }
    let (activity, audit, outbox, _) = test_support::history_counts(&db.pool, b.0).await?;
    assert_eq!(
        (activity, audit, outbox),
        (1, 1, 1),
        "the refused task wrote history"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_task_with_no_expected_version_is_the_only_one_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme").await?;
    let project = a_project(&caller).await?;
    let made = tasks(&caller, project, 2).await?;
    let todo = statuses(&db.pool, caller.workspace).await?["Todo"];

    let (status, body, _) = caller
        .post(
            BULK,
            &serde_json::json!({
                "operation": "transition",
                "task_ids": [made[0].0, made[1].0],
                "to_status_id": todo,
                // The second task's version is simply absent. A missing version
                // is the same lost-update risk as a wrong one, so it refuses
                // rather than defaulting to "whatever is there".
                "if_match": { made[0].0.to_string(): made[0].1 },
            }),
            None,
        )
        .await?;

    assert_eq!(status, StatusCode::MULTI_STATUS, "{body}");
    assert_eq!(result_for(&body, made[0].0)["status"], 200);
    assert_eq!(result_for(&body, made[1].0)["status"], 428);
    assert_eq!(result_for(&body, made[1].0)["error"]["code"], "TF-CNC-0002");
    let (stored, _) = test_support::task_status_and_state(&db.pool, made[1].0).await?;
    assert_ne!(stored, todo);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_illegal_edge_refuses_its_own_task_and_not_the_legal_one() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme").await?;
    let project = a_project(&caller).await?;
    let made = tasks(&caller, project, 2).await?;
    let by_name = statuses(&db.pool, caller.workspace).await?;
    let (todo, doing) = (by_name["Todo"], by_name["In Progress"]);

    // Move the first one forward so the batch below is genuinely mixed: the
    // default workflow has Todo → In Progress but no Backlog → In Progress.
    let (status, moved, _) = caller
        .post_conditional(
            &format!("/api/v1/tasks/{}/transitions", made[0].0),
            &serde_json::json!({ "to_status_id": todo }),
            &format!("\"{}\"", made[0].1),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{moved}");
    let advanced = moved["version"].as_i64().context("version")?;

    let (status, body, _) = caller
        .post(
            BULK,
            &serde_json::json!({
                "operation": "transition",
                "task_ids": [made[0].0, made[1].0],
                "to_status_id": doing,
                "if_match": {
                    made[0].0.to_string(): advanced,
                    made[1].0.to_string(): made[1].1,
                },
            }),
            None,
        )
        .await?;

    assert_eq!(status, StatusCode::MULTI_STATUS, "{body}");
    assert_eq!(body["succeeded"], 1, "{body}");
    assert_eq!(
        result_for(&body, made[1].0)["error"]["code"],
        "TF-WFL-0002",
        "{body}"
    );
    let (stored, _) = test_support::task_status_and_state(&db.pool, made[0].0).await?;
    assert_eq!(stored, doing);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_task_the_caller_cannot_see_is_one_row_not_the_whole_answer() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme").await?;
    let project = a_project(&caller).await?;
    let made = tasks(&caller, project, 1).await?;
    let todo = statuses(&db.pool, caller.workspace).await?["Todo"];
    let ghost = Uuid::now_v7();

    let (status, body, _) = caller
        .post(
            BULK,
            &serde_json::json!({
                "operation": "transition",
                "task_ids": [made[0].0, ghost],
                "to_status_id": todo,
                "if_match": { made[0].0.to_string(): made[0].1, ghost.to_string(): 1 },
            }),
            None,
        )
        .await?;

    assert_eq!(status, StatusCode::MULTI_STATUS, "{body}");
    assert_eq!(result_for(&body, ghost)["status"], 404);
    assert_eq!(result_for(&body, ghost)["error"]["code"], "TF-TSK-0001");
    assert_eq!(result_for(&body, made[0].0)["status"], 200);
    Ok(())
}

// ---------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_success_carries_the_call_that_reverses_it() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme").await?;
    let project = a_project(&caller).await?;
    let made = tasks(&caller, project, 2).await?;
    let by_name = statuses(&db.pool, caller.workspace).await?;
    let (backlog, todo) = (by_name["Backlog"], by_name["Todo"]);

    let (status, body, _) = caller
        .post(
            BULK,
            &serde_json::json!({
                "operation": "transition",
                "task_ids": [made[0].0, made[1].0],
                "to_status_id": todo,
                "if_match": {
                    made[0].0.to_string(): made[0].1,
                    made[1].0.to_string(): made[1].1,
                },
            }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::MULTI_STATUS, "{body}");

    // Replay the undo the response handed back, without the test remembering
    // anything about the before-state. That is exactly what a client has.
    for (id, _) in &made {
        let undo = &result_for(&body, *id)["undo"];
        assert_eq!(undo["to_status_id"], backlog.to_string(), "{body}");
        let (status, back, _) = caller
            .post_conditional(
                &format!("/api/v1/tasks/{id}/transitions"),
                &serde_json::json!({ "to_status_id": undo["to_status_id"] }),
                &format!("\"{}\"", undo["if_match"].as_i64().context("if_match")?),
            )
            .await?;
        assert_eq!(status, StatusCode::OK, "the undo was refused: {back}");
    }

    for (id, _) in &made {
        let (stored, state) = test_support::task_status_and_state(&db.pool, *id).await?;
        assert_eq!(stored, backlog, "the undo did not put {id} back");
        assert_eq!(state, "BACKLOG");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The envelope — refused whole, because the client could have known
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_malformed_envelope_is_refused_whole_rather_than_per_task() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    // Neither id needs to exist: every check below happens before anything is
    // read, and that is the claim — a client learns its envelope is wrong
    // without the server touching a row.
    let (id, todo) = (Uuid::now_v7(), Uuid::now_v7());

    let cases: Vec<(&str, serde_json::Value, &str)> = vec![
        (
            "an operation nobody implements",
            serde_json::json!({
                "operation": "incinerate",
                "task_ids": [id],
                "to_status_id": todo,
            }),
            "TF-VAL-0005",
        ),
        (
            "transition without a target",
            serde_json::json!({ "operation": "transition", "task_ids": [id] }),
            "TF-VAL-0003",
        ),
        (
            "no tasks at all",
            serde_json::json!({
                "operation": "transition",
                "task_ids": [],
                "to_status_id": todo,
            }),
            "TF-VAL-0004",
        ),
        (
            "the same task twice",
            serde_json::json!({
                "operation": "transition",
                "task_ids": [id, id],
                "to_status_id": todo,
            }),
            "TF-VAL-0004",
        ),
        (
            "a version for a task that was never named",
            serde_json::json!({
                "operation": "transition",
                "task_ids": [id],
                "to_status_id": todo,
                "if_match": { Uuid::now_v7().to_string(): 1 },
            }),
            "TF-VAL-0004",
        ),
    ];
    // A caller each. `docs/21` gives Bulk a burst of 3, so five refusals from
    // one actor would stop being about the envelope at the fourth and start
    // being about the limiter — which `rate_limit.rs` already covers.
    for (i, (what, body, code)) in cases.into_iter().enumerate() {
        let caller = signed_in(
            &db.pool,
            &format!("dev{i}@example.test"),
            &format!("acme{i}"),
        )
        .await?;
        let (status, answer, _) = caller.post(BULK, &body, None).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {answer}");
        assert_eq!(answer["error"]["code"], code, "{what}: {answer}");
    }
    Ok(())
}

/// A refused envelope names no task, so it must also write none — the check has
/// to happen before the first transaction opens, not inside the loop.
#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_refused_envelope_writes_nothing() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme").await?;
    let project = a_project(&caller).await?;
    let made = tasks(&caller, project, 1).await?;
    let todo = statuses(&db.pool, caller.workspace).await?["Todo"];
    let id = made[0].0;
    let before = test_support::history_counts(&db.pool, id).await?;

    // Named twice, and otherwise entirely valid: without the duplicate check
    // the first mention would commit and the second would refuse.
    let (status, answer, _) = caller
        .post(
            BULK,
            &serde_json::json!({
                "operation": "transition",
                "task_ids": [id, id],
                "to_status_id": todo,
                "if_match": { id.to_string(): made[0].1 },
            }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(
        test_support::history_counts(&db.pool, id).await?,
        before,
        "a refused envelope wrote history"
    );
    let (stored, _) = test_support::task_status_and_state(&db.pool, id).await?;
    assert_ne!(stored, todo, "a refused envelope moved the task");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn more_tasks_than_the_limit_names_the_limit() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme").await?;
    // Ids alone — the limit is checked before anything is read, so these do not
    // need to exist. A test that created 101 tasks would be testing the fixture.
    let ids: Vec<Uuid> = (0..101).map(|_| Uuid::now_v7()).collect();

    let (status, body, _) = caller
        .post(
            BULK,
            &serde_json::json!({
                "operation": "transition",
                "task_ids": ids,
                "to_status_id": Uuid::now_v7(),
            }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-LIM-0003", "{body}");
    Ok(())
}
