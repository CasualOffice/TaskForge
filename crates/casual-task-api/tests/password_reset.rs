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
        storage: std::sync::Arc::new(casual_task_infra::FilesystemStore::new(
            std::env::temp_dir().join("tf-test-objects"),
            "https://files.example.test".to_owned(),
            "test-object-signing-secret".to_owned(),
        )),
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

#[path = "password_reset/part1.rs"]
mod part1;
