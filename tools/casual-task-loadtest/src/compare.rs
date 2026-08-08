//! `compare` — the gate.
//!
//! `docs/30` §Measurement:
//!
//! > CI fails on a **>10% regression** against the committed baseline, not on
//! > absolute numbers — absolute thresholds fail on CI noise and get disabled,
//! > which is worse than no gate.
//!
//! Two consequences shape this module.
//!
//! **The gate is relative, and only relative.** No target from the `docs/30`
//! latency table is asserted here. A run that is 9% slower than a baseline
//! passes even if it is far above the table's number, and a run that is 11%
//! slower fails even if it is far below it. That is the stated design: absolute
//! thresholds are what get disabled.
//!
//! **A comparison that cannot be trusted is refused, not fudged.** Mismatched
//! environment, corpus scale, schema version, percentile method, a baseline
//! that is still a placeholder, or a case the report dropped all exit with
//! [`crate::EXIT_NOT_COMPARABLE`] rather than [`crate::EXIT_REGRESSION`]. A
//! gate that silently passes when it could not run is the failure mode this
//! whole design is trying to avoid.
//!
//! Only **p95** decides the verdict. p50 and p99 are printed for context: p50
//! moves too little to gate on, and p99 needs sample counts that a PR-time run
//! does not have (see [`crate::stats::P99_CONFIDENCE_MIN_SAMPLES`]).

use crate::report::{BaselineStatus, Report};
use crate::{EXIT_NOT_COMPARABLE, EXIT_REGRESSION};
use anyhow::{Context, Result};
use clap::Args;
use std::process::ExitCode;

/// `docs/30` §Measurement. Expressed as a fraction so the flag reads in the
/// same units as the document.
pub const DEFAULT_TOLERANCE: f64 = 0.10;

/// How far a corpus dimension may move before the two runs are measuring
/// different databases.
///
/// The corpus is generated deterministically from a seed and a scale, so the
/// declared counts (tasks, projects, users) are exact and the derived ones
/// (comments, activity events) vary by well under a tenth of a percent between
/// seeds. One percent is therefore loose enough that regenerating the corpus is
/// not a gate failure, and tight enough that the case this exists for — a run
/// against a corpus orders of magnitude smaller — cannot slip through.
const CORPUS_DRIFT_TOLERANCE_PCT: f64 = 1.0;

/// How far a case's result-set size may move. Deliberately looser than the
/// corpus tolerance: some cases are bounded by a `LIMIT` and some by how many
/// rows the frozen corpus clock leaves overdue, and the point here is to catch
/// a query that has started answering a different question, not to police
/// single rows.
const ROW_COUNT_DRIFT_TOLERANCE_PCT: f64 = 10.0;

/// Percentage difference between two counts, relative to the baseline, or
/// `None` when there is nothing to compare.
///
/// A baseline of zero returns `None` rather than infinity: a case that legibly
/// returned no rows when the baseline was taken is a known gap recorded in the
/// report's notes, and it is not this function's job to re-litigate it.
fn corpus_drift_pct(baseline: i64, report: i64) -> Option<f64> {
    if baseline <= 0 {
        return None;
    }
    Some(((report - baseline).abs() as f64 / baseline as f64) * 100.0)
}

#[derive(Debug, Args)]
pub struct CompareArgs {
    /// Committed baseline from benchmarks/.
    #[arg(long)]
    pub baseline: std::path::PathBuf,

    /// Report produced by `run`.
    #[arg(long)]
    pub report: std::path::PathBuf,

    /// Fraction of the baseline p95 a run may exceed before it fails.
    #[arg(long, default_value_t = DEFAULT_TOLERANCE)]
    pub tolerance: f64,

    /// Absolute increase, in microseconds, below which a proportional
    /// regression is not treated as a failure.
    ///
    /// Zero by default, which is exactly what docs/30 specifies. It exists
    /// because 10% of a 60 µs case is 6 µs, which is below the scheduling noise
    /// of any shared runner — but raising it is a deviation from the document
    /// and belongs in a PR that says so.
    #[arg(long, default_value_t = 0)]
    pub noise_floor_us: u64,
}

pub fn execute(args: &CompareArgs) -> Result<ExitCode> {
    let baseline = load(&args.baseline)?;
    let report = load(&args.report)?;
    let outcome = compare(&baseline, &report, args.tolerance, args.noise_floor_us);
    print_outcome(args, &baseline, &report, &outcome);
    Ok(ExitCode::from(outcome.exit_code() as u8))
}

fn load(path: &std::path::Path) -> Result<Report> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub case: String,
    pub baseline_p95_us: u64,
    pub report_p95_us: u64,
    pub delta_pct: f64,
    pub regressed: bool,
    /// True when the proportional change exceeded the tolerance but the
    /// absolute change was under `--noise-floor-us`.
    pub excused_by_noise_floor: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// Reasons the comparison could not be performed. Non-empty means the gate
    /// did not run; verdicts are then meaningless and are not produced.
    pub blockers: Vec<String>,
    pub verdicts: Vec<Verdict>,
    /// Cases the report measured that the baseline does not carry. Not a
    /// failure — a new case must be able to land before its baseline does — but
    /// it is printed so the follow-up is visible.
    pub unbaselined: Vec<String>,
}

impl Outcome {
    pub fn regressions(&self) -> impl Iterator<Item = &Verdict> {
        self.verdicts.iter().filter(|v| v.regressed)
    }

    pub fn exit_code(&self) -> i32 {
        if !self.blockers.is_empty() {
            EXIT_NOT_COMPARABLE
        } else if self.regressions().next().is_some() {
            EXIT_REGRESSION
        } else {
            0
        }
    }
}

/// The whole decision, as a pure function of two reports. Kept free of I/O so
/// the gate's behaviour is testable without a database.
pub fn compare(baseline: &Report, report: &Report, tolerance: f64, noise_floor_us: u64) -> Outcome {
    let mut outcome = Outcome::default();

    match &baseline.baseline {
        None => outcome.blockers.push(
            "the baseline file carries no `baseline` block, so it is a report rather than a \
             committed baseline"
                .to_owned(),
        ),
        Some(meta) if meta.status == BaselineStatus::Placeholder => outcome.blockers.push(format!(
            "the baseline for `{}` is a placeholder, not a measurement. It exists to fix the \
             file's shape; no run can pass against it. Record a measured baseline first \
             (benchmarks/README.md).",
            baseline.environment
        )),
        Some(_) => {}
    }

    if baseline.schema_version != report.schema_version {
        outcome.blockers.push(format!(
            "schemaVersion {} (baseline) vs {} (report): the meaning of the numbers changed, \
             so they are not comparable",
            baseline.schema_version, report.schema_version
        ));
    }
    if baseline.environment != report.environment {
        outcome.blockers.push(format!(
            "environment `{}` (baseline) vs `{}` (report): docs/30 §Reference environment — \
             measurements from another machine are not comparable",
            baseline.environment, report.environment
        ));
    }
    if baseline.corpus.scale != report.corpus.scale {
        outcome.blockers.push(format!(
            "corpus scale `{}` (baseline) vs `{}` (report): a reduced-corpus run does not \
             gate a reference-corpus baseline",
            baseline.corpus.scale, report.corpus.scale
        ));
    }
    // The scale *label* is free text the caller passes as `--corpus-scale`, so
    // on its own it certifies nothing: a run against ten tasks labelled
    // `reference` compares cleanly against a two-million-task baseline and every
    // case looks faster. The counts are already recorded in both files; this
    // reads them.
    for (dimension, base, now) in [
        ("tasks", baseline.corpus.tasks, report.corpus.tasks),
        ("projects", baseline.corpus.projects, report.corpus.projects),
        ("users", baseline.corpus.users, report.corpus.users),
        (
            "searchDocuments",
            baseline.corpus.search_documents,
            report.corpus.search_documents,
        ),
        (
            "activityEvents",
            baseline.corpus.activity_events,
            report.corpus.activity_events,
        ),
    ] {
        if let Some(drift) = corpus_drift_pct(base, now)
            && drift > CORPUS_DRIFT_TOLERANCE_PCT
        {
            outcome.blockers.push(format!(
                "corpus {dimension}: {base} (baseline) vs {now} (report), {drift:.1}% apart. \
                 A smaller corpus is faster for a reason that is not an improvement"
            ));
        }
    }

    if baseline.percentile_method != report.percentile_method {
        outcome.blockers.push(format!(
            "percentile method `{}` (baseline) vs `{}` (report)",
            baseline.percentile_method, report.percentile_method
        ));
    }
    if report.iterations < baseline.iterations {
        outcome.blockers.push(format!(
            "report ran {} iterations against a baseline of {}: fewer samples make a p95 \
             less well founded, never more",
            report.iterations, baseline.iterations
        ));
    }
    if report.warmup_iterations < baseline.warmup_iterations {
        outcome.blockers.push(format!(
            "report warmed up {} rounds against a baseline of {}",
            report.warmup_iterations, baseline.warmup_iterations
        ));
    }
    if !(0.0..=10.0).contains(&tolerance) {
        outcome
            .blockers
            .push(format!("--tolerance {tolerance} is outside 0.0..=10.0"));
    }
    // `--tolerance` was bounded and `--noise-floor-us` was not, which made the
    // second one a silent off switch: any large value excuses every case, each
    // verdict prints as "under noise floor", and the run exits 0. Bound it
    // against the smallest baseline p95, because the floor exists for cases
    // whose 10% is smaller than a scheduler tick — not for the slow ones it
    // would otherwise hide.
    if let Some(smallest) = baseline
        .cases
        .iter()
        .map(|c| c.p95_us)
        .filter(|p| *p > 0)
        .min()
        && noise_floor_us > smallest
    {
        outcome.blockers.push(format!(
            "--noise-floor-us {noise_floor_us} exceeds the smallest baseline p95 \
             ({smallest} µs), so it would excuse an arbitrarily large regression on \
             every case. It exists for cases where 10% is below scheduling noise"
        ));
    }

    for base_case in &baseline.cases {
        let Some(now) = report.case(&base_case.id) else {
            outcome.blockers.push(format!(
                "case `{}` is in the baseline but not in the report: a dropped case must not \
                 pass a gate",
                base_case.id
            ));
            continue;
        };
        if base_case.p95_us == 0 {
            outcome.blockers.push(format!(
                "case `{}` has a baseline p95 of 0 µs, which is not a measurement",
                base_case.id
            ));
            continue;
        }
        // `rows_returned` is recorded precisely so that a case which stops
        // finding anything cannot be read as a case that got faster. Until now
        // nothing looked at it, which left the fastest way to pass this gate —
        // return no rows — entirely unguarded.
        if base_case.rows_returned > 0 && now.rows_returned == 0 {
            outcome.blockers.push(format!(
                "case `{}` returned 0 rows against a baseline of {}: the query stopped \
                 finding anything, which is faster but is not a measurement of the path \
                 it names",
                base_case.id, base_case.rows_returned
            ));
            continue;
        }
        if let Some(drift) = corpus_drift_pct(base_case.rows_returned, now.rows_returned)
            && drift > ROW_COUNT_DRIFT_TOLERANCE_PCT
        {
            outcome.blockers.push(format!(
                "case `{}` returned {} rows against a baseline of {} ({drift:.1}% apart): \
                 the corpus is frozen, so a case answering a differently sized question is \
                 not comparable",
                base_case.id, now.rows_returned, base_case.rows_returned
            ));
            continue;
        }
        let delta_pct =
            (now.p95_us as f64 - base_case.p95_us as f64) / base_case.p95_us as f64 * 100.0;
        let over_tolerance = delta_pct > tolerance * 100.0;
        let absolute_increase = now.p95_us.saturating_sub(base_case.p95_us);
        let excused = over_tolerance && absolute_increase <= noise_floor_us;
        outcome.verdicts.push(Verdict {
            case: base_case.id.clone(),
            baseline_p95_us: base_case.p95_us,
            report_p95_us: now.p95_us,
            delta_pct,
            regressed: over_tolerance && !excused,
            excused_by_noise_floor: excused,
        });
    }

    for now in &report.cases {
        if baseline.case(&now.id).is_none() {
            outcome.unbaselined.push(now.id.clone());
        }
    }

    if !outcome.blockers.is_empty() {
        // Verdicts computed against an incomparable baseline would be read as
        // results. Withhold them.
        outcome.verdicts.clear();
    }
    outcome
}

fn print_outcome(args: &CompareArgs, baseline: &Report, report: &Report, outcome: &Outcome) {
    println!(
        // The noise floor is printed even when it is zero. A reader of a CI log
        // must be able to see whether the gate was softened without going to
        // find the invocation.
        "environment {} · corpus {} · tolerance {:.0}% · noise floor {} µs · \
         baseline {} · report {}",
        report.environment,
        report.corpus.scale,
        args.tolerance * 100.0,
        args.noise_floor_us,
        args.baseline.display(),
        args.report.display()
    );
    if baseline.generated_at != report.generated_at {
        println!(
            "baseline recorded {} · report generated {}",
            baseline.generated_at, report.generated_at
        );
    }
    println!();

    if !outcome.blockers.is_empty() {
        println!("GATE DID NOT RUN — the comparison is not valid:");
        for b in &outcome.blockers {
            println!("  - {b}");
        }
        return;
    }

    println!(
        "{:<28} {:>12} {:>12} {:>9}  verdict",
        "case", "baseline p95", "report p95", "delta"
    );
    for v in &outcome.verdicts {
        let verdict = if v.regressed {
            "REGRESSION"
        } else if v.excused_by_noise_floor {
            "under noise floor"
        } else {
            "ok"
        };
        println!(
            "{:<28} {:>9} µs {:>9} µs {:>+8.1}%  {}",
            v.case, v.baseline_p95_us, v.report_p95_us, v.delta_pct, verdict
        );
    }

    if !outcome.unbaselined.is_empty() {
        println!(
            "\nnot in the baseline (add them when this baseline is next recorded): {}",
            outcome.unbaselined.join(", ")
        );
    }

    let regressions: Vec<&Verdict> = outcome.regressions().collect();
    if regressions.is_empty() {
        println!(
            "\nno case regressed by more than {:.0}%.",
            args.tolerance * 100.0
        );
    } else {
        println!("\n{} case(s) regressed:", regressions.len());
        for v in regressions {
            println!(
                "  {} p95 {} µs → {} µs ({:+.1}%, +{} µs)",
                v.case,
                v.baseline_p95_us,
                v.report_p95_us,
                v.delta_pct,
                v.report_p95_us.saturating_sub(v.baseline_p95_us)
            );
        }
        println!(
            "\nEither the change is a regression, or the baseline is wrong. Updating the \
             baseline requires the PR to state why the new number is acceptable \
             (benchmarks/README.md, docs/30 §Measurement)."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{BaselineMeta, fixture};

    fn measured(mut r: Report) -> Report {
        r.baseline = Some(BaselineMeta {
            status: BaselineStatus::Measured,
            justification: "initial".into(),
            recorded_by: "F-007".into(),
        });
        r
    }

    #[test]
    fn an_identical_run_passes() {
        let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
        let now = fixture("ref", "reduced", &[("a", 1_000)]);
        let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
        assert_eq!(outcome.exit_code(), 0);
        assert!(outcome.blockers.is_empty());
    }

    #[test]
    fn a_ten_percent_regression_is_allowed_and_eleven_is_not() {
        let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
        let at_ten = fixture("ref", "reduced", &[("a", 1_100)]);
        assert_eq!(compare(&base, &at_ten, DEFAULT_TOLERANCE, 0).exit_code(), 0);

        let over = fixture("ref", "reduced", &[("a", 1_101)]);
        let outcome = compare(&base, &over, DEFAULT_TOLERANCE, 0);
        assert_eq!(outcome.exit_code(), EXIT_REGRESSION);
        let v = outcome.regressions().next().expect("one regression");
        assert_eq!(v.case, "a");
        assert!((v.delta_pct - 10.1).abs() < 1e-9, "{}", v.delta_pct);
    }

    #[test]
    fn an_improvement_never_fails() {
        let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
        let faster = fixture("ref", "reduced", &[("a", 10)]);
        assert_eq!(compare(&base, &faster, DEFAULT_TOLERANCE, 0).exit_code(), 0);
    }

    #[test]
    fn a_placeholder_baseline_can_never_be_passed() {
        let mut base = fixture("ref", "reduced", &[("a", 1_000)]);
        base.baseline = Some(BaselineMeta {
            status: BaselineStatus::Placeholder,
            justification: String::new(),
            recorded_by: "F-007".into(),
        });
        // Even a run that is faster than the placeholder must not pass.
        let now = fixture("ref", "reduced", &[("a", 1)]);
        let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
        assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
        assert!(outcome.verdicts.is_empty());
        assert!(
            outcome.blockers[0].contains("placeholder"),
            "{:?}",
            outcome.blockers
        );
    }

    #[test]
    fn a_plain_report_is_not_a_baseline() {
        let base = fixture("ref", "reduced", &[("a", 1_000)]);
        let now = fixture("ref", "reduced", &[("a", 1_000)]);
        let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
        assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
    }

    #[test]
    fn a_different_environment_is_refused_rather_than_compared() {
        let base = measured(fixture("reference-8vcpu-32gb", "reduced", &[("a", 1_000)]));
        let now = fixture("some-laptop", "reduced", &[("a", 1_000)]);
        let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
        assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
        assert!(outcome.blockers.iter().any(|b| b.contains("environment")));
    }

    #[test]
    fn a_different_corpus_scale_is_refused() {
        let base = measured(fixture("ref", "reference", &[("a", 1_000)]));
        let now = fixture("ref", "reduced", &[("a", 1_000)]);
        assert_eq!(
            compare(&base, &now, DEFAULT_TOLERANCE, 0).exit_code(),
            EXIT_NOT_COMPARABLE
        );
    }

    #[test]
    fn a_schema_version_change_is_refused() {
        let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
        let mut now = fixture("ref", "reduced", &[("a", 1_000)]);
        now.schema_version += 1;
        assert_eq!(
            compare(&base, &now, DEFAULT_TOLERANCE, 0).exit_code(),
            EXIT_NOT_COMPARABLE
        );
    }

    #[test]
    fn fewer_iterations_than_the_baseline_is_refused() {
        let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
        let mut now = fixture("ref", "reduced", &[("a", 1_000)]);
        now.iterations = 10;
        let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
        assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);

        // More is fine: extra samples never weaken a p95.
        let mut more = fixture("ref", "reduced", &[("a", 1_000)]);
        more.iterations = 5_000;
        assert_eq!(compare(&base, &more, DEFAULT_TOLERANCE, 0).exit_code(), 0);
    }

    #[test]
    fn dropping_a_baselined_case_does_not_pass_the_gate() {
        let base = measured(fixture("ref", "reduced", &[("a", 1_000), ("b", 1_000)]));
        let now = fixture("ref", "reduced", &[("a", 1_000)]);
        let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
        assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
        assert!(outcome.blockers.iter().any(|b| b.contains("`b`")));
    }

    #[test]
    fn a_new_case_is_reported_but_does_not_fail() {
        let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
        let now = fixture("ref", "reduced", &[("a", 1_000), ("new_case", 9_999)]);
        let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
        assert_eq!(outcome.exit_code(), 0);
        assert_eq!(outcome.unbaselined, vec!["new_case".to_owned()]);
    }

    #[test]
    fn the_noise_floor_excuses_a_small_absolute_move_only_when_asked() {
        let base = measured(fixture("ref", "reduced", &[("a", 60)]));
        let now = fixture("ref", "reduced", &[("a", 80)]); // +33%, +20 µs

        // Default behaviour is exactly docs/30: proportional only.
        assert_eq!(
            compare(&base, &now, DEFAULT_TOLERANCE, 0).exit_code(),
            EXIT_REGRESSION
        );

        // 25 µs: below the 60 µs baseline, so it is a floor rather than an off
        // switch, and still large enough to absorb the +20 µs move.
        let excused = compare(&base, &now, DEFAULT_TOLERANCE, 25);
        assert_eq!(excused.exit_code(), 0);
        assert!(excused.verdicts[0].excused_by_noise_floor);
        assert!(!excused.verdicts[0].regressed);
    }

    #[test]
    fn a_noise_floor_larger_than_the_smallest_baseline_is_refused() {
        // `--tolerance` was bounded and this was not, which made it a silent
        // off switch: a large enough value excuses every case, every verdict
        // reads "under noise floor", and the run exits 0 with nothing in the
        // log to say the gate had been softened.
        let base = measured(fixture("ref", "reduced", &[("a", 60), ("b", 90_000)]));
        let now = fixture("ref", "reduced", &[("a", 6_000), ("b", 900_000)]);

        let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 1_000_000);
        assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
        assert!(
            outcome
                .blockers
                .iter()
                .any(|b| b.contains("noise-floor-us")),
            "{:?}",
            outcome.blockers
        );
    }

    #[test]
    fn the_noise_floor_does_not_excuse_a_large_absolute_move() {
        let base = measured(fixture("ref", "reduced", &[("a", 100_000)]));
        let now = fixture("ref", "reduced", &[("a", 200_000)]);
        assert_eq!(
            compare(&base, &now, DEFAULT_TOLERANCE, 250).exit_code(),
            EXIT_REGRESSION
        );
    }

    #[test]
    fn a_zero_baseline_p95_is_refused_rather_than_dividing_by_zero() {
        let base = measured(fixture("ref", "reduced", &[("a", 0)]));
        let now = fixture("ref", "reduced", &[("a", 5)]);
        let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
        assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
    }

    #[test]
    fn blockers_suppress_verdicts_so_they_cannot_be_read_as_results() {
        let base = measured(fixture("a", "reduced", &[("a", 1_000)]));
        let now = fixture("b", "reduced", &[("a", 1_000)]);
        let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
        assert!(outcome.verdicts.is_empty());
    }
}
