//! Workspaces, membership and teams (C-002, `docs/32`).
//!
//! # Two kinds of read, and the line between them
//!
//! Everything here except [`is_member`] and [`list_for_user`] takes a
//! [`Scoped`], so the tenant predicate is applied by the type system and the
//! row-level-security policy behind it (`docs/32` §Mechanism 1 and 2). None of
//! them takes a workspace id as an argument: it comes from the scope, so the
//! row written and the policy enforced cannot disagree.
//!
//! The two exceptions are the reads that **establish** the scope, and they go
//! through migration 0019's `SECURITY DEFINER` seam rather than around the
//! policy:
//!
//! - [`is_member`] — "may this actor enter this workspace?", the check that
//!   mints an `AuthContext`. It cannot be scoped, because a scope is what it
//!   produces.
//! - [`list_for_user`] — "which workspaces does this person belong to?", which
//!   is cross-tenant by construction. `docs/32` §The `user_account` exception
//!   is the same observation from the other side: a person legitimately spans
//!   workspaces, and the membership index is the person's own data.
//!
//! # Every mutation bumps `authz_epoch`
//!
//! `docs/04` §Caching: the epoch is "bumped by any grant, role, team
//! membership, or project membership change, **in the same transaction as the
//! change**". [`bump_authz_epoch`] is that write, and it takes a `Scoped` for
//! the same reason everything else here does. A membership change committed
//! without it leaves a cache entry that a later read can still hit.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// A workspace as stored.
#[derive(Debug, Clone)]
pub struct WorkspaceRecord {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    /// Optimistic concurrency (`docs/24`). Exposed as the `ETag`.
    pub version: i64,
    pub created_at: OffsetDateTime,
}

/// One row of `workspace_membership`, with the person it names.
#[derive(Debug, Clone)]
pub struct MemberRecord {
    pub user_id: Uuid,
    pub display_name: String,
    /// `NULL` once the account is anonymized (ADR-026).
    pub email: Option<String>,
    pub member_type: String,
    pub joined_at: OffsetDateTime,
}

/// A team as stored.
#[derive(Debug, Clone)]
pub struct TeamRecord {
    pub id: Uuid,
    pub name: String,
    pub version: i64,
    pub created_at: OffsetDateTime,
}

/// The member types `workspace_membership` accepts (migration 0002).
///
/// A closed set, here rather than at the API edge, so a value that would fail
/// the `CHECK` constraint is refused before it reaches a transaction that has
/// already written history.
pub const MEMBER_TYPES: &[&str] = &["MEMBER", "GUEST"];

/// Whether `user_id` may enter `workspace_id`.
///
/// Through the ADR-032 seam (migration 0019). Runs on a bare connection because
/// it is what *establishes* the scope — the request has no `WorkspaceScope`
/// until this returns true.
///
/// Deliberately a plain `bool` rather than a membership row: a caller holding
/// the row would be tempted to read authority out of `member_type`, and
/// migration 0003 is explicit that `role_assignment` is the only source of
/// authority in the system.
///
/// A soft-deleted workspace returns `false` — a workspace in its 30-day grace
/// window (`docs/32` §Deletion) is unreachable, not merely hidden.
///
/// # Errors
///
/// Any database error.
pub async fn is_member(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    workspace_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT is_workspace_member($1, $2)")
        .bind(user_id)
        .bind(workspace_id)
        .fetch_one(conn)
        .await
}

/// The workspaces `user_id` belongs to, one keyset page at a time.
///
/// Ordered by id, which is UUIDv7 and therefore creation order (`docs/26`), and
/// paged by `after` rather than by `OFFSET` — cursor pagination everywhere,
/// with no exception for a list that is short today.
///
/// # Errors
///
/// Any database error.
pub async fn list_for_user(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<WorkspaceRecord>, sqlx::Error> {
    let rows: Vec<(Uuid, String, String, i64, OffsetDateTime)> = sqlx::query_as(
        "SELECT w.id, w.name, w.slug, w.version, w.created_at
           FROM workspace w
          WHERE w.id IN (SELECT workspace_ids_for_user($1))
            AND ($2::uuid IS NULL OR w.id > $2::uuid)
          ORDER BY w.id
          LIMIT $3",
    )
    .bind(user_id)
    .bind(after)
    .bind(limit)
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(into_workspace).collect())
}

/// A workspace row that exists and has no owner yet (D-054).
///
/// The inner record is `pub(crate)`, so no crate outside this one can open it —
/// and the only thing in this crate that does is
/// [`crate::role::bootstrap`], which opens it by granting the creator
/// `workspace.owner`. A handler that creates a workspace and skips the grant
/// therefore has nothing to build a response from and **does not compile**.
///
/// That is the point. Before this type existed, `insert` handed back a finished
/// workspace and the grant was a step somebody had to remember; forgetting it
/// produced a workspace that its own creator could not write to, and nothing
/// anywhere reported it.
#[derive(Debug)]
#[must_use = "an unowned workspace has to be passed to role::bootstrap; \
              dropping it leaves the transaction holding a workspace nobody \
              can administer"]
pub struct Unowned(pub(crate) WorkspaceRecord);

impl Unowned {
    /// The workspace's id, for a caller that needs it before the grant exists.
    ///
    /// Deliberately the *only* accessor: an id is not enough to build a
    /// response with, so exposing it does not weaken the guarantee.
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.0.id
    }

    /// Open it. Crate-private, and called from exactly one place.
    pub(crate) fn into_record(self) -> WorkspaceRecord {
        self.0
    }
}

/// Create the workspace the scope names.
///
/// The id is taken from the scope, not from an argument: the caller has already
/// committed to which workspace this transaction is for, and a second id would
/// be a second chance to disagree with the row-level-security setting.
///
/// Returns an [`Unowned`], not a [`WorkspaceRecord`] — see that type.
///
/// # Errors
///
/// Any database error. A duplicate slug surfaces as a `23505` unique violation,
/// which the caller maps to the documented status.
pub async fn insert(
    scoped: &mut Scoped<'_>,
    name: &str,
    slug: &str,
) -> Result<Unowned, sqlx::Error> {
    let id = scoped.workspace_id().as_uuid();
    let row: (Uuid, String, String, i64, OffsetDateTime) = sqlx::query_as(
        "INSERT INTO workspace (id, name, slug)
         VALUES ($1, $2, $3)
         RETURNING id, name, slug, version, created_at",
    )
    .bind(id)
    .bind(name)
    .bind(slug)
    .fetch_one(scoped.conn())
    .await?;
    Ok(Unowned(into_workspace(row)))
}

/// The workspace the scope names, or `None` if it is absent or soft-deleted.
///
/// # Errors
///
/// Any database error.
pub async fn read(scoped: &mut Scoped<'_>) -> Result<Option<WorkspaceRecord>, sqlx::Error> {
    let id = scoped.workspace_id().as_uuid();
    let row: Option<(Uuid, String, String, i64, OffsetDateTime)> = sqlx::query_as(
        "SELECT id, name, slug, version, created_at
           FROM workspace
          WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(row.map(into_workspace))
}

/// Rename, conditional on `expected_version`.
///
/// `None` means the compare-and-set failed: either the row moved on (`409`) or
/// it is gone. The caller has already read it, so it can tell those apart —
/// and `docs/24` requires the losing writer to be told, never to be silently
/// applied on top.
///
/// # Errors
///
/// Any database error.
pub async fn rename(
    scoped: &mut Scoped<'_>,
    name: &str,
    expected_version: i64,
) -> Result<Option<WorkspaceRecord>, sqlx::Error> {
    let id = scoped.workspace_id().as_uuid();
    let row: Option<(Uuid, String, String, i64, OffsetDateTime)> = sqlx::query_as(
        "UPDATE workspace
            SET name = $2, version = version + 1
          WHERE id = $1 AND version = $3 AND deleted_at IS NULL
      RETURNING id, name, slug, version, created_at",
    )
    .bind(id)
    .bind(name)
    .bind(expected_version)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(row.map(into_workspace))
}

/// Take the workspace row's write lock for the duration of the transaction.
///
/// Membership removal is guarded by "a workspace never loses its last member",
/// and a guard implemented as count-then-delete is a race: two concurrent
/// removals each read two members and each delete one. Serialising membership
/// changes on the workspace row makes the count and the delete one atomic
/// decision, which is what `docs/04` control 4 means by "inside the
/// transaction".
///
/// # Errors
///
/// Any database error.
pub async fn lock(scoped: &mut Scoped<'_>) -> Result<(), sqlx::Error> {
    let id = scoped.workspace_id().as_uuid();
    sqlx::query("SELECT id FROM workspace WHERE id = $1 FOR UPDATE")
        .bind(id)
        .execute(scoped.conn())
        .await?;
    Ok(())
}

/// The members of the scoped workspace, one keyset page at a time.
///
/// # Errors
///
/// Any database error.
pub async fn list_members(
    scoped: &mut Scoped<'_>,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<MemberRecord>, sqlx::Error> {
    let rows: Vec<(Uuid, String, Option<String>, String, OffsetDateTime)> = sqlx::query_as(
        "SELECT m.user_id, u.display_name, u.email::text, m.member_type, m.joined_at
           FROM workspace_membership m
           JOIN user_account u ON u.id = m.user_id
          WHERE m.workspace_id = $1
            AND ($2::uuid IS NULL OR m.user_id > $2::uuid)
          ORDER BY m.user_id
          LIMIT $3",
    )
    .bind(scoped.workspace_id().as_uuid())
    .bind(after)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(user_id, display_name, email, member_type, joined_at)| MemberRecord {
                user_id,
                display_name,
                email,
                member_type,
                joined_at,
            },
        )
        .collect())
}

/// Whether `user_id` is a member of the **scoped** workspace.
///
/// The scoped counterpart of [`is_member`], and not a duplicate of it: this one
/// reads through the policy rather than through the seam, so it answers only
/// about the workspace the request is already inside. It is what keeps a team
/// membership from naming a stranger — `team_membership` carries no
/// `workspace_id` and therefore no policy of its own (migration 0010), so this
/// check is the tenant boundary for that table.
///
/// # Errors
///
/// Any database error.
pub async fn is_member_scoped(scoped: &mut Scoped<'_>, user_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM workspace_membership
             WHERE workspace_id = $1 AND user_id = $2)",
    )
    .bind(scoped.workspace_id().as_uuid())
    .bind(user_id)
    .fetch_one(scoped.conn())
    .await
}

/// How many members the scoped workspace has.
///
/// # Errors
///
/// Any database error.
pub async fn member_count(scoped: &mut Scoped<'_>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM workspace_membership WHERE workspace_id = $1")
        .bind(scoped.workspace_id().as_uuid())
        .fetch_one(scoped.conn())
        .await
}

/// Add a member. `false` if they were already one — adding twice is not an
/// error, and reporting one would make the retry a client is entitled to make
/// look like a failure.
///
/// # Errors
///
/// Any database error. An unknown `user_id` surfaces as a `23503` foreign-key
/// violation.
pub async fn insert_member(
    scoped: &mut Scoped<'_>,
    user_id: Uuid,
    member_type: &str,
) -> Result<bool, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let affected = sqlx::query(
        "INSERT INTO workspace_membership (workspace_id, user_id, member_type)
         VALUES ($1, $2, $3)
         ON CONFLICT (workspace_id, user_id) DO NOTHING",
    )
    .bind(workspace)
    .bind(user_id)
    .bind(member_type)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Remove a member. `false` if they were not one.
///
/// # Errors
///
/// Any database error.
pub async fn delete_member(scoped: &mut Scoped<'_>, user_id: Uuid) -> Result<bool, sqlx::Error> {
    let affected =
        sqlx::query("DELETE FROM workspace_membership WHERE workspace_id = $1 AND user_id = $2")
            .bind(scoped.workspace_id().as_uuid())
            .bind(user_id)
            .execute(scoped.conn())
            .await?
            .rows_affected();
    Ok(affected > 0)
}

/// The workspace's current `authz_epoch` (`docs/04` §Caching, ADR-012).
///
/// One indexed read by primary key. It exists so a long-lived reader — an open
/// SSE stream — can ask "has any grant, role, team or project membership
/// changed since I was authorized?" without re-resolving the whole authority.
/// `docs/04` is explicit that the epoch answers exactly that: it is "bumped by
/// any grant, role, team membership, or project membership change, in the same
/// transaction as the change".
///
/// # Errors
///
/// Any database error.
pub async fn authz_epoch(scoped: &mut Scoped<'_>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT authz_epoch FROM workspace WHERE id = $1")
        .bind(scoped.workspace_id().as_uuid())
        .fetch_one(scoped.conn())
        .await
}

/// Bump the workspace's `authz_epoch` (`docs/04` §Caching, ADR-012).
///
/// Returns the new value so a caller can put it in the audit record: "the
/// permission cache was invalidated at epoch N" is the fact that explains why a
/// revoked user stopped seeing data when they did.
///
/// # Errors
///
/// Any database error.
pub async fn bump_authz_epoch(scoped: &mut Scoped<'_>) -> Result<i64, sqlx::Error> {
    let id = scoped.workspace_id().as_uuid();
    sqlx::query_scalar(
        "UPDATE workspace SET authz_epoch = authz_epoch + 1
          WHERE id = $1
      RETURNING authz_epoch",
    )
    .bind(id)
    .fetch_one(scoped.conn())
    .await
}

/// Create a team in the scoped workspace.
///
/// # Errors
///
/// Any database error. A duplicate name surfaces as a `23505` unique violation.
pub async fn insert_team(scoped: &mut Scoped<'_>, name: &str) -> Result<TeamRecord, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let row: (Uuid, String, i64, OffsetDateTime) = sqlx::query_as(
        "INSERT INTO team (id, workspace_id, name)
         VALUES ($1, $2, $3)
         RETURNING id, name, version, created_at",
    )
    .bind(Uuid::now_v7())
    .bind(workspace)
    .bind(name)
    .fetch_one(scoped.conn())
    .await?;
    Ok(into_team(row))
}

/// The teams of the scoped workspace, one keyset page at a time.
///
/// Ordered by name rather than by id: `UNIQUE (workspace_id, name)` is the
/// index that serves it, so the keyset walk is index-ordered rather than a sort
/// over the tenant's teams.
///
/// # Errors
///
/// Any database error.
pub async fn list_teams(
    scoped: &mut Scoped<'_>,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<TeamRecord>, sqlx::Error> {
    let rows: Vec<(Uuid, String, i64, OffsetDateTime)> = sqlx::query_as(
        "SELECT id, name, version, created_at
           FROM team
          WHERE workspace_id = $1
            AND deleted_at IS NULL
            AND ($2::text IS NULL OR name > $2::text)
          ORDER BY name
          LIMIT $3",
    )
    .bind(scoped.workspace_id().as_uuid())
    .bind(after)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;
    Ok(rows.into_iter().map(into_team).collect())
}

/// One team of the scoped workspace, or `None`.
///
/// `None` for a team in another tenant as well as for one that does not exist,
/// and the caller turns both into the same `404` (`docs/04`).
///
/// **The `workspace_id` predicate is written out, not left to the policy.**
/// `docs/32` requires two independent mechanisms that must *both* fail before
/// data crosses a boundary, and RLS is the second one. Relying on it alone also
/// silently disables the check under any connection that bypasses RLS — which
/// includes every test harness that connects as the database owner, so the hole
/// would be invisible exactly where it would be caught.
///
/// # Errors
///
/// Any database error.
pub async fn find_team(
    scoped: &mut Scoped<'_>,
    team_id: Uuid,
) -> Result<Option<TeamRecord>, sqlx::Error> {
    let row: Option<(Uuid, String, i64, OffsetDateTime)> = sqlx::query_as(
        "SELECT id, name, version, created_at
           FROM team
          WHERE workspace_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(scoped.workspace_id().as_uuid())
    .bind(team_id)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(row.map(into_team))
}

/// Add someone to a team. `false` if they were already in it, or if the team is
/// not in the scoped workspace.
///
/// # The tenant predicate is inside the statement, not beside it
///
/// `team_membership` carries no `workspace_id` and therefore has no policy of
/// its own (migration 0010), so there is no backstop here — the row this writes
/// is reachable from `team`, and `team` is where the tenant lives. Rather than
/// trusting the caller to have called [`find_team`] first, the insert selects
/// through `team` with the scope's workspace, so a team id from another tenant
/// inserts zero rows however it was obtained.
///
/// The caller must still have checked that the user is a member of the
/// workspace ([`is_member_scoped`]); that is a domain rule with its own error,
/// not a tenancy question.
///
/// # Errors
///
/// Any database error.
pub async fn insert_team_member(
    scoped: &mut Scoped<'_>,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "INSERT INTO team_membership (team_id, user_id)
         SELECT t.id, $3
           FROM team t
          WHERE t.id = $1 AND t.workspace_id = $2 AND t.deleted_at IS NULL
         ON CONFLICT (team_id, user_id) DO NOTHING",
    )
    .bind(team_id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(user_id)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// One person in a team.
///
/// Deliberately **not** [`MemberRecord`]: that carries `joined_at`, and
/// `team_membership` is `(team_id, user_id)` and nothing else (migration 0002).
/// Reusing the workspace shape would have meant filling that field with the
/// date they joined the *workspace* — a plausible-looking date that answers a
/// different question, which is worse than not answering.
#[derive(Debug, Clone)]
pub struct TeamMemberRecord {
    pub user_id: Uuid,
    pub display_name: String,
    pub email: Option<String>,
    /// Their standing in the **workspace**, so a team list can mark a guest.
    pub member_type: String,
}

/// Who is in a team, one page at a time.
///
/// # Why it joins `workspace_membership` and not just `team_membership`
///
/// `team_membership` carries no `workspace_id` and therefore no policy of its
/// own (migration 0010) — the same reason [`is_member_scoped`] exists on the
/// write path. Reading it directly would return whatever ids that table holds;
/// joining membership means a row naming someone outside this workspace is
/// invisible rather than rendered, so a tenancy failure elsewhere cannot become
/// a directory leak here. The `team` join carries the same constraint for the
/// team itself.
///
/// The keyset is `user_id`, matching [`list_members`], so the two member lists
/// paginate the same way.
///
/// # Errors
///
/// Any database error.
pub async fn list_team_members(
    scoped: &mut Scoped<'_>,
    team_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<TeamMemberRecord>, sqlx::Error> {
    let rows: Vec<(Uuid, String, Option<String>, String)> = sqlx::query_as(
        "SELECT tm.user_id, u.display_name, u.email::text, m.member_type
           FROM team_membership tm
           JOIN team t ON t.id = tm.team_id
           JOIN workspace_membership m
             ON m.user_id = tm.user_id AND m.workspace_id = t.workspace_id
           JOIN user_account u ON u.id = tm.user_id
          WHERE tm.team_id = $1
            AND t.workspace_id = $2
            AND t.deleted_at IS NULL
            AND ($3::uuid IS NULL OR tm.user_id > $3::uuid)
          ORDER BY tm.user_id
          LIMIT $4",
    )
    .bind(team_id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(after)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(user_id, display_name, email, member_type)| TeamMemberRecord {
                user_id,
                display_name,
                email,
                member_type,
            },
        )
        .collect())
}

/// Remove someone from a team. `false` if they were not in it.
///
/// # Errors
///
/// Any database error.
pub async fn delete_team_member(
    scoped: &mut Scoped<'_>,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "DELETE FROM team_membership tm
              USING team t
              WHERE tm.team_id = t.id
                AND t.id = $1
                AND t.workspace_id = $2
                AND tm.user_id = $3",
    )
    .bind(team_id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(user_id)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

fn into_workspace(
    (id, name, slug, version, created_at): (Uuid, String, String, i64, OffsetDateTime),
) -> WorkspaceRecord {
    WorkspaceRecord {
        id,
        name,
        slug,
        version,
        created_at,
    }
}

fn into_team((id, name, version, created_at): (Uuid, String, i64, OffsetDateTime)) -> TeamRecord {
    TeamRecord {
        id,
        name,
        version,
        created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_member_types_are_the_ones_the_check_constraint_allows() {
        // Migration 0002: CHECK (member_type IN ('MEMBER','GUEST')). A value
        // accepted here and refused there would abort a transaction that has
        // already written its audit row.
        let migration = include_str!("../../../migrations/0002_tenancy_and_identity.sql");
        for member_type in MEMBER_TYPES {
            assert!(
                migration.contains(&format!("'{member_type}'")),
                "{member_type} is offered by the API and refused by the schema"
            );
        }
        assert_eq!(MEMBER_TYPES.len(), 2);
    }
}
