//! Running a report (`docs/38`, ADR-027).
//!
//! # A report is not a new query path
//!
//! ADR-027: "a report is a saved filter plus an aggregation, over the same
//! closed field set as everything else". So this module executes what
//! [`crate::compile::compile_group_count`] produced and does not build SQL of
//! its own. The tenant predicate, the authorized project set and the clause
//! emitter are the list query's, unchanged — which is the only way the index
//! contract (ADR-011) survives reporting rather than being the exception that
//! breaks it.
//!
//! # Why the group key comes back as text
//!
//! The dimensions are heterogeneous: a status group is a uuid, a state group is
//! an enum, a priority group is a word. One column that holds all of them means
//! one decoder and one shape on the wire, and the caller already knows which
//! dimension it asked for. Casting in SQL rather than reading five optional
//! columns is what keeps the row type from growing a member per dimension.

use time::OffsetDateTime;

use crate::compile::{Compiled, Param};
use crate::scoped::Scoped;

/// One slice of a report.
#[derive(Debug, Clone)]
pub struct GroupRow {
    /// `None` is a real answer — unassigned, untriaged, on no environment —
    /// and not missing data.
    pub key: Option<String>,
    /// `None` unless the report asked for a time series.
    pub bucket_start: Option<OffsetDateTime>,
    pub total: i64,
}

/// Execute a grouped count the compiler produced.
///
/// # Errors
///
/// Any database error.
pub async fn run(
    scoped: &mut Scoped<'_>,
    compiled: &Compiled,
) -> Result<Vec<GroupRow>, sqlx::Error> {
    let mut query = sqlx::query_as(&compiled.sql);
    for param in &compiled.params {
        query = match param {
            Param::Workspace(w) => query.bind(w.as_uuid()),
            Param::Projects(ps) => query.bind(ps.iter().map(|p| p.as_uuid()).collect::<Vec<_>>()),
            Param::Text(t) => query.bind(t.clone()),
            Param::TextList(v) => query.bind(v.clone()),
        };
    }
    let rows: Vec<(Option<String>, Option<OffsetDateTime>, i64)> =
        query.fetch_all(scoped.conn()).await?;
    Ok(rows
        .into_iter()
        .map(|row| GroupRow {
            key: row.0,
            bucket_start: row.1,
            total: row.2,
        })
        .collect())
}
