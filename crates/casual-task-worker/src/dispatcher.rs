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
use std::time::Duration;

use casual_task_observability::labels::{LabelSet, keys};
use casual_task_observability::metrics::{
    OUTBOX_DISPATCH_TOTAL, OUTBOX_DLQ_DEPTH, OUTBOX_LAG_SECONDS,
};
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::dispatch::{self, Claimed, Dispatcher};
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

    while !cancel.is_cancelled() {
        // The lag reading is taken in the SAME transaction as the claim, before
        // the claimed rows are marked. Read afterwards it would exclude the work
        // just taken and report a healthier number than the truth — the
        // direction of error nobody investigates.
        let (claimed, lag, dlq) =
            claim_batch(pool, consumer.name(), worker_id, config.batch).await?;
        record_lag(&metrics, consumer.name(), lag, &dlq);

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
            in_flight.spawn(async move {
                let _permit = permit;
                // Delivery happens INSIDE the task. Awaiting it before spawning
                // would serialise every delivery behind the previous one and
                // make the semaphore decorative — concurrency of one, bounded
                // by a permit nobody contends for.
                let outcome = consumer.deliver(&event).await;
                record(&pool, &event, outcome, &metrics).await
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
#[allow(clippy::type_complexity)]
async fn claim_batch(
    pool: &PgPool,
    consumer: &str,
    worker_id: &str,
    batch: i64,
) -> Result<(Vec<Claimed>, Option<f64>, Vec<(String, i64)>), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut dispatcher = Dispatcher::assume(&mut tx).await?;
    let lag = dispatch::oldest_pending_seconds(&mut dispatcher, consumer).await?;
    let dlq = dispatch::dlq_depth(&mut dispatcher).await?;
    let claimed = dispatch::claim(&mut dispatcher, consumer, worker_id, batch).await?;
    tx.commit().await?;
    Ok((claimed, lag, dlq))
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
    event: &Claimed,
    outcome: Result<(), String>,
    metrics: &Recorder,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut dispatcher = Dispatcher::assume(&mut tx).await?;
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
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_bounded_and_ordered_sensibly() {
        let c = Config::default();
        assert!(c.batch > 0 && c.concurrency > 0);
        // Concurrency above the batch size cannot be used: a poll never has
        // more than `batch` deliveries to run at once.
        assert!(
            c.concurrency as i64 <= c.batch,
            "concurrency exceeds the batch size, so the extra permits are dead"
        );
        // docs/34 bounds a webhook at 30 s. A drain shorter than that abandons
        // deliveries that were about to succeed; longer than the orchestrator's
        // grace period and the process is SIGKILLed mid-drain instead.
        assert!(c.drain >= Duration::from_secs(10));
        assert!(c.drain < Duration::from_secs(30));
    }

    #[test]
    fn the_drain_is_shorter_than_the_claim_expiry() {
        // Otherwise a drain could still be running when another worker becomes
        // entitled to reclaim the same rows, turning shutdown into a guaranteed
        // double delivery rather than a rare one.
        let drain = Config::default().drain.as_secs() as i64;
        assert!(
            drain < dispatch::CLAIM_EXPIRY.whole_seconds(),
            "drain {drain}s is not shorter than the claim expiry"
        );
    }
}
