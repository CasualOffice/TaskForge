//! The request and response shapes.
//!
//! Separated from the handlers because the wire format is the API's contract
//! and the handlers are its implementation: `docs/05` fixes the first, and the
//! second changes far more often. A reviewer checking a field name against the
//! spec should not have to read a transaction to find it.

use std::collections::HashMap;

use casual_task_persistence::task::TaskRow;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::wire::{self};

/// The task representation. `docs/05`: `snake_case`, RFC 3339 UTC, UUIDv7.
#[derive(Debug, Serialize)]
pub struct TaskView {
    pub id: Uuid,
    /// The human identifier — `WR-125`. Spans `project.key` and `task.number`,
    /// which is why it is composed here and stored nowhere (D-051).
    pub key: String,
    pub project_id: Uuid,
    pub number: i64,
    pub title: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub task_type: String,
    pub priority: String,
    pub status_id: Uuid,
    /// One of the five permanent states. Derived from `status_id` and written
    /// in the same statement, so it can never disagree with it (`docs/23`).
    pub state: String,
    pub reporter_id: Uuid,
    pub environment_id: Option<Uuid>,
    pub milestone_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub start_at: Option<String>,
    pub due_at: Option<String>,
    /// The lexicographic board rank (ADR-013).
    pub position: String,
    pub created_at: String,
    pub created_by: Uuid,
    pub updated_at: String,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<String>,
    pub version: i64,
}

pub(crate) fn view(row: &TaskRow, project_key: &str) -> TaskView {
    TaskView {
        id: row.id,
        key: format!("{project_key}-{}", row.number),
        project_id: row.project_id,
        number: row.number,
        title: row.title.clone(),
        description: row.description.clone(),
        task_type: row.task_type.clone(),
        priority: row.priority.clone(),
        status_id: row.status_id,
        state: row.state.clone(),
        reporter_id: row.reporter_id,
        environment_id: row.environment_id,
        milestone_id: row.milestone_id,
        parent_id: row.parent_id,
        start_at: row.start_at.map(wire::timestamp),
        due_at: row.due_at.map(wire::timestamp),
        position: row.position.clone(),
        created_at: wire::timestamp(row.created_at),
        created_by: row.created_by,
        updated_at: wire::timestamp(row.updated_at),
        updated_by: row.updated_by,
        archived_at: row.archived_at.map(wire::timestamp),
        version: row.version,
    }
}

/// `POST /api/v1/projects/{id}/tasks`.
///
/// `status_id` is deliberately absent — see the module docs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "type")]
    pub task_type: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    /// RFC 3339. Rejected rather than coerced if it is not.
    #[serde(default)]
    pub due_at: Option<String>,
}

/// `PATCH /api/v1/tasks/{id}`.
///
/// `status_id` and `state` are **accepted and then refused** with
/// `TF-WFL-0001`. Leaving them out of the struct would make them unknown fields
/// — a `400` saying "we have never heard of `status_id`", when the truth is
/// that the field exists and has its own door (`docs/23` §The transition
/// command). The same argument `docs/23` makes for why the door exists at all
/// is the reason the error has to say so.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchRequest {
    #[serde(default)]
    pub title: Option<String>,
    /// `Option<Option<_>>`: absent leaves it alone, `null` clears it
    /// (`docs/05` §Conventions).
    #[serde(default, deserialize_with = "wire::double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, rename = "type")]
    pub task_type: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default, deserialize_with = "wire::double_option")]
    pub start_at: Option<Option<String>>,
    #[serde(default, deserialize_with = "wire::double_option")]
    pub due_at: Option<Option<String>>,
    #[serde(default)]
    pub status_id: Option<Uuid>,
    #[serde(default)]
    pub state: Option<String>,
}

/// `POST /api/v1/tasks/{id}/transitions` (`docs/23` §The transition command).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionRequestBody {
    pub to_status_id: Uuid,
    /// Values for the target transition's `required_fields`. Used to satisfy
    /// step 6; **not stored** — see the handler docs.
    #[serde(default)]
    pub fields: HashMap<String, serde_json::Value>,
    /// An optional note, written as a comment in the same transaction
    /// (`docs/23` §What commits).
    #[serde(default)]
    pub comment: Option<String>,
}

/// `POST /api/v1/tasks/{id}/assignees`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignRequest {
    pub user_id: Uuid,
}

/// `POST /api/v1/tasks/{id}/tags`.
///
/// Names an existing tag by id. There is no create-by-name here: authoring the
/// tag vocabulary is `tag.manage` and belongs to a tags endpoint that does not
/// exist yet, and inventing one inside a task write would make every typo a new
/// tag.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagRequest {
    pub tag_id: Uuid,
}

/// `migrations/0001`'s `task_type` enum.
pub(crate) const TASK_TYPES: &[&str] = &["TASK", "BUG", "FEATURE", "INCIDENT", "REQUEST"];
/// `migrations/0001`'s `task_priority` enum, in its declared order.
pub(crate) const PRIORITIES: &[&str] = &["NONE", "LOW", "MEDIUM", "HIGH", "URGENT"];

/// The fields a patch actually changed, as display values (`docs/25`).
///
/// Computed by comparing before and after rather than by echoing the request:
/// a patch that sets a field to the value it already held changed nothing, and
/// an activity stream that says otherwise is noise a reader learns to ignore.
pub(crate) fn changed_fields(before: &TaskRow, after: &TaskRow) -> serde_json::Value {
    let mut changes = serde_json::Map::new();
    let mut note = |name: &str, from: serde_json::Value, to: serde_json::Value| {
        if from != to {
            changes.insert(
                name.to_owned(),
                serde_json::json!({ "from": from, "to": to }),
            );
        }
    };
    note(
        "title",
        serde_json::json!(before.title),
        serde_json::json!(after.title),
    );
    note(
        "description",
        serde_json::json!(before.description),
        serde_json::json!(after.description),
    );
    note(
        "type",
        serde_json::json!(before.task_type),
        serde_json::json!(after.task_type),
    );
    note(
        "priority",
        serde_json::json!(before.priority),
        serde_json::json!(after.priority),
    );
    note(
        "start_at",
        serde_json::json!(before.start_at.map(wire::timestamp)),
        serde_json::json!(after.start_at.map(wire::timestamp)),
    );
    note(
        "due_at",
        serde_json::json!(before.due_at.map(wire::timestamp)),
        serde_json::json!(after.due_at.map(wire::timestamp)),
    );
    serde_json::Value::Object(changes)
}
