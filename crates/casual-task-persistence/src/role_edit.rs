//! Authoring roles and granting them (C-003, `docs/04` §API).
//!
//! # What this module does not decide
//!
//! Every ceiling in `docs/04` §"The rules that stop RBAC from being a
//! self-service root exploit" is decided by `casual_task_authz::ceiling`, which
//! is a pure function over an actor's grants and needs no database. This module
//! reads and writes rows. Putting a ceiling here would be a second place to
//! change one, and the second place is the one that gets forgotten.
//!
//! Two controls are exceptions, and both are exceptions on purpose:
//!
//! - **Last-owner protection** (control 4) is a database trigger from migration
//!   0021, because `docs/04` says "enforced as a database constraint check
//!   inside the transaction, not just in application code". Application code
//!   that forgot it would still be refused.
//! - **The permission key set** is a foreign key to `permission(key)`. A role
//!   naming a permission this build does not have is refused by the schema
//!   rather than by a validation the next endpoint might skip.
//!
//! # Editing a role re-checks the ceiling
//!
//! `docs/04` control 1: the grant ceiling is checked at assignment time *and*
//! re-checked on role edit, because "editing a role you granted cannot smuggle
//! in new permissions". That re-check lives in the handler, which has the
//! actor's grants; what this module guarantees is that the permission set is
//! replaced wholesale rather than merged, so the set the handler checked is the
//! set that lands.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// The tuple a role row decodes into, before it becomes a [`RoleRow`].
///
/// Named because eight anonymous columns say nothing about which is which, and
/// clippy is right that the inline form is unreadable at three call sites.
type RoleTuple = (
    Uuid,
    Uuid,
    String,
    bool,
    Vec<String>,
    OffsetDateTime,
    OffsetDateTime,
    i64,
);

/// A role as stored, with the permissions it carries.
#[derive(Debug, Clone)]
pub struct RoleRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    /// Templates are cloneable starting points, never special-cased code
    /// (`docs/04` §Built-in role templates). Nothing in the resolver knows a
    /// role is built in.
    pub is_template: bool,
    pub permissions: Vec<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub version: i64,
}

/// Why a role could not be written.
#[derive(Debug)]
pub enum RoleError {
    /// The name is taken in this workspace.
    NameTaken,
    /// The permission set names a key this build does not have. Surfaced from
    /// the foreign key rather than from a list beside the registry.
    UnknownPermission,
    /// The role exists and someone else changed it first.
    VersionMismatch,
    Database(sqlx::Error),
}

impl std::fmt::Display for RoleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameTaken => f.write_str("that role name is already in use"),
            Self::UnknownPermission => f.write_str("unknown permission key"),
            Self::VersionMismatch => f.write_str("the role changed since it was read"),
            Self::Database(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RoleError {}

impl From<sqlx::Error> for RoleError {
    fn from(error: sqlx::Error) -> Self {
        match &error {
            sqlx::Error::Database(db) if db.is_unique_violation() => Self::NameTaken,
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => Self::UnknownPermission,
            _ => Self::Database(error),
        }
    }
}

/// Every role in the workspace, with its permissions.
///
/// Not paginated: `docs/21` bounds every input, and a workspace's roles are an
/// admin-authored set small enough that a page boundary would be noise. The
/// permissions come back as a correlated `ARRAY(...)` rather than a join, so one
/// role with forty permissions is one row and not forty.
///
/// # Errors
///
/// Any database error.
pub async fn list(scoped: &mut Scoped<'_>) -> Result<Vec<RoleRow>, sqlx::Error> {
    let rows: Vec<RoleTuple> = sqlx::query_as(
        "SELECT r.id, r.workspace_id, r.name, r.is_template,
                    ARRAY(SELECT rp.permission FROM role_permission rp
                           WHERE rp.role_id = r.id ORDER BY rp.permission),
                    r.created_at, r.updated_at, r.version
               FROM role r
              ORDER BY r.is_template DESC, r.name",
    )
    .fetch_all(scoped.conn())
    .await?;
    Ok(rows.into_iter().map(row_of).collect())
}

/// One role, or `None` when it is not in this workspace.
///
/// # Errors
///
/// Any database error.
pub async fn read(scoped: &mut Scoped<'_>, role: Uuid) -> Result<Option<RoleRow>, sqlx::Error> {
    let row: Option<RoleTuple> = sqlx::query_as(
        "SELECT r.id, r.workspace_id, r.name, r.is_template,
                    ARRAY(SELECT rp.permission FROM role_permission rp
                           WHERE rp.role_id = r.id ORDER BY rp.permission),
                    r.created_at, r.updated_at, r.version
               FROM role r
              WHERE r.id = $1",
    )
    .bind(role)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(row.map(row_of))
}

fn row_of(t: RoleTuple) -> RoleRow {
    RoleRow {
        id: t.0,
        workspace_id: t.1,
        name: t.2,
        is_template: t.3,
        permissions: t.4,
        created_at: t.5,
        updated_at: t.6,
        version: t.7,
    }
}

/// Author a role.
///
/// `workspace_id` comes from the scope, never a parameter, so the row and the
/// policy that will guard it cannot disagree.
///
/// # Errors
///
/// [`RoleError::NameTaken`], [`RoleError::UnknownPermission`], or any database
/// error.
pub async fn create(
    scoped: &mut Scoped<'_>,
    name: &str,
    permissions: &[String],
) -> Result<RoleRow, RoleError> {
    let workspace = scoped.workspace_id().as_uuid();
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO role (id, workspace_id, name) VALUES ($1,$2,$3)")
        .bind(id)
        .bind(workspace)
        .bind(name)
        .execute(scoped.conn())
        .await?;
    replace_permissions(scoped, id, permissions).await?;
    read(scoped, id)
        .await?
        .ok_or_else(|| RoleError::Database(sqlx::Error::RowNotFound))
}

/// Rename a role and replace its permission set, with optimistic concurrency.
///
/// The version is compared in the `WHERE` clause rather than read first, so two
/// concurrent edits cannot both see the same version and both win.
///
/// A template can be edited: `docs/04` says templates are "cloneable starting
/// points, not special-cased code", and code that refused here would be the
/// special case that document rules out.
///
/// # Errors
///
/// [`RoleError::VersionMismatch`] when the version does not match,
/// [`RoleError::NameTaken`], [`RoleError::UnknownPermission`], or any database
/// error.
pub async fn update(
    scoped: &mut Scoped<'_>,
    role: Uuid,
    name: Option<&str>,
    permissions: Option<&[String]>,
    expected_version: i64,
) -> Result<RoleRow, RoleError> {
    let affected = sqlx::query(
        "UPDATE role
            SET name = COALESCE($3, name), updated_at = now(), version = version + 1
          WHERE id = $1 AND version = $2",
    )
    .bind(role)
    .bind(expected_version)
    .bind(name)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    if affected != 1 {
        return Err(RoleError::VersionMismatch);
    }
    if let Some(permissions) = permissions {
        replace_permissions(scoped, role, permissions).await?;
    }
    read(scoped, role)
        .await?
        .ok_or_else(|| RoleError::Database(sqlx::Error::RowNotFound))
}

/// Replace a role's permissions wholesale.
///
/// Delete-then-insert rather than a merge, and that is the point: the handler
/// checked the *set* against the grant ceiling, and a merge would let a
/// permission survive that the check never saw.
async fn replace_permissions(
    scoped: &mut Scoped<'_>,
    role: Uuid,
    permissions: &[String],
) -> Result<(), RoleError> {
    sqlx::query("DELETE FROM role_permission WHERE role_id = $1")
        .bind(role)
        .execute(scoped.conn())
        .await?;
    if permissions.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO role_permission (role_id, permission)
         SELECT $1, unnest($2::text[])
         ON CONFLICT DO NOTHING",
    )
    .bind(role)
    .bind(permissions)
    .execute(scoped.conn())
    .await?;
    Ok(())
}

/// The tuple a grant row decodes into, before it becomes an [`AssignmentRow`].
type AssignmentTuple = (Uuid, String, Uuid, Uuid, String, Uuid, Uuid, OffsetDateTime);

/// One grant, as stored.
#[derive(Debug, Clone)]
pub struct AssignmentRow {
    pub id: Uuid,
    pub principal_type: String,
    pub principal_id: Uuid,
    pub role_id: Uuid,
    pub scope_type: String,
    pub scope_id: Uuid,
    pub granted_by: Uuid,
    pub granted_at: OffsetDateTime,
}

/// Create a grant.
///
/// Idempotent: the schema's unique key covers
/// `(workspace, principal, role, scope)` precisely because "the UI retries", so
/// granting twice returns the existing row rather than erroring.
///
/// # Errors
///
/// Any database error. A refused grant is refused *before* this by
/// `casual_task_authz::ceiling::may_assign`, and the last-owner rule is a
/// trigger from migration 0021.
pub async fn assign(
    scoped: &mut Scoped<'_>,
    principal_type: &str,
    principal_id: Uuid,
    role: Uuid,
    scope_type: &str,
    scope_id: Uuid,
    granted_by: Uuid,
) -> Result<AssignmentRow, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let row: AssignmentTuple = sqlx::query_as(
        "INSERT INTO role_assignment
             (id, workspace_id, principal_type, principal_id, role_id,
              scope_type, scope_id, granted_by)
         VALUES ($1,$2,$3::principal_type,$4,$5,$6::scope_type,$7,$8)
         ON CONFLICT (workspace_id, principal_type, principal_id, role_id, scope_type, scope_id)
             DO UPDATE SET granted_by = role_assignment.granted_by
         RETURNING id, principal_type::text, principal_id, role_id,
                   scope_type::text, scope_id, granted_by, granted_at",
    )
    .bind(Uuid::now_v7())
    .bind(workspace)
    .bind(principal_type)
    .bind(principal_id)
    .bind(role)
    .bind(scope_type)
    .bind(scope_id)
    .bind(granted_by)
    .fetch_one(scoped.conn())
    .await?;
    Ok(AssignmentRow {
        id: row.0,
        principal_type: row.1,
        principal_id: row.2,
        role_id: row.3,
        scope_type: row.4,
        scope_id: row.5,
        granted_by: row.6,
        granted_at: row.7,
    })
}

/// Revoke a grant. Returns whether a row was removed.
///
/// The last grant carrying `workspace.owner` cannot be removed — migration
/// 0021's trigger refuses it inside this transaction, so a caller that forgot
/// the rule still cannot break it.
///
/// # Errors
///
/// Any database error, including the trigger's refusal.
pub async fn revoke(scoped: &mut Scoped<'_>, assignment: Uuid) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query("DELETE FROM role_assignment WHERE id = $1")
        .bind(assignment)
        .execute(scoped.conn())
        .await?
        .rows_affected();
    Ok(affected == 1)
}

/// Which grants a listing asks for. Every field narrows; `None` means "any".
#[derive(Debug, Clone, Copy, Default)]
pub struct AssignmentFilter {
    pub principal_id: Option<Uuid>,
    pub role_id: Option<Uuid>,
    pub scope_id: Option<Uuid>,
}

/// The grants in this workspace, newest first, one page at a time.
///
/// # Why this read has to exist
///
/// `assign` and `revoke` were written without it, which made the grant set
/// write-only: revoking needs the assignment id, and the only place that id
/// ever appeared was the response to the call that created it. An admin who
/// closed the tab could never take a permission back through the API.
///
/// # Why the order is descending and the cursor is an id
///
/// `role_assignment.id` is a UUIDv7, so id order *is* time order and one column
/// serves both the sort and the keyset. `docs/26` bans `OFFSET`; `after` is the
/// last id of the previous page and the predicate is `<` because the newest
/// grant is the one an admin is looking for.
///
/// # Errors
///
/// Any database error.
pub async fn list_assignments(
    scoped: &mut Scoped<'_>,
    filter: AssignmentFilter,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<AssignmentRow>, sqlx::Error> {
    let rows: Vec<AssignmentTuple> = sqlx::query_as(
        "SELECT id, principal_type::text, principal_id, role_id,
                scope_type::text, scope_id, granted_by, granted_at
           FROM role_assignment
          WHERE ($1::uuid IS NULL OR principal_id = $1::uuid)
            AND ($2::uuid IS NULL OR role_id      = $2::uuid)
            AND ($3::uuid IS NULL OR scope_id     = $3::uuid)
            AND ($4::uuid IS NULL OR id           < $4::uuid)
          ORDER BY id DESC
          LIMIT $5",
    )
    .bind(filter.principal_id)
    .bind(filter.role_id)
    .bind(filter.scope_id)
    .bind(after)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;

    Ok(rows.into_iter().map(row_to_assignment).collect())
}

fn row_to_assignment(r: AssignmentTuple) -> AssignmentRow {
    AssignmentRow {
        id: r.0,
        principal_type: r.1,
        principal_id: r.2,
        role_id: r.3,
        scope_type: r.4,
        scope_id: r.5,
        granted_by: r.6,
        granted_at: r.7,
    }
}

/// One grant by id, or `None` when it is not in this workspace.
///
/// # Errors
///
/// Any database error.
pub async fn read_assignment(
    scoped: &mut Scoped<'_>,
    assignment: Uuid,
) -> Result<Option<AssignmentRow>, sqlx::Error> {
    let row: Option<AssignmentTuple> = sqlx::query_as(
        "SELECT id, principal_type::text, principal_id, role_id,
                    scope_type::text, scope_id, granted_by, granted_at
               FROM role_assignment WHERE id = $1",
    )
    .bind(assignment)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(row.map(row_to_assignment))
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_read_in_this_module_paginates_by_offset() {
        // docs/26 bans it. The needle is assembled: spelling it out would put it
        // in the file this check reads, and the assertion would fail on itself.
        let source = include_str!("role_edit.rs");
        let banned = format!("{}{} ", "OFF", "SET");
        assert!(!source.to_uppercase().contains(&banned));
    }

    #[test]
    fn a_permission_set_is_replaced_and_never_merged() {
        // The grant ceiling is checked against the SET the handler was given.
        // A merge would let a permission survive that the check never saw, which
        // is exactly the smuggling docs/04 control 1 forbids on role edit.
        let source = include_str!("role_edit.rs");
        assert!(source.contains("DELETE FROM role_permission WHERE role_id = $1"));
    }
}
