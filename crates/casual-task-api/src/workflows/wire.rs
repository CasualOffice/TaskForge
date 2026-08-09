//! The request and response shapes for `/api/v1/workflows` (`docs/05`).
//!
//! Separated from the handlers because the wire format is the API's contract
//! and the handlers are its implementation. A reviewer checking a field name
//! against the spec should not have to read a transaction to find it.

use casual_task_persistence::workflow::{StatusRow, TransitionRow, WorkflowRow};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct WorkflowView {
    pub id: Uuid,
    pub name: String,
    pub is_default: bool,
    /// The `ETag` every authoring call must send back in `If-Match`. In the
    /// body as well as the header because a settings screen holds one workflow
    /// across several edits and would otherwise have to keep a header alongside
    /// its state.
    pub version: i64,
    /// In `position` order — the order a board draws its columns in.
    pub statuses: Vec<StatusView>,
    pub transitions: Vec<TransitionView>,
}

#[derive(Debug, Serialize)]
pub struct StatusView {
    pub id: Uuid,
    pub name: String,
    /// One of the five permanent states (`docs/23`). The permanent state is
    /// what integrations and reports key on; the name is what people read.
    pub state: String,
    pub position: i32,
    pub is_initial: bool,
}

impl From<StatusRow> for StatusView {
    fn from(row: StatusRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            state: row.state,
            position: row.position,
            is_initial: row.is_initial,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TransitionView {
    /// `null` for the initial transition — `docs/23` models "into the workflow"
    /// as a transition with no source, so a client must handle the absence
    /// rather than treat it as a data error.
    pub id: Uuid,
    pub from: Option<Uuid>,
    pub to: Uuid,
    pub required_permission: Option<String>,
    pub required_fields: Vec<String>,
    pub ignore_dependencies: bool,
}

impl From<TransitionRow> for TransitionView {
    fn from(row: TransitionRow) -> Self {
        Self {
            id: row.id,
            from: row.from,
            to: row.to,
            required_permission: row.required_permission,
            required_fields: row.required_fields,
            ignore_dependencies: row.ignore_dependencies,
        }
    }
}

impl WorkflowView {
    /// Assemble the representation every handler in this module returns.
    ///
    /// Every authoring call returns the **whole** workflow rather than the row
    /// it touched. Deleting a status also deletes edges, promoting an initial
    /// status also demotes one, and a reorder moves everything — so a response
    /// carrying only the edited row would leave the client's copy wrong in a
    /// way it could not detect.
    #[must_use]
    pub fn assemble(
        row: WorkflowRow,
        statuses: Vec<StatusRow>,
        transitions: Vec<TransitionRow>,
    ) -> Self {
        Self {
            id: row.id,
            name: row.name,
            is_default: row.is_default,
            version: row.version,
            statuses: statuses.into_iter().map(StatusView::from).collect(),
            transitions: transitions.into_iter().map(TransitionView::from).collect(),
        }
    }
}

/// One status with how much work is standing on it.
///
/// A separate representation from [`StatusView`], and a separate endpoint,
/// because the count costs an aggregate over `task` — cheap through
/// `task_status_ix`, but not something the board should pay on every load to
/// render a number only the settings screen shows.
#[derive(Debug, Serialize)]
pub struct StatusUsageView {
    pub id: Uuid,
    pub name: String,
    pub state: String,
    pub position: i32,
    pub is_initial: bool,
    /// Includes soft-deleted tasks: they hold the foreign key too, so this is
    /// the number that actually moves when the status is deleted.
    pub task_count: i64,
}

/// `POST /api/v1/workflows/{id}/statuses`.
///
/// `is_initial` and `position` are absent deliberately. Exactly one status is
/// initial (`docs/23`), so a create that could set it would either collide with
/// the existing one or silently demote it; and a new status is appended, which
/// leaves reordering to the one operation that can express a whole ordering.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateStatusRequest {
    pub name: String,
    /// One of the five permanent states. Validated against
    /// `casual_task_model::TaskState`, which is the enum itself rather than a
    /// list beside it.
    pub state: String,
}

/// `PATCH /api/v1/workflows/{id}/statuses/{sid}`.
///
/// `is_initial` accepts only `true`. There is no way to spell "this workflow
/// has no initial status", so `false` would be a request the schema forbids —
/// a partial unique index guarantees exactly one, and the way to move it is to
/// name the new one.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchStatusRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub is_initial: Option<bool>,
}

/// `POST /api/v1/workflows/{id}/statuses/order`.
///
/// The **complete** ordering, not a move. `workflow_status` has no unique
/// constraint on `(workflow_id, position)`, so a partial reorder can leave two
/// statuses sharing one — and then a board's column order depends on which row
/// the planner happens to return first. A permutation cannot express that.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReorderRequest {
    pub order: Vec<Uuid>,
}

/// What a status delete did, alongside the workflow it left behind.
#[derive(Debug, Serialize)]
pub struct StatusDeletedView {
    pub workflow: WorkflowView,
    /// How many tasks were moved onto the migration target, each with its own
    /// activity event attributed to the acting admin (`docs/23`).
    pub migrated_tasks: u64,
    /// Edges that named the deleted status and went with it. Reported rather
    /// than silent: an admin who removed one status and lost four transitions
    /// without being told has been surprised by their own change.
    pub removed_transitions: u64,
}

/// What a state remap recomputed.
#[derive(Debug, Serialize)]
pub struct StatusRemappedView {
    pub workflow: WorkflowView,
    /// `docs/23`: the remap "visibly changes historical reports", so the number
    /// of tasks whose `state` moved is part of the answer and not a detail.
    pub recomputed_tasks: u64,
}

/// `POST /api/v1/workflows/{id}/transitions`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTransitionRequest {
    /// `null` is `docs/23`'s "from any status" — how "Cancel from anywhere" is
    /// expressed without one row per source.
    #[serde(default)]
    pub from: Option<Uuid>,
    pub to: Uuid,
    #[serde(default)]
    pub required_permission: Option<String>,
    #[serde(default)]
    pub required_fields: Vec<String>,
    #[serde(default)]
    pub ignore_dependencies: bool,
}

/// `PATCH /api/v1/workflows/{id}/transitions/{tid}`.
///
/// `from` and `to` are absent: changing either makes it a different edge, and
/// `docs/23` says removing a transition is free — so the honest spelling is a
/// delete and a create, which the unique index then checks.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchTransitionRequest {
    /// `Option<Option<_>>`: absent leaves it alone, `null` clears the
    /// requirement (`docs/05` §Conventions).
    #[serde(default, deserialize_with = "crate::wire::double_option")]
    pub required_permission: Option<Option<String>>,
    #[serde(default)]
    pub required_fields: Option<Vec<String>>,
    #[serde(default)]
    pub ignore_dependencies: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_field_does_not_deserialize() {
        // docs/05: unknown request fields are rejected. Silently ignoring a
        // typo is how a client ships a bug that looks like a server bug.
        assert!(
            serde_json::from_str::<CreateStatusRequest>(
                r#"{"name":"QA","state":"ACTIVE","postion":3}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<PatchStatusRequest>(r#"{"nmae":"QA"}"#).is_err());
        assert!(serde_json::from_str::<ReorderRequest>(r#"{"orders":[]}"#).is_err());
    }

    #[test]
    fn a_transition_patch_distinguishes_an_absent_permission_from_a_null_one() {
        // `{}` leaves the requirement; `{"required_permission": null}` removes
        // it. Collapsing them would make it impossible to un-gate an edge.
        let absent: PatchTransitionRequest = serde_json::from_str("{}").expect("valid");
        assert_eq!(absent.required_permission, None);
        let cleared: PatchTransitionRequest =
            serde_json::from_str(r#"{"required_permission":null}"#).expect("valid");
        assert_eq!(cleared.required_permission, Some(None));
        let set: PatchTransitionRequest =
            serde_json::from_str(r#"{"required_permission":"task.close"}"#).expect("valid");
        assert_eq!(set.required_permission, Some(Some("task.close".to_owned())));
    }

    #[test]
    fn a_transition_create_defaults_to_an_ungated_edge() {
        // The default workflow's ordinary edges carry no permission and no
        // required fields, so a create that omitted them should produce one of
        // those rather than something the board cannot use.
        let created: CreateTransitionRequest =
            serde_json::from_str(r#"{"to":"018f2c00-0000-7000-8000-000000000000"}"#)
                .expect("valid");
        assert_eq!(created.from, None);
        assert_eq!(created.required_permission, None);
        assert!(created.required_fields.is_empty());
        assert!(!created.ignore_dependencies);
    }
}
