//! `/api/v1/tags` — the vocabulary, as opposed to `/tasks/{id}/tags`, the use.
//!
//! # The failure this module prevents
//!
//! A write endpoint nobody can call. `POST /api/v1/tasks/{id}/tags` has taken a
//! `tag_id` since C-008 and there was no way to discover one — the only route to
//! a valid body was a `SELECT` against the database. An endpoint whose argument
//! cannot be obtained is not a capability; it is a plan for one.
//!
//! # Authoring and applying are different authorities, and stay that way
//!
//! `tag.manage` creates and edits the vocabulary; `task.update` applies an
//! existing tag to a task (`tasks::relations` says so at its own call site). The
//! split is what stops every typo becoming a permanent tag: someone who may
//! label work is not thereby someone who may invent labels.
//!
//! Which is also why there is **no create-by-name inside the task write**. A
//! picker that quietly created `secuirty` on a mistyped search would defeat the
//! bound below within a week.
//!
//! # There is no delete, and that is stated rather than forgotten
//!
//! `task_tag` cascades on `tag` (migration 0005), so deleting a tag silently
//! strips it from every task that carried it — a bulk edit wearing the clothes
//! of a settings change. What a user should be shown first (how many tasks lose
//! it? can it be merged into another tag instead?) is a real product decision,
//! and settling it inside a `DELETE` handler is the move AGENTS.md forbids.
//! Recorded as **D-065**, open. Removing a tag from one *task* is
//! `DELETE /api/v1/tasks/{id}/tags/{tag_id}` and is unaffected.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_model::{ProjectId, permission};
use casual_task_persistence::project;
use casual_task_persistence::tag::{self, NewTag, TagRow};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::Body;

/// `docs/21` bounds every input. Long enough for `needs-design-review`, short
/// enough that a tag stays a tag rather than becoming a sentence.
const MAX_NAME: usize = 48;

/// A tag, as a client sees it.
#[derive(Debug, Serialize)]
pub struct TagView {
    pub id: Uuid,
    /// `null` for a workspace-scoped tag, usable on any task in the workspace.
    pub project_id: Option<Uuid>,
    pub name: String,
    /// A presentation hint, and only that. The foundation §7 forbids colour as
    /// the sole carrier of meaning, so every surface renders the name too.
    pub color: Option<String>,
}

impl From<&TagRow> for TagView {
    fn from(row: &TagRow) -> Self {
        Self {
            id: row.id,
            project_id: row.project_id,
            name: row.name.clone(),
            color: row.color.clone(),
        }
    }
}

/// `POST /api/v1/tags`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub name: String,
    /// Absent creates a workspace-scoped tag — the common case, and the default
    /// because a tag that only one project may use is the narrower claim.
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub color: Option<String>,
}

/// `GET /api/v1/tags?project_id=…` — the vocabulary a picker renders.
///
/// **Pass `project_id` when the picker is attached to a task.** Without it the
/// answer is every tag in the workspace, which includes other projects' private
/// tags — and `task::usable_tag` refuses those with a `422`. A picker offering
/// options the write rejects reads to the user as a broken control rather than
/// as a scope they misunderstood.
///
/// Returned whole rather than paged, with a hard bound in the repository. A
/// vocabulary is configuration; a cursor over it would make every client
/// implement pagination to render a dropdown.
///
/// # Errors
///
/// `400` for an unknown query parameter or an unparseable `project_id`, `404`
/// when a named project is not visible.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    unit::reject_unknown(&params, &["project_id"], &request_id)?;
    let project_id = params
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
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // A project id is a claim about a project, so it is resolved through
    // visibility before it narrows anything. Without this, naming an invisible
    // project would return the workspace tags and confirm the project exists by
    // not erroring — a small leak, and exactly the shape `docs/04` forbids.
    if let Some(project) = project_id
        && project::read_visible(&mut scoped, &ctx.viewer, project)
            .await
            .map_err(|error| {
                tracing::error!(%error, "reading the project failed");
                ApiError::internal(&request_id)
            })?
            .is_none()
    {
        return Err(ApiError::missing(codes::PROJECT_NOT_FOUND, &request_id));
    }

    let rows = tag::list(&mut scoped, project_id).await.map_err(|error| {
        tracing::error!(%error, "listing tags failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    let data: Vec<TagView> = rows.iter().map(TagView::from).collect();
    Ok(axum::Json(serde_json::json!({ "data": data })).into_response())
}

/// `POST /api/v1/tags` — author a tag.
///
/// # Errors
///
/// `400` for a malformed name or colour, `403` without `tag.manage`, `404` when
/// a named project is not visible, `409` when the name is taken at that scope,
/// `422` at the per-workspace limit.
pub async fn create(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Body(body): Body<CreateRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let name = validated_name(&body.name, &request_id)?;
    let color = validated_color(body.color.as_deref(), &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // A project-scoped tag is authorized against that project; a workspace one
    // at workspace scope. Same permission, different reach — which is what
    // `may_in_project` and `may_in_workspace` already express, so nothing here
    // re-derives it.
    match body.project_id {
        Some(project_id) => {
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
                    permission::TAG_MANAGE,
                    ProjectId::from_uuid(project_row.id),
                    &project_row.teams(),
                    &ctx.facts_in_project(is_member),
                ),
                &request_id,
            )?;
        }
        None => unit::authorized(
            ctx.authority.may_in_workspace(permission::TAG_MANAGE),
            &request_id,
        )?,
    }

    let held = tag::count(&mut scoped).await.map_err(|error| {
        tracing::error!(%error, "counting tags failed");
        ApiError::internal(&request_id)
    })?;
    if held >= tag::MAX_PER_WORKSPACE {
        return Err(ApiError::unprocessable(
            codes::TAG_LIMIT,
            format!(
                "A workspace may hold at most {} tags",
                tag::MAX_PER_WORKSPACE
            ),
            &request_id,
        ));
    }

    let new = NewTag {
        id: Uuid::now_v7(),
        project_id: body.project_id,
        name: name.to_owned(),
        color,
    };
    let Some(row) = tag::insert(&mut scoped, &new).await.map_err(|error| {
        tracing::error!(%error, "creating the tag failed");
        ApiError::internal(&request_id)
    })?
    else {
        return Err(ApiError::conflict(
            codes::TAG_NAME_TAKEN,
            "A tag with that name already exists at that scope",
            &request_id,
        ));
    };
    unit::commit(tx, &request_id).await?;

    // No outbox event. A tag is vocabulary, not work: `docs/25` records what
    // happened to a task, and a feed entry per label invented is the noise
    // `docs/29` §The design problem is about. The audit trail for the
    // vocabulary is D-065's problem, alongside deletion.
    Ok((StatusCode::CREATED, axum::Json(TagView::from(&row))).into_response())
}

fn validated_name<'a>(raw: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request(
            codes::MISSING_FIELD,
            "name must not be empty",
            request_id,
        ));
    }
    if name.chars().count() > MAX_NAME {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            format!("name must be at most {MAX_NAME} characters"),
            request_id,
        ));
    }
    // A tag whose name is a comma cannot be filtered: `docs/27` §URL form makes
    // `tag=a,b` the `in` operator, so the separator cannot appear in a value.
    // Refused at authoring time, where the fix is obvious, rather than at filter
    // time, where it looks like the filter is broken.
    if name.contains(',') {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "name must not contain a comma — the filter grammar uses it as a separator",
            request_id,
        ));
    }
    Ok(name)
}

/// `#rrggbb`, or nothing.
///
/// Validated because it is interpolated into a style by every client that
/// renders a tag, and an unvalidated string there is a place to put something
/// that is not a colour.
fn validated_color(raw: Option<&str>, request_id: &str) -> Result<Option<String>, ApiError> {
    let Some(color) = raw.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    let valid = color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|c| c.is_ascii_hexdigit());
    if !valid {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "color must be #rrggbb",
            request_id,
        ));
    }
    Ok(Some(color.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_bounded_and_required() {
        assert_eq!(validated_name("  security  ", "r").ok(), Some("security"));
        assert_eq!(
            validated_name(" ", "r").err().map(|e| e.code()),
            Some(codes::MISSING_FIELD)
        );
        assert_eq!(
            validated_name(&"x".repeat(MAX_NAME + 1), "r")
                .err()
                .map(|e| e.code()),
            Some(codes::OUT_OF_RANGE)
        );
    }

    #[test]
    fn a_comma_in_a_name_is_refused_at_authoring_time() {
        // `docs/27` §URL form: `tag=a,b` is the `in` operator. A tag named
        // `a,b` would be unfilterable and would silently match two tags that do
        // not exist — a bug that surfaces in the filter, three screens from the
        // typo that caused it.
        assert_eq!(
            validated_name("needs,review", "r").err().map(|e| e.code()),
            Some(codes::OUT_OF_RANGE)
        );
    }

    #[test]
    fn a_colour_is_a_hex_triplet_or_a_400() {
        assert_eq!(
            validated_color(Some("#AABBCC"), "r").ok(),
            Some(Some("#aabbcc".to_owned()))
        );
        assert_eq!(validated_color(None, "r").ok(), Some(None));
        assert_eq!(validated_color(Some("  "), "r").ok(), Some(None));
        for bad in ["red", "#abc", "#gggggg", "javascript:alert(1)", "#aabbccdd"] {
            assert_eq!(
                validated_color(Some(bad), "r").err().map(|e| e.code()),
                Some(codes::OUT_OF_RANGE),
                "{bad} was accepted"
            );
        }
    }

    #[test]
    fn an_unknown_field_does_not_deserialize() {
        // docs/05: unknown request fields are rejected rather than ignored.
        assert!(serde_json::from_str::<CreateRequest>(r#"{"nme":"x"}"#).is_err());
    }
}
