//! `/api/v1/tasks` and `/api/v1/projects/{id}/tasks` (C-008, read and create).
//!
//! # The list is the compiler's query, not this module's
//!
//! `docs/04` §The list problem: resolve the accessible project set once, then
//! filter with `project_id = ANY($accessible)`. That filter is injected by
//! `casual_task_persistence::compile`, whose signature *requires* an
//! `AuthorizedProjectSet` — so a list that forgot the permission filter does
//! not compile. This module supplies the set and never writes the predicate.
//!
//! # `status` is not a field a create chooses
//!
//! `docs/23`: "Status is **never** written through `PATCH /tasks/{id}`", and by
//! the same argument it is not written through a create either. A new task
//! enters the workflow's initial status, and the `state` it maps to is written
//! in the same statement — `casual_task_app::initial` hands back both together
//! so there is no way to write one of them.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_model::{Cursor, ProjectId, TeamId, permission};
use casual_task_persistence::task::{NewTask, TaskRow};
use casual_task_persistence::{
    AuthorizedProjectSet, Change, Page as CompilerPage, UnitOfWork, compile, idempotency, project,
    task,
};
use casual_task_search::filter::{Clause, Field, Node, Operator, Value};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::wire::{self, Body, Page, Paged};
use crate::{etag, unit};

/// The accessible project set is bounded.
///
/// `docs/26` §Permission filtering: past a few hundred projects for one actor
/// "the array stops being efficient" and the set is materialized into a
/// `project_access` table instead. That table does not exist yet, so the bound
/// is enforced here — a truncated set produces a visibly missing project rather
/// than a query that quietly degrades.
const MAX_ACCESSIBLE_PROJECTS: u32 = 500;

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

fn view(row: &TaskRow, project_key: &str) -> TaskView {
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

/// `migrations/0001`'s `task_type` enum.
const TASK_TYPES: &[&str] = &["TASK", "BUG", "FEATURE", "INCIDENT", "REQUEST"];
/// `migrations/0001`'s `task_priority` enum, in its declared order.
const PRIORITIES: &[&str] = &["NONE", "LOW", "MEDIUM", "HIGH", "URGENT"];

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

/// `POST /api/v1/tasks/{id}/transitions` — the only door to a status change.
///
/// # The validation order is `docs/23`'s, and it is observable
///
/// Steps 1–3 are here (readable → `404`, version → `409`, `task.transition` →
/// `403`); steps 4–7 are `casual_task_app::Workflow::validate`, which
/// returns the **first** failure as a `Rejection` and is where the order between
/// them is enforced. This handler does not re-derive any of those rules — it
/// supplies the facts and maps the refusal onto its documented code.
///
/// **Step 8 — plugin `validation.transition` hooks — is not implemented.** It
/// needs the plugin runtime (Phase 3, `docs/34`), and nothing here fakes it.
///
/// # What `fields` does and does not do
///
/// It satisfies step 6, and its values are then discarded. Storing them needs
/// custom-field value storage, which is **D-033** and deliberately deferred
/// until Phase 3. A transition whose workflow requires a field therefore
/// validates correctly and records nothing; the default workflow requires no
/// fields, so no path in the product reaches that gap today.
///
/// # Errors
///
/// `404`, `409`, `428`, `403`, or one of `TF-WFL-0002`..`TF-WFL-0005`.
#[allow(clippy::too_many_lines)] // one command, read top to bottom; the ORDER is the specification
pub async fn transition(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Body(body): Body<TransitionRequestBody>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let expected = etag::if_match(&headers, &request_id)?;
    if let Some(comment) = body.comment.as_deref()
        && comment.len() > 65_536
    {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "comment must be at most 65536 bytes",
            &request_id,
        ));
    }

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    // 1. Readable.
    let (current, project_key) = visible(&mut scoped, &ctx, id, &request_id).await?;
    // 2. Version.
    if current.version != expected {
        return Err(conflict(&current, &project_key, expected, &request_id));
    }
    // 3. task.transition on the project.
    let facts = facts_for(&mut scoped, &ctx, &current, &request_id).await?;
    let project_row = project::read_visible(&mut scoped, &ctx.viewer, current.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, &request_id))?;
    let team = project_row.team_id.map(TeamId::from_uuid);
    let project = ProjectId::from_uuid(current.project_id);
    unit::authorized(
        ctx.authority
            .may_in_project(permission::TASK_TRANSITION, project, team, &facts),
        &request_id,
    )?;

    // A move to the status the task already occupies is a no-op that returns
    // 200 and writes nothing — `docs/23` §Concurrency: "this makes client
    // retries safe without an idempotency key". Answered before the workflow is
    // loaded, so a retry costs nothing.
    if body.to_status_id == current.status_id {
        unit::commit(tx, &request_id).await?;
        return Ok((
            StatusCode::OK,
            [(header::ETAG, etag::tag(current.version))],
            axum::Json(view(&current, &project_key)),
        )
            .into_response());
    }

    let workflow = load_workflow(&mut scoped, project_row.workflow_id, &request_id).await?;

    // Steps 4–7, in `casual-task-workflow`. Everything it needs is resolved
    // here and passed in; it reaches nothing itself, which is what lets the
    // whole state machine be tested with no database.
    let held: Vec<casual_task_model::Permission> = permission::ALL
        .iter()
        .copied()
        .filter(|p| {
            ctx.authority
                .may_in_project(*p, project, team, &facts)
                .is_allowed()
        })
        .collect();
    let may_override = held.contains(&permission::TASK_DEPENDENCY_OVERRIDE);
    let blockers = task::unresolved_blockers(&mut scoped, &ctx.viewer, current.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading blocking dependencies failed");
            ApiError::internal(&request_id)
        })?;

    let request = casual_task_app::TransitionRequest {
        // A field present but empty does not satisfy a requirement: docs/23
        // step 6 says "present and non-empty".
        provided_fields: body
            .fields
            .iter()
            .filter(|(_, value)| !is_empty_value(value))
            .map(|(name, _)| name.clone())
            .collect(),
        unresolved_blockers: blockers
            .iter()
            .map(|b| casual_task_model::TaskId::from_uuid(*b))
            .collect(),
        may_override_dependencies: may_override,
        held_permissions: held,
    };

    let valid = workflow
        .validate(
            casual_task_model::StatusId::from_uuid(current.status_id),
            casual_task_model::StatusId::from_uuid(body.to_status_id),
            &request,
        )
        .map_err(|rejection| rejected(&rejection, &request_id))?;

    let from_status = workflow
        .status(casual_task_model::StatusId::from_uuid(current.status_id))
        .map_or("", |s| s.name.as_str())
        .to_owned();
    let to_status = workflow
        .status(valid.to_status)
        .map_or("", |s| s.name.as_str())
        .to_owned();

    let moved = task::transition(
        &mut scoped,
        current.id,
        expected,
        valid.to_status.as_uuid(),
        state_wire(valid.to_state),
        ctx.actor.as_uuid(),
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "the transition failed");
        ApiError::internal(&request_id)
    })?;
    let Some(moved) = moved else {
        let (now, key) = visible(&mut scoped, &ctx, id, &request_id).await?;
        return Err(conflict(&now, &key, expected, &request_id));
    };

    if let Some(comment) = body
        .comment
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        task::insert_comment(&mut scoped, moved.id, ctx.actor.as_uuid(), comment)
            .await
            .map_err(|error| {
                tracing::error!(%error, "writing the transition comment failed");
                ApiError::internal(&request_id)
            })?;
    }

    // docs/23 §Closing and reopening: leaving a terminal state writes a
    // DISTINCT event, "because 'how often does work come back?' is a question
    // teams need answered and a generic status-change event cannot serve".
    let was_terminal = matches!(current.state.as_str(), "COMPLETED" | "CANCELED");
    let is_terminal = matches!(state_wire(valid.to_state), "COMPLETED" | "CANCELED");
    let event_type = if was_terminal && !is_terminal {
        "task.reopened"
    } else {
        "task.status.changed"
    };

    let after_view = view(&moved, &project_key);
    let payload = serde_json::json!(after_view);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: moved.id,
            project_id: Some(moved.project_id),
            event_type: event_type.to_owned(),
            // Display values, not ids (`docs/25`): the status NAMES, because
            // either may be renamed or deleted before anyone reads this.
            activity_changes: serde_json::json!({
                "status": { "from": from_status, "to": to_status },
            }),
            audit_changes: serde_json::json!({
                "before": { "status_id": current.status_id, "state": current.state },
                "after":  { "status_id": moved.status_id,   "state": moved.state },
            }),
            payload: payload.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the transition failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(moved.version))],
        axum::Json(after_view),
    )
        .into_response())
}

/// Map a state-machine refusal onto its documented code (`docs/20`).
///
/// `MissingPermission` is a **403**, not a 422: the caller may not perform this
/// transition, which is a different answer from "this transition is not
/// possible".
fn rejected(rejection: &casual_task_app::Rejection, request_id: &str) -> ApiError {
    use casual_task_app::Rejection;
    match rejection {
        Rejection::NoSuchTransition => ApiError::unprocessable(
            codes::NO_SUCH_TRANSITION,
            "That status is not reachable from the task's current status",
            request_id,
        ),
        Rejection::MissingPermission(permission) => {
            ApiError::denied(codes::TRANSITION_PERMISSION, request_id)
                .with_details(serde_json::json!({ "required_permission": permission.as_str() }))
        }
        Rejection::MissingFields(fields) => ApiError::unprocessable(
            codes::TRANSITION_FIELDS_MISSING,
            "Required fields are missing for that status",
            request_id,
        )
        // docs/05: `details` returns ALL violations at once, never the first.
        .with_details(serde_json::json!({ "missing_fields": fields })),
        Rejection::BlockedBy(blockers) => ApiError::unprocessable(
            codes::BLOCKED_BY_DEPENDENCIES,
            "This task is blocked by unresolved dependencies",
            request_id,
        )
        .with_details(serde_json::json!({
            "blocked_by": blockers.iter().map(|b| b.as_uuid()).collect::<Vec<_>>(),
        })),
    }
}

/// Whether a supplied field value counts as absent for step 6.
///
/// `docs/23`: required fields must be "present and non-empty". A `null`, an
/// empty string, or an empty list is a field the user did not fill in, and
/// accepting one would make a required field a formality.
fn is_empty_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        serde_json::Value::Array(a) => a.is_empty(),
        serde_json::Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Assignees and tags
// ---------------------------------------------------------------------------

/// `POST /api/v1/tasks/{id}/assignees`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignRequest {
    pub user_id: Uuid,
}

/// `POST /api/v1/tasks/{id}/assignees` — assign someone.
///
/// Idempotent: assigning someone already assigned is `200`, not an error. A
/// client retrying a request whose response it never saw is doing the right
/// thing.
///
/// No `If-Match`. Assignees are not part of the task representation an `ETag`
/// describes, so requiring the version of something this does not change would
/// be ceremony — and would make two people assigning different colleagues
/// conflict with each other for no reason.
///
/// # Errors
///
/// `404` when the task is not visible, `403` without `task.assign`, `422`
/// when the assignee cannot see the project.
pub async fn assign(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Body(body): Body<AssignRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let (current, project_key) = visible(&mut scoped, &ctx, id, &request_id).await?;
    authorize_on_task(
        &mut scoped,
        &ctx,
        &current,
        permission::TASK_ASSIGN,
        &request_id,
    )
    .await?;

    if !task::may_be_assigned(&mut scoped, body.user_id, current.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "checking the assignee failed");
            ApiError::internal(&request_id)
        })?
    {
        // 422 and not 404: the *task* is visible, so this is a domain-rule
        // violation about the body, not a hidden resource. It is also one
        // answer for "no such user", "another tenant's user", and "cannot see
        // this project" — none of which this caller is entitled to tell apart.
        return Err(ApiError::unprocessable(
            codes::ASSIGNEE_NOT_PROJECT_MEMBER,
            "That user cannot be assigned work in this project",
            &request_id,
        ));
    }

    let added = task::add_assignee(&mut scoped, current.id, body.user_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "assigning the task failed");
            ApiError::internal(&request_id)
        })?;

    if added {
        UnitOfWork::record(
            &mut scoped,
            &Change {
                aggregate_type: "task".to_owned(),
                aggregate_id: current.id,
                project_id: Some(current.project_id),
                event_type: "task.assigned".to_owned(),
                activity_changes: serde_json::json!({
                    "key": format!("{project_key}-{}", current.number),
                    "assignee_id": body.user_id,
                }),
                audit_changes: serde_json::json!({
                    "before": null,
                    "after": { "assignee_id": body.user_id },
                }),
                payload: serde_json::json!({
                    "task_id": current.id,
                    "user_id": body.user_id,
                }),
                schema_version: 1,
            },
            &ctx.provenance,
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, "recording the assignment failed");
            ApiError::internal(&request_id)
        })?;
    }

    let assignees = task::assignees(&mut scoped, current.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading assignees failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    let status = if added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        axum::Json(serde_json::json!({ "assignees": assignees })),
    )
        .into_response())
}

/// `DELETE /api/v1/tasks/{id}/assignees/{user_id}` — unassign.
///
/// # Errors
///
/// `404` when the task is not visible or the user is not assigned, `403`
/// without `task.assign`.
pub async fn unassign(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let (current, project_key) = visible(&mut scoped, &ctx, id, &request_id).await?;
    authorize_on_task(
        &mut scoped,
        &ctx,
        &current,
        permission::TASK_ASSIGN,
        &request_id,
    )
    .await?;

    if !task::remove_assignee(&mut scoped, current.id, user_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "unassigning the task failed");
            ApiError::internal(&request_id)
        })?
    {
        return Err(ApiError::missing(codes::TASK_NOT_FOUND, &request_id));
    }

    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: current.id,
            project_id: Some(current.project_id),
            event_type: "task.unassigned".to_owned(),
            activity_changes: serde_json::json!({
                "key": format!("{project_key}-{}", current.number),
                "assignee_id": user_id,
            }),
            audit_changes: serde_json::json!({
                "before": { "assignee_id": user_id },
                "after": null,
            }),
            payload: serde_json::json!({ "task_id": current.id, "user_id": user_id }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the unassignment failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok(StatusCode::NO_CONTENT.into_response())
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

/// `POST /api/v1/tasks/{id}/tags` — tag a task.
///
/// # Errors
///
/// `404` when the task is not visible, `403` without `task.update`, `422` when
/// the tag does not exist or does not apply to this project.
pub async fn tag(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Body(body): Body<TagRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let (current, project_key) = visible(&mut scoped, &ctx, id, &request_id).await?;
    // Applying an existing tag changes the TASK, not the tag vocabulary, so the
    // permission is task.update rather than tag.manage.
    authorize_on_task(
        &mut scoped,
        &ctx,
        &current,
        permission::TASK_UPDATE,
        &request_id,
    )
    .await?;

    let Some(name) = task::usable_tag(&mut scoped, body.tag_id, current.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the tag failed");
            ApiError::internal(&request_id)
        })?
    else {
        return Err(ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "That tag does not exist in this workspace, or does not apply to \
             this project",
            &request_id,
        ));
    };

    let added = task::add_tag(&mut scoped, current.id, body.tag_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "tagging the task failed");
            ApiError::internal(&request_id)
        })?;

    if added {
        UnitOfWork::record(
            &mut scoped,
            &Change {
                aggregate_type: "task".to_owned(),
                aggregate_id: current.id,
                project_id: Some(current.project_id),
                event_type: "task.tagged".to_owned(),
                // The tag NAME, not its id: docs/25 wants a stream that still
                // reads correctly after the tag is renamed or deleted.
                activity_changes: serde_json::json!({
                    "key": format!("{project_key}-{}", current.number),
                    "tag": name,
                }),
                audit_changes: serde_json::json!({
                    "before": null,
                    "after": { "tag_id": body.tag_id, "tag": name },
                }),
                payload: serde_json::json!({
                    "task_id": current.id,
                    "tag_id": body.tag_id,
                    "tag": name,
                }),
                schema_version: 1,
            },
            &ctx.provenance,
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, "recording the tagging failed");
            ApiError::internal(&request_id)
        })?;
    }
    unit::commit(tx, &request_id).await?;

    let status = if added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        axum::Json(serde_json::json!({ "tag_id": body.tag_id, "name": name })),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// Shared
// ---------------------------------------------------------------------------

/// Read a task the caller may see, or refuse with `404`.
///
/// Absent, deleted, and in-a-project-you-cannot-see are one answer (`docs/04`).
async fn visible(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    id: Uuid,
    request_id: &str,
) -> Result<(TaskRow, String), ApiError> {
    task::read_visible(scoped, &ctx.viewer, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, request_id))
}

/// The constraint inputs for a decision about **this task**.
///
/// `Context::facts_in_project` is the project-level form and leaves the
/// task-level facts empty. `assignee_is_actor` and `reporter_is_actor` are the
/// two constraints in the closed set that need them, and a decision made
/// without them silently denies someone their own task.
async fn facts_for(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    row: &TaskRow,
    request_id: &str,
) -> Result<casual_task_app::ResourceFacts, ApiError> {
    let internal = |error: sqlx::Error, what: &'static str| {
        tracing::error!(%error, what, "gathering task facts failed");
        ApiError::internal(request_id)
    };
    let is_member = project::is_member(scoped, row.project_id, ctx.actor.as_uuid())
        .await
        .map_err(|e| internal(e, "membership"))?;
    let assignees = task::assignees(scoped, row.id)
        .await
        .map_err(|e| internal(e, "assignees"))?;

    Ok(casual_task_app::ResourceFacts {
        assignees: assignees
            .into_iter()
            .map(casual_task_model::UserId::from_uuid)
            .collect(),
        reporter: Some(casual_task_model::UserId::from_uuid(row.reporter_id)),
        actor_is_project_member: is_member,
        environment: row
            .environment_id
            .map(casual_task_model::EnvironmentId::from_uuid),
        actor_is_guest: ctx.is_guest,
    })
}

/// Resolve the task's facts and answer one permission question about it.
async fn authorize_on_task(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    row: &TaskRow,
    wanted: casual_task_model::Permission,
    request_id: &str,
) -> Result<(), ApiError> {
    let facts = facts_for(scoped, ctx, row, request_id).await?;
    let team = project::read_visible(scoped, &ctx.viewer, row.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(request_id)
        })?
        .and_then(|p| p.team_id)
        .map(TeamId::from_uuid);
    unit::authorized(
        ctx.authority
            .may_in_project(wanted, ProjectId::from_uuid(row.project_id), team, &facts),
        request_id,
    )
}

/// Assemble the project's workflow, or report an operational fault.
async fn load_workflow(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    workflow_id: Uuid,
    request_id: &str,
) -> Result<casual_task_app::Workflow, ApiError> {
    let (statuses, transitions) = casual_task_persistence::workflow::load(scoped, workflow_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "loading the workflow failed");
            ApiError::internal(request_id)
        })?;
    casual_task_app::compose(
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
        ApiError::internal(request_id)
    })
}

/// The `409` body. `docs/24`: the loser is told what it lost to, so the client
/// can show "Sarah changed status and assignee" and offer a merge.
fn conflict(current: &TaskRow, project_key: &str, your_version: i64, request_id: &str) -> ApiError {
    ApiError::conflict(
        codes::VERSION_CONFLICT,
        "This task was updated by someone else",
        request_id,
    )
    .with_details(serde_json::json!({
        "your_version": your_version,
        "current_version": current.version,
        "changed_by": current.updated_by,
        "changed_at": wire::timestamp(current.updated_at),
        "current": view(current, project_key),
    }))
}

/// The fields a patch actually changed, as display values (`docs/25`).
///
/// Computed by comparing before and after rather than by echoing the request:
/// a patch that sets a field to the value it already held changed nothing, and
/// an activity stream that says otherwise is noise a reader learns to ignore.
fn changed_fields(before: &TaskRow, after: &TaskRow) -> serde_json::Value {
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

/// Parse an optional, nullable RFC 3339 timestamp from a patch.
fn optional_timestamp(
    value: Option<&Option<String>>,
    field: &str,
    request_id: &str,
) -> Result<Option<Option<OffsetDateTime>>, ApiError> {
    match value {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(raw)) => OffsetDateTime::parse(raw, &Rfc3339)
            .map(|at| Some(Some(at)))
            .map_err(|_| {
                ApiError::bad_request(
                    codes::MALFORMED_BODY,
                    format!("{field} must be an RFC 3339 timestamp"),
                    request_id,
                )
            }),
    }
}

/// The stored spelling of a state.
///
/// Exhaustive, so a sixth state cannot appear without deciding what it is
/// called on disk.
const fn state_wire(state: casual_task_model::TaskState) -> &'static str {
    use casual_task_model::TaskState;
    match state {
        TaskState::Backlog => "BACKLOG",
        TaskState::Planned => "PLANNED",
        TaskState::Active => "ACTIVE",
        TaskState::Completed => "COMPLETED",
        TaskState::Canceled => "CANCELED",
    }
}

fn validated_title<'a>(title: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = title.trim();
    // migrations/0005: CHECK (length(title) BETWEEN 1 AND 512). Checked here so
    // the caller gets a described bound rather than a 500 from a constraint.
    if trimmed.is_empty() || trimmed.chars().count() > 512 {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "title must be between 1 and 512 characters",
            request_id,
        ));
    }
    Ok(trimmed)
}

fn one_of<'a>(
    value: Option<&'a str>,
    allowed: &[&'static str],
    default: &'a str,
    field: &str,
    request_id: &str,
) -> Result<&'a str, ApiError> {
    let Some(value) = value else {
        return Ok(default);
    };
    if allowed.contains(&value) {
        Ok(value)
    } else {
        Err(ApiError::bad_request(
            codes::INVALID_ENUM,
            format!("{field} is not one of the permitted values"),
            request_id,
        )
        .with_details(serde_json::json!({ "field": field, "allowed": allowed })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_enums_match_the_ones_the_schema_declares() {
        // A value the API accepts and the enum does not is a 500 from a cast.
        let migration = include_str!("../../../migrations/0001_extensions_and_types.sql");
        for value in TASK_TYPES.iter().chain(PRIORITIES.iter()) {
            assert!(
                migration.contains(&format!("'{value}'")),
                "{value} is not declared in migration 0001"
            );
        }
        assert_eq!(TASK_TYPES.len(), 5);
        assert_eq!(PRIORITIES.len(), 5);
    }

    #[test]
    fn every_state_has_a_stored_spelling_the_schema_knows() {
        let migration = include_str!("../../../migrations/0001_extensions_and_types.sql");
        for state in casual_task_model::TaskState::ALL {
            assert!(
                migration.contains(&format!("'{}'", state_wire(state))),
                "{state:?} maps to a value task_state does not declare"
            );
        }
    }

    #[test]
    fn a_title_is_bounded_at_the_schemas_bound() {
        assert!(validated_title("x", "r").is_ok());
        assert!(validated_title(&"x".repeat(512), "r").is_ok());
        assert!(validated_title(&"x".repeat(513), "r").is_err());
        assert!(validated_title("   ", "r").is_err());
        let migration = include_str!("../../../migrations/0005_tasks.sql");
        assert!(
            migration.contains("length(title) BETWEEN 1 AND 512"),
            "the schema's title bound moved; this check must move with it"
        );
    }

    #[test]
    fn a_create_cannot_name_a_status() {
        // docs/23: status is never written directly, and a create is not an
        // exception. `deny_unknown_fields` is what enforces it.
        assert!(
            serde_json::from_str::<CreateRequest>(
                r#"{"title":"t","status_id":"018f2c9e-0000-7000-8000-000000000001"}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<CreateRequest>(r#"{"title":"t","state":"DONE"}"#).is_err());
    }

    #[test]
    fn a_key_reads_as_project_key_and_number() {
        // docs/05's pagination example shows "key": "WR-125". It spans two
        // tables, so it is composed on read (D-051).
        let row = TaskRow {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            project_id: Uuid::now_v7(),
            number: 125,
            title: "t".into(),
            description: None,
            task_type: "TASK".into(),
            priority: "NONE".into(),
            status_id: Uuid::now_v7(),
            state: "BACKLOG".into(),
            reporter_id: Uuid::now_v7(),
            environment_id: None,
            milestone_id: None,
            parent_id: None,
            start_at: None,
            due_at: None,
            position: "11111111".into(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: Uuid::now_v7(),
            updated_at: OffsetDateTime::UNIX_EPOCH,
            updated_by: None,
            version: 1,
            archived_at: None,
        };
        assert_eq!(view(&row, "WR").key, "WR-125");
    }

    #[test]
    fn an_unknown_enum_value_is_refused_rather_than_defaulted() {
        assert!(one_of(None, TASK_TYPES, "TASK", "type", "r").is_ok());
        assert_eq!(
            one_of(Some("EPIC"), TASK_TYPES, "TASK", "type", "r")
                .err()
                .map(|e| e.code()),
            Some(codes::INVALID_ENUM)
        );
    }
}
