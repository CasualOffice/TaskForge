//! Workspaces, membership and teams, end to end (C-002).
//!
//! Everything here goes through `router()` rather than through a handler
//! directly, because half of what is being asserted lives in the layers: the
//! CSRF guard, the request id, and the workspace resolution that happens in the
//! extractor before a handler runs. A test that called a handler would prove
//! the handler works and nothing about whether the route is reachable the way a
//! client reaches it.

mod schema_harness;

use std::sync::Arc;

use anyhow::{Context, Result};
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
const SECRET: &str = "a-test-secret-key-long-enough-for-hmac";

/// A signed-in caller: everything a request needs to be accepted.
#[derive(Debug, Clone)]
struct Caller {
    user_id: Uuid,
    cookie: String,
    csrf: String,
}

fn app(pool: sqlx::PgPool) -> axum::Router {
    router(AppState {
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: SECRET.into(),
        public_url: "https://tasks.example.test".into(),
        mailer: Arc::new(casual_task_infra::mail::LoggingMailer),
    })
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
    let csrf = body["csrf_token"]
        .as_str()
        .context("csrf token")?
        .to_owned();

    Ok(Caller {
        user_id,
        cookie,
        csrf,
    })
}

/// A request builder pre-loaded with the caller's credentials.
fn request(caller: &Caller, method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, &caller.cookie)
        .header("x-csrf-token", &caller.csrf)
}

async fn send(app: &axum::Router, request: Request<Body>) -> Result<axum::response::Response> {
    Ok(app.clone().oneshot(request).await?)
}

async fn json_body(response: axum::response::Response) -> Result<Value> {
    let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Create a workspace and return `(id, etag)`.
async fn create_workspace(
    app: &axum::Router,
    caller: &Caller,
    slug: &str,
) -> Result<(Uuid, String)> {
    let response = send(
        app,
        request(caller, "POST", "/api/v1/workspaces")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": format!("Workspace {slug}"), "slug": slug }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CREATED, "create failed");
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .context("no ETag on a created workspace")?
        .to_owned();
    let body = json_body(response).await?;
    let id = body["id"].as_str().context("id")?.parse()?;
    Ok((id, etag))
}

// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn creating_a_workspace_makes_the_creator_a_member() -> Result<()> {
    // The unblocking property: without it a signed-in user has no workspace and
    // nothing else in the product is reachable.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;

    let (workspace, _) = create_workspace(&app, &caller, "acme").await?;

    let read = send(
        &app,
        request(&caller, "GET", &format!("/api/v1/workspaces/{workspace}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(read.status(), StatusCode::OK, "the creator cannot read it");
    let body = json_body(read).await?;
    assert_eq!(body["slug"], "acme");
    assert_eq!(body["name"], "Workspace acme");
    // docs/05 §Conventions: RFC 3339, always UTC, always Z.
    assert!(
        body["created_at"]
            .as_str()
            .unwrap_or_default()
            .ends_with('Z'),
        "created_at is not UTC-with-Z: {body}"
    );

    let members = json_body(
        send(
            &app,
            request(
                &caller,
                "GET",
                &format!("/api/v1/workspaces/{workspace}/members"),
            )
            .body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(members["data"][0]["user_id"], caller.user_id.to_string());
    assert_eq!(members["data"][0]["member_type"], "MEMBER");

    let mine = json_body(
        send(
            &app,
            request(&caller, "GET", "/api/v1/workspaces").body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(mine["data"][0]["id"], workspace.to_string());
    assert_eq!(mine["page"]["has_more"], false);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_non_member_is_told_404_and_not_403() -> Result<()> {
    // docs/04: absent and invisible are never disambiguated. A 403 on a real
    // workspace and a 404 on an imaginary one is how workspace ids get
    // enumerated by an authenticated stranger.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.test").await?;
    let stranger = sign_up(&app, &db.pool, "stranger@example.test").await?;
    let (workspace, _) = create_workspace(&app, &owner, "private").await?;

    let imaginary = Uuid::now_v7();
    for (label, id) in [("real", workspace), ("imaginary", imaginary)] {
        for path in [
            format!("/api/v1/workspaces/{id}"),
            format!("/api/v1/workspaces/{id}/members"),
            format!("/api/v1/workspaces/{id}/teams"),
        ] {
            let response =
                send(&app, request(&stranger, "GET", &path).body(Body::empty())?).await?;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{label} {path} answered something other than 404"
            );
            let body = json_body(response).await?;
            assert_eq!(
                body["error"]["code"], "TF-AZN-0008",
                "{label} {path} used a distinguishable error code"
            );
        }
    }

    // And the stranger's own list does not mention it.
    let mine = json_body(
        send(
            &app,
            request(&stranger, "GET", "/api/v1/workspaces").body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(mine["data"].as_array().map(Vec::len), Some(0));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn removing_a_member_stops_their_access_on_the_very_next_request() -> Result<()> {
    // Revocation that takes effect "eventually" is a permission hole with a
    // schedule. The membership check runs on every request precisely so this
    // holds without anything being invalidated first.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.test").await?;
    let guest = sign_up(&app, &db.pool, "guest@example.test").await?;
    let (workspace, _) = create_workspace(&app, &owner, "shared").await?;

    let added = send(
        &app,
        request(
            &owner,
            "POST",
            &format!("/api/v1/workspaces/{workspace}/members"),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "user_id": guest.user_id }).to_string()))?,
    )
    .await?;
    assert_eq!(added.status(), StatusCode::CREATED);

    let before = send(
        &app,
        request(&guest, "GET", &format!("/api/v1/workspaces/{workspace}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(before.status(), StatusCode::OK, "a new member was refused");

    let removed = send(
        &app,
        request(
            &owner,
            "DELETE",
            &format!("/api/v1/workspaces/{workspace}/members/{}", guest.user_id),
        )
        .body(Body::empty())?,
    )
    .await?;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    let after = send(
        &app,
        request(&guest, "GET", &format!("/api/v1/workspaces/{workspace}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(
        after.status(),
        StatusCode::NOT_FOUND,
        "a removed member kept their access"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unknown_json_field_is_refused_with_400() -> Result<()> {
    // docs/05 §Conventions: unknown request fields are "rejected with 400 —
    // silently ignoring a typo'd field is how clients ship bugs that look like
    // server bugs". axum's own Json rejection is a 422 with a bare text body,
    // which is why this endpoint does not use it.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;

    let response = send(
        &app,
        request(&caller, "POST", "/api/v1/workspaces")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": "Acme", "slug": "acme", "plan": "enterprise" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await?;
    assert_eq!(body["error"]["code"], "TF-VAL-0002");
    assert_eq!(body["error"]["details"]["unknown_fields"][0], "plan");
    assert!(
        !body["error"]["request_id"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "the envelope lost its request id: {body}"
    );

    // The same discipline on query parameters.
    let (workspace, _) = create_workspace(&app, &caller, "acme").await?;
    let response = send(
        &app,
        request(
            &caller,
            "GET",
            &format!("/api/v1/workspaces/{workspace}/members?limti=2"),
        )
        .body(Body::empty())?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_rename_is_conditional_on_the_version_the_caller_read() -> Result<()> {
    // docs/05 §Concurrency: 428 without If-Match, 409 against a stale one. A
    // rename that silently accepted an unconditional write would lose the other
    // editor's change without anyone being told.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;
    let (workspace, etag) = create_workspace(&app, &caller, "acme").await?;
    let uri = format!("/api/v1/workspaces/{workspace}");

    let unconditional = send(
        &app,
        request(&caller, "PATCH", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "name": "Renamed" }).to_string()))?,
    )
    .await?;
    assert_eq!(
        unconditional.status(),
        StatusCode::PRECONDITION_REQUIRED,
        "an unconditional PATCH was accepted"
    );

    let ok = send(
        &app,
        request(&caller, "PATCH", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_MATCH, &etag)
            .body(Body::from(json!({ "name": "Renamed" }).to_string()))?,
    )
    .await?;
    assert_eq!(ok.status(), StatusCode::OK);
    let next_etag = ok
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .context("etag")?
        .to_owned();
    assert_ne!(next_etag, etag, "the version did not move");
    assert_eq!(json_body(ok).await?["name"], "Renamed");

    let stale = send(
        &app,
        request(&caller, "PATCH", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_MATCH, &etag)
            .body(Body::from(json!({ "name": "Again" }).to_string()))?,
    )
    .await?;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let body = json_body(stale).await?;
    assert_eq!(body["error"]["code"], "TF-CNC-0001");
    // docs/24: the loser is told what it lost to, so it can re-read and merge.
    assert!(
        body["error"]["details"]["current_version"].is_number(),
        "{body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_mutation_writes_its_history_in_the_same_transaction() -> Result<()> {
    // ADR-006, docs/25: the domain change, its activity row, its audit row and
    // its outbox event commit together or not at all. A membership change with
    // no audit row is exactly what UnitOfWork exists to prevent.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.test").await?;
    let guest = sign_up(&app, &db.pool, "guest@example.test").await?;
    let (workspace, _) = create_workspace(&app, &owner, "acme").await?;

    send(
        &app,
        request(
            &owner,
            "POST",
            &format!("/api/v1/workspaces/{workspace}/members"),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "user_id": guest.user_id }).to_string()))?,
    )
    .await?;
    send(
        &app,
        request(
            &owner,
            "DELETE",
            &format!("/api/v1/workspaces/{workspace}/members/{}", guest.user_id),
        )
        .body(Body::empty())?,
    )
    .await?;

    let history = test_support::history(&db.pool, workspace).await?;
    let expected = vec![
        "workspace.created".to_owned(),
        "workspace.member.added".to_owned(),
        "workspace.member.removed".to_owned(),
    ];
    assert_eq!(history.activity, expected, "activity stream");
    assert_eq!(history.audit, expected, "audit stream");
    assert_eq!(history.outbox, expected, "outbox");
    assert_eq!(
        history.deliveries,
        i64::try_from(expected.len() * casual_task_persistence::CONSUMERS.len())?,
        "one delivery row per consumer per event (docs/25 §Consumer fan-out)"
    );

    // docs/04 §Caching: the epoch moves with every membership change, in the
    // same transaction, so a stale permission-cache entry simply misses.
    assert_eq!(
        test_support::authz_epoch(&db.pool, workspace).await?,
        3,
        "authz_epoch did not move with the two membership changes"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_workspace_cannot_lose_its_last_member() -> Result<()> {
    // Nothing can see a workspace with no members, so nothing can add one back
    // to it. Refusing is the only outcome that does not silently destroy data.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.test").await?;
    let (workspace, _) = create_workspace(&app, &owner, "acme").await?;

    let response = send(
        &app,
        request(
            &owner,
            "DELETE",
            &format!("/api/v1/workspaces/{workspace}/members/{}", owner.user_id),
        )
        .body(Body::empty())?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json_body(response).await?["error"]["code"], "TF-PRJ-0006");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn teams_cannot_be_reached_or_populated_across_a_tenant_boundary() -> Result<()> {
    // `team_membership` carries no workspace_id and therefore no RLS policy
    // (migration 0010). Both halves of its tenant boundary are asserted here:
    // the team must be visible in the caller's workspace, and the user must be
    // a member of it.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let alice = sign_up(&app, &db.pool, "alice@example.test").await?;
    let bob = sign_up(&app, &db.pool, "bob@example.test").await?;
    let (alpha, _) = create_workspace(&app, &alice, "alpha").await?;
    let (beta, _) = create_workspace(&app, &bob, "beta").await?;

    let created = send(
        &app,
        request(&alice, "POST", &format!("/api/v1/workspaces/{alpha}/teams"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "name": "Platform" }).to_string()))?,
    )
    .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let team: Uuid = json_body(created).await?["id"]
        .as_str()
        .context("team id")?
        .parse()?;

    // Bob is a member of beta, and points a beta-scoped request at alpha's team.
    let across = send(
        &app,
        request(&bob, "POST", &format!("/api/v1/teams/{team}/members"))
            .header(WORKSPACE_HEADER, beta.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": bob.user_id }).to_string()))?,
    )
    .await?;
    assert_eq!(
        across.status(),
        StatusCode::NOT_FOUND,
        "a team in another tenant was reachable"
    );

    // Alice can reach her own team, but not put a stranger in it.
    let stranger = send(
        &app,
        request(&alice, "POST", &format!("/api/v1/teams/{team}/members"))
            .header(WORKSPACE_HEADER, alpha.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": bob.user_id }).to_string()))?,
    )
    .await?;
    assert_eq!(stranger.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        json_body(stranger).await?["error"]["code"],
        "TF-VAL-0007",
        "a non-member was added to a team"
    );

    // And the happy path, so the refusals above are not just "everything fails".
    let own = send(
        &app,
        request(&alice, "POST", &format!("/api/v1/teams/{team}/members"))
            .header(WORKSPACE_HEADER, alpha.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": alice.user_id }).to_string()))?,
    )
    .await?;
    assert_eq!(own.status(), StatusCode::CREATED);

    let removed = send(
        &app,
        request(
            &alice,
            "DELETE",
            &format!("/api/v1/teams/{team}/members/{}", alice.user_id),
        )
        .header(WORKSPACE_HEADER, alpha.to_string())
        .body(Body::empty())?,
    )
    .await?;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_path_and_a_header_that_disagree_are_refused() -> Result<()> {
    // Preferring one silently would mean the caller cannot tell which workspace
    // they were answered about.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let alice = sign_up(&app, &db.pool, "alice@example.test").await?;
    let (alpha, _) = create_workspace(&app, &alice, "alpha").await?;
    let (beta, _) = create_workspace(&app, &alice, "beta").await?;

    let response = send(
        &app,
        request(&alice, "GET", &format!("/api/v1/workspaces/{alpha}"))
            .header(WORKSPACE_HEADER, beta.to_string())
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a request naming two different workspaces was answered"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_new_routes_are_inside_the_csrf_guard() -> Result<()> {
    // The rule this asserts is about the ROUTER, not the handler: a route
    // registered after `.layer()` escapes both the CSRF guard and the request
    // id, and nothing about the handler would show it.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;

    let response = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/api/v1/workspaces")
            .header(header::COOKIE, &caller.cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": "Acme", "slug": "acme" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a state-changing request succeeded with only a session cookie"
    );
    assert!(
        response.headers().contains_key("x-request-id"),
        "the route is outside the observability layer too"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_list_pages_by_cursor_and_never_by_offset() -> Result<()> {
    // docs/05 §Pagination and docs/26: cursor pagination everywhere, opaque to
    // the client, with the probe row used to answer has_more without a count.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;
    let mut created = Vec::new();
    for slug in ["one", "two", "three"] {
        created.push(create_workspace(&app, &caller, slug).await?.0);
    }
    created.sort_unstable();

    let first = json_body(
        send(
            &app,
            request(&caller, "GET", "/api/v1/workspaces?limit=2").body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(first["data"].as_array().map(Vec::len), Some(2));
    assert_eq!(first["page"]["has_more"], true);
    let cursor = first["page"]["next_cursor"]
        .as_str()
        .context("no cursor on a page that has more")?
        .to_owned();

    let second = json_body(
        send(
            &app,
            request(
                &caller,
                "GET",
                &format!("/api/v1/workspaces?limit=2&cursor={cursor}"),
            )
            .body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(second["data"].as_array().map(Vec::len), Some(1));
    assert_eq!(second["page"]["has_more"], false);
    assert_eq!(second["data"][0]["id"], created[2].to_string());

    // Bounds are enforced rather than clamped: a silently shortened page is
    // indistinguishable to the client from a short last page.
    let over = send(
        &app,
        request(&caller, "GET", "/api/v1/workspaces?limit=101").body(Body::empty())?,
    )
    .await?;
    assert_eq!(over.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_taken_slug_is_refused_and_nothing_is_left_behind() -> Result<()> {
    // The transaction must roll back completely: a workspace row that failed to
    // commit but left an audit row would be history for something that never
    // happened.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let alice = sign_up(&app, &db.pool, "alice@example.test").await?;
    let bob = sign_up(&app, &db.pool, "bob@example.test").await?;
    create_workspace(&app, &alice, "acme").await?;

    let response = send(
        &app,
        request(&bob, "POST", "/api/v1/workspaces")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": "Also Acme", "slug": "acme" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(json_body(response).await?["error"]["code"], "TF-PRJ-0007");

    let mine = json_body(
        send(
            &app,
            request(&bob, "GET", "/api/v1/workspaces").body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(
        mine["data"].as_array().map(Vec::len),
        Some(0),
        "a failed creation left the caller a member of something"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_bad_slug_or_name_is_refused_before_anything_is_written() -> Result<()> {
    // Every input bounded (AGENTS.md §Engineering priorities 4). A slug reaches
    // a URL, so its character set is decided here rather than inherited from
    // whatever the client sent.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;

    for (name, slug) in [
        ("Acme", "Not Lowercase"),
        ("Acme", "-leading-dash"),
        ("Acme", ""),
        ("", "acme"),
        ("   ", "acme"),
    ] {
        let response = send(
            &app,
            request(&caller, "POST", "/api/v1/workspaces")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "name": name, "slug": slug }).to_string(),
                ))?,
        )
        .await?;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "accepted name={name:?} slug={slug:?}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn adding_a_member_twice_is_not_an_error() -> Result<()> {
    // A client that retries a request whose response it never saw is doing the
    // right thing; an error there makes correct behaviour look broken.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.test").await?;
    let guest = sign_up(&app, &db.pool, "guest@example.test").await?;
    let (workspace, _) = create_workspace(&app, &owner, "acme").await?;
    let uri = format!("/api/v1/workspaces/{workspace}/members");

    let first = send(
        &app,
        request(&owner, "POST", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "user_id": guest.user_id, "member_type": "GUEST" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(json_body(first).await?["member_type"], "GUEST");

    let again = send(
        &app,
        request(&owner, "POST", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": guest.user_id }).to_string()))?,
    )
    .await?;
    assert_eq!(again.status(), StatusCode::OK);
    assert_eq!(
        json_body(again).await?["member_type"],
        "GUEST",
        "a repeat add rewrote the member type it did not ask about"
    );

    // An unknown person is a domain-rule violation, not a 500.
    let nobody = send(
        &app,
        request(&owner, "POST", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": Uuid::now_v7() }).to_string()))?,
    )
    .await?;
    assert_eq!(nobody.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json_body(nobody).await?["error"]["code"], "TF-VAL-0007");

    // And an unknown member type is refused rather than stored.
    let bad = send(
        &app,
        request(&owner, "POST", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "user_id": guest.user_id, "member_type": "OWNER" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

// ---------------------------------------------------------------------------
// D-054 — a workspace acquires an owner when it is created
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn creating_a_workspace_makes_the_creator_its_owner() -> Result<()> {
    // The defect this closes: `role_assignment` is the only source of authority
    // (migration 0003), and nothing created one. The workspace committed, its
    // creator was its only member, and every write they could ever attempt was
    // refused — with no way out, because granting requires a grant.
    //
    // Asserted through the HTTP route rather than against the repository,
    // because the guarantee is about what a *request* leaves behind.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let founder = sign_up(&app, &db.pool, "founder@example.com").await?;
    let (workspace, _) = create_workspace(&app, &founder, "acme").await?;

    let grants = test_support::workspace_grants(&db.pool, workspace).await?;
    assert!(
        grants.iter().any(|(principal, role, permission)| {
            *principal == founder.user_id && role == "Owner" && permission == "workspace.owner"
        }),
        "POST /api/v1/workspaces left a workspace with no owner: {grants:?}"
    );

    // Exactly one owner, and it is the creator. A bootstrap that granted the
    // role twice, or to somebody else as well, would pass the assertion above.
    let owners: Vec<Uuid> = grants
        .iter()
        .filter(|(_, _, permission)| permission == "workspace.owner")
        .map(|(principal, _, _)| *principal)
        .collect();
    assert_eq!(owners, vec![founder.user_id]);

    // All five templates, with the sets docs/04 describes.
    let templates = test_support::role_templates(&db.pool, workspace).await?;
    let names: Vec<&str> = templates.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Administrator",
            "Guest",
            "Member",
            "Owner",
            "Project Manager"
        ],
        "the five docs/04 templates were not materialized"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_owner_grant_is_audited_in_the_transaction_that_made_it() -> Result<()> {
    // docs/04 control 7: "Every grant, revoke, role edit, and consent writes an
    // `audit_event` with before/after." A grant nobody can find in the audit
    // trail is a grant nobody can explain during an incident.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let founder = sign_up(&app, &db.pool, "founder@example.com").await?;
    let (workspace, _) = create_workspace(&app, &founder, "acme").await?;

    let audited = test_support::audit_changes(&db.pool, workspace).await?;
    let grant = audited
        .iter()
        .find_map(|entry| entry.get("after")?.get("role_assignment"))
        .context("no audit record names the owner grant")?;

    assert_eq!(grant["role_name"], "Owner");
    assert_eq!(grant["scope_type"], "WORKSPACE");
    assert_eq!(grant["principal_type"], "USER");
    assert_eq!(grant["principal_id"], founder.user_id.to_string());
    assert_eq!(grant["scope_id"], workspace.to_string());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_refused_create_leaves_no_roles_behind() -> Result<()> {
    // The bootstrap runs in the same transaction as the workspace row, so a
    // create that fails after the row is written must leave nothing — no
    // workspace, no templates, no grant. A taken slug is the failure that
    // actually happens.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let founder = sign_up(&app, &db.pool, "founder@example.com").await?;
    let rival = sign_up(&app, &db.pool, "rival@example.com").await?;
    let (workspace, _) = create_workspace(&app, &founder, "acme").await?;

    let refused = send(
        &app,
        request(&rival, "POST", "/api/v1/workspaces")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": "Also Acme", "slug": "acme" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(refused.status(), StatusCode::CONFLICT);

    // The winner's workspace is untouched: still five templates and one owner,
    // and the loser's rolled-back attempt added nothing to it.
    assert_eq!(
        test_support::role_templates(&db.pool, workspace)
            .await?
            .len(),
        5
    );
    let owners: Vec<Uuid> = test_support::workspace_grants(&db.pool, workspace)
        .await?
        .into_iter()
        .filter(|(_, _, permission)| permission == "workspace.owner")
        .map(|(principal, _, _)| principal)
        .collect();
    assert_eq!(owners, vec![founder.user_id]);
    Ok(())
}
