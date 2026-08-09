//! A person's own account, end to end (C-001, `docs/40`).
//!
//! Three properties are worth a database to assert:
//!
//! - **These handlers answer only about the caller.** They are the one part of
//!   the product outside the tenant boundary, and that is safe only because
//!   nothing here takes another person's id. A test tries to reach someone
//!   else's session anyway.
//! - **A password change is not a reset.** It requires the current password even
//!   though the caller is signed in, because the reason to change one is usually
//!   that somebody else might know the old one — and it ends **every** session,
//!   this one included. That is the schema's rule, not the handler's: migration
//!   0016 moves `changed_at` and `live_session` refuses anything created before
//!   it.
//! - **"Sign out everywhere" keeps the device in your hand.** A person doing it
//!   because a laptop was lost does not want to be signed out of the phone they
//!   are holding.

mod schema_harness;

use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
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
    user: Uuid,
}

impl Caller {
    async fn get(&self, uri: &str) -> Result<(StatusCode, serde_json::Value)> {
        self.send(
            Request::builder()
                .uri(uri)
                .header(header::COOKIE, &self.cookie)
                .body(Body::empty())?,
        )
        .await
    }

    async fn patch(
        &self,
        uri: &str,
        body: &serde_json::Value,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.with_body("PATCH", uri, body).await
    }

    async fn post(
        &self,
        uri: &str,
        body: &serde_json::Value,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.with_body("POST", uri, body).await
    }

    async fn delete(&self, uri: &str) -> Result<(StatusCode, serde_json::Value)> {
        self.send(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .header(header::COOKIE, &self.cookie)
                .header("x-csrf-token", &self.csrf)
                .body(Body::empty())?,
        )
        .await
    }

    async fn with_body(
        &self,
        method: &str,
        uri: &str,
        body: &serde_json::Value,
    ) -> Result<(StatusCode, serde_json::Value)> {
        self.send(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, &self.cookie)
                .header("x-csrf-token", &self.csrf)
                .header("idempotency-key", Uuid::now_v7().to_string())
                .body(Body::from(body.to_string()))?,
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

/// Sign in, returning a caller. A second call for the same email is a second
/// session for the same person, which is what the session tests need.
async fn sign_in(app: &Router, pool: &sqlx::PgPool, email: &str, user: Uuid) -> Result<Caller> {
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
    let _ = pool;
    Ok(Caller {
        app: app.clone(),
        cookie,
        csrf,
        user,
    })
}

async fn a_person(pool: &sqlx::PgPool, email: &str) -> Result<(Router, Caller)> {
    let user = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        user,
        email,
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
    let app = router(state(pool.clone()));
    let caller = sign_in(&app, pool, email, user).await?;
    Ok((app, caller))
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_person_reads_and_edits_their_own_account() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let (_, me) = a_person(&db.pool, "ama@example.com").await?;

    let (status, body) = me.get("/api/v1/me").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], me.user.to_string());
    assert_eq!(body["email"], "ama@example.com");
    // Unset is not UTC — a client that assumed otherwise would be asserting a
    // day boundary nobody chose.
    assert!(body["time_zone"].is_null(), "{body}");

    let (status, body) = me
        .patch(
            "/api/v1/me",
            &json!({ "display_name": "Ama Osei", "time_zone": "Australia/Sydney" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["display_name"], "Ama Osei");
    assert_eq!(body["time_zone"], "Australia/Sydney");

    // `null` clears it; absent would have left it alone.
    let (status, body) = me
        .patch("/api/v1/me", &json!({ "time_zone": null }))
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["time_zone"].is_null(), "{body}");
    assert_eq!(body["display_name"], "Ama Osei", "the name was not touched");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_malformed_time_zone_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let (_, me) = a_person(&db.pool, "ama@example.com").await?;

    for bad in ["", "   ", "Europe/London; DROP TABLE session"] {
        let (status, body) = me.patch("/api/v1/me", &json!({ "time_zone": bad })).await?;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{bad:?} was accepted: {body}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn changing_a_password_needs_the_current_one() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let (_, me) = a_person(&db.pool, "ama@example.com").await?;

    let (status, body) = me
        .post(
            "/api/v1/me/password",
            &json!({ "current_password": "not it at all", "new_password": "a brand new long password" }),
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // The old password still works, so the refusal changed nothing.
    let (status, body) = me
        .post(
            "/api/v1/me/password",
            &json!({ "current_password": PASSWORD, "new_password": "a brand new long password" }),
        )
        .await?;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_new_password_below_the_minimum_is_refused() -> Result<()> {
    // `docs/40`: no composition rules beyond a 12-character minimum. The rule
    // lives in `hash_chosen`, so this asserts the endpoint reaches it.
    let db = schema_harness::TestDatabase::start().await?;
    let (_, me) = a_person(&db.pool, "ama@example.com").await?;

    let (status, body) = me
        .post(
            "/api/v1/me/password",
            &json!({ "current_password": PASSWORD, "new_password": "elevenchar" }),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["details"]["min_length"], 12);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn changing_a_password_ends_every_session_including_this_one() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let (app, laptop) = a_person(&db.pool, "ama@example.com").await?;
    let phone = sign_in(&app, &db.pool, "ama@example.com", laptop.user).await?;

    let (status, listed) = laptop.get("/api/v1/me/sessions").await?;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["data"].as_array().map(Vec::len), Some(2), "{listed}");

    let (status, _) = laptop
        .post(
            "/api/v1/me/password",
            &json!({ "current_password": PASSWORD, "new_password": "a brand new long password" }),
        )
        .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Both, including the one that made the change. Migration 0016 moves
    // `changed_at` and `live_session` refuses anything created before it —
    // "forces re-authentication everywhere". "Everywhere except where I am
    // standing" would be a weaker guarantee than it sounds, and this is the
    // stance the schema takes rather than one the handler chose.
    let (status, _) = laptop.get("/api/v1/me").await?;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the changing session survived"
    );
    let (status, _) = phone.get("/api/v1/me").await?;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the other session survived"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn sign_out_everywhere_keeps_the_device_in_your_hand() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let (app, laptop) = a_person(&db.pool, "ama@example.com").await?;
    let phone = sign_in(&app, &db.pool, "ama@example.com", laptop.user).await?;

    let (status, _) = laptop.delete("/api/v1/me/sessions").await?;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = laptop.get("/api/v1/me").await?;
    assert_eq!(status, StatusCode::OK, "the current session was revoked");
    let (status, _) = phone.get("/api/v1/me").await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_session_list_says_which_one_you_are_using() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let (app, laptop) = a_person(&db.pool, "ama@example.com").await?;
    let _phone = sign_in(&app, &db.pool, "ama@example.com", laptop.user).await?;

    let (status, listed) = laptop.get("/api/v1/me/sessions").await?;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let rows = listed["data"].as_array().expect("data");
    assert_eq!(
        rows.iter().filter(|s| s["current"] == true).count(),
        1,
        "exactly one session is the one asking: {listed}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn one_person_cannot_revoke_anothers_session() -> Result<()> {
    // These handlers live outside the tenant boundary, which is safe only
    // because they answer about the caller alone. Guessing a uuid must not be a
    // way to sign somebody else out.
    let db = schema_harness::TestDatabase::start().await?;
    let (_, ama) = a_person(&db.pool, "ama@example.com").await?;
    let (_, tomas) = a_person(&db.pool, "tomas@example.com").await?;

    let (_, listed) = tomas.get("/api/v1/me/sessions").await?;
    let theirs = listed["data"].as_array().expect("data")[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let (status, body) = ama.delete(&format!("/api/v1/me/sessions/{theirs}")).await?;
    // 404, never 403: telling a caller a session exists but is not theirs is
    // telling them about somebody else's session.
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, _) = tomas.get("/api/v1/me").await?;
    assert_eq!(status, StatusCode::OK, "their session was revoked anyway");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn none_of_it_answers_without_a_credential() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let app = router(state(db.pool.clone()));

    for (method, uri) in [
        ("GET", "/api/v1/me"),
        ("GET", "/api/v1/me/sessions"),
        ("DELETE", "/api/v1/me/sessions"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {uri} answered an anonymous caller"
        );
    }
    Ok(())
}
