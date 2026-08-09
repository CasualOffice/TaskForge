//! Request, response and path shapes for the workspace endpoints.
//!
//! The API's contract, separate from its implementation: `docs/05` fixes these
//! and the handlers change far more often.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct WorkspaceBody {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct MemberBody {
    pub user_id: Uuid,
    pub display_name: String,
    /// `null` once the account is anonymized (ADR-026).
    pub email: Option<String>,
    pub member_type: String,
    pub joined_at: String,
}

#[derive(Debug, Serialize)]
pub struct TeamBody {
    pub id: Uuid,
    pub name: String,
    pub created_at: String,
}

/// The documented list envelope (`docs/05` §Pagination).
#[derive(Debug, Serialize)]
pub struct PageBody<T> {
    pub data: Vec<T>,
    pub page: PageInfo,
}

#[derive(Debug, Serialize)]
pub struct PageInfo {
    /// Opaque. `docs/05`: "clients must not parse it".
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateWorkspace {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameWorkspace {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddMember {
    pub user_id: Uuid,
    /// Absent means `MEMBER`. `docs/04` §Built-in role templates gives GUEST a
    /// narrower shape, so making it the explicit choice keeps the wider one
    /// from being granted by omission.
    pub member_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTeam {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddTeamMember {
    pub user_id: Uuid,
}

/// Cursor pagination parameters.
///
/// `deny_unknown_fields` here as well as on bodies: `docs/05` says unknown
/// request fields are rejected, and a mistyped `?limti=200` that is silently
/// ignored produces the same class of client bug as a mistyped body field.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paging {
    pub limit: Option<u32>,
    /// The opaque `next_cursor` from a previous page.
    pub cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers — workspaces
// ---------------------------------------------------------------------------

/// `/workspaces/{workspace_id}/members/{user_id}`.
///
/// `workspace_id` is declared even though the handler reads the workspace from
/// the scope: it is the same parameter `WorkspaceMember` resolved the tenant
/// from, and naming every captured segment keeps this type a faithful
/// description of the route it is attached to.
#[derive(Debug, Deserialize)]
pub struct MemberPath {
    pub workspace_id: Uuid,
    pub user_id: Uuid,
}

/// `/teams/{team_id}/members`.
#[derive(Debug, Deserialize)]
pub struct TeamPath {
    pub team_id: Uuid,
}

/// `/teams/{team_id}/members/{user_id}`.
#[derive(Debug, Deserialize)]
pub struct TeamMemberPath {
    pub team_id: Uuid,
    pub user_id: Uuid,
}

// ---------------------------------------------------------------------------
// Shared plumbing
// ---------------------------------------------------------------------------

/// `docs/05` §Pagination: "limit default 50, max 100".
pub(crate) const DEFAULT_LIMIT: u32 = 50;
pub(crate) const MAX_LIMIT: u32 = 100;

/// The event schema version carried by every event this module emits.
pub(crate) const SCHEMA_VERSION: i32 = 1;

/// Bounds on the two free-text fields, so no input is unbounded (AGENTS.md
/// §Engineering priorities 4).
pub(crate) const MAX_NAME: usize = 200;
pub(crate) const MAX_SLUG: usize = 64;

// ---------------------------------------------------------------------------
// Representations
// ---------------------------------------------------------------------------
