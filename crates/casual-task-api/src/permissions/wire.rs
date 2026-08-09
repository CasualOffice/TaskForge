//! Request and response shapes for `/api/v1/permissions/*` (`docs/04` §API).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `GET /api/v1/permissions/effective?project_id=`.
///
/// There is no `team_id`. A project involves several teams (`docs/03`) and the
/// server reads them off the project it just resolved — so a client can neither
/// understate the answer by omitting one, nor name a team the project does not
/// have and be told about grants that do not reach it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveQuery {
    /// Narrow the answer to one project. Absent means workspace scope.
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// `POST /api/v1/permissions/explain`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplainRequest {
    /// Whose authority to explain. Absent means the caller's own, which needs
    /// no extra permission; naming someone else discloses their grants and so
    /// requires `role.manage`.
    #[serde(default)]
    pub actor_id: Option<Uuid>,
    /// The permission key, e.g. `task.close`.
    pub permission: String,
    /// The resource to explain it against. Absent means workspace scope.
    #[serde(default)]
    pub resource: Option<ResourceRef>,
}

/// Which resource the question is about.
///
/// A task rather than a bare project when the caller has one: the constrained
/// permissions — `assignee_is_actor`, `reporter_is_actor` — cannot be answered
/// without the task's facts, and "why can't I close *this*?" is the question
/// `docs/04` says the endpoint exists for.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRef {
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub task_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct EffectiveView {
    pub workspace_id: Uuid,
    pub actor_id: Uuid,
    pub project_id: Option<Uuid>,
    pub permissions: Vec<EffectivePermissionView>,
}

#[derive(Debug, Serialize)]
pub struct EffectivePermissionView {
    pub permission: String,
    /// `unconditional` — exercisable on every resource in the scope.
    /// `conditional` — exercisable where the grant's constraints hold, so the
    /// client asks per resource instead of assuming either answer.
    pub reach: &'static str,
    /// For `task.create` only: which types the actor may raise, or absent for
    /// all of them.
    ///
    /// `reach: conditional` says the answer may be no; it cannot say which
    /// types, and a create form offering four where one will be refused is the
    /// cognitive burden the product exists to remove. Absent rather than a
    /// list of all five, so a sixth type does not silently fall outside it.
    ///
    /// This narrows a menu. The create path re-authorizes against the type
    /// actually sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_types: Option<Vec<&'static str>>,
}

#[derive(Debug, Serialize)]
pub struct ExplainView {
    pub workspace_id: Uuid,
    pub actor_id: Uuid,
    pub permission: String,
    pub allowed: bool,
    /// `null` when allowed; `no_grant` or `constraint_unsatisfied` otherwise.
    /// `docs/04`: every `Deny` names the reason.
    pub deny_reason: Option<&'static str>,
    pub contributing_grants: Vec<ContributingGrantView>,
}

#[derive(Debug, Serialize)]
pub struct ContributingGrantView {
    pub scope_type: &'static str,
    pub scope_id: Uuid,
    pub constraints: Vec<&'static str>,
    pub constraints_satisfied: bool,
}
