//! # casual-task-app
//!
//! Command and query handlers — the only layer permitted to compose domain crates.
//!
//! **Owns:** transaction boundaries and the rule that one command equals one transaction equals one activity record equals one outbox event (ADR-006).
//!
//! **Must never own:** HTTP types, SQL, or domain rules that belong in a domain crate. A handler returns `(Change, Vec<Event>)` and never holds a publisher, so it *cannot* emit an event outside the transaction.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Two compositions exist so far, both for C-006/C-008: turning stored grants
//! into an authorization decision ([`authority`]) and stored workflow rows into
//! the state machine ([`workflow`]). See `docs/14-EXECUTION-TRACKER.md`.

pub mod authority;
pub mod workflow;

pub use authority::{Authority, StoredGrant};
pub use workflow::{CompositionError, StoredStatus, StoredTransition, compose, initial};

/// The decision vocabulary, re-exported.
///
/// `docs/19` has `casual-task-authz` "consulted by the app layer". Re-exporting
/// rather than letting the API crate depend on the resolver directly keeps that
/// true: there is one path from an HTTP handler to a permission decision, and
/// it runs through here.
pub use casual_task_authz::{Decision, DenyReason, ResourceFacts};

/// Lexicographic board ranks (ADR-013), re-exported from the task domain crate
/// for the same reason.
pub use casual_task_task::rank;
