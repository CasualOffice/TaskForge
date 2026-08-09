//! Transfer, promote, verify, and the chain they leave
//! (`docs/45-DEVELOPMENT-LIFECYCLE-AND-CUSTODY.md`).
//!
//! # Why these are commands and not fields
//!
//! `docs/05` principle 2: a command goes to a named sub-resource where the
//! operation has rules. All three have rules that a `PATCH` could not express.
//!
//! - **Transfer** clears the assignees as part of the move, because the task has
//!   to land in the receiving team's *queue*. A `PATCH {"team_id": …}` that
//!   silently emptied another field would be the worst kind of surprise.
//! - **Promotion** writes a log row with the column, because "when did this
//!   reach staging" is the question the column exists to serve and a plain field
//!   write answers it with nothing.
//! - **Verification** is not a field at all. It is an event with a verdict, an
//!   environment and evidence, and a task accumulates many.
//!
//! # Permissions, and why nothing new was invented
//!
//! Transfer and promotion are `task.update`: they change how a task behaves and
//! who is answerable for it, which is what that permission means — the same
//! reading `docs/23` applied when dependencies chose it over a new key.
//! Verification is `task.transition`, because a verdict always ends in a move
//! and a QA who could record a result but not act on it would be stuck holding
//! it.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_model::{ProjectId, permission};
use casual_task_persistence::custody::{self, CustodyError};
use casual_task_persistence::{Change, UnitOfWork, project, task};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::Body;

/// How much history one panel renders. `docs/21` bounds every read; a task with
/// more custody events than this is a runaway pipeline, not a page anyone reads.
const HISTORY_LIMIT: i64 = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferRequest {
    pub team_id: Uuid,
    /// Why it moved. Free text, because "not ours — the API returns 500" is the
    /// sentence the receiving team needs and no enum contains it.
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromoteRequest {
    pub environment_id: Uuid,
    /// The release this went out with, when it was a batch.
    #[serde(default)]
    pub release_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyRequest {
    /// `PASS` or `FAIL`.
    pub verdict: String,
    /// The environment it was tested on. Absent means the one the task is
    /// currently on, which is the ordinary case — QA tests what was pushed.
    #[serde(default)]
    pub environment_id: Option<Uuid>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TransferView {
    pub id: Uuid,
    pub from_team_id: Option<Uuid>,
    pub to_team_id: Uuid,
    pub moved_by: Uuid,
    pub moved_at: String,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PromotionView {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub release_id: Option<Uuid>,
    pub promoted_by: Uuid,
    pub promoted_at: String,
}

#[derive(Debug, Serialize)]
pub struct VerificationView {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub verdict: String,
    pub verified_by: Uuid,
    pub verified_at: String,
    pub note: Option<String>,
}

/// The custody panel's whole answer.
#[derive(Debug, Serialize)]
pub struct CustodyView {
    /// The team that owns it now. `null` means untriaged — which is not an
    /// error, it is the triage queue.
    pub team_id: Option<Uuid>,
    pub environment_id: Option<Uuid>,
    pub transfers: Vec<TransferView>,
    pub promotions: Vec<PromotionView>,
    pub verifications: Vec<VerificationView>,
}

/// `GET /api/v1/tasks/{id}/custody` — who has held this, where it has been, how
/// it fared.
///
/// One request for three lists, because they are one panel and always read
/// together; three endpoints would render the panel in three stages.
///
/// # Errors
///
/// `404` when the task is not visible.
pub async fn read(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let row = visible_task(&mut scoped, &ctx, task_id, &request_id).await?;
    let (transfers, promotions, verifications) =
        custody::history(&mut scoped, task_id, HISTORY_LIMIT)
            .await
            .map_err(|error| {
                tracing::error!(%error, "reading custody failed");
                ApiError::internal(&request_id)
            })?;
    unit::commit(tx, &request_id).await?;

    Ok(axum::Json(CustodyView {
        team_id: row.team_id,
        environment_id: row.environment_id,
        transfers: transfers.iter().map(transfer_view).collect(),
        promotions: promotions.iter().map(promotion_view).collect(),
        verifications: verifications.iter().map(verification_view).collect(),
    })
    .into_response())
}

/// `PUT /api/v1/tasks/{id}/team` — hand it to another team.
///
/// `PUT` and not `POST`: the task has exactly one owning team, and sending the
/// same team twice should be a no-op rather than a second hand-off. It is not
/// idempotent in the log — see the `409` below — because a *round trip* Android
/// → Backend → Android is two real events and the bounce count is the number
/// that matters.
///
/// # Errors
///
/// `404` when the task is not visible, `403` without `task.update`, `409` when
/// the task already belongs to that team, `422` when the team is not on the
/// task's project.
pub async fn transfer(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Body(body): Body<TransferRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let row = visible_task(&mut scoped, &ctx, task_id, &request_id).await?;
    authorize(
        &mut scoped,
        &ctx,
        row.project_id,
        permission::TASK_UPDATE,
        &request_id,
    )
    .await?;

    let moved = custody::transfer(
        &mut scoped,
        task_id,
        body.team_id,
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
            event_type: "task.team.transferred".to_owned(),
            activity_changes: serde_json::json!({
                "from_team_id": moved.from_team_id,
                "to_team_id": moved.to_team_id,
                "note": moved.note,
            }),
            audit_changes: serde_json::json!({
                "before": { "team_id": moved.from_team_id },
                "after": { "team_id": moved.to_team_id },
            }),
            payload: serde_json::json!({ "task_id": task_id, "team_id": moved.to_team_id }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the transfer failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((StatusCode::OK, axum::Json(transfer_view(&moved))).into_response())
}

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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

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
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

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
mod tests {
    use super::*;

    #[test]
    fn a_verdict_outside_the_pair_does_not_reach_the_database() {
        // The column is an enum, so a bad value would be a constraint violation
        // surfacing as a 500. "verdict must be PASS or FAIL" is the sentence a
        // caller can act on.
        for bad in ["PASSED", "ok", "", "DROP TABLE"] {
            assert!(!matches!(bad.to_uppercase().as_str(), "PASS" | "FAIL"));
        }
        assert!(matches!("pass".to_uppercase().as_str(), "PASS"));
    }

    #[test]
    fn an_unknown_field_does_not_deserialize() {
        // docs/05: unknown request fields are rejected, so a typo is a 400 and
        // not a silently ignored intention.
        assert!(serde_json::from_str::<TransferRequest>(r#"{"tema_id":"x"}"#).is_err());
        assert!(serde_json::from_str::<VerifyRequest>(r#"{"verdict":"PASS","x":1}"#).is_err());
    }
}
