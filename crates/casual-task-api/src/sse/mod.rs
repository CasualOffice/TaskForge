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
//! # What is not built yet
//!
//! Said here rather than discovered later, worst first.
//!
//! - **Revocation does not close a live stream.** A security shortfall, not a
//!   missing convenience. `docs/40`'s revocation test names it — "an SSE stream
//!   held by that session closes" — and today a stream is authorized once, at
//!   connect, then runs until the client leaves or the process stops. A session
//!   revoked at 10:00 keeps receiving events. Nothing here mitigates it. Closing
//!   it needs a per-subscription cancel handle and a revalidation tick, which is
//!   the same machinery `docs/05`'s `authz_epoch` revalidation needs, so the two
//!   belong in one change.
//! - **`Last-Event-ID` replay.** `docs/05` bounds it to 5 minutes / 1,000
//!   events. The header is accepted and ignored: a reconnecting client resumes
//!   live and may have a gap it cannot see. Needs a replay buffer with its own
//!   bound and eviction policy.
//! - **Coalescing.** `docs/05` asks for one update per aggregate per 100 ms; the
//!   hub forwards each event.

pub mod authorize;
pub mod endpoint;

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
