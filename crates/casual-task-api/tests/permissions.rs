//! `/api/v1/permissions/*` end to end, against a real PostgreSQL (C-003).
//!
//! `docs/04` calls `/permissions/explain` the answer to "why can't I close
//! this?", and the properties that make that answer worth having cannot be
//! observed from inside a handler:
//!
//! - **A constrained permission is reported, not dropped.** The effective set
//!   distinguishes "you may always" from "you may where the constraint holds",
//!   so the client neither renders a button that 403s nor hides a feature the
//!   actor has.
//! - **A grant can contribute and still not allow.** That pair — named grant,
//!   unsatisfied constraint — is the entire product of the endpoint.
//! - **It is not a permission oracle.** Explaining somebody else's authority
//!   discloses their grants, so it costs `role.manage`. Without that, a member
//!   could enumerate which colleague holds `workspace.delete`.
//! - **A subject from another workspace does not resolve.** Row-level security
//!   confines the reads, and the endpoint turns that into a 404 rather than an
//!   empty answer that reads like "they have nothing".

mod schema_harness;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::middleware::WORKSPACE_HEADER;
use casual_task_api::server::{AppState, router};
use casual_task_identity::password;
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";

struct Caller {
    user_id: Uuid,
    cookie: String,
    csrf: String,
}

fn app(pool: sqlx::PgPool) -> axum::Router {
    router(AppState {
        broadcast: casual_task_api::sse::local_hub(),
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: "a-test-secret-key-long-enough-for-hmac".into(),
        public_url: "https://tasks.example.test".into(),
        mailer: Arc::new(casual_task_infra::mail::LoggingMailer),
    })
}

async fn json_body(response: axum::response::Response) -> Result<Value> {
    let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn sign_up(app: &axum::Router, pool: &sqlx::PgPool, email: &str) -> Result<Caller> {
    let user_id = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        user_id,
        email,
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
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
    let body = json_body(response).await?;
    let csrf = body["csrf_token"].as_str().context("csrf")?.to_owned();
    Ok(Caller {
        user_id,
        cookie,
        csrf,
    })
}

fn request(caller: &Caller, workspace: Uuid, method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, &caller.cookie)
        .header("x-csrf-token", &caller.csrf)
        .header(WORKSPACE_HEADER, workspace.to_string())
}

/// A workspace the caller belongs to, created directly rather than through the
/// API: `POST /workspaces` grants the creator Owner (D-054), which holds every
/// permission and would make every assertion here trivially true.
async fn workspace_with(pool: &sqlx::PgPool, user: Uuid, slug: &str) -> Result<Uuid> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    test_support::add_workspace_member(pool, workspace, user).await?;
    Ok(workspace)
}

async fn effective(app: &axum::Router, caller: &Caller, workspace: Uuid) -> Result<Value> {
    let response = app
        .clone()
        .oneshot(
            request(caller, workspace, "GET", "/api/v1/permissions/effective")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "effective failed");
    json_body(response).await
}

async fn explain(
    app: &axum::Router,
    caller: &Caller,
    workspace: Uuid,
    body: Value,
) -> Result<axum::response::Response> {
    Ok(app
        .clone()
        .oneshot(
            request(caller, workspace, "POST", "/api/v1/permissions/explain")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))?,
        )
        .await?)
}

fn reach_of<'v>(body: &'v Value, permission: &str) -> Option<&'v str> {
    body["permissions"]
        .as_array()?
        .iter()
        .find(|p| p["permission"] == permission)?["reach"]
        .as_str()
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_member_with_no_grants_holds_nothing() -> Result<()> {
    // Migration 0003: "role_assignment is the ONLY source of authority".
    // Membership alone must not populate the effective set, or every client
    // renders every control for everyone.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "nobody@example.com").await?;
    let workspace = workspace_with(&db.pool, caller.user_id, "acme").await?;

    let body = effective(&app, &caller, workspace).await?;
    assert_eq!(
        body["permissions"].as_array().map(Vec::len),
        Some(0),
        "membership is not authority"
    );
    assert_eq!(body["actor_id"], caller.user_id.to_string());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_constrained_grant_is_conditional_and_an_unconstrained_one_is_not() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "member@example.com").await?;
    let workspace = workspace_with(&db.pool, caller.user_id, "acme").await?;

    test_support::grant_at_workspace(&db.pool, workspace, caller.user_id, &["task.read"]).await?;
    test_support::grant_at_workspace_constrained(
        &db.pool,
        workspace,
        caller.user_id,
        &["task.close"],
        json!({ "assignee_is_actor": true }),
    )
    .await?;

    let body = effective(&app, &caller, workspace).await?;
    assert_eq!(reach_of(&body, "task.read"), Some("unconditional"));
    assert_eq!(
        reach_of(&body, "task.close"),
        Some("conditional"),
        "a constrained permission must be reported, not dropped — otherwise \
         \"you may close tasks you are assigned to\" renders as \"you may not \
         close tasks\""
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_grant_can_contribute_and_still_not_allow() -> Result<()> {
    // The product of the endpoint: the named grant plus the unsatisfied
    // constraint. "No" on its own is what the endpoint exists to replace.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "member@example.com").await?;
    let workspace = workspace_with(&db.pool, caller.user_id, "acme").await?;
    test_support::grant_at_workspace_constrained(
        &db.pool,
        workspace,
        caller.user_id,
        &["task.close"],
        json!({ "assignee_is_actor": true }),
    )
    .await?;

    let response = explain(
        &app,
        &caller,
        workspace,
        json!({ "permission": "task.close", "resource": { "project_id": Uuid::now_v7() } }),
    )
    .await?;
    // The project does not exist, so the endpoint refuses before answering —
    // absent and invisible are indistinguishable (docs/04).
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // At workspace scope there is no project to hide, and the answer lands.
    let response = explain(&app, &caller, workspace, json!({ "permission": "task.close" })).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    assert_eq!(body["allowed"], false);
    assert_eq!(body["deny_reason"], "constraint_unsatisfied");
    let grants = body["contributing_grants"].as_array().context("grants")?;
    assert_eq!(grants.len(), 1, "the grant behind the refusal is named");
    assert_eq!(grants[0]["scope_type"], "WORKSPACE");
    assert_eq!(grants[0]["scope_id"], workspace.to_string());
    assert_eq!(grants[0]["constraints"][0], "assignee_is_actor");
    assert_eq!(grants[0]["constraints_satisfied"], false);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn holding_nothing_says_no_grant_rather_than_naming_one() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "member@example.com").await?;
    let workspace = workspace_with(&db.pool, caller.user_id, "acme").await?;

    let response = explain(&app, &caller, workspace, json!({ "permission": "task.close" })).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    assert_eq!(body["allowed"], false);
    assert_eq!(body["deny_reason"], "no_grant");
    assert_eq!(body["contributing_grants"].as_array().map(Vec::len), Some(0));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn explaining_someone_else_costs_role_manage() -> Result<()> {
    // Without this the endpoint is a permission oracle: any member could ask
    // who holds workspace.delete and get a target list back.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let nosy = sign_up(&app, &db.pool, "nosy@example.com").await?;
    let admin = sign_up(&app, &db.pool, "admin@example.com").await?;
    let workspace = workspace_with(&db.pool, nosy.user_id, "acme").await?;
    test_support::add_workspace_member(&db.pool, workspace, admin.user_id).await?;
    test_support::grant_at_workspace(&db.pool, workspace, admin.user_id, &["workspace.delete"])
        .await?;
    // The nosy member can read tasks, which is not the permission that governs
    // the grant graph.
    test_support::grant_at_workspace(&db.pool, workspace, nosy.user_id, &["task.read"]).await?;

    let response = explain(
        &app,
        &nosy,
        workspace,
        json!({ "actor_id": admin.user_id, "permission": "workspace.delete" }),
    )
    .await?;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "another member's grants are a disclosure, not a public fact"
    );

    // The same question from someone holding role.manage is answered.
    test_support::grant_at_workspace(&db.pool, workspace, nosy.user_id, &["role.manage"]).await?;
    let response = explain(
        &app,
        &nosy,
        workspace,
        json!({ "actor_id": admin.user_id, "permission": "workspace.delete" }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    assert_eq!(body["actor_id"], admin.user_id.to_string());
    assert_eq!(body["allowed"], true);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_subject_outside_the_workspace_is_not_found() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let admin = sign_up(&app, &db.pool, "admin@example.com").await?;
    let stranger = sign_up(&app, &db.pool, "stranger@example.com").await?;
    let workspace = workspace_with(&db.pool, admin.user_id, "acme").await?;
    test_support::grant_at_workspace(&db.pool, workspace, admin.user_id, &["role.manage"]).await?;

    // The stranger has an account and a workspace of their own, and holds a
    // grant there. Asking about them from this workspace must not reach it.
    let elsewhere = workspace_with(&db.pool, stranger.user_id, "other").await?;
    test_support::grant_at_workspace(&db.pool, elsewhere, stranger.user_id, &["workspace.delete"])
        .await?;

    let response = explain(
        &app,
        &admin,
        workspace,
        json!({ "actor_id": stranger.user_id, "permission": "workspace.delete" }),
    )
    .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a non-member resolves to not-found, never to an empty answer that \
         reads as \"they hold nothing\""
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_permission_key_this_build_does_not_know_is_refused() -> Result<()> {
    // Not an empty answer: "you do not have task.clsoe" is true and useless,
    // and a typo in an admin's debugging tool should say so.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "member@example.com").await?;
    let workspace = workspace_with(&db.pool, caller.user_id, "acme").await?;

    let response = explain(&app, &caller, workspace, json!({ "permission": "task.clsoe" })).await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await?;
    assert_eq!(body["error"]["code"], "TF-VAL-0005");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn neither_endpoint_answers_without_a_session() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let workspace = Uuid::now_v7();

    for (method, uri) in [
        ("GET", "/api/v1/permissions/effective"),
        ("POST", "/api/v1/permissions/explain"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(WORKSPACE_HEADER, workspace.to_string())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "permission": "task.read" }).to_string()))?,
            )
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{uri} answered an anonymous caller"
        );
    }
    Ok(())
}
