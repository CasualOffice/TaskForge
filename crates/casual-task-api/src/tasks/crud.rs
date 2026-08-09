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
use casual_task_search::filter::{Clause, Field, Node, Operator, Value};
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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

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
    unit::authorized(
        ctx.authority.may_in_project(
            permission::TASK_CREATE,
            ProjectId::from_uuid(project_row.id),
            project_row.team_id.map(TeamId::from_uuid),
            &ctx.facts_in_project(is_member),
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

/// `GET /api/v1/tasks/{id}` — 200 with an `ETag`, or 404.
///
/// # Errors
///
/// `404` when the task does not exist, is deleted, or sits in a project the
/// caller cannot see. All three are one answer (`docs/04`).
pub async fn read(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let found = task::read_visible(&mut scoped, &ctx.viewer, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    let Some((row, project_key)) = found else {
        return Err(ApiError::missing(codes::TASK_NOT_FOUND, &request_id));
    };
    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(row.version))],
        axum::Json(view(&row, &project_key)),
    )
        .into_response())
}

/// `GET /api/v1/tasks` — every task in the workspace the caller can reach.
///
/// # Errors
///
/// `400` for an unknown query parameter, a bad cursor, or an over-limit page.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    unit::reject_unknown(&params, &["limit", "cursor", "project_id"], &request_id)?;
    let limit = wire::limit(
        params
            .get("limit")
            .map(|raw| {
                raw.parse::<u32>().map_err(|_| {
                    ApiError::bad_request(
                        codes::PAGE_TOO_LARGE,
                        "limit must be a number",
                        &request_id,
                    )
                })
            })
            .transpose()?,
        &request_id,
    )?;
    let after = wire::cursor(params.get("cursor").map(String::as_str), &request_id)?;
    let project_filter = params
        .get("project_id")
        .map(|raw| {
            raw.parse::<Uuid>().map_err(|_| {
                ApiError::bad_request(
                    codes::MALFORMED_BODY,
                    "project_id must be a UUID",
                    &request_id,
                )
            })
        })
        .transpose()?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    // docs/04 §The list problem, step 1: resolved once, for the whole page.
    let accessible = project::accessible(&mut scoped, &ctx.viewer, MAX_ACCESSIBLE_PROJECTS)
        .await
        .map_err(|error| {
            tracing::error!(%error, "resolving the accessible project set failed");
            ApiError::internal(&request_id)
        })?;
    // A `project_id` the caller cannot see narrows the set to nothing rather
    // than returning a 404: it is a filter over a list, and a list filtered to
    // an invisible project is legitimately empty.
    let visible: Vec<ProjectId> = accessible
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| project_filter.is_none_or(|wanted| wanted == *id))
        .map(ProjectId::from_uuid)
        .collect();
    let keys: HashMap<Uuid, String> = accessible.into_iter().collect();

    let filter = project_filter.map_or_else(
        || Node::And(Vec::new()),
        |id| {
            Node::Clause(Clause {
                field: Field::Project,
                op: Operator::Eq,
                value: Value::Literal(id.to_string()),
            })
        },
    );
    let compiled = compile(
        &filter,
        ctx.workspace,
        &AuthorizedProjectSet::resolved(visible),
        &CompilerPage {
            after,
            limit,
            ..CompilerPage::default()
        },
    );
    let mut rows = task::list(&mut scoped, &compiled).await.map_err(|error| {
        tracing::error!(%error, "listing tasks failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    // The default sort is `updated_at DESC` (docs/26), so that is the key the
    // cursor carries. The id tiebreaker is mandatory — without it, ties in
    // updated_at make a page repeat or skip a row.
    let next_cursor = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| Cursor::new(vec![wire::timestamp(row.updated_at)], row.id).encode());

    let data: Vec<TaskView> = rows
        .iter()
        .map(|row| {
            let key = keys.get(&row.project_id).map_or("", String::as_str);
            view(row, key)
        })
        .collect();

    Ok(axum::Json(Paged {
        data,
        page: Page {
            next_cursor,
            has_more,
        },
    })
    .into_response())
}

// ---------------------------------------------------------------------------
// Update, delete
// ---------------------------------------------------------------------------

/// `PATCH /api/v1/tasks/{id}` — update plain fields.
///
/// # Errors
///
/// `400` for a malformed body or an attempt to write `status`, `404` when the
/// task is not visible, `409` against a stale version, `428` without
/// `If-Match`, `403` without `task.update`.
pub async fn update(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Body(body): Body<PatchRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    // Before anything is read: a client that forgot the header has a bug, and
    // the answer does not depend on whether the task exists.
    let expected = etag::if_match(&headers, &request_id)?;

    if body.status_id.is_some() || body.state.is_some() {
        return Err(ApiError::bad_request(
            codes::STATUS_NOT_DIRECTLY_WRITABLE,
            "Status is never written directly — POST to /tasks/{id}/transitions, \
             which is what enforces transition validity, required fields, \
             dependency gating and the transition's own permission",
            &request_id,
        ));
    }

    let title = body
        .title
        .as_deref()
        .map(|t| validated_title(t, &request_id))
        .transpose()?;
    let task_type = body
        .task_type
        .as_deref()
        .map(|v| one_of(Some(v), TASK_TYPES, "TASK", "type", &request_id))
        .transpose()?;
    let priority = body
        .priority
        .as_deref()
        .map(|v| one_of(Some(v), PRIORITIES, "NONE", "priority", &request_id))
        .transpose()?;
    if let Some(Some(description)) = body.description.as_ref()
        && description.len() > 65_536
    {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "description must be at most 65536 bytes",
            &request_id,
        ));
    }
    let start_at = optional_timestamp(body.start_at.as_ref(), "start_at", &request_id)?;
    let due_at = optional_timestamp(body.due_at.as_ref(), "due_at", &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    // docs/23 §Validation order: readable (404), version (409), permission
    // (403). The version check precedes the permission check deliberately — the
    // actor can already see the task, so its version is not a secret, and the
    // stale-client case is overwhelmingly the common one.
    let (current, project_key) = visible(&mut scoped, &ctx, id, &request_id).await?;
    if current.version != expected {
        return Err(conflict(&current, &project_key, expected, &request_id));
    }
    authorize_on_task(
        &mut scoped,
        &ctx,
        &current,
        permission::TASK_UPDATE,
        &request_id,
    )
    .await?;

    let patch = task::TaskPatch {
        title: title.map(ToOwned::to_owned),
        description: body.description.clone(),
        task_type: task_type.map(ToOwned::to_owned),
        priority: priority.map(ToOwned::to_owned),
        start_at,
        due_at,
    };
    let updated = task::update(
        &mut scoped,
        current.id,
        expected,
        &patch,
        ctx.actor.as_uuid(),
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "updating the task failed");
        ApiError::internal(&request_id)
    })?;

    // Zero rows means someone committed between the read above and this
    // statement. docs/24: "0 rows affected ⇒ someone else wrote first ⇒ 409".
    let Some(updated) = updated else {
        let (now, key) = visible(&mut scoped, &ctx, id, &request_id).await?;
        return Err(conflict(&now, &key, expected, &request_id));
    };

    let before = serde_json::json!(view(&current, &project_key));
    let after_view = view(&updated, &project_key);
    let after = serde_json::json!(after_view);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: updated.id,
            project_id: Some(updated.project_id),
            event_type: "task.updated".to_owned(),
            activity_changes: changed_fields(&current, &updated),
            audit_changes: serde_json::json!({ "before": before, "after": after }),
            payload: after.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the task update failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(updated.version))],
        axum::Json(after_view),
    )
        .into_response())
}

/// `DELETE /api/v1/tasks/{id}` — soft delete.
///
/// # Errors
///
/// `404` when the task is not visible, `409` against a stale version, `428`
/// without `If-Match`, `403` without `task.delete`.
pub async fn delete(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let expected = etag::if_match(&headers, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let (current, project_key) = visible(&mut scoped, &ctx, id, &request_id).await?;
    if current.version != expected {
        return Err(conflict(&current, &project_key, expected, &request_id));
    }
    authorize_on_task(
        &mut scoped,
        &ctx,
        &current,
        permission::TASK_DELETE,
        &request_id,
    )
    .await?;

    let deleted = task::soft_delete(&mut scoped, current.id, expected, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "deleting the task failed");
            ApiError::internal(&request_id)
        })?;
    let Some(deleted) = deleted else {
        let (now, key) = visible(&mut scoped, &ctx, id, &request_id).await?;
        return Err(conflict(&now, &key, expected, &request_id));
    };

    let before = serde_json::json!(view(&current, &project_key));
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: deleted.id,
            project_id: Some(deleted.project_id),
            event_type: "task.deleted".to_owned(),
            activity_changes: serde_json::json!({
                "key": format!("{project_key}-{}", deleted.number),
                "title": deleted.title,
            }),
            audit_changes: serde_json::json!({ "before": before, "after": null }),
            payload: serde_json::json!({
                "id": deleted.id,
                "project_id": deleted.project_id,
                "key": format!("{project_key}-{}", deleted.number),
            }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the task delete failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    // 204: the representation is gone, and echoing a tombstone would invite a
    // client to render it.
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------------
