//! Privilege-escalation controls (`docs/04` §Privilege escalation controls).
//!
//! "The rules that stop RBAC from being a self-service root exploit." An
//! additive model with no deny rules is only safe if granting is bounded —
//! otherwise any actor holding an assign permission can write themselves a role
//! containing everything.
//!
//! Five of the seven controls are decidable from the resolver's own inputs and
//! live here. The other two cannot be, and saying so is part of the contract:
//!
//! * **Last-owner protection** is a database constraint checked inside the
//!   transaction (`docs/04` control 4 says so explicitly — "not just in
//!   application code"). A check here would be advisory and would race.
//! * **Auditing** every grant and revoke needs the outbox and the audit table
//!   (C-011).
//!
//! Both are asserted by their own layers. Neither is silently skipped.

use casual_task_model::Permission;

use crate::constraint::ResourceFacts;
use crate::resolver::{Actor, Grant, Principal, allows};
use crate::scope::{ResourceScopes, Scope};

/// Why an assignment was refused. One variant per control, because "denied" on
/// its own does not tell an admin which rule they hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Control 1 — the role carries a permission the actor does not hold at
    /// that scope.
    ExceedsGrantCeiling { missing: Permission },
    /// Control 2 — the actor lacks the assign permission at or above the target
    /// scope.
    ExceedsScopeCeiling,
    /// Control 3 — `role.manage` exists only at workspace scope.
    RoleEditingIsWorkspaceScoped,
    /// Control 5 — the actor is assigning to themselves a role exceeding what
    /// they already hold.
    SelfElevation { missing: Permission },
}

/// An assignment somebody is attempting.
#[derive(Debug, Clone)]
pub struct ProposedAssignment {
    /// Who the grant would be for.
    pub principal: Principal,
    /// Where it would apply.
    pub scope: Scope,
    /// What the role carries.
    pub role_permissions: Vec<Permission>,
}

/// The permission required to assign at a given scope.
///
/// `docs/04` control 2: "Assigning at scope `S` requires the scope-appropriate
/// assign permission held **at or above** `S`."
///
/// **The registry conflates two acts at workspace scope, and this function
/// inherits that** (tracked as **D-049**). `docs/04` control 3 distinguishes
/// them — "Project managers assign roles; they do not author them". D-049
/// settled that the same split holds at workspace scope: `role.assign` is the
/// authority to grant an existing role, `role.manage` the authority to author
/// one.
///
/// Before that decision the registry had only `role.manage` above project
/// scope, so a workspace-level assigner had to hold the right to author roles
/// as well — which meant anyone who could assign could mint a role with more
/// power than their own and grant it to themselves. Control 1 below still
/// forbids granting a permission the actor does not hold, so the hole was
/// narrow; it was a hole nonetheless, and it sat where the most privileged
/// actors are.
fn assign_permission_for(scope: &Scope) -> Permission {
    match scope {
        Scope::Workspace(_) | Scope::Team(_) => casual_task_model::permission::ROLE_ASSIGN,
        Scope::Project(_) | Scope::Environment(_) => {
            casual_task_model::permission::PROJECT_ROLE_ASSIGN
        }
    }
}

/// Whether an actor may make this assignment.
///
/// Every control is checked, and the **first** failure is returned — an admin
/// fixing one problem should not have to rediscover the next by trial, but the
/// order is deterministic so the message is stable.
///
/// `resource` is the scope the assignment targets, expressed as a resource so
/// the actor's own permissions can be resolved there. `facts` are the actor's
/// facts at that scope, for any constrained grants they hold.
pub fn may_assign(
    actor: &Actor,
    proposed: &ProposedAssignment,
    resource: &ResourceScopes,
    facts: &ResourceFacts,
    grants: &[Grant],
) -> Result<(), Refusal> {
    // Control 3 — role.manage is workspace-only, so a role-authoring assignment
    // below workspace scope is refused before anything else is considered.
    if proposed
        .role_permissions
        .contains(&casual_task_model::permission::ROLE_MANAGE)
        && !matches!(proposed.scope, Scope::Workspace(_))
    {
        return Err(Refusal::RoleEditingIsWorkspaceScoped);
    }

    // Control 2 — the scope-appropriate assign permission, at or above.
    // `resource` already carries the ancestor chain, so resolving there answers
    // "at or above" without a second traversal.
    if !allows(
        actor,
        assign_permission_for(&proposed.scope),
        resource,
        facts,
        grants,
    )
    .is_allowed()
    {
        return Err(Refusal::ExceedsScopeCeiling);
    }

    // Control 1 — you cannot grant what you do not hold.
    //
    // Checked permission by permission rather than as a set operation so the
    // refusal can name the one that failed. docs/04 requires this at assignment
    // time *and* on role edit; the caller supplies the role's permissions, so
    // an edit is the same call with the new set.
    for permission in &proposed.role_permissions {
        if !allows(actor, *permission, resource, facts, grants).is_allowed() {
            return Err(Refusal::ExceedsGrantCeiling {
                missing: *permission,
            });
        }
    }

    // Control 5 — self-elevation.
    //
    // Strictly this is now redundant with control 1: an actor who holds every
    // permission in the role passes the grant ceiling, and assigning it to
    // themselves adds nothing. It is kept as its own check and its own refusal
    // because docs/04 states it separately, because the redundancy is an
    // accident of the current rules rather than a guarantee, and because
    // "you tried to elevate yourself" is the message an audit reader wants.
    if actor_is(actor, &proposed.principal) {
        for permission in &proposed.role_permissions {
            if !allows(actor, *permission, resource, facts, grants).is_allowed() {
                return Err(Refusal::SelfElevation {
                    missing: *permission,
                });
            }
        }
    }

    Ok(())
}

fn actor_is(actor: &Actor, principal: &Principal) -> bool {
    match principal {
        Principal::User(u) => *u == actor.user,
        Principal::Team(t) => actor.teams.contains(t),
        Principal::ServiceAccount(s) => actor.service_account == Some(*s),
    }
}

/// A plugin installation's effective permissions.
///
/// `docs/04` control 6: the **intersection** of the scopes it was granted and
/// the installing admin's permissions at install time. Intersection rather than
/// union is the whole control — a plugin cannot exceed the person who installed
/// it, however broad its manifest asks to be.
///
/// Taken by value at install time on purpose. If the installer is later
/// promoted, the plugin does not inherit the promotion; if they are demoted,
/// revoking the installation is a separate decision an admin makes knowingly.
pub fn plugin_ceiling(requested: &[Permission], installer_held: &[Permission]) -> Vec<Permission> {
    requested
        .iter()
        .filter(|p| installer_held.contains(p))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use casual_task_model::permission as perm;
    use casual_task_model::{ProjectId, UserId, WorkspaceId};

    fn holding(
        workspace: WorkspaceId,
        actor: UserId,
        scope: Scope,
        permissions: Vec<Permission>,
    ) -> Grant {
        Grant {
            workspace_id: workspace,
            principal: Principal::User(actor),
            scope,
            permissions,
            constraints: Vec::new(),
        }
    }

    #[test]
    fn you_cannot_grant_what_you_do_not_hold() {
        // Control 1, attempted rather than asserted about.
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let held = holding(
            w,
            actor,
            Scope::Workspace(w),
            vec![perm::PROJECT_ROLE_ASSIGN, perm::TASK_UPDATE],
        );
        let proposed = ProposedAssignment {
            principal: Principal::User(UserId::new()),
            scope: Scope::Project(p),
            // The actor does not hold TASK_DELETE.
            role_permissions: vec![perm::TASK_UPDATE, perm::TASK_DELETE],
        };

        assert_eq!(
            may_assign(
                &Actor::user(actor),
                &proposed,
                &ResourceScopes::project(w, p),
                &ResourceFacts::default(),
                &[held],
            ),
            Err(Refusal::ExceedsGrantCeiling {
                missing: perm::TASK_DELETE
            })
        );
    }

    #[test]
    fn a_project_manager_cannot_create_workspace_grants() {
        // Control 2. Holding project.role.assign at project scope must not
        // reach workspace scope — the classic upward escalation.
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let held = holding(
            w,
            actor,
            Scope::Project(p),
            vec![perm::PROJECT_ROLE_ASSIGN, perm::TASK_UPDATE],
        );
        let proposed = ProposedAssignment {
            principal: Principal::User(UserId::new()),
            scope: Scope::Workspace(w),
            role_permissions: vec![perm::TASK_UPDATE],
        };

        assert_eq!(
            may_assign(
                &Actor::user(actor),
                &proposed,
                &ResourceScopes::workspace(w),
                &ResourceFacts::default(),
                &[held],
            ),
            Err(Refusal::ExceedsScopeCeiling)
        );
    }

    #[test]
    fn a_workspace_assigner_does_not_need_the_right_to_author_roles() {
        // D-049, and the test that would have failed before it. Holding only
        // `role.assign`, this actor could not have assigned anything at
        // workspace scope while `role.manage` was the assign permission there.
        let (w, actor) = (WorkspaceId::new(), UserId::new());
        let held = holding(
            w,
            actor,
            Scope::Workspace(w),
            vec![perm::ROLE_ASSIGN, perm::TASK_UPDATE],
        );
        let proposed = ProposedAssignment {
            principal: Principal::User(UserId::new()),
            scope: Scope::Workspace(w),
            role_permissions: vec![perm::TASK_UPDATE],
        };

        assert_eq!(
            may_assign(
                &Actor::user(actor),
                &proposed,
                &ResourceScopes::workspace(w),
                &ResourceFacts::default(),
                &[held],
            ),
            Ok(())
        );
    }

    #[test]
    fn a_workspace_assigner_cannot_hand_out_the_authority_to_author_roles() {
        // The other half, and the reason the split is worth a migration. An
        // assigner who could grant `role.manage` could author a role with more
        // power than their own on the next request — escalation in two steps
        // instead of one, which is not a meaningful difference.
        let (w, actor) = (WorkspaceId::new(), UserId::new());
        let held = holding(w, actor, Scope::Workspace(w), vec![perm::ROLE_ASSIGN]);
        let proposed = ProposedAssignment {
            principal: Principal::User(UserId::new()),
            scope: Scope::Workspace(w),
            role_permissions: vec![perm::ROLE_MANAGE],
        };

        assert_eq!(
            may_assign(
                &Actor::user(actor),
                &proposed,
                &ResourceScopes::workspace(w),
                &ResourceFacts::default(),
                &[held],
            ),
            Err(Refusal::ExceedsGrantCeiling {
                missing: perm::ROLE_MANAGE
            }),
            "an actor holding only role.assign handed out role.manage"
        );
    }

    #[test]
    fn role_authoring_below_workspace_scope_is_refused() {
        // Control 3.
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let held = holding(
            w,
            actor,
            Scope::Workspace(w),
            vec![perm::PROJECT_ROLE_ASSIGN, perm::ROLE_MANAGE],
        );
        let proposed = ProposedAssignment {
            principal: Principal::User(UserId::new()),
            scope: Scope::Project(p),
            role_permissions: vec![perm::ROLE_MANAGE],
        };

        assert_eq!(
            may_assign(
                &Actor::user(actor),
                &proposed,
                &ResourceScopes::project(w, p),
                &ResourceFacts::default(),
                &[held],
            ),
            Err(Refusal::RoleEditingIsWorkspaceScoped)
        );
    }

    #[test]
    fn an_actor_cannot_elevate_themselves() {
        // Control 5. The actor holds the assign permission — which is what
        // makes this the dangerous case rather than a trivially refused one.
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let held = holding(
            w,
            actor,
            Scope::Workspace(w),
            vec![perm::PROJECT_ROLE_ASSIGN],
        );
        let proposed = ProposedAssignment {
            principal: Principal::User(actor),
            scope: Scope::Project(p),
            role_permissions: vec![perm::WORKSPACE_DELETE],
        };

        let refusal = may_assign(
            &Actor::user(actor),
            &proposed,
            &ResourceScopes::project(w, p),
            &ResourceFacts::default(),
            &[held],
        )
        .expect_err("self-elevation must be refused");
        // Either ceiling catching it is correct; what must not happen is Ok.
        assert!(matches!(
            refusal,
            Refusal::ExceedsGrantCeiling { .. } | Refusal::SelfElevation { .. }
        ));
    }

    #[test]
    fn a_legitimate_assignment_is_allowed() {
        // The counterweight. Ceilings that refuse everything would pass every
        // test above and make the product unusable.
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let held = holding(
            w,
            actor,
            Scope::Workspace(w),
            vec![perm::PROJECT_ROLE_ASSIGN, perm::TASK_UPDATE],
        );
        let proposed = ProposedAssignment {
            principal: Principal::User(UserId::new()),
            scope: Scope::Project(p),
            role_permissions: vec![perm::TASK_UPDATE],
        };

        assert_eq!(
            may_assign(
                &Actor::user(actor),
                &proposed,
                &ResourceScopes::project(w, p),
                &ResourceFacts::default(),
                &[held],
            ),
            Ok(())
        );
    }

    #[test]
    fn a_plugin_cannot_exceed_its_installer() {
        // Control 6 — intersection, not union.
        let requested = [perm::TASK_UPDATE, perm::TASK_DELETE, perm::WORKSPACE_DELETE];
        let installer = [perm::TASK_UPDATE, perm::TASK_DELETE];
        assert_eq!(
            plugin_ceiling(&requested, &installer),
            vec![perm::TASK_UPDATE, perm::TASK_DELETE]
        );
    }

    #[test]
    fn a_plugin_installed_by_a_powerless_admin_gets_nothing() {
        let requested = [perm::TASK_UPDATE];
        assert!(plugin_ceiling(&requested, &[]).is_empty());
    }
}
