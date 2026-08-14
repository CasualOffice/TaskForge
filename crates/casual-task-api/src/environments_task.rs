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
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

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
            &project.teams(),
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

    // The second clock never moves silently (`docs/45`). Setting an environment
    // is a promotion however it was reached, so it leaves a row here as well —
    // otherwise "when did WR-125 reach staging" would be answerable for tasks
    // promoted through one endpoint and not the other. Clearing it is not a
    // promotion: there is no environment to have reached.
    if let Some(environment) = chosen.as_ref() {
        casual_task_persistence::custody::record_promotion(
            &mut scoped,
            task.id,
            environment.id,
            ctx.actor.as_uuid(),
        )
        .await
        .map_err(|error| internal(error, "recording the promotion", &request_id))?;
    }

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
/// `PUT /api/v1/projects/{id}/environments/order` — the pipeline's sequence.
///
/// # Why the whole order rather than one move
///
/// Moving one environment changes the position of every environment it passed.
/// A per-item endpoint would make that a read-modify-write over a set two
/// people can hold at once, and the losing write leaves a pipeline with two
/// environments at the same position or a gap where one used to be. The caller
/// states what the pipeline *is*, which is atomic and idempotent.
///
/// # Errors
///
/// `400` when the ids are not exactly this project's environments, `403`
/// without `project.update`, `404` when the project is not visible.
pub async fn reorder(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Body(body): Body<ReorderRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let project = visible(&mut scoped, &ctx, project_id, &request_id).await?;
    authorize(&mut scoped, &ctx, &project, &request_id).await?;

    environment::reorder(&mut scoped, project.id, &body.environment_ids)
        .await
        .map_err(|error| match error {
            WriteError::Mismatch => ApiError::bad_request(
                codes::OUT_OF_RANGE,
                "The order must name every environment of this project exactly once",
                &request_id,
            ),
            other => write_error(other, "", &request_id),
        })?;

    let rows = environment::list(&mut scoped, project.id)
        .await
        .map_err(|error| internal(error, "listing environments", &request_id))?;

    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "project".to_owned(),
            aggregate_id: project.id,
            project_id: Some(project.id),
            event_type: "project.updated".to_owned(),
            activity_changes: serde_json::json!({
                "environments": rows.iter().map(|row| row.name.clone()).collect::<Vec<_>>(),
            }),
            audit_changes: serde_json::json!({
                "after": { "environment_order": body.environment_ids },
            }),
            payload: serde_json::json!({ "project_id": project.id }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the reorder failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok(axum::Json(serde_json::json!({
        "data": rows.into_iter().map(EnvironmentView::from).collect::<Vec<_>>(),
    }))
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
            &project.teams(),
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
        // `reorder` handles this itself with a message about the order; reaching
        // here means another caller hit it, and a generic 400 is the honest
        // answer rather than a message about a name it did not send.
        WriteError::Mismatch => ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "That set of environments does not match this project",
            request_id,
        ),
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
