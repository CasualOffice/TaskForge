//! The active delivery consumers shared by the API-facing features.
//!
//! One module each, because they fail independently and change for unrelated
//! reasons: a webhook signature scheme and a search projection have nothing to
//! say to each other, and a file holding both would be edited by two people for
//! two reasons every time either changed.
//!
//! Search and state-interval projections live at the crate root because each is
//! also a rebuild boundary. Automation, webhooks and plugin subscribers remain
//! Phase 3 work; their delivery rows wait in PostgreSQL until those consumers
//! exist rather than disappearing from history.

pub mod notification;
pub mod sse;

pub use notification::NotificationFanout;
pub use sse::SseFanout;
