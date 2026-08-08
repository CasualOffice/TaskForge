//! The at-least-once acceptance gate (`docs/25` §Acceptance gates).
//!
//! > "Kill the dispatcher mid-batch; assert every event is delivered, some
//! > twice, none lost."
//!
//! This is the test the whole outbox exists to pass. Everything else — the
//! transaction boundaries, the claim expiry, the retry ladder — is machinery in
//! service of it, and each of those can be individually correct while the system
//! still loses an event at the seam between them.
//!
//! It is deliberately not a unit test of the loop. A killed worker is a
//! *process* fact: rows left claimed, a connection dropped mid-flight, another
//! worker arriving later. Only a real database shows that.

mod schema_harness;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use casual_task_model::{WorkspaceId, WorkspaceScope};
use casual_task_observability::recorder::Recorder as Metrics;
use casual_task_persistence::{Change, Provenance, Scoped, UnitOfWork, dispatch, test_support};
use casual_task_worker::dispatcher::{Cancel, CancelOnDrop, Config, Consumer, Stopped};
use sqlx::PgPool;
use std::sync::Mutex;
use uuid::Uuid;

/// Records what it was given, and can be told to hang.
struct Recorder {
    delivered: Mutex<Vec<Uuid>>,
    /// Deliveries after this many block until cancelled — the "mid-batch" in
    /// "kill the dispatcher mid-batch".
    hang_after: usize,
    seen: AtomicUsize,
}

impl Recorder {
    fn new(hang_after: usize) -> Self {
        Self {
            delivered: Mutex::new(Vec::new()),
            hang_after,
            seen: AtomicUsize::new(0),
        }
    }

    fn event_ids(&self) -> Vec<Uuid> {
        self.delivered.lock().expect("not poisoned").clone()
    }
}

impl Consumer for Recorder {
    fn name(&self) -> &'static str {
        "sse_fanout"
    }

    async fn deliver(&self, event: &dispatch::Claimed) -> Result<(), String> {
        let n = self.seen.fetch_add(1, Ordering::SeqCst);
        if n >= self.hang_after {
            // A consumer that has received the event but will never return —
            // the worst case for at-least-once, because the delivery HAPPENED
            // and the outcome was never recorded.
            self.delivered
                .lock()
                .expect("not poisoned")
                .push(event.event_id);
            std::future::pending::<()>().await;
        }
        self.delivered
            .lock()
            .expect("not poisoned")
            .push(event.event_id);
        Ok(())
    }
}

async fn a_workspace(pool: &PgPool) -> Result<WorkspaceId> {
    // Through casual-task-persistence, not raw SQL: docs/19 puts every query in
    // that crate and casual-task-lint enforces it. See its `test_support`
    // module for why the fixture lives there rather than here.
    let w = WorkspaceId::new();
    test_support::insert_workspace(pool, w.as_uuid(), "alpha").await?;
    Ok(w)
}

async fn emit(pool: &PgPool, workspace: WorkspaceId, count: usize) -> Result<Vec<Uuid>> {
    let mut ids = Vec::new();
    for _ in 0..count {
        // A distinct aggregate each time: per-aggregate ordering deliberately
        // serialises deliveries for the same aggregate, and this test is about
        // batches, not ordering.
        let aggregate = Uuid::now_v7();
        let mut tx = pool.begin().await?;
        let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
        let event_id = UnitOfWork::record(
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
        ids.push(event_id);
    }
    Ok(ids)
}

fn config() -> Config {
    Config {
        batch: 8,
        concurrency: 8,
        idle: Duration::from_millis(50),
        // Short, because this test *wants* the drain to expire: that is what
        // "killed mid-batch" means here.
        drain: Duration::from_millis(200),
        // Short for the opposite reason to production's five seconds: assertion
        // 4 below reads the gauge after the queue drains, and on the production
        // cadence this test would end before the second sample. The cadence
        // itself is asserted in dispatch_metrics.rs.
        metrics_interval: Duration::from_millis(50),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn no_event_is_lost_when_the_dispatcher_is_killed_mid_batch() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool).await?;
    let emitted = emit(&db.pool, workspace, 8).await?;

    // --- Worker 1: delivers three, then hangs on the rest and is killed. -----
    let victim = Arc::new(Recorder::new(3));
    let (stop, cancel) = CancelOnDrop::new();
    let worker = {
        let pool = db.pool.clone();
        let consumer = Arc::clone(&victim);
        tokio::spawn(async move {
            casual_task_worker::dispatcher::run(
                &pool,
                consumer,
                "worker-1",
                config(),
                cancel,
                Arc::new(Metrics::new()),
            )
            .await
        })
    };

    // Let it claim and start delivering, then kill it.
    tokio::time::sleep(Duration::from_millis(600)).await;
    stop.cancel();
    let stopped = worker.await??;
    assert_eq!(
        stopped,
        Stopped::DrainTimedOut { abandoned: 5 },
        "the worker drained cleanly; this test needs it to die mid-batch"
    );

    let after_kill = test_support::counts(&db.pool, "sse_fanout").await?;
    assert_eq!(
        after_kill.dispatched, 3,
        "expected three recorded outcomes, got {}",
        after_kill.dispatched
    );
    // The abandoned rows are still CLAIMED. Nothing else may take them yet —
    // this is the window in which a naive implementation loses them.
    assert_eq!(
        after_kill.claimed, 5,
        "the abandoned deliveries are not claimed"
    );

    // --- Time passes: the claim expires. ------------------------------------
    // Simulated rather than waited out. Testing crash recovery by sleeping five
    // minutes means it is tested once and then disabled.
    assert_eq!(test_support::expire_all_claims(&db.pool).await?, 5);

    // --- Worker 2: healthy. Must pick up everything the first one dropped. ---
    let survivor = Arc::new(Recorder::new(usize::MAX));
    let metrics = Arc::new(Metrics::new());
    let (stop2, cancel2) = CancelOnDrop::new();
    let worker2 = {
        let pool = db.pool.clone();
        let consumer = Arc::clone(&survivor);
        let metrics = Arc::clone(&metrics);
        tokio::spawn(async move {
            casual_task_worker::dispatcher::run(
                &pool,
                consumer,
                "worker-2",
                config(),
                cancel2,
                metrics,
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(900)).await;
    stop2.cancel();
    worker2.await??;

    // --- The three assertions the gate names. -------------------------------

    // 1. NONE LOST. Every emitted event reached the consumer at least once.
    let mut reached: Vec<Uuid> = victim.event_ids();
    reached.extend(survivor.event_ids());
    for event_id in &emitted {
        assert!(
            reached.contains(event_id),
            "event {event_id} was never delivered to any worker — an event was \
             lost, which is the one outcome the outbox exists to prevent"
        );
    }

    // 2. SOME TWICE. The five the first worker had in flight were delivered by
    //    it and again by the second. At-least-once is a guarantee about the
    //    floor, and this is the ceiling it declines to promise.
    let duplicates = emitted
        .iter()
        .filter(|id| reached.iter().filter(|d| *d == *id).count() > 1)
        .count();
    assert!(
        duplicates > 0,
        "nothing was delivered twice — either the abandoned deliveries were \
         silently dropped, or this test no longer kills the worker mid-flight"
    );

    // 3. ALL SETTLED. Nothing is left pending: the queue is empty, not merely
    //    quiet.
    let settled = test_support::counts(&db.pool, "sse_fanout").await?;
    assert_eq!(
        settled.outstanding, 0,
        "{} deliveries are still pending after a healthy worker ran",
        settled.outstanding
    );

    // 4. AND IT WAS OBSERVABLE. A dispatcher that delivers correctly and emits
    //    nothing is indistinguishable from one that is not running at all —
    //    which is the question RB-01 step 3 exists to answer.
    let scraped = metrics.render();
    assert!(
        scraped.contains(r#"outbox_dispatch_total{consumer="sse_fanout",outcome="dispatched"}"#),
        "the dispatch counter was never recorded:\n{scraped}"
    );
    assert!(
        scraped.contains(r#"outbox_lag_seconds{consumer="sse_fanout"}"#),
        "the lag gauge was never recorded:\n{scraped}"
    );
    // Drained, so the gauge must read zero rather than keeping its last value —
    // a gauge that stops being written reports a backlog forever.
    assert!(
        scraped.contains(r#"outbox_lag_seconds{consumer="sse_fanout"} 0"#),
        "lag did not return to zero after the queue drained:\n{scraped}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn a_cancelled_worker_with_nothing_in_flight_drains_immediately() -> Result<()> {
    // The common case, and the one an orchestrator sees on every rolling
    // deploy. A worker that always waited out its drain would add its full
    // timeout to every pod replacement.
    let db = schema_harness::TestDatabase::start().await?;
    let consumer = Arc::new(Recorder::new(usize::MAX));
    let (stop, cancel) = CancelOnDrop::new();
    let pool = db.pool.clone();
    let worker = tokio::spawn(async move {
        casual_task_worker::dispatcher::run(
            &pool,
            consumer,
            "worker-1",
            config(),
            cancel,
            Arc::new(Metrics::new()),
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(150)).await;
    let started = std::time::Instant::now();
    stop.cancel();
    assert_eq!(worker.await??, Stopped::Drained);
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "an idle worker took {:?} to stop; every rolling deploy pays this",
        started.elapsed()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn dropping_the_handle_stops_the_worker() -> Result<()> {
    // A supervisor that panics must not leave workers claiming rows forever.
    let db = schema_harness::TestDatabase::start().await?;
    let consumer = Arc::new(Recorder::new(usize::MAX));
    let (stop, cancel) = CancelOnDrop::new();
    let pool = db.pool.clone();
    let worker = tokio::spawn(async move {
        casual_task_worker::dispatcher::run(
            &pool,
            consumer,
            "worker-1",
            config(),
            cancel,
            Arc::new(Metrics::new()),
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(150)).await;
    drop(stop);
    let stopped = tokio::time::timeout(Duration::from_secs(5), worker).await;
    assert!(
        stopped.is_ok(),
        "the worker outlived the handle that owns it"
    );
    Ok(())
}

/// Not a test — a compile-time assertion that a [`Cancel`] can be shared.
///
/// One process runs a loop per consumer, so a single shutdown signal has to
/// reach six of them. If `Cancel` were not `Clone + Send`, that would be
/// discovered when wiring the sixth, not here.
#[allow(dead_code)]
fn cancel_is_shareable(c: Cancel) -> (Cancel, Cancel) {
    fn assert_send<T: Send + Clone>(t: T) -> T {
        t
    }
    let a = assert_send(c.clone());
    (a, c)
}
