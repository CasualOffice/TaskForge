//! Task, search, and attachment fixtures; changes when work-item storage changes.

use uuid::Uuid;

/// A task's status and state, read straight from the row.
///
/// Both together, because `docs/23`'s derived-state invariant is a claim about
/// the pair.
///
/// # Errors
///
/// Any database error.
pub async fn task_status_and_state(
    pool: &sqlx::PgPool,
    task_id: Uuid,
) -> Result<(Uuid, String), sqlx::Error> {
    sqlx::query_as("SELECT status_id, state::text FROM task WHERE id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
}

/// Whether a task is soft-deleted.
///
/// # Errors
///
/// Any database error.
pub async fn task_is_deleted(pool: &sqlx::PgPool, task_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM task WHERE id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
}

/// A task's assignees.
///
/// # Errors
///
/// Any database error.
pub async fn task_assignees(pool: &sqlx::PgPool, task_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT user_id FROM task_assignee WHERE task_id = $1 ORDER BY assigned_at")
        .bind(task_id)
        .fetch_all(pool)
        .await
}

/// How many comments a task carries.
///
/// # Errors
///
/// Any database error.
pub async fn comment_count(pool: &sqlx::PgPool, task_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM comment WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
}

/// Create a tag. `project_id` of `None` is a workspace-scoped tag.
///
/// # Errors
///
/// Any database error.
pub async fn insert_tag(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    project_id: Option<Uuid>,
    name: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tag (id, workspace_id, project_id, name) VALUES ($1,$2,$3,$4::citext)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(project_id)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Record that `blocker` blocks `blocked` (`docs/23` step 7).
///
/// # Errors
///
/// Any database error.
pub async fn add_blocker(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    blocker: Uuid,
    blocked: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO task_dependency (from_task_id, to_task_id, workspace_id, kind)
         VALUES ($1,$2,$3,'BLOCKS')",
    )
    .bind(blocker)
    .bind(blocked)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// A workflow, a project and one task, as the smallest thing a projection test
/// can index.
///
/// Written out rather than driven through the API because the worker crate has
/// no HTTP: `task.status_id` and `project.workflow_id` are both `NOT NULL`, so
/// "one task" is unavoidably four rows.
///
/// # Errors
///
/// Any database error.
pub async fn insert_task_fixture(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    title: &str,
) -> Result<Uuid, sqlx::Error> {
    let workflow = Uuid::now_v7();
    let status = Uuid::now_v7();
    let project = Uuid::now_v7();
    let task = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO workflow (id, workspace_id, name, is_default) VALUES ($1,$2,'D',true)",
    )
    .bind(workflow)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO workflow_status
             (id, workflow_id, workspace_id, name, state, position, is_initial)
         VALUES ($1,$2,$3,'Backlog','BACKLOG'::task_state,1,true)",
    )
    .bind(status)
    .bind(workflow)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO project
             (id, workspace_id, key, name, workflow_id, created_by, visibility)
         VALUES ($1,$2,'WR','Work',$3,$4,'WORKSPACE'::visibility)",
    )
    .bind(project)
    .bind(workspace_id)
    .bind(workflow)
    .bind(user_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO task
             (id, workspace_id, project_id, number, title, status_id, state,
              reporter_id, position, created_by)
         VALUES ($1,$2,$3,1,$4,$5,'BACKLOG'::task_state,$6,'a0',$6)",
    )
    .bind(task)
    .bind(workspace_id)
    .bind(project)
    .bind(title)
    .bind(status)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(task)
}

/// Insert `count` task rows into an existing project's initial status.
///
/// This is intentionally a test-only bulk fixture: acceptance tests that need
/// a full page should measure the read path, not spend the API rate-limit
/// budget constructing their corpus.
///
/// # Errors
///
/// Any database error, including a project without an initial status.
pub async fn insert_task_page(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    project_id: Uuid,
    user_id: Uuid,
    count: u32,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let status: Uuid = sqlx::query_scalar(
        "SELECT ws.id
           FROM workflow_status ws
           JOIN project p ON p.workflow_id = ws.workflow_id
          WHERE p.id = $1 AND p.workspace_id = $2 AND ws.is_initial",
    )
    .bind(project_id)
    .bind(workspace_id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO task
             (id, workspace_id, project_id, number, title, status_id, state,
              reporter_id, position, created_by)
         SELECT gen_random_uuid(), $1, $2, n, 'task ' || n::text, $3,
                'BACKLOG'::task_state, $4, 'a' || lpad(n::text, 8, '0'), $4
           FROM generate_series(1, $5::bigint) AS n",
    )
    .bind(workspace_id)
    .bind(project_id)
    .bind(status)
    .bind(user_id)
    .bind(i64::from(count))
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// Rebuild one task's search document, as the projection consumer would.
///
/// The consumer itself lives in `casual-task-worker` and is exercised by its
/// own test. This is for the API tests, whose subject is the *query* path: they
/// need a populated `task_search` and should not have to run a dispatch loop to
/// get one.
///
/// # Errors
///
/// Any database error.
pub async fn index_task(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    task_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let scope = casual_task_model::WorkspaceScope::for_job(
        casual_task_model::WorkspaceId::from_uuid(workspace_id),
    );
    let mut tx = pool.begin().await?;
    let mut scoped = crate::Scoped::apply(&mut tx, &scope).await?;
    let indexed = crate::search::refresh(&mut scoped, task_id).await?;
    tx.commit().await?;
    Ok(indexed)
}

/// How many rows the search projection holds for a workspace.
///
/// # Errors
///
/// Any database error.
pub async fn indexed_count(pool: &sqlx::PgPool, workspace_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM task_search WHERE workspace_id = $1")
        .bind(workspace_id)
        .fetch_one(pool)
        .await
}

/// An uploaded-but-unscanned attachment, as `commit` leaves one.
///
/// Here rather than in the worker's test because `docs/19`'s boundary
/// invariant puts **all** SQL in this crate, and the architecture lint enforces
/// it — a fixture is not an exception, and CI caught exactly that.
///
/// # Errors
///
/// Any database error.
pub async fn insert_pending_attachment(
    pool: &sqlx::PgPool,
    workspace: Uuid,
    task: Uuid,
    uploader: Uuid,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO attachment
             (id, workspace_id, task_id, object_key, filename, content_type,
              byte_size, checksum, scan_status, uploaded_by, committed_at)
         VALUES ($1, $2, $3, $4, 'notes.txt', 'text/plain', 4, 'abc', 'PENDING', $5, NULL)",
    )
    .bind(id)
    .bind(workspace)
    .bind(task)
    .bind(format!("{workspace}/{task}/{id}"))
    .bind(uploader)
    .execute(pool)
    .await?;
    Ok(id)
}

/// An attachment's scan verdict, and whether it has been committed.
///
/// The pair is the assertion the scan pipeline turns on: `committed_at` is set
/// by `PENDING → CLEAN` alone and every read requires it, so a status without
/// it is a file nobody can see.
///
/// # Errors
///
/// Any database error.
pub async fn attachment_scan_state(
    pool: &sqlx::PgPool,
    id: Uuid,
) -> Result<(String, bool), sqlx::Error> {
    let row: (String, Option<time::OffsetDateTime>) =
        sqlx::query_as("SELECT scan_status, committed_at FROM attachment WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?;
    Ok((row.0, row.1.is_some()))
}

/// Whether an attachment row exists at all, committed or not.
///
/// The invisibility gate needs to assert that the row IS there and that no read
/// path returns it — an assertion that only checked the API would pass if the
/// pre-sign had silently written nothing.
///
/// # Errors
///
/// Any database error.
pub async fn attachment_exists(pool: &sqlx::PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM attachment WHERE id = $1)")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// An attachment's object key, to assert it is built from ids alone.
///
/// # Errors
///
/// Any database error.
pub async fn attachment_object_key(pool: &sqlx::PgPool, id: Uuid) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT object_key FROM attachment WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Apply a scan verdict, as the scan worker will.
///
/// Goes through [`crate::attachment::mark_scanned`] rather than writing the
/// columns directly, so a test cannot commit a row by a route the product does
/// not have — which is the whole point of that function being the only writer
/// of `committed_at`.
///
/// # Errors
///
/// Any database error.
pub async fn set_scan_verdict(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    id: Uuid,
    verdict: &str,
) -> Result<bool, sqlx::Error> {
    let scope = casual_task_model::WorkspaceScope::for_job(
        casual_task_model::WorkspaceId::from_uuid(workspace_id),
    );
    let mut tx = pool.begin().await?;
    let mut scoped = crate::Scoped::apply(&mut tx, &scope).await?;
    let applied = crate::attachment::mark_scanned(&mut scoped, id, verdict, None).await?;
    tx.commit().await?;
    Ok(applied)
}
