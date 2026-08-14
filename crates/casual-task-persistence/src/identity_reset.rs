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

/// The tuple a profile row decodes into.
type ProfileTuple = (Uuid, Option<String>, String, Option<String>, Option<String>);

/// The tuple a session summary decodes into. Seven columns say nothing about
/// which is which inline, and clippy is right about that.
type SessionTuple = (
    Uuid,
    String,
    OffsetDateTime,
    OffsetDateTime,
    OffsetDateTime,
    Option<String>,
    Option<String>,
);

/// A person's own account, as they see it.
#[derive(Debug, Clone)]
pub struct Profile {
    pub id: Uuid,
    pub email: Option<String>,
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// IANA zone name. `None` means unset, which is **not** UTC — a user who
    /// has never chosen one is a user whose day boundary we do not know.
    pub time_zone: Option<String>,
}

/// Read a person's own account.
///
/// Not scoped to a workspace: an account is a person, and a person belongs to
/// many workspaces (`docs/03`). This is the one read in the product that is
/// deliberately outside the tenant boundary, which is why it takes a connection
/// rather than a `Scoped` and answers only about the caller.
///
/// # Errors
///
/// Any database error.
pub async fn profile(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<Option<Profile>, sqlx::Error> {
    let row: Option<ProfileTuple> = sqlx::query_as(
        "SELECT id, email::text, display_name, avatar_url, time_zone
               FROM user_account
              WHERE id = $1 AND is_tombstone = false",
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|r| Profile {
        id: r.0,
        email: r.1,
        display_name: r.2,
        avatar_url: r.3,
        time_zone: r.4,
    }))
}

/// Update a person's own account.
///
/// `None` leaves a field alone. Email is deliberately absent: changing it is a
/// verification flow (`docs/40`), not a field edit, and offering it here would
/// let an account be moved to an address nobody proved they hold.
///
/// # Errors
///
/// Any database error.
pub async fn update_profile(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    display_name: Option<&str>,
    time_zone: Option<Option<&str>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_account
            SET display_name = COALESCE($2, display_name),
                time_zone = CASE WHEN $3 THEN $4 ELSE time_zone END,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(user_id)
    .bind(display_name)
    .bind(time_zone.is_some())
    .bind(time_zone.flatten())
    .execute(conn)
    .await?;
    Ok(())
}

/// One of a person's live sessions.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: Uuid,
    pub auth_method: String,
    pub created_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

/// Every live session a person has.
///
/// `docs/40`: "Revocation is immediate: delete the row. Admin-visible session
/// list, with sign out everywhere." A person cannot act on a session they
/// cannot see, so this is the list that makes revocation usable.
///
/// Revoked and expired rows are excluded — a list that showed them would make
/// "sign out everywhere" look as though it had failed.
///
/// # Errors
///
/// Any database error.
pub async fn sessions_of(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<Vec<SessionSummary>, sqlx::Error> {
    let rows: Vec<SessionTuple> = sqlx::query_as(
        "SELECT id, auth_method, created_at, last_seen_at, expires_at,
                host(ip_address), user_agent
           FROM session
          WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now()
          ORDER BY last_seen_at DESC",
    )
    .bind(user_id)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| SessionSummary {
            id: r.0,
            auth_method: r.1,
            created_at: r.2,
            last_seen_at: r.3,
            expires_at: r.4,
            ip_address: r.5,
            user_agent: r.6,
        })
        .collect())
}

/// Whether a session belongs to a person.
///
/// Checked before revoking by id, so one user cannot end another's session by
/// guessing a uuid.
///
/// # Errors
///
/// Any database error.
pub async fn session_belongs_to(
    conn: &mut sqlx::PgConnection,
    session_id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM session WHERE id = $1 AND user_id = $2)")
        .bind(session_id)
        .bind(user_id)
        .fetch_one(conn)
        .await
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
