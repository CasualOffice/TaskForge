//! # casual-task-persistence
//!
//! SQLx repository implementations. **All SQL in the system lives here** (`docs/22-DATABASE-SCHEMA.md`).
//!
//! **Owns:** connection pooling, the transaction-local RLS session variable, compile-checked `sqlx::query!` statements, and migrations.
//!
//! **Must never own:** business rules. A repository persists; it does not decide.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
