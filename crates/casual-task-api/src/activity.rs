//! `GET /api/v1/tasks/{id}/activity` — the History tab (C-011, `docs/05`).
//!
//! # The failure this module prevents
//!
//! Telling somebody what happened to a task they cannot see. The activity
//! stream is the most attractive read in the product for that mistake: it names
//! actors, statuses and titles, and it is keyed by a task id the caller
//! supplies.
//!
//! So visibility is resolved **first**, through the same
//! `task::read_visible` every other task read uses, and an invisible task is a
//! `404` — indistinguishable from one that never existed (`docs/04`). Only
//! after that does the permission question arise.
//!
//! # Which permission, and why it is not `audit.read`
//!
//! `docs/25` §The three streams assigns the reads explicitly: activity is
//! `task.history.read`, audit is `audit.read`. They are different streams for
//! different readers — activity "must be readable by anyone who can see the
//! task", audit is for security and compliance and carries IP, user agent and
//! before/after. Gating this on `audit.read` would hide a user's own task
//! history behind an administrator's permission.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_model::{Cursor, ProjectId, permission};
use casual_task_persistence::activity::{self, ActivityCursor, ActivityRow};
use casual_task_persistence::{project, task};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::{self, Page, Paged};

/// One entry, as the History tab renders it.
#[derive(Debug, Serialize)]
pub struct ActivityView {
    pub id: Uuid,
    /// `task.status.changed`, `comment.created`, … (`docs/25` §Event catalogue).
    pub event_type: String,
    /// `null` for a system-generated change.
    pub actor_id: Option<Uuid>,
    /// Resolved so the tab can render "Sarah moved this to Done" without a
    /// second request per row. `null` when the actor is gone or the change was
    /// the system's.
    pub actor_name: Option<String>,
    /// Display values, not ids (`docs/25`) — this is what makes an entry still
    /// readable after a status has been renamed.
    pub changes: serde_json::Value,
    pub occurred_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `GET /api/v1/tasks/{id}/activity` — newest first, cursor-paged.
///
/// # Errors
///
/// `404` when the task does not exist or is not visible — the same answer for
/// both. `403` without `task.history.read`. `400` for a malformed cursor or an
/// over-limit page.
pub async fn stream(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Query(query): Query<ActivityQuery>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let limit = wire::limit(query.limit, &request_id)?;
    let after = decode_cursor(query.cursor.as_deref(), &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // Visibility first, and it decides the status code. A task in a project the
    // caller cannot see is absent, not forbidden.
    let (task_row, _) = task::read_visible(&mut scoped, &ctx.viewer, task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, &request_id))?;

    let is_member = project::is_member(&mut scoped, task_row.project_id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading project membership failed");
            ApiError::internal(&request_id)
        })?;
    // The project row, for the team a grant may reach the task through
    // (`docs/04` §The scope containment chain). Read through the visibility
    // predicate like everything else, so it cannot hand back a project the
    // actor could not have opened.
    let team = project::read_visible(&mut scoped, &ctx.viewer, task_row.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(&request_id)
        })?
        .map(|row| row.teams())
        .unwrap_or_default();
    unit::authorized(
        ctx.authority.may_in_project(
            permission::TASK_HISTORY_READ,
            ProjectId::from_uuid(task_row.project_id),
            &team,
            &ctx.facts_in_project(is_member),
        ),
        &request_id,
    )?;

    let mut rows = activity::for_task(&mut scoped, task_id, after, limit)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the activity stream failed");
            ApiError::internal(&request_id)
        })?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);

    // One lookup for the page, not one per row.
    let actors: Vec<Uuid> = {
        let mut ids: Vec<Uuid> = rows.iter().filter_map(|row| row.actor_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let names: HashMap<Uuid, String> = activity::actor_names(&mut scoped, &actors)
        .await
        .map_err(|error| {
            tracing::error!(%error, "resolving actor names failed");
            ApiError::internal(&request_id)
        })?
        .into_iter()
        .collect();
    unit::commit(tx, &request_id).await?;

    let next_cursor = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| Cursor::new(vec![wire::timestamp(row.occurred_at)], row.id).encode());

    let data: Vec<ActivityView> = rows.iter().map(|row| view(row, &names)).collect();
    Ok((
        StatusCode::OK,
        axum::Json(Paged {
            data,
            page: Page {
                next_cursor,
                has_more,
            },
        }),
    )
        .into_response())
}

fn view(row: &ActivityRow, names: &HashMap<Uuid, String>) -> ActivityView {
    ActivityView {
        id: row.id,
        event_type: row.event_type.clone(),
        actor_id: row.actor_id,
        actor_name: row
            .actor_id
            .and_then(|id| names.get(&id))
            .map(ToOwned::to_owned),
        changes: row.changes.clone(),
        occurred_at: wire::timestamp(row.occurred_at),
    }
}

/// The cursor carries `(occurred_at, id)`.
///
/// `activity_event` is partitioned by `occurred_at`, so the timestamp is what
/// lets the planner skip partitions rather than searching every one of them for
/// an id.
fn decode_cursor(raw: Option<&str>, request_id: &str) -> Result<Option<ActivityCursor>, ApiError> {
    let malformed =
        || ApiError::bad_request(codes::BAD_CURSOR, "Malformed pagination cursor", request_id);
    let Some(cursor) = wire::cursor(raw, request_id)? else {
        return Ok(None);
    };
    let occurred_at = cursor
        .keys
        .first()
        .ok_or_else(malformed)
        .and_then(|raw| OffsetDateTime::parse(raw, &Rfc3339).map_err(|_| malformed()))?;
    Ok(Some((occurred_at, cursor.id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_round_trips_through_its_timestamp_and_id() {
        // Dropping the timestamp would still "work" — until the table has more
        // than one partition, at which point resuming means scanning all of
        // them. The failure is a slow endpoint, not an error, which is why it
        // is worth a test.
        let id = Uuid::now_v7();
        let encoded = Cursor::new(vec![wire::timestamp(OffsetDateTime::UNIX_EPOCH)], id).encode();
        let decoded = decode_cursor(Some(&encoded), "r")
            .expect("valid")
            .expect("present");
        assert_eq!(decoded.0, OffsetDateTime::UNIX_EPOCH);
        assert_eq!(decoded.1, id);
    }

    #[test]
    fn a_garbage_cursor_is_a_400_and_not_a_panic() {
        assert!(decode_cursor(None, "r").expect("none").is_none());
        for raw in ["!!!", "eyJrIjpbXSwiaWQiOiJ4In0"] {
            assert!(
                decode_cursor(Some(raw), "r").is_err(),
                "{raw:?} was accepted"
            );
        }
    }

    #[test]
    fn a_system_change_renders_without_an_actor() {
        // `actor_id` is NULL for a retention sweep or an automation running as
        // nobody. An entry that unwrapped it would panic on the first one.
        let row = ActivityRow {
            id: Uuid::now_v7(),
            event_type: "task.archived".to_owned(),
            actor_id: None,
            changes: serde_json::json!({}),
            occurred_at: OffsetDateTime::UNIX_EPOCH,
        };
        let rendered = view(&row, &HashMap::new());
        assert!(rendered.actor_id.is_none());
        assert!(rendered.actor_name.is_none());
    }

    #[test]
    fn an_actor_whose_account_is_gone_still_renders_the_entry() {
        // ADR-026 anonymizes accounts. History outlives them, and an entry that
        // vanished with its actor would make the stream lie about what happened.
        let actor = Uuid::now_v7();
        let row = ActivityRow {
            id: Uuid::now_v7(),
            event_type: "task.updated".to_owned(),
            actor_id: Some(actor),
            changes: serde_json::json!({}),
            occurred_at: OffsetDateTime::UNIX_EPOCH,
        };
        let rendered = view(&row, &HashMap::new());
        assert_eq!(rendered.actor_id, Some(actor));
        assert!(rendered.actor_name.is_none());
    }
}
