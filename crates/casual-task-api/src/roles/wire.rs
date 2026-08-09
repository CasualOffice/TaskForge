//! Request and response shapes for `/api/v1/roles` and `/api/v1/role-assignments`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `POST /api/v1/roles`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRoleRequest {
    pub name: String,
    /// Permission keys from the registry. Every one is checked against the
    /// actor's own effective set before the role is written (`docs/04` control
    /// 1: you cannot grant what you do not hold), and against
    /// `permission(key)` by the schema.
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// `PATCH /api/v1/roles/{id}`.
///
/// `permissions` is the **whole** set, not a delta. A delta would be checked
/// against the ceiling as a delta and land as a set, and the difference is
/// where a smuggled permission would live.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchRoleRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub permissions: Option<Vec<String>>,
}

/// `POST /api/v1/role-assignments`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignRequest {
    /// `USER` | `TEAM` | `SERVICE_ACCOUNT`.
    pub principal_type: String,
    pub principal_id: Uuid,
    pub role_id: Uuid,
    /// `WORKSPACE` | `TEAM` | `PROJECT` | `ENVIRONMENT`. ADR-005 excludes task
    /// scope, and the enum has no other member.
    pub scope_type: String,
    /// Omitted at workspace scope, where the scope is the workspace itself.
    #[serde(default)]
    pub scope_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct RoleView {
    pub id: Uuid,
    pub name: String,
    /// A template is a cloneable starting point, never special-cased code
    /// (`docs/04`). It is reported so a client can label it, not so anything
    /// behaves differently.
    pub is_template: bool,
    pub permissions: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

#[derive(Debug, Serialize)]
pub struct AssignmentView {
    pub id: Uuid,
    pub principal_type: String,
    pub principal_id: Uuid,
    pub role_id: Uuid,
    pub scope_type: String,
    pub scope_id: Uuid,
    pub granted_by: Uuid,
    pub granted_at: String,
}
