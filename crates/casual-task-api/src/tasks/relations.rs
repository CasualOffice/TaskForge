//! Transitions, assignees and tags.
//!
//! Everything that changes a task's relationships or its position in the
//! workflow rather than its own fields. These share the property that the
//! state machine or another aggregate decides whether they are legal, so they
//! delegate rather than deciding themselves.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_model::{ProjectId, permission};
use casual_task_persistence::{Change, UnitOfWork, project, task};
use uuid::Uuid;

use super::*;
use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::wire::Body;
use crate::{etag, unit};

/// `POST /api/v1/tasks/{id}/transitions` — the only door to a status change.
///
/// # The validation order is `docs/23`'s, and it is observable
///
/// Steps 1–3 are here (readable → `404`, version → `409`, `task.transition` →
/// `403`); steps 4–7 are `casual_task_app::Workflow::validate`, which
/// returns the **first** failure as a `Rejection` and is where the order between
/// them is enforced. This handler does not re-derive any of those rules — it
/// supplies the facts and maps the refusal onto its documented code.
///
/// **Step 8 — plugin `validation.transition` hooks — is not implemented.** It
/// needs the plugin runtime (Phase 3, `docs/34`), and nothing here fakes it.
///
/// # What `fields` does and does not do
///
/// It satisfies step 6, and its values are then discarded. Storing them needs
/// custom-field value storage, which is **D-033** and deliberately deferred
/// until Phase 3. A transition whose workflow requires a field therefore
/// validates correctly and records nothing; the default workflow requires no
/// fields, so no path in the product reaches that gap today.
///
/// # Errors
///
/// `404`, `409`, `428`, `403`, or one of `TF-WFL-0002`..`TF-WFL-0005`.
pub async fn transition(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Body(body): Body<TransitionRequestBody>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let expected = etag::if_match(&headers, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;
    let done = apply_transition(&mut scoped, &ctx, id, expected, &body, &request_id).await?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(done.version))],
        axum::Json(done.view),
    )
        .into_response())
}

/// What a completed transition leaves behind.
///
/// The version is carried beside the view because the caller needs it for the
/// `ETag` and, in the bulk case, so a client can build the inverse call for a
/// task that succeeded next to ones that refused.
pub(crate) struct Transitioned {
    pub(crate) view: TaskView,
    pub(crate) version: i64,
    /// The status the task held before this call. The moved row no longer knows
    /// it, and it is what an inverse transition needs.
    pub(crate) from_status_id: Uuid,
}

/// One task's transition, inside a transaction the caller owns and commits.
///
/// Split out of [`transition`] so `POST /api/v1/tasks/bulk` runs *this* — the
/// rules, in this order — rather than a second implementation of them that can
/// drift. The caller owns the transaction because bulk gives every task its
/// own: `docs/05` makes partial success the contract, and one task's refusal
/// must not roll back the ones that already succeeded.
///
/// # Errors
///
/// `404`, `409`, `403`, or one of `TF-WFL-0002`..`TF-WFL-0005`. Not `428` —
/// the expected version is already resolved by the caller.
#[allow(clippy::too_many_lines)] // one command, read top to bottom; the ORDER is the specification
pub(crate) async fn apply_transition(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    id: Uuid,
    expected: i64,
    body: &TransitionRequestBody,
    request_id: &str,
) -> Result<Transitioned, ApiError> {
    if let Some(comment) = body.comment.as_deref()
        && comment.len() > 65_536
    {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "comment must be at most 65536 bytes",
            request_id,
        ));
    }

    // 1. Readable.
    let (current, project_key) = visible(scoped, ctx, id, request_id).await?;
    // 2. Version.
    if current.version != expected {
        return Err(conflict(&current, &project_key, expected, request_id));
    }
    // 3. task.transition on the project.
    let facts = facts_for(scoped, ctx, &current, request_id).await?;
    let project_row = project::read_visible(scoped, &ctx.viewer, current.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, request_id))?;
    let team = project_row.teams();
    let project = ProjectId::from_uuid(current.project_id);
    unit::authorized(
        ctx.authority
            .may_in_project(permission::TASK_TRANSITION, project, &team, &facts),
        request_id,
    )?;

    // A move to the status the task already occupies is a no-op that returns
    // 200 and writes nothing — `docs/23` §Concurrency: "this makes client
    // retries safe without an idempotency key". Answered before the workflow is
    // loaded, so a retry costs nothing.
    if body.to_status_id == current.status_id {
        return Ok(Transitioned {
            view: view(&current, &project_key),
            version: current.version,
            from_status_id: current.status_id,
        });
    }

    let workflow = load_workflow(scoped, project_row.workflow_id, request_id).await?;

    // Steps 4–7, in `casual-task-workflow`. Everything it needs is resolved
    // here and passed in; it reaches nothing itself, which is what lets the
    // whole state machine be tested with no database.
    let held: Vec<casual_task_model::Permission> = permission::ALL
        .iter()
        .copied()
        .filter(|p| {
            ctx.authority
                .may_in_project(*p, project, &team, &facts)
                .is_allowed()
        })
        .collect();
    let may_override = held.contains(&permission::TASK_DEPENDENCY_OVERRIDE);
    let blockers = task::unresolved_blockers(scoped, &ctx.viewer, current.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading blocking dependencies failed");
            ApiError::internal(request_id)
        })?;

    let request = casual_task_app::TransitionRequest {
        // A field present but empty does not satisfy a requirement: docs/23
        // step 6 says "present and non-empty".
        provided_fields: body
            .fields
            .iter()
            .filter(|(_, value)| !is_empty_value(value))
            .map(|(name, _)| name.clone())
            .collect(),
        unresolved_blockers: blockers
            .iter()
            .map(|b| casual_task_model::TaskId::from_uuid(*b))
            .collect(),
        may_override_dependencies: may_override,
        held_permissions: held,
    };

    let valid = workflow
        .validate(
            casual_task_model::StatusId::from_uuid(current.status_id),
            casual_task_model::StatusId::from_uuid(body.to_status_id),
            &request,
        )
        .map_err(|rejection| rejected(&rejection, request_id))?;

    let from_status = workflow
        .status(casual_task_model::StatusId::from_uuid(current.status_id))
        .map_or("", |s| s.name.as_str())
        .to_owned();
    let to_status = workflow
        .status(valid.to_status)
        .map_or("", |s| s.name.as_str())
        .to_owned();

    let moved = task::transition(
        scoped,
        current.id,
        expected,
        valid.to_status.as_uuid(),
        state_wire(valid.to_state),
        ctx.actor.as_uuid(),
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "the transition failed");
        ApiError::internal(request_id)
    })?;
    let Some(moved) = moved else {
        let (now, key) = visible(scoped, ctx, id, request_id).await?;
        return Err(conflict(&now, &key, expected, request_id));
    };

    if let Some(comment) = body
        .comment
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        task::insert_comment(scoped, moved.id, ctx.actor.as_uuid(), comment)
            .await
            .map_err(|error| {
                tracing::error!(%error, "writing the transition comment failed");
                ApiError::internal(request_id)
            })?;
    }

    // docs/23 §Closing and reopening: leaving a terminal state writes a
    // DISTINCT event, "because 'how often does work come back?' is a question
    // teams need answered and a generic status-change event cannot serve".
    let was_terminal = matches!(current.state.as_str(), "COMPLETED" | "CANCELED");
    let is_terminal = matches!(state_wire(valid.to_state), "COMPLETED" | "CANCELED");
    let event_type = if was_terminal && !is_terminal {
        "task.reopened"
    } else {
        "task.status.changed"
    };

    let after_view = view(&moved, &project_key);
    let payload = serde_json::json!(after_view);
    UnitOfWork::record(
        scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: moved.id,
            project_id: Some(moved.project_id),
            event_type: event_type.to_owned(),
            // Display values, not ids (`docs/25`): the status NAMES, because
            // either may be renamed or deleted before anyone reads this.
            activity_changes: serde_json::json!({
                "status": { "from": from_status, "to": to_status },
            }),
            audit_changes: serde_json::json!({
                "before": { "status_id": current.status_id, "state": current.state },
                "after":  { "status_id": moved.status_id,   "state": moved.state },
            }),
            payload: payload.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the transition failed");
        ApiError::internal(request_id)
    })?;

    Ok(Transitioned {
        view: after_view,
        version: moved.version,
        from_status_id: current.status_id,
    })
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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

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
