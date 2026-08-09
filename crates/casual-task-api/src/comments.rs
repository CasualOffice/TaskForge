//! `/api/v1/tasks/{id}/comments` (C-009).
//!
//! # Visibility is decided by the task, never by the comment
//!
//! A comment carries no permission of its own. Whether you may read or write
//! one is entirely a question about the task it hangs off, so every handler
//! here resolves the task through `task::read_visible` first and refuses with
//! `404` if it is not visible — `docs/04` requires absent and invisible to be
//! indistinguishable, and a comment endpoint that answered differently would
//! leak the existence of tasks the actor cannot see.
//!
//! # Editing and deleting are the author's, not the reader's
//!
//! `TASK_COMMENT` grants posting. It does not grant editing someone else's
//! words, which is why the author check is in the `WHERE` clause of the update
//! rather than a branch above it: a check that ran before the write could pass
//! and then update a row someone else had just replaced.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_model::{Cursor, permission};
use casual_task_persistence::comment::{CommentError, CommentRow};
use casual_task_persistence::{Change, UnitOfWork, comment, task};
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

/// The largest comment body the schema accepts (`migrations/0006`).
///
/// Checked here as well so an oversized body is a `400` with a field name
/// rather than a database `CHECK` violation surfacing as a `500`.
const MAX_BODY: usize = 65_536;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub body: String,
    /// A top-level comment to reply to. Threading is one level (`docs/06`).
    #[serde(default)]
    pub parent_comment_id: Option<Uuid>,
    /// User ids mentioned in the body, resolved by the client.
    ///
    /// Stored as sent rather than re-parsed from the text: `migrations/0006`
    /// says mentions are "resolved at write time", and re-resolving `@sam`
    /// years later finds a different Sam or nobody.
    #[serde(default)]
    pub mentions: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    pub body: String,
    #[serde(default)]
    pub mentions: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct CommentView {
    pub id: Uuid,
    pub task_id: Uuid,
    pub parent_comment_id: Option<Uuid>,
    pub author_id: Uuid,
    pub body: String,
    pub mentions: Vec<Uuid>,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub version: i64,
}

fn view(row: &CommentRow) -> CommentView {
    CommentView {
        id: row.id,
        task_id: row.task_id,
        parent_comment_id: row.parent_comment_id,
        author_id: row.author_id,
        body: row.body.clone(),
        mentions: row.mentions.clone(),
        created_at: row.created_at.format(&Rfc3339).unwrap_or_default(),
        edited_at: row.edited_at.and_then(|t| t.format(&Rfc3339).ok()),
        version: row.version,
    }
}

fn validated_body<'b>(body: &'b str, request_id: &str) -> Result<&'b str, ApiError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(
            codes::MISSING_FIELD,
            "body must not be empty",
            request_id,
        ));
    }
    if body.len() > MAX_BODY {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "body must be at most 65536 bytes",
            request_id,
        ));
    }
    Ok(trimmed)
}

/// `POST /api/v1/tasks/{id}/comments`.
///
/// # Errors
///
/// `404` if the task is not visible, `403` without `task.comment`, `422` for a
/// bad parent, `400` for a malformed body.
pub async fn create(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Body(request): Body<CreateRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let body = validated_body(&request.body, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let (task_row, _) = task::read_visible(&mut scoped, &ctx.viewer, task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, &request_id))?;

    unit::authorized(
        ctx.authority.may_in_project(
            permission::TASK_COMMENT,
            casual_task_model::ProjectId::from_uuid(task_row.project_id),
            None,
            &ctx.facts_in_project(true),
        ),
        &request_id,
    )?;

    let row = comment::create(
        &mut scoped,
        task_row.id,
        ctx.actor.as_uuid(),
        body,
        request.parent_comment_id,
        &request.mentions,
    )
    .await
    .map_err(|error| match error {
        CommentError::NoSuchTask => ApiError::missing(codes::TASK_NOT_FOUND, &request_id),
        CommentError::BadParent => ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "parent_comment_id must name a top-level comment on this task",
            &request_id,
        ),
        CommentError::Database(error) => {
            tracing::error!(%error, "creating the comment failed");
            ApiError::internal(&request_id)
        }
    })?;

    let rendered = view(&row);
    let payload = serde_json::to_value(&rendered).unwrap_or(serde_json::Value::Null);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "comment".to_owned(),
            aggregate_id: row.id,
            project_id: Some(task_row.project_id),
            event_type: "comment.created".to_owned(),
            // The activity stream carries the task, not the comment body:
            // `docs/25` says the stream holds display values, and a comment
            // body is customer content that would then live in two places.
            activity_changes: serde_json::json!({ "task_id": task_row.id }),
            audit_changes: serde_json::json!({ "before": null, "after": { "id": row.id } }),
            payload: payload.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the comment create failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::CREATED,
        [
            (header::ETAG, etag::tag(row.version)),
            (header::LOCATION, format!("/api/v1/comments/{}", row.id)),
        ],
        axum::Json(payload),
    )
        .into_response())
}

#[derive(Debug, Deserialize)]
pub struct ThreadQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `GET /api/v1/tasks/{id}/comments` — the thread, oldest first, cursor-paged.
///
/// # Errors
///
/// `404` if the task is not visible, `400` for a malformed cursor.
pub async fn thread(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Query(query): Query<ThreadQuery>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let limit = i64::from(wire::limit(query.limit, &request_id)?);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let (task_row, _) = task::read_visible(&mut scoped, &ctx.viewer, task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, &request_id))?;

    let after = query
        .cursor
        .as_deref()
        .map(|raw| decode_cursor(raw, &request_id))
        .transpose()?;

    // One more than asked for, so "is there a next page" is answered by the
    // rows themselves rather than by a second count query.
    let mut rows = comment::thread(&mut scoped, task_row.id, after, limit + 1)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the thread failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    let has_more = rows.len() as i64 > limit;
    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    let next = has_more
        .then(|| rows.last())
        .flatten()
        .map(|last| encode_cursor(last.created_at, last.id));

    Ok(axum::Json(Paged {
        data: rows.iter().map(view).collect::<Vec<_>>(),
        page: Page {
            next_cursor: next,
            has_more,
        },
    })
    .into_response())
}

/// `PATCH /api/v1/comments/{id}` — edit your own comment.
///
/// # Errors
///
/// `404` if invisible or not yours, `409` on a stale version, `428` without
/// `If-Match`.
pub async fn edit(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(comment_id): Path<Uuid>,
    Body(request): Body<EditRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let body = validated_body(&request.body, &request_id)?;
    let expected = etag::if_match(&headers, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let existing = comment::read(&mut scoped, comment_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the comment failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, &request_id))?;

    // The task gate again: a comment on a task you cannot see must be as
    // invisible as the task.
    task::read_visible(&mut scoped, &ctx.viewer, existing.task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, &request_id))?;

    let updated = comment::edit(
        &mut scoped,
        comment_id,
        ctx.actor.as_uuid(),
        body,
        &request.mentions,
        expected,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "editing the comment failed");
        ApiError::internal(&request_id)
    })?;

    let Some(row) = updated else {
        // Author mismatch and version mismatch are both `None`. They are
        // reported differently: a version conflict is retryable and a
        // non-author edit never becomes possible, so conflating them would
        // send a client into a retry loop it cannot win.
        return Err(if existing.author_id == ctx.actor.as_uuid() {
            ApiError::conflict(
                codes::VERSION_CONFLICT,
                format!(
                    "The comment has changed since you read it (current version {})",
                    existing.version
                ),
                &request_id,
            )
        } else {
            ApiError::missing(codes::NOT_FOUND, &request_id)
        });
    };

    let rendered = view(&row);
    let payload = serde_json::to_value(&rendered).unwrap_or(serde_json::Value::Null);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "comment".to_owned(),
            aggregate_id: row.id,
            project_id: None,
            event_type: "comment.updated".to_owned(),
            activity_changes: serde_json::json!({ "task_id": row.task_id }),
            audit_changes: serde_json::json!({ "before": { "version": existing.version },
                                               "after":  { "version": row.version } }),
            payload: payload.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the comment edit failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(row.version))],
        axum::Json(payload),
    )
        .into_response())
}

/// `(created_at, id)` as an opaque cursor.
///
/// Opaque because `docs/05` says so: a client that parses a cursor is a client
/// that breaks when the sort key changes.
fn encode_cursor(at: OffsetDateTime, id: Uuid) -> String {
    Cursor::new(vec![wire::timestamp(at)], id).encode()
}

fn decode_cursor(raw: &str, request_id: &str) -> Result<(OffsetDateTime, Uuid), ApiError> {
    let bad = || ApiError::bad_request(codes::BAD_CURSOR, "cursor is not valid", request_id);
    let cursor = Cursor::decode(raw).map_err(|_| bad())?;
    let key = cursor.keys.first().ok_or_else(bad)?;
    let at = OffsetDateTime::parse(key, &Rfc3339).map_err(|_| bad())?;
    Ok((at, cursor.id))
}
