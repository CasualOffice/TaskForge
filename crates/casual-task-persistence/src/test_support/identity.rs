//! Credential and session fixtures; changes when authentication storage changes.

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
