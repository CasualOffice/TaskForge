/// Log a promotion for a move another module already applied.
///
/// The column update belongs to `crate::environment::set_on_task`; this records
/// the matching history inside its transaction.
///
/// # Errors
///
/// Any database error.
pub async fn record_promotion(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    environment_id: Uuid,
    promoted_by: Uuid,
) -> Result<PromotionRow, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let row: PromotionTuple = sqlx::query_as(
        "INSERT INTO task_environment_promotion
             (id, workspace_id, task_id, environment_id, promoted_by)
         VALUES ($1,$2,$3,$4,$5)
         RETURNING id, environment_id, release_id, promoted_by, promoted_at",
    )
    .bind(Uuid::now_v7())
    .bind(workspace)
    .bind(task_id)
    .bind(environment_id)
    .bind(promoted_by)
    .fetch_one(scoped.conn())
    .await?;
    Ok(PromotionRow {
        id: row.0,
        environment_id: row.1,
        release_id: row.2,
        promoted_by: row.3,
        promoted_at: row.4,
    })
}

/// Record a verdict against the environment the task was tested on.
///
/// The verdict is a *fact*, not a transition. What happens next — back to the
/// developer on a fail, forward on a pass — is a workflow move the caller makes
/// afterwards, and keeping them separate is what lets "failed twice on qa"
/// survive however many times the status has since changed.
///
/// # Errors
///
/// [`CustodyError::EnvironmentNotOnProject`] or any database error.
pub async fn verify(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    environment_id: Uuid,
    verdict: &str,
    verified_by: Uuid,
    note: Option<&str>,
) -> Result<VerificationRow, CustodyError> {
    let workspace = scoped.workspace_id().as_uuid();

    // The environment must belong to the task's project. A verdict recorded
    // against another project's "staging" is a result nobody can reproduce.
    let matches: Option<i32> = sqlx::query_scalar(
        "SELECT 1
           FROM task t
           JOIN project_environment e ON e.project_id = t.project_id
          WHERE t.id = $1 AND e.id = $2 AND t.deleted_at IS NULL",
    )
    .bind(task_id)
    .bind(environment_id)
    .fetch_optional(scoped.conn())
    .await?;
    if matches.is_none() {
        return Err(CustodyError::EnvironmentNotOnProject);
    }

    let row: VerificationTuple = sqlx::query_as(
        "INSERT INTO task_verification
             (id, workspace_id, task_id, environment_id, verdict, verified_by, note)
         VALUES ($1,$2,$3,$4,$5::verification_verdict,$6,$7)
         RETURNING id, environment_id, verdict::text, verified_by, verified_at, note",
    )
    .bind(Uuid::now_v7())
    .bind(workspace)
    .bind(task_id)
    .bind(environment_id)
    .bind(verdict)
    .bind(verified_by)
    .bind(note)
    .fetch_one(scoped.conn())
    .await?;

    Ok(VerificationRow {
        id: row.0,
        environment_id: row.1,
        verdict: row.2,
        verified_by: row.3,
        verified_at: row.4,
        note: row.5,
    })
}

/// Everything the item surface's custody panel needs, in one round trip.
///
/// Three lists rather than three endpoints: they are one panel, they are always
/// read together, and a surface that issued three requests would render in three
/// stages. Each is bounded — a task with a thousand promotions is a runaway
/// pipeline, not a page anyone reads.
///
/// # Errors
///
/// Any database error.
pub async fn history(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    limit: i64,
) -> Result<(Vec<TransferRow>, Vec<PromotionRow>, Vec<VerificationRow>), sqlx::Error> {
    let transfers: Vec<TransferTuple> = sqlx::query_as(
        "SELECT id, from_team_id, to_team_id, moved_by, moved_at, note
               FROM task_team_transfer
              WHERE task_id = $1
              ORDER BY moved_at DESC
              LIMIT $2",
    )
    .bind(task_id)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;

    let promotions: Vec<PromotionTuple> = sqlx::query_as(
        "SELECT id, environment_id, release_id, promoted_by, promoted_at
           FROM task_environment_promotion
          WHERE task_id = $1
          ORDER BY promoted_at DESC
          LIMIT $2",
    )
    .bind(task_id)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;

    let verifications: Vec<VerificationTuple> = sqlx::query_as(
        "SELECT id, environment_id, verdict::text, verified_by, verified_at, note
               FROM task_verification
              WHERE task_id = $1
              ORDER BY verified_at DESC
              LIMIT $2",
    )
    .bind(task_id)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;

    Ok((
        transfers
            .into_iter()
            .map(|r| TransferRow {
                id: r.0,
                from_team_id: r.1,
                to_team_id: r.2,
                moved_by: r.3,
                moved_at: r.4,
                note: r.5,
            })
            .collect(),
        promotions
            .into_iter()
            .map(|r| PromotionRow {
                id: r.0,
                environment_id: r.1,
                release_id: r.2,
                promoted_by: r.3,
                promoted_at: r.4,
            })
            .collect(),
        verifications
            .into_iter()
            .map(|r| VerificationRow {
                id: r.0,
                environment_id: r.1,
                verdict: r.2,
                verified_by: r.3,
                verified_at: r.4,
                note: r.5,
            })
            .collect(),
    ))
}

#[cfg(test)]
#[path = "custody_tests.rs"]
mod tests;

/// Which court a task is in, counted and sampled for one person.
///
/// # Why this is one query family and not four filters
///
/// `docs/45` defines "whose turn is it" as a derived fact over the owning team,
/// the assignees and the verification history. Composing it from four list
/// requests would put that definition in the client — a second copy of a domain
/// rule, in another language, which is the failure `docs/42` §Permissions warns
/// about for authority and which applies here for the same reason.
#[derive(Debug, Clone, Default)]
pub struct Queue {
    /// Assigned to the caller and still open. A developer's day.
    pub mine: Vec<(crate::task::TaskRow, String)>,
    /// Owned by a team the caller is in, and nobody has picked it up.
    pub team_queue: Vec<(crate::task::TaskRow, String)>,
    /// Open and owned by no team at all. `docs/45`: not missing data — the
    /// triage queue, and the list a lead opens first.
    pub triage: Vec<(crate::task::TaskRow, String)>,
    /// Pushed to an environment and not passed there since. QA's list.
    pub awaiting_verification: Vec<(crate::task::TaskRow, String)>,
}

/// The four courts, newest first, with the project key each card needs.
///
/// Full rows and not ids: the alternative is a second read to turn ids into
/// cards, and the same projection every other surface uses is already available
/// here. Bounded per bucket — a home screen is a glance (`docs/44` §The
/// moments), and a person with 400 open tasks needs a different conversation
/// rather than a longer list.
///
/// # Errors
///
/// Any database error.
pub async fn queue(
    scoped: &mut Scoped<'_>,
    viewer: &crate::project::Viewer,
    limit: i64,
) -> Result<Queue, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let visible = crate::project::VISIBLE;

    // One statement per court rather than one clever union: each is a different
    // question, they are read together but not combined, and a union would need
    // a discriminator column that exists only to be split apart again.
    let bucket = |predicate: &str| {
        format!(
            "SELECT {columns}, p.key AS project_key
               FROM task t
               JOIN project p ON p.id = t.project_id
              WHERE t.workspace_id = $1
                AND t.deleted_at IS NULL
                AND p.deleted_at IS NULL
                AND t.state NOT IN ('COMPLETED','CANCELED')
                AND {visible}
                AND {predicate}
              ORDER BY t.updated_at DESC
              LIMIT $5",
            columns = crate::task::COLUMNS
        )
    };

    // One decode for all four, so a bucket cannot render a card differently
    // from its neighbour.
    let decode = |rows: Vec<sqlx::postgres::PgRow>| -> Result<Vec<(crate::task::TaskRow, String)>, sqlx::Error> {
        use sqlx::Row as _;
        rows.iter()
            .map(|row| Ok((crate::task::row_of(row)?, row.try_get("project_key")?)))
            .collect()
    };

    let mine: Vec<sqlx::postgres::PgRow> = sqlx::query(&bucket(
        "EXISTS (SELECT 1 FROM task_assignee a
                  WHERE a.task_id = t.id AND a.user_id = $3)",
    ))
    .bind(workspace)
    .bind(&viewer.teams)
    .bind(viewer.actor)
    .bind(&viewer.granted_projects)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;

    let team_queue: Vec<sqlx::postgres::PgRow> = sqlx::query(&bucket(
        "t.team_id = ANY($2)
             AND NOT EXISTS (SELECT 1 FROM task_assignee a WHERE a.task_id = t.id)",
    ))
    .bind(workspace)
    .bind(&viewer.teams)
    .bind(viewer.actor)
    .bind(&viewer.granted_projects)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;

    let triage: Vec<sqlx::postgres::PgRow> = sqlx::query(&bucket("t.team_id IS NULL"))
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(limit)
        .fetch_all(scoped.conn())
        .await?;

    // "Pushed and not passed since." Compared against the last PROMOTION rather
    // than against the task's state, because the two clocks are independent: a
    // task can be moved back to In Progress and still be sitting on qa awaiting
    // a verdict, and a state-based test would lose it.
    let awaiting_verification: Vec<sqlx::postgres::PgRow> = sqlx::query(&bucket(
        "EXISTS (SELECT 1 FROM task_environment_promotion pr
                  WHERE pr.task_id = t.id
                    AND NOT EXISTS (
                        SELECT 1 FROM task_verification v
                         WHERE v.task_id = t.id
                           AND v.environment_id = pr.environment_id
                           AND v.verdict = 'PASS'
                           AND v.verified_at > pr.promoted_at))",
    ))
    .bind(workspace)
    .bind(&viewer.teams)
    .bind(viewer.actor)
    .bind(&viewer.granted_projects)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await?;

    Ok(Queue {
        mine: decode(mine)?,
        team_queue: decode(team_queue)?,
        triage: decode(triage)?,
        awaiting_verification: decode(awaiting_verification)?,
    })
}
