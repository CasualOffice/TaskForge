//! Membership, invitation, and MFA fixtures; changes when account policy changes.

use uuid::Uuid;

/// Membership rows read **without** the tenant setting, exactly as a repository
/// that forgot to scope would read them.
///
/// Exists for the row-level-security assertion in `tests/workspace_seam.rs`: run
/// as `taskforge_app` this must return nothing.
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
/// Distinct from [`crate::test_support::workspace_grants`], which lists every grant in the
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

/// Give `taskforge_app` a password so a test can connect AS the role RLS
/// applies to.
///
/// As the owner every tenant assertion passes with the predicates removed,
/// which is why tests that mean to exercise isolation must not use it.
///
/// # Errors
///
/// Any database error.
pub async fn enable_app_login(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw'")
        .execute(pool)
        .await?;
    Ok(())
}

/// The highest TOTP step accepted for a user's factor.
///
/// The replay guard's whole state. A test asserts it moved forward, which is
/// the thing RFC 6238 §5.2 depends on and the thing a refactor could silently
/// stop doing.
///
/// # Errors
///
/// Any database error.
pub async fn mfa_last_step(pool: &sqlx::PgPool, user_id: Uuid) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT last_step FROM mfa_factor WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map(Option::flatten)
}

/// Whether the user has a factor, and whether it is confirmed.
///
/// Returns `(exists, confirmed)` so a test can tell "no factor" from "a factor
/// nobody finished enrolling" — the distinction the whole unconfirmed-factor
/// rule turns on.
///
/// # Errors
///
/// Any database error.
pub async fn mfa_factor_state(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<(bool, bool), sqlx::Error> {
    let row: Option<(bool,)> =
        sqlx::query_as("SELECT confirmed_at IS NOT NULL FROM mfa_factor WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map_or((false, false), |(confirmed,)| (true, confirmed)))
}

/// How many recovery codes the user has, unused and used.
///
/// # Errors
///
/// Any database error.
pub async fn recovery_code_counts(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<(i64, i64), sqlx::Error> {
    sqlx::query_as(
        "SELECT count(*) FILTER (WHERE used_at IS NULL),
                count(*) FILTER (WHERE used_at IS NOT NULL)
           FROM recovery_code WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
}

/// Turn a workspace's MFA requirement on without going through the endpoint.
///
/// Used to set up the step-up tests, so they assert the *resolution* behaviour
/// rather than re-testing the toggle that switched it on.
///
/// # Errors
///
/// Any database error.
pub async fn require_workspace_mfa(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    required: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE workspace SET require_mfa = $2 WHERE id = $1")
        .bind(workspace_id)
        .bind(required)
        .execute(pool)
        .await?;
    Ok(())
}

/// Whether any live session for the user carries an MFA assertion.
///
/// # Errors
///
/// Any database error.
pub async fn session_mfa_satisfied(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM session
                         WHERE user_id = $1 AND revoked_at IS NULL
                           AND mfa_satisfied_at IS NOT NULL)",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
}
