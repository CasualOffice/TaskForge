//! The scheduled seven-day outbox cleanup (C-011).
//!
//! # The failure this prevents
//!
//! [`casual_task_persistence::dispatch::sweep`] existed without a caller, so
//! delivered rows grew forever while the tracker described a retention policy.
//! Running it independently in every process would trade that omission for
//! concurrent maintenance load. This loop combines the persisted sweep with
//! the PostgreSQL session lease from `docs/24` and the shutdown token the
//! dispatch loops already use.
//!
//! # Bounds
//!
//! One run drains oldest-first bounded batches. The two statements
//! in a batch complete before cancellation is observed; no transaction or lock
//! survives the batch. A non-leader performs one non-blocking acquisition per
//! leadership interval, and the leader heartbeats on that cadence.

use std::num::NonZeroU32;
use std::time::Duration;

use casual_task_persistence::dispatch::{self, DispatcherRole};
use casual_task_persistence::leader_lease::LeaderLease;
use sqlx::{Connection, PgPool};

use crate::dispatcher::Cancel;

/// The static advisory-lock namespace for this scheduled job.
pub const NAME: &str = "outbox-retention";

/// Internal schedule and batch bounds.
///
/// These are not deployment settings. The seven-day policy is fixed in
/// `docs/25`; exposing its implementation clock would add a way to quietly turn
/// cleanup off without changing that policy.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    batch: NonZeroU32,
    cadence: Duration,
    leadership_interval: Duration,
}

impl Config {
    /// Construct a schedule. Used by the real-PostgreSQL acceptance test to
    /// compress hours to milliseconds without a second runner implementation.
    ///
    /// # Panics
    ///
    /// If either interval is zero. This is process-owned configuration, not
    /// input; accepting zero would make the loop busy-spin forever.
    #[must_use]
    pub fn new(batch: NonZeroU32, cadence: Duration, leadership_interval: Duration) -> Self {
        assert!(!cadence.is_zero(), "retention cadence must be non-zero");
        assert!(
            !leadership_interval.is_zero(),
            "leadership interval must be non-zero"
        );
        Self {
            batch,
            cadence,
            leadership_interval,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new(
            NonZeroU32::new(1_000).expect("1,000 is non-zero"),
            Duration::from_secs(60 * 60),
            Duration::from_secs(60),
        )
    }
}

/// The loop has one normal exit: its owning process cancelled it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// No new batch was started and the lease session was closed.
    Cancelled,
}

/// Run the outbox-retention schedule until cancellation.
///
/// Acquisition is immediate. A leader sweeps immediately, heartbeats every
/// minute and sweeps hourly; a loser retries once per minute. Database errors
/// stop this task loudly so its supervisor can report the maintenance failure.
/// The lease is close-on-drop, so even an error cannot strand the advisory lock
/// in the pool.
///
/// # Errors
///
/// A PostgreSQL connection, privilege, heartbeat, sweep, or explicit-release
/// failure.
pub async fn run(
    pool: &PgPool,
    config: Config,
    mut cancel: Cancel,
) -> Result<Stopped, sqlx::Error> {
    // The scheduled job crosses tenants for the same reason dispatch does.
    // Verify that fact before competing for leadership so a wrong DSN fails
    // instead of acquiring the lease and deleting nothing under RLS.
    let role = {
        let mut connection = pool.acquire().await?;
        DispatcherRole::verify(&mut connection).await?
    };

    loop {
        if cancel.is_cancelled() {
            return Ok(Stopped::Cancelled);
        }

        let Some(mut lease) = LeaderLease::try_acquire(pool, NAME).await? else {
            tokio::select! {
                () = cancel.cancelled() => return Ok(Stopped::Cancelled),
                () = tokio::time::sleep(config.leadership_interval) => continue,
            }
        };

        tracing::info!(
            job = NAME,
            role = role.role(),
            "scheduled-job leadership acquired"
        );
        drain(&role, &mut lease, config.batch, &cancel).await?;

        if cancel.is_cancelled() {
            lease.release().await?;
            return Ok(Stopped::Cancelled);
        }

        let mut until_sweep = config.cadence;
        loop {
            let wait = until_sweep.min(config.leadership_interval);
            tokio::select! {
                () = cancel.cancelled() => {
                    lease.release().await?;
                    return Ok(Stopped::Cancelled);
                }
                () = tokio::time::sleep(wait) => {}
            }

            lease.heartbeat().await?;
            until_sweep = until_sweep.saturating_sub(wait);
            if until_sweep.is_zero() {
                drain(&role, &mut lease, config.batch, &cancel).await?;
                until_sweep = config.cadence;
            }

            if cancel.is_cancelled() {
                lease.release().await?;
                return Ok(Stopped::Cancelled);
            }
        }
    }
}

async fn drain(
    role: &DispatcherRole,
    lease: &mut LeaderLease,
    batch: NonZeroU32,
    cancel: &Cancel,
) -> Result<(), sqlx::Error> {
    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }

        let (deliveries, events) = {
            let mut transaction = lease.connection().begin().await?;
            let result = {
                let mut dispatcher = role.dispatcher(&mut transaction);
                dispatch::sweep(&mut dispatcher, batch.get()).await?
            };
            transaction.commit().await?;
            result
        };

        if deliveries > 0 || events > 0 {
            tracing::info!(job = NAME, deliveries, events, "retention batch committed");
        }

        let bound = u64::from(batch.get());
        if deliveries < bound && events < bound {
            return Ok(());
        }

        // A large catch-up stays cooperative even before cancellation. The
        // database statements are bounded, and this gives other ready tasks a
        // scheduling point between them.
        tokio::task::yield_now().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "retention cadence must be non-zero")]
    fn a_zero_sweep_cadence_is_refused() {
        let _ = Config::new(
            NonZeroU32::new(1).expect("non-zero"),
            Duration::ZERO,
            Duration::from_secs(1),
        );
    }

    #[test]
    #[should_panic(expected = "leadership interval must be non-zero")]
    fn a_zero_leadership_interval_is_refused() {
        let _ = Config::new(
            NonZeroU32::new(1).expect("non-zero"),
            Duration::from_secs(1),
            Duration::ZERO,
        );
    }
}
