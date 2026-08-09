//! `DELETE /api/v1/workflows/{wid}/statuses/{sid}?migrate_to={other_sid}`.
//!
//! # The failure this module exists to prevent
//!
//! A task on a status that no longer exists. `docs/23` §Deleting a status names
//! the two shortcuts and rejects both: "silently orphaning tasks, or lazily
//! remapping them on next read, are both rejected — they produce tasks whose
//! history does not explain their status".
//!
//! So the admin must say where the work goes, and then, in **one** transaction:
//! every task on the deleted status moves to the target, each writes an
//! activity event attributed to the acting admin with reason
//! `workflow_migration`, and the status row is removed. Either all of that
//! commits or none of it does; there is no interval in which the status is gone
//! and the tasks have not moved.
//!
//! # Why one outbox event and many activity events
//!
//! The per-task history is what `docs/23` requires, and it is written per task.
//! The *event* is one, at the workflow. `docs/25`'s fan-out writes a delivery
//! row per consumer per event, so an outbox event per migrated task would turn
//! a single administrative act into 60,000 deliveries across six consumers —
//! faithfully, which is the problem.
//!
//! # Why this is a separate file from the other status handlers
//!
//! It changes for a different reason. The others change when the shape of a
//! status does; this one changes when the rules for moving in-flight work do —
//! the threshold, the target validation, the attribution. It is also the only
//! handler here that touches `task`.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_persistence::workflow::StatusRow;
use casual_task_persistence::{Scoped, workflow_edit};
use serde::Deserialize;
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::workflows::audit::{internal, record};
use crate::workflows::guard;
use crate::workflows::statuses::status_of;
use crate::workflows::wire::{StatusDeletedView, StatusView};
use crate::{etag, unit};

/// `?migrate_to=` — the target the admin chose in the delete dialog.
#[derive(Debug, Deserialize)]
pub struct DeleteParams {
    #[serde(default)]
    pub migrate_to: Option<Uuid>,
}

/// Delete a status, migrating everything standing on it.
///
/// # Errors
///
/// - `422 TF-WFL-0006` — the status holds tasks and no `migrate_to` was given,
///   with the count in `details` so the client can say how many would move.
/// - `422 TF-WFL-0008` — `migrate_to` names a status in another workflow, or
///   the status being deleted.
/// - `422 TF-WFL-0007` — the status is the workflow's entry point.
/// - `422 TF-WFL-0010` — more tasks than one request may move (D-063).
/// - `403`, `404`, `409`, `428` as everywhere else in this module.
pub async fn delete_status(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path((workflow_id, status_id)): Path<(Uuid, Uuid)>,
    Query(params): Query<DeleteParams>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;
    let authored = guard::may_author(&mut scoped, &ctx, workflow_id, &headers, &request_id).await?;

    let doomed = status_of(&mut scoped, workflow_id, status_id, &request_id).await?;
    if doomed.is_initial {
        return Err(ApiError::unprocessable(
            codes::INITIAL_STATUS_RULE,
            "This is the workflow's initial status, and a workflow must have \
             exactly one (docs/23). Designate another status as the entry \
             point first, then delete this one",
            &request_id,
        ));
    }

    let held = workflow_edit::count_tasks_on(&mut scoped, status_id)
        .await
        .map_err(|error| internal(error, "counting tasks on the status", &request_id))?;

    let target = resolve_target(
        &mut scoped,
        workflow_id,
        &doomed,
        params.migrate_to,
        held,
        &request_id,
    )
    .await?;

    if held > workflow_edit::MIGRATION_LIMIT {
        return Err(ApiError::unprocessable(
            codes::MIGRATION_TOO_LARGE,
            "More tasks are on this status than one request may move. docs/23 \
             runs a bulk move this size as a tracked background job with \
             progress, and that job is not built yet (D-063)",
            &request_id,
        )
        .with_details(serde_json::json!({
            "task_count": held,
            "limit": workflow_edit::MIGRATION_LIMIT,
        })));
    }

    let migrated = migrate(&mut scoped, &doomed, target.as_ref(), &ctx, &request_id).await?;
    let removed_transitions = workflow_edit::delete_status(&mut scoped, status_id)
        .await
        .map_err(|error| internal(error, "deleting the status", &request_id))?;

    record(
        &mut scoped,
        &ctx,
        workflow_id,
        "workflow.status.deleted",
        serde_json::json!({
            "status": doomed.name,
            "migrated_to": target.as_ref().map(|t| t.name.clone()),
            "migrated_tasks": migrated,
            "reason": "workflow_migration",
        }),
        serde_json::json!({
            "before": StatusView::from(doomed),
            "after": serde_json::Value::Null,
            "migrated_tasks": migrated,
            "removed_transitions": removed_transitions,
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
        axum::Json(StatusDeletedView {
            workflow,
            migrated_tasks: migrated,
            removed_transitions,
        }),
    )
        .into_response())
}

/// The migration target, or the refusal `docs/23` requires when there is none.
///
/// A status holding **no** tasks needs no target, and demanding one would make
/// the ordinary case — deleting a status nobody used — impossible to express.
async fn resolve_target(
    scoped: &mut Scoped<'_>,
    workflow_id: Uuid,
    doomed: &StatusRow,
    migrate_to: Option<Uuid>,
    held: i64,
    request_id: &str,
) -> Result<Option<StatusRow>, ApiError> {
    let Some(target_id) = migrate_to else {
        if held > 0 {
            return Err(ApiError::unprocessable(
                codes::STATUS_HOLDS_TASKS,
                "This status holds tasks. Supply migrate_to naming the status \
                 they should move to — leaving them where they are would \
                 produce tasks whose history does not explain their status \
                 (docs/23)",
                request_id,
            )
            .with_details(serde_json::json!({ "task_count": held })));
        }
        return Ok(None);
    };
    if target_id == doomed.id {
        return Err(ApiError::unprocessable(
            codes::STATUS_WRONG_WORKFLOW,
            "migrate_to cannot be the status being deleted",
            request_id,
        ));
    }
    // Asked of the workflow, not of the status table: `migrate_to` is
    // caller-supplied, and a status id from another workflow would otherwise
    // move tasks onto a status their project's board does not draw.
    workflow_edit::status_in(scoped, workflow_id, target_id)
        .await
        .map_err(|error| internal(error, "reading the migration target", request_id))?
        .map(Some)
        .ok_or_else(|| {
            ApiError::unprocessable(
                codes::STATUS_WRONG_WORKFLOW,
                "migrate_to must name a status in this workflow",
                request_id,
            )
        })
}

/// Move the work, or do nothing when there is none.
async fn migrate(
    scoped: &mut Scoped<'_>,
    doomed: &StatusRow,
    target: Option<&StatusRow>,
    ctx: &Context,
    request_id: &str,
) -> Result<u64, ApiError> {
    let Some(target) = target else {
        return Ok(0);
    };
    let tasks = workflow_edit::tasks_on(scoped, doomed.id)
        .await
        .map_err(|error| internal(error, "reading the tasks to migrate", request_id))?;
    workflow_edit::migrate_tasks_off_status(scoped, doomed, target, &tasks, ctx.actor.as_uuid())
        .await
        .map_err(|error| internal(error, "migrating the tasks", request_id))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_migration_limit_is_the_one_docs_23_names() {
        // docs/23: "bulk moves above 10,000 tasks run as a tracked background
        // job with progress, not a request". If this drifts, the API either
        // refuses work it could do or does work it said it would not.
        assert_eq!(
            casual_task_persistence::workflow_edit::MIGRATION_LIMIT,
            10_000
        );
        let doc = include_str!("../../../../docs/23-WORKFLOW-AND-STATE-MACHINE.md");
        // Not the whole sentence: the document is hard-wrapped, so a phrase
        // assertion would break on a reflow rather than on a decision change.
        assert!(
            doc.contains("10,000"),
            "docs/23 no longer names 10,000 as the threshold"
        );
    }
}
