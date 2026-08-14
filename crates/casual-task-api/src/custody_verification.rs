/// `POST /api/v1/tasks/{id}/promotions` — it reached an environment.
///
/// A second promotion to the same environment is a *second event*, not a
/// duplicate: a redeploy to staging happened, and a log that swallowed it would
/// understate the work. So this is `POST` and it is deliberately not idempotent.
///
/// # Errors
///
/// `404` when the task is not visible, `403` without `task.update`, `422` when
/// the environment is not on the task's project.
pub async fn promote(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Body(body): Body<PromoteRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let row = visible_task(&mut scoped, &ctx, task_id, &request_id).await?;
    authorize(
        &mut scoped,
        &ctx,
        row.project_id,
        permission::TASK_UPDATE,
        &request_id,
    )
    .await?;

    let promoted = custody::promote(
        &mut scoped,
        task_id,
        body.environment_id,
        body.release_id,
        ctx.actor.as_uuid(),
    )
    .await
    .map_err(|error| refused(&error, &request_id))?;

    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: task_id,
            project_id: Some(row.project_id),
            event_type: "task.promoted".to_owned(),
            activity_changes: serde_json::json!({
                "environment_id": promoted.environment_id,
                "release_id": promoted.release_id,
            }),
            audit_changes: serde_json::json!({
                "before": { "environment_id": row.environment_id },
                "after": { "environment_id": promoted.environment_id },
            }),
            payload: serde_json::json!({
                "task_id": task_id,
                "environment_id": promoted.environment_id,
            }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the promotion failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((StatusCode::CREATED, axum::Json(promotion_view(&promoted))).into_response())
}

/// `POST /api/v1/tasks/{id}/verifications` — tested, and here is the verdict.
///
/// The verdict is recorded and **nothing else moves**. What follows — back to
/// the developer on a fail, forward on a pass — is a workflow transition the
/// caller makes next, and keeping them separate is what lets "failed twice on
/// qa" survive however many times the status has changed since.
///
/// # Errors
///
/// `404` when the task is not visible, `400` for a verdict that is not `PASS` or
/// `FAIL`, `403` without `task.transition`, `422` when the task is on no
/// environment and none was named.
pub async fn verify(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Body(body): Body<VerifyRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let verdict = match body.verdict.to_uppercase().as_str() {
        "PASS" => "PASS",
        "FAIL" => "FAIL",
        _ => {
            return Err(ApiError::bad_request(
                codes::INVALID_ENUM,
                "verdict must be PASS or FAIL",
                &request_id,
            ));
        }
    };

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let row = visible_task(&mut scoped, &ctx, task_id, &request_id).await?;
    authorize(
        &mut scoped,
        &ctx,
        row.project_id,
        permission::TASK_TRANSITION,
        &request_id,
    )
    .await?;

    // The environment is where it was tested. Defaulting to the task's current
    // one is the ordinary case — QA tests what was pushed — but a verdict
    // against *no* environment is untraceable, so that is refused rather than
    // recorded as a result nobody can reproduce.
    let Some(environment_id) = body.environment_id.or(row.environment_id) else {
        return Err(ApiError::unprocessable(
            codes::MISSING_FIELD,
            "This task is on no environment, so name the one you tested on",
            &request_id,
        ));
    };

    let recorded = custody::verify(
        &mut scoped,
        task_id,
        environment_id,
        verdict,
        ctx.actor.as_uuid(),
        body.note.as_deref(),
    )
    .await
    .map_err(|error| refused(&error, &request_id))?;

    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: task_id,
            project_id: Some(row.project_id),
            event_type: "task.verified".to_owned(),
            activity_changes: serde_json::json!({
                "verdict": recorded.verdict,
                "environment_id": recorded.environment_id,
                "note": recorded.note,
            }),
            audit_changes: serde_json::json!({
                "before": serde_json::Value::Null,
                "after": { "verdict": recorded.verdict, "environment_id": recorded.environment_id },
            }),
            payload: serde_json::json!({
                "task_id": task_id,
                "verdict": recorded.verdict,
                "environment_id": recorded.environment_id,
            }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the verification failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::CREATED,
        axum::Json(verification_view(&recorded)),
    )
        .into_response())
}

/// The task, or the same `404` an absent one gives (`docs/04`).
async fn visible_task(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    task_id: Uuid,
    request_id: &str,
) -> Result<casual_task_persistence::task::TaskRow, ApiError> {
    task::read_visible(scoped, &ctx.viewer, task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(request_id)
        })?
        .map(|(row, _key)| row)
        .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, request_id))
}

/// The permission check every command here shares.
async fn authorize(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    project_id: Uuid,
    needed: casual_task_model::Permission,
    request_id: &str,
) -> Result<(), ApiError> {
    let is_member = project::is_member(scoped, project_id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading project membership failed");
            ApiError::internal(request_id)
        })?;
    let teams = project::read_visible(scoped, &ctx.viewer, project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(request_id)
        })?
        .map(|row| row.teams())
        .unwrap_or_default();
    unit::authorized(
        ctx.authority.may_in_project(
            needed,
            ProjectId::from_uuid(project_id),
            &teams,
            &ctx.facts_in_project(is_member),
        ),
        request_id,
    )
}

/// Each refusal onto the code that names the rule it hit.
fn refused(error: &CustodyError, request_id: &str) -> ApiError {
    match error {
        CustodyError::TeamNotOnProject => ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "That team is not on this task's project. Add it to the project \
             first — a task owned by people who cannot see it is not a hand-off",
            request_id,
        ),
        CustodyError::EnvironmentNotOnProject => ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "That environment does not belong to this task's project",
            request_id,
        ),
        CustodyError::AlreadyThere => ApiError::conflict(
            codes::VERSION_CONFLICT,
            "This task already belongs to that team",
            request_id,
        ),
        CustodyError::Db(error) => {
            tracing::error!(%error, "a custody write failed");
            ApiError::internal(request_id)
        }
    }
}

fn transfer_view(row: &custody::TransferRow) -> TransferView {
    TransferView {
        id: row.id,
        from_team_id: row.from_team_id,
        to_team_id: row.to_team_id,
        moved_by: row.moved_by,
        moved_at: row.moved_at.format(&Rfc3339).unwrap_or_default(),
        note: row.note.clone(),
    }
}

fn promotion_view(row: &custody::PromotionRow) -> PromotionView {
    PromotionView {
        id: row.id,
        environment_id: row.environment_id,
        release_id: row.release_id,
        promoted_by: row.promoted_by,
        promoted_at: row.promoted_at.format(&Rfc3339).unwrap_or_default(),
    }
}

fn verification_view(row: &custody::VerificationRow) -> VerificationView {
    VerificationView {
        id: row.id,
        environment_id: row.environment_id,
        verdict: row.verdict.clone(),
        verified_by: row.verified_by,
        verified_at: row.verified_at.format(&Rfc3339).unwrap_or_default(),
        note: row.note.clone(),
    }
}

#[cfg(test)]
#[path = "custody_tests.rs"]
mod tests;
