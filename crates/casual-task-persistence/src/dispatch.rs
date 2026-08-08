//! Claim → commit → HTTP → record (`docs/25` §Dispatch, D-038).
//!
//! The shape of this module *is* the decision. Claiming and recording are two
//! separate calls with no way to hold a transaction between them, because the
//! rejected design held one across consumer HTTP I/O — pinning a database
//! connection for as long as a webhook chose to take.
//!
//! There is deliberately no `dispatch()` that does all three. A caller must
//! claim, drop the transaction, do its own network call, then record. Making
//! that awkward to get wrong is the point.

use uuid::Uuid;

use crate::scoped::Scoped;

/// How long a claim survives before another worker may take it.
///
/// `docs/25`: longer than any consumer timeout, short enough that recovery is
/// not an incident. The cost is stated there too — a worker merely *slow* past
/// this point has its event delivered twice, which is why at-least-once is the
/// contract rather than an apology.
pub const CLAIM_EXPIRY: time::Duration = time::Duration::minutes(5);

/// `docs/25` §Retry and dead-letter: 1 s, 4 s, 16 s, 1 m, 5 m, 30 m, then the
/// dead-letter queue.
pub const BACKOFF: [time::Duration; 6] = [
    time::Duration::seconds(1),
    time::Duration::seconds(4),
    time::Duration::seconds(16),
    time::Duration::minutes(1),
    time::Duration::minutes(5),
    time::Duration::minutes(30),
];

/// One delivery a worker has taken responsibility for.
#[derive(Debug, Clone)]
pub struct Claimed {
    pub delivery_id: Uuid,
    pub event_id: Uuid,
    pub consumer: String,
    pub event_type: String,
    pub aggregate_id: Uuid,
    pub payload: serde_json::Value,
    pub attempts: i32,
}

/// Take up to `limit` deliveries for `consumer`.
///
/// **The caller must commit before doing anything with the result.** The claim
/// is a database write; holding its transaction open through delivery is
/// exactly what D-038 rejected.
///
/// Per-aggregate ordering is enforced here rather than asserted: a delivery is
/// not claimable while an *earlier* undelivered delivery exists for the same
/// aggregate and consumer. `docs/25` promises that ordering, and nothing else
/// in the system provides it.
///
/// # Errors
///
/// Any database error.
pub async fn claim(
    scoped: &mut Scoped<'_>,
    consumer: &str,
    worker: &str,
    limit: i64,
) -> Result<Vec<Claimed>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (Uuid, Uuid, String, String, Uuid, serde_json::Value, i32)>(
        "UPDATE outbox_delivery d
            SET claimed_at = now(), claimed_by = $2, attempts = d.attempts + 1
          WHERE d.id IN (
                SELECT c.id
                  FROM outbox_delivery c
                  JOIN outbox_event e ON e.id = c.event_id
                 WHERE c.consumer = $1
                   AND c.dispatched_at IS NULL
                   AND c.dead_lettered_at IS NULL
                   AND c.next_attempt_at <= now()
                   AND (c.claimed_at IS NULL OR c.claimed_at < now() - $4::interval)
                   -- Per-aggregate ordering: nothing earlier for this
                   -- aggregate may still be outstanding for this consumer.
                   AND NOT EXISTS (
                       SELECT 1
                         FROM outbox_delivery prior
                         JOIN outbox_event pe ON pe.id = prior.event_id
                        WHERE prior.consumer = c.consumer
                          AND pe.aggregate_id = e.aggregate_id
                          AND prior.dispatched_at IS NULL
                          AND prior.dead_lettered_at IS NULL
                          AND (pe.created_at, pe.id) < (e.created_at, e.id))
                 ORDER BY e.created_at, e.id
                 LIMIT $3
                   FOR UPDATE OF c SKIP LOCKED)
      RETURNING d.id, d.event_id, d.consumer,
                (SELECT event_type   FROM outbox_event WHERE id = d.event_id),
                (SELECT aggregate_id FROM outbox_event WHERE id = d.event_id),
                (SELECT payload      FROM outbox_event WHERE id = d.event_id),
                d.attempts",
    )
    .bind(consumer)
    .bind(worker)
    .bind(limit)
    .bind(pg_interval(CLAIM_EXPIRY))
    .fetch_all(scoped.conn())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(delivery_id, event_id, consumer, event_type, aggregate_id, payload, attempts)| {
                Claimed {
                    delivery_id,
                    event_id,
                    consumer,
                    event_type,
                    aggregate_id,
                    payload,
                    attempts,
                }
            },
        )
        .collect())
}

/// Mark a delivery done. A second, short transaction.
///
/// # Errors
///
/// Any database error.
pub async fn succeeded(scoped: &mut Scoped<'_>, delivery_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE outbox_delivery
            SET dispatched_at = now(), claimed_at = NULL, claimed_by = NULL, last_error = NULL
          WHERE id = $1",
    )
    .bind(delivery_id)
    .execute(scoped.conn())
    .await?;
    Ok(())
}

/// Record a failure: back off, or dead-letter once the ladder is exhausted.
///
/// The delay is **stored**, not slept. A backoff living only in a worker's
/// memory is lost on restart, and the claim query has no way to exclude a row
/// that is waiting.
///
/// # Errors
///
/// Any database error.
pub async fn failed(
    scoped: &mut Scoped<'_>,
    delivery_id: Uuid,
    attempts: i32,
    error: &str,
) -> Result<bool, sqlx::Error> {
    let index = usize::try_from(attempts.max(1) - 1).unwrap_or(0);
    let Some(delay) = BACKOFF.get(index) else {
        sqlx::query(
            "UPDATE outbox_delivery
                SET dead_lettered_at = now(), claimed_at = NULL, claimed_by = NULL,
                    last_error = $2
              WHERE id = $1",
        )
        .bind(delivery_id)
        .bind(error)
        .execute(scoped.conn())
        .await?;
        return Ok(true);
    };

    sqlx::query(
        "UPDATE outbox_delivery
            SET next_attempt_at = now() + $3::interval,
                claimed_at = NULL, claimed_by = NULL, last_error = $2
          WHERE id = $1",
    )
    .bind(delivery_id)
    .bind(error)
    .bind(pg_interval(*delay))
    .execute(scoped.conn())
    .await?;
    Ok(false)
}

/// A PostgreSQL interval literal. Seconds only, so there is no locale or
/// month-length ambiguity in the string.
fn pg_interval(d: time::Duration) -> String {
    format!("{} seconds", d.whole_seconds())
}

/// The age of the oldest **actionable** pending delivery — D-047's definition of
/// `outbox_lag_seconds`, as a gauge.
///
/// "Actionable" excludes rows waiting on a backoff and rows already
/// dead-lettered. Counting those would make the primary health signal rise
/// during normal retry behaviour and stay high forever after one permanent
/// failure, which is how a paging alert gets muted.
///
/// # Errors
///
/// Any database error.
pub async fn oldest_pending_seconds(
    scoped: &mut Scoped<'_>,
    consumer: &str,
) -> Result<Option<f64>, sqlx::Error> {
    // `Option<f64>`, and read with `fetch_one`, because an aggregate over zero
    // rows returns one row containing NULL — not zero rows. Typing this as a
    // plain `f64` decoded fine in every test with a backlog and failed the
    // moment there was nothing pending, which is the state a healthy system is
    // in almost all of the time.
    let lag: Option<f64> = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM (now() - min(e.created_at)))::float8
           FROM outbox_delivery d
           JOIN outbox_event e ON e.id = d.event_id
          WHERE d.consumer = $1
            AND d.dispatched_at IS NULL
            AND d.dead_lettered_at IS NULL
            AND d.next_attempt_at <= now()",
    )
    .bind(consumer)
    .fetch_one(scoped.conn())
    .await?;
    Ok(lag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_ladder_matches_the_design_record() {
        // docs/25 §Retry and dead-letter, verbatim: 1 s, 4 s, 16 s, 1 m, 5 m,
        // 30 m, then the dead-letter queue.
        let seconds: Vec<i64> = BACKOFF.iter().map(|d| d.whole_seconds()).collect();
        assert_eq!(seconds, vec![1, 4, 16, 60, 300, 1800]);
        assert_eq!(BACKOFF.len(), 6, "six attempts, then dead-letter");
    }

    #[test]
    fn the_ladder_is_monotonic() {
        // A ladder that went backwards would retry a failing consumer harder
        // the longer it stayed broken.
        for pair in BACKOFF.windows(2) {
            assert!(pair[1] > pair[0], "{pair:?} is not increasing");
        }
    }

    #[test]
    fn the_claim_expiry_exceeds_the_longest_plausible_consumer_timeout() {
        // docs/25: longer than any consumer timeout. docs/34 bounds a plugin
        // call at 500 ms and a webhook at 30 s; five minutes clears both by an
        // order of magnitude, which is what stops a slow-but-alive worker from
        // having its work stolen.
        assert!(CLAIM_EXPIRY > time::Duration::seconds(30));
    }

    #[test]
    fn intervals_are_emitted_in_seconds() {
        // Not "1 month" or "30 minutes" — a seconds literal has no locale or
        // month-length ambiguity.
        assert_eq!(pg_interval(time::Duration::minutes(5)), "300 seconds");
        assert_eq!(pg_interval(CLAIM_EXPIRY), "300 seconds");
    }
}
