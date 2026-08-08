//! What the dispatch loop's health gauges cost, and when they are read.
//!
//! `outbox_lag_seconds` is an aggregate over the pending set and
//! `outbox_dlq_depth` is an aggregate over the dead-lettered one. Both were read
//! once per poll, inside the transaction that also claims — so the two most
//! expensive queries in the dispatch path ran at the poll rate, and the poll
//! rate is highest under exactly the backlog that makes them slowest.
//!
//! There is no way to count round trips from the outside, so this asserts the
//! consequence instead: on a long sampling interval the gauge must still hold
//! its first reading after the queue has drained. A loop that re-read it every
//! poll would have overwritten that with zero.
//!
//! `#[ignore]` for the same reason as every other test here: Docker.

mod schema_harness;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use casual_task_model::{WorkspaceId, WorkspaceScope};
use casual_task_observability::recorder::Recorder as Metrics;
use casual_task_persistence::{Change, Provenance, Scoped, UnitOfWork, dispatch, test_support};
use casual_task_worker::dispatcher::{CancelOnDrop, Config, Consumer};
use sqlx::PgPool;
use uuid::Uuid;

/// Delivers everything, successfully, immediately.
struct Sink;

impl Consumer for Sink {
    fn name(&self) -> &'static str {
        "sse_fanout"
    }

    async fn deliver(&self, _event: &dispatch::Claimed) -> Result<(), String> {
        Ok(())
    }
}

async fn emit(pool: &PgPool, workspace: WorkspaceId, count: usize) -> Result<()> {
    for _ in 0..count {
        // A distinct aggregate each time: per-aggregate ordering serialises
        // deliveries for one aggregate, and this test needs the queue to drain.
        let aggregate = Uuid::now_v7();
        let mut tx = pool.begin().await?;
        let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
        UnitOfWork::record(
            &mut scoped,
            &Change {
                aggregate_type: "TASK".into(),
                aggregate_id: aggregate,
                project_id: None,
                event_type: "task.created".into(),
                activity_changes: serde_json::json!({}),
                audit_changes: serde_json::json!({}),
                payload: serde_json::json!({"task_id": aggregate}),
                schema_version: 1,
            },
            &Provenance {
                actor: None,
                actor_type: casual_task_model::ActorType::System,
                request_id: None,
                correlation_id: None,
                ip: None,
                user_agent: None,
            },
        )
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

/// One gauge's value, parsed rather than substring-matched.
///
/// `contains("... 0")` also matches `0.42`, which is how a test asserting a
/// drained queue passes against a backlog of half a second.
fn gauge(scraped: &str, series: &str) -> Option<f64> {
    scraped.lines().find_map(|line| {
        line.strip_prefix(series)
            .and_then(|rest| rest.strip_prefix(' '))
            .and_then(|value| value.trim().parse().ok())
    })
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn the_health_gauges_are_read_on_their_own_cadence_not_once_per_poll() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = WorkspaceId::new();
    test_support::insert_workspace(&db.pool, workspace.as_uuid(), "alpha").await?;
    emit(&db.pool, workspace, 4).await?;

    // So the first reading is unambiguously non-zero. Without this the whole
    // assertion could rest on a sub-millisecond lag that renders as "0".
    tokio::time::sleep(Duration::from_millis(120)).await;

    let consumer = Arc::new(Sink);
    let metrics = Arc::new(Metrics::new());
    let (stop, cancel) = CancelOnDrop::new();
    let worker = {
        let pool = db.pool.clone();
        let consumer = Arc::clone(&consumer);
        let metrics = Arc::clone(&metrics);
        tokio::spawn(async move {
            casual_task_worker::dispatcher::run(
                &pool,
                consumer,
                "worker-1",
                Config {
                    batch: 8,
                    concurrency: 8,
                    idle: Duration::from_millis(10),
                    drain: Duration::from_secs(5),
                    // Longer than this test can possibly run: exactly one
                    // sample, taken on the first poll.
                    metrics_interval: Duration::from_secs(3600),
                },
                cancel,
                metrics,
            )
            .await
        })
    };

    // Long enough for dozens of polls at a 10 ms idle interval.
    tokio::time::sleep(Duration::from_millis(700)).await;
    stop.cancel();
    worker.await??;

    // The queue really did drain — otherwise a stale non-zero gauge would prove
    // nothing, because the true value would still be non-zero.
    let settled = test_support::counts(&db.pool, "sse_fanout").await?;
    assert_eq!(
        settled.outstanding, 0,
        "{} deliveries are still pending; this test cannot distinguish a stale \
         gauge from a correct one until the queue is empty",
        settled.outstanding
    );

    let scraped = metrics.render();
    let lag = gauge(&scraped, r#"outbox_lag_seconds{consumer="sse_fanout"}"#);
    let lag = lag.unwrap_or_else(|| panic!("the lag gauge was never recorded:\n{scraped}"));
    assert!(
        lag > 0.0,
        "the lag gauge reads {lag} after a drained queue and a one-hour sampling \
         interval, which means it was re-read on a later poll — the O(pending) \
         aggregate is still tied to the poll rate"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn the_lag_gauge_still_excludes_backoff_and_dead_letters() -> Result<()> {
    // D-047, restated against the query that no longer joins outbox_event. The
    // exclusions are the reason the gauge is worth paging on: counting a row
    // that is waiting on purpose makes the signal rise during normal retries,
    // and counting a dead letter makes it stay high forever after one permanent
    // failure.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = WorkspaceId::new();
    test_support::insert_workspace(&db.pool, workspace.as_uuid(), "alpha").await?;
    emit(&db.pool, workspace, 1).await?;

    let mut tx = db.pool.begin().await?;
    let mut d = dispatch::Dispatcher::assume(&mut tx).await?;
    let claimed = dispatch::claim(&mut d, "sse_fanout", "worker-1", 10).await?;
    let pending = dispatch::oldest_pending_seconds(&mut d, "sse_fanout").await?;
    tx.commit().await?;
    assert_eq!(claimed.len(), 1);
    assert!(
        pending.is_some(),
        "a claimed-but-undelivered row stopped counting as lag: a worker that \
         claims and then stalls would read as healthy"
    );

    // Into the backoff window.
    let mut tx = db.pool.begin().await?;
    let mut d = dispatch::Dispatcher::assume(&mut tx).await?;
    dispatch::failed(&mut d, claimed[0].delivery_id, 1, "502").await?;
    let waiting = dispatch::oldest_pending_seconds(&mut d, "sse_fanout").await?;
    tx.commit().await?;
    assert_eq!(
        waiting, None,
        "a delivery inside its backoff window counted as lag; the primary health \
         signal would rise during ordinary retry behaviour"
    );

    // And out of the ladder entirely.
    let mut tx = db.pool.begin().await?;
    let mut d = dispatch::Dispatcher::assume(&mut tx).await?;
    let dead = dispatch::failed(
        &mut d,
        claimed[0].delivery_id,
        dispatch::BACKOFF.len() as i32 + 1,
        "502",
    )
    .await?;
    let after = dispatch::oldest_pending_seconds(&mut d, "sse_fanout").await?;
    tx.commit().await?;
    assert!(dead);
    assert_eq!(
        after, None,
        "a dead-lettered delivery still counts as lag: one permanent failure \
         would hold the paging signal high forever"
    );
    Ok(())
}
