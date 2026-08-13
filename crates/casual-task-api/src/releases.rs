//! `/api/v1/projects/{id}/releases` and `/api/v1/releases/{id}`
//! (`docs/45-DEVELOPMENT-LIFECYCLE-AND-CUSTODY.md` §The two clocks).
//!
//! # The question this answers that no other surface can
//!
//! "What went to staging on Tuesday, and did the API fix go with it?" A status
//! board says what state each task is in. `GET /tasks?environment=…` says where
//! each one has reached. Neither says that eleven of them moved *together*,
//! which is the only fact a release conversation is made of, and the fact a
//! rollback needs.
//!
//! # Which permission gates cutting one, and why there is no `release.manage`
//!
//! `task.update` in the project — exactly what `POST /tasks/{id}/promotions`
//! demands, because a release *is* those promotions with a name tied around
//! them. The registry is closed at 29 keys (`docs/04`), and adding a thirtieth
//! for this would create an authority that is neither more nor less than one
//! that already exists: anyone who can promote eleven tasks one at a time can
//! promote them together, and anyone who cannot, cannot. A batch action taking
//! exactly the authority of the actions it batches is the property that keeps
//! that true without a new grant to reason about.
//!
//! This is a deliberate decision and not an omission. If releases later need
//! their own authority — a release manager who may cut but not edit — that is a
//! registry change with a migration, argued on its own terms.
//!
//! # Why the batch is all-or-nothing
//!
//! `POST /tasks/bulk` is `207 Multi-Status` and reports each task's fate
//! separately, which is right *there*: those tasks have nothing to do with each
//! other, so nine successes are worth keeping. Here they do have something to do
//! with each other. A release that recorded nine of eleven reads as complete,
//! and the two that are missing become invisible in the exact surface built to
//! find them. So it commits whole or refuses whole.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_model::{Cursor, ProjectId, permission};
use casual_task_persistence::project::ProjectRow;
use casual_task_persistence::release::{self, ReleaseError, ReleaseRow, ReleasedTask};
use casual_task_persistence::{Change, Scoped, UnitOfWork, project};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::{self, Body, Page, Paged};

/// `docs/21` bounds every input. The ceiling is the transaction's, not the
/// product's: the whole batch moves in one transaction, and a release of ten
/// thousand tasks is a background job with progress rather than a request.
const MAX_TASKS: usize = 200;

/// A release, as the release list and the environment view render it.
#[derive(Debug, Serialize)]
pub struct ReleaseView {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub note: Option<String>,
    pub created_by: Uuid,
    pub created_at: String,
}

impl From<ReleaseRow> for ReleaseView {
    fn from(row: ReleaseRow) -> Self {
        Self {
            id: row.id,
            project_id: row.project_id,
            name: row.name,
            note: row.note,
            created_by: row.created_by,
            created_at: wire::timestamp(row.created_at),
        }
    }
}

/// One task a release carried.
#[derive(Debug, Serialize)]
pub struct ReleasedTaskView {
    pub task_id: Uuid,
    /// `ONB-14`. Present so the list reads without a request per row.
    pub key: String,
    pub title: String,
    pub promoted_at: String,
}

impl From<ReleasedTask> for ReleasedTaskView {
    fn from(row: ReleasedTask) -> Self {
        Self {
            task_id: row.task_id,
            key: row.task_key,
            title: row.title,
            promoted_at: wire::timestamp(row.promoted_at),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CutRequest {
    pub name: String,
    #[serde(default)]
    pub note: Option<String>,
    /// Where the batch went. Required: a release with no environment records
    /// that something happened but not where, which is not a release.
    pub environment_id: Uuid,
    pub task_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `POST /api/v1/projects/{id}/releases` — these went out together.
///
/// # Errors
///
/// `400` for an empty or over-long name, an empty or over-large task list;
/// `403` without `task.update`; `404` when the project is not visible; `409`
/// when the name is already used in this project; `422` when the environment is
/// not on the project or any task is not in it.
pub async fn cut(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Body(body): Body<CutRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let name = validated_name(&body.name, &request_id)?.to_owned();
    let note = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    validated_batch(&body.task_ids, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let project = visible(&mut scoped, &ctx, project_id, &request_id).await?;
    authorize(&mut scoped, &ctx, &project, &request_id).await?;

    let record = release::create(&mut scoped, project.id, &name, note, ctx.actor.as_uuid())
        .await
        .map_err(|error| refused(&error, &name, &request_id))?;

    let moved = release::promote_batch(
        &mut scoped,
        record.id,
        project.id,
        body.environment_id,
        &body.task_ids,
        ctx.actor.as_uuid(),
    )
    .await
    .map_err(|error| refused(&error, &name, &request_id))?;

    // The release itself is one change, and each task's promotion is another.
    // Both are needed: the release list reads the first, and a task's own
    // history has to say "went out in 2.4.0" or the batch is invisible from the
    // only place someone looks when they are debugging that task.
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "release".to_owned(),
            aggregate_id: record.id,
            project_id: Some(project.id),
            event_type: "release.cut".to_owned(),
            activity_changes: serde_json::json!({
                "name": record.name,
                "environment_id": body.environment_id,
                "tasks": moved.len(),
            }),
            audit_changes: serde_json::json!({
                "after": { "name": record.name, "environment_id": body.environment_id },
            }),
            payload: serde_json::json!({
                "release_id": record.id,
                "environment_id": body.environment_id,
                "task_ids": moved,
            }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the release failed");
        ApiError::internal(&request_id)
    })?;

    for task_id in &moved {
        UnitOfWork::record(
            &mut scoped,
            &Change {
                aggregate_type: "task".to_owned(),
                aggregate_id: *task_id,
                project_id: Some(project.id),
                event_type: "task.promoted".to_owned(),
                activity_changes: serde_json::json!({
                    "environment_id": body.environment_id,
                    "release_id": record.id,
                    "release_name": record.name,
                }),
                audit_changes: serde_json::json!({
                    "after": { "environment_id": body.environment_id },
                }),
                payload: serde_json::json!({
                    "task_id": task_id,
                    "environment_id": body.environment_id,
                    "release_id": record.id,
                }),
                schema_version: 1,
            },
            &ctx.provenance,
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, "recording a release promotion failed");
            ApiError::internal(&request_id)
        })?;
    }

    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "release": ReleaseView::from(record),
            "environment_id": body.environment_id,
            "task_ids": moved,
        })),
    )
        .into_response())
}

/// `GET /api/v1/projects/{id}/releases` — newest first, cursor-paged.
///
/// # Errors
///
/// `404` when the project is not visible, `400` for a malformed cursor or an
/// over-limit page.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let limit = wire::limit(query.limit, &request_id)?;
    let after = wire::cursor(query.cursor.as_deref(), &request_id)?.map(|cursor| cursor.id);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // Visibility only, like the environment list: what shipped is the project's
    // own vocabulary, and gating it would leave a member able to see a task on
    // staging but not what carried it there.
    let project = visible(&mut scoped, &ctx, project_id, &request_id).await?;
    let mut rows = release::list(&mut scoped, project.id, after, i64::from(limit) + 1)
        .await
        .map_err(|error| internal(error, "listing releases", &request_id))?;
    unit::commit(tx, &request_id).await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| Cursor::new(Vec::new(), row.id).encode());

    Ok((
        StatusCode::OK,
        axum::Json(Paged {
            data: rows.into_iter().map(ReleaseView::from).collect::<Vec<_>>(),
            page: Page {
                next_cursor,
                has_more,
            },
        }),
    )
        .into_response())
}

/// `GET /api/v1/releases/{id}` — the release and what it carried.
///
/// # Errors
///
/// `404` when the release does not exist or its project is not visible — the
/// same answer for both, so the endpoint cannot be used to probe for release
/// names in projects the caller cannot open.
pub async fn read(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(release_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let record = release::read(&mut scoped, release_id)
        .await
        .map_err(|error| internal(error, "reading the release", &request_id))?
        .ok_or_else(|| ApiError::missing(codes::RELEASE_NOT_FOUND, &request_id))?;
    // RLS bounds this to the workspace; project visibility is the narrower
    // question and it is asked explicitly.
    visible(&mut scoped, &ctx, record.project_id, &request_id).await?;

    let tasks = release::contents(&mut scoped, release_id)
        .await
        .map_err(|error| internal(error, "reading release contents", &request_id))?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "release": ReleaseView::from(record),
            "tasks": tasks.into_iter().map(ReleasedTaskView::from).collect::<Vec<_>>(),
        })),
    )
        .into_response())
}

async fn visible(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    project_id: Uuid,
    request_id: &str,
) -> Result<ProjectRow, ApiError> {
    project::read_visible(scoped, &ctx.viewer, project_id)
        .await
        .map_err(|error| internal(error, "reading the project", request_id))?
        .ok_or_else(|| ApiError::missing(codes::PROJECT_NOT_FOUND, request_id))
}

/// `task.update` in this project — see the module docs for why not a permission
/// of its own.
async fn authorize(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    project: &ProjectRow,
    request_id: &str,
) -> Result<(), ApiError> {
    let is_member = project::is_member(scoped, project.id, ctx.actor.as_uuid())
        .await
        .map_err(|error| internal(error, "reading project membership", request_id))?;
    unit::authorized(
        ctx.authority.may_in_project(
            permission::TASK_UPDATE,
            ProjectId::from_uuid(project.id),
            &project.teams(),
            &ctx.facts_in_project(is_member),
        ),
        request_id,
    )
}

fn internal(error: sqlx::Error, what: &'static str, request_id: &str) -> ApiError {
    tracing::error!(%error, what, "a release request failed");
    ApiError::internal(request_id)
}

fn refused(error: &ReleaseError, name: &str, request_id: &str) -> ApiError {
    match error {
        ReleaseError::NameTaken => ApiError::conflict(
            codes::RELEASE_NAME_TAKEN,
            "That release name is already used in this project",
            request_id,
        )
        .with_details(serde_json::json!({ "name": name })),
        ReleaseError::EnvironmentNotOnProject => ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "That environment does not belong to this project",
            request_id,
        ),
        ReleaseError::TasksNotInProject => ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "Every task in a release must belong to the project it is cut in. \
             Nothing was recorded",
            request_id,
        ),
        ReleaseError::Db(error) => {
            tracing::error!(%error, "a release write failed");
            ApiError::internal(request_id)
        }
    }
}

/// A release name appears in a task's history for as long as the task exists,
/// so it is short and it is not blank.
fn validated_name<'a>(name: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 60 {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "A release name is 1 to 60 characters",
            request_id,
        ));
    }
    Ok(trimmed)
}

/// An empty release is not a release, and the ceiling is the transaction's.
fn validated_batch(task_ids: &[Uuid], request_id: &str) -> Result<(), ApiError> {
    if task_ids.is_empty() {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "A release carries at least one task",
            request_id,
        ));
    }
    if task_ids.len() > MAX_TASKS {
        return Err(ApiError::bad_request(
            codes::BULK_TOO_LARGE,
            "A release carries at most 200 tasks in one request",
            request_id,
        )
        .with_details(serde_json::json!({ "limit": MAX_TASKS, "sent": task_ids.len() })));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_name_is_bounded_at_both_ends() {
        assert_eq!(validated_name("  2.4.0  ", "r").ok(), Some("2.4.0"));
        for bad in ["", "   "] {
            assert!(validated_name(bad, "r").is_err(), "{bad:?}");
        }
        assert!(validated_name(&"x".repeat(60), "r").is_ok());
        assert!(validated_name(&"x".repeat(61), "r").is_err());
    }

    #[test]
    fn a_batch_is_neither_empty_nor_unbounded() {
        assert!(validated_batch(&[], "r").is_err());
        assert!(validated_batch(&[Uuid::now_v7()], "r").is_ok());
        let too_many: Vec<Uuid> = (0..=MAX_TASKS).map(|_| Uuid::now_v7()).collect();
        assert!(validated_batch(&too_many, "r").is_err());
    }
}
