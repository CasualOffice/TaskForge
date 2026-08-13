//! `GET` and `DELETE` on `/api/v1/tasks/{id}/tags`.
//!
//! The `POST` half lives in [`super::relations`] with the other operations that
//! change a task's relationships. These two are here because they arrived with
//! the tag *vocabulary* (`crate::tags`) rather than with C-008, and because
//! `relations.rs` was already at the size where nobody reads a file and everyone
//! greps it (AGENTS.md §Module size and shape).
//!
//! # Reading tags needs no permission of its own
//!
//! A tag is not a resource; it is a property of a task. So the question "may I
//! see this task's tags" is the question "may I see this task", answered once by
//! `visible` and `authorize_on_task` — the same rule `docs/04` gives
//! comments, which "carry no permission of their own".
//!
//! # Removing a tag is `task.update`, not `tag.manage`
//!
//! Untagging changes the **task**, not the vocabulary — the tag survives on
//! every other task that carries it. Requiring `tag.manage` to take a label off
//! one task would mean the people doing the work cannot correct their own
//! labels, which is how tag hygiene dies.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_model::permission;
use casual_task_persistence::{Change, UnitOfWork, tag, task};
use uuid::Uuid;

use super::{authorize_on_task, visible};
use crate::context::Context;
use crate::error::ApiError;
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::tags::TagView;
use crate::unit;

/// `GET /api/v1/tasks/{id}/tags` — the tags this task carries.
///
/// Whole names and colours, not ids. A drawer that received `["<uuid>"]` would
/// have to resolve each one against the vocabulary before it could draw a chip,
/// which is a second request to render a label.
///
/// # Errors
///
/// `404` when the task is not visible, `403` without `task.read`.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let (current, _) = visible(&mut scoped, &ctx, id, &request_id).await?;
    authorize_on_task(
        &mut scoped,
        &ctx,
        &current,
        permission::TASK_READ,
        &request_id,
    )
    .await?;

    let rows = tag::for_task(&mut scoped, current.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task's tags failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    let data: Vec<TagView> = rows.iter().map(TagView::from).collect();
    Ok(axum::Json(serde_json::json!({ "data": data })).into_response())
}

/// `DELETE /api/v1/tasks/{id}/tags/{tag_id}` — take a tag off a task.
///
/// `204` whether or not the task carried it. A client retrying a request whose
/// response it never saw is doing the right thing, and answering `404` the
/// second time would turn a successful retry into an error the user sees.
///
/// No `If-Match`. Tags are not part of the representation an `ETag` describes,
/// so requiring the version of something this does not change would make two
/// people removing two different labels conflict with each other for no reason
/// — the argument `relations::assign` already makes for assignees.
///
/// # Errors
///
/// `404` when the task is not visible, `403` without `task.update`.
pub async fn remove(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path((id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let (current, project_key) = visible(&mut scoped, &ctx, id, &request_id).await?;
    authorize_on_task(
        &mut scoped,
        &ctx,
        &current,
        permission::TASK_UPDATE,
        &request_id,
    )
    .await?;

    // The name is read BEFORE the delete, because `docs/25` wants the activity
    // stream to hold a display value and after the row is gone there is nothing
    // to read one from. `None` means the tag was already absent — the retry
    // case — and writes no event.
    let name = task::usable_tag(&mut scoped, tag_id, current.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the tag failed");
            ApiError::internal(&request_id)
        })?;

    let removed = task::remove_tag(&mut scoped, current.id, tag_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "untagging the task failed");
            ApiError::internal(&request_id)
        })?;

    if removed {
        UnitOfWork::record(
            &mut scoped,
            &Change {
                aggregate_type: "task".to_owned(),
                aggregate_id: current.id,
                project_id: Some(current.project_id),
                event_type: "task.untagged".to_owned(),
                activity_changes: serde_json::json!({
                    "key": format!("{project_key}-{}", current.number),
                    "tag": name,
                }),
                audit_changes: serde_json::json!({
                    "before": { "tag_id": tag_id, "tag": name },
                    "after": null,
                }),
                payload: serde_json::json!({
                    "task_id": current.id,
                    "tag_id": tag_id,
                    "tag": name,
                }),
                schema_version: 1,
            },
            &ctx.provenance,
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, "recording the untagging failed");
            ApiError::internal(&request_id)
        })?;
    }
    unit::commit(tx, &request_id).await?;

    // 204 either way. See the doc comment: the retry must not become an error.
    Ok(StatusCode::NO_CONTENT.into_response())
}
