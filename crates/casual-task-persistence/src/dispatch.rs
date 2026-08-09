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

use sqlx::PgConnection;
use uuid::Uuid;

/// A connection that may see across tenants.
///
/// # Why this type exists
///
/// The dispatcher polls every workspace: a background worker cannot know the
/// set of workspace ids in advance. `outbox_delivery` has a row-level security
/// policy, so a normal connection sees **nothing** — and sees it without
/// erroring. A dispatcher built on [`Scoped`](crate::Scoped) would report
/// healthy, claim zero deliveries forever, and the first symptom would be
/// silence: no notifications, no search updates, no webhooks. Nothing in a log.
///
/// So the capability is a type, and it cannot be built without a
/// [`DispatcherRole`] — a token that exists only after the privilege was
/// **verified** against the database rather than trusted from the caller.
/// Wiring the dispatcher to the wrong role fails at startup with a message
/// naming the role, instead of succeeding into that silence.
///
/// It is the deliberate counterpart to `Scoped`: `Scoped` cannot exist without
/// a tenant, and `Dispatcher` cannot exist without the privilege to ignore one.
/// A grep for this type returns every cross-tenant read in the system.
#[allow(missing_debug_implementations)]
pub struct Dispatcher<'t> {
    conn: &'t mut PgConnection,
}

/// The role a [`Dispatcher`] was asked to run as cannot bypass row-level
/// security, so it would silently see no work at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotPrivileged {
    /// `current_user` as the database reports it.
    pub role: String,
}

impl std::fmt::Display for NotPrivileged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "role `{}` cannot bypass row-level security, so the dispatcher would \
             claim nothing and report healthy. Connect as taskforge_dispatcher \
             (migration 0014).",
            self.role
        )
    }
}

impl std::error::Error for NotPrivileged {}

/// Proof, taken **once**, that the role a pool connects as can bypass row-level
/// security.
///
/// # Why the proof is separate from the capability
///
/// The check is a `pg_roles` lookup: a full round trip. It used to run inside
/// every transaction that wanted a [`Dispatcher`], and the dispatch loop opens
/// one transaction *per delivery outcome* — so a worker delivering a thousand
/// events a second spent a thousand round trips a second re-asking a question
/// whose answer is fixed for the lifetime of a connection. `current_user` does
/// not change under a running worker; the DSN is process configuration.
///
/// What is being defended against is a **misconfiguration** — the wrong role in
/// a deployment — and a misconfiguration is a startup fact. So the check runs at
/// startup, and what it produces is this token. [`DispatcherRole::dispatcher`]
/// then costs nothing, and there is still no way to obtain a [`Dispatcher`]
/// without having passed the check, because the token is unforgeable.
///
/// **The cost, stated:** the token proves that *a* connection was privileged.
/// A caller that verified against one pool and then handed in a connection from
/// a different pool would defeat it. A worker has one pool, built from one DSN,
/// and [`Dispatcher::assume`] remains available for callers that want the check
/// on this exact connection.
#[derive(Debug, Clone)]
pub struct DispatcherRole {
    role: String,
}

impl DispatcherRole {
    /// Ask the database whether this connection's role can see across tenants.
    ///
    /// # Errors
    ///
    /// [`sqlx::Error`] on any database failure, or a boxed [`NotPrivileged`]
    /// when the connected role is subject to row-level security.
    pub async fn verify(conn: &mut PgConnection) -> Result<Self, sqlx::Error> {
        // `rolsuper` as well as `rolbypassrls`: a superuser bypasses policies
        // unconditionally without the flag being set on some installations, and
        // the test harness connects as the owner.
        let (role, privileged): (String, bool) = sqlx::query_as(
            "SELECT current_user::text, bool_or(rolsuper OR rolbypassrls)
               FROM pg_roles WHERE rolname = current_user",
        )
        .fetch_one(&mut *conn)
        .await?;

        if !privileged {
            return Err(sqlx::Error::Configuration(Box::new(NotPrivileged { role })));
        }
        Ok(Self { role })
    }

    /// The role that passed, for logs and for the operator who has to be told
    /// which one is running.
    #[must_use]
    pub fn role(&self) -> &str {
        &self.role
    }

    /// Take a connection as the dispatcher. **Not** a round trip — the privilege
    /// was proven when `self` was created.
    #[must_use]
    pub fn dispatcher<'t>(&self, conn: &'t mut PgConnection) -> Dispatcher<'t> {
        Dispatcher { conn }
    }
}

impl<'t> Dispatcher<'t> {
    /// Verify this connection's role and take it as the dispatcher, in one call.
    ///
    /// One round trip **per call**. Anything in a loop should verify once with
    /// [`DispatcherRole::verify`] and use [`DispatcherRole::dispatcher`]; this
    /// is for one-off callers and for tests, where the connection under test is
    /// the point.
    ///
    /// # Errors
    ///
    /// [`sqlx::Error`] on any database failure, or a boxed [`NotPrivileged`]
    /// when the connected role is subject to row-level security.
    pub async fn assume(conn: &'t mut PgConnection) -> Result<Self, sqlx::Error> {
        let role = DispatcherRole::verify(&mut *conn).await?;
        Ok(role.dispatcher(conn))
    }

    fn conn(&mut self) -> &mut PgConnection {
        self.conn
    }
}

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
    /// The tenant the event belongs to.
    ///
    /// Every consumer needs it and none can derive it: the dispatcher polls
    /// across tenants by design (`docs/25`), so the workspace cannot come from
    /// a session the way it does on a request. A consumer that writes anything
    /// reconstructs its scope from here through
    /// [`WorkspaceScope::for_job`](casual_task_model::WorkspaceScope::for_job),
    /// which is the one constructor that exists for exactly this path.
    pub workspace_id: Uuid,
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
    dispatcher: &mut Dispatcher<'_>,
    consumer: &str,
    worker: &str,
    limit: i64,
) -> Result<Vec<Claimed>, sqlx::Error> {
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            Uuid,
            String,
            String,
            Uuid,
            serde_json::Value,
            i32,
        ),
    >(
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
      RETURNING d.id, d.event_id, d.workspace_id, d.consumer,
                (SELECT event_type   FROM outbox_event WHERE id = d.event_id),
                (SELECT aggregate_id FROM outbox_event WHERE id = d.event_id),
                (SELECT payload      FROM outbox_event WHERE id = d.event_id),
                d.attempts",
    )
    .bind(consumer)
    .bind(worker)
    .bind(limit)
    .bind(pg_interval(CLAIM_EXPIRY))
    .fetch_all(dispatcher.conn())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                delivery_id,
                event_id,
                workspace_id,
                consumer,
                event_type,
                aggregate_id,
                payload,
                attempts,
            )| Claimed {
                delivery_id,
                event_id,
                workspace_id,
                consumer,
                event_type,
                aggregate_id,
                payload,
                attempts,
            },
        )
        .collect())
}

/// Mark a delivery done. A second, short transaction.
///
/// # Errors
///
/// Any database error.
pub async fn succeeded(
    dispatcher: &mut Dispatcher<'_>,
    delivery_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE outbox_delivery
            SET dispatched_at = now(), claimed_at = NULL, claimed_by = NULL, last_error = NULL
          WHERE id = $1",
    )
    .bind(delivery_id)
    .execute(dispatcher.conn())
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
    dispatcher: &mut Dispatcher<'_>,
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
        .execute(dispatcher.conn())
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
    .execute(dispatcher.conn())
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
/// # Why this reads `outbox_delivery.created_at` and not the event's
///
/// It used to join `outbox_event` for `min(e.created_at)`. That join is the
/// whole cost: an aggregate over the pending set cannot stop early, so every
/// pending delivery meant one random primary-key lookup into `outbox_event` —
/// two million heap fetches at a two-million backlog, for one number.
///
/// The three columns this needs are all in `outbox_delivery_pending_ix`
/// (`consumer, next_attempt_at, created_at`, partial on exactly the two NULL
/// predicates below), so without the join the whole gauge is one index-only
/// scan: no heap, no join, no sort.
///
/// The two timestamps are the same instant. `UnitOfWork::record` inserts the
/// event and its delivery rows in one transaction and both default to `now()`,
/// which in PostgreSQL is the transaction's start time — identical to the
/// microsecond, not merely close. `outbox.rs` asserts that against a real
/// database, because it is an invariant this query depends on rather than
/// something the type system holds.
///
/// It is also the more literal reading of D-047, which says the age of the
/// oldest actionable **delivery**.
///
/// # Errors
///
/// Any database error.
pub async fn oldest_pending_seconds(
    dispatcher: &mut Dispatcher<'_>,
    consumer: &str,
) -> Result<Option<f64>, sqlx::Error> {
    // `Option<f64>`, and read with `fetch_one`, because an aggregate over zero
    // rows returns one row containing NULL — not zero rows. Typing this as a
    // plain `f64` decoded fine in every test with a backlog and failed the
    // moment there was nothing pending, which is the state a healthy system is
    // in almost all of the time.
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

/// How long a fully-delivered event is kept (`docs/25`: dispatched rows are
/// removed after 7 days).
///
/// Not zero, because RB-01 step 2 reads the last 30 minutes of dispatch history
/// to decide whether a backlog is draining, and a sweep that deleted on success
/// would leave it nothing to read.
pub const RETENTION: time::Duration = time::Duration::days(7);

/// Delete deliveries that completed more than [`RETENTION`] ago, and the events
/// left with none.
///
/// Returns `(deliveries, events)` removed.
///
/// **Dead-lettered rows are never swept.** `docs/25`: "a dead-lettered event is
/// never silently dropped". They leave only by being replayed or by an operator
/// deciding, in RB-02, that they should not be — and that decision is recorded
/// in an incident, not made by a timer at 3 a.m.
///
/// Bounded by `limit` so one call cannot take a lock on millions of rows and
/// stall the dispatch loop it shares a database with. The caller repeats until
/// it returns zero.
///
/// # Errors
///
/// Any database error.
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

    // An event whose deliveries are all gone has nothing left to deliver. The
    // NOT EXISTS is what makes this safe to run while a slow consumer still has
    // rows outstanding: the event survives as long as any consumer needs it.
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

/// Dead-letter depth by consumer — the label `outbox_dlq_depth` declares.
///
/// Grouped in the database rather than counted per consumer in a loop: `docs/46`
/// alerts on *any* sustained increase, so this is read on every scrape cycle and
/// six round trips per scrape is six times the cost for the same answer.
///
/// Not grouped by event type. RB-02 does group by it — in SQL, where a high
/// cardinality costs nothing — but as a metric label it would be unbounded
/// (D-053), so it is deliberately absent here rather than dropped later by the
/// caller.
///
/// # This is O(dead letters), and dead letters are never swept
///
/// `sweep` deliberately never deletes a dead-lettered row, so this set only ever
/// grows until an operator drains it through RB-02. Migration 0018 leads
/// `outbox_delivery_dlq_ix` with `consumer` so the count is an index-only scan
/// rather than one random heap read per dead row, but the scan itself still
/// grows with the queue — which is why the caller samples it on a fixed cadence
/// instead of once per poll. A dispatch loop that recomputed this every poll got
/// slower every week the DLQ was not at zero, and polled hardest during exactly
/// the backlog that made it slowest.
///
/// # Errors
///
/// Any database error.
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
    fn retention_is_long_enough_for_the_runbook_that_reads_history() {
        // RB-01 step 2 reads 30 minutes of dispatch history to decide whether a
        // backlog is draining. A sweep on success would leave it nothing.
        assert!(RETENTION > time::Duration::hours(1));
        assert_eq!(RETENTION.whole_days(), 7, "docs/25 says seven days");
    }

    #[test]
    fn intervals_are_emitted_in_seconds() {
        // Not "1 month" or "30 minutes" — a seconds literal has no locale or
        // month-length ambiguity.
        assert_eq!(pg_interval(time::Duration::minutes(5)), "300 seconds");
        assert_eq!(pg_interval(CLAIM_EXPIRY), "300 seconds");
    }
}
