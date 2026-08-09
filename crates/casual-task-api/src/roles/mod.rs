//! `/api/v1/roles` and `/api/v1/role-assignments` (C-003, `docs/04` §API).
//!
//! # The rules are not here
//!
//! `docs/04` §"The rules that stop RBAC from being a self-service root exploit"
//! lists seven controls. This module implements none of them. It reads a
//! request, asks `casual_task_authz::ceiling` through
//! [`Authority::may_assign`](casual_task_app::authority::Authority::may_assign),
//! writes rows, and turns the refusal it gets back into the `docs/20` code that
//! names the rule.
//!
//! That split is the point. The ceilings are a pure function over an actor's
//! grants, tested without a database in `casual-task-authz`, and reachable from
//! anywhere that needs them — the invitation path already uses the same idea.
//! A ceiling reimplemented here would be a second place to change one, and the
//! second place is the one that gets forgotten.
//!
//! Two controls are deliberately elsewhere:
//!
//! - **Control 4, last-owner protection**, is migration 0021's trigger, because
//!   `docs/04` requires it "as a database constraint check inside the
//!   transaction, not just in application code".
//! - **Control 7, everything is audited**, is `UnitOfWork::record`, so the grant
//!   and its audit row commit together or neither does (ADR-006).
//!
//! # Authoring and assigning are different permissions
//!
//! D-049: `role.manage` authors a role, `role.assign` grants an existing one,
//! and `project.role.assign` is the same split one level down. They were merged
//! above project scope until that decision, which meant anyone who could assign
//! could mint a role carrying more than they held and grant it to themselves.

pub mod handlers;
pub mod wire;

pub use handlers::{assign, create, list, list_assignments, revoke, update};
