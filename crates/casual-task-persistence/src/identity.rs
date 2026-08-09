//! Reading and writing credentials and sessions (C-001, `docs/40`).
//!
//! None of these tables carry `workspace_id` — a session and a password belong
//! to a person, not a tenant — so they take a plain connection rather than a
//! [`Scoped`](crate::Scoped). That is not an exemption being taken quietly:
//! migration 0016 records the reason, and `tests/schema/assertions.sql` names
//! each table so the gate that would otherwise flag them cannot be silenced by
//! accident.

use time::{Duration, OffsetDateTime};
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

// The membership check that turns an `Authenticated` caller into a
// `WorkspaceMember` used to live here, as a bare `SELECT EXISTS` over
// `workspace_membership`. It has moved to [`crate::workspace::is_member`],
// because that table carries `workspace_id` and therefore a row-level-security
// policy: run unscoped as `taskforge_app`, the policy hid every row and the
// check answered `false` for everyone. It passed its tests only because the
// harness connects as a superuser, for whom RLS is inert (migration 0012).
// Migration 0019 gives it the same `SECURITY DEFINER` seam the credential
// lookup already had.

/// The address on an account, or `None` if it has none.
///
/// `NULL` once the account is anonymized (ADR-026), which is why this is an
/// `Option` rather than a `String`: an anonymized account has no address, and
/// must therefore **fail** an invitation's address comparison rather than match
/// an empty string nobody was invited at.
///
/// Unscoped, like everything else keyed on a person — `user_account` is the one
/// table without a `workspace_id`.
///
/// # Errors
///
/// Any database error.
pub async fn email_of(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT email::text FROM user_account WHERE id = $1 AND is_tombstone = false",
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await
    .map(Option::flatten)
}

/// How long a password-reset token is valid. `docs/40` §Local authentication:
/// "Reset tokens: single-use, **1 h**, hashed at rest, invalidated by password
/// change."
///
/// Short on purpose. The token arrives by email, and an email sits in an inbox,
/// in a backup, and in a mail server's spool. An hour is long enough for a
/// person who just asked for it and short enough that a mailbox compromised
/// next week is not also an account takeover.
pub const RESET_LIFETIME: Duration = Duration::hours(1);

/// A reset token as stored — never the token itself.
#[derive(Debug, Clone)]
pub struct ResetToken {
    pub id: Uuid,
    pub user_id: Uuid,
    /// The salted hash of the verifier. The presented verifier is compared
    /// against this in `casual-task-identity`; a database dump therefore yields
    /// no usable reset link, which is the whole reason it is stored this way.
    pub verifier_hash: String,
}

/// Mint a reset-token row for a user.
///
/// The plaintext never reaches this function: it takes the selector and the
/// hash the caller has already split, so there is no signature through which
/// the credential could be written to the table by accident.
///
/// # Errors
///
/// Any database error.
pub async fn create_reset_token(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    selector: &str,
    verifier_hash: &str,
    expires_at: OffsetDateTime,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO password_reset_token
             (id, user_id, selector, verifier_hash, expires_at)
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(id)
    .bind(user_id)
    .bind(selector)
    .bind(verifier_hash)
    .bind(expires_at)
    .execute(conn)
    .await?;
    Ok(id)
}

/// Find a reset token that is neither used nor expired.
///
/// Both bounds are in the query rather than at the call site, for the reason
/// [`live_session`] carries its two lifetimes: a caller that has to remember to
/// check `used_at` is a caller that eventually forgets, and that forgetting
/// turns a single-use token into a reusable one.
///
/// A tombstoned account's tokens are dead for the same reason its sessions are
/// — deactivating a person must not leave a live way back in sitting in their
/// inbox.
///
/// # Errors
///
/// Any database error.
pub async fn live_reset_token(
    conn: &mut sqlx::PgConnection,
    selector: &str,
) -> Result<Option<ResetToken>, sqlx::Error> {
    let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT r.id, r.user_id, r.verifier_hash
           FROM password_reset_token r
           JOIN user_account u ON u.id = r.user_id
          WHERE r.selector = $1
            AND r.used_at IS NULL
            AND r.expires_at > now()
            AND u.is_tombstone = false",
    )
    .bind(selector)
    .fetch_optional(conn)
    .await?;

    Ok(row.map(|(id, user_id, verifier_hash)| ResetToken {
        id,
        user_id,
        verifier_hash,
    }))
}

/// Burn a reset token, returning whether **this** call was the one that burned
/// it.
///
/// `used_at IS NULL` is in the `WHERE` clause, not in a preceding `SELECT`.
/// That is what makes single use a property of the database rather than of the
/// order two requests happen to arrive in: two concurrent confirmations both
/// find a live token, both reach here, and exactly one updates a row. Reading
/// first and updating second is the same code with a race in it.
///
/// # Errors
///
/// Any database error.
pub async fn consume_reset_token(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE password_reset_token SET used_at = now()
          WHERE id = $1 AND used_at IS NULL AND expires_at > now()",
    )
    .bind(id)
    .execute(conn)
    .await?
    .rows_affected();
    Ok(affected == 1)
}

/// Invalidate every outstanding reset token for a user.
///
/// Someone who asks twice — because the first email was slow — must not be left
/// with a second working link in their inbox after using the first. `docs/40`
/// says a reset token is single-use; it says nothing about the *others*, and
/// leaving them live makes the exposure window the longest expiry rather than
/// the shortest.
///
/// # Errors
///
/// Any database error.
pub async fn invalidate_reset_tokens(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE password_reset_token SET used_at = now()
          WHERE user_id = $1 AND used_at IS NULL",
    )
    .bind(user_id)
    .execute(conn)
    .await?
    .rows_affected())
}

/// Set a new password, and move `changed_at` to now.
///
/// `changed_at` is not decoration: [`live_session`] refuses every session
/// created before it, which is how `docs/40`'s "invalidated by password change"
/// reaches sessions nobody remembered to revoke. The explicit
/// [`revoke_all_sessions`] beside it at the call site is the other half — this
/// one closes the door for any path that forgets, that one makes the closure
/// visible in the session list a user is shown.
///
/// The backoff is cleared in the same statement. Someone who has just proved
/// control of their mailbox and chosen a new password must not then be refused
/// by the failed attempts that made them reset it.
///
/// # Errors
///
/// Any database error.
pub async fn set_password(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_credential
            SET password_hash = $2,
                changed_at = now(),
                failed_attempts = 0,
                locked_until = NULL
          WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(conn)
    .await?;
    Ok(())
}
