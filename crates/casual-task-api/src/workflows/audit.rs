//! The history every workflow edit writes, and the one way it reports a fault.
//!
//! # Why this is its own module
//!
//! Nine authoring handlers write the same four rows — activity, audit, outbox,
//! and one delivery per consumer — through [`UnitOfWork::record`], and ADR-006
//! makes that "the domain change, its activity record, its audit record, and
//! its outbox event commit together or not at all". A helper each handler
//! called *differently* would satisfy the letter of that and lose the property
//! that matters: that every workflow event has the same aggregate, the same
//! shape, and the same scope.
//!
//! # Why `project_id` is always `None`
//!
//! A workflow is a workspace-level object that "may be shared by many
//! projects" (`docs/23` §Workflow structure). Attributing its events to one of
//! them would be picking arbitrarily and calling it a scope — and `docs/25`'s
//! fan-out filters on `project_id`, so the wrong one would deliver a
//! configuration change to one project's subscribers and hide it from the rest.

use casual_task_persistence::{Change, Scoped, UnitOfWork};
use uuid::Uuid;

use crate::context::Context;
use crate::error::ApiError;

/// Record one workflow edit, in the caller's transaction.
///
/// # Errors
///
/// `500`. There is no partial-success path: a change whose history failed to
/// write is precisely what ADR-006 forbids, so the caller must roll back.
pub async fn record(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    workflow_id: Uuid,
    event_type: &str,
    activity: serde_json::Value,
    audit: serde_json::Value,
    request_id: &str,
) -> Result<(), ApiError> {
    UnitOfWork::record(
        scoped,
        &Change {
            aggregate_type: "workflow".to_owned(),
            aggregate_id: workflow_id,
            project_id: None,
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
    .map_err(|error| internal(error, "recording the workflow change", request_id))
}

/// A database failure, logged with what was being attempted and reported as a
/// `500` carrying the request id the response header will also carry.
pub fn internal(error: sqlx::Error, what: &'static str, request_id: &str) -> ApiError {
    tracing::error!(%error, what, "a workflow authoring write failed");
    ApiError::internal(request_id)
}
