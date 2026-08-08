//! Composing a workflow from stored rows (`docs/23`).
//!
//! The state machine is `casual-task-workflow`'s and the rows are
//! `casual-task-persistence`'s. Neither may name the other (`docs/19`), so the
//! assembly happens here.
//!
//! Why go through [`casual_task_workflow::Workflow`] at all, when creating a
//! task only needs one status? Because `Workflow::new` refuses a shape the
//! database would reject — no initial status, two initial statuses, an edge to a
//! status that does not exist — and [`Workflow::initial`] then hands back the
//! status **and** its state together. Reading `WHERE is_initial` directly would
//! be one query shorter and would let a task be created with a `state` that
//! disagrees with its `status_id`, which is the one thing `docs/23` says can
//! never happen.

use casual_task_model::{StatusId, TaskState, TransitionId};
use casual_task_workflow::{Status, Transition, Workflow, WorkflowError};

/// A `workflow_status` row, as the persistence layer reads it.
#[derive(Debug, Clone)]
pub struct StoredStatus {
    pub id: uuid::Uuid,
    pub name: String,
    /// One of the five permanent states.
    pub state: String,
    pub is_initial: bool,
}

/// A `workflow_transition` row, as the persistence layer reads it.
#[derive(Debug, Clone)]
pub struct StoredTransition {
    pub id: uuid::Uuid,
    pub from: Option<uuid::Uuid>,
    pub to: uuid::Uuid,
    pub required_permission: Option<String>,
    pub required_fields: Vec<String>,
    pub ignore_dependencies: bool,
}

/// Why a stored workflow could not be assembled.
///
/// Every variant means "the database holds something this build does not
/// understand". None is recoverable by guessing, and each names the value so an
/// operator can find the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionError {
    /// A status carries a value that is not one of the five states. Only
    /// reachable if the `task_state` enum and this build disagree.
    UnknownState(String),
    /// An edge requires a permission the closed registry does not contain.
    ///
    /// Refused rather than dropped: dropping it makes the edge *easier* to
    /// take, which is the wrong direction for a requirement nobody can meet.
    UnknownPermission(String),
    /// The shape is one the schema would refuse.
    Shape(WorkflowError),
}

/// The stored spelling of a state.
///
/// An exhaustive match, so a sixth state cannot be added without deciding what
/// it is called on disk — the same tripwire `docs/23` puts on the wire format.
const fn wire(state: TaskState) -> &'static str {
    match state {
        TaskState::Backlog => "BACKLOG",
        TaskState::Planned => "PLANNED",
        TaskState::Active => "ACTIVE",
        TaskState::Completed => "COMPLETED",
        TaskState::Canceled => "CANCELED",
    }
}

fn state_of(raw: &str) -> Option<TaskState> {
    TaskState::ALL.into_iter().find(|s| wire(*s) == raw)
}

fn permission_of(key: &str) -> Option<casual_task_model::Permission> {
    casual_task_model::permission::ALL
        .iter()
        .copied()
        .find(|p| p.as_str() == key)
}

/// Assemble the state machine.
///
/// # Errors
///
/// [`CompositionError`] when a state does not parse or the shape is invalid.
pub fn compose(
    statuses: &[StoredStatus],
    transitions: &[StoredTransition],
) -> Result<Workflow, CompositionError> {
    let statuses = statuses
        .iter()
        .map(|s| {
            Ok(Status {
                id: StatusId::from_uuid(s.id),
                name: s.name.clone(),
                state: state_of(&s.state)
                    .ok_or_else(|| CompositionError::UnknownState(s.state.clone()))?,
                is_initial: s.is_initial,
            })
        })
        .collect::<Result<Vec<_>, CompositionError>>()?;

    let transitions = transitions
        .iter()
        .map(|t| {
            let required_permission = match t.required_permission.as_deref() {
                None => None,
                Some(key) => Some(
                    permission_of(key)
                        .ok_or_else(|| CompositionError::UnknownPermission(key.to_owned()))?,
                ),
            };
            Ok(Transition {
                id: TransitionId::from_uuid(t.id),
                from: t.from.map(StatusId::from_uuid),
                to: StatusId::from_uuid(t.to),
                required_permission,
                required_fields: t.required_fields.clone(),
                ignore_dependencies: t.ignore_dependencies,
            })
        })
        .collect::<Result<Vec<_>, CompositionError>>()?;

    Workflow::new(statuses, transitions).map_err(CompositionError::Shape)
}

/// The status a new task starts in, with the state it maps to.
///
/// The pair is returned together because `docs/23` writes them in one
/// statement; splitting them here would put the drift back.
#[must_use]
pub fn initial(workflow: &Workflow) -> (StatusId, TaskState) {
    let status = workflow.initial();
    (status.id, status.state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(name: &str, state: &str, is_initial: bool) -> StoredStatus {
        StoredStatus {
            id: uuid::Uuid::now_v7(),
            name: name.to_owned(),
            state: state.to_owned(),
            is_initial,
        }
    }

    #[test]
    fn the_initial_status_arrives_with_its_state() {
        let rows = vec![
            status("Backlog", "BACKLOG", true),
            status("Done", "COMPLETED", false),
        ];
        let workflow = compose(&rows, &[]).expect("valid");
        let (id, state) = initial(&workflow);
        assert_eq!(id, StatusId::from_uuid(rows[0].id));
        assert_eq!(state, TaskState::Backlog);
    }

    #[test]
    fn every_one_of_the_five_states_parses() {
        // A silent failure here would make a whole workflow unusable, and the
        // error would name a state that looks perfectly correct.
        for (raw, expected) in [
            ("BACKLOG", TaskState::Backlog),
            ("PLANNED", TaskState::Planned),
            ("ACTIVE", TaskState::Active),
            ("COMPLETED", TaskState::Completed),
            ("CANCELED", TaskState::Canceled),
        ] {
            assert_eq!(state_of(raw), Some(expected), "{raw}");
        }
        assert_eq!(state_of("DONE"), None);
    }

    #[test]
    fn a_workflow_without_an_initial_status_is_refused() {
        let rows = vec![status("Backlog", "BACKLOG", false)];
        assert_eq!(
            compose(&rows, &[]).err(),
            Some(CompositionError::Shape(WorkflowError::NoInitialStatus))
        );
    }

    #[test]
    fn an_unknown_state_is_named_rather_than_defaulted() {
        let rows = vec![status("Backlog", "ICEBOX", true)];
        assert_eq!(
            compose(&rows, &[]).err(),
            Some(CompositionError::UnknownState("ICEBOX".to_owned()))
        );
    }
}
