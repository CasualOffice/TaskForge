//! Dispatch and history assertions; changes when outbox delivery changes.

use uuid::Uuid;

use super::Counts;

/// Age every outstanding claim past [`crate::dispatch::CLAIM_EXPIRY`].
///
/// Simulates time so a crash-recovery test does not sleep five minutes.
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

/// What the three streams recorded for one workspace (`docs/25`, ADR-006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    /// `activity_event.event_type`, oldest first.
    pub activity: Vec<String>,
    /// `audit_event.event_type`, oldest first.
    pub audit: Vec<String>,
    /// `outbox_event.event_type`, oldest first.
    pub outbox: Vec<String>,
    /// Rows in `outbox_delivery` for those events.
    pub deliveries: i64,
}

/// The history a workspace accumulated.
///
/// The point of asserting on all four at once is ADR-006's guarantee: the
/// domain change, the activity row, the audit row and the outbox event commit
/// together. A test that checked only the audit row would pass while the outbox
/// silently wrote nothing, and the missing events would surface months later as
/// a consumer that never fired.
///
/// # Errors
///
/// Any database error.
pub async fn history(pool: &sqlx::PgPool, workspace_id: Uuid) -> Result<History, sqlx::Error> {
    let activity: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM activity_event WHERE workspace_id = $1 ORDER BY id",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    let audit: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM audit_event WHERE workspace_id = $1 ORDER BY id",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    let outbox: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM outbox_event WHERE workspace_id = $1 ORDER BY id",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    let deliveries: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox_delivery WHERE workspace_id = $1")
            .bind(workspace_id)
            .fetch_one(pool)
            .await?;

    Ok(History {
        activity,
        audit,
        outbox,
        deliveries,
    })
}

/// The event types audited against one aggregate.
///
/// `docs/04` control 7: every grant, revoke and role edit writes an
/// `audit_event`. A test asserting the domain row alone would pass with the
/// auditing deleted.
///
/// # Errors
///
/// Any database error.
pub async fn audit_events_for(
    pool: &sqlx::PgPool,
    aggregate_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        // `audit_event` names it `target_id`, not `aggregate_id`: the audit
        // stream is about who did what to what, and `activity_event` is the one
        // keyed on an aggregate (`docs/25` §The three streams).
        "SELECT event_type FROM audit_event WHERE target_id = $1 ORDER BY occurred_at",
    )
    .bind(aggregate_id)
    .fetch_all(pool)
    .await
}

/// A workspace's current `authz_epoch` (`docs/04` §Caching, ADR-012).
///
/// # Errors
///
/// Any database error.
pub async fn authz_epoch(pool: &sqlx::PgPool, workspace_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT authz_epoch FROM workspace WHERE id = $1")
        .bind(workspace_id)
        .fetch_one(pool)
        .await
}
