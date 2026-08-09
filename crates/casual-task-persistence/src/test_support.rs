//! Database fixtures for integration tests in *other* crates.
//!
//! # Why this is here and not in the test that needs it
//!
//! `docs/19` §Boundary invariants: **all SQL lives in this crate**, and
//! `casual-task-lint` makes that a build failure rather than a review comment.
//! The C-011 acceptance gate lives in `casual-task-worker` — it has to, because
//! it asserts what happens when a *worker* is killed mid-batch — and it needs to
//! seed a workspace, age a claim past its expiry, and count delivery states.
//!
//! Two ways to allow that were rejected:
//!
//! - **Exempt `tests/` from the lint.** That is a hole in an architecture
//!   invariant, opened to make one test compile, and it would stay open.
//! - **Add the queries to the production API.** "Expire every claim" exists only
//!   to make a five-minute timeout testable in five milliseconds. A production
//!   surface that carries it is a production surface someone can call.
//!
//! So the SQL lives where the invariant says it must, and is compiled only when
//! a test asks for it.
//!
//! # Not compiled unless requested
//!
//! Behind the non-default `test-support` feature. A release build does not
//! contain [`expire_all_claims`]; there is no flag that reaches it.

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

/// The backoff state of an account, for tests that assert on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockoutState {
    pub failed_attempts: i32,
    pub locked: bool,
    /// Whether the lock extends more than an hour into the future — the shape
    /// of a permanent lockout, which `docs/40` forbids.
    pub locked_beyond_an_hour: bool,
}

/// Insert a user account and its password credential.
///
/// # Errors
///
/// Any database error.
pub async fn insert_user_with_password(
    pool: &sqlx::PgPool,
    id: Uuid,
    email: &str,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO user_account (id, email, display_name) VALUES ($1, $2, 'Test')")
        .bind(id)
        .bind(email)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO user_credential (user_id, password_hash) VALUES ($1, $2)")
        .bind(id)
        .bind(password_hash)
        .execute(pool)
        .await?;
    Ok(())
}

/// How many sessions are neither revoked nor expired.
///
/// # Errors
///
/// Any database error.
pub async fn live_session_count(pool: &sqlx::PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM session WHERE revoked_at IS NULL AND expires_at > now()",
    )
    .fetch_one(pool)
    .await
}

/// The account's current backoff state.
///
/// # Errors
///
/// Any database error.
pub async fn lockout_state(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<LockoutState, sqlx::Error> {
    let row: (i32, bool, Option<bool>) = sqlx::query_as(
        "SELECT failed_attempts,
                locked_until IS NOT NULL,
                locked_until > now() + interval '1 hour'
           FROM user_credential WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    Ok(LockoutState {
        failed_attempts: row.0,
        locked: row.1,
        locked_beyond_an_hour: row.2.unwrap_or(false),
    })
}

/// Lock an account for a fixed interval, so a test can assert what happens
/// *during* a backoff without depending on how long the real ladder's first
/// rung is.
///
/// # Errors
///
/// Any database error.
pub async fn lock_account(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    interval: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_credential SET locked_until = now() + $2::interval WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(interval)
    .execute(pool)
    .await?;
    Ok(())
}

/// Clear a backoff, simulating its expiry.
///
/// # Errors
///
/// Any database error.
pub async fn clear_lockout(pool: &sqlx::PgPool, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE user_credential SET locked_until = NULL WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Insert an API token. Returns nothing; the caller keeps the presented value.
///
/// # Errors
///
/// Any database error.
pub async fn insert_api_token(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    principal_id: Uuid,
    principal_type: &str,
    selector: &str,
    verifier_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO api_token
             (id, workspace_id, principal_type, principal_id, token_selector,
              verifier_hash, name)
         VALUES ($1,$2,$3::principal_type,$4,$5,$6,'test')",
    )
    .bind(Uuid::now_v7())
    .bind(workspace_id)
    .bind(principal_type)
    .bind(principal_id)
    .bind(selector)
    .bind(verifier_hash)
    .execute(pool)
    .await?;
    Ok(())
}

/// Authentication events recorded for an email address, newest first.
///
/// # Errors
///
/// Any database error.
pub async fn auth_events(pool: &sqlx::PgPool, email: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT event_type FROM auth_event WHERE email = $1::citext ORDER BY occurred_at DESC",
    )
    .bind(email)
    .fetch_all(pool)
    .await
}

/// Age a session so an idle or absolute lifetime bound applies to it.
///
/// # Errors
///
/// Any database error.
pub async fn age_session(
    pool: &sqlx::PgPool,
    last_seen: &str,
    created: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE session
            SET last_seen_at = now() - $1::interval,
                created_at   = now() - $2::interval",
    )
    .bind(last_seen)
    .bind(created)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a user tombstoned (deactivated).
///
/// # Errors
///
/// Any database error.
pub async fn tombstone_user(pool: &sqlx::PgPool, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE user_account SET is_tombstone = true WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Move a credential's `changed_at` forward, as a password change would.
///
/// # Errors
///
/// Any database error.
pub async fn mark_password_changed(pool: &sqlx::PgPool, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE user_credential SET changed_at = now() WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Age every outstanding reset token past its expiry.
///
/// `docs/40` gives a reset token one hour. Testing that by sleeping for one
/// hour means it is tested once and then disabled, so the clock is moved
/// instead of the test waiting for it.
///
/// # Errors
///
/// Any database error.
pub async fn expire_reset_tokens(pool: &sqlx::PgPool, user_id: Uuid) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE password_reset_token SET expires_at = now() - interval '1 second'
          WHERE user_id = $1 AND used_at IS NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected())
}

/// How many reset tokens a user has that are neither used nor expired.
///
/// # Errors
///
/// Any database error.
pub async fn live_reset_token_count(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM password_reset_token
          WHERE user_id = $1 AND used_at IS NULL AND expires_at > now()",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
}

/// Every stored reset-token column that could conceivably hold the credential.
///
/// Returned as text so a test can assert `docs/40`'s token-hash gate directly —
/// "a database dump contains no usable credential" — against what is actually
/// in the table rather than against what the writing code intended.
///
/// # Errors
///
/// Any database error.
pub async fn reset_token_columns(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT selector || ' ' || verifier_hash FROM password_reset_token WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// The stored password hash, so a test can assert a reset actually replaced it.
///
/// # Errors
///
/// Any database error.
pub async fn password_hash_of(pool: &sqlx::PgPool, user_id: Uuid) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT password_hash FROM user_credential WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
}

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

/// A task's status and state, read straight from the row.
///
/// Both together, because `docs/23`'s derived-state invariant is a claim about
/// the pair: reading one of them could not catch a drift between them.
///
/// # Errors
///
/// Any database error.
pub async fn task_status_and_state(
    pool: &sqlx::PgPool,
    task_id: Uuid,
) -> Result<(Uuid, String), sqlx::Error> {
    sqlx::query_as("SELECT status_id, state::text FROM task WHERE id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
}

/// Whether a task is soft-deleted.
///
/// # Errors
///
/// Any database error.
pub async fn task_is_deleted(pool: &sqlx::PgPool, task_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM task WHERE id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
}

/// A task's assignees.
///
/// # Errors
///
/// Any database error.
pub async fn task_assignees(pool: &sqlx::PgPool, task_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT user_id FROM task_assignee WHERE task_id = $1 ORDER BY assigned_at")
        .bind(task_id)
        .fetch_all(pool)
        .await
}

/// How many comments a task carries.
///
/// # Errors
///
/// Any database error.
pub async fn comment_count(pool: &sqlx::PgPool, task_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM comment WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
}

/// Create a tag. `project_id` of `None` is a workspace-scoped tag.
///
/// # Errors
///
/// Any database error.
pub async fn insert_tag(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    project_id: Option<Uuid>,
    name: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tag (id, workspace_id, project_id, name) VALUES ($1,$2,$3,$4::citext)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(project_id)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Record that `blocker` blocks `blocked` (`docs/23` step 7).
///
/// # Errors
///
/// Any database error.
pub async fn add_blocker(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    blocker: Uuid,
    blocked: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO task_dependency (from_task_id, to_task_id, workspace_id, kind)
         VALUES ($1,$2,$3,'BLOCKS')",
    )
    .bind(blocker)
    .bind(blocked)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// A workflow, a project and one task, as the smallest thing a projection test
/// can index.
///
/// Written out rather than driven through the API because the worker crate has
/// no HTTP: `task.status_id` and `project.workflow_id` are both `NOT NULL`, so
/// "one task" is unavoidably four rows.
///
/// # Errors
///
/// Any database error.
pub async fn insert_task_fixture(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    title: &str,
) -> Result<Uuid, sqlx::Error> {
    let workflow = Uuid::now_v7();
    let status = Uuid::now_v7();
    let project = Uuid::now_v7();
    let task = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO workflow (id, workspace_id, name, is_default) VALUES ($1,$2,'D',true)",
    )
    .bind(workflow)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO workflow_status
             (id, workflow_id, workspace_id, name, state, position, is_initial)
         VALUES ($1,$2,$3,'Backlog','BACKLOG'::task_state,1,true)",
    )
    .bind(status)
    .bind(workflow)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO project
             (id, workspace_id, key, name, workflow_id, created_by, visibility)
         VALUES ($1,$2,'WR','Work',$3,$4,'WORKSPACE'::visibility)",
    )
    .bind(project)
    .bind(workspace_id)
    .bind(workflow)
    .bind(user_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO task
             (id, workspace_id, project_id, number, title, status_id, state,
              reporter_id, position, created_by)
         VALUES ($1,$2,$3,1,$4,$5,'BACKLOG'::task_state,$6,'a0',$6)",
    )
    .bind(task)
    .bind(workspace_id)
    .bind(project)
    .bind(title)
    .bind(status)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(task)
}

/// Rebuild one task's search document, as the projection consumer would.
///
/// The consumer itself lives in `casual-task-worker` and is exercised by its
/// own test. This is for the API tests, whose subject is the *query* path: they
/// need a populated `task_search` and should not have to run a dispatch loop to
/// get one.
///
/// # Errors
///
/// Any database error.
pub async fn index_task(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    task_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let scope = casual_task_model::WorkspaceScope::for_job(
        casual_task_model::WorkspaceId::from_uuid(workspace_id),
    );
    let mut tx = pool.begin().await?;
    let mut scoped = crate::Scoped::apply(&mut tx, &scope).await?;
    let indexed = crate::search::refresh(&mut scoped, task_id).await?;
    tx.commit().await?;
    Ok(indexed)
}

/// How many rows the search projection holds for a workspace.
///
/// # Errors
///
/// Any database error.
pub async fn indexed_count(pool: &sqlx::PgPool, workspace_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM task_search WHERE workspace_id = $1")
        .bind(workspace_id)
        .fetch_one(pool)
        .await
}

/// Age every outstanding claim past [`crate::dispatch::CLAIM_EXPIRY`].
///
/// Simulates the passage of time so a test does not have to spend it. Testing
/// crash recovery by sleeping five minutes means it is tested once and then
/// disabled.
///
/// # Errors
///
/// Any database error.
pub async fn expire_all_claims(pool: &sqlx::PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE outbox_delivery SET claimed_at = now() - $1::interval
          WHERE claimed_at IS NOT NULL AND dispatched_at IS NULL",
    )
    .bind(format!(
        "{} seconds",
        crate::dispatch::CLAIM_EXPIRY.whole_seconds() + 60
    ))
    .execute(pool)
    .await?
    .rows_affected())
}

/// Delivery state counts for one consumer.
///
/// # Errors
///
/// Any database error.
pub async fn counts(pool: &sqlx::PgPool, consumer: &str) -> Result<Counts, sqlx::Error> {
    let row: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE dispatched_at IS NOT NULL),
                count(*) FILTER (WHERE dispatched_at IS NULL
                                   AND dead_lettered_at IS NULL
                                   AND claimed_at IS NOT NULL),
                count(*) FILTER (WHERE dispatched_at IS NULL
                                   AND dead_lettered_at IS NULL),
                count(*) FILTER (WHERE dead_lettered_at IS NOT NULL)
           FROM outbox_delivery
          WHERE consumer = $1",
    )
    .bind(consumer)
    .fetch_one(pool)
    .await?;

    Ok(Counts {
        dispatched: row.0,
        claimed: row.1,
        outstanding: row.2,
        dead_lettered: row.3,
    })
}

/// What the three streams recorded for one workspace (`docs/25`, ADR-006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    /// `activity_event.event_type`, oldest first.
    pub activity: Vec<String>,
    /// `audit_event.event_type`, oldest first.
    pub audit: Vec<String>,
    /// `outbox_event.event_type`, oldest first.
    pub outbox: Vec<String>,
    /// Rows in `outbox_delivery` for those events.
    pub deliveries: i64,
}

/// The history a workspace accumulated.
///
/// The point of asserting on all four at once is ADR-006's guarantee: the
/// domain change, the activity row, the audit row and the outbox event commit
/// together. A test that checked only the audit row would pass while the outbox
/// silently wrote nothing, and the missing events would surface months later as
/// a consumer that never fired.
///
/// # Errors
///
/// Any database error.
pub async fn history(pool: &sqlx::PgPool, workspace_id: Uuid) -> Result<History, sqlx::Error> {
    let activity: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM activity_event WHERE workspace_id = $1 ORDER BY id",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    let audit: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM audit_event WHERE workspace_id = $1 ORDER BY id",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    let outbox: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM outbox_event WHERE workspace_id = $1 ORDER BY id",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    let deliveries: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox_delivery WHERE workspace_id = $1")
            .bind(workspace_id)
            .fetch_one(pool)
            .await?;

    Ok(History {
        activity,
        audit,
        outbox,
        deliveries,
    })
}

/// A workspace's current `authz_epoch` (`docs/04` §Caching, ADR-012).
///
/// # Errors
///
/// Any database error.
pub async fn authz_epoch(pool: &sqlx::PgPool, workspace_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT authz_epoch FROM workspace WHERE id = $1")
        .bind(workspace_id)
        .fetch_one(pool)
        .await
}

/// Membership rows read **without** the tenant setting, exactly as a repository
/// that forgot to scope would read them.
///
/// Exists for the row-level-security assertion in `tests/workspace_seam.rs`: run
/// as `taskforge_app` this must return nothing, which is what makes the
/// `SECURITY DEFINER` seam in migration 0019 necessary rather than decorative.
///
/// # Errors
///
/// Any database error.
pub async fn unscoped_membership_count(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM workspace_membership WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
}

/// Age every live invitation in a workspace past its expiry.
///
/// `docs/40` gives an invitation seven days. Testing that by waiting a week
/// means it is tested once and then disabled, so the clock is moved instead.
///
/// # Errors
///
/// Any database error.
pub async fn expire_invitations(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE invitation SET expires_at = now() - interval '1 second'
          WHERE workspace_id = $1 AND accepted_at IS NULL AND revoked_at IS NULL",
    )
    .bind(workspace_id)
    .execute(pool)
    .await?
    .rows_affected())
}

/// How many invitations in a workspace are neither accepted, revoked nor
/// expired.
///
/// # Errors
///
/// Any database error.
pub async fn live_invitation_count(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM invitation
          WHERE workspace_id = $1 AND accepted_at IS NULL
            AND revoked_at IS NULL AND expires_at > now()",
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await
}

/// Every stored invitation column that could conceivably hold the credential.
///
/// Returned as text so a test can assert `docs/40`'s token-hash gate against
/// what is actually in the table rather than against what the writing code
/// intended to put there.
///
/// # Errors
///
/// Any database error.
pub async fn invitation_columns(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT selector || ' ' || verifier_hash FROM invitation WHERE workspace_id = $1",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

/// Whether a user holds a workspace membership row.
///
/// Read unscoped, as the database owner, on purpose: the question is whether
/// the row exists at all, and a scoped read would answer "no" for a row hidden
/// by a policy just as it would for a row that was never written.
///
/// # Errors
///
/// Any database error.
pub async fn is_member(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM workspace_membership
                         WHERE workspace_id = $1 AND user_id = $2)",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
}

/// The id of the account for an address, if there is one.
///
/// # Errors
///
/// Any database error.
pub async fn user_id_for_email(
    pool: &sqlx::PgPool,
    email: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM user_account WHERE email = $1::citext")
        .bind(email)
        .fetch_optional(pool)
        .await
}

/// Insert a bare user account, with no credential.
///
/// # Errors
///
/// Any database error.
pub async fn insert_user(
    pool: &sqlx::PgPool,
    id: Uuid,
    email: &str,
    display_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO user_account (id, email, display_name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(email)
        .bind(display_name)
        .execute(pool)
        .await?;
    Ok(())
}

/// The workspace-scope grants held by ONE user, as `(role_id, granted_by)`.
///
/// Distinct from [`workspace_grants`], which lists every grant in the
/// workspace. Two branches independently added a `workspace_grants` with
/// different signatures; both are wanted, so this one says whose grants it
/// returns.
pub async fn workspace_grants_for_user(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<(Uuid, Uuid)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT role_id, granted_by FROM role_assignment
          WHERE workspace_id = $1 AND principal_id = $2
            AND principal_type = 'USER'::principal_type
            AND scope_type = 'WORKSPACE'::scope_type",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
}
