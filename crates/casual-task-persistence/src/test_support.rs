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
