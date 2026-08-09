//! `GET /api/v1/tasks/{id}/subtasks` — a parent's children, and the rollup.
//!
//! # The failure this module prevents
//!
//! A rollup that becomes a rule. `docs/03` §Subtasks is unambiguous: "Parent
//! status is **never** auto-derived from children. Rollup is displayed
//! (`3/5 done`), never enforced — implicit status changes are the most confusing
//! behaviour in every tracker that does it."
//!
//! This module is a **read**. It has no write path, takes no verb but `GET`, and
//! the two numbers it returns are counted from the children rather than stored
//! anywhere. There is nothing here that could complete a parent, so nothing has
//! to remember not to.
//!
//! # Why an endpoint at all, when `?parent=<id>` already filters
//!
//! `docs/27` has had `parent eq` since C-012, and a client could list children
//! with it. Two things it cannot do:
//!
//! - **The rollup is not the page.** `3/5` must count every visible child, and a
//!   filtered list returns a page. A client that counted its page would report
//!   `3/3` on a parent with twelve children and be confidently wrong.
//! - **`is_blocked` and the ordering** would each be a second decision made per
//!   client. Children are returned in board order here, once.
//!
//! # Depth is capped at 1, so this is a list and not a tree
//!
//! ADR-018. A child cannot have children, which is why the response is flat,
//! why there is no recursion below, and why
//! `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §12 forbids an expand chevron:
//! there is never a second level to expand into.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use casual_task_persistence::task;
use serde::Serialize;
use uuid::Uuid;

use super::wire::{TaskView, view};
use super::{authorize_on_task, visible};
use crate::context::Context;
use crate::error::ApiError;
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;

/// A parent's children, with the counts a `SubtaskList` renders above them.
#[derive(Debug, Serialize)]
pub struct Subtasks {
    pub data: Vec<TaskView>,
    /// Children in a `COMPLETED` state, over children the caller may see.
    ///
    /// **Displayed, never enforced.** Nothing in the product may offer to close
    /// the parent when `done == total`, and no endpoint accepts a request to.
    pub done: i64,
    pub total: i64,
    /// True when there are more children than one read returns. The rollup is
    /// still whole — it is counted in the database, not from `data`.
    pub truncated: bool,
}

/// A subtask's one line of context: the parent it belongs to.
///
/// `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §12: "A subtask shows its
/// parent as a single line of context near the identifier, not as a breadcrumb
/// trail — the trail can only ever be one level deep." Which is why this is a
/// key and a title and not a path.
#[derive(Debug, Serialize)]
pub struct ParentRef {
    pub id: Uuid,
    pub key: String,
    pub title: String,
    pub state: String,
}

/// The subtask payload for one task, whichever end of the relationship it is.
#[derive(Debug, Serialize)]
pub struct Relationship {
    /// `null` when this task is not a subtask, which is most tasks.
    pub parent: Option<ParentRef>,
    #[serde(flatten)]
    pub children: Subtasks,
}

/// `GET /api/v1/tasks/{id}/subtasks`.
///
/// Answers for both ends in one call, because the drawer needs both and they
/// come from the same visibility resolution: a parent renders its `SubtaskList`,
/// a child renders its parent line, and neither client has to know in advance
/// which it is looking at.
///
/// A child whose project the caller cannot see is **absent**, not redacted —
/// and the rollup is counted over the same visible set, so the numbers and the
/// list agree. `docs/04`: nothing may leak the existence of invisible work, and
/// `4/9` beside four rows announces five rows the reader may not see.
///
/// # Errors
///
/// `404` when the task is not visible, `403` without `task.read` on it.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let (current, _) = visible(&mut scoped, &ctx, id, &request_id).await?;
    authorize_on_task(
        &mut scoped,
        &ctx,
        &current,
        casual_task_model::permission::TASK_READ,
        &request_id,
    )
    .await?;

    let rows = task::children(&mut scoped, &ctx.viewer, current.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading subtasks failed");
            ApiError::internal(&request_id)
        })?;
    let (done, total) = task::child_rollup(&mut scoped, &ctx.viewer, current.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "counting the subtask rollup failed");
            ApiError::internal(&request_id)
        })?;

    // The parent, when this task has one. Read through `visible` like anything
    // else: a parent in a project the caller lost access to is `null` here
    // rather than a row with a title in it.
    let parent = match current.parent_id {
        None => None,
        Some(parent_id) => visible(&mut scoped, &ctx, parent_id, &request_id)
            .await
            .ok()
            .map(|(row, project_key)| ParentRef {
                id: row.id,
                key: format!("{project_key}-{}", row.number),
                title: row.title.clone(),
                state: row.state.clone(),
            }),
    };
    unit::commit(tx, &request_id).await?;

    let truncated = i64::try_from(rows.len()).unwrap_or(i64::MAX) < total;
    Ok(axum::Json(Relationship {
        parent,
        children: Subtasks {
            data: rows
                .iter()
                .map(|(row, project_key)| view(row, project_key))
                .collect(),
            done,
            total,
            truncated,
        },
    })
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_response_carries_no_way_to_complete_a_parent() {
        // `docs/03`: the rollup is displayed, never enforced. The serialized
        // shape is the contract a client builds against, so the assertion is
        // that it offers no affordance — a `can_complete` or `all_done` flag
        // would be an invitation to build the behaviour the document forbids.
        let json = serde_json::to_value(Subtasks {
            data: Vec::new(),
            done: 5,
            total: 5,
            truncated: false,
        })
        .expect("serializes");
        let object = json.as_object().expect("an object");
        assert_eq!(object.len(), 4, "unexpected field: {object:?}");
        for absent in ["can_complete", "all_done", "complete", "parent_state"] {
            assert!(!object.contains_key(absent), "{absent} must not be offered");
        }
        // Equal counts are just equal counts.
        assert_eq!(object["done"], object["total"]);
    }

    #[test]
    fn a_parent_reference_is_one_line_and_not_a_trail() {
        // design/LAYOUT §12: "not as a breadcrumb trail — the trail can only
        // ever be one level deep". A `ParentRef` that carried its own parent,
        // or a path, would let a client render the tree ADR-018 refuses.
        let json = serde_json::to_value(ParentRef {
            id: Uuid::now_v7(),
            key: "WR-1".to_owned(),
            title: "Ship it".to_owned(),
            state: "ACTIVE".to_owned(),
        })
        .expect("serializes");
        let object = json.as_object().expect("an object");
        assert_eq!(object.len(), 4);
        for absent in ["parent", "path", "ancestors", "children"] {
            assert!(!object.contains_key(absent), "{absent} must not be offered");
        }
    }
}
