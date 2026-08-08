//! # casual-task-model
//!
//! The bedrock crate. Depends on nothing else in the workspace, and everything
//! else depends on it (`docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`).
//!
//! It owns the shared vocabulary and nothing else:
//!
//! - **Typed IDs** ([`ids`]) — UUIDv7 newtypes, so a `TaskId` cannot be passed
//!   where a `ProjectId` is expected.
//! - **The tenant capability** ([`scope`]) — [`WorkspaceScope`], which every
//!   repository method requires and only authenticated middleware can mint.
//! - **The permanent state contract** ([`state`]) — five task states, frozen by
//!   a golden test.
//! - **The permission registry** ([`permission`]) — the closed set of
//!   `resource.action` keys.
//! - **Errors** ([`error`]) — typed, carrying stable registry codes.
//! - **Cursors** ([`cursor`]) — opaque pagination tokens.
//!
//! It must **not** own: any SQL, any HTTP, any business rule. Those live in
//! `casual-task-persistence`, `casual-task-api`, and the domain crates
//! respectively — enforced by the architecture lints
//! (`docs/15-CI-AND-RELEASE-GATES.md`).

pub mod cursor;
pub mod error;
pub mod ids;
pub mod permission;
pub mod scope;
pub mod state;

pub use cursor::Cursor;
pub use error::{Error, ErrorCode, Result};
pub use ids::*;
pub use permission::{Permission, PrincipalType, ScopeType};
pub use scope::{ActorType, AuthContext, WorkspaceScope};
pub use state::{Priority, TaskState, TaskType, Visibility};
