//! The state-occupancy projection (`docs/38` §Where the numbers come from).
//!
//! # Why this is a consumer and not part of the transition
//!
//! Writing the interval inline would put a second table's maintenance on the
//! latency path of every status change, which is the move a board makes on
//! every drag. The projection is a cache; `docs/38` accepts that it is
//! eventually consistent for the same reason `docs/26` accepts it for search.
//!
//! # Why every delivery rebuilds the whole series
//!
//! Delivery is at-least-once (`docs/25`). A consumer that appended an interval
//! per event would double a task's history the first time an event was
//! redelivered, and every "time in state" number would be quietly wrong with
//! nothing on screen to say so. Rebuilding from the audit stream is idempotent
//! by construction: the same event delivered five times produces the same rows.
//!
//! It also keeps the repair path exercised. A rebuild that only runs during an
//! incident is a rebuild that does not work during an incident.

use casual_task_model::{WorkspaceId, WorkspaceScope};
use casual_task_persistence::dispatch::Claimed;
use casual_task_persistence::{Scoped, state_interval};
use sqlx::PgPool;

use crate::dispatcher::Consumer;

/// Must match the entry in [`casual_task_persistence::CONSUMERS`], or the loop
/// polls forever and is handed nothing.
pub const NAME: &str = "state_interval_projection";

/// Keeps `task_state_interval` in step with the transitions in `audit_event`.
#[derive(Debug, Clone)]
pub struct StateIntervalProjection {
    /// As `taskforge_app`, not the dispatcher's role: this writes tenant rows,
    /// and the dispatcher bypasses row-level security and is granted on the two
    /// outbox tables and nothing else (migration 0014).
    pool: PgPool,
}

impl StateIntervalProjection {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Whether an event can change where a task has been.
    ///
    /// A prefix, like the search projection's, and for the same reason: the
    /// event-type registry is still open (D-053), so a renamed event is a
    /// silently unprojected task until it is closed.
    fn concerns_a_task(event_type: &str) -> bool {
        event_type.starts_with("task.")
    }
}

impl Consumer for StateIntervalProjection {
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
            .map_err(|error| format!("state projection could not begin: {error}"))?;
        let mut scoped = Scoped::apply(&mut tx, &scope)
            .await
            .map_err(|error| format!("state projection could not scope: {error}"))?;

        // One path for every event type. `rebuild` derives from current
        // history, so a deleted task writes no rows, an event that arrives
        // after the delete does the right thing, and a redelivery converges.
        state_interval::rebuild(&mut scoped, event.aggregate_id)
            .await
            .map_err(|error| format!("rebuilding the state intervals failed: {error}"))?;

        tx.commit()
            .await
            .map_err(|error| format!("state projection could not commit: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_consumer_name_is_one_the_outbox_writes_deliveries_for() {
        // A name not in CONSUMERS is a consumer that polls forever and is never
        // handed anything — a worker that looks healthy and projects nothing.
        assert!(
            casual_task_persistence::CONSUMERS.contains(&NAME),
            "{NAME} is not in docs/25's consumer list, so no delivery row is \
             ever written for it"
        );
    }
}
