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
use casual_task_model::{ProjectId, TeamId, permission};
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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;
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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;
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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;
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

/// `PUT /api/v1/tasks/{id}/environment` — set or clear a task's environment.
///
/// A route of its own rather than a field on `PATCH /tasks/{id}`, for the
/// reason `casual_task_persistence::environment::set_on_task` gives: the target
/// has to be checked against the task's own project, and folding a cross-table
/// check into the generic patch would make every title edit load a project.
///
/// Gated on `task.update` in the task's project, not on `project.update`:
/// labelling a task is ordinary work, and only *authoring* the vocabulary is
/// administration.
///
/// # Errors
///
/// `403`, `404`, `409`, `422` when the environment is in another project,
/// `428` without `If-Match`.
pub async fn set_on_task(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Body(body): Body<SetOnTaskRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let expected = etag::if_match(&headers, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    // `read_visible` returns the row and its project key; the key is the task's
    // human identifier and is not part of this answer.
    let (task, _key) =
        casual_task_persistence::task::read_visible(&mut scoped, &ctx.viewer, task_id)
            .await
            .map_err(|error| internal(error, "reading the task", &request_id))?
            .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, &request_id))?;
    let project = visible(&mut scoped, &ctx, task.project_id, &request_id).await?;
    let is_member = project::is_member(&mut scoped, project.id, ctx.actor.as_uuid())
        .await
        .map_err(|error| internal(error, "reading project membership", &request_id))?;
    unit::authorized(
        ctx.authority.may_in_project(
            permission::TASK_UPDATE,
            ProjectId::from_uuid(project.id),
            project.team_id.map(TeamId::from_uuid),
            &ctx.facts_in_project(is_member),
        ),
        &request_id,
    )?;

    let Some(requested) = body.environment_id else {
        return Err(ApiError::bad_request(
            codes::MISSING_FIELD,
            "environment_id is required. Send null to clear it — an absent \
             field would make a client bug look like an intention",
            &request_id,
        ));
    };
    let chosen = match requested {
        None => None,
        Some(id) => Some(in_project(&mut scoped, &project, id, &request_id).await?),
    };

    let version = environment::set_on_task(
        &mut scoped,
        task.id,
        chosen.as_ref().map(|e| e.id),
        expected,
        ctx.actor.as_uuid(),
    )
    .await
    .map_err(|error| internal(error, "setting the task environment", &request_id))?
    .ok_or_else(|| {
        ApiError::conflict(
            codes::VERSION_CONFLICT,
            "This task was updated by someone else",
            &request_id,
        )
        .with_details(serde_json::json!({ "your_version": expected }))
    })?;

    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: task.id,
            project_id: Some(project.id),
            event_type: "task.updated".to_owned(),
            activity_changes: serde_json::json!({
                "environment": chosen.as_ref().map(|e| e.name.clone()),
            }),
            audit_changes: serde_json::json!({
                "before": { "environment_id": task.environment_id },
                "after": { "environment_id": chosen.as_ref().map(|e| e.id) },
            }),
            payload: serde_json::json!({
                "id": task.id,
                "environment_id": chosen.as_ref().map(|e| e.id),
                "version": version,
            }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| internal(error, "recording the environment change", &request_id))?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(version))],
        axum::Json(serde_json::json!({
            "id": task.id,
            "environment_id": chosen.map(|e| e.id),
            "version": version,
        })),
    )
        .into_response())
}

/// The migration target, or the refusal `TF-PRJ-0005` describes.
async fn resolve_target(
    scoped: &mut Scoped<'_>,
    project: &ProjectRow,
    doomed: &EnvironmentRow,
    migrate_to: Option<&str>,
    held: i64,
    request_id: &str,
) -> Result<Option<EnvironmentRow>, ApiError> {
    match migrate_to {
        // `none` is the explicit "clear it": `task.environment_id` is nullable,
        // so this is a real answer — but one the admin has to give.
        Some("none") => Ok(None),
        Some(raw) => {
            let id = raw.parse::<Uuid>().map_err(|_| {
                ApiError::bad_request(
                    codes::INVALID_ENUM,
                    "migrate_to must be an environment id, or `none` to clear \
                     the environment on every task that carries this one",
                    request_id,
                )
            })?;
            if id == doomed.id {
                return Err(ApiError::unprocessable(
                    codes::ENVIRONMENT_IN_USE,
                    "migrate_to cannot be the environment being deleted",
                    request_id,
                ));
            }
            in_project(scoped, project, id, request_id).await.map(Some)
        }
        None if held > 0 => Err(ApiError::unprocessable(
            codes::ENVIRONMENT_IN_USE,
            "Tasks carry this environment. Supply migrate_to naming another \
             environment, or `none` to clear it on all of them",
            request_id,
        )
        .with_details(serde_json::json!({ "task_count": held }))),
        None => Ok(None),
    }
}

/// One environment of this project, or the `422` that says whose it is.
async fn in_project(
    scoped: &mut Scoped<'_>,
    project: &ProjectRow,
    environment_id: Uuid,
    request_id: &str,
) -> Result<EnvironmentRow, ApiError> {
    let row = environment::read(scoped, environment_id)
        .await
        .map_err(|error| internal(error, "reading the environment", request_id))?;
    match row {
        Some(row) if row.project_id == project.id => Ok(row),
        // `TF-VAL-0008` — "referenced entity belongs to another project" — is
        // the precise answer, and it is not a 404: the caller can see this
        // project, so the fact that the id is not one of *its* environments
        // discloses nothing they could not learn from the list endpoint.
        Some(_) => Err(ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "That environment belongs to a different project",
            request_id,
        )),
        None => Err(ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "That environment does not exist",
            request_id,
        )),
    }
}

/// An environment and the project that owns it, or `404`.
async fn owned(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    environment_id: Uuid,
    request_id: &str,
) -> Result<(EnvironmentRow, ProjectRow), ApiError> {
    let row = environment::read(scoped, environment_id)
        .await
        .map_err(|error| internal(error, "reading the environment", request_id))?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, request_id))?;
    let project = visible(scoped, ctx, row.project_id, request_id).await?;
    Ok((row, project))
}

/// A project the caller may see, or `404` — never disambiguated (`docs/04`).
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

/// `project.update` in this project — see the module docs for why not
/// `project.workflow.manage`.
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
            permission::PROJECT_UPDATE,
            ProjectId::from_uuid(project.id),
            project.team_id.map(TeamId::from_uuid),
            &ctx.facts_in_project(is_member),
        ),
        request_id,
    )
}

#[allow(clippy::too_many_arguments)] // every one of them is a distinct fact the event needs
async fn record(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    project: &ProjectRow,
    environment_id: Uuid,
    event_type: &str,
    activity: serde_json::Value,
    audit: serde_json::Value,
    request_id: &str,
) -> Result<(), ApiError> {
    UnitOfWork::record(
        scoped,
        &Change {
            aggregate_type: "project_environment".to_owned(),
            aggregate_id: environment_id,
            // Unlike a workflow, an environment belongs to exactly one project,
            // so the fan-out scope is not a guess.
            project_id: Some(project.id),
            event_type: event_type.to_owned(),
            activity_changes: activity.clone(),
            audit_changes: audit,
            payload: activity,
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map(|_| ())
    .map_err(|error| internal(error, "recording the environment change", request_id))
}

fn internal(error: sqlx::Error, what: &'static str, request_id: &str) -> ApiError {
    tracing::error!(%error, what, "an environment write failed");
    ApiError::internal(request_id)
}

fn write_error(error: WriteError, name: &str, request_id: &str) -> ApiError {
    match error {
        WriteError::Duplicate => ApiError::conflict(
            codes::ENVIRONMENT_NAME_TAKEN,
            "That environment name is already used in this project",
            request_id,
        )
        .with_details(serde_json::json!({ "name": name })),
        WriteError::Db(error) => internal(error, "writing the environment", request_id),
    }
}

/// `docs/21` bounds every input. `project_environment.name` has no schema
/// constraint, and an environment name appears in a filter chip and a grant
/// constraint, so it is short by design.
fn validated_name<'a>(name: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 40 {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "An environment name is 1 to 40 characters",
            request_id,
        ));
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_environment_name_is_bounded_at_both_ends() {
        assert_eq!(validated_name("  staging  ", "r").ok(), Some("staging"));
        for bad in ["", "   "] {
            assert!(validated_name(bad, "r").is_err(), "{bad:?}");
        }
        assert!(validated_name(&"x".repeat(40), "r").is_ok());
        assert!(validated_name(&"x".repeat(41), "r").is_err());
    }

    #[test]
    fn an_unknown_field_does_not_deserialize() {
        assert!(serde_json::from_str::<CreateRequest>(r#"{"nmae":"staging"}"#).is_err());
        assert!(serde_json::from_str::<SetOnTaskRequest>(r#"{"env":null}"#).is_err());
    }

    #[test]
    fn clearing_a_task_environment_is_spelled_and_not_implied() {
        // An absent field must stay DISTINGUISHABLE from an explicit null, so
        // the handler can refuse the first and honour the second.
        assert_eq!(
            serde_json::from_str::<SetOnTaskRequest>("{}")
                .expect("valid json")
                .environment_id,
            None,
            "absent, which the handler refuses with TF-VAL-0003"
        );
        let cleared: SetOnTaskRequest =
            serde_json::from_str(r#"{"environment_id":null}"#).expect("valid");
        assert_eq!(
            cleared.environment_id,
            Some(None),
            "present and null: clear it"
        );
        let set: SetOnTaskRequest =
            serde_json::from_str(r#"{"environment_id":"018f2c00-0000-7000-8000-000000000000"}"#)
                .expect("valid");
        assert!(matches!(set.environment_id, Some(Some(_))));
    }
}
