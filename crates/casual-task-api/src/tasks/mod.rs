//! `/api/v1/tasks` and `/api/v1/projects/{id}/tasks` (C-008).
//!
//! # Why this is a directory and not a file
//!
//! It was one 1,757-line file with nine handlers and twelve helpers. At that
//! size nobody reads it — they grep it, land in the middle, and change what
//! they find. Two handlers had already grown slightly different ways of
//! deciding visibility.
//!
//! The split is by REASON TO CHANGE, not by size:
//!
//! - [`wire`] — the request and response shapes. Changes when `docs/05` does.
//! - [`validate`] — pure field checks. Changes when a rule does.
//! - [`guard`] — visibility and authority. Changes when `docs/04` does.
//! - [`crud`] — a task as a record.
//! - [`relations`] — transitions, assignees, tags: the operations another
//!   aggregate or the state machine decides.
//!
//! `guard` exists so the "may they?" answer cannot be assembled two ways in two
//! handlers, which is how one endpoint ends up more permissive than the one
//! beside it.

pub mod bulk;
pub mod crud;
pub mod guard;
pub mod relations;
pub mod subtasks;
pub mod tags;
pub mod validate;
pub mod wire;

pub use bulk::bulk;
pub use crud::{create, delete, list, read, update};
pub use relations::{assign, assignees, tag, transition, unassign};
pub use subtasks::list as subtasks_of;
pub use tags::{list as tags_of, remove as untag};
pub use wire::TaskView;

pub(crate) use guard::*;
pub(crate) use validate::*;
pub(crate) use wire::*;
