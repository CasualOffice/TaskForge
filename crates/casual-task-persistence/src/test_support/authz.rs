//! Authorization and role fixtures; changes when the grant model changes.

use uuid::Uuid;

/// Every `WORKSPACE`-scope grant in a workspace, as
/// `(principal_id, role_name, permission)`.
///
/// The D-054 invariant, read back from the rows rather than from a repository
/// function — a test that asked the same code under test whether it had worked
/// would agree with itself.
///
/// # Errors
///
/// Any database error.
pub async fn workspace_grants(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
) -> Result<Vec<(Uuid, String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT ra.principal_id, r.name, rp.permission
           FROM role_assignment ra
           JOIN role r ON r.id = ra.role_id
           JOIN role_permission rp ON rp.role_id = ra.role_id
          WHERE ra.workspace_id = $1
            AND ra.scope_type = 'WORKSPACE'::scope_type
          ORDER BY r.name, rp.permission",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

/// The template roles of a workspace, `(name, permission count)`.
///
/// # Errors
///
/// Any database error.
pub async fn role_templates(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT r.name, count(rp.permission)
           FROM role r
           LEFT JOIN role_permission rp ON rp.role_id = r.id
          WHERE r.workspace_id = $1 AND r.is_template
          GROUP BY r.name
          ORDER BY r.name",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

/// The id of a workspace's owner assignment, if it has one.
///
/// # Errors
///
/// Any database error.
pub async fn owner_assignment(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT ra.id
           FROM role_assignment ra
           JOIN role_permission rp ON rp.role_id = ra.role_id
          WHERE ra.workspace_id = $1
            AND ra.scope_type = 'WORKSPACE'::scope_type
            AND rp.permission = 'workspace.owner'
          LIMIT 1",
    )
    .bind(workspace_id)
    .fetch_optional(pool)
    .await
}

/// Try to delete a role assignment, so a test can watch migration 0021's
/// trigger refuse it.
///
/// # Errors
///
/// The database error the trigger raises, which is the point.
pub async fn delete_role_assignment(pool: &sqlx::PgPool, id: Uuid) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM role_assignment WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected())
}

/// Point a role assignment at a different role, so a test can watch the
/// "downgraded" half of `docs/04` control 4.
///
/// # Errors
///
/// The database error the trigger raises.
pub async fn move_role_assignment(
    pool: &sqlx::PgPool,
    id: Uuid,
    role_id: Uuid,
) -> Result<u64, sqlx::Error> {
    Ok(
        sqlx::query("UPDATE role_assignment SET role_id = $2 WHERE id = $1")
            .bind(id)
            .bind(role_id)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

/// Grant an existing role to a user at `WORKSPACE` scope. Returns the
/// assignment id.
///
/// # Errors
///
/// Any database error.
pub async fn grant_role_at_workspace(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    role_id: Uuid,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO role_assignment
             (id, workspace_id, principal_type, principal_id, role_id,
              scope_type, scope_id, granted_by)
         VALUES ($1, $2, 'USER'::principal_type, $3, $4,
                 'WORKSPACE'::scope_type, $2, $3)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(user_id)
    .bind(role_id)
    .execute(pool)
    .await?;
    Ok(id)
}

/// A workspace's template role by name.
///
/// # Errors
///
/// Any database error.
pub async fn role_by_name(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    name: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM role WHERE workspace_id = $1 AND name = $2")
        .bind(workspace_id)
        .bind(name)
        .fetch_optional(pool)
        .await
}

/// The `changes` column of the audit rows for one target, newest first.
///
/// # Errors
///
/// Any database error.
pub async fn audit_changes(
    pool: &sqlx::PgPool,
    target_id: Uuid,
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT changes FROM audit_event WHERE target_id = $1 ORDER BY occurred_at DESC",
    )
    .bind(target_id)
    .fetch_all(pool)
    .await
}
