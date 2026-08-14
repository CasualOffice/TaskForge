/// Execute a query the filter compiler produced.
///
/// # Errors
///
/// Any database error.
pub async fn list(
    scoped: &mut Scoped<'_>,
    compiled: &Compiled,
) -> Result<Vec<TaskRow>, sqlx::Error> {
    let mut query = sqlx::query(&compiled.sql);
    for param in &compiled.params {
        query = match param {
            Param::Workspace(w) => query.bind(w.as_uuid()),
            Param::Projects(ps) => query.bind(ps.iter().map(|p| p.as_uuid()).collect::<Vec<_>>()),
            Param::Text(t) => query.bind(t.clone()),
            Param::TextList(v) => query.bind(v.clone()),
        };
    }
    let rows = query.fetch_all(scoped.conn()).await?;
    rows.iter().map(row_of).collect()
}

/// The most subtasks one read returns.
///
/// `docs/21` bounds every input. A parent's children are a list a person
/// authored by hand, so this is generous rather than tight — but it is a bound,
/// and the rollup is computed by the database rather than from this page, so
/// `7/12 done` stays correct even for a parent past it.
pub const MAX_SUBTASKS: i64 = 200;

/// A parent's children, and the rollup `docs/03` says is displayed, never
/// enforced.
///
/// # This function cannot change anything
///
/// It reads. There is no sibling that completes a parent when its children are
/// done, and there is no argument here that would ask for one. `docs/03`:
/// "Parent status is **never** auto-derived from children ... implicit status
/// changes are the most confusing behaviour in every tracker that does it."
///
/// # Errors
///
/// Any database error.
pub async fn children(
    scoped: &mut Scoped<'_>,
    viewer: &crate::project::Viewer,
    parent_id: Uuid,
) -> Result<Vec<(TaskRow, String)>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    // Visibility is resolved the same way `read_visible` resolves it — through
    // the project — so a subtask in a project the viewer cannot see is absent
    // rather than redacted. `docs/04`: nothing may leak the existence of
    // invisible work, and a rollup that counted a row the list omits would do
    // exactly that.
    let sql = format!(
        "SELECT {COLUMNS}, p.key AS project_key
           FROM task t
           JOIN project p ON p.id = t.project_id
          WHERE t.parent_id = $5
            AND t.workspace_id = $1
            AND t.deleted_at IS NULL
            AND p.deleted_at IS NULL
            AND {visible}
          ORDER BY t.position, t.id
          LIMIT {MAX_SUBTASKS}",
        visible = crate::project::VISIBLE
    );
    let rows = sqlx::query(&sql)
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(parent_id)
        .fetch_all(scoped.conn())
        .await?;
    use sqlx::Row as _;
    rows.iter()
        .map(|row| Ok((row_of(row)?, row.try_get("project_key")?)))
        .collect()
}

/// `(done, total)` over a parent's visible children.
///
/// Computed in the database rather than from the page above, so a parent with
/// more children than [`MAX_SUBTASKS`] still reports the truth. `CANCELED` is
/// not counted as done, for the reason `milestone::progress` gives.
///
/// # Errors
///
/// Any database error.
pub async fn child_rollup(
    scoped: &mut Scoped<'_>,
    viewer: &crate::project::Viewer,
    parent_id: Uuid,
) -> Result<(i64, i64), sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let sql = format!(
        "SELECT COALESCE(count(*) FILTER (WHERE t.state = 'COMPLETED'), 0) AS done,
                COALESCE(count(*), 0) AS total
           FROM task t
           JOIN project p ON p.id = t.project_id
          WHERE t.parent_id = $5
            AND t.workspace_id = $1
            AND t.deleted_at IS NULL
            AND p.deleted_at IS NULL
            AND {visible}",
        visible = crate::project::VISIBLE
    );
    sqlx::query_as(&sql)
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(parent_id)
        .fetch_one(scoped.conn())
        .await
}

/// Remove a tag from a task. `false` if it did not carry it.
///
/// Scoped to the workspace in the `WHERE` clause rather than checked before it,
/// for the reason the inbox uses the same shape: a caller naming another
/// tenant's task affects zero rows and is told zero rows changed, which is the
/// same answer they get for an id that never existed.
///
/// # Errors
///
/// Any database error.
pub async fn remove_tag(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    tag_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "DELETE FROM task_tag
          WHERE task_id = $1 AND tag_id = $2 AND workspace_id = $3",
    )
    .bind(task_id)
    .bind(tag_id)
    .bind(scoped.workspace_id().as_uuid())
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}
