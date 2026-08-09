//! Live updates over server-sent events (C-015, `docs/05` §Live updates).
//!
//! # The failure this module exists to prevent
//!
//! Two people working the same board not seeing each other's changes — and, far
//! more expensively, one of them seeing changes they were never allowed to see.
//!
//! The split here is by which of those a change would be about:
//!
//! - [`authorize`] decides **who may hear what**. It changes when the permission
//!   model changes.
//! - [`endpoint`] decides **what the wire looks like** — framing, heartbeat, how
//!   a stream ends. It changes when `docs/05`'s protocol changes.
//!
//! The fan-out itself is not here at all. It is
//! [`casual_task_infra::broadcast`], because `docs/48` makes it the same kind of
//! thing as the mail adapter: in-process on the single-node profile, Redis above
//! it, behind one trait so no caller learns which.
//!
//! # State of C-015, honestly
//!
//! Everything `docs/05` §Live updates specifies is now built: the endpoint, the
//! event shape, `Last-Event-ID` replay bounded to 5 minutes / 1,000 events with
//! a gap notice past it, the 100 ms coalescing window, the 30 s heartbeat, and
//! revalidation on `authz_epoch` change.
//!
//! **What is not proven end to end:** no test drives `GET /api/v1/stream` over
//! HTTP and reads frames off the body. Each mechanism is asserted where it lives
//! — authorization in [`authorize`], replay and fan-out in
//! `casual_task_infra::broadcast`, coalescing in [`coalesce`], revocation in
//! [`revalidate`] against a real database — but their *assembly* in [`endpoint`]
//! is covered by construction rather than by assertion. That is why `docs/14`
//! records C-015 as `Built` and not `Gated`, in the same words.

pub mod authorize;
pub mod coalesce;
pub mod endpoint;
pub mod revalidate;

pub use endpoint::{HEARTBEAT, StreamQuery, stream};

use std::sync::Arc;

/// A fresh in-process hub, as `AppState::broadcast` wants it.
///
/// Exists so a caller building an [`crate::AppState`] does not need a direct
/// dependency on `casual-task-infra` just to name the default implementation —
/// and so that when `docs/48`'s Redis hub arrives, the choice is made in one
/// place instead of at every construction site.
#[must_use]
pub fn local_hub() -> Arc<dyn casual_task_infra::broadcast::Broadcast> {
    Arc::new(casual_task_infra::broadcast::LocalBroadcast::new())
}
