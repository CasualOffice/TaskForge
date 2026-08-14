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
    // `report.iterations < baseline.iterations` above says the report is no
    // worse founded than the baseline. It says nothing about whether the
    // baseline was founded at all — a file committed from `--iterations 5`,
    // where the p95 is just the largest of five samples, gated exactly as
    // firmly as one from a thousand. `stats` already knows the number; it was
    // only ever used to write a note, and notes are never compared.
    if baseline.iterations < crate::stats::P95_CONFIDENCE_MIN_SAMPLES as u32 {
        outcome.blockers.push(format!(
            "the baseline was recorded from {} iterations; {} is the minimum this \
             harness treats as a p95 worth gating on. It is a measurement, not a gate",
            baseline.iterations,
            crate::stats::P95_CONFIDENCE_MIN_SAMPLES
        ));
    }
    for case in &baseline.cases {
        if (case.samples as usize) < crate::stats::P95_CONFIDENCE_MIN_SAMPLES {
            outcome.blockers.push(format!(
                "baseline case `{}` carries {} samples, below the {} needed for a p95 \
                 worth gating on",
                case.id,
                case.samples,
                crate::stats::P95_CONFIDENCE_MIN_SAMPLES
            ));
        }
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
#[path = "compare_tests.rs"]
mod tests;
