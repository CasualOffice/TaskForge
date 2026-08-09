//! `GET /api/v1/stream`, end to end, with frames read off the wire (C-015).
//!
//! # What this proves that nothing else did
//!
//! Every mechanism behind live updates was already asserted where it lives —
//! the permission filter in `sse::authorize`, fan-out and replay in
//! `casual-task-infra`, the window in `sse::coalesce`, revocation in
//! `sse_revocation.rs`. What none of them touched is the **assembly**: that the
//! handler wires those parts together in the order the client depends on.
//!
//! Three things can only be seen from out here, and each has been wrong in a
//! version of this code:
//!
//! - a replayed backlog arrives **before** any live frame, so a resumed client
//!   never applies a newer update on top of an older one;
//! - a burst inside one window arrives as **one** frame, not one per row;
//! - a subscriber is fed its own project and nothing else, through the endpoint
//!   rather than merely through the hub.
//!
//! # Why events are published through the hub rather than by writing tasks
//!
//! The outbox path — write a task, let the dispatcher claim it, let
//! `SseFanout` publish — is covered by its own tests at each hop. Driving it
//! here would make every assertion below wait on a dispatch loop's poll
//! interval, and a timing failure in that loop would read as a broken stream.
//! This publishes at the same seam the consumer does, which is the last point
//! before the code under test.
//!
//! `#[ignore]` for the same reason as every other test here: Docker.

mod schema_harness;

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::middleware::WORKSPACE_HEADER;
use casual_task_api::server::{AppState, router};
use casual_task_identity::password;
use casual_task_infra::broadcast::{LiveEvent, Topic};
use casual_task_model::WorkspaceId;
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use futures_core::Stream;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";
const SECRET: &str = "a-test-secret-key-long-enough-for-hmac";

/// A signed-in member of a workspace, a project they can read, and — the part
/// that matters — the **same** hub the router publishes through.
///
/// Every other test file in this crate builds a router per caller. That would
/// be wrong here: a second router is a second `LocalBroadcast`, so the test
/// would publish into a hub nobody is subscribed to and assert that no frames
/// arrive, which is exactly the false pass this file exists to avoid.
struct Fixture {
    app: Router,
    state: AppState,
    cookie: String,
    workspace: Uuid,
    project: Uuid,
}

impl Fixture {
    async fn build(pool: &sqlx::PgPool) -> Result<Self> {
        let workspace = Uuid::now_v7();
        test_support::insert_workspace(pool, workspace, "alpha").await?;

        let user = Uuid::now_v7();
        test_support::insert_user_with_password(
            pool,
            user,
            "watcher@example.test",
            &password::hash_chosen(PASSWORD).expect("hashes"),
        )
        .await?;
        test_support::add_workspace_member(pool, workspace, user).await?;
        // A real role_assignment, not a flag: migration 0003 makes that the only
        // source of authority, and a test that bypassed it would prove the
        // handler runs and nothing about whether the resolver is consulted.
        test_support::grant_at_workspace(pool, workspace, user, &["project.create", "task.read"])
            .await?;

        let state = state(pool.clone());
        let app = router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({"email": "watcher@example.test", "password": PASSWORD})
                            .to_string(),
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
            .ok_or_else(|| anyhow!("no session cookie"))?
            .to_owned();
        let body = to_bytes(response.into_body(), 64 * 1024).await?;
        let csrf = serde_json::from_slice::<serde_json::Value>(&body)?["csrf_token"]
            .as_str()
            .ok_or_else(|| anyhow!("no csrf token"))?
            .to_owned();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::COOKIE, &cookie)
                    .header("x-csrf-token", &csrf)
                    .header(WORKSPACE_HEADER, workspace.to_string())
                    .header("idempotency-key", Uuid::now_v7().to_string())
                    .body(Body::from(
                        serde_json::json!({"key": "WR", "name": "Work", "visibility": "WORKSPACE"})
                            .to_string(),
                    ))?,
            )
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "project create failed"
        );
        let body = to_bytes(response.into_body(), 64 * 1024).await?;
        let project: Uuid = serde_json::from_slice::<serde_json::Value>(&body)?["id"]
            .as_str()
            .ok_or_else(|| anyhow!("no project id"))?
            .parse()?;

        Ok(Self {
            app,
            state,
            cookie,
            workspace,
            project,
        })
    }

    /// The topic the endpoint will subscribe this fixture's stream to.
    fn topic(&self) -> Topic {
        Topic::project(WorkspaceId::from_uuid(self.workspace), self.project)
    }

    /// Open a stream, optionally resuming from `last_event_id`.
    async fn open(&self, last_event_id: Option<Uuid>) -> Result<Frames> {
        let mut request = Request::builder()
            .uri(format!("/api/v1/stream?project_id={}", self.project))
            .header(header::COOKIE, &self.cookie)
            .header(WORKSPACE_HEADER, self.workspace.to_string())
            .header(header::ACCEPT, "text/event-stream");
        if let Some(id) = last_event_id {
            request = request.header("last-event-id", id.to_string());
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::empty())?)
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "the stream was refused to a member holding task.read"
        );
        Ok(Frames {
            body: response.into_body().into_data_stream(),
            buffer: String::new(),
        })
    }

    fn publish(&self, event: &LiveEvent) -> usize {
        self.state.broadcast.publish(self.topic(), event.clone())
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

/// One `event:` / `id:` / `data:` block off the wire.
#[derive(Debug, PartialEq, Eq)]
struct Frame {
    event: String,
    id: Option<String>,
    data: String,
}

/// The response body, parsed into SSE frames as they arrive.
///
/// Reading the whole body is not an option: the stream never ends on its own,
/// so `to_bytes` would hang until the test timed out. This reads incrementally,
/// which is also the only way to assert that a burst produced *one* frame —
/// that claim is about what did **not** arrive, and needs a bounded wait.
struct Frames {
    body: axum::body::BodyDataStream,
    buffer: String,
}

impl Frames {
    /// The next frame, or `None` if none arrived within `within`.
    ///
    /// Keep-alive comments are skipped: `docs/05`'s heartbeat is a `:` line, it
    /// carries no event, and a test that counted it as one would be asserting
    /// against the clock.
    async fn next(&mut self, within: Duration) -> Option<Frame> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if let Some(frame) = self.take_buffered() {
                return Some(frame);
            }
            let chunk = tokio::time::timeout_at(deadline, async {
                std::future::poll_fn(|cx| Pin::new(&mut self.body).poll_next(cx)).await
            })
            .await;
            match chunk {
                Ok(Some(Ok(bytes))) => self.buffer.push_str(&String::from_utf8_lossy(&bytes)),
                // End of stream, a body error, or the deadline: all mean "no
                // frame", and the caller's assertion says which it expected.
                Ok(Some(Err(_)) | None) | Err(_) => return None,
            }
        }
    }

    fn take_buffered(&mut self) -> Option<Frame> {
        while let Some(end) = self.buffer.find("\n\n") {
            let block = self.buffer[..end].to_owned();
            self.buffer.drain(..end + 2);

            let mut event = None;
            let mut id = None;
            let mut data = String::new();
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("event:") {
                    event = Some(rest.trim().to_owned());
                } else if let Some(rest) = line.strip_prefix("id:") {
                    id = Some(rest.trim().to_owned());
                } else if let Some(rest) = line.strip_prefix("data:") {
                    data.push_str(rest.trim());
                }
            }
            if let Some(event) = event {
                return Some(Frame { event, id, data });
            }
            // A comment-only block (the heartbeat). Keep draining.
        }
        None
    }
}

fn event_for(aggregate: Uuid, event_type: &str, data: &str) -> LiveEvent {
    LiveEvent {
        id: Uuid::now_v7(),
        aggregate_id: aggregate,
        event_type: event_type.to_owned(),
        data: data.to_owned(),
    }
}

/// Long enough to absorb a slow container, short enough that a hung stream
/// fails the test rather than the suite.
const ARRIVES: Duration = Duration::from_secs(5);

/// A window and a bit: what a frame gets to not arrive in.
const SETTLES: Duration = Duration::from_millis(400);

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn a_subscriber_receives_a_live_frame() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let f = Fixture::build(&db.pool).await?;
    let mut frames = f.open(None).await?;

    let event = event_for(Uuid::now_v7(), "task.updated", r#"{"title":"one"}"#);
    // Retried briefly: the handler subscribes while the response is being
    // returned, so a publish issued immediately can land before the subscriber
    // is registered. That is a real race for a client too, and the answer for
    // both is that the next event arrives.
    for _ in 0..20 {
        if f.publish(&event) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let frame = frames
        .next(ARRIVES)
        .await
        .ok_or_else(|| anyhow!("no frame arrived on a live stream"))?;
    assert_eq!(frame.event, "task.updated");
    assert_eq!(frame.data, r#"{"title":"one"}"#);
    assert_eq!(
        frame.id.as_deref(),
        Some(event.id.to_string().as_str()),
        "the frame's id must be the event's own id — it is what comes back as \
         Last-Event-ID"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn a_burst_on_one_task_arrives_as_one_frame() -> Result<()> {
    // docs/05: "a rapid drag produces one update, not forty". Asserted from
    // outside, where it is a claim about the bytes on the wire rather than about
    // a buffer's contents.
    let db = schema_harness::TestDatabase::start().await?;
    let f = Fixture::build(&db.pool).await?;
    let mut frames = f.open(None).await?;

    let task = Uuid::now_v7();
    for _ in 0..20 {
        if f.publish(&event_for(task, "task.updated", r#"{"rank":0}"#)) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // The rest of the drag, inside one window.
    for rank in 1..40 {
        f.publish(&event_for(
            task,
            "task.updated",
            &format!(r#"{{"rank":{rank}}}"#),
        ));
    }

    let frame = frames
        .next(ARRIVES)
        .await
        .ok_or_else(|| anyhow!("the coalesced burst never arrived"))?;
    assert_eq!(frame.event, "task.updated");
    assert_eq!(
        frame.data, r#"{"rank":39}"#,
        "the collapsed frame carried an intermediate position, not the final one"
    );
    assert_eq!(
        frames.next(SETTLES).await,
        None,
        "the drag produced more than one frame; the 100 ms window is not \
         collapsing on the wire"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn a_reconnect_replays_what_was_missed_and_then_continues() -> Result<()> {
    // The whole point of Last-Event-ID, and the assertion that the replay
    // backlog precedes live traffic rather than racing it.
    let db = schema_harness::TestDatabase::start().await?;
    let f = Fixture::build(&db.pool).await?;

    let first = event_for(Uuid::now_v7(), "task.updated", r#"{"n":1}"#);
    {
        let mut frames = f.open(None).await?;
        for _ in 0..20 {
            if f.publish(&first) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let seen = frames
            .next(ARRIVES)
            .await
            .ok_or_else(|| anyhow!("the first stream received nothing"))?;
        assert_eq!(seen.id.as_deref(), Some(first.id.to_string().as_str()));
        // The socket dies here.
    }

    // Published while nobody is connected — the case a delivery-fed buffer
    // would miss entirely.
    let missed_one = event_for(Uuid::now_v7(), "task.updated", r#"{"n":2}"#);
    let missed_two = event_for(Uuid::now_v7(), "task.updated", r#"{"n":3}"#);
    f.publish(&missed_one);
    f.publish(&missed_two);

    let mut frames = f.open(Some(first.id)).await?;
    let replayed_one = frames
        .next(ARRIVES)
        .await
        .ok_or_else(|| anyhow!("a reconnect with Last-Event-ID replayed nothing"))?;
    let replayed_two = frames
        .next(ARRIVES)
        .await
        .ok_or_else(|| anyhow!("only one of the two missed events was replayed"))?;
    assert_eq!(
        [replayed_one.data.as_str(), replayed_two.data.as_str()],
        [r#"{"n":2}"#, r#"{"n":3}"#],
        "the replay arrived out of order, or carried the wrong events"
    );

    // ...and the stream is live, not merely a history dump that then stops.
    let live = event_for(Uuid::now_v7(), "task.updated", r#"{"n":4}"#);
    for _ in 0..20 {
        if f.publish(&live) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let frame = frames
        .next(ARRIVES)
        .await
        .ok_or_else(|| anyhow!("the resumed stream replayed and then went dead"))?;
    assert_eq!(frame.data, r#"{"n":4}"#);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn a_stream_is_fed_its_own_project_and_nothing_else() -> Result<()> {
    // The widest-blast-radius property in the product, asserted through the
    // endpoint rather than through the hub. A topic comparison that dropped
    // either half would pass every hub-level test and leak here.
    let db = schema_harness::TestDatabase::start().await?;
    let f = Fixture::build(&db.pool).await?;
    let mut frames = f.open(None).await?;

    // Same project id, different workspace.
    let elsewhere = Topic::project(WorkspaceId::new(), f.project);
    f.state
        .broadcast
        .publish(elsewhere, event_for(Uuid::now_v7(), "task.updated", "leak"));
    // Same workspace, different project.
    let other_project = Topic::project(WorkspaceId::from_uuid(f.workspace), Uuid::now_v7());
    f.state.broadcast.publish(
        other_project,
        event_for(Uuid::now_v7(), "task.updated", "leak"),
    );

    assert_eq!(
        frames.next(SETTLES).await,
        None,
        "a subscriber received an event addressed to another workspace or project"
    );

    // The counterweight: the stream is alive and would have delivered one.
    // Without this the assertion above passes against a stream that is simply
    // broken.
    let mine = event_for(Uuid::now_v7(), "task.updated", "mine");
    for _ in 0..20 {
        if f.publish(&mine) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let frame = frames.next(ARRIVES).await.ok_or_else(|| {
        anyhow!("the stream was not live, so the isolation assertion proved nothing")
    })?;
    assert_eq!(frame.data, "mine");
    Ok(())
}
