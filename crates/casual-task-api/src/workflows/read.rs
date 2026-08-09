//! `GET /api/v1/workflows/{id}` (C-007, `docs/05`).
//!
//! # The failure this prevents
//!
//! A board that cannot move a card. `POST /tasks/{id}/transitions` takes a
//! `to_status_id`, and nothing else the API returns names one: a task carries
//! its *current* status and the permanent state derived from it, never the set
//! of statuses it could move to. Without this route a browser can render a
//! board and not drag on it, which is what the web client shipped with.
//!
//! # Why membership is the whole authorization rule
//!
//! A workflow is workspace configuration — status names and the arrows between
//! them — not tenant content. `docs/04` gates *doing* a transition
//! (`required_permission` on the transition, checked where the transition is
//! performed), and gating the ability to *read* the shape as well would leave a
//! member able to make a move they were never shown. Row-level security
//! confines the read to the caller's workspace, so a workflow belonging to
//! another tenant reads as absent rather than as forbidden.
//!
//! The per-transition `required_permission` is returned as stored. The client
//! uses it with `GET /permissions/effective` to grey out the arrows the actor
//! cannot take — which is a better refusal than a 403 after the drop, and is
//! why the field is exposed rather than filtered out here.
//!
//! # Why this read carries an `ETag` and the authoring calls demand it
//!
//! Statuses and transitions have no version of their own; the workflow is the
//! aggregate (see [`crate::workflows`]). This is the read a settings screen
//! holds while it makes several edits, so it is the read that has to hand back
//! the tag those edits send in `If-Match`.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_persistence::workflow;
use uuid::Uuid;

use crate::error::ApiError;
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::workflows::guard;
use crate::workflows::wire::WorkflowView;
use crate::{etag, unit};

/// `GET /api/v1/workflows/{id}`.
///
/// # Errors
///
/// `404` if the workflow does not exist or belongs to another workspace —
/// never disambiguated. `500` on a database failure.
pub async fn read(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(workflow_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;

    let row = guard::visible(&mut scoped, workflow_id, &request_id).await?;
    let version = row.version;
    let (statuses, transitions) =
        workflow::load(&mut scoped, workflow_id)
            .await
            .map_err(|error| {
                tracing::error!(%error, "reading the statuses and transitions failed");
                ApiError::internal(&request_id)
            })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(version))],
        axum::Json(WorkflowView::assemble(row, statuses, transitions)),
    )
        .into_response())
}
