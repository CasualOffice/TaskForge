//! `/api/v1/tasks/{id}/attachments` and `/api/v1/attachments/{id}` (C-010).
//!
//! # The handshake, and why it is three requests
//!
//! `docs/28`: files never pass through the API process's memory. The client
//! asks for permission, uploads to storage directly, then asks the API to
//! verify what landed. Proxying the bytes instead would occupy a request
//! handler for the whole upload — a handful of slow clients exhausts the pool —
//! and would put file content in the same process as the permission system.
//!
//! The cost `docs/28` names is that the object exists before it has been
//! validated, which is exactly why [`commit`] verifies and the scanner runs
//! **before** `committed_at` is set.
//!
//! # The split, by reason to change
//!
//! - [`wire`] — request and response shapes (`docs/05`).
//! - [`validate`] — field rules, delegating the substance to
//!   `casual-task-attachment`, which can be tested without HTTP.
//! - [`guard`] — who may reach an attachment, and in what state it may be
//!   served. It exists so "may they download this?" is answered once: the
//!   difference between a task's visibility and an attachment's scan verdict is
//!   precisely where a second answer would be more permissive than the first.
//! - [`handlers`] — the three steps of the handshake.

pub mod guard;
pub mod handlers;
pub mod validate;
pub mod wire;

pub use handlers::{commit, download, list, presign};
pub use wire::AttachmentView;
