//! # casual-task-authz
//!
//! The permission resolver (`docs/04-RBAC-AND-AUTHORIZATION.md`). Sits directly above the model so every domain crate *can* be authorized without any domain crate *containing* authorization.
//!
//! **Owns:** grant collection, principal expansion, the scope-chain walk, constraint evaluation, the `authz_epoch` cache, the grant and scope ceilings, and `explain()`.
//!
//! **Must never own:** HTTP, SQL, or knowledge of what a task is beyond its scope chain. Isolating it this way is what makes the matrix and escalation suites runnable without a database or a web server.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Resolution, the `authz_epoch` cache mechanics and the grant/scope ceilings
//! are implemented (C-003). Database loading stays at the persistence boundary.

pub mod cache;
pub mod ceiling;
pub mod constraint;
pub mod resolver;
pub mod scope;

pub use cache::{CacheKey, EpochCache};
pub use ceiling::{ProposedAssignment, Refusal, may_assign, plugin_ceiling};
pub use constraint::{Constraint, ResourceFacts};
pub use resolver::{
    Actor, Contribution, Decision, DenyReason, Grant, Principal, allows, effective, explain,
};
pub use scope::{ResourceScopes, Scope};
