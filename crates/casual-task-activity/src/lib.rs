//! # casual-task-activity
//!
//! The three streams that make every change traceable (`docs/25-EVENTS-OUTBOX-AND-AUDIT.md`).
//!
//! **Owns:** construction of activity records, audit records, and outbox events — including denormalizing display values at write time so history stays truthful after a rename.
//!
//! **Must never own:** dispatch. Delivery is the worker's job; this crate only builds the records the transaction commits.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
