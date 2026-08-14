//! Database fixtures for integration tests in *other* crates.
//!
//! # Why this is here and not in the test that needs it
//!
//! `docs/19` §Boundary invariants: **all SQL lives in this crate**, and
//! `casual-task-lint` makes that a build failure rather than a review comment.
//! The C-011 acceptance gate lives in `casual-task-worker` — it has to, because
//! it asserts what happens when a *worker* is killed mid-batch — and it needs to
//! seed a workspace, age a claim past its expiry, and count delivery states.
//!
//! Two ways to allow that were rejected:
//!
//! - **Exempt `tests/` from the lint.** That is a hole in an architecture
//!   invariant, opened to make one test compile, and it would stay open.
//! - **Add the queries to the production API.** "Expire every claim" exists only
//!   to make a five-minute timeout testable in five milliseconds. A production
//!   surface that carries it is a production surface someone can call.
//!
//! So the SQL lives where the invariant says it must, and is compiled only when
//! a test asks for it.
//!
//! # Not compiled unless requested
//!
//! Behind the non-default `test-support` feature. A release build does not
//! contain [`expire_all_claims`]; there is no flag that reaches it.

mod accounts;
mod authz;
mod dispatch;
mod identity;
mod tasks;
mod tenant;
mod worker;

pub use accounts::*;
pub use authz::*;
pub use dispatch::*;
pub use identity::*;
pub use tasks::*;
pub use tenant::*;
pub use worker::*;
