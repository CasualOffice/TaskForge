//! # casual-task-attachment
//!
//! Streaming file lifecycle (`docs/28-ATTACHMENT-PIPELINE.md`).
//!
//! **Owns:** the pre-sign, verify, scan, and commit handshake, and the invariant that a row is invisible until `committed_at` is set.
//!
//! **Must never own:** object-store transport, which is a trait implemented in `casual-task-infra`.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
