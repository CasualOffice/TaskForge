/// Age of the oldest actionable delivery for a consumer.
///
/// Backoff and dead-letter rows are excluded. The covering pending index makes
/// this an index-only scan over deliveries, without joining the event table.
pub async fn oldest_pending_seconds(
    dispatcher: &mut Dispatcher<'_>,
    consumer: &str,
) -> Result<Option<f64>, sqlx::Error> {
    // An aggregate over zero rows returns one row containing NULL.
    let lag: Option<f64> = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM (now() - min(d.created_at)))::float8
           FROM outbox_delivery d
          WHERE d.consumer = $1
            AND d.dispatched_at IS NULL
            AND d.dead_lettered_at IS NULL
            AND d.next_attempt_at <= now()",
    )
    .bind(consumer)
    .fetch_one(dispatcher.conn())
    .await?;
    Ok(lag)
}

/// How long a fully-delivered event is kept (`docs/25`).
pub const RETENTION: time::Duration = time::Duration::days(7);

/// Delete completed deliveries older than [`RETENTION`] and orphaned events.
///
/// Dead-lettered rows are retained for an operator decision. The bounded batch
/// prevents maintenance from stalling the dispatch loop.
pub async fn sweep(dispatcher: &mut Dispatcher<'_>, limit: i64) -> Result<(u64, u64), sqlx::Error> {
    let deliveries = sqlx::query(
        "DELETE FROM outbox_delivery
          WHERE id IN (SELECT id FROM outbox_delivery
                        WHERE dispatched_at IS NOT NULL
                          AND dispatched_at < now() - $1::interval
                        LIMIT $2)",
    )
    .bind(pg_interval(RETENTION))
    .bind(limit)
    .execute(dispatcher.conn())
    .await?
    .rows_affected();

    let events = sqlx::query(
        "DELETE FROM outbox_event e
          WHERE e.created_at < now() - $1::interval
            AND NOT EXISTS (SELECT 1 FROM outbox_delivery d WHERE d.event_id = e.id)",
    )
    .bind(pg_interval(RETENTION))
    .execute(dispatcher.conn())
    .await?
    .rows_affected();

    Ok((deliveries, events))
}

/// Dead-letter depth by consumer, bounded to a declared metric label.
pub async fn dlq_depth(dispatcher: &mut Dispatcher<'_>) -> Result<Vec<(String, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT consumer, count(*)
           FROM outbox_delivery
          WHERE dead_lettered_at IS NOT NULL
          GROUP BY consumer",
    )
    .fetch_all(dispatcher.conn())
    .await
}
