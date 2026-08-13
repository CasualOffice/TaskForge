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

use std::sync::OnceLock;
use std::time::Duration;

use axum::http::HeaderMap;
use casual_task_app::{Authority, CacheKey, EpochCache, ResourceFacts, StoredGrant};
use casual_task_model::{ActorType, ProjectId, TeamId, UserId, WorkspaceId};
use casual_task_observability::Recorder;
use casual_task_observability::labels::{LabelSet, keys};
use casual_task_observability::metrics::{AUTHZ_CACHE_HIT_RATIO, AUTHZ_RESOLUTION_DURATION};
use casual_task_persistence::project::Viewer;
use casual_task_persistence::{Provenance, Scoped, authz, workspace};

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

#[derive(Debug, Clone)]
struct AuthoritySnapshot {
    authority: Authority,
    viewer: Viewer,
    is_guest: bool,
}

fn read_cache() -> &'static EpochCache<CacheKey, AuthoritySnapshot> {
    static CACHE: OnceLock<EpochCache<CacheKey, AuthoritySnapshot>> = OnceLock::new();
    CACHE.get_or_init(|| EpochCache::new(10_000, Duration::from_secs(60)))
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
        metrics: &Recorder,
        scoped: &mut Scoped<'_>,
        member: &WorkspaceMember,
        headers: &HeaderMap,
        request_id: &str,
    ) -> Result<Self, ApiError> {
        let snapshot =
            Self::resolve_measured(metrics, scoped, member, request_id, "uncached").await?;
        Ok(Self::from_snapshot(snapshot, member, headers, request_id))
    }

    /// Resolve a read-side answer through the bounded epoch cache.
    ///
    /// Mutations call [`Self::load`] and therefore re-read authority inside
    /// their transaction. Lists, reads and UI affordances may call this method;
    /// an epoch bump changes the key, so a stale answer is unreachable.
    pub async fn load_read(
        metrics: &Recorder,
        scoped: &mut Scoped<'_>,
        member: &WorkspaceMember,
        headers: &HeaderMap,
        request_id: &str,
        project: Option<ProjectId>,
    ) -> Result<Self, ApiError> {
        let epoch = workspace::authz_epoch(scoped).await.map_err(|error| {
            tracing::error!(%error, "reading authz_epoch failed");
            ApiError::internal(request_id)
        })?;
        let key = CacheKey {
            workspace: member.context.scope().id(),
            actor: member.context.actor_id(),
            actor_type: member.context.actor_type(),
            project,
            epoch,
        };
        let snapshot = match read_cache().get(&key) {
            Some(snapshot) => {
                Self::record_resolution(metrics, "cache_hit", Duration::ZERO);
                snapshot
            }
            None => {
                let snapshot =
                    Self::resolve_measured(metrics, scoped, member, request_id, "cache_miss")
                        .await?;
                read_cache().insert(key, snapshot.clone());
                snapshot
            }
        };
        let _ = metrics.set(
            AUTHZ_CACHE_HIT_RATIO,
            &LabelSet::for_metric(AUTHZ_CACHE_HIT_RATIO),
            read_cache().hit_ratio(),
        );
        Ok(Self::from_snapshot(snapshot, member, headers, request_id))
    }

    async fn resolve_measured(
        metrics: &Recorder,
        scoped: &mut Scoped<'_>,
        member: &WorkspaceMember,
        request_id: &str,
        outcome: &'static str,
    ) -> Result<AuthoritySnapshot, ApiError> {
        let started = std::time::Instant::now();
        let resolved = Self::resolve(scoped, member, request_id).await;
        Self::record_resolution(metrics, outcome, started.elapsed());
        resolved
    }

    fn record_resolution(metrics: &Recorder, outcome: &'static str, elapsed: Duration) {
        let labels = LabelSet::for_metric(AUTHZ_RESOLUTION_DURATION)
            .with(keys::OUTCOME, outcome)
            .expect("the authorization outcome label is declared");
        let _ = metrics.observe(AUTHZ_RESOLUTION_DURATION, &labels, elapsed.as_secs_f64());
    }

    async fn resolve(
        scoped: &mut Scoped<'_>,
        member: &WorkspaceMember,
        request_id: &str,
    ) -> Result<AuthoritySnapshot, ApiError> {
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

        Ok(AuthoritySnapshot {
            authority,
            viewer,
            is_guest: facts.is_guest,
        })
    }

    fn from_snapshot(
        snapshot: AuthoritySnapshot,
        member: &WorkspaceMember,
        headers: &HeaderMap,
        request_id: &str,
    ) -> Self {
        let actor = member.context.actor_id();
        let actor_type = member.context.actor_type();
        Self {
            actor,
            workspace: member.context.scope().id(),
            actor_type,
            authority: snapshot.authority,
            viewer: snapshot.viewer,
            is_guest: snapshot.is_guest,
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
        }
    }
}
