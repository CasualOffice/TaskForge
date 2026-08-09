//! The closed constraint set (`docs/04` §Constraint set (v1 — closed)).
//!
//! Five predicates, each pure over `(actor, resource)` with every input already
//! loaded — no constraint may cause a query, because the resolver runs once per
//! list and a constraint that fetched would reintroduce the N+1 the design
//! removes.
//!
//! Adding a variant here is an **ADR trigger** (`docs/11`). This list is named
//! in `docs/04` as "the thing that grows into an unreadable policy engine if
//! left unguarded", so it is an enum rather than a trait: a new kind of
//! constraint cannot be added by a plugin, or by a crate that merely implements
//! something.

use casual_task_model::{EnvironmentId, TaskType, UserId};

/// A narrowing predicate attached to one grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constraint {
    /// Satisfied when the actor is among the task's assignees.
    AssigneeIsActor,
    /// Satisfied when the actor reported the task.
    ReporterIsActor,
    /// Satisfied when the actor holds a membership row for the task's project.
    IsProjectMember,
    /// Satisfied when the task's environment is one of these.
    EnvironmentIn(Vec<EnvironmentId>),
    /// Satisfied when the task's type is one of these.
    ///
    /// The lifecycle's answer to "developers may raise a bug against their own
    /// work but may not invent a feature" (`docs/45` §Permissions). A grant of
    /// `task.create` constrained to `[BUG, INCIDENT]` says exactly that.
    ///
    /// A constraint rather than a `task.create.bug` key per type: the registry
    /// is closed, so a key per type multiplies it by the type list and breaks
    /// the day a workspace adds a type. `EnvironmentIn` set the precedent that
    /// a member of this closed set may carry values.
    TaskTypeIn(Vec<TaskType>),
    /// Satisfied when the actor is not a guest.
    NotExternal,
}

/// Everything the five constraints need, gathered before resolution.
///
/// A struct of already-known facts rather than a handle to something queryable:
/// the type is what stops a future constraint from reaching for the database.
#[derive(Debug, Clone, Default)]
pub struct ResourceFacts {
    pub assignees: Vec<UserId>,
    pub reporter: Option<UserId>,
    /// Whether the actor holds a `project_membership` row for this resource's
    /// project.
    pub actor_is_project_member: bool,
    pub environment: Option<EnvironmentId>,
    /// The task's type — or, on a create, the type being *proposed*.
    ///
    /// The one fact here that can describe a resource which does not exist yet.
    /// That is not a special case: `docs/04` decides against a *proposed*
    /// resource on every create, and the type is simply the first proposed
    /// field any constraint has needed.
    pub task_type: Option<TaskType>,
    /// Workspace membership type is `GUEST`.
    pub actor_is_guest: bool,
}

impl Constraint {
    /// Whether this constraint is satisfied for `actor` against `facts`.
    pub fn satisfied(&self, actor: UserId, facts: &ResourceFacts) -> bool {
        match self {
            Self::AssigneeIsActor => facts.assignees.contains(&actor),
            Self::ReporterIsActor => facts.reporter == Some(actor),
            Self::IsProjectMember => facts.actor_is_project_member,
            // An unset environment satisfies nothing: a task tagged to no
            // environment is not "in every environment".
            Self::EnvironmentIn(allowed) => facts.environment.is_some_and(|e| allowed.contains(&e)),
            // An unknown type satisfies nothing, for the reason above it: a
            // caller that did not say what it is proposing has not earned a
            // decision that depends on it.
            Self::TaskTypeIn(allowed) => facts.task_type.is_some_and(|t| allowed.contains(&t)),
            Self::NotExternal => !facts.actor_is_guest,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignee_and_reporter_are_actor_specific() {
        let actor = UserId::new();
        let other = UserId::new();
        let facts = ResourceFacts {
            assignees: vec![other, actor],
            reporter: Some(other),
            ..Default::default()
        };

        assert!(Constraint::AssigneeIsActor.satisfied(actor, &facts));
        assert!(!Constraint::ReporterIsActor.satisfied(actor, &facts));
        assert!(Constraint::ReporterIsActor.satisfied(other, &facts));
    }

    #[test]
    fn an_unset_environment_is_not_a_wildcard() {
        let actor = UserId::new();
        let allowed = EnvironmentId::new();
        let untagged = ResourceFacts::default();
        assert!(!Constraint::EnvironmentIn(vec![allowed]).satisfied(actor, &untagged));

        let tagged = ResourceFacts {
            environment: Some(allowed),
            ..Default::default()
        };
        assert!(Constraint::EnvironmentIn(vec![allowed]).satisfied(actor, &tagged));
    }

    #[test]
    fn a_type_constraint_admits_only_the_listed_types() {
        // The lifecycle rule: a developer may raise a bug and may not invent a
        // feature. Both halves asserted, because a constraint that admitted
        // everything would pass a test that only checked the allowed case.
        let actor = UserId::new();
        let raising = |task_type: TaskType| ResourceFacts {
            task_type: Some(task_type),
            ..Default::default()
        };
        let developer = Constraint::TaskTypeIn(vec![TaskType::Bug, TaskType::Incident]);

        assert!(developer.satisfied(actor, &raising(TaskType::Bug)));
        assert!(developer.satisfied(actor, &raising(TaskType::Incident)));
        assert!(!developer.satisfied(actor, &raising(TaskType::Feature)));
        assert!(!developer.satisfied(actor, &raising(TaskType::Task)));
    }

    #[test]
    fn an_unstated_type_is_not_a_wildcard() {
        // A create that did not say what it is proposing must not slip through a
        // type constraint. Same direction of failure as the environment case.
        let actor = UserId::new();
        assert!(
            !Constraint::TaskTypeIn(vec![TaskType::Bug])
                .satisfied(actor, &ResourceFacts::default())
        );
    }

    #[test]
    fn an_empty_type_list_allows_nothing() {
        let actor = UserId::new();
        let facts = ResourceFacts {
            task_type: Some(TaskType::Bug),
            ..Default::default()
        };
        assert!(!Constraint::TaskTypeIn(Vec::new()).satisfied(actor, &facts));
    }

    #[test]
    fn not_external_excludes_guests() {
        let actor = UserId::new();
        let guest = ResourceFacts {
            actor_is_guest: true,
            ..Default::default()
        };
        assert!(!Constraint::NotExternal.satisfied(actor, &guest));
        assert!(Constraint::NotExternal.satisfied(actor, &ResourceFacts::default()));
    }

    #[test]
    fn an_empty_environment_list_allows_nothing() {
        // A grant constrained to no environments grants nothing, rather than
        // everything — the direction a mistake here would fail in matters.
        let actor = UserId::new();
        let facts = ResourceFacts {
            environment: Some(EnvironmentId::new()),
            ..Default::default()
        };
        assert!(!Constraint::EnvironmentIn(Vec::new()).satisfied(actor, &facts));
    }
}
