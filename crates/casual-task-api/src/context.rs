//! What every tenant-scoped handler resolves before it does anything else.
//!
//! # One resolution per request, not one per row
//!
//! `docs/04` §The list problem: "Resolve the actor's accessible project set
//! **once**". [`Context::load`] is that once. It reads the actor's teams, their
//! grants, and their standing in the workspace, and hands back an [`Authority`]
//! and a [`Viewer`] that every subsequent decision in the request is made from.
//!
//! Nothing here decides anything either — [`Authority`] does, in
//! `casual-task-app`. This module's job is to make sure a handler cannot reach
//! a row without having resolved who is asking.

use axum::http::HeaderMap;
use casual_task_app::{Authority, ResourceFacts, StoredGrant};
use casual_task_model::{ActorType, TeamId, UserId, WorkspaceId};
use casual_task_persistence::project::Viewer;
use casual_task_persistence::{Provenance, Scoped, authz};

use crate::error::ApiError;
use crate::middleware::WorkspaceMember;

/// The resolved answer to "who is asking, and what may they do here".
#[derive(Debug, Clone)]
pub struct Context {
    pub actor: UserId,
    pub workspace: WorkspaceId,
    pub actor_type: ActorType,
    pub authority: Authority,
    /// The visibility inputs, for the project and task queries.
    pub viewer: Viewer,
    /// Whether the actor's workspace membership is `GUEST` — the
    /// `not_external` constraint's only input.
    pub is_guest: bool,
    pub provenance: Provenance,
}

impl Context {
    /// The inputs the closed constraint set needs for a resource in a project.
    ///
    /// `assignees`, `reporter` and `environment` are left empty here: they are
    /// task-level facts, and this is the project-level form. A task handler
    /// fills them in before asking about a task.
    #[must_use]
    pub fn facts_in_project(&self, actor_is_project_member: bool) -> ResourceFacts {
        ResourceFacts {
            actor_is_project_member,
            actor_is_guest: self.is_guest,
            ..ResourceFacts::default()
        }
    }

    /// Read everything a decision needs, in the caller's transaction.
    ///
    /// # Errors
    ///
    /// `500` on a database failure. There is no authorization failure here:
    /// this resolves authority, it does not apply it.
    pub async fn load(
        scoped: &mut Scoped<'_>,
        member: &WorkspaceMember,
        headers: &HeaderMap,
        request_id: &str,
    ) -> Result<Self, ApiError> {
        let actor = member.context.actor_id();
        let actor_type = member.context.actor_type();
        let workspace = member.context.scope().id();

        let internal = |what: &'static str| {
            move |error: sqlx::Error| {
                tracing::error!(%error, what, "resolving request authority failed");
                ApiError::internal(request_id)
            }
        };

        let teams = authz::teams_of(scoped, actor.as_uuid())
            .await
            .map_err(internal("teams"))?;

        // A service account's grants are stored against a SERVICE_ACCOUNT
        // principal. Looking them up as a USER would resolve a machine's
        // authority from whatever the person of the same id happens to hold.
        let principal_type = match actor_type {
            ActorType::ServiceAccount => "SERVICE_ACCOUNT",
            _ => "USER",
        };
        let grants = authz::grants_for(scoped, actor.as_uuid(), principal_type, &teams)
            .await
            .map_err(internal("grants"))?;
        let facts = authz::actor_facts(scoped, actor.as_uuid())
            .await
            .map_err(internal("membership"))?;

        let authority = Authority::resolved(
            actor,
            workspace,
            teams.iter().copied().map(TeamId::from_uuid).collect(),
            matches!(actor_type, ActorType::ServiceAccount),
            &grants
                .into_iter()
                .map(|g| StoredGrant {
                    scope_type: g.scope_type,
                    scope_id: g.scope_id,
                    constraints: g.constraints,
                    permission: g.permission,
                })
                .collect::<Vec<_>>(),
        );

        let viewer = Viewer {
            actor: actor.as_uuid(),
            teams,
            granted_projects: authority
                .granted_projects()
                .into_iter()
                .map(|p| p.as_uuid())
                .collect(),
        };

        Ok(Self {
            actor,
            workspace,
            actor_type,
            authority,
            viewer,
            is_guest: facts.is_guest,
            provenance: Provenance {
                actor: Some(actor),
                actor_type,
                // The request id is minted as a UUID by the observability
                // layer; a proxy-supplied one that is not a UUID is still fine
                // for correlation in logs but has nowhere to go in an `uuid`
                // column, so it is dropped rather than mangled.
                request_id: request_id
                    .parse::<uuid::Uuid>()
                    .ok()
                    .map(casual_task_model::RequestId::from_uuid),
                correlation_id: None,
                ip: crate::auth::client_ip(headers),
                user_agent: crate::auth::header_str(
                    headers,
                    axum::http::header::USER_AGENT.as_str(),
                )
                .map(ToOwned::to_owned),
            },
        })
    }
}
