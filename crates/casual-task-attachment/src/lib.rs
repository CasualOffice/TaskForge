//! # casual-task-attachment
//!
//! Streaming file lifecycle (`docs/28-ATTACHMENT-PIPELINE.md`).
//!
//! **Owns:** the pre-sign, verify, scan, and commit handshake, and the invariant that a row is invisible until `committed_at` is set.
//!
//! **Must never own:** object-store transport, which is a trait implemented in `casual-task-infra`.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! The pipeline's two decisions that need no I/O live here, and they are the
//! two that must not be made anywhere else:
//!
//! - [`sniff`] — what a file **is**, from its bytes. It takes no declared type,
//!   so it cannot be called with the client's.
//! - [`policy`] — whether an upload is allowed at all, and the object key,
//!   which is built from UUIDs so a filename cannot reach it.

pub mod policy;
pub mod sniff;

pub use policy::{Refusal, check, object_key, size_limit, workspace_prefix};
pub use sniff::{OPAQUE, PREFIX, Sniffed, agrees, sniff, stored_type};
