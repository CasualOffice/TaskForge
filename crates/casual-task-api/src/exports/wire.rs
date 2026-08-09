//! The request and response shapes for exports (`docs/05` conventions).
//!
//! # The failure this file exists to prevent
//!
//! A response that tells a client an export is "running" and nothing else. An
//! export is the one operation in this product a user waits on, and a status
//! with no progress is indistinguishable from a stuck job — which is how one
//! user becomes four concurrent exports of the same filter.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `POST /api/v1/exports`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    /// The list endpoint's own query string, e.g. `state=OPEN&assignee=@me`.
    ///
    /// A string rather than a nested object: `docs/38` wants "this view, as a
    /// file", and the view is already addressable as a query. A second encoding
    /// of the same grammar would be a second thing to keep in step with it.
    pub filter: String,
    /// `csv` | `jsonl`.
    pub format: String,
    /// Column names from the closed set. Absent means all of them.
    #[serde(default)]
    pub columns: Option<Vec<String>>,
}

/// What `POST` returns, and what `GET /exports/{id}` reports.
#[derive(Debug, Serialize)]
pub struct ExportView {
    pub id: Uuid,
    /// `queued` | `running` | `succeeded` | `failed` | `expired`.
    pub status: String,
    pub format: String,
    /// Rows written so far — the progress a waiting user needs.
    pub row_count: i64,
    /// Present once the artefact exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_size: Option<i64>,
    /// Why it failed, in words a requester can act on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// When the artefact is deleted (`docs/38`: after 7 days).
    pub expires_at: String,
    /// Whether `GET /exports/{id}/download` will currently succeed. Saves a
    /// client encoding the status rules itself and getting them subtly wrong.
    pub downloadable: bool,
}

impl ExportView {
    /// Render a job row.
    #[must_use]
    pub fn of(job: &casual_task_persistence::export::JobRow) -> Self {
        Self {
            id: job.id,
            status: job.status.clone(),
            format: job.format.clone(),
            row_count: job.row_count,
            byte_size: job.byte_size,
            failure_reason: job.failure_reason.clone(),
            expires_at: crate::wire::timestamp(job.expires_at),
            downloadable: job.status == "succeeded" && job.object_key.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(status: &str, key: Option<&str>) -> casual_task_persistence::export::JobRow {
        casual_task_persistence::export::JobRow {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            requested_by: Uuid::now_v7(),
            filter_query: "state=OPEN".to_owned(),
            format: "csv".to_owned(),
            columns: None,
            status: status.to_owned(),
            row_count: 12,
            object_key: key.map(ToOwned::to_owned),
            byte_size: Some(400),
            failure_reason: None,
            expires_at: time::OffsetDateTime::now_utc(),
        }
    }

    #[test]
    fn only_a_finished_export_with_an_artefact_is_downloadable() {
        // A client that tried to download a running export would get a refusal
        // it could have predicted; one that tried a succeeded-but-keyless job
        // would get a 404 from object storage, which reads as data loss.
        assert!(ExportView::of(&job("succeeded", Some("k"))).downloadable);
        assert!(!ExportView::of(&job("running", None)).downloadable);
        assert!(!ExportView::of(&job("failed", None)).downloadable);
        assert!(
            !ExportView::of(&job("succeeded", None)).downloadable,
            "a succeeded job with no artefact must not advertise a download"
        );
        assert!(!ExportView::of(&job("expired", None)).downloadable);
    }

    #[test]
    fn a_running_export_reports_its_progress() {
        // "running" with no number is indistinguishable from stuck.
        let view = ExportView::of(&job("running", None));
        assert_eq!(view.row_count, 12);
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        // docs/05 §Conventions. A client that misspells `format` must learn so,
        // not receive a default file.
        let refused: Result<CreateRequest, _> =
            serde_json::from_str(r#"{"filter":"","format":"csv","fromat":"jsonl"}"#);
        assert!(refused.is_err());
    }
}
