//! Claim a job, stream it to object storage, audit it (`docs/38` §Export).
//!
//! # The failure this file exists to prevent
//!
//! An export that keeps emitting rows from a project the actor lost access to
//! halfway through.
//!
//! `docs/38` is explicit: "Permissions are evaluated **per batch, not once at
//! the start**. A long export must not keep emitting rows from a project the
//! actor lost access to halfway through." Every other read path in this product
//! resolves authority once, and every other read path is over in milliseconds.
//! An export runs for minutes; a grant revoked at minute two must stop it at
//! minute two.
//!
//! So the job runner re-resolves the actor's accessible project set **before every
//! page** and recompiles the filter with it. The compiled SQL is discarded
//! between batches on purpose — reusing it would be reusing the permission
//! predicate baked into it, which is precisely the bug.
//!
//! **The cost, stated:** one extra authority resolution per batch — a teams
//! read, a grants read and a visible-projects read, roughly three indexed
//! queries per 500 rows. At the 200,000-row ceiling that is 1,200 extra queries
//! for one export. That is the price of the guarantee, and it is cheaper than
//! the alternative, which is not having it.
//!
//! # Every bound names its overflow policy (`docs/24` §D-040)
//!
//! | Bound | Value | When it is reached |
//! | --- | --- | --- |
//! | Rows held in memory | [`BATCH`] | Never exceeded: a batch is serialised and appended to object storage before the next is read. The process never holds the result set — `docs/38`'s "the API process never holds the result set in memory", applied to the worker too. |
//! | Rows per export | [`MAX_ROWS`] | The job **fails** with a reason naming the limit, and the partial artefact is left for the sweeper. Truncating silently would hand someone a file that looks complete; succeeding without a bound would let one filter export the whole tenant. |
//! | Jobs in flight per worker | one | The loop runs a job to completion before claiming another. An export is I/O-bound on the database, and running several concurrently would multiply exactly the load the batching exists to bound. |

use std::sync::Arc;
use std::time::Duration;

use casual_task_infra::ObjectStore;
use casual_task_model::{ActorType, ProjectId, WorkspaceId, WorkspaceScope};
use casual_task_persistence::compile::{AuthorizedProjectSet, Page, compile};
use casual_task_persistence::dispatch::DispatcherRole;
use casual_task_persistence::{Scoped, authz, export as store, project, task};
use serde_json::{Map, Value};
use sqlx::PgPool;

use super::{Column, Format, csv, jsonl};
use crate::dispatcher::Cancel;

/// Rows read, serialised and flushed at a time.
///
/// 500 keeps a batch's serialised form comfortably under a megabyte for typical
/// task rows, so the memory ceiling of an export is a fixed cost rather than a
/// function of the result set.
pub const BATCH: u32 = 500;

/// The most rows one export may contain.
///
/// `docs/30` sizes a workspace at 2,000,000 tasks; an unbounded export is
/// therefore an unbounded file and an unbounded read. 200,000 is generous for
/// the spreadsheet this format exists to feed and small enough to stay a
/// bounded amount of work.
pub const MAX_ROWS: u64 = 200_000;

/// How long the loop sleeps when there is no work.
pub const IDLE: Duration = Duration::from_secs(2);

/// Why a job stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The filter no longer compiles, or never did.
    BadFilter(String),
    /// [`MAX_ROWS`] was reached.
    TooManyRows,
    /// Object storage refused a write.
    Storage(String),
}

impl Failure {
    /// What the requester is told. Never a database error verbatim: a failure
    /// reason is shown to a user, and an internal error string is
    /// reconnaissance.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::BadFilter(_) => "The filter is not valid for export".to_owned(),
            Self::TooManyRows => {
                format!("The result set exceeds the {MAX_ROWS} row export limit; narrow the filter")
            }
            Self::Storage(_) => "The export artefact could not be written".to_owned(),
        }
    }
}

/// Run export jobs until cancelled.
///
/// # Errors
///
/// A database error that is not recoverable by retrying the claim. A job that
/// *fails* is not an error here — it is recorded against the job and the loop
/// continues, which is what keeps one bad filter from stopping every export.
pub async fn run(
    pool: &PgPool,
    storage: Arc<dyn ObjectStore>,
    worker_id: &str,
    mut cancel: Cancel,
) -> Result<(), sqlx::Error> {
    // Verified once, at startup, exactly as the dispatch loop does: the claim
    // reads across tenants and a role that cannot bypass RLS would claim
    // nothing, forever, without erroring.
    let role = {
        let mut conn = pool.acquire().await?;
        DispatcherRole::verify(&mut conn).await?
    };

    while !cancel.is_cancelled() {
        let mut tx = pool.begin().await?;
        let mut dispatcher = role.dispatcher(&mut tx);
        let claimed = store::claim_next(&mut dispatcher, worker_id).await?;
        tx.commit().await?;

        let Some(job) = claimed else {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(IDLE) => continue,
            }
        };

        tracing::info!(job = %job.id, format = %job.format, "running an export");
        let outcome = run_job(pool, storage.as_ref(), &job).await;

        let mut tx = pool.begin().await?;
        let mut dispatcher = role.dispatcher(&mut tx);
        match outcome {
            Ok(done) => {
                store::succeed(&mut dispatcher, job.id, done.rows, done.bytes).await?;
                // docs/38: every export writes an audit_event, with the filter,
                // the row count and the format. Written in the same transaction
                // as the completion, so an export cannot be recorded as done
                // and be missing from the audit trail.
                store::record_audit(
                    &mut dispatcher,
                    job.workspace_id,
                    job.requested_by,
                    job.id,
                    &job.format,
                    done.rows,
                    &job.filter_query,
                )
                .await?;
            }
            Err(failure) => {
                tracing::warn!(job = %job.id, ?failure, "an export failed");
                store::fail(&mut dispatcher, job.id, &failure.reason()).await?;
            }
        }
        tx.commit().await?;
    }
    Ok(())
}

/// What a completed job produced.
#[derive(Debug, Clone, Copy)]
pub struct Done {
    pub rows: i64,
    pub bytes: i64,
}

/// Stream one job's rows to object storage.
async fn run_job(
    pool: &PgPool,
    storage: &dyn ObjectStore,
    job: &store::JobRow,
) -> Result<Done, Failure> {
    let format = Format::parse(&job.format)
        .ok_or_else(|| Failure::BadFilter(format!("unknown format {}", job.format)))?;
    let columns = columns_of(job);
    // The stored query string, through the SAME pipeline the list endpoint
    // uses: parse, resolve, validate, compile. Not a second parser and not a
    // stored AST — one grammar, one implementation of it.
    //
    // `@me` resolves to the person who ASKED, not to the worker: the resolve
    // context is built from `requested_by`. An export that resolved symbolic
    // clauses against whoever happened to run it would return a different set
    // than the view it was taken from.
    let pairs = query_pairs(&job.filter_query);
    let query = casual_task_search::parse_url(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .map_err(|error| Failure::BadFilter(format!("{error:?}")))?;

    let key = store::object_key(job.workspace_id, job.id, format.extension());
    let workspace = WorkspaceId::from_uuid(job.workspace_id);

    let requester = casual_task_model::UserId::from_uuid(job.requested_by);
    let mut written: u64 = 0;
    let mut bytes: u64 = 0;
    let mut after = None;

    // The header, before any row, so a zero-row export is still a valid file
    // with columns rather than an empty one a spreadsheet refuses to open.
    if matches!(format, Format::Csv) {
        let mut head = csv::BOM.to_vec();
        head.extend_from_slice(
            csv::row(
                &columns
                    .iter()
                    .map(|c| c.as_str().to_owned())
                    .collect::<Vec<_>>(),
            )
            .as_bytes(),
        );
        bytes += head.len() as u64;
        storage
            .append(&key, &head)
            .await
            .map_err(|error| Failure::Storage(error.to_string()))?;
    }

    loop {
        // A FRESH transaction and a FRESH authority resolution per batch. This
        // is the requirement from docs/38, and it is why the compiled filter is
        // built inside the loop rather than hoisted out of it.
        let mut tx = pool.begin().await.map_err(db)?;
        let scope = WorkspaceScope::for_job(workspace);
        let mut scoped = Scoped::apply(&mut tx, &scope).await.map_err(db)?;

        let viewer = authz::viewer_for(&mut scoped, job.requested_by, actor_principal_type())
            .await
            .map_err(db)?;
        // Resolved per batch alongside the permission set: a team the actor was
        // removed from must stop matching `@my_teams` at the same moment it
        // stops conferring visibility.
        let resolve_ctx = casual_task_search::resolve::Context::new(
            requester,
            viewer
                .teams
                .iter()
                .copied()
                .map(casual_task_model::TeamId::from_uuid)
                .collect(),
            time::OffsetDateTime::now_utc(),
            time::UtcOffset::UTC,
        );
        let filter = casual_task_search::resolve(&query.filter, &resolve_ctx)
            .map_err(|error| Failure::BadFilter(format!("{error:?}")))?;
        casual_task_search::validate(&filter)
            .map_err(|error| Failure::BadFilter(format!("{error:?}")))?;
        let accessible = project::accessible(&mut scoped, &viewer, MAX_ACCESSIBLE_PROJECTS)
            .await
            .map_err(db)?;
        let visible: Vec<ProjectId> = accessible
            .into_iter()
            .map(|(id, _)| ProjectId::from_uuid(id))
            .collect();

        let page = Page {
            sort: casual_task_search::Sort::default(),
            after,
            limit: BATCH,
        };
        let compiled = compile(
            &filter,
            workspace,
            &AuthorizedProjectSet::resolved(visible),
            &page,
        );
        let rows = task::list(&mut scoped, &compiled).await.map_err(db)?;
        tx.commit().await.map_err(db)?;

        if rows.is_empty() {
            break;
        }

        // Keyset, never OFFSET: the cursor is the last row of this batch, so
        // page N+1 costs what page 1 did. An OFFSET export re-reads everything
        // it has already emitted, once per batch.
        after = rows.last().map(cursor_of);

        let page_rows = rows.len() as u64;
        if written + page_rows > MAX_ROWS {
            return Err(Failure::TooManyRows);
        }

        let chunk = serialise(format, &columns, &rows);
        bytes += chunk.len() as u64;
        storage
            .append(&key, chunk.as_bytes())
            .await
            .map_err(|error| Failure::Storage(error.to_string()))?;
        written += page_rows;

        if page_rows < u64::from(BATCH) {
            break;
        }
    }

    Ok(Done {
        rows: i64::try_from(written).unwrap_or(i64::MAX),
        bytes: i64::try_from(bytes).unwrap_or(i64::MAX),
    })
}

/// The stored query string, split into the pairs `parse_url` expects.
///
/// Percent-decoding is deliberately not done here: the string was captured from
/// the request's own query, which arrives already decoded by the router, and
/// decoding twice turns a literal `%2B` in a title filter into a `+`.
fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// `docs/26` bounds the accessible set; the same constant the list path uses.
const MAX_ACCESSIBLE_PROJECTS: u32 = 500;

/// An export is always run as the person who asked for it, never as a service
/// account: `export_job.requested_by` references `user_account`.
const fn actor_principal_type() -> &'static str {
    let _ = ActorType::User;
    "USER"
}

fn db(error: sqlx::Error) -> Failure {
    tracing::error!(%error, "an export batch failed");
    Failure::Storage(error.to_string())
}

fn columns_of(job: &store::JobRow) -> Vec<Column> {
    job.columns
        .as_ref()
        .and_then(|v| v.as_array().cloned())
        .map(|values| {
            values
                .iter()
                .filter_map(|v| v.as_str())
                .filter_map(Column::parse)
                .collect::<Vec<_>>()
        })
        .filter(|c: &Vec<Column>| !c.is_empty())
        .unwrap_or_else(|| Column::ALL.to_vec())
}

/// The keyset cursor for the next batch.
///
/// The sort is fixed at `updated_at` for exports, so the cursor key is that
/// column. `docs/26`: the cursor must carry the key of the sort actually used —
/// carrying a different one resumes against a column the query does not order
/// by, which silently repeats or skips rows and only ever after the first batch.
fn cursor_of(row: &task::TaskRow) -> casual_task_model::Cursor {
    casual_task_model::Cursor::new(vec![rfc3339(row.updated_at)], row.id)
}

/// One batch, as bytes for the artefact.
fn serialise(format: Format, columns: &[Column], rows: &[task::TaskRow]) -> String {
    let mut out = String::new();
    for row in rows {
        match format {
            Format::Csv => out.push_str(&csv::row(
                &columns.iter().map(|c| cell(*c, row)).collect::<Vec<_>>(),
            )),
            Format::Jsonl => {
                let mut record = Map::new();
                for column in columns {
                    record.insert(
                        column.as_str().to_owned(),
                        Value::String(cell(*column, row)),
                    );
                }
                if let Ok(line) = jsonl::line(&record) {
                    out.push_str(&line);
                }
            }
        }
    }
    out
}

/// One cell, as text.
///
/// Timestamps as RFC 3339 and ids as plain UUIDs: an export is read by a
/// spreadsheet and by a script, and both handle those. A localised date would
/// be ambiguous in exactly the way `docs/25` refuses for activity records.
fn cell(column: Column, row: &task::TaskRow) -> String {
    match column {
        Column::Key => row.number.to_string(),
        Column::Title => row.title.clone(),
        Column::Type => row.task_type.clone(),
        Column::Priority => row.priority.clone(),
        Column::State => row.state.clone(),
        Column::Reporter => row.reporter_id.to_string(),
        Column::DueAt => row.due_at.map(rfc3339).unwrap_or_default(),
        Column::CreatedAt => rfc3339(row.created_at),
        Column::UpdatedAt => rfc3339(row.updated_at),
    }
}

fn rfc3339(at: time::OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_row_ceiling_is_bounded_and_below_the_workspace_size() {
        // An unbounded export is an unbounded read and an unbounded file.
        // docs/30 sizes a workspace at 2,000,000 tasks.
        const { assert!(MAX_ROWS > 0) };
        const { assert!(MAX_ROWS < 2_000_000) };
    }

    #[test]
    fn a_batch_is_small_enough_to_be_a_fixed_memory_cost() {
        const { assert!(BATCH > 0 && BATCH <= 1_000) };
    }

    #[test]
    fn a_failure_reason_never_leaks_an_internal_error() {
        // The reason is shown to the requester. A database error string there
        // is reconnaissance.
        let leaked = Failure::Storage("connection refused to 10.0.0.4:5432".to_owned());
        assert!(
            !leaked.reason().contains("10.0.0.4"),
            "an internal address reached a user-visible failure reason"
        );
        assert!(
            !Failure::BadFilter("Unknown(\"t.title\")".to_owned())
                .reason()
                .contains("t.title"),
            "a column name reached a user-visible failure reason"
        );
    }

    #[test]
    fn the_row_limit_failure_tells_the_user_what_to_do() {
        let reason = Failure::TooManyRows.reason();
        assert!(reason.contains(&MAX_ROWS.to_string()), "{reason}");
        assert!(
            reason.contains("narrow"),
            "a limit with no suggested action is a dead end: {reason}"
        );
    }

    #[test]
    fn the_default_column_set_is_every_column() {
        let job = store::JobRow {
            id: uuid::Uuid::now_v7(),
            workspace_id: uuid::Uuid::now_v7(),
            requested_by: uuid::Uuid::now_v7(),
            filter_query: String::new(),
            format: "csv".to_owned(),
            columns: None,
            status: "queued".to_owned(),
            row_count: 0,
            object_key: None,
            byte_size: None,
            failure_reason: None,
            expires_at: time::OffsetDateTime::now_utc(),
        };
        assert_eq!(columns_of(&job), Column::ALL.to_vec());
    }

    #[test]
    fn an_unknown_requested_column_is_dropped_rather_than_passed_through() {
        // The closed set is the defence; this asserts the runner honours it
        // rather than trusting whatever the row holds.
        let mut job = store::JobRow {
            id: uuid::Uuid::now_v7(),
            workspace_id: uuid::Uuid::now_v7(),
            requested_by: uuid::Uuid::now_v7(),
            filter_query: String::new(),
            format: "csv".to_owned(),
            columns: Some(serde_json::json!(["title", "t.description"])),
            status: "queued".to_owned(),
            row_count: 0,
            object_key: None,
            byte_size: None,
            failure_reason: None,
            expires_at: time::OffsetDateTime::now_utc(),
        };
        assert_eq!(columns_of(&job), vec![Column::Title]);

        // ...and a request for nothing legal falls back to the default rather
        // than producing a file with no columns.
        job.columns = Some(serde_json::json!(["nonsense"]));
        assert_eq!(columns_of(&job), Column::ALL.to_vec());
    }
}
