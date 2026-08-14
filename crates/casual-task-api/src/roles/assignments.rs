/// `POST /api/v1/role-assignments` — grant a role.
///
/// # Errors
///
/// `400` for a bad scope or principal, `404` for an unknown role, `422` for any
/// ceiling refusal, `500` on a database failure.
pub async fn assign(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Body(body): Body<AssignRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let role = role_edit::read(&mut scoped, body.role_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the role failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, &request_id))?;

    let scope_id = body.scope_id.unwrap_or_else(|| ctx.workspace.as_uuid());
    let scope = parse_scope(&body.scope_type, scope_id, &request_id)?;
    let principal = parse_principal(&body.principal_type, body.principal_id, &request_id)?;
    let permissions = parse_permissions(&role.permissions, &request_id)?;

    // Every ceiling, in one call. `docs/04` controls 1, 2, 3 and 5 live in
    // `casual-task-authz` and are tested without a database; control 4 is
    // migration 0021's trigger and fires inside this transaction.
    let proposed = ProposedAssignment {
        principal,
        scope,
        role_permissions: permissions,
    };
    ctx.authority
        .may_assign(&proposed, &scopes_for(ctx.workspace, scope), &facts(&ctx))
        .map_err(|refusal| refused(&refusal, &request_id))?;

    let row = role_edit::assign(
        &mut scoped,
        &body.principal_type,
        body.principal_id,
        body.role_id,
        &body.scope_type,
        scope_id,
        ctx.actor.as_uuid(),
    )
    .await
    .map_err(|error| {
        if matches!(&error, sqlx::Error::Database(db) if db.is_foreign_key_violation()) {
            return ApiError::unprocessable(
                codes::REFERENCE_NOT_FOUND,
                "the principal or scope does not name something in this workspace",
                &request_id,
            );
        }
        tracing::error!(%error, "creating the grant failed");
        ApiError::internal(&request_id)
    })?;

    let rendered = assignment_view(&row);
    let payload = serde_json::to_value(&rendered).unwrap_or(serde_json::Value::Null);
    record_assignment(
        &mut scoped,
        &ctx,
        rendered.id,
        "role.assigned",
        serde_json::json!({ "before": null, "after": payload }),
        payload.clone(),
        &request_id,
    )
    .await?;
    bump_epoch(&mut scoped, &request_id).await?;
    unit::commit(tx, &request_id).await?;

    Ok((StatusCode::CREATED, axum::Json(payload)).into_response())
}

/// `DELETE /api/v1/role-assignments/{id}` — revoke a grant.
///
/// # Errors
///
/// `404` if it is not in this workspace, `422` when it is the last grant
/// carrying `workspace.owner`, `500` on a database failure.
pub async fn revoke(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(assignment_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let existing = role_edit::read_assignment(&mut scoped, assignment_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the grant failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, &request_id))?;

    let role = role_edit::read(&mut scoped, existing.role_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the role failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, &request_id))?;

    // Revoking is bounded by the same ceilings as granting: an actor who could
    // not have created this grant must not be able to remove it either, or the
    // scope ceiling would be a one-way door.
    let scope = parse_scope(&existing.scope_type, existing.scope_id, &request_id)?;
    let proposed = ProposedAssignment {
        principal: parse_principal(&existing.principal_type, existing.principal_id, &request_id)?,
        scope,
        role_permissions: parse_permissions(&role.permissions, &request_id)?,
    };
    ctx.authority
        .may_assign(&proposed, &scopes_for(ctx.workspace, scope), &facts(&ctx))
        .map_err(|refusal| refused(&refusal, &request_id))?;

    let before = serde_json::to_value(assignment_view(&existing)).unwrap_or_default();
    let removed = role_edit::revoke(&mut scoped, assignment_id)
        .await
        .map_err(|error| {
            // Migration 0021's trigger refuses the last owner. It raises
            // `restrict_violation` with the code in a HINT, so without this the
            // most important refusal in the model would surface as a 500.
            if matches!(&error, sqlx::Error::Database(db) if db.code().as_deref() == Some("23001"))
            {
                return ApiError::unprocessable(
                    codes::LAST_OWNER,
                    "That is the last grant carrying workspace.owner",
                    &request_id,
                );
            }
            tracing::error!(%error, "revoking the grant failed");
            ApiError::internal(&request_id)
        })?;
    if !removed {
        return Err(ApiError::missing(codes::NOT_FOUND, &request_id));
    }

    record_assignment(
        &mut scoped,
        &ctx,
        assignment_id,
        "role.revoked",
        serde_json::json!({ "before": before, "after": null }),
        serde_json::json!({ "assignment_id": assignment_id }),
        &request_id,
    )
    .await?;
    bump_epoch(&mut scoped, &request_id).await?;
    unit::commit(tx, &request_id).await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn assignment_view(row: &casual_task_persistence::role_edit::AssignmentRow) -> AssignmentView {
    AssignmentView {
        id: row.id,
        principal_type: row.principal_type.clone(),
        principal_id: row.principal_id,
        role_id: row.role_id,
        scope_type: row.scope_type.clone(),
        scope_id: row.scope_id,
        granted_by: row.granted_by,
        granted_at: row.granted_at.format(&Rfc3339).unwrap_or_default(),
    }
}

fn facts(ctx: &Context) -> casual_task_app::ResourceFacts {
    casual_task_app::ResourceFacts {
        actor_is_guest: ctx.is_guest,
        ..casual_task_app::ResourceFacts::default()
    }
}

/// The scope chain the ceiling resolves the actor's own permissions in.
fn scopes_for(workspace: WorkspaceId, scope: Scope) -> ResourceScopes {
    match scope {
        Scope::Workspace(_) => ResourceScopes::workspace(workspace),
        Scope::Team(team) => ResourceScopes::workspace(workspace).in_team(team),
        Scope::Project(project) => ResourceScopes::project(workspace, project),
        Scope::Environment(environment) => {
            ResourceScopes::workspace(workspace).in_environment(environment)
        }
    }
}

fn parse_scope(scope_type: &str, id: Uuid, request_id: &str) -> Result<Scope, ApiError> {
    match scope_type {
        "WORKSPACE" => Ok(Scope::Workspace(WorkspaceId::from_uuid(id))),
        "TEAM" => Ok(Scope::Team(TeamId::from_uuid(id))),
        "PROJECT" => Ok(Scope::Project(ProjectId::from_uuid(id))),
        "ENVIRONMENT" => Ok(Scope::Environment(EnvironmentId::from_uuid(id))),
        // ADR-005 excludes task scope and the enum has no other member, so an
        // unrecognised value is a scope this build cannot reason about.
        other => Err(ApiError::bad_request(
            codes::INVALID_ENUM,
            format!("`{other}` is not a scope type"),
            request_id,
        )),
    }
}

fn parse_principal(
    principal_type: &str,
    id: Uuid,
    request_id: &str,
) -> Result<Principal, ApiError> {
    match principal_type {
        "USER" => Ok(Principal::User(UserId::from_uuid(id))),
        "TEAM" => Ok(Principal::Team(TeamId::from_uuid(id))),
        "SERVICE_ACCOUNT" => Ok(Principal::ServiceAccount(UserId::from_uuid(id))),
        other => Err(ApiError::bad_request(
            codes::INVALID_ENUM,
            format!("`{other}` is not a principal type"),
            request_id,
        )),
    }
}

/// Control 1, applied to a set of permissions at one scope.
fn ceiling_over(
    ctx: &Context,
    wanted: &[Permission],
    scope: Scope,
    request_id: &str,
) -> Result<(), ApiError> {
    let proposed = ProposedAssignment {
        // The author, so control 5 reads as self-elevation when it applies —
        // authoring a role you could not grant yourself is the same escalation
        // wearing a different hat.
        principal: Principal::User(ctx.actor),
        scope,
        role_permissions: wanted.to_vec(),
    };
    ctx.authority
        .may_assign(&proposed, &scopes_for(ctx.workspace, scope), &facts(ctx))
        .map_err(|refusal| refused(&refusal, request_id))
}

/// Any change to who may do what bumps the epoch, in the same transaction.
///
/// `docs/04` defines that counter as the thing an open SSE stream revalidates
/// against, so an unchanged epoch is proof rather than a guess (C-015).
async fn bump_epoch(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    request_id: &str,
) -> Result<(), ApiError> {
    casual_task_persistence::workspace::bump_authz_epoch(scoped)
        .await
        .map_err(|error| {
            tracing::error!(%error, "bumping the authz epoch failed");
            ApiError::internal(request_id)
        })?;
    Ok(())
}

async fn record_assignment(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    aggregate: Uuid,
    event_type: &str,
    audit_changes: serde_json::Value,
    payload: serde_json::Value,
    request_id: &str,
) -> Result<(), ApiError> {
    UnitOfWork::record(
        scoped,
        &Change {
            aggregate_type: "role_assignment".to_owned(),
            aggregate_id: aggregate,
            project_id: None,
            event_type: event_type.to_owned(),
            activity_changes: serde_json::json!({}),
            audit_changes,
            payload,
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the grant change failed");
        ApiError::internal(request_id)
    })?;
    Ok(())
}
