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

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_status_is_added_renamed_and_appears_in_the_workflow() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&caller, "WFA").await?;

    let (status, created) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "In Review", "state": "ACTIVE" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    // The representation is the whole workflow, not the status — every
    // authoring call returns the surface a board has to re-render.
    let id = status_named(&created, "In Review")["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let (status, listed) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(status_named(&listed, "In Review")["state"], "ACTIVE");

    let (status, renamed) = caller
        .patch_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{id}"),
            &json!({ "name": "Under Review" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{renamed}");

    let (_, listed) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    assert_eq!(status_named(&listed, "Under Review")["id"], id.as_str());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_status_name_is_unique_inside_one_workflow() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&caller, "WFB").await?;

    let (status, _) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "Triage", "state": "PLANNED" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "Triage", "state": "PLANNED" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0009");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_state_outside_the_five_is_refused() -> Result<()> {
    // `docs/23`: the five permanent states are a closed enum, forever. A
    // workflow author renames and reorders statuses; they never invent a state.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&caller, "WFC").await?;

    let (status, body) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "Blocked", "state": "BLOCKED" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_ne!(
        status,
        StatusCode::CREATED,
        "BLOCKED is a status, not a state"
    );
    assert!(status.is_client_error(), "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn reordering_rewrites_the_whole_order() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&caller, "WFD").await?;

    let (_, before) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    let mut ids: Vec<String> = before["statuses"]
        .as_array()
        .expect("statuses")
        .iter()
        .map(|s| s["id"].as_str().expect("id").to_owned())
        .collect();
    assert!(ids.len() >= 2, "the default workflow has several statuses");
    ids.reverse();

    let (status, body) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses/order"),
            &json!({ "order": ids }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, after) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    let now: Vec<String> = after["statuses"]
        .as_array()
        .expect("statuses")
        .iter()
        .map(|s| s["id"].as_str().expect("id").to_owned())
        .collect();
    assert_eq!(now, ids, "the order the caller sent is the order returned");

    // Positions must stay distinct — `workflow_status` has no unique constraint
    // on `(workflow_id, position)`, and two statuses sharing one makes a board's
    // column order depend on which row the planner returns first.
    let mut positions: Vec<i64> = after["statuses"]
        .as_array()
        .expect("statuses")
        .iter()
        .map(|s| s["position"].as_i64().expect("position"))
        .collect();
    let total = positions.len();
    positions.sort_unstable();
    positions.dedup();
    assert_eq!(positions.len(), total, "two statuses share a position");
    Ok(())
}

// ── Deleting a status — the part docs/23 exists to settle ───────────────────

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_status_holding_tasks_cannot_be_deleted_without_a_target() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, workflow) = a_project(&caller, "WFE").await?;
    let task = a_task(&caller, &project, "Something in the initial status").await?;
    let (on, _) = status_of(&caller, &task).await?;
    demote_initial(&caller, &workflow, &on).await?;

    let (status, body) = caller
        .delete_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{on}"),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0006");

    // And the task is untouched — a refused delete moves nothing.
    let (still_on, _) = status_of(&caller, &task).await?;
    assert_eq!(still_on, on);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn deleting_with_a_target_moves_every_task_and_says_how_many() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, workflow) = a_project(&caller, "WFF").await?;

    let one = a_task(&caller, &project, "First").await?;
    let two = a_task(&caller, &project, "Second").await?;
    let (from, _) = status_of(&caller, &one).await?;

    // Somewhere for them to go.
    let (_, target) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "Parked", "state": "BACKLOG" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    let to = status_named(&target, "Parked")["id"]
        .as_str()
        .expect("id")
        .to_owned();

    // The initial status cannot be the one deleted — a workflow must keep
    // exactly one — so promote the target first.
    let (status, body) = caller
        .patch_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{to}"),
            &json!({ "is_initial": true }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = caller
        .delete_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{from}?migrate_to={to}"),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["migrated_tasks"], 2, "{body}");

    for task in [&one, &two] {
        let (now, _) = status_of(&caller, task).await?;
        assert_eq!(now, to, "task {task} did not move");
    }

    // `docs/23`: each move writes an activity event attributed to the acting
    // admin. Lazily remapping on next read would satisfy the assertions above
    // and produce a task whose history does not explain its status.
    let (status, history) = caller.get(&format!("/api/v1/tasks/{one}/activity")).await?;
    assert_eq!(status, StatusCode::OK, "{history}");
    let events = history["data"].as_array().expect("data");
    assert!(
        events.iter().any(|e| {
            e["event_type"]
                .as_str()
                .is_some_and(|t| t.contains("status"))
                || e["changes"].to_string().contains("workflow_migration")
        }),
        "no activity event explains the move: {history}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_migration_target_from_another_workflow_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, workflow) = a_project(&caller, "WFG").await?;
    let (_, elsewhere) = a_project(&caller, "WFH").await?;
    let task = a_task(&caller, &project, "Held").await?;
    let (from, _) = status_of(&caller, &task).await?;
    demote_initial(&caller, &workflow, &from).await?;

    let (_, other) = caller
        .get(&format!("/api/v1/workflows/{elsewhere}"))
        .await?;
    let foreign = other["statuses"].as_array().expect("statuses")[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let (status, body) = caller
        .delete_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{from}?migrate_to={foreign}"),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0008");
    Ok(())
}

// ── Authority ───────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn authoring_needs_project_workflow_manage() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&admin, "WFI").await?;

    // Everything except the authoring permission.
    let member = member_of(
        &db.pool,
        "member@example.com",
        admin.workspace,
        &["project.create", "task.create", "task.read", "task.update"],
    )
    .await?;

    let version = version_of(&admin, &workflow).await?;
    let (status, body) = member
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "Sneaky", "state": "PLANNED" }),
            version,
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    Ok(())
}

// ── Transitions ─────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_transition_is_added_and_the_same_edge_twice_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&caller, "WFJ").await?;

    let (_, view) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    let statuses = view["statuses"].as_array().expect("statuses");
    let from = statuses[0]["id"].as_str().expect("id").to_owned();
    let to = statuses[statuses.len() - 1]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let body = json!({ "from": from, "to": to, "required_fields": [] });
    let (status, created) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/transitions"),
            &body,
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let (status, again) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/transitions"),
            &body,
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{again}");
    assert_eq!(again["error"]["code"], "TF-WFL-0010");
    Ok(())
}

// ── Environments ────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_environment_is_created_listed_renamed_and_set_on_a_task() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, _) = a_project(&caller, "ENA").await?;
    let task = a_task(&caller, &project, "Fails in staging").await?;

    let (status, created) = caller
        .post(
            &format!("/api/v1/projects/{project}/environments"),
            &json!({ "name": "Staging" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().expect("id").to_owned();

    let (status, listed) = caller
        .get(&format!("/api/v1/projects/{project}/environments"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        listed["data"]
            .as_array()
            .expect("data")
            .iter()
            .any(|e| e["id"] == id.as_str())
    );

    let (status, renamed) = caller
        .patch(
            &format!("/api/v1/environments/{id}"),
            &json!({ "name": "Stage" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Stage");

    let (status, set) = caller
        .put_at(
            &format!("/api/v1/tasks/{task}/environment"),
            &json!({ "environment_id": id }),
            task_version(&caller, &task).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{set}");

    let (status, read) = caller.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["environment_id"], id.as_str());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_environment_name_is_unique_inside_one_project() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, _) = a_project(&caller, "ENB").await?;

    let make = json!({ "name": "QA" });
    let (status, _) = caller
        .post(&format!("/api/v1/projects/{project}/environments"), &make)
        .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = caller
        .post(&format!("/api/v1/projects/{project}/environments"), &make)
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "TF-PRJ-0009");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_environment_holding_tasks_needs_a_migration_target() -> Result<()> {
    // The same rule as a status, for the same reason: a task pointing at a row
    // that no longer exists is a task whose history does not explain it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, _) = a_project(&caller, "ENC").await?;
    let task = a_task(&caller, &project, "Reproduces in QA").await?;

    let (_, qa) = caller
        .post(
            &format!("/api/v1/projects/{project}/environments"),
            &json!({ "name": "QA" }),
        )
        .await?;
    let qa_id = qa["id"].as_str().expect("id").to_owned();
    let (status, set) = caller
        .put_at(
            &format!("/api/v1/tasks/{task}/environment"),
            &json!({ "environment_id": qa_id }),
            task_version(&caller, &task).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{set}");

    let (status, body) = caller
        .delete(&format!("/api/v1/environments/{qa_id}"))
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-PRJ-0005");

    // The task still points at it.
    let (_, read) = caller.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(read["environment_id"], qa_id.as_str());
    Ok(())
}
