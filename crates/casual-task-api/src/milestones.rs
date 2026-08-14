//! `/api/v1/projects/{id}/milestones` and `/api/v1/milestones/{id}`.
//!
//! # The failure this module prevents
//!
//! A milestone that closes its tasks. `docs/03` settles the neighbouring rule
//! for subtasks — "Rollup is displayed (`3/5 done`), never enforced ... implicit
//! status changes are the most confusing behaviour in every tracker that does
//! it" — and a milestone is that rule one level up. So a milestone carries a
//! `completed_at` and **nothing else moves when it is set**: `7/12 done` on a
//! closed milestone is a true and useful sentence, and the five tasks that are
//! not done are still not done.
//!
//! There is no endpoint here that writes a task. Membership is a field on the
//! task (`PATCH /tasks/{id}` with `milestone_id`), which means the permission
//! that governs it is `task.update` on *that task*, evaluated against that
//! task's project — not a milestone-side bulk operation that would need a
//! second, weaker authority check per row.
//!
//! # Which permission governs a milestone, and why it is not a new one
//!
//! `docs/04`'s permission set is **closed** — `casual-task-model::permission`
//! says adding one "is an ADR-adjacent change that must also seed the
//! `permission` table". A milestone is project configuration, like a status or
//! an environment, so authoring one is [`permission::PROJECT_UPDATE`] and
//! reading one is [`permission::TASK_READ`] — the same authority that lets the
//! actor see the work the milestone is about.
//!
//! That mapping is a judgement, not a decision this module is entitled to make
//! silently, and it is recorded as **D-064** in `docs/14`. If it turns out teams
//! want milestone authorship separated from project settings, the fix is a new
//! permission and a migration, not a different check here.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_model::{ProjectId, permission};
use casual_task_persistence::milestone::{
    self, MilestonePatch, MilestoneRow, NewMilestone, Progress,
};
use casual_task_persistence::{Change, Scoped, UnitOfWork, project};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::{self, Body};

/// `docs/21` bounds every input.
const MAX_NAME: usize = 120;

/// A milestone, as a client sees it.
#[derive(Debug, Serialize)]
pub struct MilestoneView {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub due_at: Option<String>,
    /// `null` while open. Setting it closes the milestone and moves no task.
    pub completed_at: Option<String>,
    /// Tasks in a `COMPLETED` state. **Displayed, never enforced** — see the
    /// module docs.
    pub done: i64,
    pub total: i64,
}

fn view(row: &MilestoneRow, progress: Progress) -> MilestoneView {
    MilestoneView {
        id: row.id,
        project_id: row.project_id,
        name: row.name.clone(),
        due_at: row.due_at.map(wire::timestamp),
        completed_at: row.completed_at.map(wire::timestamp),
        done: progress.done,
        total: progress.total,
    }
}

/// `POST /api/v1/projects/{id}/milestones`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub name: String,
    /// RFC 3339. Rejected rather than coerced if it is not.
    #[serde(default)]
    pub due_at: Option<String>,
}

/// `PATCH /api/v1/milestones/{id}`.
///
/// `completed` is a boolean and not a timestamp on purpose: when a milestone
/// closed is the server's answer, and accepting a `completed_at` would let a
/// client backdate one into a report.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "wire::double_option")]
    pub due_at: Option<Option<String>>,
    #[serde(default)]
    pub completed: Option<bool>,
}

/// `GET /api/v1/projects/{id}/milestones` — every milestone, with its rollup.
///
/// Returned whole rather than paged. A milestone list is project configuration
/// with a hard bound (`milestone::MAX_PER_PROJECT`), like the workflow's status
/// list beside it, and a cursor over a control panel is ceremony that every
/// client would have to implement to read twelve rows.
///
/// # Errors
///
/// `404` when the project is not visible, `403` without `task.read`.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let project_row = visible_project(&mut scoped, &ctx, project_id, &request_id).await?;
    authorize(
        &mut scoped,
        &ctx,
        &project_row,
        permission::TASK_READ,
        &request_id,
    )
    .await?;

    let rows = milestone::list_for_project(&mut scoped, project_row.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "listing milestones failed");
            ApiError::internal(&request_id)
        })?;
    let progress = milestone::progress(&mut scoped, project_row.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "counting milestone progress failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    let data: Vec<MilestoneView> = rows
        .iter()
        .map(|row| view(row, progress.get(&row.id).copied().unwrap_or_default()))
        .collect();
    Ok(axum::Json(serde_json::json!({ "data": data })).into_response())
}

/// `POST /api/v1/projects/{id}/milestones`.
///
/// # Errors
///
/// `400` for a malformed name or date, `404` when the project is not visible,
/// `403` without `project.update`, `409` when the name is taken, `422` at the
/// per-project limit.
pub async fn create(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Body(body): Body<CreateRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let name = validated_name(&body.name, &request_id)?;
    let due_at = parse_due(body.due_at.as_deref(), &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let project_row = visible_project(&mut scoped, &ctx, project_id, &request_id).await?;
    authorize(
        &mut scoped,
        &ctx,
        &project_row,
        permission::PROJECT_UPDATE,
        &request_id,
    )
    .await?;

    // Counted before the insert rather than caught as a constraint violation,
    // because there is no constraint: `docs/21` bounds inputs and the schema
    // does not. Refused rather than the list truncated — a configuration list
    // that silently stops growing is the kind of wrong that looks right.
    let held = milestone::count_in_project(&mut scoped, project_row.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "counting milestones failed");
            ApiError::internal(&request_id)
        })?;
    if held >= milestone::MAX_PER_PROJECT {
        return Err(ApiError::unprocessable(
            codes::MILESTONE_LIMIT,
            format!(
                "A project may hold at most {} milestones",
                milestone::MAX_PER_PROJECT
            ),
            &request_id,
        ));
    }

    let new = NewMilestone {
        id: Uuid::now_v7(),
        project_id: project_row.id,
        name: name.to_owned(),
        due_at,
    };
    let Some(row) = milestone::insert(&mut scoped, &new)
        .await
        .map_err(|error| {
            tracing::error!(%error, "creating the milestone failed");
            ApiError::internal(&request_id)
        })?
    else {
        return Err(ApiError::conflict(
            codes::MILESTONE_NAME_TAKEN,
            "A milestone with that name already exists in this project",
            &request_id,
        ));
    };

    record(
        &mut scoped,
        &ctx,
        &row,
        "milestone.created",
        serde_json::json!({ "name": row.name }),
        serde_json::json!({ "before": null, "after": { "name": row.name, "due_at": row.due_at.map(wire::timestamp) } }),
        &request_id,
    )
    .await?;
    unit::commit(tx, &request_id).await?;

    // `0/0`, not a second query: a milestone created one statement ago has no
    // tasks, and asking the database to confirm that is a round trip for a
    // number this handler already knows.
    Ok((
        StatusCode::CREATED,
        axum::Json(view(&row, Progress::default())),
    )
        .into_response())
}

/// `PATCH /api/v1/milestones/{id}` — rename, re-date, close, reopen.
///
/// # Closing is one row, and that is the whole feature
///
/// Nothing cascades. A closed milestone with unfinished tasks is a normal and
/// informative state — it is how a team says "we shipped, this slipped" — and a
/// product that quietly completed those five tasks would destroy the only record
/// that they slipped.
///
/// # Errors
///
/// `400` for a malformed field, `404` when the milestone or its project is not
/// visible, `403` without `project.update`, `409` when the new name is taken.
pub async fn update(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Body(body): Body<PatchRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let name = body
        .name
        .as_deref()
        .map(|raw| validated_name(raw, &request_id).map(ToOwned::to_owned))
        .transpose()?;
    let due_at = match &body.due_at {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => Some(parse_due(Some(raw.as_str()), &request_id)?),
    };

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // The milestone is read first, and then its PROJECT decides whether the
    // caller may see it at all. `docs/04`: visibility is resolved through the
    // project, so a milestone in a project the actor cannot see is a 404 with
    // the same shape as one that does not exist.
    let existing = milestone::read(&mut scoped, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the milestone failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, &request_id))?;
    let project_row = project::read_visible(&mut scoped, &ctx.viewer, existing.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, &request_id))?;
    authorize(
        &mut scoped,
        &ctx,
        &project_row,
        permission::PROJECT_UPDATE,
        &request_id,
    )
    .await?;

    let patch = MilestonePatch {
        name,
        due_at,
        completed: body.completed,
    };
    let Some(row) = milestone::update(&mut scoped, id, &patch)
        .await
        .map_err(|error| {
            tracing::error!(%error, "updating the milestone failed");
            ApiError::internal(&request_id)
        })?
    else {
        return Err(ApiError::conflict(
            codes::MILESTONE_NAME_TAKEN,
            "A milestone with that name already exists in this project",
            &request_id,
        ));
    };

    // The event type distinguishes a close from a rename, because "Milestone
    // 1.4 closed" and "Milestone 1.4 renamed" are different sentences in a feed
    // and a single `milestone.updated` would make a reader open both.
    let event = match (existing.completed_at.is_none(), row.completed_at.is_none()) {
        (true, false) => "milestone.completed",
        (false, true) => "milestone.reopened",
        _ => "milestone.updated",
    };
    record(
        &mut scoped,
        &ctx,
        &row,
        event,
        serde_json::json!({ "name": row.name }),
        serde_json::json!({
            "before": { "name": existing.name, "completed_at": existing.completed_at.map(wire::timestamp) },
            "after":  { "name": row.name,      "completed_at": row.completed_at.map(wire::timestamp) },
        }),
        &request_id,
    )
    .await?;

    let progress = milestone::progress(&mut scoped, row.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "counting milestone progress failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        axum::Json(view(
            &row,
            progress.get(&row.id).copied().unwrap_or_default(),
        )),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// Shared
// ---------------------------------------------------------------------------

async fn visible_project(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    project_id: Uuid,
    request_id: &str,
) -> Result<project::ProjectRow, ApiError> {
    project::read_visible(scoped, &ctx.viewer, project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::PROJECT_NOT_FOUND, request_id))
}

async fn authorize(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    project_row: &project::ProjectRow,
    wanted: casual_task_model::Permission,
    request_id: &str,
) -> Result<(), ApiError> {
    let is_member = project::is_member(scoped, project_row.id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading project membership failed");
            ApiError::internal(request_id)
        })?;
    unit::authorized(
        ctx.authority.may_in_project(
            wanted,
            ProjectId::from_uuid(project_row.id),
            &project_row.teams(),
            &ctx.facts_in_project(is_member),
        ),
        request_id,
    )
}

/// Write the activity and audit record for a milestone change.
async fn record(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    row: &MilestoneRow,
    event_type: &str,
    activity: serde_json::Value,
    audit: serde_json::Value,
    request_id: &str,
) -> Result<(), ApiError> {
    UnitOfWork::record(
        scoped,
        &Change {
            aggregate_type: "milestone".to_owned(),
            aggregate_id: row.id,
            project_id: Some(row.project_id),
            event_type: event_type.to_owned(),
            activity_changes: activity,
            audit_changes: audit,
            payload: serde_json::json!({
                "id": row.id,
                "project_id": row.project_id,
                "name": row.name,
            }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, event_type, "recording the milestone change failed");
        ApiError::internal(request_id)
    })?;
    Ok(())
}

fn validated_name<'a>(raw: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request(
            codes::MISSING_FIELD,
            "name must not be empty",
            request_id,
        ));
    }
    if name.chars().count() > MAX_NAME {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            format!("name must be at most {MAX_NAME} characters"),
            request_id,
        ));
    }
    Ok(name)
}

fn parse_due(raw: Option<&str>, request_id: &str) -> Result<Option<OffsetDateTime>, ApiError> {
    raw.map(|value| {
        OffsetDateTime::parse(value, &Rfc3339).map_err(|_| {
            ApiError::bad_request(
                codes::MALFORMED_BODY,
                "due_at must be an RFC 3339 timestamp",
                request_id,
            )
        })
    })
    .transpose()
}

#[cfg(test)]
#[path = "milestones_tests.rs"]
mod tests;
