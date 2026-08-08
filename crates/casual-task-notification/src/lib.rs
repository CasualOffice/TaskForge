//! # casual-task-notification
//!
//! Relevance, not coverage (`docs/29-NOTIFICATIONS-AND-DELIVERY.md`).
//!
//! **Owns:** recipient and reason computation, rank resolution so one event yields one notification, preference evaluation, coalescing, and quiet hours.
//!
//! **Must never own:** channel transport. Email and push delivery belong to the worker and `casual-task-infra`.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
