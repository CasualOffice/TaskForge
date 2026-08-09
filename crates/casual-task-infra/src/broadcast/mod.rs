//! Live-event fan-out: one publisher, many subscribers, bounded.
//!
//! # The failure this module exists to prevent
//!
//! Two of them, and they pull in opposite directions.
//!
//! **A subscriber receiving an event it may not read.** Fan-out is the one place
//! in the product where a filter mistake is not scoped to a request: a stream is
//! long-lived, it is fed by a process-wide publisher, and one wrong comparison
//! delivers every workspace's events to every listener for as long as the
//! process runs. So the addressing here is a **type**, not a convention —
//! [`Topic`] carries a workspace *and* a project, both are compared, and there
//! is no constructor that makes a topic matching more than one project.
//!
//! **A slow subscriber consuming the server.** A client on hotel wi-fi that
//! stops reading is indistinguishable, from the server's side, from one that has
//! gone away — and an unbounded queue per such client is a memory-exhaustion
//! primitive that any browser can trigger by being slow. `docs/24` §D-040:
//! every bound names its overflow policy. This one's is
//! [`Received::Lagged`]: the subscriber is **disconnected**, not queued, and
//! the client recovers by reconnecting with `Last-Event-ID`.
//!
//! # Why it is a trait with a local implementation
//!
//! The same shape as [`crate::mail`], for the same reason. `docs/48` Profile 1
//! is one binary with no Redis and must work; the same document says SSE
//! fan-out is "single-instance" there and needs Redis at ≥ 2 API instances.
//! [`LocalBroadcast`] is the Profile 1 half. The trait is what keeps a Redis
//! implementation from having to change any caller.
//!
//! **The cost, stated:** with more than one API instance and no shared
//! implementation, a subscriber connected to instance A never sees an event
//! published on instance B. That is not a degradation this module can detect or
//! warn about — it is a deployment shape `docs/48` forbids, and nothing here
//! enforces it.

mod local;

pub use local::{LocalBroadcast, Received, Subscription};

use casual_task_model::WorkspaceId;
use uuid::Uuid;

/// Where an event is addressed, and the only thing a subscriber matches on.
///
/// # Why both halves are required
///
/// A topic keyed on the project alone would be a cross-tenant leak the first
/// time two workspaces held a project id that compared equal — which is not
/// supposed to happen, and "not supposed to" is exactly the strength of
/// guarantee this product does not accept for tenant isolation (`docs/32`).
/// Carrying the workspace costs 16 bytes and removes the question.
///
/// There is deliberately **no** "all projects" or "whole workspace" topic. A
/// wildcard subscriber is one refactor away from being the default, and the
/// blast radius of that mistake is every event in the tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Topic {
    workspace: WorkspaceId,
    project: Uuid,
}

impl Topic {
    /// The topic for one project inside one workspace.
    #[must_use]
    pub const fn project(workspace: WorkspaceId, project: Uuid) -> Self {
        Self { workspace, project }
    }

    /// The workspace half, for assertions that want to prove isolation.
    #[must_use]
    pub const fn workspace(&self) -> WorkspaceId {
        self.workspace
    }

    /// The project half.
    #[must_use]
    pub const fn project_id(&self) -> Uuid {
        self.project
    }
}

/// One event on its way to whoever is listening.
///
/// The fields are `docs/05` §Live updates' wire shape — `event:`, `id:`,
/// `data:` — resolved before publication rather than at each subscriber, so a
/// thousand listeners on a busy project serialize the payload once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveEvent {
    /// The SSE `id:` field, and what a client sends back as `Last-Event-ID`.
    pub id: Uuid,
    /// The SSE `event:` field: `task.status.changed` and friends.
    pub event_type: String,
    /// The SSE `data:` field, already serialized.
    pub data: String,
}

/// Publish and subscribe to live events.
///
/// # Errors
///
/// Deliberately none. Publication is best-effort by design: an event that
/// reaches no subscriber is not a failure — it is the ordinary case, because
/// most events happen while nobody is watching that project. The durable record
/// is `outbox_event`, and the outbox's own retry ladder is what makes delivery
/// reliable. A `Result` here would invite a caller to retry a fan-out that has
/// no one to fan out to.
pub trait Broadcast: Send + Sync + 'static {
    /// Send `event` to every current subscriber of `topic`.
    ///
    /// Returns how many subscribers it was queued for — for the caller's log,
    /// not for control flow.
    fn publish(&self, topic: Topic, event: LiveEvent) -> usize;

    /// Start listening to `topic`.
    fn subscribe(&self, topic: Topic) -> Subscription;

    /// How many subscriptions are currently open, across every topic.
    ///
    /// This is what `sse_connections_active` (`docs/46`) reports.
    fn subscriber_count(&self) -> usize;

    /// Close every subscription, so a `SIGTERM` ends streams instead of
    /// dropping them mid-frame (`docs/24` §D-041).
    fn close_all(&self);
}
