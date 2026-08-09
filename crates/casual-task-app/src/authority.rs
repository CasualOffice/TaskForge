//! Turning stored grants into a decision (`docs/04`).
//!
//! # Why this is here and not in either neighbour
//!
//! `casual-task-persistence` may not know what a `Permission` is — it persists,
//! it does not decide. `casual-task-authz` may not know what a database row
//! looks like — that is what lets the matrix and escalation suites run with no
//! database. The mapping between them has to live somewhere, and `docs/19`
//! names this crate as "the only layer permitted to compose".
//!
//! # Unrecognised input is refused, never ignored
//!
//! Three things arriving from the database can fail to parse: a scope type, a
//! permission key, and a constraint. Each is a closed set, so an unparseable
//! value means the database holds something this build does not understand —
//! during a rolling deploy, or after a hand-edited row.
//!
//! A grant carrying an unknown constraint is treated as **unsatisfiable**
//! rather than unconstrained. The alternative — dropping the constraint we
//! could not read — turns a narrowed grant into a full one, which is the
//! failure direction that hands out authority nobody granted.

use std::collections::BTreeMap;

use casual_task_authz::{Actor, Constraint, Decision, Grant, Principal, ResourceFacts, Scope};
use casual_task_model::{
    EnvironmentId, Permission, ProjectId, TeamId, UserId, WorkspaceId, permission,
};

use crate::explain::{ContributingGrant, Effective, Explanation, Reach, constraint_name};

/// One `(assignment, permission)` pair as the persistence layer reads it.
///
/// A plain mirror of `casual_task_persistence::authz::GrantRow`, declared here
/// so this crate does not depend on the persistence crate — `docs/19` puts the
/// dependency the other way round.
#[derive(Debug, Clone)]
pub struct StoredGrant {
    pub scope_type: String,
    pub scope_id: uuid::Uuid,
    pub constraints: serde_json::Value,
    pub permission: String,
}

/// A grant whose constraints could not be understood.
///
/// Represented as a `Constraint` variant that nothing satisfies, so it flows
/// through the resolver's ordinary combining rule instead of needing a special
/// case there. `EnvironmentIn(vec![])` is satisfied by no environment —
/// `casual-task-authz` has a test saying exactly that.
fn unsatisfiable() -> Constraint {
    Constraint::EnvironmentIn(Vec::new())
}

fn permission_of(key: &str) -> Option<Permission> {
    permission::ALL.iter().copied().find(|p| p.as_str() == key)
}

fn scope_of(scope_type: &str, id: uuid::Uuid) -> Option<Scope> {
    match scope_type {
        "WORKSPACE" => Some(Scope::Workspace(WorkspaceId::from_uuid(id))),
        "TEAM" => Some(Scope::Team(TeamId::from_uuid(id))),
        "PROJECT" => Some(Scope::Project(ProjectId::from_uuid(id))),
        "ENVIRONMENT" => Some(Scope::Environment(EnvironmentId::from_uuid(id))),
        // ADR-005 excludes TASK scope, and the enum has no other member. An
        // unrecognised value is a row this build cannot reason about.
        _ => None,
    }
}

/// The constraint names are `docs/04` §Constraint set, in the snake_case the
/// document writes them in.
fn constraints_of(value: &serde_json::Value) -> Vec<Constraint> {
    let Some(object) = value.as_object() else {
        // Not an object at all. `'{}'` is the column default, so this is a row
        // nobody wrote through the API.
        return vec![unsatisfiable()];
    };
    object
        .iter()
        .map(|(name, argument)| match name.as_str() {
            "assignee_is_actor" => Constraint::AssigneeIsActor,
            "reporter_is_actor" => Constraint::ReporterIsActor,
            "is_project_member" => Constraint::IsProjectMember,
            "not_external" => Constraint::NotExternal,
            "environment_in" => argument.as_array().map_or_else(unsatisfiable, |ids| {
                Constraint::EnvironmentIn(
                    ids.iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|v| v.parse::<uuid::Uuid>().ok())
                        .map(EnvironmentId::from_uuid)
                        .collect(),
                )
            }),
            _ => unsatisfiable(),
        })
        .collect()
}

/// Everything a request's authority is resolved from, gathered once.
///
/// Held for the life of the request rather than re-derived per resource:
/// `docs/04` §The list problem resolves the actor's authority **once**, and a
/// type that owns the grants is what stops a handler resolving per row.
#[derive(Debug, Clone)]
pub struct Authority {
    actor: Actor,
    workspace: WorkspaceId,
    grants: Vec<Grant>,
}

impl Authority {
    /// Build from what the persistence layer read.
    ///
    /// A row whose scope type or permission key does not parse is **dropped**,
    /// not defaulted: it can only remove authority, never add it. A row whose
    /// *constraints* do not parse is kept and made unsatisfiable — see the
    /// module docs for why the two are treated differently.
    #[must_use]
    pub fn resolved(
        actor: UserId,
        workspace: WorkspaceId,
        teams: Vec<TeamId>,
        service_account: bool,
        stored: &[StoredGrant],
    ) -> Self {
        let principal = if service_account {
            Principal::ServiceAccount(actor)
        } else {
            Principal::User(actor)
        };
        let mut resolved_actor = Actor::user(actor).in_teams(teams);
        if service_account {
            resolved_actor.service_account = Some(actor);
        }

        let grants = stored
            .iter()
            .filter_map(|row| {
                let scope = scope_of(&row.scope_type, row.scope_id)?;
                let permission = permission_of(&row.permission)?;
                // The principal a row belongs to is not stored on the row: the
                // query asked for the actor's own grants and their teams'
                // grants, so a row at TEAM scope is a team grant only when its
                // scope says so. Assigning the actor's principal to every row
                // is correct because `Actor::includes` also accepts the teams.
                Some(Grant {
                    workspace_id: workspace,
                    principal,
                    scope,
                    permissions: vec![permission],
                    constraints: constraints_of(&row.constraints),
                })
            })
            .collect();

        Self {
            actor: resolved_actor,
            workspace,
            grants,
        }
    }

    /// The workspace this authority was resolved for.
    #[must_use]
    pub fn workspace(&self) -> WorkspaceId {
        self.workspace
    }

    /// Every project the actor holds a `PROJECT`-scoped grant on.
    ///
    /// `docs/04` §Visibility vs permission: "actor holds any grant scoped to
    /// this project → yes". This is the set that clause is evaluated against.
    #[must_use]
    pub fn granted_projects(&self) -> Vec<ProjectId> {
        let mut projects: Vec<ProjectId> = self
            .grants
            .iter()
            .filter_map(|g| match g.scope {
                Scope::Project(p) => Some(p),
                _ => None,
            })
            .collect();
        projects.sort_unstable();
        projects.dedup();
        projects
    }

    /// Whether the actor may exercise `permission` at workspace scope.
    #[must_use]
    pub fn may_in_workspace(&self, permission: Permission) -> Decision {
        casual_task_authz::allows(
            &self.actor,
            permission,
            &casual_task_authz::ResourceScopes::workspace(self.workspace),
            &ResourceFacts::default(),
            &self.grants,
        )
    }

    /// Whether the actor may exercise `permission` on a resource in `project`.
    ///
    /// `facts` carries the already-loaded inputs the closed constraint set
    /// needs. It is a value, not a handle: `docs/04` requires every constraint
    /// to be a pure predicate over data already in hand, because the resolver
    /// runs once per list.
    #[must_use]
    pub fn may_in_project(
        &self,
        permission: Permission,
        project: ProjectId,
        teams: &[TeamId],
        facts: &ResourceFacts,
    ) -> Decision {
        let scopes = casual_task_authz::ResourceScopes::project(self.workspace, project)
            .in_teams(teams.iter().copied());
        casual_task_authz::allows(&self.actor, permission, &scopes, facts, &self.grants)
    }

    /// Every permission the actor holds in `resource`, with how far it reaches.
    ///
    /// `docs/04` calls this "what the client uses to render affordances", and
    /// the two ways to get it wrong are in `crate::explain`'s module docs: a
    /// raw union produces buttons the API then refuses, and evaluating
    /// constrained grants against empty facts hides features the actor has.
    /// So this reports [`Reach`] rather than a yes.
    ///
    /// Sorted and deduplicated: the same permission can arrive from several
    /// grants, and a client rendering a menu should not see it twice.
    fn effective_in(&self, resource: &casual_task_authz::ResourceScopes) -> Vec<Effective> {
        let mut reach: BTreeMap<Permission, Reach> = BTreeMap::new();
        for (permission, constraints) in
            casual_task_authz::effective(&self.actor, resource, &self.grants)
        {
            let this = if constraints.is_empty() {
                Reach::Unconditional
            } else {
                Reach::Conditional
            };
            // Unconditional wins: an unconstrained grant beats a constrained
            // one, which is the resolver's own combining rule (`docs/04`).
            // Recording Conditional over it would understate the actor's
            // authority and hide a control they can always use.
            reach
                .entry(permission)
                .and_modify(|held| {
                    if this == Reach::Unconditional {
                        *held = Reach::Unconditional;
                    }
                })
                .or_insert(this);
        }
        reach
            .into_iter()
            .map(|(permission, reach)| Effective { permission, reach })
            .collect()
    }

    /// The actor's effective permission set at workspace scope.
    #[must_use]
    pub fn effective_in_workspace(&self) -> Vec<Effective> {
        self.effective_in(&casual_task_authz::ResourceScopes::workspace(
            self.workspace,
        ))
    }

    /// The actor's effective permission set in `project`.
    #[must_use]
    pub fn effective_in_project(&self, project: ProjectId, teams: &[TeamId]) -> Vec<Effective> {
        let scopes = casual_task_authz::ResourceScopes::project(self.workspace, project)
            .in_teams(teams.iter().copied());
        self.effective_in(&scopes)
    }

    /// Whether the actor may create this grant (`docs/04` §The rules).
    ///
    /// Every ceiling lives in `casual_task_authz::ceiling`, which is a pure
    /// function and needs no database. This method exists only to hand it the
    /// actor and grants it cannot reach from outside — `docs/19` makes this
    /// crate the only layer permitted to compose, and a handler assembling an
    /// `Actor` itself would be a second place to get the principal wrong.
    ///
    /// Control 4 (last owner) is deliberately absent: `docs/04` requires it "as
    /// a database constraint check inside the transaction, not just in
    /// application code", and migration 0021 is that check.
    ///
    /// # Errors
    ///
    /// The specific [`Refusal`](casual_task_authz::Refusal), so an admin is told which rule they hit rather
    /// than "denied".
    pub fn may_assign(
        &self,
        proposed: &casual_task_authz::ProposedAssignment,
        scope: &casual_task_authz::ResourceScopes,
        facts: &ResourceFacts,
    ) -> Result<(), casual_task_authz::Refusal> {
        casual_task_authz::ceiling::may_assign(&self.actor, proposed, scope, facts, &self.grants)
    }

    /// The decision and the grants behind it, owned.
    ///
    /// Owned rather than borrowed so a handler can serialise the answer
    /// without holding a borrow of `self.grants` across an `await`; see
    /// `crate::explain`.
    fn explained(
        &self,
        permission: Permission,
        scopes: &casual_task_authz::ResourceScopes,
        facts: &ResourceFacts,
    ) -> Explanation {
        let decision =
            casual_task_authz::allows(&self.actor, permission, scopes, facts, &self.grants);
        let contributing =
            casual_task_authz::explain(&self.actor, permission, scopes, facts, &self.grants)
                .into_iter()
                .map(|c| ContributingGrant {
                    scope_type: scope_type_name(c.grant.scope),
                    scope_id: scope_id(c.grant.scope),
                    permission,
                    constraints: c.grant.constraints.iter().map(constraint_name).collect(),
                    constraints_satisfied: c.constraints_satisfied,
                })
                .collect();
        Explanation::new(&decision, contributing)
    }

    /// Explain a permission at workspace scope.
    #[must_use]
    pub fn explain_in_workspace(&self, permission: Permission) -> Explanation {
        self.explained(
            permission,
            &casual_task_authz::ResourceScopes::workspace(self.workspace),
            &ResourceFacts::default(),
        )
    }

    /// Explain a permission on a resource in `project`.
    #[must_use]
    pub fn explain_in_project(
        &self,
        permission: Permission,
        project: ProjectId,
        teams: &[TeamId],
        facts: &ResourceFacts,
    ) -> Explanation {
        let scopes = casual_task_authz::ResourceScopes::project(self.workspace, project)
            .in_teams(teams.iter().copied());
        self.explained(permission, &scopes, facts)
    }
}

/// The stored spelling of a scope type, so an explanation names the row an
/// admin would go and look at.
fn scope_type_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Workspace(_) => "WORKSPACE",
        Scope::Team(_) => "TEAM",
        Scope::Project(_) => "PROJECT",
        Scope::Environment(_) => "ENVIRONMENT",
    }
}

fn scope_id(scope: Scope) -> uuid::Uuid {
    match scope {
        Scope::Workspace(id) => id.as_uuid(),
        Scope::Team(id) => id.as_uuid(),
        Scope::Project(id) => id.as_uuid(),
        Scope::Environment(id) => id.as_uuid(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use casual_task_authz::DenyReason;

    fn stored(scope_type: &str, id: uuid::Uuid, permission: &str) -> StoredGrant {
        StoredGrant {
            scope_type: scope_type.to_owned(),
            scope_id: id,
            constraints: serde_json::json!({}),
            permission: permission.to_owned(),
        }
    }

    fn constrained(
        scope_type: &str,
        id: uuid::Uuid,
        permission: &str,
        constraints: serde_json::Value,
    ) -> StoredGrant {
        StoredGrant {
            scope_type: scope_type.to_owned(),
            scope_id: id,
            constraints,
            permission: permission.to_owned(),
        }
    }

    #[test]
    fn an_unconstrained_grant_reaches_unconditionally() {
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[stored("WORKSPACE", workspace.as_uuid(), "task.close")],
        );
        assert_eq!(
            authority.effective_in_workspace(),
            vec![Effective {
                permission: permission::TASK_CLOSE,
                reach: Reach::Unconditional
            }]
        );
    }

    #[test]
    fn a_constrained_grant_is_reported_as_conditional_not_dropped() {
        // The failure this prevents: evaluating "you may close tasks you are
        // assigned to" against empty facts and reporting "you may not close
        // tasks" — a feature the actor holds and never sees.
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[constrained(
                "WORKSPACE",
                workspace.as_uuid(),
                "task.close",
                serde_json::json!({ "assignee_is_actor": true }),
            )],
        );
        assert_eq!(
            authority.effective_in_workspace(),
            vec![Effective {
                permission: permission::TASK_CLOSE,
                reach: Reach::Conditional
            }]
        );
    }

    #[test]
    fn an_unconstrained_grant_beats_a_constrained_one_for_the_same_permission() {
        // The resolver's own combining rule: an unconstrained grant wins
        // outright. Reporting Conditional here would understate authority and
        // hide a control the actor can always use.
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[
                constrained(
                    "WORKSPACE",
                    workspace.as_uuid(),
                    "task.close",
                    serde_json::json!({ "assignee_is_actor": true }),
                ),
                stored("WORKSPACE", workspace.as_uuid(), "task.close"),
            ],
        );
        assert_eq!(
            authority.effective_in_workspace(),
            vec![Effective {
                permission: permission::TASK_CLOSE,
                reach: Reach::Unconditional
            }],
            "reported once, and unconditionally"
        );
    }

    #[test]
    fn the_effective_set_never_repeats_a_permission() {
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[
                stored("WORKSPACE", workspace.as_uuid(), "task.read"),
                stored("WORKSPACE", workspace.as_uuid(), "task.read"),
            ],
        );
        assert_eq!(authority.effective_in_workspace().len(), 1);
    }

    #[test]
    fn explaining_a_permission_nobody_granted_says_no_grant_and_lists_nothing() {
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let authority = Authority::resolved(actor, workspace, Vec::new(), false, &[]);
        let explanation = authority.explain_in_workspace(permission::TASK_CLOSE);
        assert!(!explanation.allowed);
        assert_eq!(explanation.deny_reason, Some("no_grant"));
        assert!(explanation.contributing.is_empty());
    }

    #[test]
    fn a_grant_can_contribute_and_still_not_allow() {
        // This is the whole point of `/explain`. "You have task.close through a
        // workspace grant, but it requires you to be the assignee and you are
        // not" is a useful answer; "no" is not.
        let (actor, workspace, project) = (UserId::new(), WorkspaceId::new(), ProjectId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[constrained(
                "WORKSPACE",
                workspace.as_uuid(),
                "task.close",
                serde_json::json!({ "assignee_is_actor": true }),
            )],
        );
        let explanation = authority.explain_in_project(
            permission::TASK_CLOSE,
            project,
            &[],
            &ResourceFacts::default(),
        );
        assert!(!explanation.allowed);
        assert_eq!(explanation.deny_reason, Some("constraint_unsatisfied"));
        assert_eq!(explanation.contributing.len(), 1, "the grant is named");
        let grant = &explanation.contributing[0];
        assert_eq!(grant.scope_type, "WORKSPACE");
        assert_eq!(grant.scope_id, workspace.as_uuid());
        assert_eq!(grant.constraints, vec!["assignee_is_actor"]);
        assert!(!grant.constraints_satisfied);
    }

    #[test]
    fn the_same_grant_satisfies_once_the_facts_hold() {
        let (actor, workspace, project) = (UserId::new(), WorkspaceId::new(), ProjectId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[constrained(
                "WORKSPACE",
                workspace.as_uuid(),
                "task.close",
                serde_json::json!({ "assignee_is_actor": true }),
            )],
        );
        let facts = ResourceFacts {
            assignees: vec![actor],
            ..ResourceFacts::default()
        };
        let explanation =
            authority.explain_in_project(permission::TASK_CLOSE, project, &[], &facts);
        assert!(explanation.allowed);
        assert_eq!(explanation.deny_reason, None);
        assert!(explanation.contributing[0].constraints_satisfied);
    }

    #[test]
    fn an_explanation_never_names_a_grant_from_another_workspace() {
        // `applicable` filters on workspace before anything else, and this
        // asserts the explanation inherits that rather than re-deriving it.
        let (actor, workspace, elsewhere) = (UserId::new(), WorkspaceId::new(), WorkspaceId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[stored("WORKSPACE", elsewhere.as_uuid(), "task.close")],
        );
        let explanation = authority.explain_in_workspace(permission::TASK_CLOSE);
        assert!(explanation.contributing.is_empty());
        assert_eq!(explanation.deny_reason, Some("no_grant"));
    }

    #[test]
    fn an_actor_with_no_grants_may_do_nothing() {
        // The direction that matters. A workspace with no role assignment must
        // not confer authority by virtue of membership — migration 0003:
        // "role_assignment is the ONLY source of authority in the system".
        let authority =
            Authority::resolved(UserId::new(), WorkspaceId::new(), Vec::new(), false, &[]);
        assert_eq!(
            authority.may_in_workspace(permission::PROJECT_CREATE),
            Decision::Deny(DenyReason::NoGrant)
        );
    }

    #[test]
    fn a_workspace_grant_reaches_a_project_in_that_workspace() {
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[stored("WORKSPACE", workspace.as_uuid(), "task.create")],
        );
        assert!(
            authority
                .may_in_project(
                    permission::TASK_CREATE,
                    ProjectId::new(),
                    &[],
                    &ResourceFacts::default()
                )
                .is_allowed()
        );
    }

    #[test]
    fn a_grant_from_another_workspace_never_contributes() {
        // The cross-tenant case. The resolver filters on workspace, and this is
        // the mapping that has to keep the workspace on the grant.
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[stored(
                "WORKSPACE",
                WorkspaceId::new().as_uuid(),
                "project.create",
            )],
        );
        assert_eq!(
            authority.may_in_workspace(permission::PROJECT_CREATE),
            Decision::Deny(DenyReason::NoGrant)
        );
    }

    #[test]
    fn a_project_grant_does_not_reach_a_sibling_project() {
        let (actor, workspace, project) = (UserId::new(), WorkspaceId::new(), ProjectId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[stored("PROJECT", project.as_uuid(), "task.create")],
        );
        assert!(
            authority
                .may_in_project(
                    permission::TASK_CREATE,
                    project,
                    &[],
                    &ResourceFacts::default()
                )
                .is_allowed()
        );
        assert!(
            !authority
                .may_in_project(
                    permission::TASK_CREATE,
                    ProjectId::new(),
                    &[],
                    &ResourceFacts::default()
                )
                .is_allowed()
        );
    }

    #[test]
    fn an_unreadable_constraint_narrows_the_grant_to_nothing() {
        // The failure direction. Dropping a constraint we cannot parse would
        // widen the grant to unconstrained, which hands out authority nobody
        // granted.
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let mut row = stored("WORKSPACE", workspace.as_uuid(), "project.create");
        row.constraints = serde_json::json!({ "invented_by_a_future_version": true });
        let authority = Authority::resolved(actor, workspace, Vec::new(), false, &[row]);
        assert_eq!(
            authority.may_in_workspace(permission::PROJECT_CREATE),
            Decision::Deny(DenyReason::ConstraintUnsatisfied)
        );
    }

    #[test]
    fn a_documented_constraint_is_understood() {
        // The other half: `not_external` must actually work, or the test above
        // would pass with a parser that understands nothing.
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let mut row = stored("WORKSPACE", workspace.as_uuid(), "project.create");
        row.constraints = serde_json::json!({ "not_external": true });
        let authority = Authority::resolved(actor, workspace, Vec::new(), false, &[row]);
        assert!(
            authority
                .may_in_workspace(permission::PROJECT_CREATE)
                .is_allowed()
        );
    }

    #[test]
    fn an_unknown_permission_key_removes_authority_rather_than_adding_it() {
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[stored("WORKSPACE", workspace.as_uuid(), "project.invent")],
        );
        assert_eq!(
            authority.may_in_workspace(permission::PROJECT_CREATE),
            Decision::Deny(DenyReason::NoGrant)
        );
    }

    #[test]
    fn project_scoped_grants_are_collected_for_the_visibility_clause() {
        let (actor, workspace) = (UserId::new(), WorkspaceId::new());
        let (a, b) = (ProjectId::new(), ProjectId::new());
        let authority = Authority::resolved(
            actor,
            workspace,
            Vec::new(),
            false,
            &[
                stored("PROJECT", a.as_uuid(), "task.read"),
                stored("PROJECT", a.as_uuid(), "task.create"),
                stored("PROJECT", b.as_uuid(), "task.read"),
                stored("WORKSPACE", workspace.as_uuid(), "task.read"),
            ],
        );
        let mut expected = vec![a, b];
        expected.sort_unstable();
        assert_eq!(
            authority.granted_projects(),
            expected,
            "workspace-scoped grants must not appear here: docs/04 confers \
             visibility from a grant scoped to the project, and widening it \
             would make every private project visible to every member"
        );
    }
}
