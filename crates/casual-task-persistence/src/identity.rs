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

/// How long a session may go unused. `docs/40`: 14 days idle.
pub const IDLE_LIFETIME: &str = "14 days";

/// How long a session may live at all, however active. `docs/40`: 30 days
/// absolute. This is the bound that ends a session someone left open.
pub const ABSOLUTE_LIFETIME: &str = "30 days";

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
/// **Idle and absolute lifetimes are both enforced** (`docs/40`: 14 d idle /
/// 30 d absolute). A single `expires_at` cannot express both: a session used
/// once a day would live forever under an idle-only rule, and one left open in
/// a browser tab stays valid for its whole absolute life under an
/// expiry-only rule. Both bounds are in the query, so no caller can apply one
/// and forget the other.
///
/// A tombstoned account's sessions are dead immediately, for the same reason
/// revocation is: deactivating a person who is currently signed in has to end
/// the session they are holding, not the next one they create.
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
               JOIN user_account u ON u.id = s.user_id
              WHERE s.selector = $1
                AND s.revoked_at IS NULL
                AND s.expires_at > now()
                AND u.is_tombstone = false
                AND s.last_seen_at > now() - $2::interval
                AND s.created_at   > now() - $3::interval
                AND (c.changed_at IS NULL OR s.created_at >= c.changed_at)",
    )
    .bind(selector)
    .bind(IDLE_LIFETIME)
    .bind(ABSOLUTE_LIFETIME)
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

/// How stale `last_seen_at` may get before it is written again.
///
/// See [`touch_session`]. Five minutes is far finer than any use of the value —
/// a session list showing "active 4 minutes ago" as "active just now" is not a
/// defect — and it turns a write on every request into a write every few
/// hundred.
pub const LAST_SEEN_RESOLUTION: &str = "5 minutes";

/// Update `last_seen_at`, **at most once per [`LAST_SEEN_RESOLUTION`]**.
///
/// The throttle is the point, and it is in the `WHERE` clause rather than in a
/// read-then-write so it stays one statement and one round trip.
///
/// Written unconditionally, this ran an `UPDATE` on **every authenticated
/// request**. That is a hot row per active session: every read becomes a write,
/// every write is WAL and a dead tuple, and the table an authentication path
/// depends on is the one autovacuum is chasing. `docs/30` sets p95 read
/// < 150 ms, and a row-level lock on the session being used by the request in
/// front of you is not the way to hold it.
///
/// # Errors
///
/// Any database error.
pub async fn touch_session(conn: &mut sqlx::PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE session SET last_seen_at = now()
          WHERE id = $1 AND last_seen_at < now() - $2::interval",
    )
    .bind(id)
    .bind(LAST_SEEN_RESOLUTION)
    .execute(conn)
    .await?;
    Ok(())
}

/// Record an authentication event (`docs/40` §What is audited).
///
/// Best-effort at the call site is not acceptable here — a failed login that
/// was not recorded is the one an incident responder needs — so this returns a
/// `Result` and callers log loudly rather than discarding it.
///
/// # Errors
///
/// Any database error.
pub async fn record_auth_event(
    conn: &mut sqlx::PgConnection,
    user_id: Option<Uuid>,
    email: Option<&str>,
    event_type: &str,
    ip: Option<&str>,
    user_agent: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO auth_event (id, user_id, email, event_type, ip_address, user_agent)
         VALUES ($1,$2,$3::citext,$4,$5::inet,$6)",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(email)
    .bind(event_type)
    .bind(ip)
    .bind(user_agent)
    .execute(conn)
    .await?;
    Ok(())
}

/// Whether a user is a member of a workspace.
///
/// `docs/05` §Authentication: the workspace "is validated against membership on
/// every request — never trusted from the client". This is that validation, and
/// it deliberately returns a plain `bool` rather than a membership row: a
/// caller that received the row would be tempted to read a role out of it, and
/// authority comes from the resolver in `casual-task-authz`, not from
/// membership.
///
/// Runs unscoped, because it is what *establishes* the scope — the request has
/// no `WorkspaceScope` until this returns true. It is the second and last
/// unscoped read in the system, after the pre-workspace credential seam, and
/// like that one it returns the narrowest possible answer.
///
/// # Errors
///
/// Any database error.
pub async fn is_workspace_member(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    workspace_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM workspace_membership m
             WHERE m.user_id = $1 AND m.workspace_id = $2)",
    )
    .bind(user_id)
    .bind(workspace_id)
    .fetch_one(conn)
    .await
}
