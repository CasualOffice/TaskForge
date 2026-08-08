//! Loading the authority behind a request (`docs/04`).
//!
//! # This module reads grants; it does not interpret them
//!
//! `docs/19` puts the resolution algorithm in `casual-task-authz` and every
//! SQL statement here. So this module returns rows, and `casual-task-app`
//! turns them into `Grant`s and asks the resolver. Nothing in this file decides
//! anything — it cannot, because it has no way to name a `Permission`.
//!
//! # Why the lookup is a `UNION ALL` and not an `OR`
//!
//! `tests/explain/queries/15-authz-resolver-lookup.sql` fixes the shape, and
//! its comment says why: an `OR` across `(principal_type, principal_id)` forces
//! the planner to choose between a `BitmapOr` and a scan, and the scan wins as
//! soon as the estimate slips. `role_assignment` is a tenant-scale table, so a
//! scan there is a gate failure — and this read runs on **every** authorized
//! request.

use uuid::Uuid;

use crate::scoped::Scoped;

/// One `(assignment, permission)` pair, exactly as stored.
///
/// Flattened rather than nested: the join produces one row per permission, and
/// re-nesting here would mean this module deciding what a grant *is*, which is
/// `casual-task-authz`'s job.
#[derive(Debug, Clone)]
pub struct GrantRow {
    /// `WORKSPACE` | `TEAM` | `PROJECT` | `ENVIRONMENT`.
    pub scope_type: String,
    pub scope_id: Uuid,
    /// The closed constraint set, as stored. Interpreted by `casual-task-app`.
    pub constraints: serde_json::Value,
    /// A `resource.action` key from the closed registry.
    pub permission: String,
}

/// Everything the closed constraint set needs that is not on the resource
/// itself (`docs/04` §Constraint set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorFacts {
    /// `workspace_membership.member_type = 'GUEST'`.
    pub is_guest: bool,
}

/// The teams the actor belongs to **in this workspace**.
///
/// `team_membership` carries no `workspace_id` of its own, so the join to
/// `team` is what applies the tenant policy — without it a membership of a team
/// in another workspace would expand the principal set across the boundary.
///
/// # Errors
///
/// Any database error.
pub async fn teams_of(scoped: &mut Scoped<'_>, actor: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT tm.team_id
           FROM team_membership tm
           JOIN team t ON t.id = tm.team_id
          WHERE tm.user_id = $1
            AND t.deleted_at IS NULL",
    )
    .bind(actor)
    .fetch_all(scoped.conn())
    .await
}

/// Facts about the actor's standing in the workspace.
///
/// A missing membership row reads as a guest: the conservative direction, and
/// unreachable in practice because `WorkspaceMember` already refused anyone
/// without one.
///
/// # Errors
///
/// Any database error.
pub async fn actor_facts(scoped: &mut Scoped<'_>, actor: Uuid) -> Result<ActorFacts, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let member_type: Option<String> = sqlx::query_scalar(
        "SELECT member_type FROM workspace_membership
          WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace)
    .bind(actor)
    .fetch_optional(scoped.conn())
    .await?;

    Ok(ActorFacts {
        is_guest: member_type.as_deref() != Some("MEMBER"),
    })
}

/// Every grant reaching `actor` in this workspace, expanded over their teams.
///
/// `principal_type` for the actor's own grants is passed in rather than assumed
/// to be `USER`: a service-account token authenticates as a
/// `SERVICE_ACCOUNT` principal, and defaulting it to `USER` would silently
/// resolve a machine's authority from a person's grants.
///
/// # Errors
///
/// Any database error.
pub async fn grants_for(
    scoped: &mut Scoped<'_>,
    actor: Uuid,
    actor_principal_type: &str,
    teams: &[Uuid],
) -> Result<Vec<GrantRow>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let rows: Vec<(String, Uuid, serde_json::Value, String)> = sqlx::query_as(
        "SELECT ra.scope_type::text, ra.scope_id, ra.constraints, rp.permission
           FROM role_assignment ra
           JOIN role_permission rp ON rp.role_id = ra.role_id
          WHERE ra.workspace_id = $1
            AND ra.principal_type = $2::principal_type
            AND ra.principal_id = $3
         UNION ALL
         SELECT ra.scope_type::text, ra.scope_id, ra.constraints, rp.permission
           FROM role_assignment ra
           JOIN role_permission rp ON rp.role_id = ra.role_id
          WHERE ra.workspace_id = $1
            AND ra.principal_type = 'TEAM'::principal_type
            AND ra.principal_id = ANY($4)",
    )
    .bind(workspace)
    .bind(actor_principal_type)
    .bind(actor)
    .bind(teams)
    .fetch_all(scoped.conn())
    .await?;

    Ok(rows
        .into_iter()
        .map(|(scope_type, scope_id, constraints, permission)| GrantRow {
            scope_type,
            scope_id,
            constraints,
            permission,
        })
        .collect())
}
