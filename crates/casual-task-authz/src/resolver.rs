//! Resolution: additive union, no deny rules (`docs/04` §Resolution).
//!
//! ```text
//! effective(actor, resource):
//!     principals = {actor} ∪ teams_of(actor) ∪ {service_account if acting as one}
//!     scopes     = ancestors_of(resource) ∪ {scope_of(resource)}
//!     grants     = { g | g.principal ∈ principals ∧ g.scope ∈ scopes
//!                      ∧ g.workspace_id = resource.workspace_id }
//!     return ⋃ { (p, g.constraints) | g ∈ grants, p ∈ permissions_of(g.role) }
//! ```
//!
//! **There are no deny grants**, and there is no way to express one — `Grant`
//! carries permissions and constraints, and nothing subtracts. ADR-004 makes
//! that a property of the type rather than a rule: "everyone except X" is
//! expressed by removing a grant or by project visibility.
//!
//! The one subtlety is [`allows`]: a constraint narrows *its own* grant and
//! nothing else, so an unconstrained grant always beats a constrained one.

use casual_task_model::{Permission, UserId, WorkspaceId};

use crate::constraint::{Constraint, ResourceFacts};
use crate::scope::{ResourceScopes, Scope};

/// Something a grant can be assigned to.
///
/// Modelled as an enum rather than `(PrincipalType, Uuid)` so that a user id
/// cannot be compared against a team grant by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Principal {
    User(UserId),
    Team(casual_task_model::TeamId),
    ServiceAccount(casual_task_model::UserId),
}

/// One row of `role_assignment`, with its role's permissions already resolved.
///
/// The permissions are carried rather than the role id: `docs/04`'s resolution
/// is over `permissions_of(g.role)`, and expanding roles is the persistence
/// layer's job. Keeping it out here is what lets the matrix suites run with no
/// database.
#[derive(Debug, Clone)]
pub struct Grant {
    pub workspace_id: WorkspaceId,
    pub principal: Principal,
    pub scope: Scope,
    pub permissions: Vec<Permission>,
    /// Empty means unconstrained, which is the strongest a grant can be.
    pub constraints: Vec<Constraint>,
}

/// Who the actor is, expanded to every principal that can carry a grant for
/// them.
#[derive(Debug, Clone)]
pub struct Actor {
    pub user: UserId,
    pub teams: Vec<casual_task_model::TeamId>,
    /// Set when acting as a service account rather than as the person.
    pub service_account: Option<casual_task_model::UserId>,
}

impl Actor {
    pub fn user(user: UserId) -> Self {
        Self {
            user,
            teams: Vec::new(),
            service_account: None,
        }
    }

    pub fn in_teams(mut self, teams: Vec<casual_task_model::TeamId>) -> Self {
        self.teams = teams;
        self
    }

    fn includes(&self, principal: &Principal) -> bool {
        match principal {
            Principal::User(u) => *u == self.user,
            Principal::Team(t) => self.teams.contains(t),
            Principal::ServiceAccount(s) => self.service_account == Some(*s),
        }
    }
}

/// Why a request was refused. `docs/04` distinguishes the two, because the
/// answers to "why can't I close this?" are different.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// No grant carried the permission at all.
    NoGrant,
    /// A grant carried it, but every such grant was constrained and no
    /// constraint was satisfied.
    ConstraintUnsatisfied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(DenyReason),
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// The grants that contributed a permission, in the order they were supplied.
///
/// This is what `POST /permissions/explain` returns — `docs/04` makes "why
/// can't I close this?" answerable with the actual contributing grants, so the
/// resolver produces them rather than reconstructing them afterwards.
#[derive(Debug, Clone)]
pub struct Contribution<'g> {
    pub grant: &'g Grant,
    pub constraints_satisfied: bool,
}

/// Every grant that reaches `resource` for `actor`.
///
/// Filtered on all three axes `docs/04` names: principal, scope containment,
/// and workspace. The workspace check is not redundant with scope containment —
/// a `Scope::Project` grant carries no workspace of its own, so without it a
/// grant from another tenant whose project id somehow matched would apply.
fn applicable<'g>(actor: &Actor, resource: &ResourceScopes, grants: &'g [Grant]) -> Vec<&'g Grant> {
    grants
        .iter()
        .filter(|g| g.workspace_id == resource.workspace_id())
        .filter(|g| actor.includes(&g.principal))
        .filter(|g| resource.contains(&g.scope))
        .collect()
}

/// The additive union: every `(permission, constraints)` pair reaching the
/// resource.
pub fn effective<'g>(
    actor: &Actor,
    resource: &ResourceScopes,
    grants: &'g [Grant],
) -> Vec<(Permission, &'g [Constraint])> {
    applicable(actor, resource, grants)
        .into_iter()
        .flat_map(|g| {
            g.permissions
                .iter()
                .map(move |p| (*p, g.constraints.as_slice()))
        })
        .collect()
}

/// Whether `actor` may exercise `permission` on `resource`.
///
/// The combining rule from `docs/04`, in order: no contributing grant is
/// `NoGrant`; an unconstrained grant wins outright; otherwise any satisfied
/// constraint allows; otherwise `ConstraintUnsatisfied`.
pub fn allows(
    actor: &Actor,
    permission: Permission,
    resource: &ResourceScopes,
    facts: &ResourceFacts,
    grants: &[Grant],
) -> Decision {
    let contributing: Vec<_> = effective(actor, resource, grants)
        .into_iter()
        .filter(|(p, _)| *p == permission)
        .map(|(_, c)| c)
        .collect();

    if contributing.is_empty() {
        return Decision::Deny(DenyReason::NoGrant);
    }
    // An unconstrained grant always beats a constrained one. A constraint
    // narrows the grant it is attached to and never restricts another.
    if contributing.iter().any(|c| c.is_empty()) {
        return Decision::Allow;
    }
    if contributing
        .iter()
        .any(|cs| cs.iter().all(|c| c.satisfied(actor.user, facts)))
    {
        return Decision::Allow;
    }
    Decision::Deny(DenyReason::ConstraintUnsatisfied)
}

/// The grants behind a decision, for `/permissions/explain`.
pub fn explain<'g>(
    actor: &Actor,
    permission: Permission,
    resource: &ResourceScopes,
    facts: &ResourceFacts,
    grants: &'g [Grant],
) -> Vec<Contribution<'g>> {
    applicable(actor, resource, grants)
        .into_iter()
        .filter(|g| g.permissions.contains(&permission))
        .map(|g| Contribution {
            grant: g,
            constraints_satisfied: g.constraints.iter().all(|c| c.satisfied(actor.user, facts)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use casual_task_model::{ProjectId, TeamId};

    const EDIT: Permission = casual_task_model::permission::TASK_UPDATE;

    fn grant(workspace: WorkspaceId, principal: Principal, scope: Scope) -> Grant {
        Grant {
            workspace_id: workspace,
            principal,
            scope,
            permissions: vec![EDIT],
            constraints: Vec::new(),
        }
    }

    #[test]
    fn no_grant_denies_with_the_reason_that_says_so() {
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let d = allows(
            &Actor::user(actor),
            EDIT,
            &ResourceScopes::project(w, p),
            &ResourceFacts::default(),
            &[],
        );
        assert_eq!(d, Decision::Deny(DenyReason::NoGrant));
    }

    #[test]
    fn a_workspace_grant_reaches_a_task_in_a_project() {
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let grants = [grant(w, Principal::User(actor), Scope::Workspace(w))];
        assert!(
            allows(
                &Actor::user(actor),
                EDIT,
                &ResourceScopes::project(w, p),
                &ResourceFacts::default(),
                &grants,
            )
            .is_allowed()
        );
    }

    #[test]
    fn a_grant_in_another_workspace_never_applies() {
        // The cross-tenant property, at the resolver rather than at the query.
        let (w, other, p, actor) = (
            WorkspaceId::new(),
            WorkspaceId::new(),
            ProjectId::new(),
            UserId::new(),
        );
        let grants = [grant(
            other,
            Principal::User(actor),
            Scope::Workspace(other),
        )];
        assert_eq!(
            allows(
                &Actor::user(actor),
                EDIT,
                &ResourceScopes::project(w, p),
                &ResourceFacts::default(),
                &grants,
            ),
            Decision::Deny(DenyReason::NoGrant)
        );
    }

    #[test]
    fn a_team_grant_reaches_a_member_and_not_a_stranger() {
        let (w, p, t) = (WorkspaceId::new(), ProjectId::new(), TeamId::new());
        let member = UserId::new();
        let stranger = UserId::new();
        let grants = [grant(w, Principal::Team(t), Scope::Team(t))];
        let resource = ResourceScopes::project(w, p).in_team(t);

        assert!(
            allows(
                &Actor::user(member).in_teams(vec![t]),
                EDIT,
                &resource,
                &ResourceFacts::default(),
                &grants
            )
            .is_allowed()
        );
        assert!(
            !allows(
                &Actor::user(stranger),
                EDIT,
                &resource,
                &ResourceFacts::default(),
                &grants
            )
            .is_allowed()
        );
    }

    #[test]
    fn an_unconstrained_grant_beats_a_constrained_one() {
        // docs/04's worked example: "Member (may edit only own tasks)" on the
        // workspace plus "Project Manager (unconstrained)" on one project means
        // they may edit anything in that project.
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let mut narrow = grant(w, Principal::User(actor), Scope::Workspace(w));
        narrow.constraints = vec![Constraint::AssigneeIsActor];
        let wide = grant(w, Principal::User(actor), Scope::Project(p));

        // Not an assignee, so the constrained grant alone would refuse.
        let facts = ResourceFacts::default();
        let resource = ResourceScopes::project(w, p);

        assert_eq!(
            allows(
                &Actor::user(actor),
                EDIT,
                &resource,
                &facts,
                &[narrow.clone()]
            ),
            Decision::Deny(DenyReason::ConstraintUnsatisfied)
        );
        assert!(
            allows(
                &Actor::user(actor),
                EDIT,
                &resource,
                &facts,
                &[narrow, wide]
            )
            .is_allowed(),
            "an unconstrained grant must win outright"
        );
    }

    #[test]
    fn a_constrained_grant_allows_when_its_constraint_holds() {
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let mut g = grant(w, Principal::User(actor), Scope::Workspace(w));
        g.constraints = vec![Constraint::AssigneeIsActor];
        let resource = ResourceScopes::project(w, p);

        let assigned = ResourceFacts {
            assignees: vec![actor],
            ..Default::default()
        };
        assert!(
            allows(
                &Actor::user(actor),
                EDIT,
                &resource,
                &assigned,
                &[g.clone()]
            )
            .is_allowed()
        );
        assert_eq!(
            allows(
                &Actor::user(actor),
                EDIT,
                &resource,
                &ResourceFacts::default(),
                &[g]
            ),
            Decision::Deny(DenyReason::ConstraintUnsatisfied)
        );
    }

    #[test]
    fn all_constraints_on_one_grant_must_hold_together() {
        // Constraints on a single grant narrow it jointly. Satisfying one of
        // two is not enough — that would silently widen every multi-constraint
        // role.
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let mut g = grant(w, Principal::User(actor), Scope::Workspace(w));
        g.constraints = vec![Constraint::AssigneeIsActor, Constraint::NotExternal];
        let resource = ResourceScopes::project(w, p);

        let assigned_guest = ResourceFacts {
            assignees: vec![actor],
            actor_is_guest: true,
            ..Default::default()
        };
        assert_eq!(
            allows(&Actor::user(actor), EDIT, &resource, &assigned_guest, &[g]),
            Decision::Deny(DenyReason::ConstraintUnsatisfied)
        );
    }

    #[test]
    fn explain_returns_the_contributing_grants_and_whether_each_is_satisfied() {
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let mut narrow = grant(w, Principal::User(actor), Scope::Workspace(w));
        narrow.constraints = vec![Constraint::AssigneeIsActor];
        let wide = grant(w, Principal::User(actor), Scope::Project(p));

        let grants = [narrow, wide];
        let why = explain(
            &Actor::user(actor),
            EDIT,
            &ResourceScopes::project(w, p),
            &ResourceFacts::default(),
            &grants,
        );
        assert_eq!(why.len(), 2, "both grants carry the permission");
        assert!(
            !why[0].constraints_satisfied,
            "the assignee constraint fails"
        );
        assert!(
            why[1].constraints_satisfied,
            "the unconstrained grant holds"
        );
    }

    #[test]
    fn a_permission_the_grant_does_not_carry_is_not_granted() {
        let (w, p, actor) = (WorkspaceId::new(), ProjectId::new(), UserId::new());
        let grants = [grant(w, Principal::User(actor), Scope::Workspace(w))];
        assert_eq!(
            allows(
                &Actor::user(actor),
                casual_task_model::permission::TASK_DELETE,
                &ResourceScopes::project(w, p),
                &ResourceFacts::default(),
                &grants,
            ),
            Decision::Deny(DenyReason::NoGrant)
        );
    }
}
