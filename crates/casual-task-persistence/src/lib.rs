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
//! The scoped-connection seam, the project and task repositories, and workflow
//! storage are implemented. See `docs/14-EXECUTION-TRACKER.md`.

pub mod activity;
pub mod attachment;
pub mod audience;
pub mod auth;
pub mod authz;
pub mod comment;
pub mod compile;
pub mod dependency;
pub mod dispatch;
pub mod environment;
pub mod export;
pub mod health;
pub mod idempotency;
pub mod identity;
pub mod invitation;
pub mod mfa;
pub mod milestone;
pub mod notification;
pub mod project;
pub mod role;
pub mod role_edit;
pub mod scoped;
pub mod search;
pub mod tag;
pub mod task;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod unit_of_work;
pub mod workflow;
pub mod workflow_edge;
pub mod workflow_edit;
pub mod workspace;

pub use compile::{AuthorizedProjectSet, Compiled, Page, Param, compile};
pub use scoped::Scoped;
pub use unit_of_work::{CONSUMERS, Change, Provenance, UnitOfWork};
