//! `/api/v1/tasks/{id}/dependencies` — the Relations panel (C-008, ADR-019).
//!
//! # The failure this module prevents
//!
//! A dependency graph with a loop in it, and a task drawer that can name tasks
//! the caller cannot see.
//!
//! The cycle check is not here. It is one `INSERT ... WHERE NOT EXISTS` in
//! `casual_task_persistence::dependency`, under an advisory lock, in the same
//! transaction as the write — so there is no "is this safe?" call this handler
//! could forget and no window in which a concurrent request closes the loop
//! from the other side. This module's job is to turn its refusals into the
//! documented status codes.
//!
//! # The read shape, and why this one
//!
//! `docs/05` specifies the write and **no read**, so the shape is a choice.
//! The drawer's Relations panel renders two lists — what is blocking this task,
//! and what this task is blocking — so the response is exactly that:
//!
//! ```json
//! { "blocked_by": [ { "id", "key", "title", "state" } ],
//!   "blocks":     [ … ] }
//! ```
//!
//! Two named lists rather than one flat array with a `direction` field, because
//! the panel renders them as two headed sections and a flat array would make
//! every client partition it again. `key`, `title` and `state` are included
//! because a relation the user cannot read is not a relation they can act on —
//! and `state` in particular is what lets the panel strike through a blocker
//! that is already `COMPLETED` instead of showing it as live.
//!
//! **Not paginated.** `docs/21` bounds dependencies at 100 per task, so the
//! whole set is one bounded response; a cursor over at most 100 rows would be
//! ceremony. The bound is enforced on the write, which is what makes that safe.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_model::{ProjectId, TeamId, permission};
use casual_task_persistence::dependency::{self, DependencyError, RelatedTask};
use casual_task_persistence::{Change, UnitOfWork, project, task};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::Body;

/// One end of a relation.
///
/// Every field but `restricted` is nullable, and all of them are null together.
/// `docs/03`: a blocking task "shows as 'restricted' if the viewer cannot see
/// its project, never as its title" — so the edge is reported and its identity
/// is not. Omitting the row instead would show a task as blocked by nothing,
/// which is a worse answer than "something you cannot see".
#[derive(Debug, Serialize)]
pub struct RelationView {
    pub id: Option<Uuid>,
    /// The human key, `WR-125`. `null` when restricted.
    pub key: Option<String>,
    pub title: Option<String>,
    pub state: Option<String>,
    /// `true` when the other end is in a project this viewer cannot see.
    pub restricted: bool,
}

impl From<&RelatedTask> for RelationView {
    fn from(row: &RelatedTask) -> Self {
        Self {
            id: row.id,
            key: row.key.clone(),
            title: row.title.clone(),
            state: row.state.clone(),
            restricted: row.restricted,
        }
    }
}

/// The Relations panel's two lists.
#[derive(Debug, Serialize)]
pub struct Relations {
    /// Tasks that must finish before this one can proceed.
    pub blocked_by: Vec<RelationView>,
    /// Tasks this one is holding up.
    pub blocks: Vec<RelationView>,
}

/// `POST /api/v1/tasks/{id}/dependencies`.
///
/// `blocks` names the task **this one blocks**; `blocked_by` names a task that
/// blocks this one. Exactly one, because a request carrying both is a client
/// that has not decided which direction it means, and picking one silently is
/// how a Relations panel ends up drawing the arrow backwards.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddDependency {
    #[serde(default)]
    pub blocks: Option<Uuid>,
    #[serde(default)]
    pub blocked_by: Option<Uuid>,
}

/// `GET /api/v1/tasks/{id}/dependencies`.
///
/// # Errors
///
/// `404` when the task does not exist or is not visible.
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

    // Visibility of the task decides the status code; visibility of each
    // *related* task decides whether it appears at all — a blocker in a project
    // the caller cannot open is omitted rather than named.
    task::read_visible(&mut scoped, &ctx.viewer, task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, &request_id))?;

    let blocked_by = dependency::blocked_by(&mut scoped, &ctx.viewer, task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading blockers failed");
            ApiError::internal(&request_id)
        })?;
    let blocks = dependency::blocks(&mut scoped, &ctx.viewer, task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading blocked tasks failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        axum::Json(Relations {
            blocked_by: blocked_by.iter().map(RelationView::from).collect(),
            blocks: blocks.iter().map(RelationView::from).collect(),
        }),
    )
        .into_response())
}

/// `POST /api/v1/tasks/{id}/dependencies` — cycle-checked.
///
/// # Errors
///
/// `404` when either task is absent or invisible — the same answer for both, so
/// a caller cannot discover task ids by proposing dependencies on them. `403`
/// without `task.update`. `422 TF-TSK-0003` when the edge would close a loop.
pub async fn add(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Body(body): Body<AddDependency>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let direction = Direction::of(&body, task_id, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let (task_row, project_key) = task::read_visible(&mut scoped, &ctx.viewer, task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, &request_id))?;

    // A dependency changes how a task behaves — it gates its transitions
    // (ADR-019) — so it is a task update, governed by `task.update`. There is
    // no `task.dependency.add` in the closed registry, and inventing one would
    // settle a permission question in a handler.
    let is_member = project::is_member(&mut scoped, task_row.project_id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading project membership failed");
            ApiError::internal(&request_id)
        })?;
    let team = project::read_visible(&mut scoped, &ctx.viewer, task_row.project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(&request_id)
        })?
        .and_then(|row| row.team_id);
    unit::authorized(
        ctx.authority.may_in_project(
            permission::TASK_UPDATE,
            ProjectId::from_uuid(task_row.project_id),
            team.map(TeamId::from_uuid),
            &ctx.facts_in_project(is_member),
        ),
        &request_id,
    )?;

    let (blocker, blocked) = direction.edge();
    let created = dependency::insert(&mut scoped, &ctx.viewer, blocker, blocked)
        .await
        .map_err(|error| match error {
            DependencyError::WouldCycle(path) => {
                // `docs/03` bounds the reachability check; naming the loop is
                // what makes the refusal actionable. "Invalid dependency" tells
                // a user nothing they can act on — `ONB-4 → API-2 → ONB-4`
                // tells them which link to remove.
                let message = if path.is_empty() {
                    "That dependency would create a cycle".to_owned()
                } else {
                    format!("That dependency would create a cycle: {}", path.join(" → "))
                };
                ApiError::unprocessable(codes::DEPENDENCY_CYCLE, message, &request_id)
                    .with_details(serde_json::json!({ "cycle": path }))
            }
            // Absent and invisible are one answer (`docs/04`).
            DependencyError::NotVisible => ApiError::missing(codes::TASK_NOT_FOUND, &request_id),
            DependencyError::TooMany => ApiError::unprocessable(
                codes::OUT_OF_RANGE,
                "This task already has the maximum number of dependencies",
                &request_id,
            ),
            DependencyError::Db(error) => {
                tracing::error!(%error, "adding the dependency failed");
                ApiError::internal(&request_id)
            }
        })?;

    if created {
        // ADR-006: the edge and its history commit together. `docs/25` names
        // the event `task.dependency.added`.
        UnitOfWork::record(
            &mut scoped,
            &Change {
                aggregate_type: "task".to_owned(),
                aggregate_id: task_id,
                project_id: Some(task_row.project_id),
                event_type: "task.dependency.added".to_owned(),
                // Display values, not ids (`docs/25`): the key is what the
                // History tab can still render years later.
                activity_changes: serde_json::json!({
                    "key": format!("{project_key}-{}", task_row.number),
                    "direction": direction.label(),
                    "other_task_id": direction.other(),
                }),
                audit_changes: serde_json::json!({
                    "before": null,
                    "after": { "from_task_id": blocker, "to_task_id": blocked },
                }),
                payload: serde_json::json!({
                    "task_id": task_id,
                    "from_task_id": blocker,
                    "to_task_id": blocked,
                }),
                schema_version: 1,
            },
            &ctx.provenance,
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, "recording the dependency failed");
            ApiError::internal(&request_id)
        })?;
    }

    let blocked_by = dependency::blocked_by(&mut scoped, &ctx.viewer, task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading blockers failed");
            ApiError::internal(&request_id)
        })?;
    let blocks = dependency::blocks(&mut scoped, &ctx.viewer, task_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading blocked tasks failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    // 201 for a new edge, 200 when it was already there. Re-adding is a no-op
    // rather than an error — the drawer's button is idempotent, and a duplicate
    // is not a cycle.
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        axum::Json(Relations {
            blocked_by: blocked_by.iter().map(RelationView::from).collect(),
            blocks: blocks.iter().map(RelationView::from).collect(),
        }),
    )
        .into_response())
}

/// Which way the edge points, once the request is known to be coherent.
#[derive(Debug, Clone, Copy)]
enum Direction {
    /// This task blocks `other`.
    Blocks { this: Uuid, other: Uuid },
    /// `other` blocks this task.
    BlockedBy { this: Uuid, other: Uuid },
}

impl Direction {
    fn of(body: &AddDependency, this: Uuid, request_id: &str) -> Result<Self, ApiError> {
        match (body.blocks, body.blocked_by) {
            (Some(_), Some(_)) => Err(ApiError::bad_request(
                codes::MALFORMED_BODY,
                "Send either `blocks` or `blocked_by`, not both",
                request_id,
            )),
            (Some(other), None) => Ok(Self::Blocks { this, other }),
            (None, Some(other)) => Ok(Self::BlockedBy { this, other }),
            (None, None) => Err(ApiError::bad_request(
                codes::MISSING_FIELD,
                "Send `blocks` with the task this one blocks, or `blocked_by` \
                 with the task that blocks it",
                request_id,
            )),
        }
    }

    /// `(blocker, blocked)` — the row's `from_task_id` and `to_task_id`.
    ///
    /// Migration 0005 stores the edge as `from` blocks `to`. Getting this
    /// backwards would draw every arrow the wrong way and gate the wrong
    /// transitions, so it is one function rather than an inline swap at each
    /// call site.
    const fn edge(self) -> (Uuid, Uuid) {
        match self {
            Self::Blocks { this, other } => (this, other),
            Self::BlockedBy { this, other } => (other, this),
        }
    }

    const fn other(self) -> Uuid {
        match self {
            Self::Blocks { other, .. } | Self::BlockedBy { other, .. } => other,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Blocks { .. } => "blocks",
            Self::BlockedBy { .. } => "blocked_by",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(json: &str) -> AddDependency {
        serde_json::from_str(json).expect("valid")
    }

    #[test]
    fn a_request_must_name_exactly_one_direction() {
        let this = Uuid::now_v7();
        assert_eq!(
            Direction::of(&body("{}"), this, "r")
                .err()
                .map(|e| e.code()),
            Some(codes::MISSING_FIELD)
        );
        let both = format!(
            r#"{{"blocks":"{}","blocked_by":"{}"}}"#,
            Uuid::now_v7(),
            Uuid::now_v7()
        );
        assert_eq!(
            Direction::of(&body(&both), this, "r")
                .err()
                .map(|e| e.code()),
            Some(codes::MALFORMED_BODY)
        );
    }

    #[test]
    fn the_two_directions_produce_opposite_edges() {
        // The bug this catches renders the whole Relations panel backwards and
        // gates the wrong task's transitions — and looks entirely plausible.
        let (this, other) = (Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(Direction::Blocks { this, other }.edge(), (this, other));
        assert_eq!(Direction::BlockedBy { this, other }.edge(), (other, this));
    }

    #[test]
    fn the_edge_direction_matches_the_schemas_column_names() {
        // migration 0005: `from_task_id` blocks `to_task_id`, and
        // `task::unresolved_blockers` reads it that way — it joins blockers on
        // `from_task_id` where `to_task_id` is the task being transitioned.
        let migration = include_str!("../../../migrations/0005_tasks.sql");
        assert!(migration.contains("from_task_id"));
        assert!(migration.contains("to_task_id"));
        let (this, other) = (Uuid::now_v7(), Uuid::now_v7());
        let (from, to) = Direction::Blocks { this, other }.edge();
        assert_eq!((from, to), (this, other), "`this` blocks `other`");
    }

    #[test]
    fn an_unknown_field_does_not_deserialize() {
        // docs/05: unknown request fields are rejected.
        assert!(serde_json::from_str::<AddDependency>(r#"{"blcoks":"x"}"#).is_err());
    }

    #[test]
    fn the_label_names_the_direction_the_caller_asked_for() {
        // It goes into the activity record, which is rendered years later.
        let (this, other) = (Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(Direction::Blocks { this, other }.label(), "blocks");
        assert_eq!(Direction::BlockedBy { this, other }.label(), "blocked_by");
        assert_eq!(Direction::Blocks { this, other }.other(), other);
    }
}
