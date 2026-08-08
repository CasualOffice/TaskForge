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
//! The scoped-connection seam is implemented. Repositories are not yet. See
//! `docs/14-EXECUTION-TRACKER.md`.

pub mod compile;
pub mod dispatch;
pub mod scoped;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod unit_of_work;

pub use compile::{AuthorizedProjectSet, Compiled, Page, Param, compile};
pub use scoped::Scoped;
pub use unit_of_work::{CONSUMERS, Change, Provenance, UnitOfWork};
