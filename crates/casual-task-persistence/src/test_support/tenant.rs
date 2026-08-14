//! Tenant and workflow fixtures; changes when workspace bootstrap changes.

use uuid::Uuid;

/// How many deliveries a consumer has in each state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub dispatched: i64,
    /// Not dispatched, not dead-lettered, currently claimed by some worker.
    pub claimed: i64,
    /// Not dispatched and not dead-lettered, whether claimed or not.
    pub outstanding: i64,
    pub dead_lettered: i64,
}

/// Insert a workspace. The smallest row that satisfies the tenant foreign keys.
///
/// # Errors
///
/// Any database error.
pub async fn insert_workspace(
    pool: &sqlx::PgPool,
    id: Uuid,
    slug: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO workspace (id, name, slug) VALUES ($1, $2, $2)")
        .bind(id)
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(())
}

/// Add a user to a workspace, so the membership check passes.
///
/// # Errors
///
/// Any database error.
pub async fn add_workspace_member(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO workspace_membership (workspace_id, user_id, member_type)
         VALUES ($1, $2, 'MEMBER')",
    )
    .bind(workspace_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Grant a user a role carrying `permissions`, at workspace scope.
///
/// `role_assignment` is the only source of authority in the system (migration
/// 0003), and nothing creates one yet — C-002 owns workspace bootstrap and the
/// built-in role templates. Until it lands, this is how an authorization test
/// puts a real grant in front of the resolver instead of asserting against a
/// stub.
///
/// # Errors
///
/// Any database error.
pub async fn grant_at_workspace(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    permissions: &[&str],
) -> Result<Uuid, sqlx::Error> {
    let role = Uuid::now_v7();
    sqlx::query("INSERT INTO role (id, workspace_id, name) VALUES ($1,$2,$3)")
        .bind(role)
        .bind(workspace_id)
        .bind(format!("test-{role}"))
        .execute(pool)
        .await?;
    for permission in permissions {
        sqlx::query("INSERT INTO role_permission (role_id, permission) VALUES ($1,$2)")
            .bind(role)
            .bind(*permission)
            .execute(pool)
            .await?;
    }
    sqlx::query(
        "INSERT INTO role_assignment
             (id, workspace_id, principal_type, principal_id, role_id,
              scope_type, scope_id, granted_by)
         VALUES ($1,$2,'USER'::principal_type,$3,$4,'WORKSPACE'::scope_type,$2,$3)",
    )
    .bind(Uuid::now_v7())
    .bind(workspace_id)
    .bind(user_id)
    .bind(role)
    .execute(pool)
    .await?;
    Ok(role)
}

/// A team in `workspace`, returning its id.
///
/// # Errors
///
/// Any database error.
pub async fn insert_team(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    name: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO team (id, workspace_id, name) VALUES ($1,$2,$3)")
        .bind(id)
        .bind(workspace_id)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(id)
}

/// Put a user in a team.
///
/// # Errors
///
/// Any database error.
pub async fn add_team_member(
    pool: &sqlx::PgPool,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    // `team_membership` carries no workspace_id: the team does, and a team is in
    // exactly one workspace (migration 0002).
    sqlx::query(
        "INSERT INTO team_membership (team_id, user_id) VALUES ($1,$2)
         ON CONFLICT DO NOTHING",
    )
    .bind(team_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Put a team on a project.
///
/// # Errors
///
/// Any database error.
pub async fn add_project_team(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    project_id: Uuid,
    team_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO project_team (workspace_id, project_id, team_id) VALUES ($1,$2,$3)
         ON CONFLICT DO NOTHING",
    )
    .bind(workspace_id)
    .bind(project_id)
    .bind(team_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Grant a role carrying `permissions` to a **team**, scoped to that team.
///
/// Scoped to the *team*, not to the project, and that distinction is the whole
/// point. A grant scoped to a project reaches it however the principal is
/// expanded — the team is only who holds it. A grant scoped to the **team**
/// reaches a project's tasks only when that team is in the project's scope
/// chain, which is exactly what `project_team` decides.
///
/// # Errors
///
/// Any database error.
pub async fn grant_to_team_at_team_scope(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    team_id: Uuid,
    granted_by: Uuid,
    permissions: &[&str],
) -> Result<Uuid, sqlx::Error> {
    let role = Uuid::now_v7();
    sqlx::query("INSERT INTO role (id, workspace_id, name) VALUES ($1,$2,$3)")
        .bind(role)
        .bind(workspace_id)
        .bind(format!("test-team-{role}"))
        .execute(pool)
        .await?;
    for permission in permissions {
        sqlx::query("INSERT INTO role_permission (role_id, permission) VALUES ($1,$2)")
            .bind(role)
            .bind(*permission)
            .execute(pool)
            .await?;
    }
    sqlx::query(
        "INSERT INTO role_assignment
             (id, workspace_id, principal_type, principal_id, role_id,
              scope_type, scope_id, granted_by)
         VALUES ($1,$2,'TEAM'::principal_type,$3,$4,'TEAM'::scope_type,$3,$5)",
    )
    .bind(Uuid::now_v7())
    .bind(workspace_id)
    .bind(team_id)
    .bind(role)
    .bind(granted_by)
    .execute(pool)
    .await?;
    Ok(role)
}

/// Grant a user a role carrying `permissions`, narrowed by `constraints`.
///
/// The constrained form exists because a constrained grant and an unconstrained
/// one take different paths through the resolver's combining rule, and
/// `/permissions/effective` reports them differently — one is exercisable
/// everywhere in the scope and one only where its constraints hold. A suite
/// that could only build unconstrained grants could not tell those apart.
///
/// `constraints` is the same JSON shape the `role_assignment.constraints`
/// column stores, in `docs/04` §Constraint set's snake_case.
///
/// # Errors
///
/// Any database error.
pub async fn grant_at_workspace_constrained(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    permissions: &[&str],
    constraints: serde_json::Value,
) -> Result<Uuid, sqlx::Error> {
    let role = Uuid::now_v7();
    sqlx::query("INSERT INTO role (id, workspace_id, name) VALUES ($1,$2,$3)")
        .bind(role)
        .bind(workspace_id)
        .bind(format!("test-{role}"))
        .execute(pool)
        .await?;
    for permission in permissions {
        sqlx::query("INSERT INTO role_permission (role_id, permission) VALUES ($1,$2)")
            .bind(role)
            .bind(*permission)
            .execute(pool)
            .await?;
    }
    sqlx::query(
        "INSERT INTO role_assignment
             (id, workspace_id, principal_type, principal_id, role_id,
              scope_type, scope_id, granted_by, constraints)
         VALUES ($1,$2,'USER'::principal_type,$3,$4,'WORKSPACE'::scope_type,$2,$3,$5)",
    )
    .bind(Uuid::now_v7())
    .bind(workspace_id)
    .bind(user_id)
    .bind(role)
    .bind(constraints)
    .execute(pool)
    .await?;
    Ok(role)
}

/// How many history rows one aggregate has: activity, audit, outbox, delivery.
///
/// ADR-006 makes all four a property of a single transaction, so a test that
/// asserts on the domain row alone would pass with the eventing deleted.
///
/// # Errors
///
/// Any database error.
pub async fn history_counts(
    pool: &sqlx::PgPool,
    aggregate_id: Uuid,
) -> Result<(i64, i64, i64, i64), sqlx::Error> {
    let activity: i64 =
        sqlx::query_scalar("SELECT count(*) FROM activity_event WHERE aggregate_id = $1")
            .bind(aggregate_id)
            .fetch_one(pool)
            .await?;
    let audit: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_event WHERE target_id = $1")
        .bind(aggregate_id)
        .fetch_one(pool)
        .await?;
    let outbox: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox_event WHERE aggregate_id = $1")
            .bind(aggregate_id)
            .fetch_one(pool)
            .await?;
    let deliveries: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_delivery d
           JOIN outbox_event e ON e.id = d.event_id
          WHERE e.aggregate_id = $1",
    )
    .bind(aggregate_id)
    .fetch_one(pool)
    .await?;
    Ok((activity, audit, outbox, deliveries))
}

/// The status names of one workflow, in board order.
///
/// # Errors
///
/// Any database error.
pub async fn workflow_status_names(
    pool: &sqlx::PgPool,
    workflow_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT name FROM workflow_status WHERE workflow_id = $1 ORDER BY position")
        .bind(workflow_id)
        .fetch_all(pool)
        .await
}

/// The event types recorded in the outbox for one aggregate, oldest first.
///
/// # Errors
///
/// Any database error.
pub async fn outbox_event_types(
    pool: &sqlx::PgPool,
    aggregate_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT event_type FROM outbox_event WHERE aggregate_id = $1 ORDER BY created_at, id",
    )
    .bind(aggregate_id)
    .fetch_all(pool)
    .await
}

/// The default workflow's statuses, as `(name, id)`, in board order.
///
/// A transition test needs the id of "Todo" and there is no endpoint that
/// serves one yet — workflow reads are C-007's `GET /workflows/{id}`.
///
/// # Errors
///
/// Any database error.
pub async fn default_status_ids(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
) -> Result<Vec<(String, Uuid)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT s.name, s.id
           FROM workflow_status s
           JOIN workflow w ON w.id = s.workflow_id
          WHERE w.workspace_id = $1 AND w.is_default
          ORDER BY s.position",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}
