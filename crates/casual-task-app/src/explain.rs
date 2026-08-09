//! Answering "why can I / can't I?" — the backing types for
//! `GET /permissions/effective` and `POST /permissions/explain` (`docs/04`).
//!
//! # The failure this prevents
//!
//! Two of them, and they pull in opposite directions.
//!
//! An **affordance the client renders and the API then refuses** — a Close
//! button that 403s. That happens when the effective set is the raw union of
//! granted permissions and ignores the constraints narrowing them.
//!
//! A **feature the user holds and never sees** — the opposite error, made by
//! evaluating constrained grants against empty facts and dropping every
//! permission that did not survive. "You may close tasks you are assigned to"
//! becomes "you may not close tasks".
//!
//! So the effective set does not answer yes or no. It answers with
//! [`Reach`]: unconditional permissions are always exercisable in the scope,
//! conditional ones are exercisable where the grant's constraints hold. A
//! client renders the first outright and the second per resource, and neither
//! failure is reachable.
//!
//! # Why the explanation is owned rather than borrowed
//!
//! `casual_task_authz::explain` returns `Contribution<'g>`, tied to the grant
//! slice inside [`Authority`](crate::authority::Authority). A handler that
//! serialises it would hold that borrow across an `await`. These types copy
//! what the answer needs, which is small and bounded — a grant has one
//! permission here and at most five constraints.

use casual_task_authz::{Constraint, Decision, DenyReason};
use casual_task_model::Permission;
use uuid::Uuid;

/// How far a granted permission reaches in a scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// At least one grant carries this permission with no constraints, so it
    /// applies to every resource in the scope.
    Unconditional,
    /// Every grant carrying it is constrained. Whether it applies to a given
    /// resource depends on that resource's facts, so the client must ask per
    /// resource rather than assume either answer.
    Conditional,
}

/// One entry in the actor's effective permission set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effective {
    pub permission: Permission,
    pub reach: Reach,
}

/// One grant that carried the permission being explained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributingGrant {
    /// `WORKSPACE`, `TEAM`, `PROJECT` or `ENVIRONMENT` — the same spelling the
    /// database stores, so an admin can find the row from the answer.
    pub scope_type: &'static str,
    pub scope_id: Uuid,
    pub permission: Permission,
    /// Constraint names, in `docs/04` §Constraint set's snake_case.
    pub constraints: Vec<&'static str>,
    /// Whether this grant's constraints hold for the resource asked about.
    /// A grant can contribute and still not allow.
    pub constraints_satisfied: bool,
}

/// Why a decision came out the way it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub allowed: bool,
    /// `None` when allowed. `docs/04` requires every deny to name its reason.
    pub deny_reason: Option<&'static str>,
    /// Every grant carrying the permission and reaching the resource — the
    /// short list `docs/04` promises. Empty when the answer is "you hold no
    /// grant for this at all", which is what `no_grant` already says.
    pub contributing: Vec<ContributingGrant>,
}

/// The wire name of a deny reason.
///
/// A total match rather than `Debug`: these strings are API, and `Debug` output
/// changing with a variant rename would be a silent contract break.
#[must_use]
pub fn deny_reason_name(reason: DenyReason) -> &'static str {
    match reason {
        DenyReason::NoGrant => "no_grant",
        DenyReason::ConstraintUnsatisfied => "constraint_unsatisfied",
    }
}

/// The wire name of a constraint.
#[must_use]
pub fn constraint_name(constraint: &Constraint) -> &'static str {
    match constraint {
        Constraint::AssigneeIsActor => "assignee_is_actor",
        Constraint::ReporterIsActor => "reporter_is_actor",
        Constraint::IsProjectMember => "is_project_member",
        Constraint::EnvironmentIn(_) => "environment_in",
        Constraint::NotExternal => "not_external",
    }
}

impl Explanation {
    /// Build from a decision and the grants behind it.
    #[must_use]
    pub fn new(decision: &Decision, contributing: Vec<ContributingGrant>) -> Self {
        Self {
            allowed: decision.is_allowed(),
            deny_reason: match decision {
                Decision::Allow => None,
                Decision::Deny(reason) => Some(deny_reason_name(*reason)),
            },
            contributing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_allow_names_no_reason_and_a_deny_always_does() {
        let allowed = Explanation::new(&Decision::Allow, Vec::new());
        assert!(allowed.allowed);
        assert_eq!(allowed.deny_reason, None);

        for reason in [DenyReason::NoGrant, DenyReason::ConstraintUnsatisfied] {
            let denied = Explanation::new(&Decision::Deny(reason), Vec::new());
            assert!(!denied.allowed);
            assert!(
                denied.deny_reason.is_some(),
                "docs/04: every Deny names the reason"
            );
        }
    }

    #[test]
    fn every_constraint_has_a_wire_name_in_the_documented_spelling() {
        // These names are API and are the same strings the `constraints` JSONB
        // column holds, so an admin reading an explanation can find the row.
        let doc = include_str!("../../../docs/04-RBAC-AND-AUTHORIZATION.md");
        for constraint in [
            Constraint::AssigneeIsActor,
            Constraint::ReporterIsActor,
            Constraint::IsProjectMember,
            Constraint::EnvironmentIn(Vec::new()),
            Constraint::NotExternal,
        ] {
            let name = constraint_name(&constraint);
            assert!(
                doc.contains(name),
                "{name} is not the spelling docs/04 uses for this constraint"
            );
        }
    }

    #[test]
    fn deny_reason_names_are_distinct() {
        assert_ne!(
            deny_reason_name(DenyReason::NoGrant),
            deny_reason_name(DenyReason::ConstraintUnsatisfied),
            "two reasons collapsing to one string makes the answer useless"
        );
    }
}
