//! The outbox dispatch loop (C-011 runtime half, D-038/D-040/D-041).
//!
//! # Shape
//!
//! Per consumer, forever: claim a batch in one transaction, **commit**, deliver
//! each claimed event over the network, then record each outcome in its own
//! short transaction. The claim and the record are separate calls in
//! [`casual_task_persistence::dispatch`] precisely so that no transaction can be
//! held across delivery, and this loop is arranged to make that visible rather
//! than merely true.
//!
//! # Bounds, and what happens when they are reached (D-040)
//!
//! Every quantity here is bounded and every bound has a stated overflow policy.
//!
//! - **Batch size.** At most [`Config::batch`] deliveries are claimed per poll.
//!   Overflow policy: the rest stay in the database, which is the queue. There
//!   is no in-memory backlog to lose.
//! - **In-flight deliveries.** At most [`Config::concurrency`] are being
//!   delivered at once, enforced by a semaphore. Overflow policy: the loop waits
//!   for a permit rather than spawning. Unbounded spawning is how a slow
//!   consumer turns into thousands of sockets and an OOM kill.
//! - **Poll interval.** When a poll finds nothing, the loop sleeps
//!   [`Config::idle`]. When it finds a full batch it polls again immediately —
//!   a backlog should drain at the speed of the work, not the speed of a timer.
//! - **Health-metric sampling.** The two gauges are read at most once per
//!   [`Config::metrics_interval`], in their own transaction. Overflow policy:
//!   the gauge keeps its last value, which is what a gauge is for.
//!
//! # Why the gauges are sampled on a clock and the claim is not
//!
//! `outbox_lag_seconds` and `outbox_dlq_depth` are aggregates over the pending
//! and dead-lettered sets. Reading them once per poll tied their cost to the
//! poll rate — and the poll rate is *highest* under a backlog, which is exactly
//! when those sets are largest and when the database can least afford it. Worse,
//! they were read inside the claim's transaction, so a slow aggregate lengthened
//! the transaction holding the claimed rows' locks.
//!
//! They are metrics. Their cadence belongs to the scrape, not to the work.
//!
//! The cost, stated: a gauge can be up to one [`Config::metrics_interval`] stale.
//! The alerts that read it (`docs/50` RB-01, RB-02) evaluate over 5 and 15
//! minutes, so the default is two orders of magnitude inside their window.
//!
//! # Shutdown (D-041)
//!
//! On cancellation the loop stops claiming and drains what it already claimed,
//! bounded by [`Config::drain`]. Deliveries still running when that expires are
//! abandoned, not awaited: their rows stay claimed and become reclaimable after
//! the claim expiry, so the work is delayed rather than lost.
//!
//! Stopping claims first is the load-bearing half. A drain that kept claiming
//! would never finish under load, and the orchestrator would `SIGKILL` the
//! process mid-delivery instead.

use std::sync::Arc;
use std::time::{Duration, Instant};

use casual_task_observability::labels::{LabelSet, keys};
use casual_task_observability::metrics::{
    OUTBOX_DISPATCH_TOTAL, OUTBOX_DLQ_DEPTH, OUTBOX_LAG_SECONDS,
};
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::dispatch::{self, Claimed, DispatcherRole};
use sqlx::PgPool;
use tokio::sync::{Semaphore, watch};

/// Cooperative cancellation.
///
/// A local type rather than a dependency: the semantics needed here are "a flag
/// that can also be awaited", and writing them down is cheaper than auditing a
/// crate for it. [`Cancel::cancelled`] is what makes every sleep in the loop
/// interruptible — a worker asked to stop must not first sit out its idle
/// interval.
#[derive(Debug, Clone)]
pub struct Cancel(watch::Receiver<bool>);

/// The handle that stops a [`Cancel`]. Dropping it also cancels, so a worker
/// cannot be orphaned by a supervisor that panicked.
#[derive(Debug)]
pub struct CancelOnDrop(watch::Sender<bool>);

impl CancelOnDrop {
    /// A cancellation pair.
    #[must_use]
    pub fn new() -> (Self, Cancel) {
        let (tx, rx) = watch::channel(false);
        (Self(tx), Cancel(rx))
    }

    /// Ask every holder to stop.
    pub fn cancel(&self) {
        let _ = self.0.send(true);
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Cancel {
    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    /// Resolves when cancellation is requested, immediately if it already was.
    pub async fn cancelled(&mut self) {
        // `changed()` only fires on a *transition*, so a token cancelled before
        // this is first awaited would hang here forever without the check.
        while !*self.0.borrow_and_update() {
            if self.0.changed().await.is_err() {
                return; // sender dropped — treat as cancelled
            }
        }
    }
}

/// What a consumer does with an event.
///
/// The network side of dispatch lives behind this trait so the loop can be
/// tested without one — and so a consumer cannot reach the database through it.
/// The loop owns the transactions; a consumer that could open its own would
/// reintroduce exactly the transaction-across-HTTP shape D-038 rejects.
#[allow(async_fn_in_trait)]
pub trait Consumer: Send + Sync {
    /// The name this consumer claims deliveries under. Must match a value in
    /// [`casual_task_persistence::CONSUMERS`] or it will never be given work.
    fn name(&self) -> &'static str;

    /// Deliver one event. `Err` carries a message stored as `last_error` and
    /// read by RB-02, so it should say what failed, not that something did.
    fn deliver(
        &self,
        event: &Claimed,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;
}

/// Loop bounds. Every field is a bound; none is optional.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Deliveries claimed per poll.
    pub batch: i64,
    /// Deliveries in flight at once.
    pub concurrency: usize,
    /// Sleep after a poll that found nothing.
    pub idle: Duration,
    /// How long shutdown waits for in-flight deliveries.
    pub drain: Duration,
    /// Shortest gap between two readings of the health gauges.
    pub metrics_interval: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Small enough that a crash re-delivers little, large enough that
            // the per-poll round trip is amortised. At-least-once means every
            // claimed row is a candidate for duplicate delivery if this worker
            // dies, so the batch size is also the blast radius.
            batch: 64,
            // Bounded well below the connection pool: recording an outcome
            // needs a connection, and concurrency above the pool size would
            // turn every completion into a wait for one.
            concurrency: 16,
            idle: Duration::from_millis(500),
            // Longer than a webhook timeout (docs/34: 30 s), shorter than the
            // orchestrator's default SIGKILL grace (Kubernetes: 30 s), because
            // being killed mid-drain is what the drain exists to avoid.
            drain: Duration::from_secs(20),
            // Well inside RB-01's 5-minute and RB-02's 15-minute evaluation
            // windows, so an alert is not delayed by the sampling; and far
            // longer than a poll, which is the entire point — under a backlog
            // the loop polls continuously, and tying an O(pending) aggregate to
            // that rate is how the metric became the most expensive query in
            // the dispatch path.
            metrics_interval: Duration::from_secs(5),
        }
    }
}

/// Why the loop stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// Cancelled, and every in-flight delivery finished within the drain.
    Drained,
    /// Cancelled, and the drain expired with deliveries still running. Their
    /// rows stay claimed and are reclaimable after the claim expiry — delayed,
    /// not lost.
    DrainTimedOut { abandoned: usize },
}

/// Run one consumer's dispatch loop until `cancel` fires.
///
/// # Errors
///
/// Any database error that is not recoverable by retrying the poll. A failure
/// to *deliver* is not an error here — it is recorded against the delivery and
/// the loop continues, which is the entire point of the retry ladder.
pub async fn run<C: Consumer + 'static>(
    pool: &PgPool,
    consumer: Arc<C>,
    worker_id: &str,
    config: Config,
    mut cancel: Cancel,
    metrics: Arc<Recorder>,
) -> Result<Stopped, sqlx::Error> {
    let permits = Arc::new(Semaphore::new(config.concurrency));
    let mut in_flight = tokio::task::JoinSet::new();

    // Verified once, here, rather than inside every transaction below. The
    // question — can this role bypass RLS? — is answered by the DSN, and the DSN
    // does not change while the loop runs. Asking it per transaction cost a
    // round trip per claim AND a round trip per delivery outcome; see
    // `DispatcherRole`. Failing here also fails the loop at startup, which is
    // where a wrong role should be discovered.
    let role = {
        let mut conn = pool.acquire().await?;
        Arc::new(DispatcherRole::verify(&mut conn).await?)
    };
    tracing::info!(
        role = role.role(),
        consumer = consumer.name(),
        "dispatching"
    );

    // `None` means "never sampled", so the first poll always publishes: a worker
    // that started during an incident must not wait out an interval before
    // saying anything.
    let mut last_sample: Option<Instant> = None;

    while !cancel.is_cancelled() {
        // Sampled BEFORE the claim, and in its own transaction. Before, because
        // the lag reading should include the work this poll is about to take.
        // Separate, because these are aggregates over the whole pending and
        // dead-lettered sets, and running them inside the claim's transaction
        // held the claimed rows' locks for as long as they took.
        if last_sample.is_none_or(|at| at.elapsed() >= config.metrics_interval) {
            let (lag, dlq) = sample_health(pool, &role, consumer.name()).await?;
            record_lag(&metrics, consumer.name(), lag, &dlq);
            last_sample = Some(Instant::now());
        }

        let claimed = claim_batch(pool, &role, consumer.name(), worker_id, config.batch).await?;

        if claimed.is_empty() {
            // Cancellable sleep: a worker asked to stop must not sit here for
            // the full idle interval first.
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(config.idle) => continue,
            }
        }

        let full_batch = claimed.len() as i64 == config.batch;

        for event in claimed {
            // Acquiring before spawning is what bounds this. Spawning first and
            // acquiring inside the task would bound the *work* while leaving
            // the number of tasks unbounded, which is the same memory problem
            // wearing a semaphore.
            let permit = Arc::clone(&permits)
                .acquire_owned()
                .await
                .expect("the semaphore is never closed");
            let pool = pool.clone();
            let consumer = Arc::clone(&consumer);
            let metrics = Arc::clone(&metrics);
            let role = Arc::clone(&role);
            in_flight.spawn(async move {
                let _permit = permit;
                // Delivery happens INSIDE the task. Awaiting it before spawning
                // would serialise every delivery behind the previous one and
                // make the semaphore decorative — concurrency of one, bounded
                // by a permit nobody contends for.
                let outcome = consumer.deliver(&event).await;
                record(&pool, &role, &event, outcome, &metrics).await
            });
        }

        // Reap finished tasks so the set cannot grow without bound across polls.
        while let Some(joined) = in_flight.try_join_next() {
            if let Ok(Err(error)) = joined {
                tracing::error!(%error, "recording a delivery outcome failed");
            }
        }

        if !full_batch {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(config.idle) => {},
            }
        }
    }

    drain(in_flight, config.drain).await
}

/// Claim in its own transaction, and **commit before returning**. The signature
/// is why: nothing borrowed from the transaction escapes, so a caller cannot
/// hold it across the delivery that follows.
///
/// Nothing else runs in this transaction. It holds `FOR UPDATE` locks on every
/// row it claims, so every statement added here is time another worker spends
/// skipping locked rows.
async fn claim_batch(
    pool: &PgPool,
    role: &DispatcherRole,
    consumer: &str,
    worker_id: &str,
    batch: i64,
) -> Result<Vec<Claimed>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut dispatcher = role.dispatcher(&mut tx);
    let claimed = dispatch::claim(&mut dispatcher, consumer, worker_id, batch).await?;
    tx.commit().await?;
    Ok(claimed)
}

/// Read the two health gauges, in a transaction that claims nothing and
/// therefore locks nothing.
async fn sample_health(
    pool: &PgPool,
    role: &DispatcherRole,
    consumer: &str,
) -> Result<(Option<f64>, Vec<(String, i64)>), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut dispatcher = role.dispatcher(&mut tx);
    let lag = dispatch::oldest_pending_seconds(&mut dispatcher, consumer).await?;
    let dlq = dispatch::dlq_depth(&mut dispatcher).await?;
    tx.commit().await?;
    Ok((lag, dlq))
}

/// Map a consumer name that came back from the database to the declared
/// `&'static str` it must be, or to `other`.
///
/// This exists because `LabelSet::with` accepts only `&'static str` — the
/// cardinality guard from `docs/46` — and a name read out of `outbox_delivery`
/// is a runtime `String` even though it was written from [`CONSUMERS`]. The
/// compiler refusing it is not an inconvenience here: `docs/34` lets a **plugin**
/// subscribe, so consumer names are open at runtime, and passing them straight
/// through would make `outbox_dlq_depth` grow a series per installed plugin.
///
/// Unknown names collapse to `other`, which is the same shape the tenant
/// allow-list uses: the metric stays bounded and the detail is in the database.
fn declared_consumer(name: &str) -> &'static str {
    casual_task_persistence::CONSUMERS
        .iter()
        .copied()
        .find(|declared| *declared == name)
        .unwrap_or("other")
}

/// Publish the two gauges `docs/46` calls the primary health signal.
///
/// Called once per [`Config::metrics_interval`], not once per poll — see the
/// module header. Writing a gauge is free; *reading* the numbers is not.
///
/// A gauge with no pending work is set to **0**, not left at its last value: a
/// gauge that stops being written keeps reporting the last number it saw, so a
/// backlog that drained would show as a backlog forever.
fn record_lag(metrics: &Recorder, consumer: &'static str, lag: Option<f64>, dlq: &[(String, i64)]) {
    let Ok(labels) = LabelSet::for_metric(OUTBOX_LAG_SECONDS).with(keys::CONSUMER, consumer) else {
        // A consumer name is a `&'static str` from CONSUMERS, so this cannot
        // fail today. Logged rather than unwrapped because a metric is not
        // worth killing a dispatch loop over.
        tracing::warn!(consumer, "outbox lag label rejected");
        return;
    };
    if let Err(error) = metrics.set(OUTBOX_LAG_SECONDS, &labels, lag.unwrap_or(0.0)) {
        tracing::error!(%error, "recording outbox lag");
    }

    for (dlq_consumer, depth) in dlq {
        let labels = LabelSet::for_metric(OUTBOX_DLQ_DEPTH)
            .with(keys::CONSUMER, declared_consumer(dlq_consumer));
        match labels {
            Ok(labels) => {
                #[allow(clippy::cast_precision_loss)]
                if let Err(error) = metrics.set(OUTBOX_DLQ_DEPTH, &labels, *depth as f64) {
                    tracing::error!(%error, "recording dead-letter depth");
                }
            }
            Err(error) => tracing::warn!(%error, "dead-letter depth label rejected"),
        }
    }
}

/// Record one outcome in its own short transaction.
async fn record(
    pool: &PgPool,
    role: &DispatcherRole,
    event: &Claimed,
    outcome: Result<(), String>,
    metrics: &Recorder,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    // Not `Dispatcher::assume`: this runs once per delivery, and `assume` would
    // re-ask `pg_roles` whether the role is privileged on every one of them.
    let mut dispatcher = role.dispatcher(&mut tx);
    let mut result = "dispatched";
    match outcome {
        Ok(()) => dispatch::succeeded(&mut dispatcher, event.delivery_id).await?,
        Err(error) => {
            let dead = dispatch::failed(&mut dispatcher, event.delivery_id, event.attempts, &error)
                .await?;
            result = if dead { "dead_lettered" } else { "failed" };
            if dead {
                // docs/46 alerts on any sustained DLQ increase, so this is the
                // log line that explains one.
                tracing::error!(
                    consumer = %event.consumer,
                    event_id = %event.event_id,
                    attempts = event.attempts,
                    %error,
                    "delivery dead-lettered"
                );
            }
        }
    }
    tx.commit().await?;

    // Counted after the outcome is committed, so the counter cannot claim a
    // delivery the database rolled back.
    let labels = LabelSet::for_metric(OUTBOX_DISPATCH_TOTAL)
        .with(keys::CONSUMER, declared_consumer(&event.consumer))
        .and_then(|l| l.with(keys::OUTCOME, result));
    match labels {
        Ok(labels) => {
            if let Err(error) = metrics.increment(OUTBOX_DISPATCH_TOTAL, &labels, 1) {
                tracing::error!(%error, "recording a dispatch outcome");
            }
        }
        Err(error) => tracing::warn!(%error, "dispatch outcome label rejected"),
    }
    Ok(())
}

/// Wait for in-flight deliveries, bounded.
async fn drain(
    mut in_flight: tokio::task::JoinSet<Result<(), sqlx::Error>>,
    limit: Duration,
) -> Result<Stopped, sqlx::Error> {
    let outstanding = in_flight.len();
    if outstanding == 0 {
        return Ok(Stopped::Drained);
    }
    tracing::info!(outstanding, "draining in-flight deliveries");

    match tokio::time::timeout(limit, async {
        while let Some(joined) = in_flight.join_next().await {
            if let Ok(Err(error)) = joined {
                tracing::error!(%error, "recording a delivery outcome failed during drain");
            }
        }
    })
    .await
    {
        Ok(()) => Ok(Stopped::Drained),
        Err(_) => {
            let abandoned = in_flight.len();
            // Abandoned, not awaited. Their rows stay claimed and become
            // reclaimable after the claim expiry — delayed, not lost. Saying so
            // in the log matters because the duplicate delivery that follows is
            // expected behaviour, not an incident.
            tracing::warn!(
                abandoned,
                "drain deadline expired; claimed rows will be redelivered after \
                 the claim expiry"
            );
            in_flight.abort_all();
            Ok(Stopped::DrainTimedOut { abandoned })
        }
    }
}

#[cfg(test)]
#[path = "dispatcher_tests.rs"]
mod tests;
