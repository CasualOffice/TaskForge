//! Releases, end to end (`docs/45` §The two clocks).
//!
//! # What is worth asserting here
//!
//! Not that a row is written. The properties below are the ones a release is
//! *for*, and each is one a plausible implementation gets wrong:
//!
//! - the batch is **atomic** — a task from another project refuses the whole
//!   release and moves nothing, because a release that recorded nine of eleven
//!   reads as complete and hides the two that matter;
//! - it **moves the second clock**, so the environment view answers "what is on
//!   staging" without a second concept;
//! - each task's own history says **which release carried it**, because the
//!   task is the only place someone looks when debugging that task;
//! - a **duplicate name** in one project is refused, since "did 2.4.0 go out"
//!   has to have one answer;
//! - a **repeated id** in the request is not a missing task, and does not
//!   silently make the release smaller than it claims.

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

/// Cutting a release is `task.update`, and the fixture needs to author a
/// project and its environments to have somewhere to release to.
const MEMBER: &[&str] = &[
    "project.create",
    "project.update",
    "task.create",
    "task.read",
    "task.update",
    "task.history.read",
];

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
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
    })
}

async fn project(caller: &Caller, key: &str, name: &str) -> Result<Uuid> {
    let (status, project) = caller
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": key, "name": name, "visibility": "WORKSPACE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    Ok(project["id"].as_str().context("project id")?.parse()?)
}

async fn task(caller: &Caller, project: Uuid, title: &str) -> Result<Uuid> {
    let (status, task) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{project}/tasks"),
            &json!({ "title": title }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    Ok(task["id"].as_str().context("task id")?.parse()?)
}

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

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_release_moves_every_task_it_names_and_says_so_on_each_one() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "lead@example.test", "acme").await?;
    let project_id = project(&caller, "WR", "Work").await?;
    let staging = environment(&caller, project_id, "staging").await?;
    let one = task(&caller, project_id, "Login crashes on rotate").await?;
    let two = task(&caller, project_id, "Search returns nothing").await?;

    let (status, cut) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{project_id}/releases"),
            &json!({
                "name": "2.4.0",
                "note": "Friday train",
                "environment_id": staging,
                "task_ids": [one, two],
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{cut}");
    assert_eq!(cut["release"]["name"], "2.4.0");
    assert_eq!(cut["task_ids"].as_array().context("ids")?.len(), 2);

    // The second clock moved, which is what makes the environment view able to
    // answer "what is on staging" without a second concept.
    for id in [one, two] {
        let (_, task) = caller.get(&format!("/api/v1/tasks/{id}")).await?;
        assert_eq!(
            task["environment_id"],
            staging.to_string(),
            "task {id} did not reach staging: {task}"
        );
    }

    // And the task itself says what carried it there. A release recorded only
    // against the release is invisible from the one screen someone opens when
    // they are debugging that task.
    let (_, history) = caller.get(&format!("/api/v1/tasks/{one}/activity")).await?;
    let carried = history["data"]
        .as_array()
        .context("activity")?
        .iter()
        .any(|entry| entry["changes"]["release_name"] == "2.4.0");
    assert!(
        carried,
        "the task's history does not name the release: {history}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_task_from_another_project_refuses_the_whole_release() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "lead@example.test", "acme").await?;
    let ours = project(&caller, "WR", "Work").await?;
    let theirs = project(&caller, "OT", "Other").await?;
    let staging = environment(&caller, ours, "staging").await?;
    let mine = task(&caller, ours, "Login crashes on rotate").await?;
    let stranger = task(&caller, theirs, "Not ours").await?;

    let (status, refused) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{ours}/releases"),
            &json!({
                "name": "2.4.0",
                "environment_id": staging,
                "task_ids": [mine, stranger],
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");

    // Nothing moved — including the task that *was* eligible. That is the whole
    // point of the atomicity: a half-recorded release is worse than none.
    let (_, task) = caller.get(&format!("/api/v1/tasks/{mine}")).await?;
    assert!(
        task["environment_id"].is_null(),
        "the eligible task moved anyway: {task}"
    );

    // And no release is left behind to claim it happened.
    let (_, list) = caller
        .get(&format!("/api/v1/projects/{ours}/releases"))
        .await?;
    assert!(
        list["data"].as_array().context("data")?.is_empty(),
        "a refused release was recorded: {list}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn one_project_cannot_have_two_releases_of_the_same_name() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "lead@example.test", "acme").await?;
    let project_id = project(&caller, "WR", "Work").await?;
    let staging = environment(&caller, project_id, "staging").await?;
    let one = task(&caller, project_id, "First").await?;
    let two = task(&caller, project_id, "Second").await?;

    let body =
        |task: Uuid| json!({ "name": "2.4.0", "environment_id": staging, "task_ids": [task] });
    let (status, first) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{project_id}/releases"),
            &body(one),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{first}");

    let (status, second) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{project_id}/releases"),
            &body(two),
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{second}");
    assert_eq!(second["error"]["code"], "TF-PRJ-0015", "{second}");

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn naming_the_same_task_twice_is_one_task_and_not_a_refusal() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "lead@example.test", "acme").await?;
    let project_id = project(&caller, "WR", "Work").await?;
    let staging = environment(&caller, project_id, "staging").await?;
    let only = task(&caller, project_id, "Login crashes on rotate").await?;

    // A client that builds the list from a multi-select can repeat an id. The
    // update moves one row, and comparing that against the raw length would
    // read as "a task is missing" and refuse a release that is perfectly fine.
    let (status, cut) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{project_id}/releases"),
            &json!({
                "name": "2.4.0",
                "environment_id": staging,
                "task_ids": [only, only],
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{cut}");
    assert_eq!(
        cut["task_ids"].as_array().context("ids")?.len(),
        1,
        "the release claims more tasks than it carried: {cut}"
    );

    let (status, read) = caller
        .get(&format!(
            "/api/v1/releases/{}",
            cut["release"]["id"].as_str().context("release id")?
        ))
        .await?;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(
        read["tasks"].as_array().context("tasks")?.len(),
        1,
        "{read}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_environment_from_another_project_is_refused_before_anything_moves() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "lead@example.test", "acme").await?;
    let ours = project(&caller, "WR", "Work").await?;
    let theirs = project(&caller, "OT", "Other").await?;
    let elsewhere = environment(&caller, theirs, "staging").await?;
    let mine = task(&caller, ours, "Login crashes on rotate").await?;

    let (status, refused) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{ours}/releases"),
            &json!({
                "name": "2.4.0",
                "environment_id": elsewhere,
                "task_ids": [mine],
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");

    let (_, list) = caller
        .get(&format!("/api/v1/projects/{ours}/releases"))
        .await?;
    assert!(
        list["data"].as_array().context("data")?.is_empty(),
        "a refused release was recorded: {list}"
    );

    Ok(())
}
