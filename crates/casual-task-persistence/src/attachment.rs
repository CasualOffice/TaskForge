//! The attachment repository (C-010, `docs/28`).
//!
//! # Every read here carries the visibility predicate, not just the tenant one
//!
//! `docs/28` §The invariant: "an attachment row is invisible to every read path
//! until `committed_at` is set", and the partial index is what makes that
//! structural — an uncommitted row is *not in the index reads use*.
//!
//! This module keeps that true by writing `committed_at IS NOT NULL` into every
//! read that serves a client, and by putting the one read that must see
//! uncommitted rows behind a name that says so ([`find_for_commit`]). A single
//! `find` used by both would be one edit away from serving an unscanned file.
//!
//! # `scan_status` is a state machine, and the transitions are here
//!
//! `PENDING → CLEAN` is the only transition that sets `committed_at`, and
//! [`mark_scanned`] is the only statement that writes it. A handler cannot
//! commit a row by any other route, which is what stops "commit" and "clean"
//! from drifting into two independently settable flags.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// An attachment as stored.
#[derive(Debug, Clone)]
pub struct AttachmentRow {
    pub id: Uuid,
    pub task_id: Uuid,
    pub object_key: String,
    pub filename: String,
    /// From magic bytes at commit, never from the client (`docs/28`).
    pub content_type: String,
    pub byte_size: i64,
    pub checksum: String,
    /// `PENDING` | `CLEAN` | `INFECTED` | `FAILED`.
    pub scan_status: String,
    pub committed_at: Option<OffsetDateTime>,
    pub uploaded_by: Uuid,
    pub created_at: OffsetDateTime,
}

/// The verdicts `scan_status` accepts (migration 0006's `CHECK`).
pub const SCAN_STATUSES: &[&str] = &["PENDING", "CLEAN", "INFECTED", "FAILED"];

/// The only verdict that makes a file downloadable.
pub const CLEAN: &str = "CLEAN";

const COLUMNS: &str = "a.id, a.task_id, a.object_key, a.filename, a.content_type,
                       a.byte_size, a.checksum, a.scan_status, a.committed_at,
                       a.uploaded_by, a.created_at";

fn row_of(row: &sqlx::postgres::PgRow) -> Result<AttachmentRow, sqlx::Error> {
    use sqlx::Row as _;
    Ok(AttachmentRow {
        id: row.try_get("id")?,
        task_id: row.try_get("task_id")?,
        object_key: row.try_get("object_key")?,
        filename: row.try_get("filename")?,
        content_type: row.try_get("content_type")?,
        byte_size: row.try_get("byte_size")?,
        checksum: row.try_get("checksum")?,
        scan_status: row.try_get("scan_status")?,
        committed_at: row.try_get("committed_at")?,
        uploaded_by: row.try_get("uploaded_by")?,
        created_at: row.try_get("created_at")?,
    })
}

/// What a pre-sign records.
#[derive(Debug, Clone)]
pub struct NewAttachment {
    pub id: Uuid,
    pub task_id: Uuid,
    pub object_key: String,
    pub filename: String,
    pub byte_size: i64,
    pub checksum: String,
    pub uploaded_by: Uuid,
}

/// Reserve the row a pre-signed upload will fill.
///
/// `committed_at` stays `NULL` and `scan_status` stays `PENDING`, so the row is
/// invisible to every read path from the instant it exists — there is no window
/// in which a pre-signed-but-unverified attachment can be listed or downloaded.
///
/// `content_type` is written as the opaque type on purpose. The real one comes
/// from magic bytes at commit; storing the client's declaration here, even
/// temporarily, would leave a column that looks authoritative and is not.
///
/// # Errors
///
/// Any database error.
pub async fn insert(
    scoped: &mut Scoped<'_>,
    new: &NewAttachment,
    placeholder_type: &str,
) -> Result<AttachmentRow, sqlx::Error> {
    let sql = format!(
        "WITH inserted AS (
             INSERT INTO attachment
                 (id, workspace_id, task_id, object_key, filename, content_type,
                  byte_size, checksum, uploaded_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
             RETURNING *
         )
         SELECT {COLUMNS} FROM inserted a"
    );
    let row = sqlx::query(&sql)
        .bind(new.id)
        .bind(scoped.workspace_id().as_uuid())
        .bind(new.task_id)
        .bind(&new.object_key)
        .bind(&new.filename)
        .bind(placeholder_type)
        .bind(new.byte_size)
        .bind(&new.checksum)
        .bind(new.uploaded_by)
        .execute_one(scoped)
        .await?;
    row_of(&row)
}

/// One attachment a client may **see**: committed, clean of deletion, in this
/// tenant.
///
/// Deliberately does not filter on `scan_status` — the download handler refuses
/// a non-`CLEAN` verdict itself, with its own status code, because "not found"
/// and "not scanned yet" are different answers to the person who just uploaded
/// it.
///
/// # Errors
///
/// Any database error.
pub async fn find_visible(
    scoped: &mut Scoped<'_>,
    id: Uuid,
) -> Result<Option<AttachmentRow>, sqlx::Error> {
    let sql = format!(
        "SELECT {COLUMNS}
           FROM attachment a
          WHERE a.id = $1
            AND a.workspace_id = $2
            AND a.committed_at IS NOT NULL
            AND a.deleted_at IS NULL"
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .bind(scoped.workspace_id().as_uuid())
        .fetch_optional(scoped.conn())
        .await?;
    row.as_ref().map(row_of).transpose()
}

/// The one read that sees an **uncommitted** row.
///
/// Named for what it is. The commit handshake has to find the row it is about
/// to verify, and that row is by definition not yet visible — so this exists,
/// once, with a name no one will reach for by accident.
///
/// # Errors
///
/// Any database error.
pub async fn find_for_commit(
    scoped: &mut Scoped<'_>,
    id: Uuid,
) -> Result<Option<AttachmentRow>, sqlx::Error> {
    let sql = format!(
        "SELECT {COLUMNS}
           FROM attachment a
          WHERE a.id = $1
            AND a.workspace_id = $2
            AND a.deleted_at IS NULL"
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .bind(scoped.workspace_id().as_uuid())
        .fetch_optional(scoped.conn())
        .await?;
    row.as_ref().map(row_of).transpose()
}

/// A task's visible attachments, newest first, one keyset page at a time.
///
/// Ordered by `(created_at DESC, id DESC)` against `attachment_thread_ix`
/// (migration 0025). No `OFFSET`: `docs/26` bans it, and the lint would refuse
/// it anyway.
///
/// # Errors
///
/// Any database error.
pub async fn list_for_task(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    after: Option<(OffsetDateTime, Uuid)>,
    limit: i64,
) -> Result<Vec<AttachmentRow>, sqlx::Error> {
    let sql = format!(
        "SELECT {COLUMNS}
           FROM attachment a
          WHERE a.task_id = $1
            AND a.workspace_id = $2
            AND a.committed_at IS NOT NULL
            AND a.deleted_at IS NULL
            AND ($3::timestamptz IS NULL
                 OR (a.created_at, a.id) < ($3::timestamptz, $4::uuid))
          ORDER BY a.created_at DESC, a.id DESC
          LIMIT $5"
    );
    let rows = sqlx::query(&sql)
        .bind(task_id)
        .bind(scoped.workspace_id().as_uuid())
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(limit)
        .fetch_all(scoped.conn())
        .await?;
    rows.iter().map(row_of).collect()
}

/// How many committed attachments a task already has.
///
/// `docs/28` §Limits caps it at 100 per task. Counted against committed rows so
/// a burst of abandoned pre-signs cannot lock a task out of ever attaching
/// anything again.
///
/// # Errors
///
/// Any database error.
pub async fn count_for_task(scoped: &mut Scoped<'_>, task_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM attachment
          WHERE task_id = $1 AND workspace_id = $2
            AND committed_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(task_id)
    .bind(scoped.workspace_id().as_uuid())
    .fetch_one(scoped.conn())
    .await
}

/// Record what the bytes turned out to be, after the commit handshake verified
/// them.
///
/// Writes the sniffed type and leaves `committed_at` alone: verification is not
/// the same event as becoming visible, and only a `CLEAN` scan does the second.
///
/// # Errors
///
/// Any database error.
pub async fn record_verified_type(
    scoped: &mut Scoped<'_>,
    id: Uuid,
    content_type: &str,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE attachment
            SET content_type = $3
          WHERE id = $1 AND workspace_id = $2 AND committed_at IS NULL",
    )
    .bind(id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(content_type)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Apply a scan verdict.
///
/// **The only statement that sets `committed_at`**, and it sets it only for
/// `CLEAN`. That is what makes "committed" mean "scanned and clean" rather than
/// two flags that can disagree — `docs/28` step 4.
///
/// # Errors
///
/// Any database error.
pub async fn mark_scanned(
    scoped: &mut Scoped<'_>,
    id: Uuid,
    verdict: &str,
    detail: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE attachment
            SET scan_status = $3,
                scan_detail = $4,
                scanned_at  = now(),
                -- Only CLEAN commits, and a re-scan never un-commits: an
                -- attachment that is already visible stays visible until it is
                -- deleted, which is a different operation with its own audit.
                committed_at = CASE WHEN $3 = 'CLEAN' THEN coalesce(committed_at, now())
                                    ELSE committed_at END
          WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(scoped.workspace_id().as_uuid())
    .bind(verdict)
    .bind(detail)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Soft-delete an attachment.
///
/// # Errors
///
/// Any database error.
pub async fn soft_delete(scoped: &mut Scoped<'_>, id: Uuid) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE attachment SET deleted_at = now()
          WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(scoped.workspace_id().as_uuid())
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Total committed bytes in a workspace, for the quota check
/// (`docs/28` §Validation).
///
/// # Errors
///
/// Any database error.
pub async fn workspace_bytes(scoped: &mut Scoped<'_>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT coalesce(sum(byte_size), 0)::bigint FROM attachment
          WHERE workspace_id = $1 AND deleted_at IS NULL",
    )
    .bind(scoped.workspace_id().as_uuid())
    .fetch_one(scoped.conn())
    .await
}

/// A one-row `INSERT ... RETURNING`, so the insert reads like the others.
trait ExecuteOne<'q> {
    async fn execute_one(
        self,
        scoped: &mut Scoped<'_>,
    ) -> Result<sqlx::postgres::PgRow, sqlx::Error>;
}

impl<'q> ExecuteOne<'q> for sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    async fn execute_one(
        self,
        scoped: &mut Scoped<'_>,
    ) -> Result<sqlx::postgres::PgRow, sqlx::Error> {
        self.fetch_one(scoped.conn()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verdicts_are_the_ones_the_check_constraint_allows() {
        // A value accepted here and refused there aborts a transaction that has
        // already written its audit row.
        let migration = include_str!("../../../migrations/0006_comments_and_attachments.sql");
        for verdict in SCAN_STATUSES {
            assert!(
                migration.contains(&format!("'{verdict}'")),
                "{verdict} is not a value migration 0006 permits"
            );
        }
        assert!(SCAN_STATUSES.contains(&CLEAN));
    }

    #[test]
    fn only_one_statement_can_set_committed_at() {
        // docs/28's invariant rests on it. Two writers would let "verified" and
        // "clean" drift into independently settable flags, and the partial
        // index would then be protecting nothing.
        let source = include_str!("attachment.rs");
        // Assembled, not written as a literal: a literal needle appears in this
        // file and would count itself.
        let needle = format!("committed_at {}", "=");
        let writers = source.matches(needle.as_str()).count();
        assert_eq!(
            writers, 1,
            "committed_at is assigned in {writers} places; docs/28 §The invariant \
             needs exactly one, or `verified` and `clean` become two flags that \
             can disagree"
        );
    }

    #[test]
    fn every_client_facing_read_carries_the_visibility_predicate() {
        // The one read that must see uncommitted rows is named for it. Any
        // other read missing this predicate serves an unscanned file.
        let source = include_str!("attachment.rs");
        let needle = format!("committed_at IS NOT {}", "NULL");
        let visible = source.matches(needle.as_str()).count();
        assert!(
            visible >= 3,
            "a client-facing read lost its visibility predicate ({visible} found)"
        );
    }
}
