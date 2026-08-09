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

/// The state machine's vocabulary, re-exported for the same reason.
///
/// `docs/19` lets this layer compose domain crates and forbids the API crate
/// from reaching past it. A transition handler needs the workflow it is
/// validating against, the facts it validates with, and the refusal it maps to
/// an error code — so those three travel through here rather than becoming a
/// second dependency edge from HTTP straight into the domain.
pub use casual_task_workflow::{Rejection, TransitionRequest, ValidTransition, Workflow};

/// The attachment pipeline's two I/O-free decisions, re-exported for the same
/// reason as the state machine's.
///
/// `docs/19` lets this layer compose domain crates and keeps the API crate from
/// reaching past it. A handler needs to know what a file *is* and whether an
/// upload is allowed; both answers come from `casual-task-attachment`, and they
/// travel through here rather than adding a second dependency edge from HTTP
/// straight into the domain.
pub mod attachment {
    pub use casual_task_attachment::policy::{
        self, DEFAULT_MAX_BYTES, DOWNLOAD_TTL_SECONDS, MAX_FILES_PER_TASK, Refusal,
        UPLOAD_TTL_SECONDS, object_key, size_limit, workspace_prefix,
    };
    pub use casual_task_attachment::sniff::{OPAQUE, PREFIX, Sniffed, agrees, sniff, stored_type};
}

/// Lexicographic board ranks (ADR-013), re-exported from the task domain crate
/// for the same reason.
pub use casual_task_task::rank;

/// The notification domain, re-exported.
///
/// `docs/19` makes this crate "the only layer permitted to compose domain
/// crates", so the worker reaches `casual-task-notification` through here
/// rather than depending on it directly.
pub use casual_task_notification as notification;
