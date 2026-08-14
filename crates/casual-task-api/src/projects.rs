//! `/api/v1/projects` (C-006).
//!
//! # The shape of every handler here
//!
//! One transaction, opened before anything is read and committed once. Inside
//! it, in order: apply the tenant scope, resolve the actor's authority once
//! ([`Context`]), answer the visibility question, answer the permission
//! question, write, and record the change through [`UnitOfWork`].
//!
//! That order is not arbitrary. Visibility comes before permission because
//! `docs/04` evaluates it first and because the two produce different answers —
//! `404` for something you cannot see, `403` for something you can see and may
//! not touch. Getting them the other way round leaks the existence of every
//! private project to anyone who probes for a `403`.
//!
//! # Everything commits together
//!
//! ADR-006: "the domain change, its activity record, its audit record, and its
//! outbox event commit together or not at all". The `UnitOfWork::record` call
//! sits in the same transaction as the `INSERT`, so there is no interval in
//! which a project exists with no history.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_model::{Cursor, ProjectId, permission};
use casual_task_persistence::project::{self, CreateError, NewProject, ProjectPatch, ProjectRow};
use casual_task_persistence::{Change, Scoped, UnitOfWork, idempotency};
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

/// The project representation. `docs/05`: `snake_case`, RFC 3339 UTC, UUIDv7.
#[derive(Debug, Serialize)]
pub struct ProjectView {
    pub id: Uuid,
    /// Immutable after creation (ADR-007) — task keys appear in commits, chat,
    /// and external tickets.
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub visibility: String,
    pub team_ids: Vec<Uuid>,
    pub workflow_id: Uuid,
    pub created_at: String,
    pub created_by: Uuid,
    pub updated_at: String,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<String>,
    /// The same number the `ETag` carries. Present in the body as well because
    /// a client batching a list cannot read one `ETag` per row.
    pub version: i64,
}

impl From<&ProjectRow> for ProjectView {
    fn from(row: &ProjectRow) -> Self {
        Self {
            id: row.id,
            key: row.key.clone(),
            name: row.name.clone(),
            description: row.description.clone(),
            visibility: row.visibility.clone(),
            team_ids: row.team_ids.clone(),
            workflow_id: row.workflow_id,
            created_at: wire::timestamp(row.created_at),
            created_by: row.created_by,
            updated_at: wire::timestamp(row.updated_at),
            updated_by: row.updated_by,
            archived_at: row.archived_at.map(wire::timestamp),
            version: row.version,
        }
    }
}

/// `POST /api/v1/projects`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    /// The teams this project involves. `docs/03`: a project may involve any
    /// number of them, and every one sits in the task's scope chain.
    #[serde(default)]
    pub team_ids: Option<Vec<Uuid>>,
}

/// `PATCH /api/v1/projects/{id}`.
///
/// `key` is accepted and then refused with `TF-PRJ-0003`. Leaving it out of the
/// struct would make it an *unknown field* — a `400` saying "we have never
/// heard of `key`", when the truth is that the field exists and cannot change.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// `Option<Option<_>>`: absent leaves the description alone, `null` clears
    /// it (`docs/05` §Conventions).
    #[serde(default, deserialize_with = "double_option")]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
}

use crate::wire::double_option;

/// The visibility values `docs/22`'s `visibility` enum permits.
const VISIBILITIES: &[&str] = &["PRIVATE", "TEAM", "WORKSPACE"];

/// ADR-007's key format, as `migrations/0004` writes it:
/// `^[A-Z][A-Z0-9]{1,9}$`.
///
/// Checked here as well as by the `CHECK` constraint so the caller gets
/// `TF-PRJ-0004` and a description of the rule, rather than a `500` from a
/// constraint violation nobody can read.
fn well_formed_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    (2..=10).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// `GET /api/v1/projects` — the projects the caller can see, newest first.
///
/// # Errors
///
/// `400` for a bad cursor or page size; `404` when the workspace is not one the
/// caller belongs to (refused by [`WorkspaceMember`] before this runs).
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let (limit, after) = page_params(&params, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load_read(
        &state.metrics,
        &mut scoped,
        &member,
        &headers,
        &request_id,
        None,
    )
    .await?;

    let mut rows = project::list_visible(&mut scoped, &ctx.viewer, after, limit)
        .await
        .map_err(|error| {
            tracing::error!(%error, "listing projects failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    // One more row than asked for was fetched; its existence is the answer to
    // "is there a next page", and it is not part of this one.
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| Cursor::new(vec![wire::timestamp(row.created_at)], row.id).encode());

    Ok(axum::Json(Paged {
        data: rows.iter().map(ProjectView::from).collect::<Vec<_>>(),
        page: Page {
            next_cursor,
            has_more,
        },
    })
    .into_response())
}

/// `POST /api/v1/projects`.
///
/// # Errors
///
/// `400` for a malformed key or unknown field, `403` when the caller holds no
/// `project.create` grant, `409` when the key is taken.
pub async fn create(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Body(body): Body<CreateRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let key = body.key.trim().to_owned();
    if !well_formed_key(&key) {
        return Err(ApiError::bad_request(
            codes::PROJECT_KEY_FORMAT,
            "A project key is 2-10 characters: an uppercase letter followed by \
             uppercase letters or digits",
            &request_id,
        ));
    }
    let name = validated_name(&body.name, &request_id)?;
    let visibility = validated_visibility(body.visibility.as_deref(), &request_id)?;
    let idempotency_key = unit::idempotency_key(&headers, &request_id)?;
    let request_hash = unit::hash(&[key.as_bytes(), name.as_bytes(), visibility.as_bytes()]);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // `project.create` is a workspace-scope authority: there is no project yet
    // to scope it to.
    unit::authorized(
        ctx.authority.may_in_workspace(permission::PROJECT_CREATE),
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

    // docs/23: a project has exactly one workflow, and the default one "works
    // with zero configuration". Nothing else creates it, so the first project
    // in a workspace brings it into existence — in this transaction, so a
    // rolled-back create leaves no orphan workflow behind.
    let workflow = casual_task_persistence::workflow::ensure_default_workflow(&mut scoped)
        .await
        .map_err(|error| {
            tracing::error!(%error, "provisioning the default workflow failed");
            ApiError::internal(&request_id)
        })?;

    let new = NewProject {
        id: Uuid::now_v7(),
        key: key.clone(),
        name: name.to_owned(),
        description: body.description.clone(),
        visibility: visibility.to_owned(),
        workflow_id: workflow,
        created_by: ctx.actor.as_uuid(),
    };
    let wanted_teams = body.team_ids.clone().unwrap_or_default();
    let row = match project::insert(&mut scoped, &new).await {
        Ok(row) => row,
        Err(CreateError::KeyTaken) => {
            return Err(ApiError::conflict(
                codes::PROJECT_KEY_TAKEN,
                "That project key is already in use in this workspace",
                &request_id,
            )
            .with_details(serde_json::json!({ "key": key })));
        }
        Err(CreateError::Db(error)) => {
            // A team id from another workspace, or one that does not exist,
            // arrives here as a foreign-key violation. 422 rather than 500:
            // the request is well-formed and names something that is not there.
            if matches!(&error, sqlx::Error::Database(db) if db.is_foreign_key_violation()) {
                return Err(ApiError::unprocessable(
                    codes::REFERENCE_NOT_FOUND,
                    "team_id does not name a team in this workspace",
                    &request_id,
                ));
            }
            tracing::error!(%error, "creating the project failed");
            return Err(ApiError::internal(&request_id));
        }
    };

    // Teams go on through the same helper the dedicated endpoint uses, so
    // create cannot grow a laxer check than add. It refuses a team that is not
    // in this workspace with 422 rather than a foreign-key 500.
    let attached = crate::project_teams::attach_all(
        &mut scoped,
        row.id,
        ctx.actor.as_uuid(),
        &wanted_teams,
        &request_id,
    )
    .await?;

    // `project_membership` conveys belonging, never capability (migration
    // 0003). The creator belongs to what they created — without this, creating
    // a PRIVATE project would produce something its author cannot read back.
    project::add_member(&mut scoped, row.id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "recording project membership failed");
            ApiError::internal(&request_id)
        })?;

    // The row was read before the teams were attached, so the view carries
    // what actually landed rather than an empty list the caller would have to
    // re-fetch to correct.
    let mut view = ProjectView::from(&row);
    view.team_ids = attached;
    let payload = serde_json::to_value(&view).unwrap_or(serde_json::Value::Null);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "project".to_owned(),
            aggregate_id: row.id,
            project_id: Some(row.id),
            event_type: "project.created".to_owned(),
            // Display VALUES, not ids: docs/25 — the stream is rendered years
            // later, possibly after a rename, and must still read correctly.
            activity_changes: serde_json::json!({ "key": row.key, "name": row.name }),
            audit_changes: serde_json::json!({ "before": null, "after": payload }),
            payload: payload.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the project create failed");
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
            (header::LOCATION, format!("/api/v1/projects/{}", row.id)),
        ],
        axum::Json(body),
    )
        .into_response())
}

include!("project_updates.rs");
#[cfg(test)]
#[path = "projects_tests.rs"]
mod tests;
