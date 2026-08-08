//! # casual-task-identity
//!
//! Identity and access primitives.
//!
//! **Owns:** users, workspace membership, teams, sessions, service accounts, and API tokens (`docs/40-IDENTITY-AUTH-AND-SESSION.md`).
//!
//! **Must never own:** permission decisions — those belong to `casual-task-authz`. Authentication answers *who*; authorization answers *what may they do*.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
