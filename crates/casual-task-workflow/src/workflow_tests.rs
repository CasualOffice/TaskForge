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
