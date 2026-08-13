//! Authoring the edges of a workflow (`docs/23`).
//!
//! # Why removing an edge is free and removing a status is not
//!
//! `docs/23` §Removing a transition: "allowed freely — it constrains future
//! moves only. Tasks are never *in* a transition, only in a status." That
//! asymmetry is the whole reason this file is short and
//! [`crate::workflows::statuses`] is not: nothing has to be migrated, because
//! nothing was ever standing on the thing being removed.
//!
//! # Why `required_permission` is checked by the database
//!
//! `workflow_transition.required_permission` is a foreign key into `permission`
//! (`migrations/0004`), and `docs/04` says the permission set is closed. So an
//! unknown key arrives here as a foreign-key violation and becomes `422`, not
//! `500` — and there is no list in this file that could drift from the
//! registry, because there is no list.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_persistence::workflow_edge::{self, NewTransition, TransitionPatch};
use casual_task_persistence::workflow_edit::{self, WriteError};
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::wire::Body;
use crate::workflows::audit::{internal, record};
use crate::workflows::guard;
use crate::workflows::wire::{CreateTransitionRequest, PatchTransitionRequest};
use crate::{etag, unit};

/// The most fields one transition may require.
///
/// `docs/21` bounds every input. Unbounded, a single row could carry a
/// megabyte of field names that the transition command then has to check on
/// every move.
const MAX_REQUIRED_FIELDS: usize = 20;

/// `POST /api/v1/workflows/{id}/transitions` — add an edge.
///
/// # Errors
///
/// `403`, `404`, `409 TF-WFL-0011` for a duplicate edge, `422` for a status in
/// another workflow or a permission outside the registry, `428` without
/// `If-Match`.
pub async fn create_transition(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(workflow_id): Path<Uuid>,
    Body(body): Body<CreateTransitionRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let fields = validated_fields(&body.required_fields, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let authored = guard::may_author(&mut scoped, &ctx, workflow_id, &headers, &request_id).await?;

    // Both endpoints of the edge, checked against **this** workflow. Without
    // it, an edge could name a status from another workflow and the board would
    // offer a move onto a column it does not draw.
    let to = endpoint(&mut scoped, workflow_id, body.to, &request_id).await?;
    let from = match body.from {
        Some(id) => Some(endpoint(&mut scoped, workflow_id, id, &request_id).await?),
        None => None,
    };

    let id = workflow_edge::insert_transition(
        &mut scoped,
        workflow_id,
        &NewTransition {
            from: from.as_ref().map(|s| s.id),
            to: to.id,
            required_permission: body.required_permission.as_deref(),
            required_fields: &fields,
            ignore_dependencies: body.ignore_dependencies,
        },
    )
    .await
    .map_err(|error| write_error(error, &request_id))?;

    record(
        &mut scoped,
        &ctx,
        workflow_id,
        "workflow.transition.created",
        serde_json::json!({
            // Display VALUES (`docs/25`): "from any status" is what a null
            // source means, and rendering it as `null` years later reads as
            // missing data rather than as the wildcard it is.
            "from": from.as_ref().map_or("any status", |s| s.name.as_str()),
            "to": to.name,
            "required_permission": body.required_permission,
        }),
        serde_json::json!({
            "before": null,
            "after": {
                "id": id,
                "from": from.as_ref().map(|s| s.id),
                "to": to.id,
                "required_permission": body.required_permission,
                "required_fields": fields,
                "ignore_dependencies": body.ignore_dependencies,
            },
        }),
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

/// `PATCH /api/v1/workflows/{id}/transitions/{tid}` — edit an edge's rules.
///
/// # Errors
///
/// `403`, `404`, `409`, `422` for a permission outside the registry, `428`
/// without `If-Match`.
pub async fn update_transition(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path((workflow_id, transition_id)): Path<(Uuid, Uuid)>,
    Body(body): Body<PatchTransitionRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let fields = body
        .required_fields
        .as_deref()
        .map(|fields| validated_fields(fields, &request_id))
        .transpose()?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let authored = guard::may_author(&mut scoped, &ctx, workflow_id, &headers, &request_id).await?;

    let patch = TransitionPatch {
        required_permission: body.required_permission.clone(),
        required_fields: fields.clone(),
        ignore_dependencies: body.ignore_dependencies,
    };
    let found = workflow_edge::update_transition(&mut scoped, workflow_id, transition_id, &patch)
        .await
        .map_err(|error| write_error(error, &request_id))?;
    if !found {
        return Err(ApiError::missing(codes::NOT_FOUND, &request_id));
    }

    record(
        &mut scoped,
        &ctx,
        workflow_id,
        "workflow.transition.updated",
        serde_json::json!({
            "required_permission": body.required_permission,
            "required_fields": fields,
            "ignore_dependencies": body.ignore_dependencies,
        }),
        serde_json::json!({ "transition": transition_id }),
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

/// `DELETE /api/v1/workflows/{id}/transitions/{tid}`.
///
/// # Errors
///
/// `403`, `404`, `409`, `428` without `If-Match`.
pub async fn delete_transition(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path((workflow_id, transition_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let authored = guard::may_author(&mut scoped, &ctx, workflow_id, &headers, &request_id).await?;

    let removed = workflow_edge::delete_transition(&mut scoped, workflow_id, transition_id)
        .await
        .map_err(|error| internal(error, "deleting the transition", &request_id))?;
    if !removed {
        return Err(ApiError::missing(codes::NOT_FOUND, &request_id));
    }

    record(
        &mut scoped,
        &ctx,
        workflow_id,
        "workflow.transition.deleted",
        serde_json::json!({ "transition": transition_id }),
        serde_json::json!({ "before": { "id": transition_id }, "after": null }),
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

/// A status of this workflow, or the `422` that says whose it is.
async fn endpoint(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    workflow_id: Uuid,
    status_id: Uuid,
    request_id: &str,
) -> Result<casual_task_persistence::workflow::StatusRow, ApiError> {
    workflow_edit::status_in(scoped, workflow_id, status_id)
        .await
        .map_err(|error| internal(error, "reading a transition endpoint", request_id))?
        .ok_or_else(|| {
            ApiError::unprocessable(
                codes::STATUS_WRONG_WORKFLOW,
                "A transition may only join two statuses of the same workflow",
                request_id,
            )
            .with_details(serde_json::json!({ "status_id": status_id }))
        })
}

fn write_error(error: WriteError, request_id: &str) -> ApiError {
    match error {
        WriteError::Duplicate => ApiError::conflict(
            codes::TRANSITION_EXISTS,
            "This workflow already has an edge between those two statuses",
            request_id,
        ),
        WriteError::UnknownReference => ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "required_permission must be a key in the permission registry \
             (docs/04). The set is closed; a permission that is not in it does \
             not exist",
            request_id,
        ),
        WriteError::Db(error) => internal(error, "writing the transition", request_id),
    }
}

/// `docs/21` bounds every input, and a field name is checked for shape here
/// because nothing downstream can: D-033 defers custom-field value storage, so
/// the transition command validates presence against a name and never resolves
/// it to a column.
fn validated_fields(fields: &[String], request_id: &str) -> Result<Vec<String>, ApiError> {
    if fields.len() > MAX_REQUIRED_FIELDS {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "A transition may require at most 20 fields",
            request_id,
        ));
    }
    let mut out = Vec::with_capacity(fields.len());
    for field in fields {
        let trimmed = field.trim();
        if trimmed.is_empty() || trimmed.chars().count() > 64 {
            return Err(ApiError::bad_request(
                codes::OUT_OF_RANGE,
                "A required field name is 1 to 64 characters",
                request_id,
            ));
        }
        out.push(trimmed.to_owned());
    }
    out.sort();
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_fields_are_bounded_deduplicated_and_ordered() {
        // Ordered and deduplicated so that TF-WFL-0004 — which names every
        // missing field at once — reports a stable list rather than one that
        // depends on the order an admin happened to type.
        let fields = [
            "resolution".to_owned(),
            " resolution ".to_owned(),
            "note".to_owned(),
        ];
        assert_eq!(
            validated_fields(&fields, "r").expect("valid"),
            vec!["note".to_owned(), "resolution".to_owned()]
        );
        assert!(validated_fields(&[String::new()], "r").is_err());
        assert!(validated_fields(&["x".repeat(65)], "r").is_err());
        let too_many: Vec<String> = (0..21).map(|n| format!("f{n}")).collect();
        assert!(validated_fields(&too_many, "r").is_err());
    }

    #[test]
    fn an_unknown_permission_is_the_callers_error_and_not_a_fault() {
        // The registry is closed (`docs/04`) and enforced by a foreign key, so
        // this is the only place the refusal is shaped. A 500 here would blame
        // the server for a client naming a permission that does not exist.
        let error = write_error(WriteError::UnknownReference, "r");
        assert_eq!(error.code(), codes::REFERENCE_NOT_FOUND);
        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
