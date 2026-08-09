//! The unit of work: a domain change and its history, or neither.
//!
//! `docs/25` and ADR-006: "Domain change + activity + audit + outbox commit in
//! one transaction, from Phase 1, even when SSE is the only consumer. Eventing
//! is never introduced later."
//!
//! # Why this is a type and not a convention
//!
//! The guarantee is that there is **no window in which a change exists without
//! its history**. A convention cannot provide that: any repository method that
//! forgets the audit row still compiles, still passes its own test, and only
//! shows up as a gap in an export months later.
//!
//! [`UnitOfWork`] makes the four writes one call. A caller cannot record the
//! domain change without also supplying what happened, because
//! [`UnitOfWork::record`] takes all of it together.
//!
//! # What it deliberately does not do
//!
//! It does not commit. The caller owns the transaction, because a single unit
//! of work frequently spans more than one aggregate — a status transition
//! writes the task, its activity, its audit row, its outbox event, and possibly
//! a comment — and a type that committed on its own would make that impossible
//! to express.

use casual_task_model::{ActorType, CorrelationId, RequestId, UserId, WorkspaceId};
use uuid::Uuid;

use crate::scoped::Scoped;

/// The request behind a change, for the audit trail.
///
/// [`ActorType`] is the model's, not a copy: see [`ActorType::as_audit_str`].
#[derive(Debug, Clone)]
pub struct Provenance {
    pub actor: Option<UserId>,
    pub actor_type: ActorType,
    pub request_id: Option<RequestId>,
    /// `docs/46`: "the thread tying a user action to every effect it caused".
    pub correlation_id: Option<CorrelationId>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

/// One thing that happened, and everything that must be written because of it.
#[derive(Debug, Clone)]
pub struct Change {
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub project_id: Option<Uuid>,
    pub event_type: String,
    /// Display **values**, not ids. `docs/25`: the stream is rendered years
    /// later, possibly after a status was renamed or deleted, and must still
    /// read correctly.
    pub activity_changes: serde_json::Value,
    /// Before/after for the audit trail.
    pub audit_changes: serde_json::Value,
    /// The event payload consumers receive.
    pub payload: serde_json::Value,
    pub schema_version: i32,
}

/// The consumers an event fans out to (`docs/25` §Consumer fan-out).
///
/// A fixed list rather than a runtime lookup: every one of these exists in the
/// design record, and a delivery row per consumer is written in the producing
/// transaction so that a consumer added later cannot silently miss events that
/// happened before it existed — it gets no row, which is visible, rather than
/// an event it never sees.
pub const CONSUMERS: &[&str] = &[
    "sse_fanout",
    "search_projection",
    "notification_fanout",
    "automation_matcher",
    "webhook_delivery",
    "plugin_subscribers",
];

#[derive(Debug)]
pub struct UnitOfWork;

impl UnitOfWork {
    /// Write the activity record, the audit record, the outbox event, and one
    /// delivery row per consumer — in the caller's transaction.
    ///
    /// The domain write itself is the repository's job and must happen in the
    /// same transaction. This method exists so the *other three* cannot be
    /// forgotten independently of each other.
    ///
    /// # Errors
    ///
    /// Any database error. The caller must roll back — a change whose history
    /// failed to write is precisely what ADR-006 forbids, so there is no
    /// partial-success path here.
    pub async fn record(
        scoped: &mut Scoped<'_>,
        change: &Change,
        who: &Provenance,
    ) -> Result<Uuid, sqlx::Error> {
        let workspace: WorkspaceId = scoped.workspace_id();
        let event_id = Uuid::now_v7();

        sqlx::query(
            "INSERT INTO activity_event
                 (id, workspace_id, project_id, aggregate_type, aggregate_id,
                  event_type, actor_id, changes)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(Uuid::now_v7())
        .bind(workspace.as_uuid())
        .bind(change.project_id)
        .bind(&change.aggregate_type)
        .bind(change.aggregate_id)
        .bind(&change.event_type)
        .bind(who.actor.map(|a| a.as_uuid()))
        .bind(&change.activity_changes)
        .execute(scoped.conn())
        .await?;

        sqlx::query(
            "INSERT INTO audit_event
                 (id, workspace_id, event_type, actor_id, actor_type,
                  target_type, target_id, changes, request_id, correlation_id,
                  ip_address, user_agent)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11::inet,$12)",
        )
        .bind(Uuid::now_v7())
        .bind(workspace.as_uuid())
        .bind(&change.event_type)
        .bind(who.actor.map(|a| a.as_uuid()))
        .bind(who.actor_type.as_audit_str())
        .bind(&change.aggregate_type)
        .bind(change.aggregate_id)
        .bind(&change.audit_changes)
        .bind(who.request_id.map(|r| r.as_uuid()))
        .bind(who.correlation_id.map(|c| c.as_uuid()))
        .bind(who.ip.as_deref())
        .bind(who.user_agent.as_deref())
        .execute(scoped.conn())
        .await?;

        sqlx::query(
            // `project_id` (migration 0023) is the authorization scope every
            // fan-out consumer filters on. It is written here, in the producing
            // transaction, because here is the only place it is known for
            // certain — a consumer reading it back from the aggregate gets the
            // wrong answer for a delete, and reading it out of `payload` trusts
            // a JSON field no schema enforces.
            //
            // `actor_id` (migration 0024) is the same argument for a different
            // field: it is the one thing a consumer cannot reconstruct at all.
            // `docs/29` rule 1 — "you are never notified about your own
            // action" — is unimplementable without it, and `docs/25`'s event
            // envelope specifies it. This is the only place either is filled,
            // so every event carries both or no event carries either.
            "INSERT INTO outbox_event
                 (id, workspace_id, project_id, event_type, aggregate_type,
                  aggregate_id, payload, schema_version, actor_id)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(event_id)
        .bind(workspace.as_uuid())
        .bind(change.project_id)
        .bind(&change.event_type)
        .bind(&change.aggregate_type)
        .bind(change.aggregate_id)
        .bind(&change.payload)
        .bind(change.schema_version)
        .bind(who.actor.map(|a| a.as_uuid()))
        .execute(scoped.conn())
        .await?;

        // One delivery row per consumer, in the same transaction. Creating them
        // later — at dispatch time — would mean an event with no delivery rows
        // is indistinguishable from one already delivered.
        for consumer in CONSUMERS {
            sqlx::query(
                "INSERT INTO outbox_delivery (id, workspace_id, event_id, consumer)
                 VALUES ($1,$2,$3,$4)",
            )
            .bind(Uuid::now_v7())
            .bind(workspace.as_uuid())
            .bind(event_id)
            .bind(*consumer)
            .execute(scoped.conn())
            .await?;
        }

        Ok(event_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_consumer_in_the_design_record_has_a_delivery_row() {
        // docs/25 §Consumer fan-out names six. A consumer missing here gets no
        // delivery row and therefore silently never receives anything.
        assert_eq!(CONSUMERS.len(), 6, "docs/25 names six consumers");
        for expected in [
            "sse_fanout",
            "search_projection",
            "notification_fanout",
            "automation_matcher",
            "webhook_delivery",
            "plugin_subscribers",
        ] {
            assert!(CONSUMERS.contains(&expected), "{expected} is missing");
        }
    }

    #[test]
    fn consumer_names_are_unique() {
        // A duplicate would violate the (event_id, consumer) unique constraint
        // and fail every write, which is loud — but it would fail at runtime
        // against a database rather than here.
        let mut sorted = CONSUMERS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CONSUMERS.len());
    }
}
