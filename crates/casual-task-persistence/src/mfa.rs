//! MFA factors, recovery codes, and the replay guard (C-001, `docs/40` §MFA).
//!
//! # The failure this module exists to prevent
//!
//! Three, and each is a single `WHERE` clause away from being reintroduced:
//!
//! 1. **An unconfirmed factor satisfying MFA.** `confirmed_at IS NOT NULL` is
//!    in [`confirmed_factor`]'s query, not at its call sites. A user who
//!    abandoned enrolment halfway — scanned the QR code, closed the tab — would
//!    otherwise be locked out by a factor they do not have, which is the exact
//!    failure `docs/40` and migration 0016's comment both call out.
//! 2. **Replaying an observed code.** [`accept_step`] is an `UPDATE` whose
//!    predicate is `last_step IS NULL OR last_step < $2`, so a step at or below
//!    the highest already accepted updates nothing and the caller is told.
//!    RFC 6238 §5.2 requires this, and `Totp::verify` returns the step
//!    precisely so it is possible.
//! 3. **A recovery code surviving its use.** [`redeem_recovery_code`] burns by
//!    id under `used_at IS NULL`, so concurrent redemptions of the same code
//!    resolve to one winner.
//!
//! None of these tables carry `workspace_id` — a factor belongs to a person,
//! not a tenant — so they take a plain connection rather than a
//! [`Scoped`], and migration 0016 records that exemption with
//! its reason.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// The only factor kind that exists. `docs/40` §MFA makes TOTP the baseline and
/// WebAuthn the preferred second factor "later"; the column is text so that
/// later needs no migration, and this constant is the one place the current
/// value is written.
pub const KIND_TOTP: &str = "totp";

/// A factor as stored.
///
/// The secret is **not** in this struct. Reading it is [`factor_secret`], a
/// separate call with a separate name, for the same reason migration 0016 gives
/// `lookup_api_token_verifier` its own door: finding a factor and reading the
/// one recoverable plaintext in the schema should not be the same operation.
#[derive(Debug, Clone, Copy)]
pub struct Factor {
    pub id: Uuid,
    /// The highest TOTP step accepted so far, or `None` if none has been.
    pub last_step: Option<i64>,
}

/// Begin enrolment: store an **unconfirmed** factor.
///
/// Replaces any existing unconfirmed factor for the user, so restarting
/// enrolment after closing the tab works rather than colliding with the
/// `UNIQUE (user_id, kind)` constraint. It deliberately does **not** replace a
/// *confirmed* one — re-enrolling over a working factor without proving control
/// of the current one would make enrolment a way to displace it.
///
/// # Errors
///
/// Any database error. A unique violation means a confirmed factor already
/// exists; the caller decides what that means.
pub async fn begin_enrolment(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    secret: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO mfa_factor (id, user_id, kind, secret)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT (user_id, kind) DO UPDATE
            SET secret = EXCLUDED.secret,
                id = EXCLUDED.id,
                created_at = now(),
                last_step = NULL
          WHERE mfa_factor.confirmed_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .bind(KIND_TOTP)
    .bind(secret)
    .execute(conn)
    .await?;
    Ok(id)
}

/// The user's **confirmed** factor, if they have one.
///
/// `confirmed_at IS NOT NULL` is here rather than at the call sites. See the
/// module docs: a caller that had to remember the check is a caller that
/// eventually forgets, and forgetting it locks out everyone who abandoned
/// enrolment.
///
/// # Errors
///
/// Any database error.
pub async fn confirmed_factor(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<Option<Factor>, sqlx::Error> {
    let row: Option<(Uuid, Option<i64>)> = sqlx::query_as(
        "SELECT id, last_step FROM mfa_factor
          WHERE user_id = $1 AND kind = $2 AND confirmed_at IS NOT NULL",
    )
    .bind(user_id)
    .bind(KIND_TOTP)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|(id, last_step)| Factor { id, last_step }))
}

/// The user's **pending** factor, if enrolment has begun and not finished.
///
/// # Errors
///
/// Any database error.
pub async fn pending_factor(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<Option<Factor>, sqlx::Error> {
    let row: Option<(Uuid, Option<i64>)> = sqlx::query_as(
        "SELECT id, last_step FROM mfa_factor
          WHERE user_id = $1 AND kind = $2 AND confirmed_at IS NULL",
    )
    .bind(user_id)
    .bind(KIND_TOTP)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|(id, last_step)| Factor { id, last_step }))
}

/// Whether the user has a confirmed factor.
///
/// A `bool` rather than the row: this answers `docs/40`'s "the enforcing admin
/// must already have MFA enrolled" check, and a caller handed the factor would
/// be tempted to do something else with it.
///
/// # Errors
///
/// Any database error.
pub async fn has_confirmed_factor(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM mfa_factor
                         WHERE user_id = $1 AND confirmed_at IS NOT NULL)",
    )
    .bind(user_id)
    .fetch_one(conn)
    .await
}

/// The stored base32 secret for a factor.
///
/// Its own function, deliberately narrow: this is the **one recoverable
/// plaintext in the schema** (migration 0016 says so), and it exists only
/// because TOTP verification recomputes the code from it. The value must be
/// wrapped before it goes anywhere near a log — `casual-task-observability`'s
/// `Redacted<T>` is what for.
///
/// # Errors
///
/// Any database error.
pub async fn factor_secret(
    conn: &mut sqlx::PgConnection,
    factor_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT secret FROM mfa_factor WHERE id = $1")
        .bind(factor_id)
        .fetch_optional(conn)
        .await
}

/// Confirm a factor, recording the step that proved it.
///
/// Returns `false` if it was already confirmed, so a replayed confirmation is
/// visible to the caller rather than silently idempotent.
///
/// # Errors
///
/// Any database error.
pub async fn confirm_factor(
    conn: &mut sqlx::PgConnection,
    factor_id: Uuid,
    step: i64,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE mfa_factor SET confirmed_at = now(), last_step = $2
          WHERE id = $1 AND confirmed_at IS NULL",
    )
    .bind(factor_id)
    .bind(step)
    .execute(conn)
    .await?
    .rows_affected();
    Ok(affected == 1)
}

/// Accept a TOTP step, or refuse it as a replay.
///
/// **RFC 6238 §5.2, as a `WHERE` clause.** `last_step < $2` means a step at or
/// below the highest already accepted updates no row and returns `false`. The
/// predicate is here rather than in a preceding `SELECT` so that two requests
/// presenting the same observed code cannot both pass the check before either
/// writes — which is exactly the race an attacker with a captured code is in.
///
/// Monotonic on purpose: refusing every *earlier* step as well is what closes
/// the window on a code captured seconds ago and presented after the clock
/// ticks on.
///
/// # Errors
///
/// Any database error.
pub async fn accept_step(
    conn: &mut sqlx::PgConnection,
    factor_id: Uuid,
    step: i64,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE mfa_factor SET last_step = $2
          WHERE id = $1
            AND confirmed_at IS NOT NULL
            AND (last_step IS NULL OR last_step < $2)",
    )
    .bind(factor_id)
    .bind(step)
    .execute(conn)
    .await?
    .rows_affected();
    Ok(affected == 1)
}

/// Remove the user's factor and every recovery code with it.
///
/// Returns whether there was one. The codes go too: they are bypasses for a
/// factor that no longer exists, and leaving them behind would let someone who
/// copied the list authenticate against nothing.
///
/// # Errors
///
/// Any database error.
pub async fn disable(conn: &mut sqlx::PgConnection, user_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query("DELETE FROM recovery_code WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    let affected = sqlx::query("DELETE FROM mfa_factor WHERE user_id = $1")
        .bind(user_id)
        .execute(conn)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

/// Replace the user's recovery codes with a fresh set.
///
/// Replace, never append. `docs/40` says the ten codes are "shown once"; a
/// second issue that added to the first would leave codes live that the user
/// believes were superseded, and no screen would ever show the true list.
///
/// # Errors
///
/// Any database error.
pub async fn replace_recovery_codes(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    hashes: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM recovery_code WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    for hash in hashes {
        sqlx::query("INSERT INTO recovery_code (id, user_id, code_hash) VALUES ($1,$2,$3)")
            .bind(Uuid::now_v7())
            .bind(user_id)
            .bind(hash)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Every unused recovery code for a user, as `(id, hash)`.
///
/// The whole set, because a recovery code is Argon2-hashed with a per-row salt
/// and therefore cannot be looked up by value — the caller compares against
/// each. Served by `recovery_code_user_ix`, which is partial on
/// `used_at IS NULL`, so the scan is over at most ten rows.
///
/// # Errors
///
/// Any database error.
pub async fn unused_recovery_codes(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as("SELECT id, code_hash FROM recovery_code WHERE user_id = $1 AND used_at IS NULL")
        .bind(user_id)
        .fetch_all(conn)
        .await
}

/// Burn a recovery code, reporting whether **this** call burned it.
///
/// `used_at IS NULL` in the `WHERE` clause, as everywhere else in this codebase
/// that something is single-use: two requests presenting the same code both
/// find it unused, both reach here, and exactly one succeeds.
///
/// # Errors
///
/// Any database error.
pub async fn redeem_recovery_code(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected =
        sqlx::query("UPDATE recovery_code SET used_at = now() WHERE id = $1 AND used_at IS NULL")
            .bind(id)
            .execute(conn)
            .await?
            .rows_affected();
    Ok(affected == 1)
}

/// How many recovery codes the user has left.
///
/// Shown to the user, so "you have two codes left" can prompt a re-issue before
/// the answer is zero and the only path left is break-glass.
///
/// # Errors
///
/// Any database error.
pub async fn remaining_recovery_codes(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM recovery_code WHERE user_id = $1 AND used_at IS NULL")
        .bind(user_id)
        .fetch_one(conn)
        .await
}

/// Record that this session has satisfied MFA.
///
/// On the **session**, not on the user: `docs/40` §Workspace-level SSO and MFA
/// step-up makes the session the thing that carries the assertion, so signing
/// in somewhere else does not inherit a step-up performed here.
///
/// # Errors
///
/// Any database error.
pub async fn mark_session_satisfied(
    conn: &mut sqlx::PgConnection,
    session_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE session SET mfa_satisfied_at = $2 WHERE id = $1")
        .bind(session_id)
        .bind(at)
        .execute(conn)
        .await?;
    Ok(())
}

/// Whether a workspace demands MFA.
///
/// Read **unscoped**, and that needs no seam: `workspace` is exempt from the
/// tenancy backstop because row identity *is* the tenant (migration 0010), and
/// this is asked during workspace resolution — before a scope exists, which is
/// the whole point of the policy living here rather than at login.
///
/// A workspace being deleted returns `false` rather than `true`: it is
/// unreachable anyway, and answering `true` would demand a step-up for a
/// workspace nobody can enter.
///
/// # Errors
///
/// Any database error.
pub async fn workspace_requires_mfa(
    conn: &mut sqlx::PgConnection,
    workspace_id: Uuid,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT require_mfa FROM workspace WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(workspace_id)
    .fetch_optional(conn)
    .await?
    .unwrap_or(false))
}

/// Turn a workspace's MFA requirement on or off.
///
/// Scoped, because this is tenant configuration and the caller is inside the
/// workspace by the time they can ask for it.
///
/// # Errors
///
/// Any database error.
pub async fn set_workspace_mfa(scoped: &mut Scoped<'_>, required: bool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE workspace SET require_mfa = $2, version = version + 1 WHERE id = $1")
        .bind(scoped.workspace_id().as_uuid())
        .bind(required)
        .execute(scoped.conn())
        .await?;
    Ok(())
}

/// Clear a user's MFA, for the break-glass path.
///
/// Identical in effect to [`disable`] and separate in name on purpose: this one
/// is reachable only from the operator command, and a distinct symbol is what
/// lets `git grep` answer "what can remove someone's second factor without
/// their code" with a complete list.
///
/// # Errors
///
/// Any database error.
pub async fn break_glass_clear(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    disable(conn, user_id).await
}
