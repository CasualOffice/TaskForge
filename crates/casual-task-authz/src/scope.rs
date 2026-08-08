//! The scope containment chain (`docs/04` §The scope containment chain).
//!
//! ```text
//! WORKSPACE
//!     ├── TEAM ────────┐
//!     └── PROJECT ◀────┘        (a project may belong to a team)
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
///
/// `workspace` is not optional. Every resource belongs to exactly one
/// workspace, and a grant from another one must never contribute — which is
/// why [`Self::contains`] compares it rather than assuming the caller filtered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceScopes {
    workspace: WorkspaceId,
    team: Option<TeamId>,
    project: Option<ProjectId>,
    environment: Option<EnvironmentId>,
}

impl ResourceScopes {
    /// A resource that is only workspace-scoped — a workspace setting, a role.
    pub fn workspace(workspace: WorkspaceId) -> Self {
        Self {
            workspace,
            team: None,
            project: None,
            environment: None,
        }
    }

    /// A resource inside a project.
    pub fn project(workspace: WorkspaceId, project: ProjectId) -> Self {
        Self {
            workspace,
            team: None,
            project: Some(project),
            environment: None,
        }
    }

    /// The project's owning team, when it has one. A project may belong to a
    /// team, so a team grant reaches the project's tasks through it.
    pub fn in_team(mut self, team: TeamId) -> Self {
        self.team = Some(team);
        self
    }

    /// The environment a task is tagged to, when it has one.
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
    pub fn contains(&self, scope: &Scope) -> bool {
        match scope {
            Scope::Workspace(w) => *w == self.workspace,
            Scope::Team(t) => self.team == Some(*t),
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
}
