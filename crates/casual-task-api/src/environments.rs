//! `/api/v1/projects/{id}/environments` and `/api/v1/environments/{id}`
//! (`docs/05` §Projects, workflow, admin; `docs/01` FR-3).
//!
//! # An environment is part of the permission model, not just a task field
//!
//! `scope_type` has an `ENVIRONMENT` member (`migrations/0001`), `Scope` has an
//! `Environment` variant, and `docs/04`'s closed constraint set has
//! `environment_in`. So creating an environment creates a *scope somebody can
//! be granted authority in*, and narrowing a grant to `environment_in
//! [staging]` is how "this contractor may close tickets, but only in staging"
//! is expressed. That is why authoring them is an administrative act and not a
//! field edit.
//!
//! # Which permission gates it, and why it is not `project.workflow.manage`
//!
//! `project.update`. `migrations/0011` seeds `project.workflow.manage` with the
//! description "Configure statuses and transitions" — using it for environments
//! would make the registry's own description of its own permission wrong, which
//! is worse than reusing a slightly broader one. An environment is project
//! configuration, `project.update` is the authority over project configuration,
//! and both are project-scoped so the grant reaches exactly as far as the
//! object does.
//!
//! # Why deleting one demands an explicit target
//!
//! `TF-PRJ-0005` — "cannot delete an environment in use — supply a migration
//! target" — is the same rule `docs/23` states for a status, and it is in the
//! registry for the same reason: tasks that silently lose a field they were
//! filtered and granted by are tasks whose history does not explain them.
//! `task.environment_id` is nullable, so "clear it" *is* an available answer
//! here where it is not for a status — but it has to be said out loud, because
//! untagging four thousand tasks is a decision and not a default.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_model::{ProjectId, permission};
use casual_task_persistence::environment::{self, EnvironmentRow, WriteError};
use casual_task_persistence::project::ProjectRow;
use casual_task_persistence::{Change, Scoped, UnitOfWork, project};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::wire::Body;
use crate::{etag, unit};

/// The environment representation. `docs/05`: `snake_case`, UUIDv7.
#[derive(Debug, Serialize)]
pub struct EnvironmentView {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    /// Pipeline order — `dev`, `staging`, `production`. Not alphabetical, which
    /// would put production in the middle.
    pub position: i32,
}

impl From<EnvironmentRow> for EnvironmentView {
    fn from(row: EnvironmentRow) -> Self {
        Self {
            id: row.id,
            project_id: row.project_id,
            name: row.name,
            position: row.position,
        }
    }
}

/// `POST /api/v1/projects/{id}/environments`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub name: String,
}

/// `PATCH /api/v1/environments/{id}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameRequest {
    pub name: String,
}

/// `PUT /api/v1/projects/{id}/environments/order`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReorderRequest {
    /// Every environment of this project, in the order they deploy.
    pub environment_ids: Vec<Uuid>,
}

/// `DELETE /api/v1/environments/{id}?migrate_to=…`.
#[derive(Debug, Deserialize)]
pub struct DeleteParams {
    /// Another environment's id, or the literal `none` to clear the field on
    /// every task that carries this one.
    ///
    /// A `uuid` type here would make "clear it" unspellable and force a second
    /// parameter that could contradict the first. `none` cannot collide with a
    /// UUID, so one parameter carries the whole decision.
    #[serde(default)]
    pub migrate_to: Option<String>,
}

/// `PUT /api/v1/tasks/{id}/environment`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetOnTaskRequest {
    /// `null` clears it. **Absent is refused** with `TF-VAL-0003` — the field is
    /// the whole request, and an empty body meaning "clear" would make a client
    /// that forgot to send the value indistinguishable from a user who meant to
    /// remove one.
    ///
    /// `Option<Option<_>>` is the only shape that can tell those apart: serde
    /// fills a bare `Option` from a missing key without complaint, which is
    /// exactly the confusion being prevented.
    #[serde(default, deserialize_with = "crate::wire::double_option")]
    pub environment_id: Option<Option<Uuid>>,
}

/// `GET /api/v1/projects/{id}/environments`.
///
/// # Errors
///
/// `404` when the project is absent or invisible.
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

    // Visibility only. Reading the environments of a project you can see is
    // reading the vocabulary its tasks are labelled with — the same argument
    // `crate::workflows::read` makes for statuses. Gating it would leave a
    // member able to filter by an environment they were never shown the name of.
    let project = visible(&mut scoped, &ctx, project_id, &request_id).await?;
    let rows = environment::list(&mut scoped, project.id)
        .await
        .map_err(|error| internal(error, "listing environments", &request_id))?;
    unit::commit(tx, &request_id).await?;

    Ok(axum::Json(serde_json::json!({
        "data": rows.into_iter().map(EnvironmentView::from).collect::<Vec<_>>(),
    }))
    .into_response())
}

/// `POST /api/v1/projects/{id}/environments`.
///
/// # Errors
///
/// `400` for a bad name, `403` without `project.update`, `404`, `409` for a
/// duplicate name, `422` when the project already holds the maximum.
pub async fn create(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Body(body): Body<CreateRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let name = validated_name(&body.name, &request_id)?.to_owned();

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let project = visible(&mut scoped, &ctx, project_id, &request_id).await?;
    authorize(&mut scoped, &ctx, &project, &request_id).await?;

    let held = environment::count_in(&mut scoped, project.id)
        .await
        .map_err(|error| internal(error, "counting environments", &request_id))?;
    if held >= environment::MAX_PER_PROJECT {
        return Err(ApiError::unprocessable(
            codes::OUT_OF_RANGE,
            "This project already has the maximum number of environments",
            &request_id,
        )
        .with_details(serde_json::json!({ "limit": environment::MAX_PER_PROJECT })));
    }

    let row = environment::insert(&mut scoped, project.id, &name)
        .await
        .map_err(|error| write_error(error, &name, &request_id))?;
    let view = EnvironmentView::from(row);
    record(
        &mut scoped,
        &ctx,
        &project,
        view.id,
        "project.environment.created",
        serde_json::json!({ "environment": view.name }),
        serde_json::json!({ "before": serde_json::Value::Null, "after": view }),
        &request_id,
    )
    .await?;
    unit::commit(tx, &request_id).await?;

    Ok((StatusCode::CREATED, axum::Json(view)).into_response())
}

/// `PATCH /api/v1/environments/{id}`.
///
/// # Errors
///
/// `400`, `403`, `404`, `409` for a duplicate name.
pub async fn rename(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(environment_id): Path<Uuid>,
    Body(body): Body<RenameRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let name = validated_name(&body.name, &request_id)?.to_owned();

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let (before, project) = owned(&mut scoped, &ctx, environment_id, &request_id).await?;
    authorize(&mut scoped, &ctx, &project, &request_id).await?;

    environment::rename(&mut scoped, environment_id, &name)
        .await
        .map_err(|error| write_error(error, &name, &request_id))?;

    let after = EnvironmentView {
        name: name.clone(),
        ..EnvironmentView::from(before.clone())
    };
    record(
        &mut scoped,
        &ctx,
        &project,
        environment_id,
        "project.environment.updated",
        serde_json::json!({ "environment": { "from": before.name, "to": name } }),
        serde_json::json!({ "before": EnvironmentView::from(before), "after": &after }),
        &request_id,
    )
    .await?;
    unit::commit(tx, &request_id).await?;

    Ok(axum::Json(after).into_response())
}

/// `DELETE /api/v1/environments/{id}?migrate_to={eid|none}`.
///
/// # Errors
///
/// `403`, `404`, `422 TF-PRJ-0005` when tasks carry it and no target was given,
/// `422` when the target is in another project.
pub async fn delete(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(environment_id): Path<Uuid>,
    Query(params): Query<DeleteParams>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let (doomed, project) = owned(&mut scoped, &ctx, environment_id, &request_id).await?;
    authorize(&mut scoped, &ctx, &project, &request_id).await?;

    let held = environment::count_tasks_on(&mut scoped, project.id, environment_id)
        .await
        .map_err(|error| internal(error, "counting tasks on the environment", &request_id))?;
    let target = resolve_target(
        &mut scoped,
        &project,
        &doomed,
        params.migrate_to.as_deref(),
        held,
        &request_id,
    )
    .await?;

    let moved = environment::delete_with_migration(
        &mut scoped,
        environment_id,
        target.as_ref().map(|t| t.id),
        ctx.actor.as_uuid(),
    )
    .await
    .map_err(|error| internal(error, "deleting the environment", &request_id))?;

    record(
        &mut scoped,
        &ctx,
        &project,
        environment_id,
        "project.environment.deleted",
        serde_json::json!({
            "environment": doomed.name,
            "migrated_to": target.as_ref().map(|t| t.name.clone()),
            "migrated_tasks": moved,
        }),
        serde_json::json!({
            "before": EnvironmentView::from(doomed),
            "after": serde_json::Value::Null,
            "migrated_tasks": moved,
        }),
        &request_id,
    )
    .await?;
    unit::commit(tx, &request_id).await?;

    Ok(axum::Json(serde_json::json!({ "migrated_tasks": moved })).into_response())
}

include!("environments_task.rs");
#[cfg(test)]
#[path = "environments_tests.rs"]
mod tests;
