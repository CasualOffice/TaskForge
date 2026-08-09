//! Export jobs: the row, its claim, and its audit record (C-021, `docs/38`).
//!
//! # The failure this module exists to prevent
//!
//! An export nobody can account for. `docs/38`: "Every export writes an
//! `audit_event` with the filter, row count, and format. Bulk data leaving the
//! system is exactly what an audit trail is for."
//!
//! That sentence is why [`record_audit`] takes the row count rather than being
//! called at the start: an audit record written when the job was *queued* says
//! what someone asked for, and the question an investigator has is what they
//! got.
//!
//! # Two connection kinds, on purpose
//!
//! The API writes and reads jobs through [`Scoped`] — the tenant's own
//! transaction, subject to the policy in migration 0026. The worker claims
//! across tenants through [`Dispatcher`], exactly as it does for
//! `outbox_delivery`, because a background runner cannot know the set of
//! workspace ids in advance.
//!
//! The two are different types and neither can be used for the other's job,
//! which is the same guarantee `dispatch` relies on.

use uuid::Uuid;

use crate::dispatch::Dispatcher;
use crate::scoped::Scoped;

/// What a client asked for, once it has been validated at the edge.
#[derive(Debug, Clone)]
pub struct NewJob {
    pub id: Uuid,
    pub requested_by: Uuid,
    /// The list endpoint's own query string — see the table comment in
    /// migration 0026.
    pub filter_query: String,
    pub format: String,
    /// `None` means the default column set.
    pub columns: Option<serde_json::Value>,
}

/// An export job as the API reports it.
#[derive(Debug, Clone)]
pub struct JobRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub requested_by: Uuid,
    pub filter_query: String,
    pub format: String,
    pub columns: Option<serde_json::Value>,
    pub status: String,
    pub row_count: i64,
    pub object_key: Option<String>,
    pub byte_size: Option<i64>,
    pub failure_reason: Option<String>,
    pub expires_at: time::OffsetDateTime,
}

type JobTuple = (
    Uuid,
    Uuid,
    Uuid,
    String,
    String,
    Option<serde_json::Value>,
    String,
    i64,
    Option<String>,
    Option<i64>,
    Option<String>,
    time::OffsetDateTime,
);

fn row_of(t: JobTuple) -> JobRow {
    JobRow {
        id: t.0,
        workspace_id: t.1,
        requested_by: t.2,
        filter_query: t.3,
        format: t.4,
        columns: t.5,
        status: t.6,
        row_count: t.7,
        object_key: t.8,
        byte_size: t.9,
        failure_reason: t.10,
        expires_at: t.11,
    }
}

const COLUMNS: &str = "id, workspace_id, requested_by, filter_query, format, columns, \
                       status, row_count, object_key, byte_size, failure_reason, expires_at";

/// Queue a job in the caller's workspace.
///
/// # Errors
///
/// Any database error.
pub async fn insert(scoped: &mut Scoped<'_>, new: &NewJob) -> Result<JobRow, sqlx::Error> {
    let sql = format!(
        "INSERT INTO export_job (id, workspace_id, requested_by, filter_query, format, columns)
         VALUES ($1,$2,$3,$4,$5,$6)
         RETURNING {COLUMNS}"
    );
    // `workspace_id` from the scope, never from an argument: `Scoped` is the
    // only thing that knows which tenant this transaction is for, so the row
    // written and the policy enforced cannot disagree.
    let row: JobTuple = sqlx::query_as(&sql)
        .bind(new.id)
        .bind(scoped.workspace_id().as_uuid())
        .bind(new.requested_by)
        .bind(&new.filter_query)
        .bind(&new.format)
        .bind(&new.columns)
        .fetch_one(scoped.conn())
        .await?;
    Ok(row_of(row))
}

/// Read one job the caller's workspace owns.
///
/// # Errors
///
/// Any database error.
pub async fn read(scoped: &mut Scoped<'_>, id: Uuid) -> Result<Option<JobRow>, sqlx::Error> {
    let sql = format!("SELECT {COLUMNS} FROM export_job WHERE id = $1");
    let row: Option<JobTuple> = sqlx::query_as(&sql)
        .bind(id)
        .fetch_optional(scoped.conn())
        .await?;
    Ok(row.map(row_of))
}

/// Take the oldest queued job, across every tenant.
///
/// `FOR UPDATE SKIP LOCKED` for the same reason `dispatch::claim` uses it: two
/// workers must not run the same export twice, and the second one must not
/// block waiting to find that out.
///
/// # Errors
///
/// Any database error.
pub async fn claim_next(
    dispatcher: &mut Dispatcher<'_>,
    worker: &str,
) -> Result<Option<JobRow>, sqlx::Error> {
    let sql = format!(
        "UPDATE export_job j
            SET status = 'running', claimed_at = now(), claimed_by = $1,
                started_at = COALESCE(j.started_at, now())
          WHERE j.id = (SELECT c.id FROM export_job c
                         WHERE c.status = 'queued'
                         ORDER BY c.created_at
                         LIMIT 1
                           FOR UPDATE SKIP LOCKED)
      RETURNING {COLUMNS}"
    );
    let row: Option<JobTuple> = sqlx::query_as(&sql)
        .bind(worker)
        .fetch_optional(dispatcher.conn())
        .await?;
    Ok(row.map(row_of))
}

/// Record progress after a batch.
///
/// Per batch rather than at the end so `GET /exports/{id}` can answer "how far"
/// instead of "still running", which is the difference between a user waiting
/// and a user starting a second export.
///
/// # Errors
///
/// Any database error.
pub async fn record_progress(
    dispatcher: &mut Dispatcher<'_>,
    id: Uuid,
    rows: i64,
    object_key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE export_job SET row_count = $2, object_key = $3 WHERE id = $1")
        .bind(id)
        .bind(rows)
        .bind(object_key)
        .execute(dispatcher.conn())
        .await?;
    Ok(())
}

/// Mark a job finished.
///
/// # Errors
///
/// Any database error.
pub async fn succeed(
    dispatcher: &mut Dispatcher<'_>,
    id: Uuid,
    rows: i64,
    bytes: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE export_job
            SET status = 'succeeded', row_count = $2, byte_size = $3,
                completed_at = now(), claimed_at = NULL, claimed_by = NULL
          WHERE id = $1",
    )
    .bind(id)
    .bind(rows)
    .bind(bytes)
    .execute(dispatcher.conn())
    .await?;
    Ok(())
}

/// Mark a job failed, with a reason a requester can act on.
///
/// # Errors
///
/// Any database error.
pub async fn fail(
    dispatcher: &mut Dispatcher<'_>,
    id: Uuid,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE export_job
            SET status = 'failed', failure_reason = $2, completed_at = now(),
                claimed_at = NULL, claimed_by = NULL
          WHERE id = $1",
    )
    .bind(id)
    .bind(reason)
    .execute(dispatcher.conn())
    .await?;
    Ok(())
}

/// Write the `audit_event` `docs/38` requires for every export.
///
/// Takes a [`Dispatcher`] because the worker writes it, and takes the workspace
/// explicitly for the same reason: this connection is not scoped to a tenant, so
/// the tenant must be named rather than assumed.
///
/// # Errors
///
/// Any database error.
pub async fn record_audit(
    dispatcher: &mut Dispatcher<'_>,
    workspace_id: Uuid,
    actor_id: Uuid,
    job_id: Uuid,
    format: &str,
    rows: i64,
    filter_query: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_event
             (id, workspace_id, event_type, actor_id, actor_type,
              target_type, target_id, changes)
         VALUES ($1,$2,'export.completed',$3,'USER','export',$4,$5)",
    )
    .bind(Uuid::now_v7())
    .bind(workspace_id)
    .bind(actor_id)
    .bind(job_id)
    // The filter, the format and the row count together: docs/38 names all
    // three, and any two of them leave an investigator guessing at the third.
    .bind(serde_json::json!({
        "format": format,
        "row_count": rows,
        "filter": filter_query,
    }))
    .execute(dispatcher.conn())
    .await?;
    Ok(())
}

/// Artefacts whose retention has elapsed (`docs/38`: deleted after 7 days).
///
/// Returns the keys to remove; the caller deletes them from object storage and
/// then calls [`mark_expired`]. Two steps because the object store is not
/// transactional with the database, and the safe order is "forget the bytes,
/// then forget the row" — the reverse leaves an artefact nothing points at.
///
/// # Errors
///
/// Any database error.
pub async fn expired_artefacts(
    dispatcher: &mut Dispatcher<'_>,
    limit: i64,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, object_key FROM export_job
          WHERE object_key IS NOT NULL
            AND status <> 'expired'
            AND expires_at < now()
          ORDER BY expires_at
          LIMIT $1",
    )
    .bind(limit)
    .fetch_all(dispatcher.conn())
    .await
}

/// Record that an artefact is gone.
///
/// # Errors
///
/// Any database error.
pub async fn mark_expired(dispatcher: &mut Dispatcher<'_>, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE export_job SET status = 'expired', object_key = NULL WHERE id = $1")
        .bind(id)
        .execute(dispatcher.conn())
        .await?;
    Ok(())
}

/// Where an artefact lives.
///
/// `{workspace}/exports/{job}.{ext}` — the workspace first, so the tree is
/// partitioned by tenant exactly as attachments are (`docs/28`) and a directory
/// listing never crosses one.
#[must_use]
pub fn object_key(workspace: Uuid, job: Uuid, extension: &str) -> String {
    format!("{workspace}/exports/{job}.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_object_key_is_partitioned_by_tenant() {
        let workspace = Uuid::now_v7();
        let job = Uuid::now_v7();
        let key = object_key(workspace, job, "csv");
        assert!(
            key.starts_with(&workspace.to_string()),
            "the key must lead with the workspace, or a listing crosses tenants: {key}"
        );
        assert!(key.ends_with(".csv"));
    }

    #[test]
    fn an_object_key_contains_nothing_a_client_chose() {
        // Both components are server-minted UUIDs. A key built from a filename
        // or a title would be a traversal vector aimed at the object store.
        let key = object_key(Uuid::now_v7(), Uuid::now_v7(), "jsonl");
        assert!(!key.contains(".."), "{key}");
        assert_eq!(key.matches('/').count(), 2, "{key}");
    }
}
