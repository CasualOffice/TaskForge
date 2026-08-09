//! The project repository (C-006).
//!
//! # Visibility is a predicate, never a post-filter
//!
//! `docs/04` §Visibility vs permission defines `visible(actor, project)`, and
//! `docs/04` §The list problem says post-filtering an authorized page "is a bug,
//! not an optimization: it silently shrinks pages and breaks cursors". So the
//! rule is compiled into one private `VISIBLE` predicate and joined onto every
//! read in this module —
//! the list, the single read, and the accessible-project set that the task
//! queries are filtered by all use the *same* text.
//!
//! One consequence worth stating: a project in another workspace, and a project
//! in this workspace the actor cannot see, both produce **no row**. The handler
//! has nothing to distinguish, which is what makes `404, never 403` structural
//! rather than a rule someone has to remember (`docs/04`).

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// `docs/04` §Visibility vs permission, as SQL.
///
/// Parameter positions are fixed, and shared by every query in this module:
///
/// | | |
/// | --- | --- |
/// | `$1` | workspace |
/// | `$2` | the actor's teams |
/// | `$3` | the actor |
/// | `$4` | projects the actor holds a `PROJECT`-scoped grant on |
///
/// `$4` is the literal reading of "actor holds any grant scoped to this
/// project". A workspace-scoped grant deliberately does **not** confer
/// visibility: `docs/04` resolves "Member everywhere except this one project"
/// by making that project private, and widening this clause would take that
/// answer away.
pub(crate) const VISIBLE: &str = "(   p.visibility = 'WORKSPACE'
                        OR (p.visibility = 'TEAM'
                            AND EXISTS (SELECT 1 FROM project_team pt
                                         WHERE pt.project_id = p.id
                                           AND pt.team_id = ANY($2)))
                        OR EXISTS (SELECT 1 FROM project_membership pm
                                    WHERE pm.project_id = p.id AND pm.user_id = $3)
                        OR p.id = ANY($4))";

/// The columns every read in this module returns, in [`ProjectRow`] order.
///
/// **One list, used everywhere.** There were two — a `p.`-qualified spelling for
/// the `SELECT`s and a bare one for `RETURNING` — kept in step by a test that
/// compared them. `INSERT INTO project AS p` and `UPDATE project p` both accept
/// an alias, so the second spelling is not needed, and a list that does not
/// exist cannot drift from the one that does.
///
/// `team_ids` is a correlated `ARRAY(...)` rather than a join or a second
/// query. A join would multiply the project row by its team count and break the
/// keyset page size; a second query per row is the N+1 `docs/04` §The list
/// problem exists to forbid. `ARRAY()` yields `{}` for a project with no teams,
/// which is `docs/03`'s "a project with no teams is still legal" without a
/// `COALESCE` anyone could forget.
pub(crate) const COLUMNS: &str = "p.id, p.workspace_id, p.key, p.name, p.description,
                       p.visibility::text AS visibility, p.workflow_id, p.created_at, p.created_by,
                       p.updated_at, p.updated_by, p.version, p.archived_at,
                       ARRAY(SELECT pt.team_id FROM project_team pt WHERE pt.project_id = p.id ORDER BY pt.team_id) AS team_ids";

/// A project as stored.
#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    /// `PRIVATE` | `TEAM` | `WORKSPACE`.
    pub visibility: String,
    pub workflow_id: Uuid,
    pub created_at: OffsetDateTime,
    pub created_by: Uuid,
    pub updated_at: OffsetDateTime,
    pub updated_by: Option<Uuid>,
    pub version: i64,
    pub archived_at: Option<OffsetDateTime>,
    /// Every team on this project, ascending (`docs/03` §Teams on a project).
    /// Empty is legal and means the project has no team.
    pub team_ids: Vec<Uuid>,
}

pub(crate) type ProjectTuple = (
    Uuid,
    Uuid,
    String,
    String,
    Option<String>,
    String,
    Uuid,
    OffsetDateTime,
    Uuid,
    OffsetDateTime,
    Option<Uuid>,
    i64,
    Option<OffsetDateTime>,
    Vec<Uuid>,
);

pub(crate) fn row_of(t: ProjectTuple) -> ProjectRow {
    ProjectRow {
        id: t.0,
        workspace_id: t.1,
        key: t.2,
        name: t.3,
        description: t.4,
        visibility: t.5,
        workflow_id: t.6,
        created_at: t.7,
        created_by: t.8,
        updated_at: t.9,
        updated_by: t.10,
        version: t.11,
        archived_at: t.12,
        team_ids: t.13,
    }
}

/// Who is asking, expanded once per request.
///
/// A struct rather than four positional arguments because the four are always
/// passed together and swapping `actor` with a project id would still compile.
#[derive(Debug, Clone)]
pub struct Viewer {
    pub actor: Uuid,
    pub teams: Vec<Uuid>,
    /// Projects the actor holds a `PROJECT`-scoped grant on.
    pub granted_projects: Vec<Uuid>,
}

/// What a project create supplies. Everything else is a database default.
#[derive(Debug, Clone)]
pub struct NewProject {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    /// `PRIVATE` | `TEAM` | `WORKSPACE`.
    pub visibility: String,
    pub workflow_id: Uuid,
    pub created_by: Uuid,
}

/// The fields `PATCH /projects/{id}` may change.
///
/// `Option<Option<T>>` on `description` is `docs/05` §Conventions: "absent =
/// leave unchanged; `null` = clear". One `Option` cannot express both, and
/// collapsing them is how a `PATCH {}` silently wipes a field.
#[derive(Debug, Clone, Default)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub visibility: Option<String>,
}

/// Why a create was refused.
#[derive(Debug)]
pub enum CreateError {
    /// `UNIQUE (workspace_id, key)` — `TF-PRJ-0002`.
    KeyTaken,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for CreateError {
    fn from(error: sqlx::Error) -> Self {
        match &error {
            sqlx::Error::Database(db) if db.is_unique_violation() => Self::KeyTaken,
            _ => Self::Db(error),
        }
    }
}

impl ProjectRow {
    /// The project's teams, typed for the authorization layer.
    ///
    /// `docs/03` §Teams on a project makes the team position of the scope chain
    /// a set. Every authority question about a project passes this, so there is
    /// one place that turns stored ids into `TeamId` and no handler can pass a
    /// team the project does not actually have.
    #[must_use]
    pub fn teams(&self) -> Vec<casual_task_model::TeamId> {
        self.team_ids
            .iter()
            .copied()
            .map(casual_task_model::TeamId::from_uuid)
            .collect()
    }
}

/// The cursor position for the project list: `(created_at, id)`.
pub type ProjectCursor = (OffsetDateTime, Uuid);

/// One page of the projects `viewer` can see, newest first.
///
/// `limit` is the caller's page size; **one more row than that is fetched**, so
/// "is there a next page" is answered without a second `COUNT` over the match
/// set (`docs/05` §Pagination).
///
/// # Errors
///
/// Any database error.
pub async fn list_visible(
    scoped: &mut Scoped<'_>,
    viewer: &Viewer,
    after: Option<ProjectCursor>,
    limit: u32,
) -> Result<Vec<ProjectRow>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    // Row-value comparison, not the expanded `a < x OR (a = x AND b < y)` form:
    // `docs/26` — PostgreSQL can drive a composite index from the row-value
    // form and often cannot from the expanded one. NULL means "first page",
    // and `$5 IS NULL OR ...` keeps that in one statement rather than two that
    // could drift.
    let sql = format!(
        "SELECT {COLUMNS}
           FROM project p
          WHERE p.workspace_id = $1
            AND p.deleted_at IS NULL
            AND {VISIBLE}
            AND ($5::timestamptz IS NULL
                 OR (p.created_at, p.id) < ($5::timestamptz, $6::uuid))
          ORDER BY p.created_at DESC, p.id DESC
          LIMIT $7"
    );
    let rows: Vec<ProjectTuple> = sqlx::query_as(&sql)
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(after.map(|c| c.0))
        .bind(after.map(|c| c.1))
        .bind(i64::from(limit).saturating_add(1))
        .fetch_all(scoped.conn())
        .await?;
    Ok(rows.into_iter().map(row_of).collect())
}

/// One project, or `None` when it does not exist **or** is not visible.
///
/// The two are the same answer on purpose — see the module docs.
///
/// # Errors
///
/// Any database error.
pub async fn read_visible(
    scoped: &mut Scoped<'_>,
    viewer: &Viewer,
    id: Uuid,
) -> Result<Option<ProjectRow>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let sql = format!(
        "SELECT {COLUMNS}
           FROM project p
          WHERE p.id = $5
            AND p.workspace_id = $1
            AND p.deleted_at IS NULL
            AND {VISIBLE}"
    );
    let row: Option<ProjectTuple> = sqlx::query_as(&sql)
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(id)
        .fetch_optional(scoped.conn())
        .await?;
    Ok(row.map(row_of))
}

/// The accessible project set — `docs/04` §The list problem, step 1.
///
/// Returned as `(id, key)` because every task representation carries the human
/// key `WR-125`, which spans `project.key` and `task.number`. Resolving it once
/// per request here is what keeps the task list from joining `project` per row.
///
/// Bounded by `limit`: `docs/26` notes that beyond a few hundred projects the
/// `= ANY(array)` filter stops being efficient and the set is materialized
/// instead. Truncating rather than growing without bound keeps the failure
/// visible (a missing project) rather than gradual (a slow query).
///
/// # Errors
///
/// Any database error.
pub async fn accessible(
    scoped: &mut Scoped<'_>,
    viewer: &Viewer,
    limit: u32,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let sql = format!(
        "SELECT p.id, p.key
           FROM project p
          WHERE p.workspace_id = $1
            AND p.deleted_at IS NULL
            AND {VISIBLE}
          ORDER BY p.id
          LIMIT $5"
    );
    sqlx::query_as(&sql)
        .bind(workspace)
        .bind(&viewer.teams)
        .bind(viewer.actor)
        .bind(&viewer.granted_projects)
        .bind(i64::from(limit))
        .fetch_all(scoped.conn())
        .await
}

/// Insert a project.
///
/// `workspace_id` comes from the scope, never from an argument: `Scoped` is the
/// only thing that knows which tenant this transaction is for, so the row
/// written and the policy enforced cannot disagree.
///
/// # Errors
///
/// [`CreateError::KeyTaken`] when the key is in use, otherwise the database
/// error.
pub async fn insert(scoped: &mut Scoped<'_>, new: &NewProject) -> Result<ProjectRow, CreateError> {
    let workspace = scoped.workspace_id().as_uuid();
    // `AS p` so the one column list above serves this statement too — see its
    // docs for why there is only one.
    let sql = format!(
        "INSERT INTO project AS p
             (id, workspace_id, key, name, description, visibility,
              workflow_id, created_by)
         VALUES ($1,$2,$3,$4,$5,$6::visibility,$7,$8)
         RETURNING {COLUMNS}"
    );
    let row: ProjectTuple = sqlx::query_as(&sql)
        .bind(new.id)
        .bind(workspace)
        .bind(&new.key)
        .bind(&new.name)
        .bind(new.description.as_deref())
        .bind(&new.visibility)
        .bind(new.workflow_id)
        .bind(new.created_by)
        .fetch_one(scoped.conn())
        .await?;
    Ok(row_of(row))
}

/// A project member row. `project_membership` conveys **belonging, never
/// capability** (migration 0003) — it satisfies the `is_project_member`
/// constraint and confers visibility, and grants nothing on its own.
///
/// # Errors
///
/// Any database error.
pub async fn add_member(
    scoped: &mut Scoped<'_>,
    project: Uuid,
    user: Uuid,
) -> Result<(), sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    sqlx::query(
        "INSERT INTO project_membership (project_id, user_id, workspace_id)
         VALUES ($1,$2,$3)
         ON CONFLICT (project_id, user_id) DO NOTHING",
    )
    .bind(project)
    .bind(user)
    .bind(workspace)
    .execute(scoped.conn())
    .await?;
    Ok(())
}

/// Whether the actor holds a `project_membership` row for this project.
///
/// The `is_project_member` constraint's only input (`docs/04` §Constraint set).
/// Read separately from the visibility predicate because the two answer
/// different questions: visibility is satisfied by *any* of four routes, and a
/// constraint needs to know whether this particular one holds.
///
/// # Errors
///
/// Any database error.
pub async fn is_member(
    scoped: &mut Scoped<'_>,
    project: Uuid,
    user: Uuid,
) -> Result<bool, sqlx::Error> {
    let found: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM project_membership WHERE project_id = $1 AND user_id = $2",
    )
    .bind(project)
    .bind(user)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(found.is_some())
}

/// Apply a patch, guarded by `expected_version`.
///
/// `None` means **no row matched the version** — the `409` path. The caller
/// re-reads to build the conflict body `docs/24` requires, rather than this
/// function guessing why (`docs/24`: 0 rows affected ⇒ someone else wrote
/// first).
///
/// # Errors
///
/// Any database error.
pub async fn update(
    scoped: &mut Scoped<'_>,
    id: Uuid,
    expected_version: i64,
    patch: &ProjectPatch,
    actor: Uuid,
) -> Result<Option<ProjectRow>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    // COALESCE against a NULL parameter is what makes "absent = unchanged"
    // one statement. `description` needs the extra flag because NULL is a
    // meaningful *value* there, not only "unchanged".
    let sql = format!(
        "UPDATE project p
            SET name        = COALESCE($4::text, p.name),
                description = CASE WHEN $5 THEN $6::text ELSE p.description END,
                visibility  = COALESCE($7::visibility, p.visibility),
                updated_at  = now(),
                updated_by  = $8,
                version     = p.version + 1
          WHERE p.id = $1
            AND p.workspace_id = $2
            AND p.deleted_at IS NULL
            AND p.version = $3
        RETURNING {COLUMNS}"
    );
    let row: Option<ProjectTuple> = sqlx::query_as(&sql)
        .bind(id)
        .bind(workspace)
        .bind(expected_version)
        .bind(patch.name.as_deref())
        .bind(patch.description.is_some())
        .bind(patch.description.clone().flatten())
        .bind(patch.visibility.as_deref())
        .bind(actor)
        .fetch_optional(scoped.conn())
        .await?;
    Ok(row.map(row_of))
}

/// Allocate the next task number for a project, in the caller's transaction.
///
/// ADR-008: allocated in-transaction rather than from a sequence, because a
/// sequence leaks numbers on rollback and users read gaps as lost data. The
/// `UPDATE ... RETURNING` takes a row lock, so two concurrent creates in one
/// project serialize here rather than colliding on `UNIQUE (project_id,
/// number)`.
///
/// # Errors
///
/// Any database error.
pub async fn allocate_number(scoped: &mut Scoped<'_>, project: Uuid) -> Result<i64, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    sqlx::query_scalar(
        "UPDATE project SET task_seq = task_seq + 1
          WHERE id = $1 AND workspace_id = $2
      RETURNING task_seq",
    )
    .bind(project)
    .bind(workspace)
    .fetch_one(scoped.conn())
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_visibility_predicate_matches_the_four_documented_routes() {
        // docs/04 §Visibility vs permission lists exactly four ways in. A route
        // silently dropped here becomes a project somebody can no longer see,
        // and nothing else in the system would report it.
        assert!(VISIBLE.contains("p.visibility = 'WORKSPACE'"));
        assert!(VISIBLE.contains("p.visibility = 'TEAM'"));
        assert!(VISIBLE.contains("project_membership"));
        assert!(VISIBLE.contains("p.id = ANY($4)"));
    }

    #[test]
    fn team_visibility_reads_the_join_and_not_the_superseded_column() {
        // Migration 0027 replaced `project.team_id` with `project_team`. A
        // predicate left on the old column would answer for the FIRST team a
        // project ever had and silently hide it from every other one.
        assert!(VISIBLE.contains("project_team"));
        assert!(!VISIBLE.contains("p.team_id"));
        assert!(!COLUMNS.contains("p.team_id"));
    }

    #[test]
    fn no_read_in_this_module_paginates_by_offset() {
        // docs/26 bans it outright and casual-task-lint bans the token; this
        // asserts the shared fragments cannot smuggle one in.
        for sql in [VISIBLE, COLUMNS] {
            assert!(!sql.to_uppercase().contains("OFFSET "), "{sql}");
        }
    }
}
