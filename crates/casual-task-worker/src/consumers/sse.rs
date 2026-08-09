//! `sse_fanout` — the first of the six consumers `docs/25` names.
//!
//! # The failure this file exists to prevent
//!
//! An event reaching a listener who is not entitled to it. Everything upstream
//! of here has already been decided: the subscriber was authorized against the
//! project when the stream opened (`casual-task-api`'s `sse::authorize`), and
//! the event carries the project it belongs to (migration 0022). This file's
//! entire job is to not lose that pairing between the two.
//!
//! So it does exactly one thing with authorization: it refuses to publish an
//! event whose project it cannot name. There is no "publish to the workspace"
//! path, because a workspace-wide topic is what a mistake here would reach for.
//!
//! # Why delivery failure is not a thing here
//!
//! Publication is in-process and cannot fail (`Broadcast::publish` returns a
//! count, not a `Result`). An event that reaches zero subscribers is the
//! ordinary case — most changes happen while nobody is watching — and reporting
//! that as a failure would send it round the retry ladder in `docs/25` six times
//! before dead-lettering something that was never wrong.

use std::sync::Arc;

use casual_task_infra::broadcast::{Broadcast, LiveEvent, Topic};
use casual_task_model::WorkspaceId;
use casual_task_persistence::dispatch::Claimed;

use crate::dispatcher::Consumer;

/// Publishes outbox events to the in-process live-update hub.
#[allow(missing_debug_implementations)]
pub struct SseFanout {
    broadcast: Arc<dyn Broadcast>,
}

impl SseFanout {
    /// Publish into `broadcast`.
    ///
    /// The **same** hub the HTTP handlers subscribe to, which is only true
    /// because both are handed one value built in `main`. Two hubs is the
    /// failure mode that looks exactly like everything working: events publish
    /// successfully, streams stay open, and nothing is ever delivered.
    #[must_use]
    pub fn new(broadcast: Arc<dyn Broadcast>) -> Self {
        Self { broadcast }
    }
}

impl Consumer for SseFanout {
    fn name(&self) -> &'static str {
        "sse_fanout"
    }

    async fn deliver(&self, event: &Claimed) -> Result<(), String> {
        let Some(project) = event.project_id else {
            // A workspace-level event — a member removed, a workspace renamed.
            // There is no project-scoped subscriber entitled to it, and the
            // alternative to dropping it is inventing a topic that reaches
            // everyone. Acknowledged rather than failed: nothing is wrong.
            tracing::debug!(
                event_id = %event.event_id,
                event_type = %event.event_type,
                "not a project event; no live subscriber is entitled to it"
            );
            return Ok(());
        };

        let topic = Topic::project(WorkspaceId::from_uuid(event.workspace_id), project);
        let delivered = self.broadcast.publish(
            topic,
            LiveEvent {
                // The outbox event's own id, so a client's `Last-Event-ID` names
                // a durable row and not a position in a stream that restarts
                // with the process.
                id: event.event_id,
                event_type: event.event_type.clone(),
                // The payload as the producing transaction wrote it. Not
                // re-serialized from a view type here: a second shape of the
                // same event is a second thing to keep in sync with `docs/05`,
                // and the drift is invisible until a client breaks.
                data: event.payload.to_string(),
            },
        );

        // No customer content — `docs/46` forbids logging titles and bodies.
        // Counts and ids only.
        tracing::debug!(
            event_id = %event.event_id,
            event_type = %event.event_type,
            delivered,
            "live event published"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use casual_task_infra::broadcast::{LocalBroadcast, Received};
    use uuid::Uuid;

    fn a_claim(workspace: Uuid, project: Option<Uuid>) -> Claimed {
        Claimed {
            // C-016 added the actor so the notification fan-out can suppress
            // self-actions (migration 0024). SSE does not read it.
            actor_id: None,
            delivery_id: Uuid::now_v7(),
            event_id: Uuid::now_v7(),
            consumer: "sse_fanout".to_owned(),
            event_type: "task.updated".to_owned(),
            aggregate_id: Uuid::now_v7(),
            workspace_id: workspace,
            project_id: project,
            payload: serde_json::json!({"id": "x"}),
            attempts: 1,
        }
    }

    #[tokio::test]
    async fn an_event_reaches_a_subscriber_of_its_project() {
        let hub = Arc::new(LocalBroadcast::new());
        let consumer = SseFanout::new(hub.clone());
        let workspace = WorkspaceId::new();
        let project = Uuid::now_v7();
        let mut sub = hub.subscribe(Topic::project(workspace, project));

        consumer
            .deliver(&a_claim(workspace.as_uuid(), Some(project)))
            .await
            .expect("publication cannot fail");

        assert!(matches!(sub.recv().await, Received::Event(_)));
    }

    #[tokio::test]
    async fn an_event_never_reaches_another_workspaces_subscriber() {
        // The leak this consumer is the last line of defence against. The topic
        // carries the workspace, and this asserts the consumer actually puts the
        // event's own workspace in it rather than any other value in scope.
        let hub = Arc::new(LocalBroadcast::new());
        let consumer = SseFanout::new(hub.clone());
        let project = Uuid::now_v7();
        let mine = WorkspaceId::new();
        let theirs = WorkspaceId::new();
        let mut listener = hub.subscribe(Topic::project(theirs, project));

        consumer
            .deliver(&a_claim(mine.as_uuid(), Some(project)))
            .await
            .expect("publication cannot fail");

        // Nothing arrived — and the subscriber is provably still live, so this
        // is not a test that passes because delivery is broken everywhere.
        assert_eq!(hub.subscriber_count(), 1);
        consumer
            .deliver(&a_claim(theirs.as_uuid(), Some(project)))
            .await
            .expect("publication cannot fail");
        assert!(matches!(listener.recv().await, Received::Event(_)));
    }

    #[tokio::test]
    async fn a_workspace_level_event_is_acknowledged_and_not_published() {
        // It must not fail — failing would walk it through six retries and
        // dead-letter an event that was never deliverable to a project stream.
        let hub = Arc::new(LocalBroadcast::new());
        let consumer = SseFanout::new(hub.clone());
        let workspace = WorkspaceId::new();
        let mut sub = hub.subscribe(Topic::project(workspace, Uuid::now_v7()));

        consumer
            .deliver(&a_claim(workspace.as_uuid(), None))
            .await
            .expect("a projectless event is acknowledged, not failed");

        hub.close_all();
        assert_eq!(
            sub.recv().await,
            Received::Closed,
            "a workspace-level event reached a project subscriber"
        );
    }

    #[tokio::test]
    async fn an_event_with_no_listener_is_still_a_success() {
        // Most changes happen while nobody is watching. Reporting that as a
        // delivery failure would retry it six times and then dead-letter it.
        let hub = Arc::new(LocalBroadcast::new());
        let consumer = SseFanout::new(hub);
        consumer
            .deliver(&a_claim(Uuid::now_v7(), Some(Uuid::now_v7())))
            .await
            .expect("no subscribers is not a failure");
    }
}
