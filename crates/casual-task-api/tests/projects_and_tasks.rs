//! Projects and tasks, end to end, against a real PostgreSQL (C-006, C-008).
//!
//! Two kinds of test live here and both are necessary. The happy path proves a
//! user can now do the thing the product exists for — create a project, create
//! a task in it, read both back. Everything else proves a refusal: another
//! workspace's project is a `404` and not a `403`, a write without `If-Match`
//! is refused rather than silently accepted, and a stale `If-Match` loses
//! rather than overwriting.
//!
//! Every test here fails without the code it covers. That is the bar, and it is
//! why the history assertions count rows rather than asserting a `201`: a create
//! that returned `201` and wrote no audit record would pass a status-code test
//! and violate ADR-006.

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
    metrics: Arc<Recorder>,
    cookie: String,
    csrf: String,
    workspace: Uuid,
    user: Uuid,
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

fn state(pool: sqlx::PgPool, metrics: Arc<Recorder>) -> AppState {
    AppState {
        storage: std::sync::Arc::new(casual_task_infra::FilesystemStore::new(
            std::env::temp_dir().join("tf-test-objects"),
            "https://files.example.test".to_owned(),
            "test-object-signing-secret".to_owned(),
        )),
        broadcast: casual_task_api::sse::local_hub(),
        pool,
        metrics,
        secret_key: SECRET.into(),
        public_url: "https://tasks.example.test".into(),
        mailer: Arc::new(casual_task_infra::mail::LoggingMailer),
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

    let metrics = Arc::new(Recorder::new());
    let app = router(state(pool.clone(), Arc::clone(&metrics)));
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
        metrics,
        cookie,
        csrf,
        workspace,
        user,
    })
}

/// The permissions a Member-shaped role carries for these endpoints.
const MEMBER: &[&str] = &[
    "project.create",
    "project.update",
    "task.create",
    "task.read",
];

fn key() -> String {
    Uuid::now_v7().to_string()
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_project_and_a_task_in_it_can_be_created_and_read_back() -> Result<()> {
    // The whole point of C-006 and C-008: before this, the product could log in
    // and nothing else.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let (status, project, etag) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["key"], "WR");
    assert_eq!(project["version"], 1);
    assert_eq!(etag.as_deref(), Some("\"1\""), "a create returns its ETag");
    let project_id = project["id"].as_str().expect("id").to_owned();

    let (status, task, etag) = caller
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "Ship the thing", "priority": "HIGH" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    // The task enters the default workflow's initial status, and the state it
    // maps to is written with it (docs/23).
    assert_eq!(task["state"], "BACKLOG");
    assert_eq!(
        task["key"], "WR-1",
        "the human key spans project and number"
    );
    assert_eq!(task["number"], 1);
    assert_eq!(task["priority"], "HIGH");
    assert_eq!(etag.as_deref(), Some("\"1\""));
    let task_id = task["id"].as_str().expect("id").to_owned();

    let (status, read_back, etag) = caller
        .get(&format!("/api/v1/projects/{project_id}"))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_back["id"], project["id"]);
    assert_eq!(etag.as_deref(), Some("\"1\""), "a read returns an ETag");

    let (status, read_back, etag) = caller.get(&format!("/api/v1/tasks/{task_id}")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_back["title"], "Ship the thing");
    assert_eq!(etag.as_deref(), Some("\"1\""));

    // And both appear in their lists.
    let (status, page, _) = caller.get("/api/v1/projects").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["data"][0]["id"], project["id"]);
    assert_eq!(page["page"]["has_more"], false);

    let (status, page, _) = caller.get("/api/v1/tasks").await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["data"][0]["id"], task["id"]);
    assert_eq!(page["data"][0]["key"], "WR-1");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_create_writes_its_activity_audit_and_outbox_rows_in_the_same_transaction() -> Result<()>
{
    // ADR-006. Without this, a create that returned 201 and wrote no history
    // would pass every other test in this file.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    let project_id: Uuid = project["id"].as_str().expect("id").parse()?;

    let (activity, audit, outbox, deliveries) =
        test_support::history_counts(&db.pool, project_id).await?;
    assert_eq!(activity, 1, "no activity row for the project create");
    assert_eq!(audit, 1, "no audit row for the project create");
    assert_eq!(outbox, 1, "no outbox event for the project create");
    assert_eq!(
        deliveries,
        i64::try_from(casual_task_persistence::CONSUMERS.len()).expect("consumer count"),
        "one delivery row per consumer, written in the producing transaction"
    );
    assert_eq!(
        test_support::outbox_event_types(&db.pool, project_id).await?,
        vec!["project.created".to_owned()],
        "docs/25 names the event type"
    );

    let (_, task, _) = caller
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "t" }),
            Some(&key()),
        )
        .await?;
    let task_id: Uuid = task["id"].as_str().expect("id").parse()?;
    let (activity, audit, outbox, _) = test_support::history_counts(&db.pool, task_id).await?;
    assert_eq!((activity, audit, outbox), (1, 1, 1));
    assert_eq!(
        test_support::outbox_event_types(&db.pool, task_id).await?,
        vec!["task.created".to_owned()]
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_project_in_another_workspace_is_404_and_never_403() -> Result<()> {
    // docs/04: absent and invisible are never disambiguated. A 403 here would
    // confirm the project exists, which is how project ids get enumerated —
    // and the ids are in every task key the other tenant publishes.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = caller(&db.pool, "owner@example.com", "acme", MEMBER).await?;
    let stranger = caller(&db.pool, "stranger@example.com", "other", MEMBER).await?;

    let (_, project, _) = owner
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
            Some(&key()),
        )
        .await?;
    let project_id = project["id"].as_str().expect("id").to_owned();

    let (real, _, _) = stranger
        .get(&format!("/api/v1/projects/{project_id}"))
        .await?;
    let (imaginary, _, _) = stranger
        .get(&format!("/api/v1/projects/{}", Uuid::now_v7()))
        .await?;
    assert_eq!(real, StatusCode::NOT_FOUND);
    assert_eq!(
        real, imaginary,
        "a project in another workspace is distinguishable from one that does \
         not exist"
    );

    // And it is absent from the stranger's list, rather than merely unreadable.
    let (status, page, _) = stranger.get("/api/v1/projects").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["data"].as_array().map(Vec::len), Some(0));

    // The task in it is invisible for the same reason.
    let (_, task, _) = owner
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "secret" }),
            Some(&key()),
        )
        .await?;
    let task_id = task["id"].as_str().expect("id");
    let (status, _, _) = stranger.get(&format!("/api/v1/tasks/{task_id}")).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Creating into someone else's project is the same answer, not a 403.
    let (status, _, _) = stranger
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "intruder" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_private_project_is_invisible_to_a_fellow_member() -> Result<()> {
    // The other half of docs/04's visibility rule, inside one workspace. A
    // workspace-scoped grant does not confer visibility of a private project —
    // that is how "Member everywhere except this one project" is expressed.
    let db = schema_harness::TestDatabase::start().await?;
    let author = caller(&db.pool, "author@example.com", "acme", MEMBER).await?;
    // A second member of the SAME workspace, holding every permission these
    // endpoints use.
    let colleague = member_of(&db.pool, "colleague@example.com", author.workspace, MEMBER).await?;

    let (_, private, _) = author
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "SEC", "name": "Secret", "visibility": "PRIVATE" }),
            Some(&key()),
        )
        .await?;
    let private_id = private["id"].as_str().expect("id").to_owned();

    // The author can see it: creating something you cannot read back is a bug.
    let (status, _, _) = author
        .get(&format!("/api/v1/projects/{private_id}"))
        .await?;
    assert_eq!(status, StatusCode::OK);

    // The colleague holds every permission and still cannot see it.
    let (status, _, _) = colleague
        .get(&format!("/api/v1/projects/{private_id}"))
        .await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a PRIVATE project was visible to a workspace member who is not in it"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_patch_without_if_match_is_428_and_a_stale_one_is_409() -> Result<()> {
    // docs/05: 428 rather than silently accepting an unconditional write, and
    // 409 rather than the silent overwrite ADR-023 exists to prevent. Both are
    // easy to lose, and losing either is invisible until someone's edit
    // vanishes in production.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    let uri = format!("/api/v1/projects/{}", project["id"].as_str().expect("id"));

    let (status, body, _) = caller
        .patch(&uri, &serde_json::json!({ "name": "Renamed" }), None)
        .await?;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["error"]["code"], "TF-CNC-0002");

    // The current version is 1, so 7 is stale.
    let (status, body, _) = caller
        .patch(
            &uri,
            &serde_json::json!({ "name": "Renamed" }),
            Some("\"7\""),
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "TF-CNC-0001");
    assert_eq!(body["error"]["details"]["your_version"], 7);
    assert_eq!(body["error"]["details"]["current_version"], 1);
    assert_eq!(
        body["error"]["details"]["current"]["name"], "Work",
        "docs/24: the conflict body carries the current representation so the \
         client can show what changed"
    );

    // The refused writes changed nothing.
    let (_, unchanged, _) = caller.get(&uri).await?;
    assert_eq!(unchanged["name"], "Work");
    assert_eq!(unchanged["version"], 1);

    // And the correct tag succeeds, bumping the version.
    let (status, updated, etag) = caller
        .patch(
            &uri,
            &serde_json::json!({ "name": "Renamed" }),
            Some("\"1\""),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "Renamed");
    assert_eq!(updated["version"], 2);
    assert_eq!(etag.as_deref(), Some("\"2\""));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_patch_cannot_change_the_key_and_can_clear_a_description() -> Result<()> {
    // ADR-007 makes the key immutable, and docs/05 §Conventions makes `null`
    // mean "clear" while absent means "leave alone". Both are one-line rules
    // that a PATCH implementation gets wrong by default.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work", "description": "notes" }),
            Some(&key()),
        )
        .await?;
    let uri = format!("/api/v1/projects/{}", project["id"].as_str().expect("id"));

    let (status, body, _) = caller
        .patch(&uri, &serde_json::json!({ "key": "OPS" }), Some("\"1\""))
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-PRJ-0003");

    // An empty patch leaves the description alone.
    let (status, unchanged, _) = caller
        .patch(&uri, &serde_json::json!({}), Some("\"1\""))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unchanged["description"], "notes");

    // An explicit null clears it.
    let (status, cleared, _) = caller
        .patch(
            &uri,
            &serde_json::json!({ "description": null }),
            Some("\"2\""),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert!(cleared["description"].is_null());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn authority_comes_from_a_grant_and_nowhere_else() -> Result<()> {
    // migration 0003: "role_assignment is the ONLY source of authority in the
    // system. No permission is granted anywhere else — not by a boolean column,
    // not by an is_admin flag, and not by project membership."
    let db = schema_harness::TestDatabase::start().await?;
    let ungranted = caller(&db.pool, "member@example.com", "acme", &[]).await?;

    let (status, body, _) = ungranted
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0001");

    // Reading is not blocked by the same rule: docs/04 gives visibility an
    // implicit read grant, so a member with no grants still sees the workspace.
    let (status, page, _) = ungranted.get("/api/v1/projects").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["data"].as_array().map(Vec::len), Some(0));

    // A grant that carries project.create but not task.create authorizes one
    // and refuses the other — the resolver is consulted per permission, not
    // per endpoint family.
    let partial = caller(&db.pool, "partial@example.com", "beta", &["project.create"]).await?;
    let (status, project, _) = partial
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let (status, body, _) = partial
        .post(
            &format!(
                "/api/v1/projects/{}/tasks",
                project["id"].as_str().expect("id")
            ),
            &serde_json::json!({ "title": "t" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0001");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_duplicate_key_is_409_and_a_malformed_one_is_400() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let (status, _, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Other" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "TF-PRJ-0002");

    for bad in ["wr", "W", "W-R", "TOOLONGAKEY1"] {
        let (status, body, _) = caller
            .post(
                "/api/v1/projects",
                &serde_json::json!({ "key": bad, "name": "x" }),
                Some(&key()),
            )
            .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} was accepted");
        assert_eq!(body["error"]["code"], "TF-PRJ-0004");
    }

    // An unknown field is a 400 and names itself, rather than being ignored.
    let (status, body, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "OPS", "name": "x", "visibilty": "TEAM" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-VAL-0002");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_retried_create_returns_the_first_response_rather_than_a_second_task() -> Result<()> {
    // docs/24: "a timeout that actually succeeded produces a duplicate task,
    // and the user has no way to tell". The key is what makes the retry safe,
    // and the request hash is what catches the client that reuses one key for
    // two different tasks.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;
    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    let uri = format!(
        "/api/v1/projects/{}/tasks",
        project["id"].as_str().expect("id")
    );
    let idempotency = key();
    let body = serde_json::json!({ "title": "Ship it" });

    let (status, first, _) = caller.post(&uri, &body, Some(&idempotency)).await?;
    assert_eq!(status, StatusCode::CREATED);
    let (status, replay, _) = caller.post(&uri, &body, Some(&idempotency)).await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(replay["id"], first["id"], "the retry created a second task");

    let (_, page, _) = caller.get("/api/v1/tasks").await?;
    assert_eq!(page["data"].as_array().map(Vec::len), Some(1));

    // The same key with a different body is the client bug docs/24 names.
    let (status, body, _) = caller
        .post(
            &uri,
            &serde_json::json!({ "title": "Something else" }),
            Some(&idempotency),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-IDM-0002");

    // And a create with no key at all is refused: docs/05 requires one.
    let (status, body, _) = caller
        .post(&uri, &serde_json::json!({ "title": "x" }), None)
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-IDM-0003");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_list_pages_by_cursor_and_never_repeats_or_skips_a_row() -> Result<()> {
    // docs/26 bans OFFSET because it "duplicates or skips rows under concurrent
    // writes". This asserts the keyset actually works — the second page is a
    // real query against a real database, which is where a cursor whose type
    // cast is missing fails and no unit test can see it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;
    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    let uri = format!(
        "/api/v1/projects/{}/tasks",
        project["id"].as_str().expect("id")
    );
    for n in 0..5 {
        let (status, body, _) = caller
            .post(
                &uri,
                &serde_json::json!({ "title": format!("task {n}") }),
                Some(&key()),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    let mut seen: Vec<String> = Vec::new();
    let mut next: Option<String> = None;
    for _ in 0..5 {
        let uri = next.map_or_else(
            || "/api/v1/tasks?limit=2".to_owned(),
            |c| format!("/api/v1/tasks?limit=2&cursor={c}"),
        );
        let (status, page, _) = caller.get(&uri).await?;
        assert_eq!(status, StatusCode::OK, "{page}");
        for row in page["data"].as_array().expect("data").iter() {
            seen.push(row["id"].as_str().expect("id").to_owned());
        }
        next = page["page"]["next_cursor"].as_str().map(ToOwned::to_owned);
        if next.is_none() {
            break;
        }
    }

    assert_eq!(seen.len(), 5, "paging saw {} of 5 tasks", seen.len());
    let unique: std::collections::HashSet<&String> = seen.iter().collect();
    assert_eq!(unique.len(), 5, "a row was served twice: {seen:?}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_full_task_page_resolves_authority_once() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;
    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    let project_id = project["id"].as_str().expect("id").parse()?;
    test_support::insert_task_page(&db.pool, caller.workspace, project_id, caller.user, 100)
        .await?;

    let before = metric_count(
        &caller.metrics.render(),
        "authz_resolution_duration_count{outcome=\"cache_miss\"}",
    );
    let (status, body, _) = caller.get("/api/v1/tasks?limit=100").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"].as_array().expect("data").len(), 100);
    let after = metric_count(
        &caller.metrics.render(),
        "authz_resolution_duration_count{outcome=\"cache_miss\"}",
    );
    assert_eq!(
        after - before,
        1,
        "one list page must perform one authorization resolution"
    );
    Ok(())
}

fn metric_count(rendered: &str, series: &str) -> u64 {
    rendered
        .lines()
        .find_map(|line| {
            line.strip_prefix(series)
                .and_then(|value| value.trim().parse().ok())
        })
        .unwrap_or(0)
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_page_size_and_the_query_parameters_are_bounded() -> Result<()> {
    // docs/26 §Query limits caps a page at 100. Clamping instead of refusing
    // would tell a client that asked for 500 there were only 100 rows.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    for uri in ["/api/v1/tasks?limit=101", "/api/v1/projects?limit=101"] {
        let (status, body, _) = caller.get(uri).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(body["error"]["code"], "TF-QRY-0007");
    }
    // TF-QRY-0001, not the generic TF-VAL-0002: since C-012 the list endpoint
    // reads unrecognised query parameters as filter fields, so a typo'd `limit`
    // genuinely *is* an unknown filter field, and that code's docs URL points at
    // the grammar the client needs. The property docs/05 requires is unchanged —
    // the typo is refused rather than silently ignored.
    let (status, body, _) = caller.get("/api/v1/tasks?limt=10").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-QRY-0001");
    // And it names the key that was wrong: "one of your parameters is unknown"
    // makes a client bisect its own query string.
    assert_eq!(body["error"]["details"]["field"], "limt", "{body}");

    let (status, body, _) = caller.get("/api/v1/tasks?cursor=!!!nonsense!!!").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-QRY-0006");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_tenant_route_refuses_a_request_with_no_workspace() -> Result<()> {
    // The structural rule: every one of these takes `WorkspaceMember`, which is
    // the only thing that mints an AuthContext. Without a workspace header
    // there is no membership to validate, and docs/04 makes that a 404 rather
    // than a 400 so the header cannot be probed.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;
    let id = Uuid::now_v7();

    for uri in [
        "/api/v1/projects".to_owned(),
        format!("/api/v1/projects/{id}"),
        "/api/v1/tasks".to_owned(),
        format!("/api/v1/tasks/{id}"),
    ] {
        let response = caller
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&uri)
                    .header(header::COOKIE, &caller.cookie)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }

    // And with no credential at all, 401 — before any tenant row is touched.
    let response = caller
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/projects")
                .header(WORKSPACE_HEADER, caller.workspace.to_string())
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_create_without_a_csrf_token_is_refused() -> Result<()> {
    // The new routes are registered BEFORE the layers in server.rs. If one were
    // appended after `.layer()` it would escape the CSRF guard entirely, and
    // nothing else in the suite would notice.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let response = caller
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/projects")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, &caller.cookie)
                .header(WORKSPACE_HEADER, caller.workspace.to_string())
                .header("idempotency-key", key())
                .body(Body::from(
                    serde_json::json!({ "key": "WR", "name": "Work" }).to_string(),
                ))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a state-changing request succeeded with only a session cookie"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_first_project_in_a_workspace_brings_the_default_workflow_with_it() -> Result<()> {
    // docs/23: the default workflow "works with zero configuration". Nothing
    // else creates one, so a project create in a fresh workspace either
    // provisions it or fails — and the second project must reuse it rather than
    // making another.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let mut workflows = Vec::new();
    for project_key in ["WR", "OPS"] {
        let (status, project, _) = caller
            .post(
                "/api/v1/projects",
                &serde_json::json!({ "key": project_key, "name": project_key }),
                Some(&key()),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{project}");
        workflows.push(project["workflow_id"].as_str().expect("id").to_owned());
    }
    assert_eq!(
        workflows[0], workflows[1],
        "the second project created a second default workflow"
    );

    let statuses = test_support::workflow_status_names(&db.pool, workflows[0].parse()?).await?;
    assert_eq!(
        statuses,
        vec![
            "Backlog".to_owned(),
            "Todo".to_owned(),
            "In Progress".to_owned(),
            "Blocked".to_owned(),
            "Done".to_owned(),
            "Canceled".to_owned(),
        ],
        "the default workflow is not the one docs/23 draws"
    );
    Ok(())
}
