//! Correlation — the thread that ties a user action to every effect it caused.
//!
//! `docs/46-OBSERVABILITY-AND-OPERATIONS.md` §Correlation:
//!
//! ```text
//! user clicks "Done"
//!   → request_id R, correlation_id C
//!   → transition committed          [C]
//!   → outbox event                  [C]
//!   → automation matched            [C]
//!   → automation created a subtask  [C]
//!   → notification sent             [C]
//!   → webhook delivered             [C]
//! ```
//!
//! One query on `C` reconstructs the entire causal chain. Without it, "why did
//! this task appear?" is unanswerable, and `docs/46` calls that the single most
//! common support question once automations exist.
//!
//! The two ids are not interchangeable and the distinction is the whole design:
//!
//! - **`request_id`** identifies one unit of work — one HTTP request, or one
//!   outbox event being dispatched by a worker. It is minted fresh at every hop.
//! - **`correlation_id`** identifies the *cause*. It is minted **once**, at the
//!   edge, and copied unchanged into every effect, however many processes and
//!   hours later.
//!
//! [`CorrelationContext`] enforces that by construction: the only functions that
//! mint a correlation id are [`CorrelationContext::at_edge`] and
//! [`CorrelationContext::at_edge_with_request`], and every other constructor
//! carries one in. There is no setter, so a downstream hop cannot break the
//! chain by assigning a new one.
//!
//! Both id types are the ones in `casual-task-model` (`src/ids.rs`) — UUIDv7
//! newtypes, re-exported rather than redefined, so a `RequestId` minted at the
//! edge is the same type the outbox row stores (`docs/22`, `event` table).

pub use casual_task_model::{CorrelationId, RequestId};
use casual_task_model::{UserId, WorkspaceId};
use tracing::Span;

/// The identity of one unit of work and the cause it belongs to.
///
/// Carries the four fields `docs/46` §The three signals requires on every log
/// line: `request_id`, `correlation_id`, `workspace_id`, `actor_id`. The fifth,
/// `trace_id`, is an OpenTelemetry concern and is not implemented — see the
/// crate docs.
///
/// **`workspace_id` is present here on purpose.** `docs/46` allows tenant
/// identity in logs and traces (it is how per-tenant investigation works) and
/// forbids it in metric labels. The split is deliberate: this type is for logs,
/// and [`crate::labels`] is for metrics.
///
/// ```
/// use casual_task_observability::CorrelationContext;
///
/// let edge = CorrelationContext::at_edge();
/// let outbox_dispatch = edge.continued();
/// let notification = outbox_dispatch.continued();
///
/// // One cause, three units of work.
/// assert_eq!(notification.correlation_id(), edge.correlation_id());
/// assert_ne!(notification.request_id(), edge.request_id());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrelationContext {
    request_id: RequestId,
    correlation_id: CorrelationId,
    workspace_id: Option<WorkspaceId>,
    actor_id: Option<UserId>,
}

impl CorrelationContext {
    /// Begin a causal chain. **Only the API edge may call this**
    /// (`docs/46` §Correlation: "generated at the edge").
    ///
    /// Calling it anywhere downstream starts a second chain, which is exactly
    /// the bug that makes "why did this task appear?" unanswerable.
    pub fn at_edge() -> Self {
        Self::at_edge_with_request(RequestId::new())
    }

    /// Begin a causal chain with a request id the edge already minted — for
    /// example one echoed from an inbound header after validation.
    pub fn at_edge_with_request(request_id: RequestId) -> Self {
        Self {
            request_id,
            correlation_id: CorrelationId::new(),
            workspace_id: None,
            actor_id: None,
        }
    }

    /// Continue this chain in a new unit of work: a fresh `request_id`, the same
    /// `correlation_id`, and the same tenant and actor.
    ///
    /// This is the in-process hop — a command handler spawning an effect.
    #[must_use]
    pub fn continued(&self) -> Self {
        Self {
            request_id: RequestId::new(),
            ..*self
        }
    }

    /// Resume a chain that arrived from somewhere else — a worker picking up an
    /// outbox event, which carries `correlation_id` in its payload (`docs/25`
    /// §Event envelope).
    ///
    /// This is the cross-process hop, and it is the reason `correlation_id` is
    /// persisted on `event` (`docs/22`) rather than held in memory.
    pub fn resumed(correlation_id: CorrelationId) -> Self {
        Self {
            request_id: RequestId::new(),
            correlation_id,
            workspace_id: None,
            actor_id: None,
        }
    }

    /// Attach the tenant, once authentication has resolved it.
    #[must_use]
    pub fn with_workspace(mut self, workspace_id: WorkspaceId) -> Self {
        self.workspace_id = Some(workspace_id);
        self
    }

    /// Attach the acting user, once authentication has resolved it.
    #[must_use]
    pub fn with_actor(mut self, actor_id: UserId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    /// This unit of work.
    pub fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// The cause this unit of work belongs to. There is no setter.
    pub fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }

    /// The tenant, if authentication has resolved it.
    pub fn workspace_id(&self) -> Option<WorkspaceId> {
        self.workspace_id
    }

    /// The acting user, if there is one. `None` for system and worker activity.
    pub fn actor_id(&self) -> Option<UserId> {
        self.actor_id
    }

    /// Open a span carrying these fields, so every event logged inside it is
    /// queryable by `correlation_id` (`docs/46` §Logs).
    ///
    /// `name` is `&'static str` because span names are a metric-shaped
    /// dimension: they must come from source, never from request data.
    ///
    /// Only ids are recorded. No customer content is placed on a span — that is
    /// what [`Redacted`](crate::Redacted) is for.
    pub fn span(&self, name: &'static str) -> Span {
        let span = tracing::info_span!(
            "unit_of_work",
            otel.name = name,
            request_id = %self.request_id,
            correlation_id = %self.correlation_id,
            workspace_id = tracing::field::Empty,
            actor_id = tracing::field::Empty,
        );
        if let Some(workspace_id) = self.workspace_id {
            span.record("workspace_id", tracing::field::display(workspace_id));
        }
        if let Some(actor_id) = self.actor_id {
            span.record("actor_id", tracing::field::display(actor_id));
        }
        span
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn the_edge_mints_both_ids() {
        let first = CorrelationContext::at_edge();
        let second = CorrelationContext::at_edge();
        assert_ne!(
            first.correlation_id(),
            second.correlation_id(),
            "two user actions are two causes"
        );
        assert_ne!(first.request_id(), second.request_id());
        assert!(first.workspace_id().is_none());
        assert!(first.actor_id().is_none());
    }

    #[test]
    fn the_documented_causal_chain_shares_one_correlation_id() {
        // The exact chain in docs/46 §Correlation.
        let workspace = WorkspaceId::new();
        let actor = UserId::new();

        let request = CorrelationContext::at_edge()
            .with_workspace(workspace)
            .with_actor(actor);

        let transition = request.continued();
        let outbox_event = transition.continued();
        // The worker is a different process: it only has the id off the event.
        let automation_match =
            CorrelationContext::resumed(outbox_event.correlation_id()).with_workspace(workspace);
        let subtask_created = automation_match.continued();
        let notification_sent = subtask_created.continued();
        let webhook_delivered = notification_sent.continued();

        let chain = [
            request,
            transition,
            outbox_event,
            automation_match,
            subtask_created,
            notification_sent,
            webhook_delivered,
        ];

        for hop in &chain {
            assert_eq!(
                hop.correlation_id(),
                request.correlation_id(),
                "a hop broke the chain; one query on C must reconstruct all of it"
            );
        }

        let request_ids: BTreeSet<_> = chain.iter().map(CorrelationContext::request_id).collect();
        assert_eq!(
            request_ids.len(),
            chain.len(),
            "each hop is its own unit of work and needs its own request_id"
        );
    }

    #[test]
    fn continuing_carries_tenant_and_actor() {
        let workspace = WorkspaceId::new();
        let actor = UserId::new();
        let context = CorrelationContext::at_edge()
            .with_workspace(workspace)
            .with_actor(actor)
            .continued()
            .continued();

        assert_eq!(context.workspace_id(), Some(workspace));
        assert_eq!(context.actor_id(), Some(actor));
    }

    #[test]
    fn resuming_across_a_process_boundary_preserves_the_cause() {
        // What the worker actually does: it has a correlation id read from the
        // outbox row and nothing else in memory.
        let edge = CorrelationContext::at_edge().with_actor(UserId::new());
        let persisted = edge.correlation_id();

        let worker = CorrelationContext::resumed(persisted);
        assert_eq!(worker.correlation_id(), persisted);
        assert_ne!(worker.request_id(), edge.request_id());
        assert!(
            worker.actor_id().is_none(),
            "the worker did not read an actor; it must not invent one"
        );
    }

    #[test]
    fn a_chain_is_distinguishable_from_an_unrelated_one() {
        let mine = CorrelationContext::at_edge().continued().continued();
        let theirs = CorrelationContext::at_edge().continued();
        assert_ne!(mine.correlation_id(), theirs.correlation_id());
    }

    #[test]
    fn a_span_carries_the_ids_and_nothing_else() {
        // Guards against a future field being added to the span: the span
        // metadata is the contract for what reaches the log line.
        let context = CorrelationContext::at_edge()
            .with_workspace(WorkspaceId::new())
            .with_actor(UserId::new());

        // A span carries metadata only while a subscriber that enables it is
        // installed; the bare registry enables everything.
        tracing::subscriber::with_default(tracing_subscriber::registry(), || {
            let span = context.span("task.transition");
            let fields: Vec<_> = span
                .metadata()
                .expect("the registry enables every span")
                .fields()
                .iter()
                .map(|f| f.name().to_owned())
                .collect();

            assert_eq!(
                fields,
                vec![
                    "otel.name",
                    "request_id",
                    "correlation_id",
                    "workspace_id",
                    "actor_id",
                ]
            );
        });
    }
}
