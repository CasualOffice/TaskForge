//! # casual-task-observability
//!
//! Tracing, metrics, and correlation (`docs/46-OBSERVABILITY-AND-OPERATIONS.md`).
//!
//! **Owns:** the tracing subscriber, the metrics registry, and correlation-id propagation — the thread that ties a user action to every effect it caused.
//!
//! **Must never own:** customer content. Task titles, descriptions, and comment bodies never reach the logger; IDs do.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
