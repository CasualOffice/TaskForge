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

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use casual_task_persistence::workflow;
use serde::Serialize;
use uuid::Uuid;

use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;

#[derive(Debug, Serialize)]
pub struct WorkflowView {
    pub id: Uuid,
    pub name: String,
    pub is_default: bool,
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

#[derive(Debug, Serialize)]
pub struct TransitionView {
    pub id: Uuid,
    /// `null` for the initial transition — `docs/23` models "into the workflow"
    /// as a transition with no source, so a client must handle the absence
    /// rather than treat it as a data error.
    pub from: Option<Uuid>,
    pub to: Uuid,
    pub required_permission: Option<String>,
    pub required_fields: Vec<String>,
    pub ignore_dependencies: bool,
}

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

    let internal = |what: &'static str, error: sqlx::Error| {
        tracing::error!(%error, what, "reading the workflow failed");
        ApiError::internal(&request_id)
    };

    let row = workflow::read(&mut scoped, workflow_id)
        .await
        .map_err(|e| internal("workflow", e))?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, &request_id))?;

    let (statuses, transitions) = workflow::load(&mut scoped, workflow_id)
        .await
        .map_err(|e| internal("statuses and transitions", e))?;

    tx.commit().await.map_err(|error| {
        tracing::error!(%error, "committing the read failed");
        ApiError::internal(&request_id)
    })?;

    Ok(axum::Json(WorkflowView {
        id: row.id,
        name: row.name,
        is_default: row.is_default,
        version: row.version,
        statuses: statuses
            .into_iter()
            .map(|s| StatusView {
                id: s.id,
                name: s.name,
                state: s.state,
                position: s.position,
                is_initial: s.is_initial,
            })
            .collect(),
        transitions: transitions
            .into_iter()
            .map(|t| TransitionView {
                id: t.id,
                from: t.from,
                to: t.to,
                required_permission: t.required_permission,
                required_fields: t.required_fields,
                ignore_dependencies: t.ignore_dependencies,
            })
            .collect(),
    })
    .into_response())
}
