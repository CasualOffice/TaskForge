//! Worker-facing task fixtures; changes when projections and notifications change.

use uuid::Uuid;

/// Soft-delete a task, so a projection consumer can be tested against the
/// removal path as well as the write path.
///
/// # Errors
///
/// Any database error.
pub async fn soft_delete_task(pool: &sqlx::PgPool, task_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE task SET deleted_at = now() WHERE id = $1")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// A workspace with one project and one task in it, for the notification
/// fan-out tests (C-016).
///
/// Returned rather than assembled by the caller because the SQL has to live in
/// this crate (`docs/19`, enforced by `casual-task-lint` including in tests) and
/// the caller is `casual-task-worker`.
#[derive(Debug, Clone, Copy)]
pub struct TaskFixture {
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub task_id: Uuid,
    pub status_id: Uuid,
    pub reporter_id: Uuid,
}

/// Seed a workspace, a workflow, a project and one task.
///
/// `visibility` is a `visibility` enum value — `WORKSPACE`, `TEAM` or
/// `PRIVATE`. The private case is what the permission test needs.
///
/// # Errors
///
/// Any database error.
pub async fn seed_task(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    reporter_id: Uuid,
    visibility: &str,
    title: &str,
) -> Result<TaskFixture, sqlx::Error> {
    let workflow = Uuid::now_v7();
    let status = Uuid::now_v7();
    let project = Uuid::now_v7();
    let task = Uuid::now_v7();

    sqlx::query("INSERT INTO workflow (id, workspace_id, name) VALUES ($1,$2,'Default')")
        .bind(workflow)
        .bind(workspace_id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO workflow_status
             (id, workspace_id, workflow_id, name, state, position, is_initial)
         VALUES ($1,$2,$3,'Backlog','BACKLOG',1,true)",
    )
    .bind(status)
    .bind(workspace_id)
    .bind(workflow)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO project
             (id, workspace_id, key, name, visibility, workflow_id, created_by)
         VALUES ($1,$2,'WR','Work',$3::visibility,$4,$5)",
    )
    .bind(project)
    .bind(workspace_id)
    .bind(visibility)
    .bind(workflow)
    .bind(reporter_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO task
             (id, workspace_id, project_id, number, title, status_id, state,
              reporter_id, position, created_by)
         VALUES ($1,$2,$3,1,$4,$5,'BACKLOG',$6,'11111111',$6)",
    )
    .bind(task)
    .bind(workspace_id)
    .bind(project)
    .bind(title)
    .bind(status)
    .bind(reporter_id)
    .execute(pool)
    .await?;

    Ok(TaskFixture {
        workspace_id,
        project_id: project,
        task_id: task,
        status_id: status,
        reporter_id,
    })
}

/// Assign a user to a task, so `ASSIGNED` applies to them.
///
/// # Errors
///
/// Any database error.
pub async fn assign_task(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    task_id: Uuid,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO task_assignee (task_id, user_id, workspace_id) VALUES ($1,$2,$3)
         ON CONFLICT DO NOTHING",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Add a comment, optionally mentioning people. Returns the comment id.
///
/// # Errors
///
/// Any database error.
pub async fn seed_comment(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    task_id: Uuid,
    author_id: Uuid,
    mentions: &[Uuid],
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO comment (id, workspace_id, task_id, author_id, body, mentions)
         VALUES ($1,$2,$3,$4,'a comment',$5)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(task_id)
    .bind(author_id)
    .bind(mentions)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Add a project membership row, which confers visibility of a private project.
///
/// # Errors
///
/// Any database error.
pub async fn add_project_member(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO project_membership (project_id, user_id, workspace_id)
         VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(user_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Every notification a person has, as `(reason, event_type, aggregate_id)`,
/// newest first.
///
/// # Errors
///
/// Any database error.
pub async fn notifications_for(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<Vec<(String, String, Option<Uuid>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT reason, event_type, aggregate_id
           FROM notification
          WHERE user_id = $1
          ORDER BY created_at DESC, id DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Age every notification so the coalescing window no longer covers it.
///
/// Simulates the passage of time rather than spending it — the same argument as
/// `expire_all_claims`.
///
/// # Errors
///
/// Any database error.
pub async fn age_notifications(pool: &sqlx::PgPool, interval: &str) -> Result<u64, sqlx::Error> {
    Ok(
        sqlx::query("UPDATE notification SET created_at = created_at - $1::interval")
            .bind(interval)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

/// Put a task in a state for a window, without driving a workflow to get there.
///
/// A measure test is about what the *aggregate* does with an interval, not
/// about the transition path that produced one — and reaching `CANCELED`
/// through a real workflow is a different test. Here rather than in the API
/// suite because `docs/19` keeps every statement in this crate: SQL written in
/// a test is SQL that drifts from the statements the repository actually runs.
///
/// # Errors
///
/// Any database error.
pub async fn insert_state_interval(
    pool: &sqlx::PgPool,
    task_id: Uuid,
    workspace_id: Uuid,
    project_id: Uuid,
    state: &str,
    entered_days_ago: i32,
    exited_days_ago: Option<i32>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO task_state_interval
             (task_id, workspace_id, project_id, state, status_id, entered_at, exited_at)
         VALUES ($1,$2,$3,$4::task_state,$5,
                 now() - make_interval(days => $6),
                 CASE WHEN $7::int IS NULL THEN NULL
                      ELSE now() - make_interval(days => $7::int) END)",
    )
    .bind(task_id)
    .bind(workspace_id)
    .bind(project_id)
    .bind(state)
    .bind(Uuid::now_v7())
    .bind(entered_days_ago)
    .bind(exited_days_ago)
    .execute(pool)
    .await?;
    Ok(())
}

/// Move every interval a task has in one state into another.
///
/// The flip a measure test needs to show that a zero was the rule working
/// rather than the query finding nothing.
///
/// # Errors
///
/// Any database error.
pub async fn move_intervals(
    pool: &sqlx::PgPool,
    task_id: Uuid,
    from_state: &str,
    to_state: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE task_state_interval SET state = $3::task_state
          WHERE task_id = $1 AND state = $2::task_state",
    )
    .bind(task_id)
    .bind(from_state)
    .bind(to_state)
    .execute(pool)
    .await?;
    Ok(())
}
