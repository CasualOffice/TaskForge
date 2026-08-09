//! May this actor reshape this workflow, and are they looking at the current
//! one?
//!
//! # The failure this module prevents
//!
//! Six authoring handlers answering the same question six ways. `docs/04`'s
//! `guard` split exists for exactly this: "may this actor do this" assembled
//! twice is how one endpoint ends up more permissive than the one beside it,
//! and a workflow is the one object in the product where a permissive edit
//! moves other people's work.
//!
//! # Why a project-scoped grant is not enough on its own
//!
//! `project.workflow.manage` is a **project**-scope permission, and a workflow
//! is a **workspace**-level object that "may be shared by many projects"
//! (`docs/23` §Workflow structure). Those two facts do not compose on their
//! own: an actor granted `project.workflow.manage` on one project would, under
//! the obvious reading, be able to rename a status that every other project in
//! the workspace draws its board from.
//!
//! So the rule here is the strict one — the actor must hold
//! `project.workflow.manage` **in every project on this workflow**. A
//! workspace-scoped grant satisfies all of them at once, which is what a
//! workspace administrator has, and a single-project grant satisfies it only
//! when that project is the workflow's only user. That is the reading `docs/04`
//! §The escalation ceilings implies and `docs/23` never spelled out; it is
//! recorded as **D-064** rather than left as an accident of this file.
//!
//! A workflow with no projects on it — provisioned and not yet used — falls
//! back to the workspace-scope question, because there is no project to ask in.

use casual_task_app::ResourceFacts;
use casual_task_model::{ProjectId, TeamId, permission};
use casual_task_persistence::workflow::{StatusRow, TransitionRow, WorkflowRow};
use casual_task_persistence::{Scoped, workflow, workflow_edit};
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::workflows::wire::WorkflowView;
use crate::{etag, unit};

/// A workflow the caller may edit, with the version they claimed.
#[derive(Debug)]
pub struct Authored {
    pub row: WorkflowRow,
    /// The version **after** the claim — the `ETag` the response carries.
    pub version: i64,
}

/// Read a workflow the caller can see, or refuse with `404`.
///
/// A workflow is workspace configuration rather than tenant content, so
/// membership is the whole read rule (see [`crate::workflows::read`](mod@crate::workflows::read)). Row-level
/// security confines it to the caller's workspace, which makes another tenant's
/// workflow read as absent rather than as forbidden.
///
/// # Errors
///
/// `404` when it does not exist or belongs to another workspace — never
/// disambiguated. `500` on a database failure.
pub async fn visible(
    scoped: &mut Scoped<'_>,
    workflow_id: Uuid,
    request_id: &str,
) -> Result<WorkflowRow, ApiError> {
    workflow::read(scoped, workflow_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the workflow failed");
            ApiError::internal(request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, request_id))
}

/// The whole gate every authoring handler opens with.
///
/// In this order, and the order is the specification:
///
/// 1. **Readable** → else `404`. An invisible workflow must not be
///    distinguishable from an absent one (`docs/04`).
/// 2. **`If-Match` present and current** → else `428` / `409`. Before the
///    permission check, matching `docs/23` §Validation order and
///    `crate::projects::update`: the actor can already see the workflow, so its
///    version is not a secret, and a stale client is the overwhelmingly common
///    case.
/// 3. **`project.workflow.manage` everywhere this workflow lands** → else
///    `403`.
/// 4. **The version claimed** — `UPDATE … WHERE version = $expected`, which is
///    what actually serializes two admins and is why step 2 alone is not
///    enough.
///
/// # Errors
///
/// `404`, `428`, `409`, `403`, or `500`.
pub async fn may_author(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    workflow_id: Uuid,
    headers: &axum::http::HeaderMap,
    request_id: &str,
) -> Result<Authored, ApiError> {
    let row = visible(scoped, workflow_id, request_id).await?;
    let expected = etag::if_match(headers, request_id)?;
    if row.version != expected {
        return Err(stale(&row, expected, request_id));
    }

    authorize(scoped, ctx, workflow_id, request_id).await?;

    // The claim. Between the read above and this statement another admin may
    // have committed, so the guarded UPDATE — not the comparison — is what
    // makes the two serialize.
    let version = workflow_edit::claim_workflow(scoped, workflow_id, expected)
        .await
        .map_err(|error| {
            tracing::error!(%error, "claiming the workflow version failed");
            ApiError::internal(request_id)
        })?
        .ok_or_else(|| stale(&row, expected, request_id))?;

    Ok(Authored { row, version })
}

/// Step 3 on its own, for the read-side count endpoint which changes nothing.
///
/// # Errors
///
/// `403` when the actor lacks `project.workflow.manage` somewhere the workflow
/// lands; `500` on a database failure.
pub async fn authorize(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    workflow_id: Uuid,
    request_id: &str,
) -> Result<(), ApiError> {
    // The cheap answer first: a workspace-scope grant covers every project on
    // the workflow, present and future, and settles it without a query.
    if ctx
        .authority
        .may_in_workspace(permission::PROJECT_WORKFLOW_MANAGE)
        .is_allowed()
    {
        return Ok(());
    }

    let projects = workflow_edit::projects_on(scoped, workflow_id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the projects on this workflow failed");
            ApiError::internal(request_id)
        })?;

    // No project uses it yet, so there is nowhere to ask the project-scope
    // question. The workspace-scope answer above was the only one available,
    // and it was no.
    if projects.is_empty() {
        return unit::authorized(
            ctx.authority
                .may_in_workspace(permission::PROJECT_WORKFLOW_MANAGE),
            request_id,
        );
    }

    // Every project, not any: the edit lands in all of them.
    for (project, teams, is_member) in projects {
        let facts = ResourceFacts {
            actor_is_project_member: is_member,
            actor_is_guest: ctx.is_guest,
            ..ResourceFacts::default()
        };
        unit::authorized(
            ctx.authority.may_in_project(
                permission::PROJECT_WORKFLOW_MANAGE,
                ProjectId::from_uuid(project),
                &teams.into_iter().map(TeamId::from_uuid).collect::<Vec<_>>(),
                &facts,
            ),
            request_id,
        )?;
    }
    Ok(())
}

/// Re-read the workflow and return the representation every authoring handler
/// answers with.
///
/// Every one of them returns the **whole** workflow: deleting a status also
/// deletes edges, promoting an initial status demotes another, and a reorder
/// moves everything. A response carrying only the row that was touched leaves
/// the client's copy wrong in a way it cannot detect.
///
/// # Errors
///
/// `500` on a database failure.
pub async fn assemble(
    scoped: &mut Scoped<'_>,
    row: WorkflowRow,
    version: i64,
    request_id: &str,
) -> Result<WorkflowView, ApiError> {
    let (statuses, transitions): (Vec<StatusRow>, Vec<TransitionRow>) =
        workflow::load(scoped, row.id).await.map_err(|error| {
            tracing::error!(%error, "reading the workflow back failed");
            ApiError::internal(request_id)
        })?;
    Ok(WorkflowView::assemble(
        WorkflowRow { version, ..row },
        statuses,
        transitions,
    ))
}

/// The `409` body `docs/24` §The conflict response describes.
fn stale(current: &WorkflowRow, your_version: i64, request_id: &str) -> ApiError {
    ApiError::conflict(
        codes::VERSION_CONFLICT,
        "This workflow was changed by someone else",
        request_id,
    )
    .with_details(serde_json::json!({
        "your_version": your_version,
        "current_version": current.version,
    }))
}
