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

use casual_task_authz::{Actor, Constraint, Decision, Grant, Principal, ResourceFacts, Scope};
use casual_task_model::{
    EnvironmentId, Permission, ProjectId, TeamId, UserId, WorkspaceId, permission,
};

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
        team: Option<TeamId>,
        facts: &ResourceFacts,
    ) -> Decision {
        let mut scopes = casual_task_authz::ResourceScopes::project(self.workspace, project);
        if let Some(team) = team {
            scopes = scopes.in_team(team);
        }
        casual_task_authz::allows(&self.actor, permission, &scopes, facts, &self.grants)
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
                    None,
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
                    None,
                    &ResourceFacts::default()
                )
                .is_allowed()
        );
        assert!(
            !authority
                .may_in_project(
                    permission::TASK_CREATE,
                    ProjectId::new(),
                    None,
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
