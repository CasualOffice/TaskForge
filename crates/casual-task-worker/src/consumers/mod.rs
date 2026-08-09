//! The six consumers `docs/25` §Consumer fan-out names.
//!
//! One module each, because they fail independently and change for unrelated
//! reasons: a webhook signature scheme and a search projection have nothing to
//! say to each other, and a file holding both would be edited by two people for
//! two reasons every time either changed.
//!
//! Four are not built yet — search projection, automation matcher, webhook
//! delivery and plugin subscribers arrive with C-013 and the Phase 3 work.
//! Their delivery rows are already written by `UnitOfWork::record`, so an event
//! that happens before a consumer exists is waiting for it rather than lost.

pub mod notification;
pub mod sse;

pub use notification::NotificationFanout;
pub use sse::SseFanout;
