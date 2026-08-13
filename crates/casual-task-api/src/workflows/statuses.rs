//! Adding, renaming, remapping and reordering the statuses of a workflow
//! (`docs/23` §Editing a workflow).
//!
//! Deleting one is [`crate::workflows::migrate`], because it is the only
//! operation here that moves in-flight work and it changes for that reason
//! rather than for this file's.
//!
//! # Why a state remap arrives through `PATCH` like a rename does
//!
//! Because it is the same kind of act — an edit to one status — and giving it a
//! different verb would suggest otherwise. What makes it different is what it
//! does downstream: `docs/23` calls it "permitted, and **retroactive** by
//! construction", so `task.state` is recomputed for every task on the status in
//! the same transaction, and every historical report about those tasks changes.
//! That is why the response carries the count and the audit event is a distinct
//! type from an ordinary update.
//!
//! # Why the five states are validated against the enum and not a list
//!
//! `docs/23`: "state is a closed enum for the life of the API ... adding a
//! sixth state is a breaking change requiring a major API version." A `&[&str]`
//! of allowed values beside `validated_state` would be a second place to add
//! one. There is nowhere else to put a sixth, which is the point.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_model::TaskState;
use casual_task_persistence::workflow::StatusRow;
use casual_task_persistence::{Scoped, workflow, workflow_edit};
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::wire::Body;
use crate::workflows::audit::{internal, record};
use crate::workflows::guard;
use crate::workflows::wire::{
    CreateStatusRequest, PatchStatusRequest, ReorderRequest, StatusRemappedView, StatusUsageView,
    StatusView,
};
use crate::{etag, unit};

/// `GET /api/v1/workflows/{id}/statuses` — every status with its task count.
///
/// Separate from `GET /workflows/{id}` because the count is an aggregate over
/// `task`. It is cheap through `task_status_ix` (migration 0026) but not free,
/// and the board loads the workflow on every navigation to draw columns — never
/// to render a number only an administrator looks at.
///
/// Gated on `project.workflow.manage` for the same reason: this is the number
/// that tells an admin how much work a delete would move, not part of the
/// board's contract.
///
/// # Errors
///
/// `404` when the workflow is absent or invisible, `403` without the
/// permission.
pub async fn list_statuses(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(workflow_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    guard::visible(&mut scoped, workflow_id, &request_id).await?;
    guard::authorize(&mut scoped, &ctx, workflow_id, &request_id).await?;

    let (statuses, _) = workflow::load(&mut scoped, workflow_id)
        .await
        .map_err(|error| internal(error, "reading the statuses", &request_id))?;
    let counts = workflow_edit::counts_by_status(&mut scoped, workflow_id)
        .await
        .map_err(|error| internal(error, "counting tasks per status", &request_id))?;
    unit::commit(tx, &request_id).await?;

    let data: Vec<StatusUsageView> = statuses
        .into_iter()
        .map(|status| StatusUsageView {
            task_count: counts
                .iter()
                .find(|(id, _)| *id == status.id)
                .map_or(0, |(_, count)| *count),
            id: status.id,
            name: status.name,
            state: status.state,
            position: status.position,
            is_initial: status.is_initial,
        })
        .collect();
    Ok(axum::Json(serde_json::json!({ "data": data })).into_response())
}

/// `POST /api/v1/workflows/{id}/statuses` — add a status.
///
/// # Errors
///
/// `400` for a bad name or a state outside the five, `403`, `404`, `409` for a
/// duplicate name or a stale `If-Match`, `428` without one.
pub async fn create_status(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(workflow_id): Path<Uuid>,
    Body(body): Body<CreateStatusRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let name = validated_name(&body.name, &request_id)?.to_owned();
    let target = validated_state(&body.state, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let authored = guard::may_author(&mut scoped, &ctx, workflow_id, &headers, &request_id).await?;

    let status = workflow_edit::insert_status(&mut scoped, workflow_id, &name, target.as_str())
        .await
        .map_err(|error| write_error(error, &name, &request_id))?;

    let view = StatusView::from(status);
    record(
        &mut scoped,
        &ctx,
        workflow_id,
        "workflow.status.created",
        serde_json::json!({ "status": view.name, "state": view.state }),
        serde_json::json!({ "before": serde_json::Value::Null, "after": view }),
        &request_id,
    )
    .await?;

    let representation =
        guard::assemble(&mut scoped, authored.row, authored.version, &request_id).await?;
    unit::commit(tx, &request_id).await?;
    Ok((
        StatusCode::CREATED,
        [(header::ETAG, etag::tag(authored.version))],
        axum::Json(representation),
    )
        .into_response())
}

/// `PATCH /api/v1/workflows/{id}/statuses/{sid}` — rename, remap, promote.
///
/// # Errors
///
/// `400`, `403`, `404`, `409`, `422 TF-WFL-0007` for `is_initial: false`,
/// `428` without `If-Match`.
pub async fn update_status(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path((workflow_id, status_id)): Path<(Uuid, Uuid)>,
    Body(body): Body<PatchStatusRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let name = body
        .name
        .as_deref()
        .map(|name| validated_name(name, &request_id).map(ToOwned::to_owned))
        .transpose()?;
    let target = body
        .state
        .as_deref()
        .map(|state| validated_state(state, &request_id))
        .transpose()?;
    if body.is_initial == Some(false) {
        return Err(ApiError::unprocessable(
            codes::INITIAL_STATUS_RULE,
            "A workflow has exactly one initial status (docs/23). To move it, \
             set is_initial on the status that should become the entry point \
             rather than clearing it on this one",
            &request_id,
        ));
    }

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let authored = guard::may_author(&mut scoped, &ctx, workflow_id, &headers, &request_id).await?;
    let before = status_of(&mut scoped, workflow_id, status_id, &request_id).await?;

    if let Some(name) = &name {
        workflow_edit::rename_status(&mut scoped, status_id, name)
            .await
            .map_err(|error| write_error(error, name, &request_id))?;
    }
    if body.is_initial == Some(true) {
        workflow_edit::set_initial(&mut scoped, workflow_id, status_id)
            .await
            .map_err(|error| internal(error, "designating the initial status", &request_id))?;
    }

    // Last, so the count it reports is the number of tasks whose permanent
    // state actually moved rather than a number entangled with a rename in the
    // same request.
    let remapped = target.is_some_and(|t| t.as_str() != before.state);
    let mut recomputed = 0;
    if let Some(target) = target
        && remapped
    {
        recomputed = workflow_edit::remap_status_state(
            &mut scoped,
            status_id,
            target.as_str(),
            ctx.actor.as_uuid(),
        )
        .await
        .map_err(|error| internal(error, "recomputing task state", &request_id))?;
    }

    let after = status_of(&mut scoped, workflow_id, status_id, &request_id).await?;
    // `docs/23` requires a state remap to write "a prominent audit event". A
    // rename is an ordinary configuration edit; a remap silently rewrites what
    // every historical report says about the tasks on this status, so the two
    // are not one event even though they arrive through one verb.
    let event = if remapped {
        "workflow.status.remapped"
    } else {
        "workflow.status.updated"
    };
    record(
        &mut scoped,
        &ctx,
        workflow_id,
        event,
        serde_json::json!({
            "status": after.name,
            "state": { "from": before.state, "to": after.state },
            "recomputed_tasks": recomputed,
        }),
        serde_json::json!({
            "before": StatusView::from(before),
            "after": StatusView::from(after),
            "recomputed_tasks": recomputed,
        }),
        &request_id,
    )
    .await?;

    let workflow =
        guard::assemble(&mut scoped, authored.row, authored.version, &request_id).await?;
    unit::commit(tx, &request_id).await?;
    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(authored.version))],
        axum::Json(StatusRemappedView {
            workflow,
            recomputed_tasks: recomputed,
        }),
    )
        .into_response())
}

/// `POST /api/v1/workflows/{id}/statuses/order` — rewrite the column order.
///
/// # Errors
///
/// `403`, `404`, `409`, `422` when `order` is not a permutation of this
/// workflow's statuses, `428` without `If-Match`.
pub async fn reorder_statuses(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(workflow_id): Path<Uuid>,
    Body(body): Body<ReorderRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let authored = guard::may_author(&mut scoped, &ctx, workflow_id, &headers, &request_id).await?;

    let (statuses, _) = workflow::load(&mut scoped, workflow_id)
        .await
        .map_err(|error| internal(error, "reading the statuses", &request_id))?;

    // A permutation, checked before anything is written. A partial order would
    // leave the statuses it omitted holding their old positions, which can
    // collide with the ones just assigned — and then a board's column order
    // depends on which row the planner happens to return first.
    let mut submitted = body.order.clone();
    submitted.sort_unstable();
    submitted.dedup();
    let mut known: Vec<Uuid> = statuses.iter().map(|s| s.id).collect();
    known.sort_unstable();
    if submitted.len() != body.order.len() || submitted != known {
        return Err(ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "order must list every status of this workflow exactly once",
            &request_id,
        )
        .with_details(serde_json::json!({ "expected": known, "received": body.order })));
    }

    workflow_edit::reorder_statuses(&mut scoped, workflow_id, &body.order)
        .await
        .map_err(|error| internal(error, "reordering the statuses", &request_id))?;

    let names: Vec<&str> = body
        .order
        .iter()
        .filter_map(|id| statuses.iter().find(|s| s.id == *id))
        .map(|s| s.name.as_str())
        .collect();
    record(
        &mut scoped,
        &ctx,
        workflow_id,
        "workflow.statuses.reordered",
        serde_json::json!({ "order": names }),
        serde_json::json!({ "before": known, "after": body.order }),
        &request_id,
    )
    .await?;

    let representation =
        guard::assemble(&mut scoped, authored.row, authored.version, &request_id).await?;
    unit::commit(tx, &request_id).await?;
    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(authored.version))],
        axum::Json(representation),
    )
        .into_response())
}

/// One status of this workflow, or `404`.
///
/// Asked of the workflow rather than of the status table so that "belongs to a
/// different workflow" is answered by the query rather than by a comparison a
/// caller could forget.
pub(crate) async fn status_of(
    scoped: &mut Scoped<'_>,
    workflow_id: Uuid,
    status_id: Uuid,
    request_id: &str,
) -> Result<StatusRow, ApiError> {
    workflow_edit::status_in(scoped, workflow_id, status_id)
        .await
        .map_err(|error| internal(error, "reading the status", request_id))?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, request_id))
}

/// A duplicate name is the caller's to fix; anything else is a fault.
fn write_error(error: workflow_edit::WriteError, name: &str, request_id: &str) -> ApiError {
    match error {
        workflow_edit::WriteError::Duplicate => ApiError::conflict(
            codes::STATUS_NAME_TAKEN,
            "That status name is already used in this workflow",
            request_id,
        )
        .with_details(serde_json::json!({ "name": name })),
        workflow_edit::WriteError::UnknownReference => ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "A referenced row does not exist",
            request_id,
        ),
        workflow_edit::WriteError::Db(error) => internal(error, "writing the status", request_id),
    }
}

/// `docs/21` bounds every input. `workflow_status.name` has no schema
/// constraint, so the bound is here — an unbounded text field is a storage
/// amplifier, and a status name is rendered in every board column header.
fn validated_name<'a>(name: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 40 {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "A status name is 1 to 40 characters",
            request_id,
        ));
    }
    Ok(trimmed)
}

/// The five permanent states, validated against the enum itself.
fn validated_state(value: &str, request_id: &str) -> Result<TaskState, ApiError> {
    TaskState::parse(value).ok_or_else(|| {
        ApiError::bad_request(
            codes::INVALID_ENUM,
            "state must be one of the five permanent states (docs/23). A team \
             renames and reorders statuses; it never invents a state",
            request_id,
        )
        .with_details(serde_json::json!({
            "allowed": TaskState::ALL.map(|s| s.as_str()),
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_the_enum_declares_is_accepted_and_nothing_else_is() {
        for state in TaskState::ALL {
            assert_eq!(validated_state(state.as_str(), "r").ok(), Some(state));
        }
        // `Blocked` is the one a team reaches for, and docs/23 is explicit that
        // it is a STATUS whose state is ACTIVE — not a sixth state.
        for refused in ["BLOCKED", "IN_PROGRESS", "active", "", "DONE"] {
            assert_eq!(
                validated_state(refused, "r").err().map(|e| e.code()),
                Some(codes::INVALID_ENUM),
                "{refused} should not be a state"
            );
        }
    }

    #[test]
    fn a_status_name_is_bounded_at_both_ends() {
        assert_eq!(
            validated_name("  Ready for QA  ", "r").ok(),
            Some("Ready for QA")
        );
        for bad in ["", "   "] {
            assert!(validated_name(bad, "r").is_err(), "{bad:?}");
        }
        assert!(validated_name(&"x".repeat(40), "r").is_ok());
        assert!(validated_name(&"x".repeat(41), "r").is_err());
    }
}
