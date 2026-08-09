//! The search-projection consumer (C-013, `docs/25` §Consumer fan-out).
//!
//! # Why this is a consumer and not a trigger in the request path
//!
//! `docs/26` §The search projection: "the outbox worker refreshes the row after
//! commit". Rebuilding the document inline would make every task write wait on
//! GIN maintenance, whose pending-list flushes are bursty — the exact latency
//! spike a drag-and-drop board must not have. The cost is stated there too:
//! search is **eventually consistent, typically < 1 s**, while structured
//! filters read `task` directly and are strictly consistent. That split is
//! deliberate: a user who just typed a title expects to *filter* to it at once
//! and tolerates a beat before it is *searchable*.
//!
//! # Two pools, and why they are different roles
//!
//! The dispatch loop runs as `taskforge_dispatcher`, which by design holds
//! privileges on the two outbox tables **and nothing else** (migration 0014) —
//! it bypasses row-level security, so its grants are what bound it. This
//! consumer has to read `task`, `tag`, `comment` and write `task_search`, none
//! of which that role can touch.
//!
//! So it carries its own pool connected as `taskforge_app`, the ordinary
//! request-serving role, and every statement it issues goes through a
//! [`Scoped`] transaction exactly as a request would. The tenant comes from the
//! claimed event, and `WorkspaceScope::for_job` is the constructor that exists
//! for precisely this path — a grep for it returns every place a scope exists
//! without a live request behind it.
//!
//! The alternative — granting the dispatcher role access to the task tables —
//! would give a `BYPASSRLS` role read access to every tenant's task text, which
//! is the one thing migration 0014's grant list is written to prevent.

use casual_task_model::{WorkspaceId, WorkspaceScope};
use casual_task_persistence::dispatch::Claimed;
use casual_task_persistence::{Scoped, search};
use sqlx::PgPool;

use crate::dispatcher::Consumer;

/// The consumer name. Must match the entry in
/// [`casual_task_persistence::CONSUMERS`] or the loop is handed no work.
pub const NAME: &str = "search_projection";

/// Keeps `task_search` in step with `task`.
#[derive(Debug, Clone)]
pub struct SearchProjection {
    /// A pool as `taskforge_app` — **not** the dispatcher's. See the module
    /// docs.
    pool: PgPool,
}

impl SearchProjection {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Whether an event can change a task's search document.
    ///
    /// Every event this projection cares about is about a task, and every one
    /// of those is named `task.*` — creation, update, transition, assignment,
    /// tagging, deletion. Anything else (a workspace rename, a team change)
    /// cannot alter a document and is acknowledged without a database round
    /// trip.
    ///
    /// This is a string prefix because the event-type registry is still open
    /// (**D-053**). A closed registry is what would make this a match on a
    /// value rather than on a spelling, and until it exists a renamed event is
    /// a silently unindexed task.
    fn concerns_a_task(event_type: &str) -> bool {
        event_type.starts_with("task.")
    }
}

impl Consumer for SearchProjection {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn deliver(&self, event: &Claimed) -> Result<(), String> {
        if !Self::concerns_a_task(&event.event_type) {
            return Ok(());
        }

        let scope = WorkspaceScope::for_job(WorkspaceId::from_uuid(event.workspace_id));
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| format!("search projection could not begin: {error}"))?;
        let mut scoped = Scoped::apply(&mut tx, &scope)
            .await
            .map_err(|error| format!("search projection could not scope: {error}"))?;

        // One path for every event type, deliberately. `refresh` rebuilds from
        // current state and reports whether the task still qualifies; a task
        // that is gone, soft-deleted, or in another tenant writes no row and is
        // then removed. So `task.deleted` needs no special case, an event that
        // arrives after the task was deleted does the right thing, and a
        // redelivery of any event converges — which is what at-least-once
        // delivery requires (`docs/25`).
        let indexed = search::refresh(&mut scoped, event.aggregate_id)
            .await
            .map_err(|error| format!("rebuilding the search document failed: {error}"))?;
        if !indexed {
            search::remove(&mut scoped, event.aggregate_id)
                .await
                .map_err(|error| {
                    format!("removing a deleted task from the projection failed: {error}")
                })?;
        }

        tx.commit()
            .await
            .map_err(|error| format!("search projection could not commit: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_consumer_name_is_one_the_outbox_writes_deliveries_for() {
        // A name not in CONSUMERS is a consumer that polls forever and is never
        // handed anything — a worker that looks healthy and indexes nothing.
        assert!(
            casual_task_persistence::CONSUMERS.contains(&NAME),
            "{NAME} is not in docs/25's consumer list, so no delivery row is \
             ever written for it"
        );
    }

    #[test]
    fn only_task_events_reach_the_database() {
        for indexed in [
            "task.created",
            "task.updated",
            "task.status.changed",
            "task.reopened",
            "task.assigned",
            "task.unassigned",
            "task.tagged",
            "task.deleted",
        ] {
            assert!(
                SearchProjection::concerns_a_task(indexed),
                "{indexed} would leave the projection stale"
            );
        }
        for ignored in [
            "workspace.created",
            "workspace.member.added",
            "team.created",
            "project.updated",
        ] {
            assert!(
                !SearchProjection::concerns_a_task(ignored),
                "{ignored} would cost a pointless transaction per event"
            );
        }
    }
}
