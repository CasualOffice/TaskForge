//! Invitations (C-001, `docs/40` §Invitations).
//!
//! > "Invite by email, single-use, 7-day expiry, tied to the address."
//!
//! # Two halves, and the boundary between them is the whole design
//!
//! **Managing invitations is tenant work.** Creating, listing and revoking all
//! happen inside a workspace the caller is already a member of, so they take a
//! [`Scoped`] and are covered by `invitation`'s row-level-security policy like
//! any other tenant table.
//!
//! **Accepting one is not.** The acceptor may have no account at all — that is
//! the point of inviting by email — so there is no `WorkspaceScope` to apply,
//! and there cannot be one until the invitation itself says which workspace.
//! Those functions take a plain connection and go through the `SECURITY
//! DEFINER` seam in migration 0022, which ADR-032 §The pre-workspace seam named
//! this table for when it was written.
//!
//! The split is visible in the signatures, deliberately: a `Scoped` argument
//! means "inside a tenant", a bare connection means "through the seam". A
//! reader does not have to remember which is which.

use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::scoped::Scoped;

/// How long an invitation is valid. `docs/40` §Invitations: **7 days**.
///
/// Longer than a password reset's hour, and for the opposite reason: a reset is
/// answered by someone already waiting for it, while an invitation lands on
/// someone who was not expecting it and may be away for a week.
pub const INVITATION_LIFETIME: Duration = Duration::days(7);

/// The column tuple both reads project.
///
/// Named rather than repeated inline: the two queries must select the same
/// columns in the same order, and a tuple written twice is two places for that
/// to stop being true.
type Row = (
    Uuid,
    String,
    Option<Uuid>,
    Option<Uuid>,
    OffsetDateTime,
    OffsetDateTime,
);

/// An invitation as shown to the workspace that issued it.
///
/// No selector and no verifier hash. The link is shown to nobody but its
/// recipient — an admin listing invitations is entitled to know that an address
/// was invited, not to acquire the credential that accepts it.
#[derive(Debug, Clone)]
pub struct InvitationRecord {
    pub id: Uuid,
    pub email: String,
    pub role_id: Option<Uuid>,
    pub invited_by: Option<Uuid>,
    pub expires_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
}

/// What the acceptance path learns from a presented selector.
#[derive(Debug, Clone)]
pub struct PendingInvitation {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// The address the invitation is **tied to** (`docs/40`). The acceptance
    /// path compares this against the account being used; without that
    /// comparison an invitation is a bearer token for whoever reads the
    /// mailbox.
    pub email: String,
    pub role_id: Option<Uuid>,
}

/// Create an invitation.
///
/// The plaintext never reaches this function — it takes the selector and hash
/// the caller has already split, so there is no signature through which the
/// credential could be written to the table by accident. The same shape as
/// [`crate::identity::create_reset_token`], for the same reason.
///
/// # Errors
///
/// Any database error. A unique violation on `invitation_pending_ix` means a
/// live invitation for that address already exists; the caller decides what
/// that means.
pub async fn insert(
    scoped: &mut Scoped<'_>,
    email: &str,
    role_id: Option<Uuid>,
    invited_by: Uuid,
    selector: &str,
    verifier_hash: &str,
    expires_at: OffsetDateTime,
) -> Result<InvitationRecord, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let id = Uuid::now_v7();
    let row: Row = sqlx::query_as(
        "INSERT INTO invitation
                 (id, workspace_id, email, role_id, invited_by, selector,
                  verifier_hash, expires_at)
             VALUES ($1,$2,$3::citext,$4,$5,$6,$7,$8)
             RETURNING id, email::text, role_id, invited_by, expires_at, created_at",
    )
    .bind(id)
    .bind(workspace)
    .bind(email)
    .bind(role_id)
    .bind(invited_by)
    .bind(selector)
    .bind(verifier_hash)
    .bind(expires_at)
    .fetch_one(scoped.conn())
    .await?;

    Ok(InvitationRecord {
        id: row.0,
        email: row.1,
        role_id: row.2,
        invited_by: row.3,
        expires_at: row.4,
        created_at: row.5,
    })
}

/// The live invitations in this workspace, oldest id first, keyset-paginated.
///
/// `after` is the last id of the previous page. Keyset and not `OFFSET`:
/// `docs/26` bans offset pagination, and `casual-task-lint` makes it a build
/// failure rather than a review comment.
///
/// # Errors
///
/// Any database error.
pub async fn list_live(
    scoped: &mut Scoped<'_>,
    after: Option<Uuid>,
    limit: u32,
) -> Result<Vec<InvitationRecord>, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, email::text, role_id, invited_by, expires_at, created_at
               FROM invitation
              WHERE workspace_id = $1
                AND accepted_at IS NULL
                AND revoked_at IS NULL
                AND expires_at > now()
                AND ($2::uuid IS NULL OR id > $2)
              ORDER BY id
              LIMIT $3",
    )
    .bind(scoped.workspace_id().as_uuid())
    .bind(after)
    .bind(i64::from(limit))
    .fetch_all(scoped.conn())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, email, role_id, invited_by, expires_at, created_at)| InvitationRecord {
                id,
                email,
                role_id,
                invited_by,
                expires_at,
                created_at,
            },
        )
        .collect())
}

/// Revoke an invitation. `false` if it was not live in this workspace.
///
/// Revoked rather than deleted: `docs/25` wants "an invitation was withdrawn"
/// to survive in the trail, and the partial unique index already allows a
/// re-invite afterwards.
///
/// # Errors
///
/// Any database error.
pub async fn revoke(scoped: &mut Scoped<'_>, id: Uuid) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE invitation SET revoked_at = now()
          WHERE id = $1 AND workspace_id = $2
            AND accepted_at IS NULL AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(scoped.workspace_id().as_uuid())
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected == 1)
}

/// Whether a live invitation already exists for an address in this workspace.
///
/// Used to make re-inviting idempotent instead of a 409. See the API handler:
/// the alternative leaks whether an address was already invited, and `docs/40`
/// requires the invite response to be constant.
///
/// # Errors
///
/// Any database error.
pub async fn live_for_email(
    scoped: &mut Scoped<'_>,
    email: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM invitation
          WHERE workspace_id = $1 AND email = $2::citext
            AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at > now()",
    )
    .bind(scoped.workspace_id().as_uuid())
    .bind(email)
    .fetch_optional(scoped.conn())
    .await
}

/// Find the invitation behind a presented selector, **through the seam**.
///
/// Unscoped by necessity — see the module docs — and therefore through
/// migration 0022's fixed projection rather than a `SELECT` this crate wrote.
/// The function returns nothing for an accepted, revoked, or expired
/// invitation, so those three cases are indistinguishable here from an unknown
/// selector.
///
/// # Errors
///
/// Any database error.
pub async fn find_pending(
    conn: &mut sqlx::PgConnection,
    selector: &str,
) -> Result<Option<PendingInvitation>, sqlx::Error> {
    let row: Option<(Uuid, Uuid, String, Option<Uuid>)> =
        sqlx::query_as("SELECT id, workspace_id, email::text, role_id FROM lookup_invitation($1)")
            .bind(selector)
            .fetch_optional(conn)
            .await?;

    Ok(
        row.map(|(id, workspace_id, email, role_id)| PendingInvitation {
            id,
            workspace_id,
            email,
            role_id,
        }),
    )
}

/// The stored verifier hash for a selector, through its own door.
///
/// # Errors
///
/// Any database error.
pub async fn pending_verifier(
    conn: &mut sqlx::PgConnection,
    selector: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT lookup_invitation_verifier($1)")
        .bind(selector)
        .fetch_optional(conn)
        .await
        .map(Option::flatten)
}

/// Burn an invitation, returning whether **this** call was the one that burned
/// it.
///
/// The predicate lives in migration 0022's `consume_invitation`, not here, so
/// that it cannot be separated from the write by a caller who reads only this
/// signature. Two concurrent acceptances both find a live invitation and
/// exactly one gets `true`.
///
/// # Errors
///
/// Any database error.
pub async fn consume(conn: &mut sqlx::PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT consume_invitation($1)")
        .bind(id)
        .fetch_one(conn)
        .await
}

/// Find a user account by email, or `None`.
///
/// `user_account` carries no `workspace_id` — a person spans workspaces — so
/// this needs no scope and no seam. Tombstoned accounts return `None`: a
/// deactivated person accepting an invitation would silently reactivate
/// nothing, and the acceptance would create a membership pointing at an account
/// that cannot sign in.
///
/// # Errors
///
/// Any database error.
pub async fn user_by_email(
    conn: &mut sqlx::PgConnection,
    email: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM user_account WHERE email = $1::citext AND is_tombstone = false",
    )
    .bind(email)
    .fetch_optional(conn)
    .await
}

/// Create an account for an invitee who has none.
///
/// No credential row is written. `docs/40` §Local authentication makes a
/// password something a human chooses, and this path has no password to hash —
/// the invitee sets one through the reset flow, which is the same journey a
/// person who forgot theirs takes. Inventing a password here, or accepting one
/// from the accept request without proving control of the mailbox first, are
/// both worse than having none.
///
/// # Errors
///
/// Any database error, including a unique violation if the address was created
/// concurrently.
pub async fn insert_user(
    conn: &mut sqlx::PgConnection,
    email: &str,
    display_name: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO user_account (id, email, display_name) VALUES ($1, $2::citext, $3)")
        .bind(id)
        .bind(email)
        .bind(display_name)
        .execute(conn)
        .await?;
    Ok(id)
}

/// Grant the invited role at workspace scope.
///
/// Idempotent through the `UNIQUE` in migration 0003, so a retried acceptance
/// does not fail on the grant it already made.
///
/// `granted_by` is the person who *issued* the invitation, not the invitee.
/// The audit question is "who gave them this authority", and the answer is
/// never "they did" — an acceptance that recorded the invitee as the granter
/// would read, years later, as a self-grant.
///
/// # Errors
///
/// Any database error.
pub async fn assign_role(
    scoped: &mut Scoped<'_>,
    user_id: Uuid,
    role_id: Uuid,
    granted_by: Uuid,
) -> Result<(), sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    sqlx::query(
        "INSERT INTO role_assignment
             (id, workspace_id, principal_type, principal_id, role_id,
              scope_type, scope_id, granted_by)
         VALUES ($1,$2,'USER'::principal_type,$3,$4,'WORKSPACE'::scope_type,$2,$5)
         ON CONFLICT (workspace_id, principal_type, principal_id, role_id,
                      scope_type, scope_id) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(workspace)
    .bind(user_id)
    .bind(role_id)
    .bind(granted_by)
    .execute(scoped.conn())
    .await?;
    Ok(())
}

/// The permissions a role carries, for the grant ceiling.
///
/// `docs/04` control 1: "you cannot grant what you do not hold", checked
/// permission by permission. An invitation carrying a role is a **deferred
/// grant**, so it is subject to the same ceiling as an immediate one — without
/// this, inviting would be a way to hand out a role the inviter does not hold,
/// which is the escalation hole D-049 split `role.assign` from `role.manage`
/// to prevent.
///
/// # Errors
///
/// Any database error.
pub async fn role_permissions(
    scoped: &mut Scoped<'_>,
    role_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT rp.permission
           FROM role_permission rp
           JOIN role r ON r.id = rp.role_id
          WHERE rp.role_id = $1 AND r.workspace_id = $2",
    )
    .bind(role_id)
    .bind(scoped.workspace_id().as_uuid())
    .fetch_all(scoped.conn())
    .await
}

/// Who issued an invitation.
///
/// Read **scoped**, after the acceptance transaction has entered the workspace,
/// rather than through the seam. Adding `invited_by` to migration 0022's
/// projection would widen a deliberate hole in the ADR-020 backstop to carry a
/// value the acceptor never sees, and the value is reachable inside the tenant
/// where it belongs.
///
/// # Errors
///
/// Any database error.
pub async fn inviter_of(scoped: &mut Scoped<'_>, id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT invited_by FROM invitation WHERE id = $1 AND workspace_id = $2")
        .bind(id)
        .bind(scoped.workspace_id().as_uuid())
        .fetch_optional(scoped.conn())
        .await
        .map(Option::flatten)
}

/// Whether a role exists in this workspace.
///
/// Checked before an invitation stores it, so that a bad role id is a 422 at
/// invite time rather than a silent no-op at acceptance time — the invitee
/// would otherwise join with no role and nobody would know why.
///
/// # Errors
///
/// Any database error.
pub async fn role_exists(scoped: &mut Scoped<'_>, role_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM role WHERE id = $1 AND workspace_id = $2)")
        .bind(role_id)
        .bind(scoped.workspace_id().as_uuid())
        .fetch_one(scoped.conn())
        .await
}
