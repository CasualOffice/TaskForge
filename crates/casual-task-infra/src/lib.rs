//! # casual-task-infra
//!
//! Optional infrastructure, each behind a trait with a local fallback.
//!
//! **Owns:** Redis, object storage, and mail adapters — so the single-node profile needs none of them (`docs/48-DEPLOYMENT-PROFILES.md`).
//!
//! **Must never own:** domain knowledge. A backend swap must never change the security model: the filesystem attachment path runs the identical handshake as S3.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! Phase 0 scaffold — no implementation yet. See `docs/14-EXECUTION-TRACKER.md`.
