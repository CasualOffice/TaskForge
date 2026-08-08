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

use casual_task_model::{EnvironmentId, UserId};

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
