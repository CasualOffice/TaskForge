//! Authentication, workspace resolution and CSRF, end to end.
//!
//! Everything here is a refusal. That is the point: the interesting behaviour
//! of an auth layer is what it *stops*, and a test suite that only proves the
//! happy path proves the layer exists rather than that it works.

mod schema_harness;

use std::sync::Arc;

use anyhow::Result;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{Request, StatusCode, header};
use axum::response::IntoResponse;
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::middleware::{WORKSPACE_HEADER, WorkspaceMember};
use casual_task_api::server::{AppState, router};
use casual_task_identity::password;
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";
const SECRET: &str = "a-test-secret-key-long-enough-for-hmac";

fn state(pool: sqlx::PgPool) -> AppState {
    AppState {
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: SECRET.into(),
        public_url: "https://tasks.example.test".into(),
        mailer: std::sync::Arc::new(casual_task_infra::mail::LoggingMailer),
    }
}

/// A router with one extra route that requires workspace membership.
///
/// Added here rather than in the server, because no product endpoint exists yet
/// and inventing one to make a test pass would put an endpoint in the API that
/// `docs/05` does not describe. The extractor is public, so exercising it
/// through a route is exactly how a real handler will use it.
fn app_with_protected_route(pool: sqlx::PgPool) -> axum::Router {
    let state = state(pool);
    router(state.clone()).route(
        "/test/workspace",
        axum::routing::get(async |State(_): State<AppState>, member: WorkspaceMember| {
            (
                StatusCode::OK,
                member.context.scope().id().as_uuid().to_string(),
            )
                .into_response()
        })
        .with_state(state),
    )
}

async fn seed(pool: &sqlx::PgPool, email: &str) -> Result<Uuid> {
    let id = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        id,
        email,
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
    Ok(id)
}

/// Log in and return `(session cookie, csrf token)`.
async fn login(app: &axum::Router, email: &str) -> Result<(String, String)> {
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
    let token = body["csrf_token"].as_str().expect("csrf token").to_owned();
    Ok((cookie, token))
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_session_identifies_its_owner() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed(&db.pool, "user@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());
    let (cookie, _) = login(&app, "user@example.com").await?;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(body["actor_id"], user.to_string());
    assert_eq!(body["actor_type"], "USER");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn no_credential_is_401() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let response = app_with_protected_route(db.pool.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_revoked_session_stops_working_on_the_very_next_request() -> Result<()> {
    // The property docs/40 rejects JWTs to get. Not at expiry — now.
    let db = schema_harness::TestDatabase::start().await?;
    seed(&db.pool, "user@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());
    let (cookie, token) = login(&app, "user@example.com").await?;

    // It works.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // Log out.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &token)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // The same cookie is now worthless.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a revoked session still authenticated"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_forged_session_cookie_is_refused() -> Result<()> {
    // The selector is guessable by construction — it is not a secret. Only the
    // verifier makes a cookie a credential, so a cookie with a real selector
    // and a wrong verifier must fail.
    let db = schema_harness::TestDatabase::start().await?;
    seed(&db.pool, "user@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());
    let (cookie, _) = login(&app, "user@example.com").await?;

    let (name, value) = cookie.split_once('=').expect("cookie");
    let (selector, _) = value.split_once('.').expect("selector.verifier");
    let forged = format!("{name}={selector}.{}", "0".repeat(48));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, forged)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a cookie with the right selector and a wrong verifier authenticated"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_workspace_the_actor_is_not_a_member_of_is_404_not_403() -> Result<()> {
    // docs/04: absent and invisible are never disambiguated. A 403 here tells an
    // authenticated stranger that the workspace exists, which is how workspace
    // ids get enumerated.
    let db = schema_harness::TestDatabase::start().await?;
    seed(&db.pool, "outsider@example.com").await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "private").await?;

    let app = app_with_protected_route(db.pool.clone());
    let (cookie, _) = login(&app, "outsider@example.com").await?;

    let real = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/test/workspace")
                .header(header::COOKIE, &cookie)
                .header(WORKSPACE_HEADER, workspace.to_string())
                .body(Body::empty())?,
        )
        .await?;
    let imaginary = app
        .oneshot(
            Request::builder()
                .uri("/test/workspace")
                .header(header::COOKIE, &cookie)
                .header(WORKSPACE_HEADER, Uuid::now_v7().to_string())
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(real.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        real.status(),
        imaginary.status(),
        "a real workspace the actor cannot see is distinguishable from one that \
         does not exist — that is how workspace ids get enumerated"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn membership_grants_the_workspace_scope() -> Result<()> {
    // The other half: the guard must not refuse everyone.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed(&db.pool, "member@example.com").await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "shared").await?;
    test_support::add_workspace_member(&db.pool, workspace, user).await?;

    let app = app_with_protected_route(db.pool.clone());
    let (cookie, _) = login(&app, "member@example.com").await?;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/test/workspace")
                .header(header::COOKIE, &cookie)
                .header(WORKSPACE_HEADER, workspace.to_string())
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), 1024).await?;
    assert_eq!(String::from_utf8_lossy(&bytes), workspace.to_string());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unsafe_method_without_a_csrf_token_is_refused() -> Result<()> {
    // docs/05: "every unsafe method without a valid token is rejected". The
    // session cookie alone is exactly what a cross-site form submission carries.
    let db = schema_harness::TestDatabase::start().await?;
    seed(&db.pool, "user@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());
    let (cookie, token) = login(&app, "user@example.com").await?;

    let without = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        without.status(),
        StatusCode::FORBIDDEN,
        "a state-changing request succeeded with only a session cookie"
    );

    // A token from a different session must not work either — this is what
    // binding the token to the session buys over a plain double submit.
    let wrong = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", "0".repeat(64))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);

    // And the real one works.
    let with = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &token)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(with.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_safe_method_needs_no_csrf_token() -> Result<()> {
    // The guard must not break reads. A CSRF check on GET would make every page
    // load fail for a client that has not yet been issued a token.
    let db = schema_harness::TestDatabase::start().await?;
    seed(&db.pool, "user@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());
    let (cookie, _) = login(&app, "user@example.com").await?;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_error_body_carries_the_same_request_id_as_the_header() -> Result<()> {
    // docs/05 promises "a `request_id` the user can quote to support". Two
    // different values for one request makes that promise false in exactly the
    // situation it exists for — a user reading an error and quoting the number
    // in front of them.
    //
    // Every rejection in this file used to carry a hardcoded literal ("auth",
    // "login", "csrf") in the body while the header carried a real id.
    let db = schema_harness::TestDatabase::start().await?;
    seed(&db.pool, "user@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());
    let (cookie, _) = login(&app, "user@example.com").await?;

    for (name, request) in [
        (
            "unauthenticated",
            Request::builder()
                .uri("/api/v1/auth/session")
                .body(Body::empty())?,
        ),
        (
            "csrf",
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())?,
        ),
        // Deliberately NOT /test/workspace. That route is attached after
        // `router()` returns, so it sits outside the observability layer and
        // has no request id at all — which is a fact about this test harness,
        // and also a warning: a route added after `.layer()` escapes both the
        // request id and the CSRF guard. Every real route is registered before
        // the layers, and `server.rs` says why.
    ] {
        let response = app.clone().oneshot(request).await?;
        let header_id = response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
        let body: serde_json::Value = serde_json::from_slice(&bytes)?;
        let body_id = body["error"]["request_id"].as_str().unwrap_or_default();

        assert!(!header_id.is_empty(), "{name}: no request id header");
        assert_eq!(
            body_id, header_id,
            "{name}: the error body says {body_id:?} and the header says \
             {header_id:?}; a user quoting one of them cannot be found by the \
             other"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_token_issued_for_one_workspace_does_not_work_in_another() -> Result<()> {
    // The bug this was written for: the token's own workspace was discarded and
    // the client's X-Workspace-Id header was trusted, so a token for A worked
    // in B whenever its owner was a member of B.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed(&db.pool, "alice@example.com").await?;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    for (id, slug) in [(a, "alpha"), (b, "beta")] {
        test_support::insert_workspace(&db.pool, id, slug).await?;
        test_support::add_workspace_member(&db.pool, id, user).await?;
    }

    let minted = casual_task_identity::credential::mint()?;
    let (selector, _) =
        casual_task_identity::credential::split(&minted.presented).expect("well formed");
    test_support::insert_api_token(&db.pool, a, user, "USER", selector, &minted.verifier_hash)
        .await?;

    let app = app_with_protected_route(db.pool.clone());
    let call = |workspace: Uuid| {
        let app = app.clone();
        let bearer = format!("Bearer {}", minted.presented);
        async move {
            app.oneshot(
                Request::builder()
                    .uri("/test/workspace")
                    .header(header::AUTHORIZATION, bearer)
                    .header(WORKSPACE_HEADER, workspace.to_string())
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
        }
    };

    assert_eq!(
        call(a).await?.status(),
        StatusCode::OK,
        "the token failed in its own workspace"
    );
    assert_eq!(
        call(b).await?.status(),
        StatusCode::NOT_FOUND,
        "a token issued for workspace A authenticated for workspace B"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_forged_token_verifier_is_refused() -> Result<()> {
    // Without this, the verifier comparison in the bearer branch could be
    // deleted and every other test still passed.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed(&db.pool, "alice@example.com").await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "alpha").await?;
    test_support::add_workspace_member(&db.pool, workspace, user).await?;

    let minted = casual_task_identity::credential::mint()?;
    let (selector, _) =
        casual_task_identity::credential::split(&minted.presented).expect("well formed");
    test_support::insert_api_token(
        &db.pool,
        workspace,
        user,
        "USER",
        selector,
        &minted.verifier_hash,
    )
    .await?;

    let response = app_with_protected_route(db.pool.clone())
        .oneshot(
            Request::builder()
                .uri("/test/workspace")
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {selector}.{}", "0".repeat(48)),
                )
                .header(WORKSPACE_HEADER, workspace.to_string())
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_tombstoned_account_cannot_log_in() -> Result<()> {
    // The is_tombstone filter in credential_for_email had no test; it could be
    // deleted silently and a deactivated user would keep signing in.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed(&db.pool, "gone@example.com").await?;
    test_support::tombstone_user(&db.pool, user).await?;

    let response = app_with_protected_route(db.pool.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "email": "gone@example.com", "password": PASSWORD })
                        .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_password_change_invalidates_existing_sessions() -> Result<()> {
    // docs/40 requires it and live_session implements it, but nothing exercised
    // the clause — it could have been deleted with every test still green.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed(&db.pool, "user@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());
    let (cookie, _) = login(&app, "user@example.com").await?;

    let before = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(before.status(), StatusCode::OK);

    test_support::mark_password_changed(&db.pool, user).await?;

    let after = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        after.status(),
        StatusCode::UNAUTHORIZED,
        "a session created before the password changed still works"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn tombstoning_an_account_kills_its_live_sessions() -> Result<()> {
    // Deactivating someone who is currently signed in has to end the session
    // they are holding, not the next one they create.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed(&db.pool, "leaver@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());
    let (cookie, _) = login(&app, "leaver@example.com").await?;

    let probe = |app: axum::Router, cookie: String| async move {
        app.oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
    };

    assert_eq!(
        probe(app.clone(), cookie.clone()).await?.status(),
        StatusCode::OK
    );
    test_support::tombstone_user(&db.pool, user).await?;
    assert_eq!(
        probe(app, cookie).await?.status(),
        StatusCode::UNAUTHORIZED,
        "a deactivated account kept its live session"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_session_dies_of_idleness_and_of_old_age() -> Result<()> {
    // docs/40: 14 d idle / 30 d absolute. One expires_at column cannot express
    // both — a session used daily would live forever under an idle-only rule,
    // and one left open in a tab survives its whole absolute life under an
    // expiry-only rule. Both bounds are tested because both are easy to lose.
    for (last_seen, created, label) in [
        ("15 days", "15 days", "idle"),
        ("1 hour", "31 days", "absolute"),
    ] {
        let db = schema_harness::TestDatabase::start().await?;
        seed(&db.pool, "user@example.com").await?;
        let app = app_with_protected_route(db.pool.clone());
        let (cookie, _) = login(&app, "user@example.com").await?;

        test_support::age_session(&db.pool, last_seen, created).await?;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "a session past its {label} lifetime still authenticated"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_authentication_attempt_is_recorded() -> Result<()> {
    // docs/40 §What is audited. The unknown-address row is the one that matters:
    // an attacker guessing addresses produces only those, and without them the
    // burst that signals an attack is invisible.
    let db = schema_harness::TestDatabase::start().await?;
    seed(&db.pool, "real@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());

    let attempt = |email: &'static str, password: &'static str| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "email": email, "password": password }).to_string(),
                    ))
                    .expect("request"),
            )
            .await
        }
    };

    attempt("real@example.com", "wrong").await?;
    attempt("nobody@example.com", PASSWORD).await?;
    attempt("real@example.com", PASSWORD).await?;

    assert_eq!(
        test_support::auth_events(&db.pool, "real@example.com").await?,
        vec!["login.succeeded".to_owned(), "login.failed".to_owned()]
    );
    assert_eq!(
        test_support::auth_events(&db.pool, "nobody@example.com").await?,
        vec!["login.failed".to_owned()],
        "an attempt on an unknown address left no trace, so credential \
         stuffing is invisible"
    );
    Ok(())
}
