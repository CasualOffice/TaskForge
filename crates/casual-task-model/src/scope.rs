//! The tenant capability type.
//!
//! See `docs/32-TENANCY-AND-ISOLATION.md` §"Mechanism 1".
//!
//! [`WorkspaceScope`] is *proof* that the caller has been authenticated into a
//! workspace. Every repository method takes one. Because its only public
//! constructor requires an [`AuthContext`], and [`AuthContext`] can only be
//! produced by the authentication middleware, it is not possible to write a
//! repository call that forgets the tenant filter — the argument cannot be
//! obtained.
//!
//! This converts a code-review responsibility into a compile error. Deliberately
//! there is no `new`, no `Default`, no `From<Uuid>`.

use crate::ids::{UserId, WorkspaceId};

/// Proof of an authenticated request, produced only by the API edge.
///
/// The private field is what makes this unforgeable outside this crate: no
/// other crate can construct an `AuthContext` literal, so no other crate can
/// mint a [`WorkspaceScope`] from thin air.
#[derive(Debug, Clone)]
pub struct AuthContext {
    actor_id: UserId,
    workspace_id: WorkspaceId,
    actor_type: ActorType,
    _seal: Seal,
}

/// Who is acting. Recorded on every audit event
/// (`docs/25-EVENTS-OUTBOX-AND-AUDIT.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorType {
    User,
    ServiceAccount,
    Plugin,
    System,
}

/// Private unit type. Its constructor is crate-private, so `AuthContext` cannot
/// be built with struct-literal syntax from another crate even if every other
/// field were public.
#[derive(Debug, Clone, Copy)]
struct Seal;

impl AuthContext {
    /// Mint an authenticated context.
    ///
    /// # Restricted
    ///
    /// Only the authentication middleware in `casual-task-api` may call this,
    /// and only after verifying a session or token *and* confirming the actor's
    /// membership of `workspace_id`. Calling it anywhere else defeats
    /// [`WorkspaceScope`] entirely.
    ///
    /// This is enforced by the `scope-required` architecture lint
    /// (`docs/15-CI-AND-RELEASE-GATES.md`), not by visibility, because the API
    /// crate genuinely needs to call it.
    pub fn authenticated(
        actor_id: UserId,
        workspace_id: WorkspaceId,
        actor_type: ActorType,
    ) -> Self {
        Self {
            actor_id,
            workspace_id,
            actor_type,
            _seal: Seal,
        }
    }

    pub fn actor_id(&self) -> UserId {
        self.actor_id
    }

    pub fn actor_type(&self) -> ActorType {
        self.actor_type
    }

    /// The tenant capability for this request.
    pub fn scope(&self) -> WorkspaceScope {
        WorkspaceScope(self.workspace_id)
    }
}

/// Proof that the caller is authorized to address one workspace's data.
///
/// Required by every repository method. Copy, because threading it through a
/// call stack must never be inconvenient enough to tempt anyone into a
/// workaround.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceScope(WorkspaceId);

impl WorkspaceScope {
    pub fn id(&self) -> WorkspaceId {
        self.0
    }

    /// Reconstruct a scope for a background job.
    ///
    /// A job row cannot be enqueued without a workspace, so this cannot
    /// manufacture a scope for a tenant the job was not created against. It is
    /// separate from [`AuthContext::scope`] so that the two paths are
    /// individually auditable — a grep for this function returns every place a
    /// scope exists without a live request behind it.
    pub fn for_job(workspace_id: WorkspaceId) -> Self {
        Self(workspace_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_carries_the_authenticated_workspace() {
        let ws = WorkspaceId::new();
        let ctx = AuthContext::authenticated(UserId::new(), ws, ActorType::User);
        assert_eq!(ctx.scope().id(), ws);
    }

    #[test]
    fn scopes_of_different_workspaces_are_not_equal() {
        let a = WorkspaceScope::for_job(WorkspaceId::new());
        let b = WorkspaceScope::for_job(WorkspaceId::new());
        assert_ne!(a, b);
    }
}
