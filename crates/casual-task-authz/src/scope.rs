//! The scope containment chain (`docs/04` §The scope containment chain).
//!
//! ```text
//! WORKSPACE
//!     ├── TEAM ────────┐
//!     └── PROJECT ◀────┘        (a project may involve SEVERAL teams)
//!             └── ENVIRONMENT
//! ```
//!
//! There is no `TASK` scope, and there is deliberately no way to construct one:
//! ADR-005 excludes it because per-task grants multiply the grant table by the
//! task count and make the resolver unbounded. `ScopeType` in the model has no
//! `Task` variant either, so the omission is structural rather than a rule
//! somebody has to remember.

use casual_task_model::{EnvironmentId, ProjectId, ScopeType, TeamId, WorkspaceId};

/// Where a grant applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Workspace(WorkspaceId),
    Team(TeamId),
    Project(ProjectId),
    Environment(EnvironmentId),
}

impl Scope {
    /// The discriminant, for reporting and for the storage mapping.
    pub fn scope_type(&self) -> ScopeType {
        match self {
            Self::Workspace(_) => ScopeType::Workspace,
            Self::Team(_) => ScopeType::Team,
            Self::Project(_) => ScopeType::Project,
            Self::Environment(_) => ScopeType::Environment,
        }
    }
}

/// Where a resource sits in the chain — its own scope plus every ancestor.
///
/// `docs/04`: "For a task in project `P` (in team `T`, workspace `W`,
/// environment `E`), the **applicable scope set** is `{W, T, P, E}`."
/// `docs/03` §"Teams on a project" widens the team position to a set, so the
/// applicable scope set is `{W, T₁…Tₙ, P, E}`.
///
/// The widening is **additive only**: the set is tested for membership rather
/// than for equality, so a resource with exactly one team behaves precisely as
/// it did when the field was an `Option`. No combining rule changes — this type
/// answers containment, and [`crate::resolver`] does the combining.
///
/// `workspace` is not optional. Every resource belongs to exactly one
/// workspace, and a grant from another one must never contribute — which is
/// why [`Self::contains`] compares it rather than assuming the caller filtered.
///
/// Not `Copy`: the team set is owned. That is deliberate rather than
/// incidental — a `Copy` set would have to be bounded at some arbitrary arity,
/// and a project with one team more than the bound would silently lose reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceScopes {
    workspace: WorkspaceId,
    /// Every team the resource's project involves. Order is not significant and
    /// duplicates are removed on insertion, so `contains` is a set test.
    teams: Vec<TeamId>,
    project: Option<ProjectId>,
    environment: Option<EnvironmentId>,
}

impl ResourceScopes {
    /// A resource that is only workspace-scoped — a workspace setting, a role.
    pub fn workspace(workspace: WorkspaceId) -> Self {
        Self {
            workspace,
            teams: Vec::new(),
            project: None,
            environment: None,
        }
    }

    /// A resource inside a project.
    pub fn project(workspace: WorkspaceId, project: ProjectId) -> Self {
        Self {
            workspace,
            teams: Vec::new(),
            project: Some(project),
            environment: None,
        }
    }

    /// Add one of the project's teams. A grant on that team reaches the
    /// project's tasks through it.
    ///
    /// Additive, and idempotent: calling it twice with the same team leaves one
    /// entry, so a caller that folds a list cannot inflate the set.
    #[must_use]
    pub fn in_team(mut self, team: TeamId) -> Self {
        if !self.teams.contains(&team) {
            self.teams.push(team);
        }
        self
    }

    /// Add every team a project involves, in one call.
    #[must_use]
    pub fn in_teams<I: IntoIterator<Item = TeamId>>(mut self, teams: I) -> Self {
        for team in teams {
            self = self.in_team(team);
        }
        self
    }

    /// The teams this resource's project involves.
    #[must_use]
    pub fn team_ids(&self) -> &[TeamId] {
        &self.teams
    }

    /// The environment a task is tagged to, when it has one.
    #[must_use]
    pub fn in_environment(mut self, environment: EnvironmentId) -> Self {
        self.environment = Some(environment);
        self
    }

    pub fn workspace_id(&self) -> WorkspaceId {
        self.workspace
    }

    pub fn project_id(&self) -> Option<ProjectId> {
        self.project
    }

    pub fn environment_id(&self) -> Option<EnvironmentId> {
        self.environment
    }

    /// Whether a grant at `scope` reaches this resource.
    ///
    /// Containment only — a grant on *this* project reaches it, a grant on a
    /// *different* project does not, and a grant on an environment reaches only
    /// resources tagged to that environment.
    ///
    /// The team arm is a **membership test over the set**, which is `docs/03`'s
    /// "a grant scoped to *any* of the project's teams reaches the task". An
    /// intersection rule — reachable only by a grant on *every* team — is a
    /// rule nobody would predict, and it would hide a project from the people
    /// added to it most recently.
    #[must_use]
    pub fn contains(&self, scope: &Scope) -> bool {
        match scope {
            Scope::Workspace(w) => *w == self.workspace,
            Scope::Team(t) => self.teams.contains(t),
            Scope::Project(p) => self.project == Some(*p),
            Scope::Environment(e) => self.environment == Some(*e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (WorkspaceId, TeamId, ProjectId, EnvironmentId) {
        (
            WorkspaceId::new(),
            TeamId::new(),
            ProjectId::new(),
            EnvironmentId::new(),
        )
    }

    #[test]
    fn every_ancestor_contains_the_resource() {
        let (w, t, p, e) = ids();
        let task = ResourceScopes::project(w, p).in_team(t).in_environment(e);

        assert!(task.contains(&Scope::Workspace(w)));
        assert!(task.contains(&Scope::Team(t)));
        assert!(task.contains(&Scope::Project(p)));
        assert!(task.contains(&Scope::Environment(e)));
    }

    #[test]
    fn a_sibling_at_the_same_level_does_not_contain_it() {
        let (w, t, p, e) = ids();
        let task = ResourceScopes::project(w, p).in_team(t).in_environment(e);

        assert!(!task.contains(&Scope::Project(ProjectId::new())));
        assert!(!task.contains(&Scope::Team(TeamId::new())));
        assert!(!task.contains(&Scope::Environment(EnvironmentId::new())));
    }

    #[test]
    fn another_workspace_never_contains_it() {
        // The cross-tenant case. A workspace grant is the broadest thing there
        // is, so if containment were sloppy anywhere it would be here.
        let (w, _, p, _) = ids();
        let task = ResourceScopes::project(w, p);
        assert!(!task.contains(&Scope::Workspace(WorkspaceId::new())));
    }

    #[test]
    fn absent_levels_are_not_wildcards() {
        // A task with no environment must not be reached by an environment
        // grant. `None` means "not in one", never "in all of them".
        let (w, _, p, e) = ids();
        let untagged = ResourceScopes::project(w, p);
        assert!(!untagged.contains(&Scope::Environment(e)));
        assert!(!untagged.contains(&Scope::Team(TeamId::new())));
    }

    #[test]
    fn a_grant_on_any_one_of_several_teams_reaches_the_resource() {
        // docs/03 §Teams on a project: "A grant scoped to *any* of the
        // project's teams reaches the task." The alternative — every team —
        // would hide a project from whoever was added to it most recently.
        let (w, a, p, _) = ids();
        let (b, c) = (TeamId::new(), TeamId::new());
        let task = ResourceScopes::project(w, p).in_teams([a, b]);

        assert!(task.contains(&Scope::Team(a)));
        assert!(task.contains(&Scope::Team(b)));
        assert!(!task.contains(&Scope::Team(c)), "a team not on the project");
    }

    #[test]
    fn one_team_behaves_exactly_as_it_did_when_the_field_was_an_option() {
        // The regression this widening had to not cause. Every question that
        // could be asked of the old `Option<TeamId>` gets the same answer from
        // the one-element set: the team on it reaches, any other does not, and
        // an empty set is not a wildcard.
        let (w, t, p, _) = ids();
        let single = ResourceScopes::project(w, p).in_team(t);
        let none = ResourceScopes::project(w, p);

        assert!(single.contains(&Scope::Team(t)));
        assert!(!single.contains(&Scope::Team(TeamId::new())));
        assert!(!none.contains(&Scope::Team(t)));
        assert_eq!(single.team_ids(), &[t]);
        assert!(none.team_ids().is_empty());
    }

    #[test]
    fn adding_the_same_team_twice_leaves_one_entry() {
        // A caller folding a list must not be able to inflate the set — the
        // membership test would still be correct, but `team_ids()` is reported
        // to clients and a duplicate there is a rendering bug.
        let (w, t, p, _) = ids();
        let scopes = ResourceScopes::project(w, p).in_team(t).in_team(t);
        assert_eq!(scopes.team_ids(), &[t]);
    }

    #[test]
    fn a_project_with_no_teams_is_legal_and_reaches_nothing_by_team() {
        // docs/03: "A project with no teams is still legal." It must not become
        // reachable by *every* team grant, which is what an empty-set-as-
        // wildcard reading would do.
        let (w, t, p, _) = ids();
        let teamless = ResourceScopes::project(w, p);
        assert!(!teamless.contains(&Scope::Team(t)));
        assert!(teamless.contains(&Scope::Project(p)));
        assert!(teamless.contains(&Scope::Workspace(w)));
    }
}
