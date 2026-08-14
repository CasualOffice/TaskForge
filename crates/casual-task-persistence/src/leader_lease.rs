//! PostgreSQL leadership for scheduled jobs (`docs/24` §The scheduled-job lease).
//!
//! # The failure this prevents
//!
//! Retention, rank compaction and reminders may run in every worker process,
//! but each sweep must have one leader. A transaction advisory lock is released
//! before the job starts; a session lock returned to a connection pool remains
//! held by an apparently idle connection. This type makes the connection the
//! ownership token and closes that session whenever the token is dropped.

use sqlx::pool::PoolConnection;
use sqlx::{PgConnection, PgPool, Postgres};

/// One held scheduled-job lease.
///
/// The checked-out connection is deliberately retained for the lease's whole
/// lifetime. Dropping this value closes the connection instead of returning it
/// to the pool, which makes PostgreSQL release the session advisory lock even
/// on cancellation or panic.
pub struct LeaderLease {
    connection: PoolConnection<Postgres>,
    name: &'static str,
}

impl std::fmt::Debug for LeaderLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LeaderLease")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl LeaderLease {
    /// Try to lead `name` without waiting for another instance.
    ///
    /// Static names keep the lock set bounded and make every scheduled job
    /// discoverable in source. The name is bound into the query and hashed to
    /// PostgreSQL's 64-bit advisory-lock key space.
    ///
    /// # Errors
    ///
    /// Any connection or PostgreSQL error. Contention is not an error; it
    /// returns `Ok(None)` so the caller's own cadence decides when to retry.
    pub async fn try_acquire(
        pool: &PgPool,
        name: &'static str,
    ) -> Result<Option<Self>, sqlx::Error> {
        let mut connection = pool.acquire().await?;
        let acquired: bool = sqlx::query_scalar(
            "SELECT pg_try_advisory_lock(\
                hashtextextended('taskforge:scheduled-job:' || $1, 0))",
        )
        .bind(name)
        .fetch_one(&mut *connection)
        .await?;

        if !acquired {
            return Ok(None);
        }

        // Session locks survive transactions and therefore survive a pool
        // checkout. Returning this connection would strand the lock inside an
        // idle pooled session. Closing the session is the release mechanism.
        connection.close_on_drop();
        Ok(Some(Self { connection, name }))
    }

    /// The static name whose leadership this token proves.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Prove the owning PostgreSQL session is still alive.
    ///
    /// A failed heartbeat means leadership is lost: the caller must stop the
    /// scheduled job and drop this token. A checked-out SQLx connection is not
    /// transparently replaced, so success is evidence from the lock-owning
    /// session rather than from a different pool connection.
    ///
    /// # Errors
    ///
    /// Any error from the owning session.
    pub async fn heartbeat(&mut self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1")
            .execute(&mut *self.connection)
            .await?;
        Ok(())
    }

    /// The lock-owning connection for the scheduled repository operation.
    ///
    /// SQL remains in this crate; callers pass this connection to repository
    /// capabilities rather than issuing job SQL from the worker crate.
    #[must_use]
    pub fn connection(&mut self) -> &mut PgConnection {
        &mut self.connection
    }

    /// Release leadership and wait until PostgreSQL has closed the session.
    ///
    /// Drop is also safe, but explicit release gives shutdown code a point it
    /// can await before another instance acquires the same name.
    ///
    /// # Errors
    ///
    /// Any error while closing the PostgreSQL session.
    pub async fn release(self) -> Result<(), sqlx::Error> {
        self.connection.close().await
    }
}
