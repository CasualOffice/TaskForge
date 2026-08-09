//! `/api/v1/permissions/*` (C-003).
//!
//! # Why this endpoint is not a nice-to-have
//!
//! `docs/04`: *"Why can't I close this?" is the single most common support
//! question in every tracker, and the additive model is what makes the answer
//! short enough to show a user.* An authorization model that can only say no
//! forces every such question through a human who reads the grant table by
//! hand. This is that table, answered by the resolver rather than by a person
//! guessing what it would say.
//!
//! # The split
//!
//! - [`wire`] — request and response shapes.
//! - [`subject`] — whose authority is being asked about, and the one rule that
//!   governs asking about somebody else.
//! - [`handlers`] — the two endpoints.

pub mod handlers;
pub mod subject;
pub mod wire;

pub use handlers::{effective, explain};
