//! Workflow structure and transition validation (`docs/23`).
//!
//! # The two-layer model, as types
//!
//! A **status** is a team's word — "In Progress", "Blocked", "In Review". A
//! **state** is one of five permanent semantic values the product reasons about.
//! Every status carries exactly one state, and there is no way to build a
//! [`Status`] without one, which is what makes "status is yours; state is ours"
//! structural rather than a convention.
//!
//! The `state` column is written in the same statement as `status_id` (`docs/23`
//! §What commits) so it can never drift. Here, the equivalent guarantee is that
//! a validated transition reports the destination *status* and its *state*
//! together — a caller cannot write one without the other because
//! [`ValidTransition`] carries both.
//!
//! # What this module does not do
//!
//! Steps 1–3 and 8 of the validation order need things this crate deliberately
//! cannot reach: reading a task (persistence), the actor's permissions
//! (`casual-task-authz`), and plugin hooks (Phase 3). The caller performs those
//! and passes the results in — see [`TransitionRequest`]. That split is what
//! lets the whole state machine be tested without a database or a runtime.

use std::collections::BTreeMap;

use casual_task_model::{Permission, StatusId, TaskState, TransitionId};

/// A team-defined status, permanently mapped to one of the five states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub id: StatusId,
    pub name: String,
    pub state: TaskState,
    pub is_initial: bool,
}

/// An allowed edge. `from` of `None` means **from any status** — `docs/23`:
/// "how 'Cancel from anywhere' is expressed without n rows".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub id: TransitionId,
    pub from: Option<StatusId>,
    pub to: StatusId,
    pub required_permission: Option<Permission>,
    /// Field names that must be present and non-empty.
    pub required_fields: Vec<String>,
    /// Whether this edge opts out of blocking-dependency gating.
    pub ignore_dependencies: bool,
}

/// A set of statuses plus the allowed transitions between them.
#[derive(Debug, Clone)]
pub struct Workflow {
    statuses: BTreeMap<StatusId, Status>,
    transitions: Vec<Transition>,
}

/// Why a workflow could not be constructed.
///
/// Construction is fallible on purpose: `docs/22` enforces the initial-status
/// rule with a partial unique index, and a type that could represent a workflow
/// the database would reject is a type that invites the mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    NoInitialStatus,
    ManyInitialStatuses(usize),
    /// A transition names a status the workflow does not contain.
    UnknownStatus(StatusId),
}

impl Workflow {
    /// # Errors
    ///
    /// [`WorkflowError`] when the shape is one the schema would refuse.
    pub fn new(statuses: Vec<Status>, transitions: Vec<Transition>) -> Result<Self, WorkflowError> {
        let initial = statuses.iter().filter(|s| s.is_initial).count();
        match initial {
            0 => return Err(WorkflowError::NoInitialStatus),
            1 => {}
            n => return Err(WorkflowError::ManyInitialStatuses(n)),
        }

        let statuses: BTreeMap<_, _> = statuses.into_iter().map(|s| (s.id, s)).collect();
        for t in &transitions {
            if let Some(from) = t.from
                && !statuses.contains_key(&from)
            {
                return Err(WorkflowError::UnknownStatus(from));
            }
            if !statuses.contains_key(&t.to) {
                return Err(WorkflowError::UnknownStatus(t.to));
            }
        }
        Ok(Self {
            statuses,
            transitions,
        })
    }

    pub fn status(&self, id: StatusId) -> Option<&Status> {
        self.statuses.get(&id)
    }

    /// The status new tasks start in.
    pub fn initial(&self) -> &Status {
        self.statuses
            .values()
            .find(|s| s.is_initial)
            .expect("construction guarantees exactly one")
    }
}

/// Everything the caller has already established, passed in rather than fetched.
#[derive(Debug, Clone, Default)]
pub struct TransitionRequest {
    /// Field names the caller supplied with a non-empty value.
    pub provided_fields: Vec<String>,
    /// Unresolved blockers the actor can see (`docs/23` step 7 — the error names
    /// the ones they can see, not the ones they cannot).
    pub unresolved_blockers: Vec<casual_task_model::TaskId>,
    /// Whether the actor holds `task.dependency.override`.
    pub may_override_dependencies: bool,
    /// Permissions the actor holds on this project, already resolved by
    /// `casual-task-authz`. This crate does not resolve permissions; it is not
    /// allowed to know how.
    pub held_permissions: Vec<Permission>,
}

/// A refusal, in the order `docs/23` fixes. The first failure is the one
/// reported, because "the error a user sees is the most actionable one, not
/// whichever check happened to run first".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// Step 4 — `TF-WFL-0002`.
    NoSuchTransition,
    /// Step 5 — `TF-WFL-0003`.
    MissingPermission(Permission),
    /// Step 6 — `TF-WFL-0004`. Names **every** missing field at once, because
    /// one per round trip is how a form becomes unusable.
    MissingFields(Vec<String>),
    /// Step 7 — `TF-WFL-0005`.
    BlockedBy(Vec<casual_task_model::TaskId>),
}

/// A transition that passed every check this crate can make.
///
/// Carries the destination status **and** its state together: the caller cannot
/// write one without the other, which is the in-memory form of the invariant
/// that `state` is written in the same statement as `status_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidTransition {
    pub transition: TransitionId,
    pub to_status: StatusId,
    pub to_state: TaskState,
}

impl Workflow {
    /// Steps 4–7 of `docs/23` §Validation order, in order.
    ///
    /// # Errors
    ///
    /// The first check that fails, as a [`Rejection`].
    pub fn validate(
        &self,
        from: StatusId,
        to: StatusId,
        request: &TransitionRequest,
    ) -> Result<ValidTransition, Rejection> {
        // 4. The edge exists. An explicit `from` beats a wildcard, so a
        // workflow that narrows one path does not lose the general one.
        let edge = self
            .transitions
            .iter()
            .find(|t| t.to == to && t.from == Some(from))
            .or_else(|| {
                self.transitions
                    .iter()
                    .find(|t| t.to == to && t.from.is_none())
            })
            .ok_or(Rejection::NoSuchTransition)?;

        // 5. The transition's own permission, separate from `task.transition`,
        // which the caller checked at step 3.
        if let Some(required) = edge.required_permission
            && !request.held_permissions.contains(&required)
        {
            return Err(Rejection::MissingPermission(required));
        }

        // 6. Required fields — all of them, named at once.
        let missing: Vec<String> = edge
            .required_fields
            .iter()
            .filter(|f| !request.provided_fields.contains(f))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(Rejection::MissingFields(missing));
        }

        // 7. Blocking dependencies, unless the edge opts out or the actor may
        // override.
        if !edge.ignore_dependencies
            && !request.may_override_dependencies
            && !request.unresolved_blockers.is_empty()
        {
            return Err(Rejection::BlockedBy(request.unresolved_blockers.clone()));
        }

        let status = self
            .statuses
            .get(&to)
            .expect("construction checked every transition target");
        Ok(ValidTransition {
            transition: edge.id,
            to_status: to,
            to_state: status.state,
        })
    }
}

#[cfg(test)]
#[path = "workflow_tests.rs"]
mod tests;
