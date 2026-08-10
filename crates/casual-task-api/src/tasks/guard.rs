//! Visibility, authority, and the facts a decision needs.
//!
//! Everything here answers "may this actor do this, to this row" BEFORE a
//! handler writes anything. It is one module so that the answer cannot be
//! assembled two different ways in two handlers — which is how one endpoint
//! ends up more permissive than its neighbour.

use casual_task_model::ProjectId;
use casual_task_persistence::task::TaskRow;
use casual_task_persistence::{project, task};
use uuid::Uuid;

use super::*;
use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::unit;
use crate::wire::{self};

/// The accessible project set is bounded.
///
/// `docs/26` §Permission filtering: past a few hundred projects for one actor
/// "the array stops being efficient" and the set is materialized into a
/// `project_access` table instead. That table does not exist yet, so the bound
/// is enforced here — a truncated set produces a visibly missing project rather
/// than a query that quietly degrades.
pub(crate) const MAX_ACCESSIBLE_PROJECTS: u32 = 500;

/// Read a task the caller may see, or refuse with `404`.
///
/// Absent, deleted, and in-a-project-you-cannot-see are one answer (`docs/04`).
pub(crate) async fn visible(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    id: Uuid,
    request_id: &str,
) -> Result<(TaskRow, String), ApiError> {
    task::read_visible(scoped, &ctx.viewer, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, request_id))
}

/// The stored `task_type` as the model's enum.
///
/// `None` for anything unrecognised rather than a default: a type constraint is
/// satisfied only by a type it lists, so an unknown value denies. A `TASK`
/// default would silently *grant* against a value nobody understood.
pub(crate) fn task_type_of(stored: &str) -> Option<casual_task_model::TaskType> {
    match stored {
        "TASK" => Some(casual_task_model::TaskType::Task),
        "BUG" => Some(casual_task_model::TaskType::Bug),
        "FEATURE" => Some(casual_task_model::TaskType::Feature),
        "INCIDENT" => Some(casual_task_model::TaskType::Incident),
        "REQUEST" => Some(casual_task_model::TaskType::Request),
        _ => None,
    }
}

/// The constraint inputs for a decision about **this task**.
///
/// `Context::facts_in_project` is the project-level form and leaves the
/// task-level facts empty. `assignee_is_actor` and `reporter_is_actor` are the
/// two constraints in the closed set that need them, and a decision made
/// without them silently denies someone their own task.
pub(crate) async fn facts_for(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    row: &TaskRow,
    request_id: &str,
) -> Result<casual_task_app::ResourceFacts, ApiError> {
    let internal = |error: sqlx::Error, what: &'static str| {
        tracing::error!(%error, what, "gathering task facts failed");
        ApiError::internal(request_id)
    };
    let is_member = project::is_member(scoped, row.project_id, ctx.actor.as_uuid())
        .await
        .map_err(|e| internal(e, "membership"))?;
    let assignees = task::assignees(scoped, row.id)
        .await
        .map_err(|e| internal(e, "assignees"))?;

    Ok(casual_task_app::ResourceFacts {
        assignees: assignees
            .into_iter()
            .map(casual_task_model::UserId::from_uuid)
            .collect(),
        reporter: Some(casual_task_model::UserId::from_uuid(row.reporter_id)),
        actor_is_project_member: is_member,
        environment: row
            .environment_id
            .map(casual_task_model::EnvironmentId::from_uuid),
        // The task's own type, so a grant narrowed to `TaskTypeIn` decides
        // against what this task *is* rather than against nothing. Parsed
        // leniently: an unrecognised value leaves it unset, which satisfies no
        // type constraint — the safe direction (`docs/45` §Permissions).
        task_type: task_type_of(&row.task_type),
        actor_is_guest: ctx.is_guest,
    })
}

/// Resolve the task's facts and answer one permission question about it.
pub(crate) async fn authorize_on_task(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    row: &TaskRow,
    wanted: casual_task_model::Permission,
    request_id: &str,
) -> Result<(), ApiError> {
    let facts = facts_for(scoped, ctx, row, request_id).await?;
    let team = project::read_visible(scoped, &ctx.viewer, row.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(request_id)
        })?
        .map(|p| p.teams())
        .unwrap_or_default();
    unit::authorized(
        ctx.authority
            .may_in_project(wanted, ProjectId::from_uuid(row.project_id), &team, &facts),
        request_id,
    )
}

/// Assemble the project's workflow, or report an operational fault.
pub(crate) async fn load_workflow(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    workflow_id: Uuid,
    request_id: &str,
) -> Result<casual_task_app::Workflow, ApiError> {
    let (statuses, transitions) = casual_task_persistence::workflow::load(scoped, workflow_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "loading the workflow failed");
            ApiError::internal(request_id)
        })?;
    casual_task_app::compose(
        &statuses
            .iter()
            .map(|s| casual_task_app::StoredStatus {
                id: s.id,
                name: s.name.clone(),
                state: s.state.clone(),
                is_initial: s.is_initial,
            })
            .collect::<Vec<_>>(),
        &transitions
            .iter()
            .map(|t| casual_task_app::StoredTransition {
                id: t.id,
                from: t.from,
                to: t.to,
                required_permission: t.required_permission.clone(),
                required_fields: t.required_fields.clone(),
                ignore_dependencies: t.ignore_dependencies,
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|error| {
        // A workflow this build cannot assemble is an operational fault, not a
        // bad request: nothing the caller sent produced it.
        tracing::error!(?error, "the project's workflow could not be assembled");
        ApiError::internal(request_id)
    })
}

/// The `409` body. `docs/24`: the loser is told what it lost to, so the client
/// can show "Sarah changed status and assignee" and offer a merge.
pub(crate) fn conflict(
    current: &TaskRow,
    project_key: &str,
    your_version: i64,
    request_id: &str,
) -> ApiError {
    ApiError::conflict(
        codes::VERSION_CONFLICT,
        "This task was updated by someone else",
        request_id,
    )
    .with_details(serde_json::json!({
        "your_version": your_version,
        "current_version": current.version,
        "changed_by": current.updated_by,
        "changed_at": wire::timestamp(current.updated_at),
        "current": view(current, project_key),
    }))
}

/// Map a state-machine refusal onto its documented code (`docs/20`).
///
/// `MissingPermission` is a **403**, not a 422: the caller may not perform this
/// transition, which is a different answer from "this transition is not
/// possible".
pub(crate) fn rejected(rejection: &casual_task_app::Rejection, request_id: &str) -> ApiError {
    use casual_task_app::Rejection;
    match rejection {
        Rejection::NoSuchTransition => ApiError::unprocessable(
            codes::NO_SUCH_TRANSITION,
            "That status is not reachable from the task's current status",
            request_id,
        ),
        Rejection::MissingPermission(permission) => {
            ApiError::denied(codes::TRANSITION_PERMISSION, request_id)
                .with_details(serde_json::json!({ "required_permission": permission.as_str() }))
        }
        Rejection::MissingFields(fields) => ApiError::unprocessable(
            codes::TRANSITION_FIELDS_MISSING,
            "Required fields are missing for that status",
            request_id,
        )
        // docs/05: `details` returns ALL violations at once, never the first.
        .with_details(serde_json::json!({ "missing_fields": fields })),
        Rejection::BlockedBy(blockers) => ApiError::unprocessable(
            codes::BLOCKED_BY_DEPENDENCIES,
            "This task is blocked by unresolved dependencies",
            request_id,
        )
        .with_details(serde_json::json!({
            "blocked_by": blockers.iter().map(|b| b.as_uuid()).collect::<Vec<_>>(),
        })),
    }
}
