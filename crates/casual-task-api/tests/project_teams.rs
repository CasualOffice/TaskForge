//! A project involves many teams, end to end (`docs/03` §"Teams on a project").
//!
//! The change these tests exist for is a widening of the authorization scope
//! chain: `docs/04` makes a task's applicable scope set `{W, T, P, E}`, and with
//! several teams it becomes `{W, T₁…Tₙ, P, E}`. A grant scoped to **any** of a
//! project's teams reaches the task.
//!
//! Two properties matter and neither is observable from inside a handler:
//!
//! - **The widening is additive.** A grant on the second team reaches work it
//!   could not reach before, and no combining rule changed to make that true.
//! - **The single-team case is unchanged.** The whole product ran on one team
//!   per project; if that behaviour moved, every existing workspace moved with
//!   it.
//!
//! `TEAM` visibility means *any* of them, not all. "All" would hide a project
//! from the people added to it most recently, which is a rule nobody would
//! predict.

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
    user: Uuid,
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
        self.send(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, &self.cookie)
                .header("x-csrf-token", &self.csrf)
                .header("idempotency-key", Uuid::now_v7().to_string())
                .header(WORKSPACE_HEADER, self.workspace.to_string())
                .body(Body::from(body.to_string()))?,
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
        user,
    })
}

const OWNER: &[&str] = &[
    "project.create",
    "project.update",
    "project.member.manage",
    "task.create",
    "task.read",
];

/// An owner of a fresh workspace, and the workspace id.
async fn owner(pool: &sqlx::PgPool, slug: &str) -> Result<Caller> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    member_of(pool, "owner@example.com", workspace, OWNER).await
}

/// A `TEAM`-visible project with a task in it.
async fn team_visible_project(owner: &Caller) -> Result<(Uuid, Uuid)> {
    let (status, project) = owner
        .post(
            "/api/v1/projects",
            &json!({ "name": "Shared service", "key": "SVC", "visibility": "TEAM" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id: Uuid = project["id"].as_str().expect("id").parse()?;

    let (status, task) = owner
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &json!({ "title": "Shared work" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    Ok((project_id, task["id"].as_str().expect("id").parse()?))
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_grant_on_the_second_team_reaches_the_project() -> Result<()> {
    // The whole point. Before this change a project named one team, so a grant
    // on the other reached nothing and the only ways to include them were
    // WORKSPACE visibility — which shows the project to everyone — or a grant
    // per person, which is the administration the role model exists to avoid.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = owner(&db.pool, "acme").await?;
    let (project, task) = team_visible_project(&owner).await?;

    let platform = test_support::insert_team(&db.pool, owner.workspace, "Platform").await?;
    let product = test_support::insert_team(&db.pool, owner.workspace, "Product").await?;
    test_support::add_project_team(&db.pool, owner.workspace, project, platform).await?;
    test_support::add_project_team(&db.pool, owner.workspace, project, product).await?;

    // A member of the *second* team, with no workspace-scoped grant at all.
    let ama = member_of(&db.pool, "ama@example.com", owner.workspace, &[]).await?;
    test_support::add_team_member(&db.pool, product, ama.user).await?;
    test_support::grant_to_team_at_team_scope(
        &db.pool,
        owner.workspace,
        product,
        owner.user,
        &["task.read"],
    )
    .await?;

    let (status, body) = ama.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "a grant on the second team did not reach the task: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn team_visibility_means_any_team_not_all_of_them() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let owner = owner(&db.pool, "acme").await?;
    let (project, _) = team_visible_project(&owner).await?;

    let platform = test_support::insert_team(&db.pool, owner.workspace, "Platform").await?;
    let product = test_support::insert_team(&db.pool, owner.workspace, "Product").await?;
    test_support::add_project_team(&db.pool, owner.workspace, project, platform).await?;
    test_support::add_project_team(&db.pool, owner.workspace, project, product).await?;

    // In one team only. "All" would hide the project from whoever was added
    // most recently, which is a rule nobody would predict.
    let ama = member_of(&db.pool, "ama@example.com", owner.workspace, &[]).await?;
    test_support::add_team_member(&db.pool, platform, ama.user).await?;

    let (status, body) = ama.get("/api/v1/projects").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["data"]
            .as_array()
            .expect("data")
            .iter()
            .any(|p| p["id"] == project.to_string()),
        "a member of one of the project's teams cannot see it: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_member_of_none_of_the_teams_sees_nothing() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let owner = owner(&db.pool, "acme").await?;
    let (project, task) = team_visible_project(&owner).await?;

    let platform = test_support::insert_team(&db.pool, owner.workspace, "Platform").await?;
    test_support::add_project_team(&db.pool, owner.workspace, project, platform).await?;

    let stranger = member_of(&db.pool, "stranger@example.com", owner.workspace, &[]).await?;

    let (status, body) = stranger.get("/api/v1/projects").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body["data"]
            .as_array()
            .expect("data")
            .iter()
            .any(|p| p["id"] == project.to_string()),
        "a TEAM-visible project leaked to a non-member: {body}"
    );

    // And absent, not forbidden — `docs/04` requires the two to be
    // indistinguishable.
    let (status, _) = stranger.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn one_team_behaves_exactly_as_it_did_before() -> Result<()> {
    // The regression that would hurt most: every existing workspace has one
    // team per project, and if the widening moved that behaviour it moved all
    // of them.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = owner(&db.pool, "acme").await?;
    let (project, task) = team_visible_project(&owner).await?;

    let only = test_support::insert_team(&db.pool, owner.workspace, "Only").await?;
    test_support::add_project_team(&db.pool, owner.workspace, project, only).await?;

    let ama = member_of(&db.pool, "ama@example.com", owner.workspace, &[]).await?;
    test_support::add_team_member(&db.pool, only, ama.user).await?;
    test_support::grant_to_team_at_team_scope(
        &db.pool,
        owner.workspace,
        only,
        owner.user,
        &["task.read"],
    )
    .await?;

    let (status, body) = ama.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_project_with_no_teams_is_legal() -> Result<()> {
    // Teams are how a *group* gets access; they are not required for a project
    // to exist, and PRIVATE and WORKSPACE visibility are unaffected.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = owner(&db.pool, "acme").await?;
    let (status, project) = owner
        .post(
            "/api/v1/projects",
            &json!({ "name": "Solo", "key": "SOLO", "visibility": "WORKSPACE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(
        project["team_ids"].as_array().map(Vec::len),
        Some(0),
        "{project}"
    );

    let id = project["id"].as_str().expect("id");
    let (status, teams) = owner.get(&format!("/api/v1/projects/{id}/teams")).await?;
    assert_eq!(status, StatusCode::OK, "{teams}");
    assert_eq!(teams["data"].as_array().map(Vec::len), Some(0));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn adding_and_removing_a_team_changes_reach_and_bumps_the_epoch() -> Result<()> {
    // Adding or removing a team is an authorization change, so it must bump
    // `workspace.authz_epoch` in the same transaction — that counter is what
    // open SSE streams revalidate against (C-015).
    let db = schema_harness::TestDatabase::start().await?;
    let owner = owner(&db.pool, "acme").await?;
    let (project, task) = team_visible_project(&owner).await?;

    let product = test_support::insert_team(&db.pool, owner.workspace, "Product").await?;
    let ama = member_of(&db.pool, "ama@example.com", owner.workspace, &[]).await?;
    test_support::add_team_member(&db.pool, product, ama.user).await?;
    test_support::grant_to_team_at_team_scope(
        &db.pool,
        owner.workspace,
        product,
        owner.user,
        &["task.read"],
    )
    .await?;

    // Before: the team is not on the project, so the grant reaches nothing.
    let (before, _) = ama.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(
        before,
        StatusCode::NOT_FOUND,
        "reach before the team was added"
    );

    let epoch_before = test_support::authz_epoch(&db.pool, owner.workspace).await?;
    let (status, body) = owner
        .post(
            &format!("/api/v1/projects/{project}/teams"),
            &json!({ "team_id": product }),
        )
        .await?;
    assert!(status.is_success(), "{body}");
    let epoch_added = test_support::authz_epoch(&db.pool, owner.workspace).await?;
    assert!(
        epoch_added > epoch_before,
        "adding a team did not bump the epoch"
    );

    let (after, body) = ama.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(after, StatusCode::OK, "the grant did not reach: {body}");

    // Removing it takes the reach away again — that is the point, not a
    // side effect.
    let (status, body) = owner
        .delete(&format!("/api/v1/projects/{project}/teams/{product}"))
        .await?;
    assert!(status.is_success(), "{body}");
    let epoch_removed = test_support::authz_epoch(&db.pool, owner.workspace).await?;
    assert!(
        epoch_removed > epoch_added,
        "removing a team did not bump the epoch"
    );

    let (gone, _) = ama.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(gone, StatusCode::NOT_FOUND, "reach survived the removal");
    Ok(())
}
