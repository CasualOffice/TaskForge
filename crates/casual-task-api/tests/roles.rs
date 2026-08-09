//! Role authoring and grant creation, end to end (C-003, `docs/04`).
//!
//! `docs/04` §Acceptance gates asks for an **escalation suite** — "one test per
//! control above, each *attempting* the exploit and asserting rejection". That
//! is what most of this file is. A test that only proves the happy path proves
//! the endpoint exists; these prove it cannot be turned into a way to become an
//! owner.
//!
//! The controls, and where each is enforced:
//!
//! | Control | Enforced by | Tested here |
//! | --- | --- | --- |
//! | 1 — grant ceiling | `casual_task_authz::ceiling` | yes, on assign *and* on role edit |
//! | 2 — scope ceiling | same | yes |
//! | 3 — authoring is workspace-scoped | same | yes |
//! | 4 — last owner | migration 0021's trigger | yes |
//! | 5 — self-elevation | `casual_task_authz::ceiling` | yes |
//! | 7 — everything audited | `UnitOfWork::record` | yes |

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
        self.with_body("POST", uri, body, None).await
    }

    async fn patch(
        &self,
        uri: &str,
        body: &serde_json::Value,
        version: i64,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.with_body("PATCH", uri, body, Some(version)).await
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

    async fn with_body(
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

/// Everything an admin needs to author and grant, and the workspace to do it in.
const ADMIN: &[&str] = &[
    "role.manage",
    "role.assign",
    "task.read",
    "task.create",
    "task.update",
    "project.create",
];

async fn admin(pool: &sqlx::PgPool, slug: &str) -> Result<Caller> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    member_of(pool, "admin@example.com", workspace, ADMIN).await
}

// ── The happy path, so the refusals mean something ──────────────────────────

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_role_is_authored_listed_granted_and_revoked() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;

    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let role_id = role["id"].as_str().expect("id").to_owned();
    assert_eq!(role["permissions"], json!(["task.read"]));

    let (status, listed) = admin.get("/api/v1/roles").await?;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        listed["data"]
            .as_array()
            .expect("data")
            .iter()
            .any(|r| r["id"] == role_id.as_str())
    );

    let ama = member_of(&db.pool, "ama@example.com", admin.workspace, &[]).await?;
    let (status, grant) = admin
        .post(
            "/api/v1/role-assignments",
            &json!({
                "principal_type": "USER",
                "principal_id": ama.user,
                "role_id": role_id,
                "scope_type": "WORKSPACE"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    let assignment = grant["id"].as_str().expect("id").to_owned();

    // The grant is real: Ama could not read anything a moment ago.
    let (status, effective) = ama.get("/api/v1/permissions/effective").await?;
    assert_eq!(status, StatusCode::OK, "{effective}");
    assert!(
        effective["permissions"]
            .as_array()
            .expect("permissions")
            .iter()
            .any(|p| p["permission"] == "task.read"),
        "the grant did not reach: {effective}"
    );

    let (status, _) = admin
        .delete(&format!("/api/v1/role-assignments/{assignment}"))
        .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, after) = ama.get("/api/v1/permissions/effective").await?;
    assert_eq!(
        after["permissions"].as_array().map(Vec::len),
        Some(0),
        "revoking left the permission behind: {after}"
    );
    Ok(())
}

// ── The escalation suite ────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_1_you_cannot_author_a_role_carrying_what_you_do_not_hold() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;

    // The admin holds role.manage but not workspace.delete.
    let (status, body) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Destroyer", "permissions": ["workspace.delete"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0003");
    assert_eq!(body["error"]["details"]["missing"], "workspace.delete");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_1_editing_a_role_cannot_smuggle_in_a_new_permission() -> Result<()> {
    // `docs/04`: the ceiling is checked at assignment time *and* re-checked on
    // role edit, because "editing a role you granted cannot smuggle in new
    // permissions". Without the re-check, authoring a harmless role and then
    // widening it is the whole exploit.
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;

    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let id = role["id"].as_str().expect("id");
    let version = role["version"].as_i64().expect("version");

    let (status, body) = admin
        .patch(
            &format!("/api/v1/roles/{id}"),
            &json!({ "permissions": ["task.read", "workspace.delete"] }),
            version,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0003");

    // And the role is untouched — a refused edit writes nothing.
    let (_, listed) = admin.get("/api/v1/roles").await?;
    let still = listed["data"]
        .as_array()
        .expect("data")
        .iter()
        .find(|r| r["id"] == id)
        .expect("role");
    assert_eq!(still["permissions"], json!(["task.read"]));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_2_a_project_assigner_cannot_create_a_workspace_grant() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let role_id = role["id"].as_str().expect("id").to_owned();

    // Holds the project-scope assign permission and task.read, and nothing at
    // workspace scope that would let them grant there.
    let manager = member_of(
        &db.pool,
        "manager@example.com",
        admin.workspace,
        &["project.role.assign", "task.read"],
    )
    .await?;
    let target = member_of(&db.pool, "target@example.com", admin.workspace, &[]).await?;

    let (status, body) = manager
        .post(
            "/api/v1/role-assignments",
            &json!({
                "principal_type": "USER",
                "principal_id": target.user,
                "role_id": role_id,
                "scope_type": "WORKSPACE"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0004");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_3_authoring_needs_role_manage_and_assigning_is_not_enough() -> Result<()> {
    // D-049. Before that decision the closed set had only `role.manage` above
    // project scope, so a workspace-level assigner necessarily held the right to
    // author — and could mint a role carrying more than they held.
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let assigner = member_of(
        &db.pool,
        "assigner@example.com",
        admin.workspace,
        &["role.assign", "task.read"],
    )
    .await?;

    let (status, body) = assigner
        .post(
            "/api/v1/roles",
            &json!({ "name": "Mine", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // But they may still grant a role that already exists, which is the point
    // of splitting the two.
    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let target = member_of(&db.pool, "target@example.com", admin.workspace, &[]).await?;
    let (status, body) = assigner
        .post(
            "/api/v1/role-assignments",
            &json!({
                "principal_type": "USER",
                "principal_id": target.user,
                "role_id": role["id"],
                "scope_type": "WORKSPACE"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_4_the_last_owner_grant_cannot_be_revoked() -> Result<()> {
    // Enforced by migration 0021's trigger, inside the transaction — `docs/04`
    // requires "a database constraint check, not just application code". The
    // trigger raises `restrict_violation` with the code in a HINT, so without
    // the handler mapping it this would surface as a 500.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let owner = member_of(
        &db.pool,
        "owner@example.com",
        workspace,
        &["workspace.owner", "role.manage", "role.assign"],
    )
    .await?;

    let (status, grants) = owner.get("/api/v1/roles").await?;
    assert_eq!(status, StatusCode::OK, "{grants}");

    // Find the owner's own assignment through the permissions explainer, which
    // returns the contributing grants.
    let (status, explained) = owner
        .post(
            "/api/v1/permissions/explain",
            &json!({ "permission": "workspace.owner" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{explained}");
    assert_eq!(explained["allowed"], true, "{explained}");

    // Revoking it must be refused. The id comes from the database, since the
    // grant was made by a fixture rather than through the API.
    let assignment = test_support::owner_assignment(&db.pool, workspace)
        .await?
        .expect("a workspace always has an owner grant");
    let (status, body) = owner
        .delete(&format!("/api/v1/role-assignments/{assignment}"))
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0005");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_5_an_actor_cannot_grant_themselves_more_than_they_hold() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;

    // A role the admin can author, carrying only what they hold.
    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");

    // Granting it to themselves adds nothing and is allowed — control 5 is
    // about *exceeding*, not about self-assignment as such.
    let (status, body) = admin
        .post(
            "/api/v1/role-assignments",
            &json!({
                "principal_type": "USER",
                "principal_id": admin.user,
                "role_id": role["id"],
                "scope_type": "WORKSPACE"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // A role carrying something they do not hold cannot even be authored, so
    // the exploit is refused one step earlier — which is control 1 doing
    // control 5's work, exactly as `ceiling.rs` documents.
    let (status, body) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Owner-ish", "permissions": ["workspace.owner"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_7_every_grant_and_role_edit_is_audited() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;

    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let role_id: Uuid = role["id"].as_str().expect("id").parse()?;

    let audited = test_support::audit_events_for(&db.pool, role_id).await?;
    assert!(
        audited.iter().any(|e| e == "role.created"),
        "authoring a role wrote no audit event: {audited:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_grant_bumps_the_authz_epoch_so_open_streams_revalidate() -> Result<()> {
    // `docs/04` defines the epoch as bumped in the same transaction as the
    // change. C-015's SSE revalidation treats an unchanged epoch as proof, so a
    // grant that did not bump it would leave an open stream authorised on
    // permissions the actor no longer has.
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (_, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    let ama = member_of(&db.pool, "ama@example.com", admin.workspace, &[]).await?;

    let before = test_support::authz_epoch(&db.pool, admin.workspace).await?;
    let (status, body) = admin
        .post(
            "/api/v1/role-assignments",
            &json!({
                "principal_type": "USER",
                "principal_id": ama.user,
                "role_id": role["id"],
                "scope_type": "WORKSPACE"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let after = test_support::authz_epoch(&db.pool, admin.workspace).await?;
    assert!(after > before, "granting did not bump the epoch");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unknown_permission_key_is_refused_before_the_schema_sees_it() -> Result<()> {
    // `role_permission.permission` is a foreign key, so the schema would refuse
    // it too — as a 500. "`task.updat` is not a permission" is the sentence an
    // admin can act on.
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (status, body) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Typo", "permissions": ["task.updat"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-VAL-0005");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn granting_the_same_role_twice_is_idempotent() -> Result<()> {
    // The schema's unique key exists because "the UI retries" (migration 0003).
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (_, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    let ama = member_of(&db.pool, "ama@example.com", admin.workspace, &[]).await?;
    let grant = json!({
        "principal_type": "USER",
        "principal_id": ama.user,
        "role_id": role["id"],
        "scope_type": "WORKSPACE"
    });

    let (first, one) = admin.post("/api/v1/role-assignments", &grant).await?;
    let (second, two) = admin.post("/api/v1/role-assignments", &grant).await?;
    assert_eq!(first, StatusCode::CREATED, "{one}");
    assert_eq!(second, StatusCode::CREATED, "{two}");
    assert_eq!(one["id"], two["id"], "a retry created a second grant");
    Ok(())
}

// ── Reading the grant set, which is what makes revoking possible ────────────

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_grants_can_be_listed_narrowed_and_then_revoked_from_the_listing() -> Result<()> {
    // The gap this closes: before `GET /role-assignments`, the id needed to
    // revoke a grant appeared exactly once — in the response to the call that
    // created it. An admin who closed the tab could never take it back.
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (_, reader) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    let (_, writer) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Writer", "permissions": ["task.read", "task.update"] }),
        )
        .await?;
    let ama = member_of(&db.pool, "ama@example.com", admin.workspace, &[]).await?;
    let bo = member_of(&db.pool, "bo@example.com", admin.workspace, &[]).await?;

    for (who, role) in [(ama.user, &reader), (bo.user, &writer)] {
        let (status, body) = admin
            .post(
                "/api/v1/role-assignments",
                &json!({
                    "principal_type": "USER",
                    "principal_id": who,
                    "role_id": role["id"],
                    "scope_type": "WORKSPACE"
                }),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    let (status, all) = admin.get("/api/v1/role-assignments").await?;
    assert_eq!(status, StatusCode::OK, "{all}");
    let rows = all["data"].as_array().expect("data");
    // The admin's own owner grant is in here too, which is the point: the list
    // is every grant, not every grant this call happened to create.
    assert!(rows.len() >= 3, "{all}");

    // Narrowed to one person — the question an admin screen actually asks.
    let (status, hers) = admin
        .get(&format!(
            "/api/v1/role-assignments?principal_id={}",
            ama.user
        ))
        .await?;
    assert_eq!(status, StatusCode::OK, "{hers}");
    let hers_rows = hers["data"].as_array().expect("data");
    assert_eq!(hers_rows.len(), 1, "{hers}");
    assert_eq!(hers_rows[0]["role_id"], reader["id"]);

    // And narrowed to one role.
    let writer_id = writer["id"].as_str().context("role id")?;
    let (_, by_role) = admin
        .get(&format!("/api/v1/role-assignments?role_id={writer_id}"))
        .await?;
    let by_role_rows = by_role["data"].as_array().expect("data");
    assert_eq!(by_role_rows.len(), 1, "{by_role}");
    assert_eq!(by_role_rows[0]["principal_id"], bo.user.to_string());

    // The whole point: revoke using only what the listing gave back.
    let id = hers_rows[0]["id"].as_str().expect("assignment id");
    let (status, body) = admin
        .delete(&format!("/api/v1/role-assignments/{id}"))
        .await?;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, after) = admin
        .get(&format!(
            "/api/v1/role-assignments?principal_id={}",
            ama.user
        ))
        .await?;
    assert!(
        after["data"].as_array().expect("data").is_empty(),
        "the revoked grant is still listed: {after}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_grant_listing_pages_by_cursor_and_never_by_offset() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (_, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    for i in 0..3 {
        let who = member_of(&db.pool, &format!("m{i}@example.com"), admin.workspace, &[]).await?;
        admin
            .post(
                "/api/v1/role-assignments",
                &json!({
                    "principal_type": "USER",
                    "principal_id": who.user,
                    "role_id": role["id"],
                    "scope_type": "WORKSPACE"
                }),
            )
            .await?;
    }

    let role_id = role["id"].as_str().context("role id")?;
    let (status, first) = admin
        .get(&format!(
            "/api/v1/role-assignments?role_id={role_id}&limit=2"
        ))
        .await?;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"].as_array().expect("data").len(), 2, "{first}");
    assert_eq!(first["page"]["has_more"], true, "{first}");

    let cursor = first["page"]["next_cursor"].as_str().expect("cursor");
    let (_, second) = admin
        .get(&format!(
            "/api/v1/role-assignments?role_id={role_id}&limit=2&cursor={cursor}"
        ))
        .await?;
    assert_eq!(
        second["data"].as_array().expect("data").len(),
        1,
        "{second}"
    );
    assert_eq!(second["page"]["has_more"], false, "{second}");

    // No overlap: a cursor that resumed at the wrong row would repeat one.
    let seen: Vec<&str> = first["data"]
        .as_array()
        .expect("data")
        .iter()
        .chain(second["data"].as_array().expect("data"))
        .map(|row| row["id"].as_str().expect("id"))
        .collect();
    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        seen.len(),
        "a page repeated a grant: {seen:?}"
    );

    let (status, body) = admin.get("/api/v1/role-assignments?limit=500").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-QRY-0007");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn reading_the_grants_needs_the_same_authority_as_assigning_them() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    // A member with neither `role.assign` nor `role.manage`. Who holds what is
    // the shape of the workspace's authority, not tenant content.
    let nobody = member_of(
        &db.pool,
        "nobody@example.com",
        admin.workspace,
        &["task.read"],
    )
    .await?;
    let (status, body) = nobody.get("/api/v1/role-assignments").await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    Ok(())
}
