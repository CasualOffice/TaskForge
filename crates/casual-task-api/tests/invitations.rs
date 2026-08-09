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

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn inviting_an_existing_account_is_indistinguishable_from_inviting_a_stranger() -> Result<()>
{
    // docs/40 §Acceptance gates: "login, reset, and invite responses are
    // indistinguishable for existing and non-existing accounts, in body, status,
    // and timing envelope". THIS IS THE INVITE HALF — the gate that could not
    // close until this endpoint existed.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    // One address has an account already; the other has never been seen.
    test_support::insert_user(
        &db.pool,
        Uuid::now_v7(),
        "known@example.com",
        "Known Person",
    )
    .await?;

    let started = Instant::now();
    let known =
        status_and_body(invite(&app, &owner, workspace, "known@example.com", None).await?).await;
    let known_elapsed = started.elapsed();

    let started = Instant::now();
    let stranger =
        status_and_body(invite(&app, &owner, workspace, "stranger@example.com", None).await?).await;
    let stranger_elapsed = started.elapsed();

    assert_eq!(known.0, stranger.0, "the status differs");
    assert_eq!(known.1, stranger.1, "the body differs");
    assert_eq!(known.0, StatusCode::ACCEPTED);

    // The envelope, not a tight bound. What must not happen is one branch
    // holding the request open for work the other skips.
    let (slower, faster) = if known_elapsed > stranger_elapsed {
        (known_elapsed, stranger_elapsed)
    } else {
        (stranger_elapsed, known_elapsed)
    };
    assert!(
        slower < faster + Duration::from_millis(500),
        "one branch took {slower:?} and the other {faster:?}; that gap is an account oracle"
    );

    // Both were really invited — a response that refused both would satisfy the
    // comparison above and invite nobody.
    assert_eq!(
        test_support::live_invitation_count(&db.pool, workspace).await?,
        2
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_invitation_creates_the_account_adds_membership_and_burns_once() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    assert_eq!(
        invite(&app, &owner, workspace, "newcomer@example.com", None)
            .await?
            .status(),
        StatusCode::ACCEPTED
    );
    let token = token_from(&mailer.message(0).await?)?;

    // Nobody by that address exists yet — the acceptance is what creates them.
    assert_eq!(
        test_support::user_id_for_email(&db.pool, "newcomer@example.com").await?,
        None
    );

    let response = app
        .clone()
        .oneshot(accept_request(&token, Some("New Comer"))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    let user_id: Uuid = body["user_id"].as_str().context("user_id")?.parse()?;
    assert_eq!(
        body["workspace_id"].as_str().context("ws")?,
        workspace.to_string()
    );

    assert!(test_support::is_member(&db.pool, workspace, user_id).await?);
    assert_eq!(
        test_support::user_id_for_email(&db.pool, "newcomer@example.com").await?,
        Some(user_id)
    );

    // Single use. A link that works twice is a link that works for whoever
    // reads the mailbox next.
    let second = app.clone().oneshot(accept_request(&token, None)?).await?;
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_expired_invitation_is_refused() -> Result<()> {
    // docs/40 gives an invitation seven days. The clock is moved rather than
    // the test waiting for it.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    invite(&app, &owner, workspace, "late@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;

    assert_eq!(
        test_support::expire_invitations(&db.pool, workspace).await?,
        1
    );

    let response = app.clone().oneshot(accept_request(&token, None)?).await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        test_support::user_id_for_email(&db.pool, "late@example.com").await?,
        None,
        "an expired invitation still created an account"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_revoked_invitation_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    invite(&app, &owner, workspace, "withdrawn@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;

    // Find it through the list, which is the only way an inviter gets the id —
    // the constant 202 deliberately does not return one.
    let listed = app
        .clone()
        .oneshot(
            request(
                &owner,
                "GET",
                &format!("/api/v1/workspaces/{workspace}/invitations"),
            )
            .header("x-workspace-id", workspace.to_string())
            .body(Body::empty())?,
        )
        .await?;
    assert_eq!(listed.status(), StatusCode::OK);
    let body = json_body(listed).await?;
    let id = body["data"][0]["id"].as_str().context("invitation id")?;
    assert_eq!(body["data"][0]["email"], "withdrawn@example.com");

    let revoked = app
        .clone()
        .oneshot(
            request(
                &owner,
                "DELETE",
                &format!("/api/v1/workspaces/{workspace}/invitations/{id}"),
            )
            .header("x-workspace-id", workspace.to_string())
            .body(Body::empty())?,
        )
        .await?;
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let response = app.clone().oneshot(accept_request(&token, None)?).await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_invitation_cannot_be_accepted_by_a_different_account() -> Result<()> {
    // docs/40 §Invitations: "tied to the address". Forwarding the email — which
    // people do, in good faith — must not hand membership to the wrong person.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    invite(&app, &owner, workspace, "intended@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;

    // Somebody else, signed in, holding the forwarded link.
    let bystander = sign_up(&app, &db.pool, "bystander@example.com").await?;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/invitations/accept")
                .header(header::COOKIE, &bystander.cookie)
                .header("x-csrf-token", &bystander.csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "token": token }).to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        !test_support::is_member(&db.pool, workspace, bystander.user_id).await?,
        "a forwarded invitation added the wrong person"
    );

    // And the intended recipient can still use it — a refusal that also burned
    // the invitation would lock out the person it was for.
    let intended = app.clone().oneshot(accept_request(&token, None)?).await?;
    assert_eq!(intended.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn accepting_while_signed_in_as_the_invited_address_works() -> Result<()> {
    // The companion to the refusal above: a check that compared the wrong thing
    // would satisfy that test and break every legitimate acceptance.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    let guest = sign_up(&app, &db.pool, "guest@example.com").await?;
    invite(&app, &owner, workspace, "guest@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/invitations/accept")
                .header(header::COOKIE, &guest.cookie)
                .header("x-csrf-token", &guest.csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "token": token }).to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(test_support::is_member(&db.pool, workspace, guest.user_id).await?);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn inviting_twice_leaves_only_the_newest_link_working() -> Result<()> {
    // Someone re-inviting because the first email was lost must not leave two
    // working links in one inbox.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    invite(&app, &owner, workspace, "twice@example.com", None).await?;
    let first = token_from(&mailer.message(0).await?)?;
    invite(&app, &owner, workspace, "twice@example.com", None).await?;
    let second = token_from(&mailer.message(1).await?)?;
    assert_ne!(first, second);
    assert_eq!(
        test_support::live_invitation_count(&db.pool, workspace).await?,
        1
    );

    let stale = app.clone().oneshot(accept_request(&first, None)?).await?;
    assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);
    let fresh = app.clone().oneshot(accept_request(&second, None)?).await?;
    assert_eq!(fresh.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_forged_verifier_against_a_real_selector_is_refused() -> Result<()> {
    // The reason the verifier is stored hashed at all.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    invite(&app, &owner, workspace, "target@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;
    let (selector, _) = token.split_once('.').context("selector.verifier")?;

    for bad in [
        format!("{selector}.{}", "0".repeat(48)),
        String::new(),
        "nonsense".to_owned(),
        "tf_pat_abc.def".to_owned(),
    ] {
        let response = app.clone().oneshot(accept_request(&bad, None)?).await?;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "accepted {bad:?}"
        );
    }

    assert_eq!(
        test_support::user_id_for_email(&db.pool, "target@example.com").await?,
        None
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_stored_invitation_is_not_a_usable_link() -> Result<()> {
    // docs/40 §Acceptance gates, "token-hash test": a database dump contains no
    // usable credential. Asserted against what is IN the table.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    invite(&app, &owner, workspace, "dump@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;
    let (_, verifier) = token.split_once('.').context("selector.verifier")?;

    for stored in test_support::invitation_columns(&db.pool, workspace).await? {
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
async fn an_invitation_in_another_workspace_is_invisible_and_unrevocable() -> Result<()> {
    // docs/04: absent and invisible are never disambiguated, and docs/32 says
    // no data crosses a workspace boundary.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let alice = sign_up(&app, &db.pool, "alice@example.com").await?;
    let alice_ws = create_workspace(&app, &alice, "alice-co").await?;
    invite(&app, &alice, alice_ws, "invitee@example.com", None).await?;

    let mallory = sign_up(&app, &db.pool, "mallory@example.com").await?;
    let mallory_ws = create_workspace(&app, &mallory, "mallory-co").await?;

    // Mallory cannot see Alice's invitations from her own workspace.
    let listed = app
        .clone()
        .oneshot(
            request(
                &mallory,
                "GET",
                &format!("/api/v1/workspaces/{mallory_ws}/invitations"),
            )
            .header("x-workspace-id", mallory_ws.to_string())
            .body(Body::empty())?,
        )
        .await?;
    let body = json_body(listed).await?;
    assert_eq!(body["data"].as_array().context("data")?.len(), 0);

    // Nor revoke one by id, even naming Alice's workspace in the path: the
    // membership check refuses her before the handler runs.
    let denied = app
        .clone()
        .oneshot(
            request(
                &mallory,
                "DELETE",
                &format!(
                    "/api/v1/workspaces/{alice_ws}/invitations/{}",
                    Uuid::now_v7()
                ),
            )
            .header("x-workspace-id", alice_ws.to_string())
            .body(Body::empty())?,
        )
        .await?;
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);

    assert_eq!(
        test_support::live_invitation_count(&db.pool, alice_ws).await?,
        1,
        "Alice's invitation was disturbed"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn inviting_with_a_role_the_inviter_does_not_hold_is_refused() -> Result<()> {
    // docs/04 control 1: you cannot grant what you do not hold. An invitation
    // carrying a role is a DEFERRED GRANT, and without this it would be a way
    // around `role.assign` — the escalation hole D-049 exists to prevent.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    // The INVITER must not be the owner. D-054 grants a workspace creator the
    // Owner role, which is asserted against `permission::ALL` — so the creator
    // legitimately holds every permission and control 1 has nothing to refuse.
    // This test needs someone who may invite and may not delete.
    let inviter = sign_up(&app, &db.pool, "inviter@example.com").await?;
    test_support::add_workspace_member(&db.pool, workspace, inviter.user_id).await?;
    test_support::grant_at_workspace(&db.pool, workspace, inviter.user_id, &["role.assign"])
        .await?;

    // A powerful role exists and somebody else holds it; the inviter does not.
    // Granted to a REAL account: `role_assignment.granted_by` is a foreign key,
    // so a made-up uuid fails the insert rather than the assertion.
    let admin = Uuid::now_v7();
    test_support::insert_user(&db.pool, admin, "admin@example.com", "Admin").await?;
    let powerful =
        test_support::grant_at_workspace(&db.pool, workspace, admin, &["workspace.delete"]).await?;

    let response = invite(
        &app,
        &inviter,
        workspace,
        "escalate@example.com",
        Some(powerful),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        test_support::live_invitation_count(&db.pool, workspace).await?,
        0,
        "a refused invitation was still created"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_invited_role_is_granted_on_acceptance_and_credited_to_the_inviter() -> Result<()> {
    // The companion to the refusal above, and the audit property: `granted_by`
    // is the INVITER. Recording the acceptor would read, years later, as a
    // self-grant.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    // The inviter holds the role they are handing out, and `role.assign`.
    let role = test_support::grant_at_workspace(
        &db.pool,
        workspace,
        owner.user_id,
        &["role.assign", "task.read"],
    )
    .await?;

    let response = invite(&app, &owner, workspace, "colleague@example.com", Some(role)).await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let token = token_from(&mailer.message(0).await?)?;

    let accepted = app.clone().oneshot(accept_request(&token, None)?).await?;
    assert_eq!(accepted.status(), StatusCode::OK);
    let body = json_body(accepted).await?;
    let user_id: Uuid = body["user_id"].as_str().context("user_id")?.parse()?;

    let grants = test_support::workspace_grants_for_user(&db.pool, workspace, user_id).await?;
    assert_eq!(grants.len(), 1, "the invited role was not granted");
    assert_eq!(grants[0].0, role);
    assert_eq!(
        grants[0].1, owner.user_id,
        "the grant was credited to the acceptor rather than the inviter"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_role_from_another_workspace_is_refused_at_invite_time() -> Result<()> {
    // Refused when the invitation is written, not when it is accepted — the
    // invitee would otherwise join with no role and nobody would know why.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let alice = sign_up(&app, &db.pool, "alice@example.com").await?;
    let alice_ws = create_workspace(&app, &alice, "alice-co").await?;
    let other_ws = create_workspace(&app, &alice, "other-co").await?;
    let foreign =
        test_support::grant_at_workspace(&db.pool, other_ws, alice.user_id, &["task.read"]).await?;
    test_support::grant_at_workspace(&db.pool, alice_ws, alice.user_id, &["role.assign"]).await?;

    let response = invite(&app, &alice, alice_ws, "x@example.com", Some(foreign)).await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(response).await?;
    assert_eq!(body["error"]["code"], "TF-VAL-0007");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_malformed_address_is_refused_before_anything_is_written() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    // The header-injection case first: it must never reach the mailer.
    for bad in ["nope", "a@b@c.com", "user@example.com\r\nBcc: x@y.com", ""] {
        let response = invite(&app, &owner, workspace, bad, None).await?;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "accepted {bad:?}"
        );
    }
    assert_eq!(
        test_support::live_invitation_count(&db.pool, workspace).await?,
        0
    );
    assert_eq!(mailer.count(), 0, "a malformed address reached the mailer");
    Ok(())
}
