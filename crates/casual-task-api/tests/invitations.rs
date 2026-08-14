//! Invitations end to end, against a real PostgreSQL (C-001, `docs/40`).
//!
//! The properties asserted here cannot be observed from inside a handler:
//!
//! - **The enumeration gate.** `docs/40` §Acceptance gates covers login, reset
//!   **and invite**. Inviting an address that already has an account is
//!   compared byte for byte against inviting one that does not. Until this
//!   test existed, that gate could not close.
//! - **Tied to the address.** An invitation is not a bearer token for whoever
//!   holds the link.
//! - **Single use**, including the case where the invitation is spent and the
//!   same link is presented again.
//! - **The seam works as `taskforge_app`.** Every acceptance here runs through
//!   migration 0022's `SECURITY DEFINER` functions. C-002 shipped this exact
//!   bug: read unscoped under the policy, the lookup returns nothing and every
//!   acceptance fails — while every test passes, because the harness connects
//!   as the owner and RLS is inert for a superuser.

mod schema_harness;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::server::{AppState, router};
use casual_task_identity::password;
use casual_task_infra::{Mailer, Message};
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";
const PUBLIC_URL: &str = "https://tasks.example.test";

/// A [`Mailer`] that keeps what it was handed.
///
/// The invitation token exists in exactly one place outside the database — the
/// email body — so a test that wants to *use* a link has to read it the way an
/// invitee does. Reaching into the table for the selector would test a
/// different system than the one users have.
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
    /// Wait for message `index`, since delivery is deliberately off the request
    /// path. Polled rather than slept on: a fixed sleep is either flaky or slow.
    async fn message(&self, index: usize) -> Result<Message> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(message) = self.sent.lock().expect("not poisoned").get(index).cloned() {
                return Ok(message);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Err(anyhow!(
            "no invitation email was delivered within the deadline"
        ))
    }

    fn count(&self) -> usize {
        self.sent.lock().expect("not poisoned").len()
    }
}

struct Caller {
    user_id: Uuid,
    cookie: String,
    csrf: String,
}

fn app(pool: sqlx::PgPool, mailer: Arc<dyn Mailer>) -> axum::Router {
    router(AppState {
        storage: std::sync::Arc::new(casual_task_infra::FilesystemStore::new(
            std::env::temp_dir().join("tf-test-objects"),
            "https://files.example.test".to_owned(),
            "test-object-signing-secret".to_owned(),
        )),
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: "a-test-secret-key-long-enough-for-hmac".into(),
        broadcast: casual_task_api::sse::local_hub(),
        public_url: PUBLIC_URL.into(),
        mailer,
    })
}

async fn json_body(response: axum::response::Response) -> Result<Value> {
    let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn status_and_body(response: axum::response::Response) -> (StatusCode, String) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Through casual-task-persistence, not raw SQL: `docs/19` puts every query in
/// that crate and `casual-task-lint` enforces it, **including in tests**.
async fn sign_up(app: &axum::Router, pool: &sqlx::PgPool, email: &str) -> Result<Caller> {
    let user_id = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        user_id,
        email,
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
    let (cookie, csrf) = login(app, email).await?;
    Ok(Caller {
        user_id,
        cookie,
        csrf,
    })
}

async fn login(app: &axum::Router, email: &str) -> Result<(String, String)> {
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
    Ok((cookie, csrf))
}

fn request(caller: &Caller, method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, &caller.cookie)
        .header("x-csrf-token", &caller.csrf)
}

async fn create_workspace(app: &axum::Router, caller: &Caller, slug: &str) -> Result<Uuid> {
    let response = app
        .clone()
        .oneshot(
            request(caller, "POST", "/api/v1/workspaces")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "name": format!("Workspace {slug}"), "slug": slug }).to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED, "create failed");
    let body = json_body(response).await?;
    Ok(body["id"].as_str().context("id")?.parse()?)
}

async fn invite(
    app: &axum::Router,
    caller: &Caller,
    workspace: Uuid,
    email: &str,
    role_id: Option<Uuid>,
) -> Result<axum::response::Response> {
    let mut body = json!({ "email": email });
    if let Some(role) = role_id {
        body["role_id"] = json!(role);
    }
    Ok(app
        .clone()
        .oneshot(
            request(
                caller,
                "POST",
                &format!("/api/v1/workspaces/{workspace}/invitations"),
            )
            .header("x-workspace-id", workspace.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))?,
        )
        .await?)
}

/// The token out of the emailed link, exactly as an invitee's mail client would
/// hand it back.
fn token_from(message: &Message) -> Result<String> {
    message
        .expose_body()
        .split_once("?token=")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("the email carried no invitation link"))
}

fn accept_request(token: &str, display_name: Option<&str>) -> Result<Request<Body>> {
    let mut body = json!({ "token": token });
    if let Some(name) = display_name {
        body["display_name"] = json!(name);
    }
    Ok(Request::builder()
        .method("POST")
        .uri("/api/v1/auth/invitations/accept")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))?)
}

#[path = "invitations/part1.rs"]
mod part1;
#[path = "invitations/part2.rs"]
mod part2;
