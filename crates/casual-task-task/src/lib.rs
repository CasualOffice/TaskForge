//! # casual-task-task
//!
//! The universal work item.
//!
//! **Owns:** tasks, assignees, dependencies (including the depth-bounded cycle check), subtasks, and lexicographic board ranks (`docs/03`, ADR-013, ADR-019).
//!
//! **Must never own:** workflow validation, permission checks, or SQL.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
