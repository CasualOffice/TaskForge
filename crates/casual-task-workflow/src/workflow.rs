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
mod tests {
    use super::*;
    use casual_task_model::{TaskId, permission as perm};

    struct Fixture {
        workflow: Workflow,
        backlog: StatusId,
        todo: StatusId,
        in_progress: StatusId,
        done: StatusId,
        canceled: StatusId,
    }

    /// The default workflow from `docs/23` §The default workflow.
    fn default_workflow() -> Fixture {
        let (backlog, todo, in_progress, done, canceled) = (
            StatusId::new(),
            StatusId::new(),
            StatusId::new(),
            StatusId::new(),
            StatusId::new(),
        );
        let status = |id, name: &str, state, is_initial| Status {
            id,
            name: name.to_owned(),
            state,
            is_initial,
        };
        let edge = |from, to| Transition {
            id: TransitionId::new(),
            from,
            to,
            required_permission: None,
            required_fields: Vec::new(),
            ignore_dependencies: false,
        };

        let statuses = vec![
            status(backlog, "Backlog", TaskState::Backlog, true),
            status(todo, "Todo", TaskState::Planned, false),
            status(in_progress, "In Progress", TaskState::Active, false),
            status(done, "Done", TaskState::Completed, false),
            status(canceled, "Canceled", TaskState::Canceled, false),
        ];
        let mut transitions = vec![
            edge(Some(backlog), todo),
            edge(Some(todo), in_progress),
            edge(Some(todo), backlog),
            edge(Some(in_progress), todo),
        ];
        // Done requires task.reopen to leave, and Cancel is from anywhere.
        transitions.push(Transition {
            id: TransitionId::new(),
            from: Some(in_progress),
            to: done,
            required_permission: None,
            required_fields: vec!["resolution".to_owned()],
            ignore_dependencies: false,
        });
        transitions.push(Transition {
            id: TransitionId::new(),
            from: Some(done),
            to: in_progress,
            required_permission: Some(perm::TASK_REOPEN),
            required_fields: Vec::new(),
            ignore_dependencies: false,
        });
        transitions.push(Transition {
            id: TransitionId::new(),
            from: None,
            to: canceled,
            required_permission: None,
            required_fields: Vec::new(),
            ignore_dependencies: true,
        });

        Fixture {
            workflow: Workflow::new(statuses, transitions).expect("valid"),
            backlog,
            todo,
            in_progress,
            done,
            canceled,
        }
    }

    #[test]
    fn a_workflow_needs_exactly_one_initial_status() {
        let s = |is_initial| Status {
            id: StatusId::new(),
            name: "x".into(),
            state: TaskState::Backlog,
            is_initial,
        };
        assert_eq!(
            Workflow::new(vec![s(false)], vec![]).err(),
            Some(WorkflowError::NoInitialStatus)
        );
        assert_eq!(
            Workflow::new(vec![s(true), s(true)], vec![]).err(),
            Some(WorkflowError::ManyInitialStatuses(2))
        );
    }

    #[test]
    fn a_transition_to_an_unknown_status_is_refused() {
        let only = Status {
            id: StatusId::new(),
            name: "Backlog".into(),
            state: TaskState::Backlog,
            is_initial: true,
        };
        let stranger = StatusId::new();
        let err = Workflow::new(
            vec![only],
            vec![Transition {
                id: TransitionId::new(),
                from: None,
                to: stranger,
                required_permission: None,
                required_fields: Vec::new(),
                ignore_dependencies: false,
            }],
        );
        assert_eq!(err.err(), Some(WorkflowError::UnknownStatus(stranger)));
    }

    #[test]
    fn a_missing_edge_is_refused() {
        let f = default_workflow();
        assert_eq!(
            f.workflow
                .validate(f.backlog, f.done, &TransitionRequest::default()),
            Err(Rejection::NoSuchTransition)
        );
    }

    #[test]
    fn cancel_works_from_anywhere_without_an_edge_per_status() {
        let f = default_workflow();
        for from in [f.backlog, f.todo, f.in_progress, f.done] {
            let v = f
                .workflow
                .validate(from, f.canceled, &TransitionRequest::default())
                .expect("cancel is a wildcard edge");
            assert_eq!(v.to_state, TaskState::Canceled);
        }
    }

    #[test]
    fn the_destination_state_comes_with_the_destination_status() {
        // The in-memory form of "state is written in the same statement as
        // status_id" — a caller cannot get one without the other.
        let f = default_workflow();
        let v = f
            .workflow
            .validate(f.backlog, f.todo, &TransitionRequest::default())
            .expect("backlog -> todo exists");
        assert_eq!(v.to_status, f.todo);
        assert_eq!(v.to_state, TaskState::Planned);
    }

    #[test]
    fn reopening_from_done_requires_the_permission() {
        let f = default_workflow();
        assert_eq!(
            f.workflow
                .validate(f.done, f.in_progress, &TransitionRequest::default()),
            Err(Rejection::MissingPermission(perm::TASK_REOPEN))
        );

        let held = TransitionRequest {
            held_permissions: vec![perm::TASK_REOPEN],
            ..Default::default()
        };
        assert!(f.workflow.validate(f.done, f.in_progress, &held).is_ok());
    }

    #[test]
    fn every_missing_field_is_named_at_once() {
        // docs/23 step 6: "naming every missing field at once (not one per
        // round-trip)".
        let (a, b) = (StatusId::new(), StatusId::new());
        let workflow = Workflow::new(
            vec![
                Status {
                    id: a,
                    name: "A".into(),
                    state: TaskState::Active,
                    is_initial: true,
                },
                Status {
                    id: b,
                    name: "B".into(),
                    state: TaskState::Completed,
                    is_initial: false,
                },
            ],
            vec![Transition {
                id: TransitionId::new(),
                from: Some(a),
                to: b,
                required_permission: None,
                required_fields: vec!["resolution".into(), "root_cause".into()],
                ignore_dependencies: false,
            }],
        )
        .expect("valid");

        assert_eq!(
            workflow.validate(a, b, &TransitionRequest::default()),
            Err(Rejection::MissingFields(vec![
                "resolution".into(),
                "root_cause".into()
            ]))
        );
    }

    #[test]
    fn blockers_gate_the_transition_unless_overridden_or_opted_out() {
        let f = default_workflow();
        let blocker = TaskId::new();
        let blocked = TransitionRequest {
            provided_fields: vec!["resolution".into()],
            unresolved_blockers: vec![blocker],
            ..Default::default()
        };

        assert_eq!(
            f.workflow.validate(f.in_progress, f.done, &blocked),
            Err(Rejection::BlockedBy(vec![blocker]))
        );

        let overriding = TransitionRequest {
            may_override_dependencies: true,
            ..blocked.clone()
        };
        assert!(
            f.workflow
                .validate(f.in_progress, f.done, &overriding)
                .is_ok()
        );

        // Cancel opts out of dependency gating entirely.
        assert!(
            f.workflow
                .validate(f.in_progress, f.canceled, &blocked)
                .is_ok()
        );
    }

    #[test]
    fn the_first_failure_is_the_one_reported() {
        // docs/23 fixes the order. A request failing both the permission check
        // (step 5) and the field check (step 6) must report the permission —
        // telling someone to fill in a form they may not submit is the wrong
        // error.
        let (a, b) = (StatusId::new(), StatusId::new());
        let workflow = Workflow::new(
            vec![
                Status {
                    id: a,
                    name: "A".into(),
                    state: TaskState::Active,
                    is_initial: true,
                },
                Status {
                    id: b,
                    name: "B".into(),
                    state: TaskState::Completed,
                    is_initial: false,
                },
            ],
            vec![Transition {
                id: TransitionId::new(),
                from: Some(a),
                to: b,
                required_permission: Some(perm::TASK_CLOSE),
                required_fields: vec!["resolution".into()],
                ignore_dependencies: false,
            }],
        )
        .expect("valid");

        assert_eq!(
            workflow.validate(a, b, &TransitionRequest::default()),
            Err(Rejection::MissingPermission(perm::TASK_CLOSE))
        );
    }

    #[test]
    fn an_explicit_edge_beats_the_wildcard() {
        // A workflow that narrows one path to Canceled — say, requiring a
        // reason from In Progress — must not silently keep the permissive
        // wildcard for that path.
        let f = default_workflow();
        let transitions: Vec<Transition> = vec![
            Transition {
                id: TransitionId::new(),
                from: None,
                to: f.canceled,
                required_permission: None,
                required_fields: Vec::new(),
                ignore_dependencies: true,
            },
            Transition {
                id: TransitionId::new(),
                from: Some(f.in_progress),
                to: f.canceled,
                required_permission: Some(perm::TASK_CLOSE),
                required_fields: Vec::new(),
                ignore_dependencies: true,
            },
        ];
        let statuses = vec![
            Status {
                id: f.in_progress,
                name: "In Progress".into(),
                state: TaskState::Active,
                is_initial: true,
            },
            Status {
                id: f.canceled,
                name: "Canceled".into(),
                state: TaskState::Canceled,
                is_initial: false,
            },
        ];
        let w = Workflow::new(statuses, transitions).expect("valid");

        assert_eq!(
            w.validate(f.in_progress, f.canceled, &TransitionRequest::default()),
            Err(Rejection::MissingPermission(perm::TASK_CLOSE)),
            "the narrower explicit edge must win over the wildcard"
        );
    }
}
