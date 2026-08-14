//! Create, read, list, update, delete.
//!
//! The lifecycle of a task as a record. Status changes are deliberately NOT
//! here: `docs/23` says status is never written through `PATCH`, and keeping
//! the transition in its own module is what stops someone adding a `status`
//! field to the patch body because it was convenient.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_model::{Cursor, ProjectId, TeamId, permission};
use casual_task_persistence::task::NewTask;
use casual_task_persistence::{
    AuthorizedProjectSet, Change, Page as CompilerPage, UnitOfWork, compile, idempotency, project,
    task,
};
use casual_task_search::filter::{Field, Node, Value};
use casual_task_search::sort::{Direction, Sort, SortField};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::*;
use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::wire::{self, Body, Page, Paged};
use crate::{etag, unit};
use casual_task_persistence::task::TaskRow;
use serde::Deserialize;

/// `POST /api/v1/projects/{id}/tasks`.
///
/// # Errors
///
/// `400` for a malformed body or a missing `Idempotency-Key`, `404` when the
/// project is not visible, `403` without `task.create`, `422` for a parent in
/// another project.
#[allow(clippy::too_many_lines)] // one command, read top to bottom; splitting it hides the order
pub async fn create(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Body(body): Body<CreateRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let title = validated_title(&body.title, &request_id)?;
    let task_type = one_of(
        body.task_type.as_deref(),
        TASK_TYPES,
        "TASK",
        "type",
        &request_id,
    )?;
    let priority = one_of(
        body.priority.as_deref(),
        PRIORITIES,
        "NONE",
        "priority",
        &request_id,
    )?;
    let due_at = body
        .due_at
        .as_deref()
        .map(|raw| {
            OffsetDateTime::parse(raw, &Rfc3339).map_err(|_| {
                ApiError::bad_request(
                    codes::MALFORMED_BODY,
                    "due_at must be an RFC 3339 timestamp",
                    &request_id,
                )
            })
        })
        .transpose()?;
    if let Some(description) = body.description.as_deref()
        && description.len() > 65_536
    {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "description must be at most 65536 bytes",
            &request_id,
        ));
    }
    let idempotency_key = unit::idempotency_key(&headers, &request_id)?;
    let request_hash = unit::hash(&[
        project_id.as_bytes(),
        title.as_bytes(),
        task_type.as_bytes(),
        priority.as_bytes(),
        body.description.as_deref().unwrap_or_default().as_bytes(),
    ]);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // Visibility first: an invisible project is a 404, and a 403 here would
    // tell an outsider it exists (`docs/04`).
    let project_row = project::read_visible(&mut scoped, &ctx.viewer, project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::PROJECT_NOT_FOUND, &request_id))?;

    let is_member = project::is_member(&mut scoped, project_row.id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading project membership failed");
            ApiError::internal(&request_id)
        })?;
    // The *requested* type is the fact this decision turns on. `docs/45` allows
    // "a developer may raise a bug but not a feature", spelled `task_type_in`
    // on the grant — and a constraint evaluated against no type at all is
    // satisfied by nothing, so passing the project-level facts here did not
    // merely lose the rule, it denied every create to anyone holding it.
    unit::authorized(
        ctx.authority.may_in_project(
            permission::TASK_CREATE,
            ProjectId::from_uuid(project_row.id),
            &project_row.teams(),
            &casual_task_app::ResourceFacts {
                task_type: super::guard::task_type_of(task_type),
                ..ctx.facts_in_project(is_member)
            },
        ),
        &request_id,
    )?;

    if let Some(replay) = unit::replay(
        &mut scoped,
        ctx.actor.as_uuid(),
        &idempotency_key,
        &request_hash,
        &request_id,
    )
    .await?
    {
        unit::commit(tx, &request_id).await?;
        return Ok(replay);
    }

    // ADR-018 caps subtask depth at 1, and TF-TSK-0006 requires a parent in the
    // same project. Both are checked against a task the actor can *see*, so a
    // parent id from another project is refused identically whether it exists
    // or not.
    if let Some(parent) = body.parent_id {
        let found = task::read_visible(&mut scoped, &ctx.viewer, parent)
            .await
            .map_err(|error| {
                tracing::error!(%error, "reading the parent task failed");
                ApiError::internal(&request_id)
            })?;
        match found {
            Some((parent_row, _)) if parent_row.project_id != project_row.id => {
                return Err(ApiError::unprocessable(
                    codes::PARENT_OUT_OF_PROJECT,
                    "A parent task must be in the same project",
                    &request_id,
                ));
            }
            Some((parent_row, _)) if parent_row.parent_id.is_some() => {
                return Err(ApiError::unprocessable(
                    codes::PARENT_OUT_OF_PROJECT,
                    "Subtasks are capped at one level (ADR-018)",
                    &request_id,
                ));
            }
            Some(_) => {}
            None => {
                return Err(ApiError::unprocessable(
                    codes::REFERENCE_NOT_FOUND,
                    "parent_id does not name a task in this project",
                    &request_id,
                ));
            }
        }
    }

    let (statuses, transitions) =
        casual_task_persistence::workflow::load(&mut scoped, project_row.workflow_id)
            .await
            .map_err(|error| {
                tracing::error!(%error, "loading the workflow failed");
                ApiError::internal(&request_id)
            })?;
    let workflow = casual_task_app::compose(
        &statuses
            .iter()
            .map(|s| casual_task_app::StoredStatus {
                id: s.id,
                name: s.name.clone(),
                state: s.state.clone(),
                is_initial: s.is_initial,
            })
            .collect::<Vec<_>>(),
        &transitions
            .iter()
            .map(|t| casual_task_app::StoredTransition {
                id: t.id,
                from: t.from,
                to: t.to,
                required_permission: t.required_permission.clone(),
                required_fields: t.required_fields.clone(),
                ignore_dependencies: t.ignore_dependencies,
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|error| {
        // A workflow this build cannot assemble is an operational fault, not a
        // bad request: nothing the caller sent produced it.
        tracing::error!(?error, "the project's workflow could not be assembled");
        ApiError::internal(&request_id)
    })?;
    let (status_id, state) = casual_task_app::initial(&workflow);

    // ADR-008: allocated in-transaction, so a rollback leaks no number. Users
    // read gaps in `WR-1, WR-2, WR-4` as lost data.
    let number = project::allocate_number(&mut scoped, project_row.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "allocating the task number failed");
            ApiError::internal(&request_id)
        })?;

    let new = NewTask {
        id: Uuid::now_v7(),
        project_id: project_row.id,
        number,
        title: title.to_owned(),
        description: body.description.clone(),
        task_type: task_type.to_owned(),
        priority: priority.to_owned(),
        status_id: status_id.as_uuid(),
        state: state_wire(state).to_owned(),
        reporter_id: ctx.actor.as_uuid(),
        parent_id: body.parent_id,
        due_at,
        position: casual_task_app::rank::appended(number),
        created_by: ctx.actor.as_uuid(),
    };
    let row = task::insert(&mut scoped, &new).await.map_err(|error| {
        tracing::error!(%error, "creating the task failed");
        ApiError::internal(&request_id)
    })?;

    let view = view(&row, &project_row.key);
    let payload = serde_json::to_value(&view).unwrap_or(serde_json::Value::Null);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: row.id,
            project_id: Some(row.project_id),
            event_type: "task.created".to_owned(),
            // Display values, not ids (`docs/25`): the status NAME, because the
            // status may be renamed or deleted before anyone reads this.
            activity_changes: serde_json::json!({
                "key": view.key,
                "title": row.title,
                "status": workflow.initial().name,
            }),
            audit_changes: serde_json::json!({ "before": null, "after": payload }),
            payload: payload.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the task create failed");
        ApiError::internal(&request_id)
    })?;

    let body = serde_json::json!(view);
    idempotency::record(
        &mut scoped,
        ctx.actor.as_uuid(),
        &idempotency_key,
        i32::from(StatusCode::CREATED.as_u16()),
        &body,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the idempotency response failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::CREATED,
        [
            (header::ETAG, etag::tag(row.version)),
            (header::LOCATION, format!("/api/v1/tasks/{}", row.id)),
        ],
        axum::Json(body),
    )
        .into_response())
}

include!("crud_reads.rs");
include!("crud_mutations.rs");
