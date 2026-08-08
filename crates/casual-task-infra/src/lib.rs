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
//! The mail adapter ([`mail`]) is the first of those traits to exist: SMTP with
//! STARTTLS required (D-046), and a no-op that logs when `TF_SMTP_HOST` is
//! empty — which `docs/48` §Configuration makes a supported deployment, not a
//! degraded one.

pub mod mail;

pub use mail::{Mailer, Message, SmtpConfig};
