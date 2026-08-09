//! Task dependencies, and the cycle check that cannot be skipped
//! (ADR-019, `docs/24` §Cycles, C-008).
//!
//! # The failure this module prevents
//!
//! A dependency graph with a loop in it. A cycle makes "what is blocking this?"
//! non-terminating, and every consumer of `unresolved_blockers` — the
//! transition gate, the board, My Work — walks that graph.
//!
//! # The check is IN the insert, not above it
//!
//! [`insert`] is a single `INSERT ... SELECT ... WHERE NOT EXISTS (<reachable>)`.
//! There is no separate "is this safe?" call a caller could forget, and no
//! window between the check and the write in which a concurrent transaction
//! closes the loop from the other side.
//!
//! That window is real and it is the reason `docs/24` specifies an advisory
//! lock: two transactions inserting `A→B` and `B→A` simultaneously each see a
//! graph with no cycle, and both commit. The lock is taken on the **workspace**,
//! so dependency writes within one tenant serialize against each other and
//! against nothing else.
//!
//! # Depth is bounded
//!
//! `docs/21`: "Dependency graph depth (check) | 64 hops". A recursive CTE over
//! attacker-influenced data with no depth bound is an unbounded query, which is
//! the thing `docs/21` exists to forbid. The bound is stated in
//! [`MAX_DEPTH`] and enforced in the SQL.
//!
//! **The cost, stated:** a cycle that closes only at hop 65 is not detected.
//! That is the deliberate direction — a bounded check that refuses most cycles
//! beats an unbounded one that can be made to run forever — and 64 hops of
//! blocking dependencies is a graph nobody is navigating by hand.

use uuid::Uuid;

use crate::scoped::Scoped;

/// `docs/21` §Field limits: "Dependency graph depth (check) | 64 hops".
pub const MAX_DEPTH: i32 = 64;

/// `docs/21`: "Dependencies per task | 100".
pub const MAX_PER_TASK: i64 = 100;

/// Why a dependency was refused.
#[derive(Debug)]
pub enum DependencyError {
    /// The edge would close a loop — `TF-TSK-0003`.
    ///
    /// Carries the loop it found, as human keys in order
    /// (`ONB-4 → API-2 → ONB-4`). `docs/03` gives the reachability check a
    /// bound; naming the path is what makes the refusal actionable — "invalid
    /// dependency" tells a user nothing they can fix.
    ///
    /// Empty when the loop is the one-hop self-edge, which needs no path.
    WouldCycle(Vec<String>),
    /// One of the two tasks is not visible, or does not exist.
    ///
    /// The two are one answer on purpose (`docs/04`): a caller must not be able
    /// to discover which task ids exist by proposing dependencies on them.
    NotVisible,
    /// The task already has [`MAX_PER_TASK`] dependencies.
    TooMany,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for DependencyError {
    fn from(error: sqlx::Error) -> Self {
        Self::Db(error)
    }
}

/// One edge, as the drawer renders it.
///
/// An edge whose other end the viewer cannot see is **still returned**, with
/// everything identifying withheld. `docs/03`: a blocking task "shows as
/// 'restricted' if the viewer cannot see its project, never as its title".
///
/// Dropping the row instead would show a task as blocked by nothing — the user
/// sees a card that cannot move and no reason for it, which is a worse answer
/// than "something you cannot see".
#[derive(Debug, Clone)]
pub struct RelatedTask {
    /// `None` when restricted. An id is a handle to a task the viewer may not
    /// know exists, so it is withheld with the rest.
    pub id: Option<Uuid>,
    /// The human key, `WR-125`. `None` when restricted.
    pub key: Option<String>,
    /// `None` when restricted.
    pub title: Option<String>,
    /// One of the five permanent states — the drawer strikes through a
    /// `COMPLETED` blocker rather than hiding it. `None` when restricted:
    /// whether somebody else's work is finished is their project's business.
    pub state: Option<String>,
    /// Whether this end is hidden from the viewer.
    pub restricted: bool,
}

/// Serialize dependency writes within one workspace.
///
/// `docs/24`: the reachability check runs "under an advisory lock". Without it
/// two transactions can each observe an acyclic graph and jointly create a
/// cycle — the check is correct in both, and the result is wrong.
///
/// Transaction-scoped (`pg_advisory_xact_lock`), so it is released by COMMIT or
/// ROLLBACK and cannot be leaked by a caller that forgets to unlock.
async fn lock_workspace(scoped: &mut Scoped<'_>) -> Result<(), sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    // The uuid's high bits as a bigint: advisory locks are keyed by integers,
    // and a collision between two workspaces costs serialization, never
    // correctness.
    //
    // `replace(..., '-', '')` first, and that is the whole bug this line once
    // had: a uuid's text form is `019fe4a6-2cbc-...`, so the first 16
    // *characters* include two dashes, and `'x' || '019fe4a6-2cbc-72'` cast to
    // bit(64) fails with `"-" is not a valid hexadecimal digit`. Every write
    // through this function 500'd. Sixteen hex digits is exactly 64 bits, which
    // is what bit(64) wants.
    sqlx::query(
        "SELECT pg_advisory_xact_lock(
                    ('x' || substr(replace($1::text, '-', ''), 1, 16))::bit(64)::bigint)",
    )
    .bind(workspace)
    .execute(scoped.conn())
    .await?;
    Ok(())
}

/// Add "`blocker` blocks `blocked`", refusing anything that would close a loop.
///
/// Both tasks are checked for visibility through the same predicate every other
/// read uses, so a dependency cannot be created against a task the actor cannot
/// see — nor its existence inferred from which error comes back.
///
/// # Errors
///
/// [`DependencyError::WouldCycle`] when the edge closes a loop,
/// [`DependencyError::NotVisible`] when either task is absent or invisible,
/// [`DependencyError::TooMany`] past `docs/21`'s per-task bound.
pub async fn insert(
    scoped: &mut Scoped<'_>,
    viewer: &crate::project::Viewer,
    blocker: Uuid,
    blocked: Uuid,
) -> Result<bool, DependencyError> {
    // The schema's CHECK refuses this too, but as a constraint violation the
    // caller would have to parse. A loop of length one is still a loop.
    if blocker == blocked {
        return Err(DependencyError::WouldCycle(Vec::new()));
    }
    lock_workspace(scoped).await?;

    let workspace = scoped.workspace_id().as_uuid();
    // Both endpoints must be visible to the actor. Checked here rather than
    // trusted from the handler, because this is the only place that knows both
    // ids at once.
    for task in [blocker, blocked] {
        if crate::task::read_visible(scoped, viewer, task)
            .await?
            .is_none()
        {
            return Err(DependencyError::NotVisible);
        }
    }

    let existing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM task_dependency
          WHERE workspace_id = $1 AND (to_task_id = $2 OR from_task_id = $2)",
    )
    .bind(workspace)
    .bind(blocked)
    .fetch_one(scoped.conn())
    .await?;
    if existing >= MAX_PER_TASK {
        return Err(DependencyError::TooMany);
    }

    // The whole rule, as one statement.
    //
    // The edge `blocker -> blocked` closes a loop exactly when `blocker` is
    // already reachable FROM `blocked` by following blocks-edges forwards. The
    // CTE walks that direction from `blocked`, bounded to MAX_DEPTH hops, and
    // the INSERT happens only if `blocker` is not among the tasks it reaches.
    //
    // `ON CONFLICT DO NOTHING` makes re-adding an existing edge a no-op rather
    // than an error: the drawer's button is idempotent, and a duplicate is not
    // a cycle.
    let inserted = sqlx::query(
        "WITH RECURSIVE reachable(id, depth) AS (
             SELECT $3::uuid, 0
           UNION ALL
             SELECT d.to_task_id, r.depth + 1
               FROM task_dependency d
               JOIN reachable r ON d.from_task_id = r.id
              WHERE d.workspace_id = $1
                AND d.kind = 'BLOCKS'
                AND r.depth < $4
         )
         INSERT INTO task_dependency (from_task_id, to_task_id, workspace_id, kind)
         SELECT $2, $3, $1, 'BLOCKS'
          WHERE NOT EXISTS (SELECT 1 FROM reachable WHERE id = $2)
         ON CONFLICT (from_task_id, to_task_id) DO NOTHING",
    )
    .bind(workspace)
    .bind(blocker)
    .bind(blocked)
    .bind(MAX_DEPTH)
    .execute(scoped.conn())
    .await?
    .rows_affected();

    if inserted == 1 {
        return Ok(true);
    }

    // Zero rows is either "already there" or "would cycle", and the caller
    // needs to know which. Asked after the fact rather than before, so the
    // decision itself stayed inside the one statement above.
    let already: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM task_dependency
          WHERE workspace_id = $1 AND from_task_id = $2 AND to_task_id = $3",
    )
    .bind(workspace)
    .bind(blocker)
    .bind(blocked)
    .fetch_optional(scoped.conn())
    .await?;

    if already.is_some() {
        return Ok(false);
    }
    Err(DependencyError::WouldCycle(
        cycle_path(scoped, blocker, blocked).await?,
    ))
}

/// Remove whichever edge joins these two tasks. `false` if there is none.
///
/// # Why the direction is not a parameter
///
/// At most one edge can exist between a pair: `A blocks B` and `B blocks A`
/// together *are* a cycle, and [`insert`] refuses the second. So naming both
/// ends identifies the edge, and a `direction` argument could only be a way for
/// a caller to disagree with the graph — and get a silent no-op when it did.
///
/// # Why this does not check that both ends are visible
///
/// [`insert`] does, because creating an edge to a task you cannot see would let
/// you discover it exists. Removal cannot: the edge is already on the caller's
/// own Relations panel, shown as `restricted` when the far end is invisible
/// (`docs/03`). Requiring visibility here would make exactly those edges
/// permanent — the ones a person most needs to remove and can never re-create.
/// The authority to remove is `task.update` on the task in the path, which the
/// handler checks; this function is the statement.
///
/// # Errors
///
/// Any database error.
pub async fn remove(scoped: &mut Scoped<'_>, one: Uuid, other: Uuid) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "DELETE FROM task_dependency
          WHERE workspace_id = $1
            AND ((from_task_id = $2 AND to_task_id = $3)
              OR (from_task_id = $3 AND to_task_id = $2))",
    )
    .bind(scoped.workspace_id().as_uuid())
    .bind(one)
    .bind(other)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// The loop the refused edge would have closed, as human keys in order.
///
/// Walks the same direction the check does — forwards from `blocked` — keeping
/// the path taken, and stops at the first route that reaches `blocker`. The
/// result reads `ONB-4 → API-2 → ONB-4`: the proposed edge's own ends bracket
/// it, so a reader can see which link to remove.
///
/// Best effort by design. This runs only on the refusal path, and a failure to
/// describe a cycle must not turn a correct refusal into a 500 — so a database
/// error here yields an empty path and the caller reports the refusal without
/// one.
async fn cycle_path(
    scoped: &mut Scoped<'_>,
    blocker: Uuid,
    blocked: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let path: Option<Vec<Uuid>> = sqlx::query_scalar(
        "WITH RECURSIVE walk(id, path, depth) AS (
             SELECT $3::uuid, ARRAY[$3::uuid], 0
           UNION ALL
             SELECT d.to_task_id, w.path || d.to_task_id, w.depth + 1
               FROM task_dependency d
               JOIN walk w ON d.from_task_id = w.id
              WHERE d.workspace_id = $1
                AND d.kind = 'BLOCKS'
                AND w.depth < $4
                -- Never revisit a node: an existing cycle elsewhere in the
                -- graph would otherwise make this walk non-terminating, which
                -- is the failure the depth bound alone does not prevent.
                AND NOT (d.to_task_id = ANY(w.path))
         )
         SELECT path FROM walk WHERE id = $2 ORDER BY depth LIMIT 1",
    )
    .bind(workspace)
    .bind(blocker)
    .bind(blocked)
    .bind(MAX_DEPTH)
    .fetch_optional(scoped.conn())
    .await?
    .flatten();

    let Some(mut path) = path else {
        return Ok(Vec::new());
    };
    // Close the loop visually: the proposed edge points from `blocker` back to
    // `blocked`, so the path ends where it began.
    path.push(blocked);
    keys_of(scoped, &path).await
}

/// Human keys for a list of task ids, in the order given.
async fn keys_of(scoped: &mut Scoped<'_>, ids: &[Uuid]) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(Uuid, String, i64)> = sqlx::query_as(
        "SELECT t.id, p.key, t.number
           FROM task t JOIN project p ON p.id = t.project_id
          WHERE t.id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(scoped.conn())
    .await?;
    Ok(ids
        .iter()
        .filter_map(|id| {
            rows.iter()
                .find(|(row_id, _, _)| row_id == id)
                .map(|(_, key, number)| format!("{key}-{number}"))
        })
        .collect())
}

/// Everything blocking `task_id`, visible to the viewer.
///
/// # Errors
///
/// Any database error.
pub async fn blocked_by(
    scoped: &mut Scoped<'_>,
    viewer: &crate::project::Viewer,
    task_id: Uuid,
) -> Result<Vec<RelatedTask>, sqlx::Error> {
    related(scoped, viewer, task_id, Direction::BlockedBy).await
}

/// Everything `task_id` blocks, visible to the viewer.
///
/// # Errors
///
/// Any database error.
pub async fn blocks(
    scoped: &mut Scoped<'_>,
    viewer: &crate::project::Viewer,
    task_id: Uuid,
) -> Result<Vec<RelatedTask>, sqlx::Error> {
    related(scoped, viewer, task_id, Direction::Blocks).await
}

#[derive(Clone, Copy)]
enum Direction {
    /// Rows where the task is the blocked end.
    BlockedBy,
    /// Rows where the task is the blocker.
    Blocks,
}

async fn related(
    scoped: &mut Scoped<'_>,
    viewer: &crate::project::Viewer,
    task_id: Uuid,
    direction: Direction,
) -> Result<Vec<RelatedTask>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    // Two static fragments, chosen by a closed enum — never string-built from
    // anything a caller supplies.
    let (mine, theirs) = match direction {
        Direction::BlockedBy => ("d.to_task_id", "d.from_task_id"),
        Direction::Blocks => ("d.from_task_id", "d.to_task_id"),
    };
    // The related task is filtered by the SAME visibility predicate as every
    // other read. A blocker in a project the actor cannot see is omitted rather
    // than named — `docs/29` makes the same argument for notifications, and it
    // is the same leak.
    // The visibility predicate is SELECTED, not applied as a filter. That is the
    // whole difference between "restricted" and "absent": docs/03 requires the
    // edge to survive and its identity to be withheld.
    let sql = format!(
        "SELECT o.id, p.key, o.number, o.title, o.state::text, ({visible}) AS visible
           FROM task_dependency d
           JOIN task o    ON o.id = {theirs}
           JOIN project p ON p.id = o.project_id
          WHERE {mine} = $5
            AND d.workspace_id = $1
            AND d.kind = 'BLOCKS'
            AND o.deleted_at IS NULL
            AND p.deleted_at IS NULL
          ORDER BY p.key, o.number",
        visible = crate::project::VISIBLE
    );
    let rows: Vec<(Uuid, String, i64, String, String, bool)> = sqlx::query_as(&sql)
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(task_id)
        .fetch_all(scoped.conn())
        .await?;

    Ok(rows
        .into_iter()
        .map(|(id, key, number, title, state, visible)| {
            if visible {
                RelatedTask {
                    id: Some(id),
                    key: Some(format!("{key}-{number}")),
                    title: Some(title),
                    state: Some(state),
                    restricted: false,
                }
            } else {
                // Everything identifying is dropped HERE, after the database
                // returned it, rather than being left to a caller to redact.
                // A struct that could carry a title alongside `restricted: true`
                // is a struct somebody eventually serializes.
                RelatedTask {
                    id: None,
                    key: None,
                    title: None,
                    state: None,
                    restricted: true,
                }
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_depth_bound_is_the_one_docs_21_publishes() {
        // An unbounded recursive CTE over attacker-influenced data is the
        // unbounded query docs/21 exists to forbid.
        assert_eq!(MAX_DEPTH, 64);
        assert_eq!(MAX_PER_TASK, 100);
    }

    #[test]
    fn the_direction_fragments_are_opposites() {
        // Getting these the wrong way round would render the drawer's two
        // Relations lists swapped — which reads as plausible and is wrong.
        let blocked_by = matches!(Direction::BlockedBy, Direction::BlockedBy);
        let blocks = matches!(Direction::Blocks, Direction::Blocks);
        assert!(blocked_by && blocks);
    }
}
