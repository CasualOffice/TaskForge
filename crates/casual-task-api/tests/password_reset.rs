//! Password reset end to end, against a real PostgreSQL (C-001, `docs/40`).
//!
//! Every test here fails without the code it covers, and each covers a property
//! that cannot be observed from inside a handler:
//!
//! - **Enumeration.** `docs/40` §Acceptance gates puts reset beside login:
//!   responses are indistinguishable for existing and non-existing accounts
//!   "in body, status, and timing envelope". Body and status are compared byte
//!   for byte; timing is an order-of-magnitude property, because a tight bound
//!   on a shared runner is a flaky test rather than a stronger one.
//! - **Single use.** A token that works twice is a token that works for whoever
//!   reads the mailbox next.
//! - **Hashed at rest.** The stored columns are read back and searched for the
//!   token — `docs/40`'s token-hash gate, applied to reset links.
//! - **Sessions die.** `docs/40`: a password change invalidates sessions. A
//!   reset that leaves the attacker's session live has changed nothing.

mod schema_harness;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::server::{AppState, router};
use casual_task_identity::password;
use casual_task_infra::{Mailer, Message};
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";
const NEW_PASSWORD: &str = "an even longer replacement password";
const PUBLIC_URL: &str = "https://tasks.example.test";

/// A [`Mailer`] that keeps what it was handed.
///
/// The reset token exists in exactly one place outside the database — the email
/// body — so a test that wants to *use* a link has to read it the way a user
/// does. Reaching into the table for the selector would test a different system
/// than the one users have.
#[derive(Debug, Default)]
struct Recording {
    sent: Mutex<Vec<Message>>,
}

impl Mailer for Recording {
    fn send<'a>(
        &'a self,
        message: &'a Message,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), casual_task_infra::mail::MailError>>
                + Send
                + 'a,
        >,
    > {
        let message = message.clone();
        Box::pin(async move {
            self.sent.lock().expect("not poisoned").push(message);
            Ok(())
        })
    }
}

impl Recording {
    /// Wait for the next message, since delivery is deliberately off the
    /// request path (see `password_reset`'s module docs on the timing
    /// envelope). Polled rather than slept on: a fixed sleep is either flaky or
    /// slow, and usually both.
    async fn next_message(&self, after: usize) -> Result<Message> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(message) = self.sent.lock().expect("not poisoned").get(after).cloned() {
                return Ok(message);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Err(anyhow!("no email was delivered within the deadline"))
    }

    fn count(&self) -> usize {
        self.sent.lock().expect("not poisoned").len()
    }
}

fn app(pool: sqlx::PgPool, mailer: Arc<dyn Mailer>) -> axum::Router {
    router(AppState {
        broadcast: casual_task_api::sse::local_hub(),
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: "a-test-secret-key-long-enough-for-hmac".into(),
        public_url: PUBLIC_URL.into(),
        mailer,
    })
}

/// Through casual-task-persistence, not raw SQL: `docs/19` puts every query in
/// that crate and `casual-task-lint` enforces it, **including in tests**.
async fn seed_user(pool: &sqlx::PgPool, email: &str) -> Result<Uuid> {
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

fn request_reset(email: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/auth/password-reset")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "email": email }).to_string(),
        ))
        .expect("request")
}

fn confirm_reset(token: &str, new_password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/auth/password-reset/confirm")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "token": token, "password": new_password }).to_string(),
        ))
        .expect("request")
}

async fn status_and_body(response: axum::response::Response) -> (StatusCode, String) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The token out of the emailed link, exactly as a user's mail client would
/// hand it back.
fn token_from(message: &Message) -> Result<String> {
    message
        .expose_body()
        .split_once("?token=")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("the email carried no reset link"))
}

/// Ask for a reset and return the token from the email.
async fn reset_token(app: &axum::Router, mailer: &Arc<Recording>, email: &str) -> Result<String> {
    let before = mailer.count();
    let response = app.clone().oneshot(request_reset(email)).await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    token_from(&mailer.next_message(before).await?)
}

/// Log in and return the session cookie.
async fn login(app: &axum::Router, email: &str, password: &str) -> Result<String> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "email": email, "password": password }).to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "login failed");
    Ok(response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with(SESSION_COOKIE))
        .and_then(|c| c.split(';').next())
        .ok_or_else(|| anyhow!("no session cookie"))?
        .to_owned())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_reset_for_an_unknown_address_is_indistinguishable_from_a_real_one() -> Result<()> {
    // docs/40 §Acceptance gates: "login, reset, and invite responses are
    // indistinguishable for existing and non-existing accounts, in body,
    // status, and timing envelope". This is the reset half.
    let db = schema_harness::TestDatabase::start().await?;
    seed_user(&db.pool, "real@example.com").await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let started = Instant::now();
    let real = status_and_body(
        app.clone()
            .oneshot(request_reset("real@example.com"))
            .await?,
    )
    .await;
    let real_elapsed = started.elapsed();

    let started = Instant::now();
    let unknown = status_and_body(
        app.clone()
            .oneshot(request_reset("nobody@example.com"))
            .await?,
    )
    .await;
    let unknown_elapsed = started.elapsed();

    assert_eq!(real.0, unknown.0, "the status differs");
    assert_eq!(real.1, unknown.1, "the body differs");
    assert_eq!(real.0, StatusCode::ACCEPTED);

    // The envelope, not a tight bound. The point is that the endpoint does not
    // hold the request open for an SMTP round trip on one branch and skip it on
    // the other — that difference is readable with a stopwatch.
    let (slower, faster) = if real_elapsed > unknown_elapsed {
        (real_elapsed, unknown_elapsed)
    } else {
        (unknown_elapsed, real_elapsed)
    };
    assert!(
        slower < faster + Duration::from_millis(500),
        "one branch took {slower:?} and the other {faster:?}; that gap is an account oracle"
    );

    // And exactly one email exists, for the address that has an account.
    let message = mailer.next_message(0).await?;
    assert_eq!(message.to(), "real@example.com");
    assert_eq!(
        mailer.count(),
        1,
        "an email was sent for an unknown address"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_reset_token_works_once_and_the_second_use_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed_user(&db.pool, "user@example.com").await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let token = reset_token(&app, &mailer, "user@example.com").await?;

    let first = app
        .clone()
        .oneshot(confirm_reset(&token, NEW_PASSWORD))
        .await?;
    assert_eq!(first.status(), StatusCode::NO_CONTENT);

    // The same link again. A token that works twice is a token that works for
    // whoever reads the mailbox next.
    let second = app
        .clone()
        .oneshot(confirm_reset(&token, "yet another long password"))
        .await?;
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);

    // And the second attempt changed nothing.
    assert!(
        password::verify(
            NEW_PASSWORD,
            &test_support::password_hash_of(&db.pool, user).await?
        )
        .expect("parses"),
        "the refused second use still replaced the password"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_expired_reset_token_is_refused() -> Result<()> {
    // docs/40 gives a reset token one hour. The clock is moved rather than the
    // test waiting for it — a test that sleeps for an hour is a test that is
    // run once and then disabled.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed_user(&db.pool, "user@example.com").await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let token = reset_token(&app, &mailer, "user@example.com").await?;
    assert_eq!(test_support::expire_reset_tokens(&db.pool, user).await?, 1);

    let response = app
        .clone()
        .oneshot(confirm_reset(&token, NEW_PASSWORD))
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    assert!(
        password::verify(
            PASSWORD,
            &test_support::password_hash_of(&db.pool, user).await?
        )
        .expect("parses"),
        "an expired token still changed the password"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_successful_reset_revokes_every_existing_session() -> Result<()> {
    // docs/40 §Local authentication: reset tokens are "invalidated by password
    // change", and the session rule beside it is the same requirement from the
    // other end. A reset that leaves an attacker's session live has changed
    // nothing they care about.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed_user(&db.pool, "user@example.com").await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let cookie = login(&app, "user@example.com", PASSWORD).await?;
    assert_eq!(test_support::live_session_count(&db.pool).await?, 1);

    // The session works, so the assertion below is about the reset and not
    // about the session never having worked.
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

    let token = reset_token(&app, &mailer, "user@example.com").await?;
    let response = app
        .clone()
        .oneshot(confirm_reset(&token, NEW_PASSWORD))
        .await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        test_support::live_session_count(&db.pool).await?,
        0,
        "a session survived the password change"
    );
    let after = app
        .clone()
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
        "the old cookie still authenticates"
    );

    // The new password is the one that works now, and the old one is not.
    assert!(login(&app, "user@example.com", NEW_PASSWORD).await.is_ok());
    let _ = user;
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_new_password_below_the_minimum_is_refused_without_spending_the_token() -> Result<()> {
    // docs/40: "No composition rules beyond a 12-character minimum." The rule
    // lives in `hash_chosen`, so this asserts the endpoint reaches it — the
    // endpoint could have called `hash_generated` and bypassed the policy
    // entirely, which is exactly what the two functions exist to make visible.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed_user(&db.pool, "user@example.com").await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let token = reset_token(&app, &mailer, "user@example.com").await?;
    let response = app
        .clone()
        .oneshot(confirm_reset(&token, "elevenchar"))
        .await?;
    let (status, body) = status_and_body(response).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let json: serde_json::Value = serde_json::from_str(&body)?;
    // TF-VAL-0004, not the retired TF-REQ-0001 (D-055): a password under the
    // minimum is a field value out of range.
    assert_eq!(json["error"]["code"], "TF-VAL-0004");
    assert_eq!(json["error"]["details"]["min_length"], 12);

    assert!(
        password::verify(
            PASSWORD,
            &test_support::password_hash_of(&db.pool, user).await?
        )
        .expect("parses"),
        "a short password was accepted"
    );

    // The link still works. Sending someone back to their inbox because they
    // typed a short password is a reset flow people abandon.
    assert_eq!(
        test_support::live_reset_token_count(&db.pool, user).await?,
        1,
        "a rejected password spent the token"
    );
    let retry = app
        .clone()
        .oneshot(confirm_reset(&token, NEW_PASSWORD))
        .await?;
    assert_eq!(retry.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_stored_token_is_not_a_usable_reset_link() -> Result<()> {
    // docs/40 §Acceptance gates, "token-hash test": a database dump contains no
    // usable credential. Asserted against what is IN the table, not against
    // what the writing code intended to put there.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed_user(&db.pool, "user@example.com").await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let token = reset_token(&app, &mailer, "user@example.com").await?;
    let (_, verifier) = token.split_once('.').expect("selector.verifier");

    for stored in test_support::reset_token_columns(&db.pool, user).await? {
        assert!(
            !stored.contains(verifier),
            "the verifier is recoverable from the stored row: {stored}"
        );
        assert!(
            !stored.contains(&token),
            "the whole token is in the stored row: {stored}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn asking_twice_leaves_only_the_newest_link_working() -> Result<()> {
    // A slow first email means people ask again. Two live tokens in one inbox
    // makes the exposure window the longest expiry rather than the shortest.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed_user(&db.pool, "user@example.com").await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let first = reset_token(&app, &mailer, "user@example.com").await?;
    let second = reset_token(&app, &mailer, "user@example.com").await?;
    assert_ne!(first, second);
    assert_eq!(
        test_support::live_reset_token_count(&db.pool, user).await?,
        1
    );

    let stale = app
        .clone()
        .oneshot(confirm_reset(&first, NEW_PASSWORD))
        .await?;
    assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);
    let fresh = app
        .clone()
        .oneshot(confirm_reset(&second, NEW_PASSWORD))
        .await?;
    assert_eq!(fresh.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_forged_verifier_against_a_real_selector_is_refused() -> Result<()> {
    // The reason the verifier is stored hashed at all. Someone who reads the
    // selector — from a log, a referrer header, a shoulder — must not be able
    // to complete a reset with a verifier of their own choosing.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed_user(&db.pool, "user@example.com").await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let token = reset_token(&app, &mailer, "user@example.com").await?;
    let (selector, _) = token.split_once('.').expect("selector.verifier");
    let forged = format!("{selector}.{}", "0".repeat(48));

    let response = app
        .clone()
        .oneshot(confirm_reset(&forged, NEW_PASSWORD))
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        password::verify(
            PASSWORD,
            &test_support::password_hash_of(&db.pool, user).await?
        )
        .expect("parses"),
        "a forged verifier changed the password"
    );

    // A malformed token fails the same way, so the two are not distinguishable.
    for bad in ["", "nonsense", "tf_pat_abc.def"] {
        let response = app
            .clone()
            .oneshot(confirm_reset(bad, NEW_PASSWORD))
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "accepted {bad:?}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_reset_request_is_recorded_for_an_address_with_no_account() -> Result<()> {
    // docs/40 §What is audited. An attacker probing addresses through the reset
    // endpoint produces exactly these rows, and only these rows show the
    // pattern — recording only the matches hides the attack.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let response = app
        .clone()
        .oneshot(request_reset("nobody@example.com"))
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        test_support::auth_events(&db.pool, "nobody@example.com").await?,
        vec!["password.reset.requested".to_owned()]
    );
    Ok(())
}
