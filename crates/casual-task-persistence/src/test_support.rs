//! Database fixtures for integration tests in *other* crates.
//!
//! # Why this is here and not in the test that needs it
//!
//! `docs/19` §Boundary invariants: **all SQL lives in this crate**, and
//! `casual-task-lint` makes that a build failure rather than a review comment.
//! The C-011 acceptance gate lives in `casual-task-worker` — it has to, because
//! it asserts what happens when a *worker* is killed mid-batch — and it needs to
//! seed a workspace, age a claim past its expiry, and count delivery states.
//!
//! Two ways to allow that were rejected:
//!
//! - **Exempt `tests/` from the lint.** That is a hole in an architecture
//!   invariant, opened to make one test compile, and it would stay open.
//! - **Add the queries to the production API.** "Expire every claim" exists only
//!   to make a five-minute timeout testable in five milliseconds. A production
//!   surface that carries it is a production surface someone can call.
//!
//! So the SQL lives where the invariant says it must, and is compiled only when
//! a test asks for it.
//!
//! # Not compiled unless requested
//!
//! Behind the non-default `test-support` feature. A release build does not
//! contain [`expire_all_claims`]; there is no flag that reaches it.

use uuid::Uuid;

/// How many deliveries a consumer has in each state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub dispatched: i64,
    /// Not dispatched, not dead-lettered, currently claimed by some worker.
    pub claimed: i64,
    /// Not dispatched and not dead-lettered, whether claimed or not.
    pub outstanding: i64,
    pub dead_lettered: i64,
}

/// Insert a workspace. The smallest row that satisfies the tenant foreign keys.
///
/// # Errors
///
/// Any database error.
pub async fn insert_workspace(
    pool: &sqlx::PgPool,
    id: Uuid,
    slug: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO workspace (id, name, slug) VALUES ($1, $2, $2)")
        .bind(id)
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(())
}

/// Age every outstanding claim past [`crate::dispatch::CLAIM_EXPIRY`].
///
/// Simulates the passage of time so a test does not have to spend it. Testing
/// crash recovery by sleeping five minutes means it is tested once and then
/// disabled.
///
/// # Errors
///
/// Any database error.
pub async fn expire_all_claims(pool: &sqlx::PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE outbox_delivery SET claimed_at = now() - $1::interval
          WHERE claimed_at IS NOT NULL AND dispatched_at IS NULL",
    )
    .bind(format!(
        "{} seconds",
        crate::dispatch::CLAIM_EXPIRY.whole_seconds() + 60
    ))
    .execute(pool)
    .await?
    .rows_affected())
}

/// Delivery state counts for one consumer.
///
/// # Errors
///
/// Any database error.
pub async fn counts(pool: &sqlx::PgPool, consumer: &str) -> Result<Counts, sqlx::Error> {
    let row: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE dispatched_at IS NOT NULL),
                count(*) FILTER (WHERE dispatched_at IS NULL
                                   AND dead_lettered_at IS NULL
                                   AND claimed_at IS NOT NULL),
                count(*) FILTER (WHERE dispatched_at IS NULL
                                   AND dead_lettered_at IS NULL),
                count(*) FILTER (WHERE dead_lettered_at IS NOT NULL)
           FROM outbox_delivery
          WHERE consumer = $1",
    )
    .bind(consumer)
    .fetch_one(pool)
    .await?;

    Ok(Counts {
        dispatched: row.0,
        claimed: row.1,
        outstanding: row.2,
        dead_lettered: row.3,
    })
}
