//! `POST /exports`, `GET /exports/{id}`, `GET /exports/{id}/download` (C-021).
//!
//! # The failure this module exists to prevent
//!
//! An export that fails an hour after someone asked for it, for a reason they
//! could have been told immediately.
//!
//! The endpoint's job is to refuse everything refusable **now**: an unknown
//! format, a column outside the closed set, a filter the grammar rejects. What
//! it must not do is the export — `docs/38` §Export is a job, not a request, and
//! a synchronous export holds a connection and a transaction for minutes.
//!
//! # The shape, and one thing it deliberately does not check
//!
//! There is **no workspace-level permission gate** on `POST /exports`. The
//! authoritative rule is per project — `docs/38` says `task.read` "on the
//! projects in scope" — and the runner applies exactly that, per batch, through
//! the same accessible-project set the list endpoint uses. A member whose access
//! is project-scoped would be refused by a workspace-level check while being
//! perfectly entitled to export the two projects they can see; an export that
//! returns zero rows is the correct answer for an actor with no access, and it
//! is the answer the filter already gives.
//!
//! Refusing at the edge on a *workspace* permission would therefore be both
//! wrong and more restrictive than the thing it guards. Said here because its
//! absence looks like an oversight and is not.

pub mod handlers;
pub mod wire;

pub use handlers::{create, download, read};
