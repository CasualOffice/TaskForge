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
            // `docs/45`: "QA and management raise anything, a developer raises
            // bugs". The constraint existed in the closed set with its own
            // tests and its own name in `explain`, and every stored grant
            // carrying it decoded to *unsatisfiable* — so the rule denied its
            // holder everything instead of narrowing them. Nothing failed
            // loudly, because failing closed looks like a strict
            // administrator.
            //
            // An unparseable member is dropped rather than made unsatisfiable:
            // a list of types this build does not know is a narrower list, not
            // a broken grant, and it still denies the types it does not name.
            "task_type_in" => argument.as_array().map_or_else(unsatisfiable, |kinds| {
                Constraint::TaskTypeIn(
                    kinds
                        .iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(task_type_of)
                        .collect(),
                )
            }),
            _ => unsatisfiable(),
        })
        .collect()
}

/// A stored task type as the model's enum.
///
/// The `UPPERCASE` spelling the filter grammar, the create body and the wire
/// all use, so a grant is written in the same words a client would send.
fn task_type_of(stored: &str) -> Option<casual_task_model::TaskType> {
    match stored {
        "TASK" => Some(casual_task_model::TaskType::Task),
        "BUG" => Some(casual_task_model::TaskType::Bug),
        "FEATURE" => Some(casual_task_model::TaskType::Feature),
        "INCIDENT" => Some(casual_task_model::TaskType::Incident),
        "REQUEST" => Some(casual_task_model::TaskType::Request),
        _ => None,
    }
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

    /// Which task types the actor may raise here, or `None` for all of them.
    ///
    /// # Why the client needs this and cannot derive it
    ///
    /// `docs/45` allows "a developer may raise a bug but not a feature", and it
    /// is expressed as `task_type_in` on the grant. `effective_in_*` reports
    /// that `task.create` is *conditional*, which is enough to know the answer
    /// may be no but not enough to draw a menu — and a create form offering
    /// four types where one will be refused is the cognitive burden the product
    /// exists to remove.
    ///
    /// # Why `None` and not "all five"
    ///
    /// The set is closed today and will not stay closed — `docs/04` allows the
    /// type vocabulary to grow. `None` means "this grant does not narrow by
    /// type", which stays true when a sixth type is added; a materialised list
    /// of five would quietly become a list that excludes the new one.
    ///
    /// This narrows a menu. It does not decide anything: the create path
    /// re-authorizes against the type actually sent, and a caller who ignores
    /// this is refused there.
    #[must_use]
    pub fn creatable_task_types_in_workspace(&self) -> Option<Vec<casual_task_model::TaskType>> {
        self.creatable_task_types(&casual_task_authz::ResourceScopes::workspace(
            self.workspace,
        ))
    }

    /// The same question, in one project.
    ///
    /// Two named methods rather than one taking a scope, mirroring
    /// `effective_in_workspace` / `effective_in_project`: `docs/19` makes this
    /// crate the only layer permitted to compose, and a handler assembling a
    /// `ResourceScopes` itself would be a second place to get the containment
    /// chain wrong.
    #[must_use]
    pub fn creatable_task_types_in_project(
        &self,
        project: ProjectId,
        teams: &[TeamId],
    ) -> Option<Vec<casual_task_model::TaskType>> {
        let scopes = casual_task_authz::ResourceScopes::project(self.workspace, project)
            .in_teams(teams.iter().copied());
        self.creatable_task_types(&scopes)
    }

    fn creatable_task_types(
        &self,
        resource: &casual_task_authz::ResourceScopes,
    ) -> Option<Vec<casual_task_model::TaskType>> {
        let mut union: Vec<casual_task_model::TaskType> = Vec::new();
        for (permission, constraints) in
            casual_task_authz::effective(&self.actor, resource, &self.grants)
        {
            if permission != casual_task_model::permission::TASK_CREATE {
                continue;
            }
            let narrowing: Vec<&Vec<casual_task_model::TaskType>> = constraints
                .iter()
                .filter_map(|c| match c {
                    Constraint::TaskTypeIn(allowed) => Some(allowed),
                    _ => None,
                })
                .collect();
            // A grant that does not narrow by type allows every type, and the
            // combining rule is a union — so one such grant settles it.
            if narrowing.is_empty() {
                return None;
            }
            for allowed in narrowing {
                for kind in allowed {
                    if !union.contains(kind) {
                        union.push(*kind);
                    }
                }
            }
        }
        Some(union)
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
#[path = "authority_tests.rs"]
mod tests;
