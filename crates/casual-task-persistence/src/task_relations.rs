/// The unresolved blockers of a task that the actor may see.
///
/// Invisible blockers still block but are not named; completed and canceled
/// blockers do not.
///
/// # Errors
///
/// Any database error.
pub async fn unresolved_blockers(
    scoped: &mut Scoped<'_>,
    viewer: &crate::project::Viewer,
    task_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let sql = format!(
        "SELECT b.id
           FROM task_dependency d
           JOIN task b    ON b.id = d.from_task_id
           JOIN project p ON p.id = b.project_id
          WHERE d.to_task_id = $5
            AND d.workspace_id = $1
            AND d.kind = 'BLOCKS'
            AND b.deleted_at IS NULL
            AND p.deleted_at IS NULL
            AND b.state NOT IN ('COMPLETED','CANCELED')
            AND {visible}
          ORDER BY b.id",
        visible = crate::project::VISIBLE
    );
    sqlx::query_scalar(&sql)
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(task_id)
        .fetch_all(scoped.conn())
        .await
}

/// Whether `user` is in this workspace **and** can see `project`.
///
/// This is the check behind `TF-TSK-0005` ("assignee is not a member of the
/// project"), and it is deliberately the *visibility* rule from `docs/04`
/// §Visibility vs permission rather than a `project_membership` lookup. On a
/// `WORKSPACE`-visible project — the default — there are usually no membership
/// rows at all, so requiring one would make assignment impossible in the common
/// case while claiming to enforce a rule nobody could satisfy.
///
/// What it does enforce is the invariant that actually matters: **work is never
/// assigned to someone who cannot see it**. A stranger, a member of another
/// tenant, or a colleague who cannot open the project are all refused, and for
/// the same reason.
///
/// The clauses mirror `crate::project::VISIBLE` with `user` in the viewer's
/// place. They are written out rather than reusing that constant because this
/// query resolves the user's teams and project grants inline — the caller has a
/// `Viewer` for the *actor*, and the question here is about somebody else.
///
/// # Errors
///
/// Any database error.
pub async fn may_be_assigned(
    scoped: &mut Scoped<'_>,
    user: Uuid,
    project_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1
              FROM project p
             WHERE p.id = $1
               AND p.workspace_id = $2
               AND p.deleted_at IS NULL
               AND EXISTS (SELECT 1 FROM workspace_membership wm
                            WHERE wm.workspace_id = $2 AND wm.user_id = $3)
               AND (   p.visibility = 'WORKSPACE'
                    OR (p.visibility = 'TEAM'
                        AND EXISTS (SELECT 1 FROM project_team pt
                                     JOIN team_membership tm ON tm.team_id = pt.team_id
                                    WHERE pt.project_id = p.id AND tm.user_id = $3))
                    OR EXISTS (SELECT 1 FROM project_membership pm
                                WHERE pm.project_id = p.id AND pm.user_id = $3)
                    OR EXISTS (SELECT 1 FROM role_assignment ra
                                WHERE ra.workspace_id = $2
                                  AND ra.scope_type = 'PROJECT'
                                  AND ra.scope_id = p.id
                                  AND ra.principal_type = 'USER'
                                  AND ra.principal_id = $3)))",
    )
    .bind(project_id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(user)
    .fetch_one(scoped.conn())
    .await
}

/// The users assigned to a task, oldest assignment first.
///
/// # Errors
///
/// Any database error.
/// Who is on each of these tasks — one query for a page, not one per row.
///
/// The same shape `activity::actor_names` uses, and for the same reason: a list
/// that resolved this per row would make a page of fifty tasks fifty requests,
/// which is the difference between a list that shows who is on the work and a
/// list that cannot afford to.
///
/// # Errors
///
/// Any database error.
pub async fn assignees_for(
    scoped: &mut Scoped<'_>,
    task_ids: &[Uuid],
) -> Result<Vec<(Uuid, Uuid)>, sqlx::Error> {
    if task_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT task_id, user_id FROM task_assignee WHERE task_id = ANY($1) ORDER BY task_id",
    )
    .bind(task_ids)
    .fetch_all(scoped.conn())
    .await?;
    Ok(rows)
}

pub async fn assignees(scoped: &mut Scoped<'_>, task_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT user_id FROM task_assignee
          WHERE task_id = $1 AND workspace_id = $2
          ORDER BY assigned_at, user_id",
    )
    .bind(task_id)
    .bind(scoped.workspace_id().as_uuid())
    .fetch_all(scoped.conn())
    .await
}

/// Assign a user. `false` if they were already assigned.
///
/// The `workspace_id` is taken from the scope rather than from a parameter:
/// `task_assignee` carries the column but no foreign key to `workspace`, so the
/// scope is the only thing that keeps the denormalized value honest.
///
/// # Errors
///
/// Any database error.
pub async fn add_assignee(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "INSERT INTO task_assignee (task_id, user_id, workspace_id)
         SELECT t.id, $2, t.workspace_id
           FROM task t
          WHERE t.id = $1 AND t.workspace_id = $3 AND t.deleted_at IS NULL
         ON CONFLICT (task_id, user_id) DO NOTHING",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(scoped.workspace_id().as_uuid())
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Unassign a user. `false` if they were not assigned.
///
/// # Errors
///
/// Any database error.
pub async fn remove_assignee(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "DELETE FROM task_assignee
          WHERE task_id = $1 AND user_id = $2 AND workspace_id = $3",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(scoped.workspace_id().as_uuid())
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// A tag's name, when it exists and is usable on tasks in `project`.
///
/// `tag.project_id` of `NULL` is a workspace-scoped tag (migration 0005), usable
/// anywhere in the workspace; a project-scoped one is usable only on tasks in
/// that project. Returning the name rather than a bare boolean is what lets the
/// activity record hold a display value instead of an id (`docs/25`).
///
/// # Errors
///
/// Any database error.
pub async fn usable_tag(
    scoped: &mut Scoped<'_>,
    tag_id: Uuid,
    project_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT name::text FROM tag
          WHERE id = $1
            AND workspace_id = $2
            AND (project_id IS NULL OR project_id = $3)",
    )
    .bind(tag_id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(project_id)
    .fetch_optional(scoped.conn())
    .await
}

/// Tag a task. `false` if it already carried the tag.
///
/// # Errors
///
/// Any database error.
pub async fn add_tag(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    tag_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "INSERT INTO task_tag (task_id, tag_id, workspace_id)
         SELECT t.id, $2, t.workspace_id
           FROM task t
          WHERE t.id = $1 AND t.workspace_id = $3 AND t.deleted_at IS NULL
         ON CONFLICT (task_id, tag_id) DO NOTHING",
    )
    .bind(task_id)
    .bind(tag_id)
    .bind(scoped.workspace_id().as_uuid())
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Write the comment a transition carried (`docs/23` §What commits).
///
/// In the transition's own transaction, so a move that was explained and a move
/// that was not are never confused: either both rows commit or neither does.
///
/// # Errors
///
/// Any database error.
pub async fn insert_comment(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    author: Uuid,
    body: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO comment (id, workspace_id, task_id, author_id, body)
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(task_id)
    .bind(author)
    .bind(body)
    .execute(scoped.conn())
    .await?;
    Ok(id)
}
