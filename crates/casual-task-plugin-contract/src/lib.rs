//! # casual-task-plugin-contract
//!
//! The extension contract, versioned independently of the application (ADR-015).
//!
//! **Owns:** extension point definitions, manifest types and validation, the scope registry, and signature verification (`docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md`).
//!
//! **Must never own:** an execution host. Customer code never runs in either binary (ADR-016). This crate defines types and transport only.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
