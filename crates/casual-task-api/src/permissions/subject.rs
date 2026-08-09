//! Whose authority is being asked about, and what it costs to ask.
//!
//! # The failure this prevents
//!
//! A permission oracle. `/permissions/explain` returns another person's
//! grants, so an endpoint that answered for any `actor_id` would let any
//! member enumerate every colleague's authority — including which admin holds
//! `workspace.delete`, which is a target list. Asking about *yourself* needs
//! nothing beyond membership; asking about anyone else requires `role.manage`,
//! the permission that already governs who may see and change the grant graph.
//!
//! The check is here rather than in each handler because there is exactly one
//! rule and two callers, and the way this goes wrong is one caller forgetting.

use casual_task_app::authority::{Authority, StoredGrant};
use casual_task_model::{TeamId, UserId, permission};
use casual_task_persistence::{authz, workspace};
use casual_task_persistence::scoped::Scoped;
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::unit;

/// The actor an answer is about, and the authority resolved for them.
#[derive(Debug)]
pub struct Subject {
    pub actor: Uuid,
    pub authority: Authority,
    /// Whether the subject is an external/guest member — one of the five
    /// constraints (`not_external`) is a fact about them, not about the task.
    pub is_guest: bool,
}

impl Subject {
    /// Resolve whose authority to answer about.
    ///
    /// `requested` is the `actor_id` from the request; `None` means the
    /// caller. A caller asking about themselves reuses the authority already
    /// resolved for the request rather than re-reading it — one round trip, and
    /// no chance of the two answers disagreeing.
    ///
    /// # Errors
    ///
    /// `403` when asking about someone else without `role.manage`, `500` on a
    /// database failure.
    pub async fn resolve(
        scoped: &mut Scoped<'_>,
        ctx: &Context,
        requested: Option<Uuid>,
        request_id: &str,
    ) -> Result<Self, ApiError> {
        let target = requested.unwrap_or_else(|| ctx.actor.as_uuid());

        if target == ctx.actor.as_uuid() {
            return Ok(Self {
                actor: target,
                authority: ctx.authority.clone(),
                is_guest: ctx.is_guest,
            });
        }

        // Someone else's grants. This is the disclosure, so it is gated before
        // a single row is read — a 403 that arrives after the query has run
        // still leaves the query in the log.
        unit::authorized(
            ctx.authority.may_in_workspace(permission::ROLE_MANAGE),
            request_id,
        )?;

        let internal = |what: &'static str| {
            move |error: sqlx::Error| {
                tracing::error!(%error, what, "resolving the subject's authority failed");
                ApiError::internal(request_id)
            }
        };

        // Row-level security confines every read below to this workspace, so a
        // uuid from another tenant resolves to no teams and no grants rather
        // than to someone else's authority. The membership check turns that
        // into an explicit 404 instead of a confusing empty answer.
        let member = workspace::is_member_scoped(scoped, target)
            .await
            .map_err(internal("membership"))?;
        if !member {
            return Err(ApiError::missing(codes::NOT_FOUND, request_id));
        }

        let teams = authz::teams_of(scoped, target)
            .await
            .map_err(internal("teams"))?;
        // A person, always: explaining a service account's authority through a
        // user-shaped endpoint would resolve a machine's grants from the human
        // of the same id, which is the bug `Context::load` documents.
        let grants = authz::grants_for(scoped, target, "USER", &teams)
            .await
            .map_err(internal("grants"))?;
        let facts = authz::actor_facts(scoped, target)
            .await
            .map_err(internal("facts"))?;

        let authority = Authority::resolved(
            UserId::from_uuid(target),
            ctx.workspace,
            teams.iter().copied().map(TeamId::from_uuid).collect(),
            false,
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

        Ok(Self {
            actor: target,
            authority,
            is_guest: facts.is_guest,
        })
    }
}
