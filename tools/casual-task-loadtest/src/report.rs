//! The report format, and the split between fields that are *compared* and
//! fields that are merely *recorded*.
//!
//! # Which fields must be stable
//!
//! A baseline is a report that was committed. The gate reads a committed file
//! written weeks or months ago and asks one question of it: "is this run's p95
//! more than 10% worse?" That question is only meaningful when the two runs are
//! comparable, so the format separates three kinds of field:
//!
//! **Identity — must match exactly, or the comparison is refused.**
//! `schemaVersion`, `environment`, `corpus.scale`. Two runs on different
//! hardware, different corpus sizes, or different report semantics produce
//! numbers that look comparable and are not. Refusing is the whole point;
//! `docs/30` §Reference environment says measurements from another environment
//! "are recorded with the environment attached and are not comparable".
//!
//! **Comparability — must be at least as strong as the baseline.**
//! `iterations` and `warmupIterations`. More samples never makes a p95 worse-
//! founded, so a run may exceed the baseline's counts but not fall below them.
//! A 50-iteration p95 compared against a 5,000-iteration p95 is noise wearing a
//! gate's clothes.
//!
//! **Provenance — recorded, never compared.** `generatedAt`, `corpus` counts,
//! `notMeasured`, `notes`, and every measured number other than the p95 the
//! gate reads. These describe the run; they do not decide it.
//!
//! # Why `generatedAt` is an argument and not `SystemTime::now()`
//!
//! Report identity must not depend on when the binary happened to run. Taking
//! the timestamp as a required argument buys three things a clock call does
//! not:
//!
//! 1. **A rerun of the same measurement is byte-identical.** That makes the
//!    serializer itself testable, and makes a diff of two committed baselines
//!    show only what actually moved.
//! 2. **CI can stamp the commit timestamp**, so a baseline's date describes the
//!    change it belongs to rather than the minute the runner picked up the job.
//! 3. **A clock call in report identity is a hidden input.** Everything else in
//!    the report is derived from arguments and from the database; leaving one
//!    ambient source of variation in is how "why did this file change?" starts.
//!
//! The cost is stated plainly: a caller can pass a wrong or duplicated
//! timestamp, and nothing here can detect that. The field is validated as
//! RFC 3339 and otherwise trusted.

use serde::{Deserialize, Serialize};

/// Bumped whenever the meaning of any measured number changes — the percentile
/// definition, what is included in a sample, or the identity rules above. The
/// gate refuses to compare across versions, so a bump forces every baseline to
/// be re-measured deliberately rather than reinterpreted silently.
pub const SCHEMA_VERSION: u32 = 1;

/// Name of the percentile definition, recorded so a reader of a committed
/// baseline never has to guess. See [`crate::stats`].
pub const PERCENTILE_METHOD: &str = "nearest-rank";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub harness: String,
    pub harness_version: String,
    /// Named machine profile from `docs/30` §Reference environment.
    pub environment: String,
    /// Provenance only. Supplied by the caller — see the module docs.
    pub generated_at: String,
    pub corpus: Corpus,
    pub iterations: u32,
    pub warmup_iterations: u32,
    pub percentile_method: String,
    pub cases: Vec<CaseResult>,
    /// Operations from the `docs/30` latency table this run did not cover, each
    /// with the reason. Present in every report so a reader cannot mistake the
    /// covered set for the whole table.
    pub not_measured: Vec<NotMeasured>,
    /// Free-text caveats attached by the run (low sample counts, corpus
    /// shortfalls). Never compared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Present only on a committed baseline file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<BaselineMeta>,
}

/// The corpus a run executed against. `scale` is an identity field; the counts
/// are provenance, recorded so a reader can see whether the corpus drifted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Corpus {
    /// Label such as `reference` or `reduced`. `docs/30` §Measurement: full
    /// runs are nightly at reference scale, PR runs use a reduced corpus.
    pub scale: String,
    pub workspace_id: String,
    pub tasks: i64,
    pub projects: i64,
    pub users: i64,
    pub search_documents: i64,
    pub activity_events: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseResult {
    pub id: String,
    /// The row of the `docs/30` §Server-side latency targets table this case
    /// contributes to, or `None` for a case that measures the harness itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub samples: u32,
    pub min_us: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
    pub mean_us: u64,
    /// `false` when `samples` is below [`crate::stats::P99_CONFIDENCE_MIN_SAMPLES`].
    /// The p99 is still reported — suppressing it would hide data — but a
    /// reader is told not to draw a conclusion from it.
    pub p99_confident: bool,
    /// Rows the query returned, probed **once before warm-up** and copied into
    /// every case.
    ///
    /// A case that suddenly returns zero rows gets faster and would otherwise
    /// read as an improvement; [`crate::compare`] blocks on that rather than
    /// leaving it to review.
    ///
    /// The limitation, since the field is the signal for exactly this: a single
    /// probe cannot see a result set that empties *partway through* a run. It
    /// compares corpora, not iterations. Catching mid-run drift would mean
    /// re-probing after the last measured round, which is worth doing if a case
    /// is ever observed to drift.
    pub rows_returned: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotMeasured {
    /// The `docs/30` operation name, verbatim.
    pub operation: String,
    pub reason: String,
    /// Tracker phase in which it becomes measurable.
    pub arrives: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BaselineStatus {
    /// Shape-only. Carries no measured number and can never satisfy the gate.
    Placeholder,
    /// Measured on the named environment at the named corpus scale.
    Measured,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaselineMeta {
    pub status: BaselineStatus,
    /// Why this baseline was recorded or moved. `docs/30` §Measurement makes
    /// updating a baseline a reviewed act; this is where the review reads the
    /// argument. Empty on a placeholder.
    pub justification: String,
    /// Tracker item or PR that recorded it.
    pub recorded_by: String,
}

impl Report {
    /// Serialize with a trailing newline, the form committed to `benchmarks/`.
    /// Field order follows declaration order, so re-running the same
    /// measurement produces a byte-identical file.
    pub fn to_json(&self) -> anyhow::Result<String> {
        let mut s = serde_json::to_string_pretty(self)?;
        s.push('\n');
        Ok(s)
    }

    pub fn case(&self, id: &str) -> Option<&CaseResult> {
        self.cases.iter().find(|c| c.id == id)
    }
}

#[cfg(test)]
pub(crate) fn fixture(environment: &str, scale: &str, p95: &[(&str, u64)]) -> Report {
    Report {
        schema_version: SCHEMA_VERSION,
        harness: "casual-task-loadtest".into(),
        harness_version: "0.0.0".into(),
        environment: environment.into(),
        generated_at: "2026-08-08T00:00:00Z".into(),
        corpus: Corpus {
            scale: scale.into(),
            workspace_id: "11111111-1111-7111-8111-111111111111".into(),
            tasks: 10,
            projects: 1,
            users: 1,
            search_documents: 10,
            activity_events: 10,
        },
        iterations: 1_000,
        warmup_iterations: 100,
        percentile_method: PERCENTILE_METHOD.into(),
        cases: p95
            .iter()
            .map(|(id, v)| CaseResult {
                id: (*id).into(),
                target: None,
                samples: 1_000,
                min_us: *v,
                p50_us: *v,
                p95_us: *v,
                p99_us: *v,
                max_us: *v,
                mean_us: *v,
                p99_confident: true,
                rows_returned: 1,
            })
            .collect(),
        not_measured: Vec::new(),
        notes: Vec::new(),
        baseline: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let r = fixture("reference", "reduced", &[("task_read_by_id", 400)]);
        let json = r.to_json().expect("serialize");
        let back: Report = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(r, back);
    }

    #[test]
    fn serialization_is_stable_for_the_same_measurement() {
        // The property that makes `generatedAt` an argument worth having: two
        // serializations of the same data are byte-identical, so a diff of two
        // committed baselines shows only what moved.
        let a = fixture("reference", "reduced", &[("task_read_by_id", 400)]);
        let b = fixture("reference", "reduced", &[("task_read_by_id", 400)]);
        assert_eq!(a.to_json().expect("a"), b.to_json().expect("b"));
    }

    #[test]
    fn report_ends_with_exactly_one_newline() {
        let json = fixture("reference", "reduced", &[])
            .to_json()
            .expect("json");
        assert!(json.ends_with("}\n"));
        assert!(!json.ends_with("\n\n"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        // A baseline with a typo'd key must fail loudly rather than silently
        // defaulting a field the gate then reads.
        let mut json: serde_json::Value = serde_json::from_str(
            &fixture("reference", "reduced", &[])
                .to_json()
                .expect("json"),
        )
        .expect("value");
        json["iteratoins"] = serde_json::json!(5);
        let err = serde_json::from_value::<Report>(json).expect_err("must reject");
        assert!(err.to_string().contains("iteratoins"), "{err}");
    }

    #[test]
    fn placeholder_status_serializes_as_kebab_case() {
        let json = serde_json::to_string(&BaselineStatus::Placeholder).expect("json");
        assert_eq!(json, "\"placeholder\"");
    }
}
