//! `run` — measure the case catalogue and emit a versioned report.
//!
//! # Why the cases are interleaved rather than run one after another
//!
//! Measurement rounds are round-robin: every case runs once per round, for
//! `--iterations` rounds. Running each case to completion in turn would give
//! each one a different few seconds of machine weather, and a background
//! process that woke up during case four would show up as a regression in case
//! four alone. Interleaving spreads that noise across every case, which is what
//! a gate comparing per-case p95s needs.
//!
//! The cost is stated: interleaving also means each case's working set is
//! evicted by the others between samples, so no case gets a private warm cache.
//! That is closer to a mixed workload than to a microbenchmark, and it is the
//! behaviour the product will have.
//!
//! # Warm-up
//!
//! `--warmup` rounds run before measurement and are discarded. They pay for
//! plan caching, first-touch of the buffer pool, and `psql`'s own lazy
//! initialisation. Their samples are thrown away entirely rather than averaged
//! in, because a cold first sample is a different measurement — see the crate
//! docs for what a cold profile would need.

use crate::cases::{CASES, Case, GAPS};
use crate::db::{ConnectSpec, Session};
use crate::report::{CaseResult, NotMeasured, PERCENTILE_METHOD, Report, SCHEMA_VERSION};
use crate::{cases, fixtures, stats};
use anyhow::{Context, Result, bail};
use clap::Args;

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Named machine profile from docs/30 §Reference environment. Baselines are
    /// per environment; a measurement from another machine is not comparable.
    #[arg(long)]
    pub environment: String,

    /// RFC 3339 timestamp recorded as the report's provenance. Supplied rather
    /// than read from the clock so a rerun of the same measurement is
    /// byte-identical — see the report module docs.
    #[arg(long)]
    pub generated_at: String,

    /// Corpus label, e.g. `reference` or `reduced`. Part of the baseline's
    /// identity: docs/30 §Measurement runs the full corpus nightly and a
    /// reduced one per PR, and those two are never compared to each other.
    #[arg(long)]
    pub corpus_scale: String,

    /// PostgreSQL connection string. Must be the non-superuser application
    /// role: row-level security is inert for a superuser (migrations/0012), so
    /// a superuser measurement omits a predicate every real query pays.
    #[arg(long)]
    pub dsn: String,

    /// How to reach `psql`, whitespace-separated. Defaults to `psql` on PATH;
    /// `--psql "docker exec -i tf-pg psql"` routes through a container the way
    /// scripts/verify-schema.sh does.
    #[arg(long, default_value = "psql")]
    pub psql: String,

    /// Workspace to measure. Defaults to the oldest in the database; give it
    /// explicitly for any corpus with more than one.
    #[arg(long)]
    pub workspace: Option<String>,

    /// Measured rounds per case.
    #[arg(long, default_value_t = 1_000)]
    pub iterations: u32,

    /// Discarded rounds before measurement begins.
    #[arg(long, default_value_t = 100)]
    pub warmup: u32,

    /// Restrict the run to these case ids. Repeatable. A report that omits a
    /// case the baseline contains fails the gate, so this is for investigation,
    /// not for producing a baseline.
    #[arg(long = "case")]
    pub only: Vec<String>,

    /// Term bound into the full-text case. Restricted to letters, digits,
    /// spaces, `-` and `_`.
    #[arg(long, default_value = "task")]
    pub search_term: String,

    /// Write the report here instead of stdout.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
}

pub fn execute(args: &RunArgs) -> Result<()> {
    validate(args)?;
    let selected = select_cases(&args.only)?;

    let spec = ConnectSpec {
        command: args
            .psql
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        dsn: args.dsn.clone(),
    };
    let mut session = Session::connect(&spec)?;
    refuse_superuser(&mut session)?;

    // Everything up to the first warmup round is preflight: it describes and
    // checks the corpus rather than measuring it, and it is the first thing to
    // touch these tables, so it pays every cold-cache read. See
    // `Session::preflight`.
    let workspace_id =
        session.preflight(|s| fixtures::resolve_workspace(s, args.workspace.as_deref()))?;
    // Session-scoped rather than SET LOCAL: see the crate docs. The policy in
    // migrations/0010 reads current_setting either way.
    session
        .execute(&format!("SET taskforge.workspace_id = '{workspace_id}'"))
        .context("setting the tenant scope")?;

    let mut fx = session
        .preflight(|s| fixtures::bind(s, &workspace_id, &args.corpus_scale, &args.search_term))?;

    eprintln!(
        "corpus: {} tasks, {} projects, {} members, {} search docs, {} activity events",
        fx.corpus.tasks,
        fx.corpus.projects,
        fx.corpus.users,
        fx.corpus.search_documents,
        fx.corpus.activity_events
    );

    let rows = session.preflight(|s| probe_row_counts(s, &selected))?;
    for (case, count) in selected.iter().zip(&rows) {
        if *count == 0 {
            fx.notes.push(format!(
                "{}: returned 0 rows. A query that finds nothing is fast for the wrong \
                 reason; its number is not a measurement of the path it names.",
                case.id
            ));
        }
    }

    let samples = measure(&mut session, &selected, args)?;

    let mut cases_out = Vec::with_capacity(selected.len());
    for ((case, samples), rows_returned) in selected.iter().zip(samples).zip(rows) {
        let summary = stats::summarise(&samples)
            .with_context(|| format!("case {} produced no samples", case.id))?;
        cases_out.push(CaseResult {
            id: case.id.to_owned(),
            target: case.target.map(str::to_owned),
            samples: samples.len() as u32,
            min_us: summary.min_us,
            p50_us: summary.p50_us,
            p95_us: summary.p95_us,
            p99_us: summary.p99_us,
            max_us: summary.max_us,
            mean_us: summary.mean_us,
            p99_confident: samples.len() >= stats::P99_CONFIDENCE_MIN_SAMPLES,
            rows_returned,
        });
    }

    if (args.iterations as usize) < stats::P95_CONFIDENCE_MIN_SAMPLES {
        fx.notes.push(format!(
            "{} iterations is below the {} this harness considers enough for a p95 worth \
             gating on; treat this run as exploratory.",
            args.iterations,
            stats::P95_CONFIDENCE_MIN_SAMPLES
        ));
    }
    if selected.len() < CASES.len() {
        fx.notes.push(format!(
            "partial run: {} of {} cases measured. Not a baseline candidate.",
            selected.len(),
            CASES.len()
        ));
    }

    let report = Report {
        schema_version: SCHEMA_VERSION,
        harness: "casual-task-loadtest".to_owned(),
        harness_version: env!("CARGO_PKG_VERSION").to_owned(),
        environment: args.environment.clone(),
        generated_at: args.generated_at.clone(),
        corpus: fx.corpus,
        iterations: args.iterations,
        warmup_iterations: args.warmup,
        percentile_method: PERCENTILE_METHOD.to_owned(),
        cases: cases_out,
        not_measured: GAPS
            .iter()
            .map(|g| NotMeasured {
                operation: g.operation.to_owned(),
                reason: g.reason.to_owned(),
                arrives: g.arrives.to_owned(),
            })
            .collect(),
        notes: fx.notes,
        baseline: None,
    };

    let json = report.to_json()?;
    match &args.out {
        Some(path) => {
            std::fs::write(path, &json).with_context(|| format!("writing {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => print!("{json}"),
    }
    Ok(())
}

fn validate(args: &RunArgs) -> Result<()> {
    use time::format_description::well_known::Rfc3339;
    time::OffsetDateTime::parse(&args.generated_at, &Rfc3339).with_context(|| {
        format!(
            "--generated-at `{}` is not RFC 3339 (e.g. 2026-08-08T12:00:00Z)",
            args.generated_at
        )
    })?;

    if args.iterations == 0 {
        bail!("--iterations must be at least 1");
    }
    if args.environment.trim().is_empty() || args.corpus_scale.trim().is_empty() {
        bail!(
            "--environment and --corpus-scale must be non-empty: they are the identity of a baseline"
        );
    }
    if !args
        .search_term
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        || args.search_term.trim().is_empty()
    {
        bail!("--search-term must be non-empty ascii letters, digits, spaces, `-` or `_`");
    }
    Ok(())
}

fn select_cases(only: &[String]) -> Result<Vec<&'static Case>> {
    if only.is_empty() {
        return Ok(CASES.iter().collect());
    }
    only.iter()
        .map(|id| {
            cases::find(id).ok_or_else(|| {
                anyhow::anyhow!("unknown case `{id}`; run `casual-task-loadtest cases`")
            })
        })
        .collect()
}

/// `docs/32` and `migrations/0012`: RLS and append-only history are both inert
/// for a superuser. Measuring as one omits the policy predicate from every plan
/// and reads rows the application cannot see, so the numbers describe a system
/// that will never run.
fn refuse_superuser(session: &mut Session) -> Result<()> {
    let is_superuser = session
        .fetch_scalar("SELECT current_setting('is_superuser')")?
        .unwrap_or_default();
    if is_superuser == "on" {
        bail!(
            "connected as a superuser. Row-level security is inert for superusers \
             (migrations/0012), so every measurement would omit a predicate the \
             application pays on every query. Connect as taskforge_app."
        );
    }
    Ok(())
}

/// Rows each case returns, probed once and untimed. A case whose result set
/// silently empties gets faster; without this the report would call that an
/// improvement.
fn probe_row_counts(session: &mut Session, selected: &[&'static Case]) -> Result<Vec<i64>> {
    selected
        .iter()
        .map(|case| {
            let sql = format!("SELECT count(*) FROM ({}) _probe", case.sql);
            let raw = session
                .fetch_scalar(&sql)
                .with_context(|| format!("probing row count for case {}", case.id))?
                .unwrap_or_else(|| "0".to_owned());
            raw.parse::<i64>()
                .with_context(|| format!("parsing row count `{raw}` for case {}", case.id))
        })
        .collect()
}

fn measure(
    session: &mut Session,
    selected: &[&'static Case],
    args: &RunArgs,
) -> Result<Vec<Vec<u64>>> {
    for round in 0..args.warmup {
        for case in selected {
            session
                .time_query(case.sql)
                .with_context(|| format!("warm-up round {round}, case {}", case.id))?;
        }
    }

    let mut samples: Vec<Vec<u64>> = selected
        .iter()
        .map(|_| Vec::with_capacity(args.iterations as usize))
        .collect();
    let progress_every = (args.iterations / 10).max(1);
    for round in 0..args.iterations {
        for (index, case) in selected.iter().enumerate() {
            let us = session
                .time_query(case.sql)
                .with_context(|| format!("round {round}, case {}", case.id))?;
            samples[index].push(us);
        }
        if round % progress_every == 0 {
            eprintln!("  round {}/{}", round + 1, args.iterations);
        }
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> RunArgs {
        RunArgs {
            environment: "local".into(),
            generated_at: "2026-08-08T00:00:00Z".into(),
            corpus_scale: "reduced".into(),
            dsn: "postgres://x/y".into(),
            psql: "psql".into(),
            workspace: None,
            iterations: 10,
            warmup: 1,
            only: Vec::new(),
            search_term: "task".into(),
            out: None,
        }
    }

    #[test]
    fn a_valid_invocation_passes_validation() {
        assert!(validate(&args()).is_ok());
    }

    #[test]
    fn a_clock_shaped_timestamp_is_rejected() {
        let mut a = args();
        a.generated_at = "8 August 2026".into();
        assert!(validate(&a).is_err());
    }

    #[test]
    fn zero_iterations_is_rejected() {
        let mut a = args();
        a.iterations = 0;
        assert!(validate(&a).is_err());
    }

    #[test]
    fn an_empty_environment_is_rejected() {
        let mut a = args();
        a.environment = "  ".into();
        assert!(validate(&a).is_err());
    }

    #[test]
    fn a_search_term_that_could_escape_a_literal_is_rejected() {
        for term in ["o'brien", "a\\b", "", "  "] {
            let mut a = args();
            a.search_term = term.into();
            assert!(validate(&a).is_err(), "accepted {term:?}");
        }
    }

    #[test]
    fn selecting_no_cases_selects_all_of_them() {
        assert_eq!(select_cases(&[]).expect("all").len(), CASES.len());
    }

    #[test]
    fn selecting_an_unknown_case_is_an_error() {
        assert!(select_cases(&["nope".to_owned()]).is_err());
    }

    #[test]
    fn selecting_preserves_the_requested_order() {
        let picked = select_cases(&["activity_page".to_owned(), "roundtrip_floor".to_owned()])
            .expect("both");
        assert_eq!(picked[0].id, "activity_page");
        assert_eq!(picked[1].id, "roundtrip_floor");
    }
}
