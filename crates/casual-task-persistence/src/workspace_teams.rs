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

/// The teams the actor is in, by name.
///
/// # Why this is not `list_teams` with a filter
///
/// "Which teams exist here" and "which am I in" are asked by different screens
/// for different reasons: the first is an administrator picking one, the second
/// is a person finding their own work. A caller who is in three of a hundred
/// teams should not page through a hundred to find them, and a sidebar built on
/// the unfiltered list would grow with the workspace rather than with the
/// person.
///
/// Not paged, deliberately. A person is in a handful of teams — if that ever
/// stops being true the sidebar has a worse problem than pagination.
///
/// Joins `workspace_membership` for the same reason [`list_team_members`] does:
/// `team_membership` carries no `workspace_id` and therefore no policy of its
/// own, so a row naming a team in another tenant must be invisible rather than
/// rendered.
///
/// # Errors
///
/// Any database error.
pub async fn list_my_teams(
    scoped: &mut Scoped<'_>,
    user_id: Uuid,
) -> Result<Vec<TeamRecord>, sqlx::Error> {
    let rows: Vec<(Uuid, String, i64, OffsetDateTime)> = sqlx::query_as(
        "SELECT t.id, t.name, t.version, t.created_at
           FROM team_membership tm
           JOIN team t ON t.id = tm.team_id
           JOIN workspace_membership m
             ON m.user_id = tm.user_id AND m.workspace_id = t.workspace_id
          WHERE tm.user_id = $1
            AND t.workspace_id = $2
            AND t.deleted_at IS NULL
          ORDER BY t.name",
    )
    .bind(user_id)
    .bind(scoped.workspace_id().as_uuid())
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
