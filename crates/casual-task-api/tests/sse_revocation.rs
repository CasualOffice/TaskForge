//! `docs/40`'s revocation gate, for the channel that has no next request.
//!
//! > "A revoked session is rejected on the next request; **an SSE stream held by
//! > that session closes.**"
//!
//! The first half was already true — every request re-authenticates. The second
//! half was not: a stream was authorized once at connect and then ran until the
//! client left or the process stopped, so a session destroyed at 10:00 kept
//! receiving events. That is not a slow revocation, it is none.
//!
//! These drive `sse::revalidate::watch` against a real database rather than the
//! HTTP endpoint, because what is worth asserting here is the *decision* — does
//! a revoked credential end the subscription — and the framing around it is
//! asserted by unit tests that need no container.
//!
//! `#[ignore]` for the same reason as every other test here: Docker.

mod schema_harness;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use casual_task_api::middleware::WorkspaceMember;
use casual_task_api::server::AppState;
use casual_task_api::sse::revalidate::{self, Ended, Watch};
use casual_task_infra::broadcast::{Received, Topic};
use casual_task_model::{ActorType, AuthContext, UserId, WorkspaceId};
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::{identity, test_support};
use time::OffsetDateTime;
use uuid::Uuid;

/// Short enough that a test does not wait out the production window, long
/// enough that the first tick does not race the setup.
const TICK: Duration = Duration::from_millis(150);

const PASSWORD: &str = "a sufficiently long password";

struct Fixture {
    state: AppState,
    epoch: i64,
    member: WorkspaceMember,
    workspace: WorkspaceId,
    session_id: Uuid,
    cookie: axum::http::HeaderMap,
}

/// A user with a live session, a workspace, and the state a watch needs.
async fn fixture(pool: sqlx::PgPool) -> Result<Fixture> {
    let user = Uuid::now_v7();
    test_support::insert_user_with_password(
        &pool,
        user,
        "watcher@example.test",
        &casual_task_identity::password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;

    let workspace = WorkspaceId::new();
    test_support::insert_workspace(&pool, workspace.as_uuid(), "alpha").await?;

    // A real session credential, minted the way login mints one, so the
    // revalidation path is re-presented exactly what it would see in production.
    let minted = casual_task_identity::credential::mint().expect("entropy");
    let (selector, _) = casual_task_identity::credential::split(&minted.presented).expect("shape");
    let mut conn = pool.acquire().await?;
    let session_id = identity::create_session(
        &mut conn,
        user,
        selector,
        &minted.verifier_hash,
        "password",
        OffsetDateTime::now_utc() + time::Duration::hours(1),
        None,
        None,
    )
    .await?;
    drop(conn);

    let mut cookie = axum::http::HeaderMap::new();
    cookie.insert(
        axum::http::header::COOKIE,
        axum::http::HeaderValue::from_str(&format!("tf_session={}", minted.presented))
            .expect("a valid cookie"),
    );

    let pool_for_epoch = pool.clone();
    let state = AppState {
        pool,
        broadcast: casual_task_api::sse::local_hub(),
        metrics: Arc::new(Recorder::new()),
        secret_key: "a-test-secret-key-long-enough-for-hmac".into(),
        public_url: "https://tasks.example.test".into(),
        mailer: Arc::new(casual_task_infra::mail::LoggingMailer),
    };

    let epoch = test_support::authz_epoch(&pool_for_epoch, workspace.as_uuid()).await?;

    Ok(Fixture {
        state,
        epoch,
        member: WorkspaceMember {
            context: AuthContext::authenticated(
                UserId::from_uuid(user),
                workspace,
                ActorType::User,
            ),
        },
        workspace,
        session_id,
        cookie,
    })
}

fn a_watch(f: &Fixture, canceller: casual_task_infra::broadcast::Canceller) -> Watch {
    Watch {
        state: f.state.clone(),
        headers: f.cookie.clone(),
        member: f.member.clone(),
        // A project this actor cannot see — which is fine, and deliberate: the
        // authorization branch only runs when the epoch has moved, and this
        // fixture starts at the workspace's real epoch. A watch that consulted
        // authorization every tick would cancel here for the wrong reason, and
        // the control assertion below is what catches that.
        project: Uuid::now_v7(),
        epoch: f.epoch,
        canceller,
        request_id: "test".to_owned(),
        interval: TICK,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn revoking_a_session_closes_the_stream_it_holds() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let f = fixture(db.pool.clone()).await?;

    let topic = Topic::project(f.workspace, Uuid::now_v7());
    let mut subscription = f.state.broadcast.subscribe(topic);
    let watcher = tokio::spawn(revalidate::watch(a_watch(&f, subscription.canceller())));

    // The control. A live session must NOT be cancelled — a watch that closed
    // every stream would pass the assertion below while making live updates
    // useless, and that failure is invisible if the only check is "it stopped".
    tokio::time::sleep(TICK * 4).await;
    assert!(
        !watcher.is_finished(),
        "the watch ended a stream whose session is still live"
    );
    assert_eq!(
        f.state.broadcast.subscriber_count(),
        1,
        "a live stream stopped being counted"
    );

    // Revoke, exactly as `POST /auth/logout` and an admin revocation do.
    let mut conn = db.pool.acquire().await?;
    identity::revoke_session(&mut conn, f.session_id).await?;
    drop(conn);

    // The stream must end, and say why: a client told only "closed" would
    // reconnect with the same dead credential.
    let ended = tokio::time::timeout(Duration::from_secs(10), subscription.recv())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "the stream was still open ten seconds after its session was \
                 revoked; docs/40 requires it to close"
            )
        })?;
    assert_eq!(ended, Received::Cancelled);

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), watcher).await??,
        Some(Ended::CredentialRevoked),
        "the stream closed for some reason other than the revoked credential"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn a_revoked_stream_stops_being_counted_as_an_open_connection() -> Result<()> {
    // `sse_connections_active` must not include a stream that has been cancelled
    // but whose task has not been polled yet — during a revocation incident that
    // is exactly when an operator is reading the gauge.
    let db = schema_harness::TestDatabase::start().await?;
    let f = fixture(db.pool.clone()).await?;

    let topic = Topic::project(f.workspace, Uuid::now_v7());
    // Deliberately never polled after this point: the count must fall without
    // the stream task doing anything at all.
    let subscription = f.state.broadcast.subscribe(topic);
    tokio::spawn(revalidate::watch(a_watch(&f, subscription.canceller())));
    assert_eq!(f.state.broadcast.subscriber_count(), 1);

    let mut conn = db.pool.acquire().await?;
    identity::revoke_session(&mut conn, f.session_id).await?;
    drop(conn);

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while f.state.broadcast.subscriber_count() > 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(TICK).await;
    }
    assert_eq!(
        f.state.broadcast.subscriber_count(),
        0,
        "a cancelled subscription is still counted as an open connection"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn the_watch_stops_when_the_client_disconnects() -> Result<()> {
    // Otherwise every closed stream leaves a task behind querying the database
    // on its behalf — a leak that only appears under the traffic that caused it.
    let db = schema_harness::TestDatabase::start().await?;
    let f = fixture(db.pool.clone()).await?;

    let topic = Topic::project(f.workspace, Uuid::now_v7());
    let subscription = f.state.broadcast.subscribe(topic);
    let watcher = tokio::spawn(revalidate::watch(a_watch(&f, subscription.canceller())));

    drop(subscription);

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), watcher).await??,
        None,
        "the watch outlived the stream it was watching"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn a_grant_change_reaches_an_open_stream() -> Result<()> {
    // `docs/05`: "Membership is revalidated on every `authz_epoch` change, not
    // only at connect. A revoked user's stream closes within one epoch bump — a
    // long-lived stream is otherwise a permission-revocation hole."
    //
    // The credential stays perfectly valid throughout. What changes is the
    // authority behind it, which is the half a session check alone cannot see.
    let db = schema_harness::TestDatabase::start().await?;
    let f = fixture(db.pool.clone()).await?;

    let topic = Topic::project(f.workspace, Uuid::now_v7());
    let mut subscription = f.state.broadcast.subscribe(topic);
    let watcher = tokio::spawn(revalidate::watch(a_watch(&f, subscription.canceller())));

    // The control again: nothing has moved, so nothing may close.
    tokio::time::sleep(TICK * 4).await;
    assert!(
        !watcher.is_finished(),
        "the stream closed while its epoch and its session were both unchanged"
    );

    // Any grant, role, team or project membership change bumps this in the same
    // transaction (docs/04). Bumping it directly is the cheapest way to say
    // "something that could change the answer happened".
    let mut tx = db.pool.begin().await?;
    let scope = f.member.context.scope();
    let mut scoped = casual_task_persistence::Scoped::apply(&mut tx, &scope).await?;
    casual_task_persistence::workspace::bump_authz_epoch(&mut scoped).await?;
    tx.commit().await?;

    let ended = tokio::time::timeout(Duration::from_secs(10), subscription.recv())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "the stream survived an authz_epoch bump that withdrew its \
                 authority; docs/05 requires membership to be revalidated on \
                 every epoch change"
            )
        })?;
    assert_eq!(ended, Received::Cancelled);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), watcher).await??,
        Some(Ended::AuthorityWithdrawn),
        "the stream closed, but not for the reason the epoch bump gave it"
    );
    Ok(())
}
