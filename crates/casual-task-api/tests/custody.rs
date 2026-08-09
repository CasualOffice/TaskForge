//! The chain of custody, end to end (`docs/45`).
//!
//! # What is worth asserting here
//!
//! Not that a row is written — that is the easy half. The properties below are
//! the ones the lifecycle depends on and that a plausible implementation gets
//! wrong:
//!
//! - a transfer **clears the assignees**, because the task has to land in the
//!   receiving team's queue rather than staying attached to someone who is done
//!   with it;
//! - a task cannot be handed to a team that is **not on its project**, because a
//!   task owned by people who cannot see it is a disappearance, not a hand-off;
//! - a promotion writes a **log row as well as the column**, or "when did this
//!   reach staging" is unanswerable and the column is the only fact left;
//! - the environment endpoint that predates all of this **also** logs, so the
//!   history is complete regardless of which door the task went through;
//! - a verdict is **not** a status change, so a task can fail twice on the same
//!   environment and both survive.

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

/// Everything the custody commands need between them.
const MEMBER: &[&str] = &[
    "project.create",
    "task.create",
    "task.read",
    "task.update",
    "task.assign",
    "task.transition",
    // Authoring an environment is `project.update` — the fixture creates the
    // pipeline these tests promote along, which is a project configuration
    // change and not a task one.
    "project.update",
];

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
    user: Uuid,
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
        user,
    })
}

/// A project, a task in it, and two teams on the project.
struct Fixture {
    project: Uuid,
    task: Uuid,
    android: Uuid,
    backend: Uuid,
}

async fn fixture(pool: &sqlx::PgPool, caller: &Caller) -> Result<Fixture> {
    let (status, project) = caller
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;

    let (status, task) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{project_id}/tasks"),
            &json!({ "title": "Login crashes on rotate" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let task_id: Uuid = task["id"].as_str().context("task id")?.parse()?;

    let android = test_support::insert_team(pool, caller.workspace, "Android").await?;
    let backend = test_support::insert_team(pool, caller.workspace, "Backend").await?;
    for team in [android, backend] {
        test_support::add_project_team(pool, caller.workspace, project_id, team).await?;
    }

    Ok(Fixture {
        project: project_id,
        task: task_id,
        android,
        backend,
    })
}

/// An environment on the project, since only the API can make one.
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
async fn a_transfer_moves_the_team_clears_the_assignees_and_is_logged() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "qa@example.test", "acme").await?;
    let f = fixture(&db.pool, &caller).await?;

    // Someone is working on it, which is what makes the clearing observable.
    caller
        .send_json(
            "POST",
            &format!("/api/v1/tasks/{}/assignees", f.task),
            &json!({ "user_id": caller.user }),
        )
        .await?;

    let (status, moved) = caller
        .send_json(
            "PUT",
            &format!("/api/v1/tasks/{}/team", f.task),
            &json!({ "team_id": f.android, "note": "crash is in the app" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["to_team_id"], f.android.to_string());
    // The first hand-off is from nobody: intake happens before triage.
    assert!(moved["from_team_id"].is_null(), "{moved}");

    // The point of the transfer: it lands in a queue, not on the last person.
    let (_, assignees) = caller
        .get(&format!("/api/v1/tasks/{}/assignees", f.task))
        .await?;
    assert!(
        assignees["assignees"]
            .as_array()
            .context("assignees")?
            .is_empty(),
        "the transfer left the previous assignee attached: {assignees}"
    );

    let (status, custody) = caller
        .get(&format!("/api/v1/tasks/{}/custody", f.task))
        .await?;
    assert_eq!(status, StatusCode::OK, "{custody}");
    assert_eq!(custody["team_id"], f.android.to_string());
    assert_eq!(
        custody["transfers"].as_array().context("transfers")?.len(),
        1
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_round_trip_between_teams_is_two_events_and_the_bounce_is_countable() -> Result<()> {
    // The number that exposes a broken process. A log that collapsed a return
    // trip into "currently Android" would lose exactly the fact worth having.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "qa@example.test", "acme").await?;
    let f = fixture(&db.pool, &caller).await?;

    for team in [f.android, f.backend, f.android] {
        let (status, body) = caller
            .send_json(
                "PUT",
                &format!("/api/v1/tasks/{}/team", f.task),
                &json!({ "team_id": team }),
            )
            .await?;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let (_, custody) = caller
        .get(&format!("/api/v1/tasks/{}/custody", f.task))
        .await?;
    let transfers = custody["transfers"].as_array().context("transfers")?;
    assert_eq!(transfers.len(), 3, "the bounce was collapsed: {custody}");
    assert_eq!(custody["team_id"], f.android.to_string());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn handing_a_task_to_the_team_that_already_owns_it_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "qa@example.test", "acme").await?;
    let f = fixture(&db.pool, &caller).await?;

    caller
        .send_json(
            "PUT",
            &format!("/api/v1/tasks/{}/team", f.task),
            &json!({ "team_id": f.android }),
        )
        .await?;
    let (status, body) = caller
        .send_json(
            "PUT",
            &format!("/api/v1/tasks/{}/team", f.task),
            &json!({ "team_id": f.android }),
        )
        .await?;
    // A second identical hand-off is not an event; recording one would inflate
    // the bounce count that the previous test depends on.
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_task_cannot_be_handed_to_a_team_that_is_not_on_its_project() -> Result<()> {
    // Not a hand-off — a disappearance. The receiving team could not see it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "qa@example.test", "acme").await?;
    let f = fixture(&db.pool, &caller).await?;
    let outsider = test_support::insert_team(&db.pool, caller.workspace, "Marketing").await?;

    let (status, body) = caller
        .send_json(
            "PUT",
            &format!("/api/v1/tasks/{}/team", f.task),
            &json!({ "team_id": outsider }),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-VAL-0007");

    let (_, custody) = caller
        .get(&format!("/api/v1/tasks/{}/custody", f.task))
        .await?;
    assert!(custody["team_id"].is_null(), "the refused transfer applied");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_promotion_moves_the_second_clock_and_leaves_a_trail() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "qa@example.test", "acme").await?;
    let f = fixture(&db.pool, &caller).await?;
    let qa = environment(&caller, f.project, "qa").await?;
    let staging = environment(&caller, f.project, "staging").await?;

    for env in [qa, staging] {
        let (status, promoted) = caller
            .send_json(
                "POST",
                &format!("/api/v1/tasks/{}/promotions", f.task),
                &json!({ "environment_id": env }),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{promoted}");
    }

    let (_, custody) = caller
        .get(&format!("/api/v1/tasks/{}/custody", f.task))
        .await?;
    // The column is where it is now; the log is how it got there. Both, or the
    // question "when did this reach staging" has no answer.
    assert_eq!(custody["environment_id"], staging.to_string());
    let promotions = custody["promotions"].as_array().context("promotions")?;
    assert_eq!(promotions.len(), 2, "{custody}");
    assert_eq!(promotions[0]["environment_id"], staging.to_string());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_redeploy_to_the_same_environment_is_a_second_event() -> Result<()> {
    // Deliberately not idempotent: a redeploy to staging happened, and a log
    // that swallowed it would understate the work.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "qa@example.test", "acme").await?;
    let f = fixture(&db.pool, &caller).await?;
    let staging = environment(&caller, f.project, "staging").await?;

    for _ in 0..2 {
        caller
            .send_json(
                "POST",
                &format!("/api/v1/tasks/{}/promotions", f.task),
                &json!({ "environment_id": staging }),
            )
            .await?;
    }

    let (_, custody) = caller
        .get(&format!("/api/v1/tasks/{}/custody", f.task))
        .await?;
    assert_eq!(
        custody["promotions"]
            .as_array()
            .context("promotions")?
            .len(),
        2
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_older_environment_endpoint_also_leaves_a_promotion() -> Result<()> {
    // `PUT /tasks/{id}/environment` predates the promotion log and still writes
    // the column under `If-Match`. If it did not also log, the history would be
    // complete or not depending on which door the task went through.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "qa@example.test", "acme").await?;
    let f = fixture(&db.pool, &caller).await?;
    let qa = environment(&caller, f.project, "qa").await?;

    let (_, task) = caller.get(&format!("/api/v1/tasks/{}", f.task)).await?;
    let version = task["version"].as_i64().context("version")?;

    let response = caller
        .app
        .clone()
        .oneshot(
            caller
                .base("PUT", &format!("/api/v1/tasks/{}/environment", f.task))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::IF_MATCH, format!("\"{version}\""))
                .body(Body::from(json!({ "environment_id": qa }).to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let (_, custody) = caller
        .get(&format!("/api/v1/tasks/{}/custody", f.task))
        .await?;
    let promotions = custody["promotions"].as_array().context("promotions")?;
    assert_eq!(
        promotions.len(),
        1,
        "the older door left no trail: {custody}"
    );
    assert_eq!(promotions[0]["environment_id"], qa.to_string());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_verdict_is_recorded_against_an_environment_and_failures_accumulate() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "qa@example.test", "acme").await?;
    let f = fixture(&db.pool, &caller).await?;
    let qa = environment(&caller, f.project, "qa").await?;
    caller
        .send_json(
            "POST",
            &format!("/api/v1/tasks/{}/promotions", f.task),
            &json!({ "environment_id": qa }),
        )
        .await?;

    // The environment defaults to the one the task is on — QA tests what was
    // pushed, and making them name it every time is ceremony.
    for note in ["still crashes on rotate", "crashes on cold start too"] {
        let (status, verdict) = caller
            .send_json(
                "POST",
                &format!("/api/v1/tasks/{}/verifications", f.task),
                &json!({ "verdict": "FAIL", "note": note }),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{verdict}");
        assert_eq!(verdict["environment_id"], qa.to_string());
    }
    caller
        .send_json(
            "POST",
            &format!("/api/v1/tasks/{}/verifications", f.task),
            &json!({ "verdict": "pass" }),
        )
        .await?;

    let (_, custody) = caller
        .get(&format!("/api/v1/tasks/{}/custody", f.task))
        .await?;
    let verdicts = custody["verifications"]
        .as_array()
        .context("verifications")?;
    // "Failed twice on qa, then passed" — the sentence a status column cannot
    // produce, because a status only ever holds the latest value.
    assert_eq!(verdicts.len(), 3, "{custody}");
    assert_eq!(verdicts[0]["verdict"], "PASS");
    assert_eq!(
        verdicts.iter().filter(|v| v["verdict"] == "FAIL").count(),
        2
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_verdict_needs_an_environment_and_a_bad_one_is_refused_early() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "qa@example.test", "acme").await?;
    let f = fixture(&db.pool, &caller).await?;

    // On no environment and none named: a verdict nobody can reproduce.
    let (status, body) = caller
        .send_json(
            "POST",
            &format!("/api/v1/tasks/{}/verifications", f.task),
            &json!({ "verdict": "PASS" }),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    // And a verdict outside the pair never reaches the enum column, where it
    // would surface as a 500 instead of a sentence.
    let (status, body) = caller
        .send_json(
            "POST",
            &format!("/api/v1/tasks/{}/verifications", f.task),
            &json!({ "verdict": "MAYBE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-VAL-0005");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn custody_of_an_invisible_task_is_404_and_never_403() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let owner = signed_in(&db.pool, "owner@example.test", "acme").await?;
    let f = fixture(&db.pool, &owner).await?;
    let elsewhere = signed_in(&db.pool, "other@example.test", "other").await?;

    let (status, body) = elsewhere
        .get(&format!("/api/v1/tasks/{}/custody", f.task))
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "TF-TSK-0001");
    Ok(())
}
