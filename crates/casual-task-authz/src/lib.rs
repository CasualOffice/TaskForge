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
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
