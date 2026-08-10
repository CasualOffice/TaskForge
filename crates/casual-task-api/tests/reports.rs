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

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_report_groups_by_the_dimension_asked_for_and_keeps_the_null_slice() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let caller = signed_in(&db.pool, "lead@example.test", workspace, MEMBER).await?;

    let (status, project) = caller
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;

    for kind in ["BUG", "BUG", "FEATURE"] {
        let (status, made) = caller
            .send_json(
                "POST",
                &format!("/api/v1/projects/{project_id}/tasks"),
                &json!({ "title": "Something", "type": kind }),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{made}");
    }

    let (status, report) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(total_for(&report, Some("BUG")), 2, "{report}");
    assert_eq!(total_for(&report, Some("FEATURE")), 1, "{report}");
    assert_eq!(report["total"], 3, "{report}");

    // Nothing has been handed to a team yet, so every task is untriaged — and
    // that is the slice, not a gap in the data. A report that filtered out the
    // null group would hide the queue `docs/45` makes a place.
    let (status, by_team) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "team" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{by_team}");
    assert_eq!(total_for(&by_team, None), 3, "{by_team}");

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_report_counts_only_what_the_viewer_can_see() -> Result<()> {
    // `docs/38`: "aggregate numbers are not comparable between viewers. A
    // manager's '47 open' and a guest's '12 open' are both right." The failure
    // this pins is silent — a leaked row still produces a plausible number, and
    // nobody audits a total.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let owner = signed_in(&db.pool, "owner@example.test", workspace, MEMBER).await?;

    let (status, project) = owner
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": "PV", "name": "Private", "visibility": "PRIVATE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;
    for _ in 0..3 {
        owner
            .send_json(
                "POST",
                &format!("/api/v1/projects/{project_id}/tasks"),
                &json!({ "title": "Secret", "type": "BUG" }),
            )
            .await?;
    }

    let (status, mine) = owner
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(total_for(&mine, Some("BUG")), 3, "{mine}");

    // Another member of the same workspace, with the same permissions, who was
    // never added to a private project.
    let outsider = signed_in(&db.pool, "outsider@example.test", workspace, MEMBER).await?;
    let (status, theirs) = outsider
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{theirs}");
    assert_eq!(
        theirs["total"], 0,
        "a private project's tasks reached another member's report: {theirs}"
    );
    assert_eq!(theirs["scope"]["projects"], 0, "{theirs}");

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_filter_narrows_a_report_the_same_way_it_narrows_a_list() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let caller = signed_in(&db.pool, "lead@example.test", workspace, MEMBER).await?;

    let (_, project) = caller
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
        )
        .await?;
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;
    for (kind, priority) in [("BUG", "URGENT"), ("BUG", "LOW"), ("FEATURE", "URGENT")] {
        caller
            .send_json(
                "POST",
                &format!("/api/v1/projects/{project_id}/tasks"),
                &json!({ "title": "Something", "type": kind, "priority": priority }),
            )
            .await?;
    }

    let (status, urgent) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type", "filter": { "priority": "URGENT" } }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{urgent}");
    assert_eq!(urgent["total"], 2, "{urgent}");
    assert_eq!(total_for(&urgent, Some("BUG")), 1, "{urgent}");

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unbuilt_measure_and_an_unknown_dimension_are_both_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let caller = signed_in(&db.pool, "lead@example.test", workspace, MEMBER).await?;

    // Designed, scheduled, not built — and said so, rather than answered with a
    // count somebody would quote.
    let (status, refused) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "assignee", "measure": "cycle_time" }),
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{refused}");
    assert_eq!(refused["error"]["code"], "TF-SYS-0007", "{refused}");

    // And a dimension outside the closed set never reaches the compiler.
    let (status, bad) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "t.title) FROM task; --" }),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_state_projection_is_rebuilt_from_history_and_survives_redelivery() -> Result<()> {
    // The property the whole projection rests on. Outbox delivery is
    // at-least-once, so a consumer that appended an interval per event would
    // double a task's history the first time one was redelivered — and every
    // duration measure would be quietly wrong, with nothing on screen to say
    // so. Rebuilding from the audit stream is idempotent by construction, and
    // this is the assertion that says so.
    use casual_task_model::{WorkspaceId, WorkspaceScope};
    use casual_task_persistence::{Scoped, state_interval};

    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let caller = signed_in(&db.pool, "lead@example.test", workspace, MEMBER).await?;

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

    let scope = WorkspaceScope::for_job(WorkspaceId::from_uuid(workspace));
    let rebuild_once = || async {
        let mut tx = db.pool.begin().await.expect("begin");
        let mut scoped = Scoped::apply(&mut tx, &scope).await.expect("scope");
        state_interval::rebuild(&mut scoped, task_id)
            .await
            .expect("rebuild");
        let rows = state_interval::for_task(&mut scoped, task_id)
            .await
            .expect("read");
        tx.commit().await.expect("commit");
        rows
    };

    let first = rebuild_once().await;
    assert!(
        !first.is_empty(),
        "a created task has been somewhere, so it has at least one interval"
    );
    // Exactly one open interval: the task is in a state right now, and only
    // one. A second open row would double-count it in every aggregate, which
    // is why the schema makes it a unique index rather than a hope.
    assert_eq!(
        first.iter().filter(|row| row.exited_at.is_none()).count(),
        1,
        "{first:?}"
    );

    // The same delivery again, twice more. Converges or the projection is
    // unusable under the delivery guarantee it actually has.
    let second = rebuild_once().await;
    let third = rebuild_once().await;
    assert_eq!(first.len(), second.len(), "redelivery changed the series");
    assert_eq!(second.len(), third.len(), "redelivery changed the series");
    assert_eq!(
        third.iter().filter(|row| row.exited_at.is_none()).count(),
        1,
        "redelivery left more than one open interval: {third:?}"
    );

    Ok(())
}
