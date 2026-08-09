//! The comment repository (C-009).
//!
//! # Mentions are resolved at write time
//!
//! `migrations/0006` stores `mentions uuid[]` and says so: "resolved at write
//! time". A comment rendered years later must still say who was notified, and
//! re-parsing the body then would resolve `@sam` against today's directory —
//! a different Sam, or nobody. The array is the record of what actually
//! happened.
//!
//! # Threading is one level, enforced here
//!
//! `parent_comment_id` references `comment(id)`, which permits arbitrary depth
//! at the schema level. `docs/06` says one level. A reply to a reply is refused
//! by [`create`] rather than by a convention, because the alternative is a
//! thread nobody can render and a database nobody can migrate out of.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// A comment as stored.
#[derive(Debug, Clone)]
pub struct CommentRow {
    pub id: Uuid,
    pub task_id: Uuid,
    pub parent_comment_id: Option<Uuid>,
    pub author_id: Uuid,
    pub body: String,
    pub mentions: Vec<Uuid>,
    pub created_at: OffsetDateTime,
    pub edited_at: Option<OffsetDateTime>,
    pub version: i64,
}

/// Why a comment could not be written.
/// `thiserror` is not a dependency of this crate — every sibling repository
/// returns `sqlx::Error` — so the impls are written out rather than adding one
/// for a three-variant enum.
#[derive(Debug)]
pub enum CommentError {
    /// The task does not exist, or is not visible in this scope. One variant
    /// for both: `docs/04` requires absent and invisible to be
    /// indistinguishable, and two variants here would become two status codes
    /// at the boundary.
    NoSuchTask,
    /// The parent is not a comment on this task, or is itself a reply.
    BadParent,
    Database(sqlx::Error),
}

impl std::fmt::Display for CommentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchTask => f.write_str("no such task"),
            Self::BadParent => f.write_str("the parent is not a top-level comment on this task"),
            Self::Database(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CommentError {}

impl From<sqlx::Error> for CommentError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

const COLUMNS: &str = "c.id, c.task_id, c.parent_comment_id, c.author_id, c.body, \
                       c.mentions, c.created_at, c.edited_at, c.version";

type Row = (
    Uuid,
    Uuid,
    Option<Uuid>,
    Uuid,
    String,
    Vec<Uuid>,
    OffsetDateTime,
    Option<OffsetDateTime>,
    i64,
);

fn to_row(r: Row) -> CommentRow {
    CommentRow {
        id: r.0,
        task_id: r.1,
        parent_comment_id: r.2,
        author_id: r.3,
        body: r.4,
        mentions: r.5,
        created_at: r.6,
        edited_at: r.7,
        version: r.8,
    }
}

/// Post a comment.
///
/// `workspace_id` is written from the scope, never from a parameter, so the row
/// and the policy that will guard it cannot disagree.
///
/// # Errors
///
/// [`CommentError::NoSuchTask`] if the task is not visible in this scope,
/// [`CommentError::BadParent`] if `parent` is not a top-level comment on the
/// same task, or any database error.
pub async fn create(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    author: Uuid,
    body: &str,
    parent: Option<Uuid>,
    mentions: &[Uuid],
) -> Result<CommentRow, CommentError> {
    let workspace = scoped.workspace_id().as_uuid();

    // The task must be visible in THIS scope. Row-level security already hides
    // another tenant's task, so this reads as "not found" rather than as a
    // permission error — which is what `docs/04` requires anyway.
    let task_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM task WHERE id = $1 AND archived_at IS NULL)",
    )
    .bind(task_id)
    .fetch_one(scoped.conn())
    .await?;
    if !task_exists {
        return Err(CommentError::NoSuchTask);
    }

    if let Some(parent_id) = parent {
        // One level: the parent must exist, belong to the same task, and itself
        // be top-level. Checked in one query so a concurrent reply cannot slip
        // between an existence check and a depth check.
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM comment
                 WHERE id = $1 AND task_id = $2
                   AND parent_comment_id IS NULL
                   AND deleted_at IS NULL)",
        )
        .bind(parent_id)
        .bind(task_id)
        .fetch_one(scoped.conn())
        .await?;
        if !valid {
            return Err(CommentError::BadParent);
        }
    }

    let row: Row = sqlx::query_as(&format!(
        "INSERT INTO comment
             (id, workspace_id, task_id, parent_comment_id, author_id, body, mentions)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         RETURNING {}",
        COLUMNS.replace("c.", "")
    ))
    .bind(Uuid::now_v7())
    .bind(workspace)
    .bind(task_id)
    .bind(parent)
    .bind(author)
    .bind(body)
    .bind(mentions)
    .fetch_one(scoped.conn())
    .await?;

    Ok(to_row(row))
}

/// One page of a task's thread, oldest first.
///
/// Keyset, never `OFFSET` — `docs/26` bans it and `casual-task-lint` enforces
/// the ban. The cursor is `(created_at, id)` because two comments posted in the
/// same millisecond would otherwise make a page boundary ambiguous, and the
/// row that falls in the gap is the one nobody ever sees.
///
/// # Errors
///
/// Any database error.
pub async fn thread(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    after: Option<(OffsetDateTime, Uuid)>,
    limit: i64,
) -> Result<Vec<CommentRow>, sqlx::Error> {
    let rows: Vec<Row> = match after {
        None => {
            sqlx::query_as(&format!(
                "SELECT {COLUMNS} FROM comment c
                  WHERE c.task_id = $1 AND c.deleted_at IS NULL
                  ORDER BY c.created_at, c.id
                  LIMIT $2"
            ))
            .bind(task_id)
            .bind(limit)
            .fetch_all(scoped.conn())
            .await?
        }
        Some((at, id)) => {
            // Row-value comparison, not `created_at > $2 OR (= AND id > $3)`:
            // the row form is what lets the planner use `comment_task_ix`
            // directly, and the expanded form does not.
            sqlx::query_as(&format!(
                "SELECT {COLUMNS} FROM comment c
                  WHERE c.task_id = $1 AND c.deleted_at IS NULL
                    AND (c.created_at, c.id) > ($2, $3)
                  ORDER BY c.created_at, c.id
                  LIMIT $4"
            ))
            .bind(task_id)
            .bind(at)
            .bind(id)
            .bind(limit)
            .fetch_all(scoped.conn())
            .await?
        }
    };
    Ok(rows.into_iter().map(to_row).collect())
}

/// Read one comment.
///
/// # Errors
///
/// Any database error.
pub async fn read(scoped: &mut Scoped<'_>, id: Uuid) -> Result<Option<CommentRow>, sqlx::Error> {
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM comment c WHERE c.id = $1 AND c.deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(row.map(to_row))
}

/// Edit a comment's body, with optimistic concurrency.
///
/// Returns `None` when the version does not match, which the boundary turns
/// into a 409 — `docs/05` §Concurrency. The version is compared in the `WHERE`
/// clause rather than read first, so two concurrent edits cannot both see the
/// same version and both win.
///
/// # Errors
///
/// Any database error.
pub async fn edit(
    scoped: &mut Scoped<'_>,
    id: Uuid,
    author: Uuid,
    body: &str,
    mentions: &[Uuid],
    expected_version: i64,
) -> Result<Option<CommentRow>, sqlx::Error> {
    let row: Option<Row> = sqlx::query_as(&format!(
        "UPDATE comment c
            SET body = $3, mentions = $4, edited_at = now(), version = c.version + 1
          WHERE c.id = $1 AND c.author_id = $2 AND c.version = $5
            AND c.deleted_at IS NULL
        RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(author)
    .bind(body)
    .bind(mentions)
    .bind(expected_version)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(row.map(to_row))
}

/// Soft-delete a comment.
///
/// Soft, because a thread with a hole in it reads as though something was
/// hidden, and because `docs/25`'s activity stream references the comment by id
/// forever. Returns whether a row was affected.
///
/// # Errors
///
/// Any database error.
pub async fn soft_delete(
    scoped: &mut Scoped<'_>,
    id: Uuid,
    expected_version: i64,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE comment SET deleted_at = now(), version = version + 1
          WHERE id = $1 AND version = $2 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(expected_version)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected == 1)
}
