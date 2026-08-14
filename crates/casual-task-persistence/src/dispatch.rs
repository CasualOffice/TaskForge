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

    /// `pub(crate)` for `crate::export`, which claims and updates export jobs
    /// across tenants for the same reason `dispatch` does: a background runner
    /// cannot know the set of workspace ids in advance. Still not public — the
    /// bypass stays inside the crate that owns the SQL.
    pub(crate) fn conn(&mut self) -> &mut PgConnection {
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
    /// The authorization scope (migration 0022). `None` for workspace-level
    /// events, which no project-scoped subscriber may receive.
    pub project_id: Option<Uuid>,
    pub payload: serde_json::Value,
    pub attempts: i32,
    /// Who caused the event; `None` for system-generated (migration 0024).
    ///
    /// `docs/29` rule 1 — "you are never notified about your own action" — is
    /// unimplementable without this, and it is not recoverable from anywhere
    /// else on the row: the outbox carries no other trace of the actor.
    pub actor_id: Option<Uuid>,
}

/// The tuple `claim` decodes. Named because it is nine wide and clippy is
/// right that an anonymous nine-tuple in a signature is unreadable.
type ClaimRow = (
    Uuid,
    Uuid,
    String,
    String,
    Uuid,
    Uuid,
    Option<Uuid>,
    serde_json::Value,
    i32,
    Option<Uuid>,
);

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
    let rows = sqlx::query_as::<_, ClaimRow>(
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
      -- `d.workspace_id` is deliberately NOT returned. A delivery and its event
      -- are the same workspace by construction, and returning both put an
      -- eleventh column in front of a ten-element tuple. sqlx reports that as a
      -- type mismatch on column 2, which names neither the column that moved
      -- nor the query that moved it.
      RETURNING d.id, d.event_id, d.consumer,
                (SELECT event_type   FROM outbox_event WHERE id = d.event_id),
                (SELECT aggregate_id FROM outbox_event WHERE id = d.event_id),
                (SELECT workspace_id FROM outbox_event WHERE id = d.event_id),
                (SELECT project_id   FROM outbox_event WHERE id = d.event_id),
                (SELECT payload      FROM outbox_event WHERE id = d.event_id),
                d.attempts,
                (SELECT actor_id     FROM outbox_event WHERE id = d.event_id)",
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
                consumer,
                event_type,
                aggregate_id,
                workspace_id,
                project_id,
                payload,
                attempts,
                actor_id,
            )| {
                Claimed {
                    delivery_id,
                    event_id,
                    consumer,
                    event_type,
                    aggregate_id,
                    workspace_id,
                    project_id,
                    payload,
                    attempts,
                    actor_id,
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

include!("dispatch_maintenance.rs");

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
