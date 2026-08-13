//! `POST /api/v1/tasks/bulk` — many tasks, one request, one answer each.
//!
//! # Why partial success is the contract and not a failure mode
//!
//! `docs/05` §Bulk operations: "Bulk operations across 100 tasks with
//! individual permission and workflow rules will legitimately partially fail,
//! and all-or-nothing would make the feature useless." Selecting forty cards on
//! a board and dragging them to Done is the ordinary case, and six of them
//! being blocked, in a project the caller cannot transition in, or already
//! moved by someone else is *also* the ordinary case. Refusing all forty
//! because of those six teaches people not to use the feature.
//!
//! So every task gets its own transaction and its own result, and the response
//! is `207 Multi-Status` whatever the mix — including all-success and
//! all-failure. A client parses the same body every time rather than branching
//! on the status line and then discovering it must parse per-task results
//! anyway.
//!
//! # Undo
//!
//! A `207` across forty tasks where six refused cannot be reversed by one
//! inverse call, so each success carries what reversing *it* needs: the status
//! it came from, and the version it now holds. The client builds N inverse
//! transitions from the results; it does not need to have kept the before-state
//! itself.
//!
//! # What this does not do
//!
//! Above the limit the client is directed to the async job endpoint (`docs/05`),
//! which does not exist yet — C-024. Until it does, the refusal names the limit
//! and the client splits the batch.

use std::collections::{HashMap, HashSet};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::relations::apply_transition;
use super::wire::{TaskView, TransitionRequestBody};
use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::Body;

/// The most tasks one request may name (`docs/21`, `TF-LIM-0003`).
const MAX_TASKS: usize = 100;

/// The only operation implemented. Named in the body rather than the path so
/// the endpoint can grow assign, tag and archive without a new route each.
const TRANSITION: &str = "transition";

/// `POST /api/v1/tasks/bulk`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkRequestBody {
    pub operation: String,
    pub task_ids: Vec<Uuid>,
    /// `transition` only.
    #[serde(default)]
    pub to_status_id: Option<Uuid>,
    /// One note, written against every task that moves.
    #[serde(default)]
    pub comment: Option<String>,
    /// The version each task is expected to be at, by task id.
    ///
    /// Per task, not one header: forty tasks are at forty versions, and a
    /// single `If-Match` could only be a wildcard — which is exactly the
    /// lost-update the concurrency rule exists to prevent (`docs/23`).
    #[serde(default)]
    pub if_match: HashMap<Uuid, i64>,
}

/// One task's outcome, in the order it was named.
#[derive(Debug, Serialize)]
pub struct BulkResult {
    pub task_id: Uuid,
    /// The status the same operation would have returned on its own endpoint.
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskView>,
    /// What reversing this one task needs. Present only on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<Undo>,
    /// The same object a single-task refusal would carry under `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
}

/// The inverse of one successful transition.
#[derive(Debug, Serialize)]
pub struct Undo {
    /// The status this task was on before. `POST` it back to
    /// `/tasks/{id}/transitions` to reverse this one task.
    pub to_status_id: Uuid,
    /// The version the task now holds — the `If-Match` for that reversal.
    pub if_match: i64,
}

#[derive(Debug, Serialize)]
pub struct BulkResponse {
    pub results: Vec<BulkResult>,
    /// Counted here so a client can render "34 moved, 6 refused" without
    /// walking the list first.
    pub succeeded: usize,
    pub failed: usize,
}

/// Apply one operation to many tasks.
///
/// # The envelope is checked once; everything task-shaped is a per-task result
///
/// A malformed envelope — an unknown operation, no tasks, too many tasks —
/// refuses the whole request with a normal error, because there is nothing to
/// report per task. Anything that could be true of one task and false of the
/// next — not found, no permission, stale, blocked, no such transition — is a
/// row in the `207`. The dividing line is whether the client could have known
/// before sending.
///
/// # Errors
///
/// `400` with `TF-VAL-0005` (unknown operation), `TF-VAL-0003` (the operation's
/// field is missing), `TF-VAL-0004` (empty, or a repeated or unlisted task) or
/// `TF-LIM-0003` (over the limit). `503` when the pool cannot supply a
/// connection.
pub async fn bulk(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Body(body): Body<BulkRequestBody>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    if body.operation != TRANSITION {
        return Err(ApiError::bad_request(
            codes::INVALID_ENUM,
            format!("operation must be one of: {TRANSITION}"),
            &request_id,
        ));
    }
    let Some(to_status_id) = body.to_status_id else {
        return Err(ApiError::bad_request(
            codes::MISSING_FIELD,
            "to_status_id is required for the transition operation",
            &request_id,
        ));
    };
    if body.task_ids.is_empty() {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            format!("task_ids must name between 1 and {MAX_TASKS} tasks"),
            &request_id,
        ));
    }
    if body.task_ids.len() > MAX_TASKS {
        return Err(ApiError::bad_request(
            codes::BULK_TOO_LARGE,
            format!(
                "a bulk request may name at most {MAX_TASKS} tasks; \
                 split the batch or use the async job endpoint"
            ),
            &request_id,
        ));
    }
    let named: HashSet<Uuid> = body.task_ids.iter().copied().collect();
    if named.len() != body.task_ids.len() {
        // The second mention of a task is necessarily stale by the time it is
        // reached, so it would refuse with a conflict that reads like someone
        // else's edit. Refusing the request says what actually happened.
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "task_ids must not repeat a task",
            &request_id,
        ));
    }
    if let Some(stray) = body.if_match.keys().find(|id| !named.contains(id)) {
        // This one cannot be a per-task result — there is no row for a task
        // that was never named — and silently ignoring it would drop a task the
        // caller believed they were moving.
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            format!("if_match names {stray}, which is not in task_ids"),
            &request_id,
        ));
    }
    if let Some(comment) = body.comment.as_deref()
        && comment.len() > 65_536
    {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "comment must be at most 65536 bytes",
            &request_id,
        ));
    }

    // The authority snapshot is taken ONCE, in its own transaction, and every
    // task is then decided against it. Reloading it per task would be a hundred
    // round trips for an answer that cannot legitimately change mid-request,
    // and would let a grant revoked halfway through split one batch into two
    // policies.
    let ctx = {
        let mut tx = unit::begin(&state, &request_id).await?;
        let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
        let ctx =
            Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
        unit::commit(tx, &request_id).await?;
        ctx
    };

    let mut results = Vec::with_capacity(body.task_ids.len());
    for id in &body.task_ids {
        results.push(
            one(
                &state,
                &member,
                &ctx,
                *id,
                to_status_id,
                body.comment.clone(),
                body.if_match.get(id).copied(),
                &request_id,
            )
            .await,
        );
    }

    let succeeded = results.iter().filter(|r| r.error.is_none()).count();
    let failed = results.len() - succeeded;
    Ok((
        StatusCode::MULTI_STATUS,
        axum::Json(BulkResponse {
            results,
            succeeded,
            failed,
        }),
    )
        .into_response())
}

/// One task, in its own transaction.
///
/// Never returns `Err`: a failure here is this task's result, not the request's.
/// The transaction is opened per task deliberately — `docs/05`: "one bad task
/// does not roll back 99 good ones" — and dropping it without a commit is the
/// rollback.
#[allow(clippy::too_many_arguments)] // every one of them is per-request state, not a knob
async fn one(
    state: &AppState,
    member: &WorkspaceMember,
    ctx: &Context,
    id: Uuid,
    to_status_id: Uuid,
    comment: Option<String>,
    expected: Option<i64>,
    request_id: &str,
) -> BulkResult {
    let Some(expected) = expected else {
        return refused(id, &ApiError::precondition_required(request_id));
    };

    let mut tx = match unit::begin(state, request_id).await {
        Ok(tx) => tx,
        Err(error) => return refused(id, &error),
    };
    let mut scoped = match unit::scope(&mut tx, member, request_id).await {
        Ok(scoped) => scoped,
        Err(error) => return refused(id, &error),
    };

    let body = TransitionRequestBody {
        to_status_id,
        fields: HashMap::new(),
        comment,
    };
    let done = match apply_transition(&mut scoped, ctx, id, expected, &body, request_id).await {
        Ok(done) => done,
        // Dropping `tx` here rolls this task back and leaves the others alone.
        Err(error) => return refused(id, &error),
    };
    let from = done.from_status_id;
    if let Err(error) = unit::commit(tx, request_id).await {
        return refused(id, &error);
    }

    BulkResult {
        task_id: id,
        status: StatusCode::OK.as_u16(),
        // A no-op move — already on the target status — reports the status it is
        // on, so the undo it offers is a move to where it already is. Reversing
        // it is a no-op too, which is correct: nothing happened.
        undo: Some(Undo {
            to_status_id: from,
            if_match: done.version,
        }),
        task: Some(done.view),
        error: None,
    }
}

fn refused(task_id: Uuid, error: &ApiError) -> BulkResult {
    BulkResult {
        task_id,
        status: error.status().as_u16(),
        task: None,
        undo: None,
        error: Some(error.envelope()),
    }
}
