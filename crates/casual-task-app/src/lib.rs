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
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
