//! # casual-task-project
//!
//! The collaboration boundary and its satellites.
//!
//! **Owns:** projects, project membership, visibility, environments, milestones, and tags (`docs/03-DOMAIN-MODEL.md`).
//!
//! **Must never own:** tasks, or any other domain crate's aggregate.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
