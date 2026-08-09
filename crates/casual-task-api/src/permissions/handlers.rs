//! The two handlers.
//!
//! # Neither of these applies authority — they report it
//!
//! Every other module in this crate asks `may_in_project(...)` and refuses.
//! These two answer the question instead of acting on it, which is why they
//! carry no permission check of their own beyond [`Subject`]'s: a member is
//! always allowed to know what they themselves may do. Telling someone they
//! lack a permission discloses nothing they could not learn by pressing the
//! button.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use casual_task_app::ResourceFacts;
use casual_task_app::explain::Reach;
use casual_task_model::{Permission, ProjectId, UserId, permission};
use casual_task_persistence::{project, task};

use super::subject::Subject;
use super::wire::{
    ContributingGrantView, EffectivePermissionView, EffectiveQuery, EffectiveView, ExplainRequest,
    ExplainView, ResourceRef,
};
use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::Body;

fn reach_name(reach: Reach) -> &'static str {
    match reach {
        Reach::Unconditional => "unconditional",
        Reach::Conditional => "conditional",
    }
}

/// Parse a permission key, refusing anything outside the registry.
///
/// An unknown key is a `400` and not an empty answer: "you do not have
/// `task.clsoe`" is technically true and completely useless, and a typo in an
/// admin's debugging tool should say so.
fn parse_permission(key: &str, request_id: &str) -> Result<Permission, ApiError> {
    permission::ALL
        .iter()
        .copied()
        .find(|p| p.as_str() == key)
        .ok_or_else(|| {
            ApiError::bad_request(
                codes::INVALID_ENUM,
                format!("`{key}` is not a permission this build knows"),
                request_id,
            )
        })
}

/// `GET /api/v1/permissions/effective`.
///
/// # Errors
///
/// `404` if `project_id` is not visible to the caller, `500` on a database
/// failure.
pub async fn effective(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Query(query): Query<EffectiveQuery>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    // The caller's own set only. Rendering affordances for someone else is not
    // a thing a client does, and accepting an actor_id here would be a second
    // disclosure path to keep in step with `explain`'s.
    let permissions = match query.project_id {
        None => ctx.authority.effective_in_workspace(),
        Some(project_id) => {
            // Visibility first: answering for a project the caller cannot see
            // confirms it exists, which `docs/04` requires be indistinguishable
            // from absent.
            let project = project::read_visible(&mut scoped, &ctx.viewer, project_id)
                .await
                .map_err(|error| {
                    tracing::error!(%error, "reading the project failed");
                    ApiError::internal(&request_id)
                })?
                .ok_or_else(|| ApiError::missing(codes::PROJECT_NOT_FOUND, &request_id))?;
            ctx.authority
                .effective_in_project(ProjectId::from_uuid(project_id), &project.teams())
        }
    };

    tx.commit().await.map_err(|error| {
        tracing::error!(%error, "committing the read failed");
        ApiError::internal(&request_id)
    })?;

    Ok(axum::Json(EffectiveView {
        workspace_id: ctx.workspace.as_uuid(),
        actor_id: ctx.actor.as_uuid(),
        project_id: query.project_id,
        permissions: permissions
            .into_iter()
            .map(|e| EffectivePermissionView {
                permission: e.permission.as_str().to_owned(),
                reach: reach_name(e.reach),
            })
            .collect(),
    })
    .into_response())
}

/// The facts a constrained permission is evaluated against.
///
/// Loaded from the task when one is named, because `assignee_is_actor` and
/// `reporter_is_actor` cannot be answered without it — and "why can't I close
/// *this*?" is the question the endpoint exists for (`docs/04`). Note the
/// facts are about the **subject**, not the caller: an admin explaining
/// someone else's authority must get that person's answer.
async fn facts_for(
    scoped: &mut casual_task_persistence::scoped::Scoped<'_>,
    subject: &Subject,
    task_row: Option<&task::TaskRow>,
    actor_is_project_member: bool,
    request_id: &str,
) -> Result<ResourceFacts, ApiError> {
    let mut facts = ResourceFacts {
        actor_is_project_member,
        actor_is_guest: subject.is_guest,
        ..ResourceFacts::default()
    };
    if let Some(row) = task_row {
        facts.reporter = Some(UserId::from_uuid(row.reporter_id));
        facts.environment = row
            .environment_id
            .map(casual_task_model::EnvironmentId::from_uuid);
        facts.assignees = task::assignees(scoped, row.id)
            .await
            .map_err(|error| {
                tracing::error!(%error, "reading assignees failed");
                ApiError::internal(request_id)
            })?
            .into_iter()
            .map(UserId::from_uuid)
            .collect();
    }
    Ok(facts)
}

/// `POST /api/v1/permissions/explain`.
///
/// # Errors
///
/// `400` for an unknown permission key, `403` when explaining someone else
/// without `role.manage`, `404` when the subject or the named resource is not
/// visible, `500` on a database failure.
pub async fn explain(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Body(request): Body<ExplainRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let permission = parse_permission(&request.permission, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;
    let subject = Subject::resolve(&mut scoped, &ctx, request.actor_id, &request_id).await?;

    let resource = request.resource.unwrap_or(ResourceRef {
        project_id: None,
        task_id: None,
    });

    // A task named without a project supplies its own; a task and a project
    // that disagree is a caller bug and the task wins, because the task is the
    // more specific statement of what is being asked about.
    let task_row = match resource.task_id {
        None => None,
        Some(task_id) => Some(
            task::read_visible(&mut scoped, &ctx.viewer, task_id)
                .await
                .map_err(|error| {
                    tracing::error!(%error, "reading the task failed");
                    ApiError::internal(&request_id)
                })?
                .ok_or_else(|| ApiError::missing(codes::TASK_NOT_FOUND, &request_id))?
                .0,
        ),
    };

    let project_id = task_row
        .as_ref()
        .map(|r| r.project_id)
        .or(resource.project_id);

    let explanation = match project_id {
        None => subject.authority.explain_in_workspace(permission),
        Some(project_id) => {
            // Read whether or not a task named the project: the teams are part
            // of the scope chain, and when a task supplied the project its
            // visibility has already been established by reading the task.
            let teams = project::read_visible(&mut scoped, &ctx.viewer, project_id)
                .await
                .map_err(|error| {
                    tracing::error!(%error, "reading the project failed");
                    ApiError::internal(&request_id)
                })?
                .ok_or_else(|| ApiError::missing(codes::PROJECT_NOT_FOUND, &request_id))?
                .teams();
            let subject_in_project = subject
                .authority
                .granted_projects()
                .contains(&ProjectId::from_uuid(project_id));
            let facts = facts_for(
                &mut scoped,
                &subject,
                task_row.as_ref(),
                subject_in_project,
                &request_id,
            )
            .await?;
            subject.authority.explain_in_project(
                permission,
                ProjectId::from_uuid(project_id),
                &teams,
                &facts,
            )
        }
    };

    tx.commit().await.map_err(|error| {
        tracing::error!(%error, "committing the read failed");
        ApiError::internal(&request_id)
    })?;

    Ok(axum::Json(ExplainView {
        workspace_id: ctx.workspace.as_uuid(),
        actor_id: subject.actor,
        permission: permission.as_str().to_owned(),
        allowed: explanation.allowed,
        deny_reason: explanation.deny_reason,
        contributing_grants: explanation
            .contributing
            .into_iter()
            .map(|g| ContributingGrantView {
                scope_type: g.scope_type,
                scope_id: g.scope_id,
                constraints: g.constraints,
                constraints_satisfied: g.constraints_satisfied,
            })
            .collect(),
    })
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_permission_key_is_a_400_not_an_empty_answer() {
        let error = parse_permission("task.clsoe", "r").expect_err("refused");
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn every_registered_permission_parses() {
        // The registry is the contract; a key an admin can hold and cannot ask
        // about would make the debugging tool lie about its own coverage.
        for p in permission::ALL {
            assert_eq!(parse_permission(p.as_str(), "r").expect("known"), *p);
        }
    }

    #[test]
    fn reach_names_are_distinct_and_stable() {
        assert_ne!(
            reach_name(Reach::Unconditional),
            reach_name(Reach::Conditional)
        );
        assert_eq!(reach_name(Reach::Unconditional), "unconditional");
        assert_eq!(reach_name(Reach::Conditional), "conditional");
    }
}
