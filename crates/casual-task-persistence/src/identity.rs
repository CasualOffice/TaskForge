//! Reading and writing credentials and sessions (C-001, `docs/40`).
//!
//! None of these tables carry `workspace_id` — a session and a password belong
//! to a person, not a tenant — so they take a plain connection rather than a
//! [`Scoped`](crate::Scoped). That is not an exemption being taken quietly:
//! migration 0016 records the reason, and `tests/schema/assertions.sql` names
//! each table so the gate that would otherwise flag them cannot be silenced by
//! accident.

use time::OffsetDateTime;
use uuid::Uuid;

/// What a login attempt needs to know about an account.
#[derive(Debug, Clone)]
pub struct Credential {
    pub user_id: Uuid,
    pub password_hash: String,
    pub failed_attempts: i32,
    pub locked_until: Option<OffsetDateTime>,
}

/// A session as stored.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub verifier_hash: String,
    pub auth_method: String,
    pub mfa_satisfied_at: Option<OffsetDateTime>,
    pub expires_at: OffsetDateTime,
}

/// Find the credential for an email address.
///
/// Returns `None` for an unknown address **and** for a tombstoned account, so
/// the caller cannot distinguish them — `docs/40` §Local authentication:
/// "account enumeration through the login endpoint is the most commonly
/// shipped auth bug".
///
/// # Errors
///
/// Any database error.
pub async fn credential_for_email(
    conn: &mut sqlx::PgConnection,
    email: &str,
) -> Result<Option<Credential>, sqlx::Error> {
    let row: Option<(Uuid, String, i32, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT c.user_id, c.password_hash, c.failed_attempts, c.locked_until
           FROM user_credential c
           JOIN user_account u ON u.id = c.user_id
          WHERE u.email = $1::citext
            AND u.is_tombstone = false",
    )
    .bind(email)
    .fetch_optional(conn)
    .await?;

    Ok(row.map(
        |(user_id, password_hash, failed_attempts, locked_until)| Credential {
            user_id,
            password_hash,
            failed_attempts,
            locked_until,
        },
    ))
}

/// Record a failed attempt and the resulting backoff.
///
/// # Errors
///
/// Any database error.
pub async fn record_failure(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    locked_until: Option<OffsetDateTime>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_credential
            SET failed_attempts = failed_attempts + 1, locked_until = $2
          WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(locked_until)
    .execute(conn)
    .await?;
    Ok(())
}

/// Clear the backoff after a successful authentication.
///
/// # Errors
///
/// Any database error.
pub async fn clear_failures(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_credential SET failed_attempts = 0, locked_until = NULL WHERE user_id = $1",
    )
    .bind(user_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// Create a session.
///
/// # Errors
///
/// Any database error.
#[allow(clippy::too_many_arguments)]
pub async fn create_session(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    selector: &str,
    verifier_hash: &str,
    auth_method: &str,
    expires_at: OffsetDateTime,
    ip: Option<&str>,
    user_agent: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO session
             (id, user_id, selector, verifier_hash, auth_method, expires_at,
              ip_address, user_agent)
         VALUES ($1,$2,$3,$4,$5,$6,$7::inet,$8)",
    )
    .bind(id)
    .bind(user_id)
    .bind(selector)
    .bind(verifier_hash)
    .bind(auth_method)
    .bind(expires_at)
    .bind(ip)
    .bind(user_agent)
    .execute(conn)
    .await?;
    Ok(id)
}

/// Load a live session by selector.
///
/// Expired and revoked sessions return `None`. **Revocation is immediate** —
/// `docs/40` rejects JWTs for exactly this reason, and nothing caches this row
/// (ADR-032 withdrew the read-through cache because a cache reintroduces the
/// staleness window that argument rejects).
///
/// A session created before the account's password changed also returns `None`:
/// `docs/40` §Local authentication requires a password change to invalidate
/// existing sessions, and doing it here means every entry point gets it rather
/// than the ones that remember.
///
/// # Errors
///
/// Any database error.
pub async fn live_session(
    conn: &mut sqlx::PgConnection,
    selector: &str,
) -> Result<Option<SessionRecord>, sqlx::Error> {
    let row: Option<(
        Uuid,
        Uuid,
        String,
        String,
        Option<OffsetDateTime>,
        OffsetDateTime,
    )> = sqlx::query_as(
        "SELECT s.id, s.user_id, s.verifier_hash, s.auth_method,
                    s.mfa_satisfied_at, s.expires_at
               FROM session s
               LEFT JOIN user_credential c ON c.user_id = s.user_id
              WHERE s.selector = $1
                AND s.revoked_at IS NULL
                AND s.expires_at > now()
                AND (c.changed_at IS NULL OR s.created_at >= c.changed_at)",
    )
    .bind(selector)
    .fetch_optional(conn)
    .await?;

    Ok(row.map(
        |(id, user_id, verifier_hash, auth_method, mfa_satisfied_at, expires_at)| SessionRecord {
            id,
            user_id,
            verifier_hash,
            auth_method,
            mfa_satisfied_at,
            expires_at,
        },
    ))
}

/// Revoke one session. Idempotent.
///
/// # Errors
///
/// Any database error.
pub async fn revoke_session(conn: &mut sqlx::PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE session SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Revoke every session for a user — "sign out everywhere".
///
/// Returns how many were revoked.
///
/// # Errors
///
/// Any database error.
pub async fn revoke_all_sessions(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE session SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(conn)
    .await?
    .rows_affected())
}

/// Update `last_seen_at`.
///
/// # Errors
///
/// Any database error.
pub async fn touch_session(conn: &mut sqlx::PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE session SET last_seen_at = now() WHERE id = $1")
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}
