//! `/api/v1/tasks` and `/api/v1/projects/{id}/tasks` (C-008, read and create).
//!
//! # The list is the compiler's query, not this module's
//!
//! `docs/04` §The list problem: resolve the accessible project set once, then
//! filter with `project_id = ANY($accessible)`. That filter is injected by
//! `casual_task_persistence::compile`, whose signature *requires* an
//! `AuthorizedProjectSet` — so a list that forgot the permission filter does
//! not compile. This module supplies the set and never writes the predicate.
//!
//! # `status` is not a field a create chooses
//!
//! `docs/23`: "Status is **never** written through `PATCH /tasks/{id}`", and by
//! the same argument it is not written through a create either. A new task
//! enters the workflow's initial status, and the `state` it maps to is written
//! in the same statement — `casual_task_app::initial` hands back both together
//! so there is no way to write one of them.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_model::{Cursor, ProjectId, TeamId, permission};
use casual_task_persistence::task::{NewTask, TaskRow};
use casual_task_persistence::{
    AuthorizedProjectSet, Change, Page as CompilerPage, UnitOfWork, compile, idempotency, project,
    task,
};
use casual_task_search::filter::{Clause, Field, Node, Operator, Value};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::wire::{self, Body, Page, Paged};
use crate::{etag, unit};

/// The accessible project set is bounded.
///
/// `docs/26` §Permission filtering: past a few hundred projects for one actor
/// "the array stops being efficient" and the set is materialized into a
/// `project_access` table instead. That table does not exist yet, so the bound
/// is enforced here — a truncated set produces a visibly missing project rather
/// than a query that quietly degrades.
const MAX_ACCESSIBLE_PROJECTS: u32 = 500;

/// The task representation. `docs/05`: `snake_case`, RFC 3339 UTC, UUIDv7.
#[derive(Debug, Serialize)]
pub struct TaskView {
    pub id: Uuid,
    /// The human identifier — `WR-125`. Spans `project.key` and `task.number`,
    /// which is why it is composed here and stored nowhere (D-051).
    pub key: String,
    pub project_id: Uuid,
    pub number: i64,
    pub title: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub task_type: String,
    pub priority: String,
    pub status_id: Uuid,
    /// One of the five permanent states. Derived from `status_id` and written
    /// in the same statement, so it can never disagree with it (`docs/23`).
    pub state: String,
    pub reporter_id: Uuid,
    pub environment_id: Option<Uuid>,
    pub milestone_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub start_at: Option<String>,
    pub due_at: Option<String>,
    /// The lexicographic board rank (ADR-013).
    pub position: String,
    pub created_at: String,
    pub created_by: Uuid,
    pub updated_at: String,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<String>,
    pub version: i64,
}

fn view(row: &TaskRow, project_key: &str) -> TaskView {
    TaskView {
        id: row.id,
        key: format!("{project_key}-{}", row.number),
        project_id: row.project_id,
        number: row.number,
        title: row.title.clone(),
        description: row.description.clone(),
        task_type: row.task_type.clone(),
        priority: row.priority.clone(),
        status_id: row.status_id,
        state: row.state.clone(),
        reporter_id: row.reporter_id,
        environment_id: row.environment_id,
        milestone_id: row.milestone_id,
        parent_id: row.parent_id,
        start_at: row.start_at.map(wire::timestamp),
        due_at: row.due_at.map(wire::timestamp),
        position: row.position.clone(),
        created_at: wire::timestamp(row.created_at),
        created_by: row.created_by,
        updated_at: wire::timestamp(row.updated_at),
        updated_by: row.updated_by,
        archived_at: row.archived_at.map(wire::timestamp),
        version: row.version,
    }
}

/// `POST /api/v1/projects/{id}/tasks`.
///
/// `status_id` is deliberately absent — see the module docs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "type")]
    pub task_type: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    /// RFC 3339. Rejected rather than coerced if it is not.
    #[serde(default)]
    pub due_at: Option<String>,
}

/// `migrations/0001`'s `task_type` enum.
const TASK_TYPES: &[&str] = &["TASK", "BUG", "FEATURE", "INCIDENT", "REQUEST"];
/// `migrations/0001`'s `task_priority` enum, in its declared order.
const PRIORITIES: &[&str] = &["NONE", "LOW", "MEDIUM", "HIGH", "URGENT"];

/// `POST /api/v1/projects/{id}/tasks`.
///
/// # Errors
///
/// `400` for a malformed body or a missing `Idempotency-Key`, `404` when the
/// project is not visible, `403` without `task.create`, `422` for a parent in
/// another project.
#[allow(clippy::too_many_lines)] // one command, read top to bottom; splitting it hides the order
pub async fn create(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Body(body): Body<CreateRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let title = validated_title(&body.title, &request_id)?;
    let task_type = one_of(
        body.task_type.as_deref(),
        TASK_TYPES,
        "TASK",
        "type",
        &request_id,
    )?;
    let priority = one_of(
        body.priority.as_deref(),
        PRIORITIES,
        "NONE",
        "priority",
        &request_id,
    )?;
    let due_at = body
        .due_at
        .as_deref()
        .map(|raw| {
            OffsetDateTime::parse(raw, &Rfc3339).map_err(|_| {
                ApiError::bad_request(
                    codes::MALFORMED_BODY,
                    "due_at must be an RFC 3339 timestamp",
                    &request_id,
                )
            })
        })
        .transpose()?;
    if let Some(description) = body.description.as_deref()
        && description.len() > 65_536
    {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "description must be at most 65536 bytes",
            &request_id,
        ));
    }
    let idempotency_key = unit::idempotency_key(&headers, &request_id)?;
    let request_hash = unit::hash(&[
        project_id.as_bytes(),
        title.as_bytes(),
        task_type.as_bytes(),
        priority.as_bytes(),
        body.description.as_deref().unwrap_or_default().as_bytes(),
    ]);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    // Visibility first: an invisible project is a 404, and a 403 here would
    // tell an outsider it exists (`docs/04`).
    let project_row = project::read_visible(&mut scoped, &ctx.viewer, project_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::PROJECT_NOT_FOUND, &request_id))?;

    let is_member = project::is_member(&mut scoped, project_row.id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading project membership failed");
            ApiError::internal(&request_id)
        })?;
    unit::authorized(
        ctx.authority.may_in_project(
            permission::TASK_CREATE,
            ProjectId::from_uuid(project_row.id),
            project_row.team_id.map(TeamId::from_uuid),
            &ctx.facts_in_project(is_member),
        ),
        &request_id,
    )?;

    if let Some(replay) = unit::replay(
        &mut scoped,
        ctx.actor.as_uuid(),
        &idempotency_key,
        &request_hash,
        &request_id,
    )
    .await?
    {
        unit::commit(tx, &request_id).await?;
        return Ok(replay);
    }

    // ADR-018 caps subtask depth at 1, and TF-TSK-0006 requires a parent in the
    // same project. Both are checked against a task the actor can *see*, so a
    // parent id from another project is refused identically whether it exists
    // or not.
    if let Some(parent) = body.parent_id {
        let found = task::read_visible(&mut scoped, &ctx.viewer, parent)
            .await
            .map_err(|error| {
                tracing::error!(%error, "reading the parent task failed");
                ApiError::internal(&request_id)
            })?;
        match found {
            Some((parent_row, _)) if parent_row.project_id != project_row.id => {
                return Err(ApiError::unprocessable(
                    codes::PARENT_OUT_OF_PROJECT,
                    "A parent task must be in the same project",
                    &request_id,
                ));
            }
            Some((parent_row, _)) if parent_row.parent_id.is_some() => {
                return Err(ApiError::unprocessable(
                    codes::PARENT_OUT_OF_PROJECT,
                    "Subtasks are capped at one level (ADR-018)",
                    &request_id,
                ));
            }
            Some(_) => {}
            None => {
                return Err(ApiError::unprocessable(
                    codes::REFERENCE_NOT_FOUND,
                    "parent_id does not name a task in this project",
                    &request_id,
                ));
            }
        }
    }

    let (statuses, transitions) =
        casual_task_persistence::workflow::load(&mut scoped, project_row.workflow_id)
            .await
            .map_err(|error| {
                tracing::error!(%error, "loading the workflow failed");
                ApiError::internal(&request_id)
            })?;
    let workflow = casual_task_app::compose(
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
        ApiError::internal(&request_id)
    })?;
    let (status_id, state) = casual_task_app::initial(&workflow);

    // ADR-008: allocated in-transaction, so a rollback leaks no number. Users
    // read gaps in `WR-1, WR-2, WR-4` as lost data.
    let number = project::allocate_number(&mut scoped, project_row.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "allocating the task number failed");
            ApiError::internal(&request_id)
        })?;

    let new = NewTask {
        id: Uuid::now_v7(),
        project_id: project_row.id,
        number,
        title: title.to_owned(),
        description: body.description.clone(),
        task_type: task_type.to_owned(),
        priority: priority.to_owned(),
        status_id: status_id.as_uuid(),
        state: state_wire(state).to_owned(),
        reporter_id: ctx.actor.as_uuid(),
        parent_id: body.parent_id,
        due_at,
        position: casual_task_app::rank::appended(number),
        created_by: ctx.actor.as_uuid(),
    };
    let row = task::insert(&mut scoped, &new).await.map_err(|error| {
        tracing::error!(%error, "creating the task failed");
        ApiError::internal(&request_id)
    })?;

    let view = view(&row, &project_row.key);
    let payload = serde_json::to_value(&view).unwrap_or(serde_json::Value::Null);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: row.id,
            project_id: Some(row.project_id),
            event_type: "task.created".to_owned(),
            // Display values, not ids (`docs/25`): the status NAME, because the
            // status may be renamed or deleted before anyone reads this.
            activity_changes: serde_json::json!({
                "key": view.key,
                "title": row.title,
                "status": workflow.initial().name,
            }),
            audit_changes: serde_json::json!({ "before": null, "after": payload }),
            payload: payload.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the task create failed");
        ApiError::internal(&request_id)
    })?;

    let body = serde_json::json!(view);
    idempotency::record(
        &mut scoped,
        ctx.actor.as_uuid(),
        &idempotency_key,
        i32::from(StatusCode::CREATED.as_u16()),
        &body,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the idempotency response failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::CREATED,
        [
            (header::ETAG, etag::tag(row.version)),
            (header::LOCATION, format!("/api/v1/tasks/{}", row.id)),
        ],
        axum::Json(body),
    )
        .into_response())
}

/// `GET /api/v1/tasks/{id}` — 200 with an `ETag`, or 404.
///
/// # Errors
///
/// `404` when the task does not exist, is deleted, or sits in a project the
/// caller cannot see. All three are one answer (`docs/04`).
pub async fn read(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let found = task::read_visible(&mut scoped, &ctx.viewer, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    let Some((row, project_key)) = found else {
        return Err(ApiError::missing(codes::TASK_NOT_FOUND, &request_id));
    };
    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(row.version))],
        axum::Json(view(&row, &project_key)),
    )
        .into_response())
}

/// `GET /api/v1/tasks` — every task in the workspace the caller can reach.
///
/// # Errors
///
/// `400` for an unknown query parameter, a bad cursor, or an over-limit page.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    unit::reject_unknown(&params, &["limit", "cursor", "project_id"], &request_id)?;
    let limit = wire::limit(
        params
            .get("limit")
            .map(|raw| {
                raw.parse::<u32>().map_err(|_| {
                    ApiError::bad_request(
                        codes::PAGE_TOO_LARGE,
                        "limit must be a number",
                        &request_id,
                    )
                })
            })
            .transpose()?,
        &request_id,
    )?;
    let after = wire::cursor(params.get("cursor").map(String::as_str), &request_id)?;
    let project_filter = params
        .get("project_id")
        .map(|raw| {
            raw.parse::<Uuid>().map_err(|_| {
                ApiError::bad_request(
                    codes::MALFORMED_BODY,
                    "project_id must be a UUID",
                    &request_id,
                )
            })
        })
        .transpose()?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    // docs/04 §The list problem, step 1: resolved once, for the whole page.
    let accessible = project::accessible(&mut scoped, &ctx.viewer, MAX_ACCESSIBLE_PROJECTS)
        .await
        .map_err(|error| {
            tracing::error!(%error, "resolving the accessible project set failed");
            ApiError::internal(&request_id)
        })?;
    // A `project_id` the caller cannot see narrows the set to nothing rather
    // than returning a 404: it is a filter over a list, and a list filtered to
    // an invisible project is legitimately empty.
    let visible: Vec<ProjectId> = accessible
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| project_filter.is_none_or(|wanted| wanted == *id))
        .map(ProjectId::from_uuid)
        .collect();
    let keys: HashMap<Uuid, String> = accessible.into_iter().collect();

    let filter = project_filter.map_or_else(
        || Node::And(Vec::new()),
        |id| {
            Node::Clause(Clause {
                field: Field::Project,
                op: Operator::Eq,
                value: Value::Literal(id.to_string()),
            })
        },
    );
    let compiled = compile(
        &filter,
        ctx.workspace,
        &AuthorizedProjectSet::resolved(visible),
        &CompilerPage {
            after,
            limit,
            ..CompilerPage::default()
        },
    );
    let mut rows = task::list(&mut scoped, &compiled).await.map_err(|error| {
        tracing::error!(%error, "listing tasks failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    // The default sort is `updated_at DESC` (docs/26), so that is the key the
    // cursor carries. The id tiebreaker is mandatory — without it, ties in
    // updated_at make a page repeat or skip a row.
    let next_cursor = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| Cursor::new(vec![wire::timestamp(row.updated_at)], row.id).encode());

    let data: Vec<TaskView> = rows
        .iter()
        .map(|row| {
            let key = keys.get(&row.project_id).map_or("", String::as_str);
            view(row, key)
        })
        .collect();

    Ok(axum::Json(Paged {
        data,
        page: Page {
            next_cursor,
            has_more,
        },
    })
    .into_response())
}

/// The stored spelling of a state.
///
/// Exhaustive, so a sixth state cannot appear without deciding what it is
/// called on disk.
const fn state_wire(state: casual_task_model::TaskState) -> &'static str {
    use casual_task_model::TaskState;
    match state {
        TaskState::Backlog => "BACKLOG",
        TaskState::Planned => "PLANNED",
        TaskState::Active => "ACTIVE",
        TaskState::Completed => "COMPLETED",
        TaskState::Canceled => "CANCELED",
    }
}

fn validated_title<'a>(title: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = title.trim();
    // migrations/0005: CHECK (length(title) BETWEEN 1 AND 512). Checked here so
    // the caller gets a described bound rather than a 500 from a constraint.
    if trimmed.is_empty() || trimmed.chars().count() > 512 {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "title must be between 1 and 512 characters",
            request_id,
        ));
    }
    Ok(trimmed)
}

fn one_of<'a>(
    value: Option<&'a str>,
    allowed: &[&'static str],
    default: &'a str,
    field: &str,
    request_id: &str,
) -> Result<&'a str, ApiError> {
    let Some(value) = value else {
        return Ok(default);
    };
    if allowed.contains(&value) {
        Ok(value)
    } else {
        Err(ApiError::bad_request(
            codes::INVALID_ENUM,
            format!("{field} is not one of the permitted values"),
            request_id,
        )
        .with_details(serde_json::json!({ "field": field, "allowed": allowed })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_enums_match_the_ones_the_schema_declares() {
        // A value the API accepts and the enum does not is a 500 from a cast.
        let migration = include_str!("../../../migrations/0001_extensions_and_types.sql");
        for value in TASK_TYPES.iter().chain(PRIORITIES.iter()) {
            assert!(
                migration.contains(&format!("'{value}'")),
                "{value} is not declared in migration 0001"
            );
        }
        assert_eq!(TASK_TYPES.len(), 5);
        assert_eq!(PRIORITIES.len(), 5);
    }

    #[test]
    fn every_state_has_a_stored_spelling_the_schema_knows() {
        let migration = include_str!("../../../migrations/0001_extensions_and_types.sql");
        for state in casual_task_model::TaskState::ALL {
            assert!(
                migration.contains(&format!("'{}'", state_wire(state))),
                "{state:?} maps to a value task_state does not declare"
            );
        }
    }

    #[test]
    fn a_title_is_bounded_at_the_schemas_bound() {
        assert!(validated_title("x", "r").is_ok());
        assert!(validated_title(&"x".repeat(512), "r").is_ok());
        assert!(validated_title(&"x".repeat(513), "r").is_err());
        assert!(validated_title("   ", "r").is_err());
        let migration = include_str!("../../../migrations/0005_tasks.sql");
        assert!(
            migration.contains("length(title) BETWEEN 1 AND 512"),
            "the schema's title bound moved; this check must move with it"
        );
    }

    #[test]
    fn a_create_cannot_name_a_status() {
        // docs/23: status is never written directly, and a create is not an
        // exception. `deny_unknown_fields` is what enforces it.
        assert!(
            serde_json::from_str::<CreateRequest>(
                r#"{"title":"t","status_id":"018f2c9e-0000-7000-8000-000000000001"}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<CreateRequest>(r#"{"title":"t","state":"DONE"}"#).is_err());
    }

    #[test]
    fn a_key_reads_as_project_key_and_number() {
        // docs/05's pagination example shows "key": "WR-125". It spans two
        // tables, so it is composed on read (D-051).
        let row = TaskRow {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            project_id: Uuid::now_v7(),
            number: 125,
            title: "t".into(),
            description: None,
            task_type: "TASK".into(),
            priority: "NONE".into(),
            status_id: Uuid::now_v7(),
            state: "BACKLOG".into(),
            reporter_id: Uuid::now_v7(),
            environment_id: None,
            milestone_id: None,
            parent_id: None,
            start_at: None,
            due_at: None,
            position: "11111111".into(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: Uuid::now_v7(),
            updated_at: OffsetDateTime::UNIX_EPOCH,
            updated_by: None,
            version: 1,
            archived_at: None,
        };
        assert_eq!(view(&row, "WR").key, "WR-125");
    }

    #[test]
    fn an_unknown_enum_value_is_refused_rather_than_defaulted() {
        assert!(one_of(None, TASK_TYPES, "TASK", "type", "r").is_ok());
        assert_eq!(
            one_of(Some("EPIC"), TASK_TYPES, "TASK", "type", "r")
                .err()
                .map(|e| e.code()),
            Some(codes::INVALID_ENUM)
        );
    }
}
