/// `GET /api/v1/tasks/{id}/assignees` — who is on this task.
///
/// # Why this read had to exist
///
/// The assignee set was write-only: `POST` returned it and `DELETE` did not, so
/// the only way a client could learn who was on a task was to assign someone.
/// A task surface cannot show "who is working on this" — the second question
/// anyone asks after "what is it?" — without it, and `TaskView` deliberately
/// carries no `assignees` field: a 200-card board would fetch 200 assignee sets
/// it does not draw, which is the N+1 `docs/04` §The list problem forbids.
///
/// # Ids, not names
///
/// The same shape `POST` returns. A client resolves ids through the workspace
/// member directory it already holds (`GET /workspaces/{id}/members`), and a
/// second source of display names here would be a second thing to keep in step
/// with anonymization (ADR-026).
///
/// # Errors
///
/// `404` when the task is not visible, `403` without `task.read`.
pub async fn assignees(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // Visibility is the whole check. Seeing a task and not who is on it is not
    // a distinction `docs/04` draws — there is no `task.assignee.read` in the
    // closed registry, and inventing one would settle a permission question in
    // a handler.
    let (current, _) = visible(&mut scoped, &ctx, id, &request_id).await?;
    let assignees = task::assignees(&mut scoped, current.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading assignees failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    Ok(axum::Json(serde_json::json!({ "assignees": assignees })).into_response())
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
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

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
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

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
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

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
