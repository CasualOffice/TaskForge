//! `casual-task-loadtest` — the latency harness and its baseline format (F-007).
//!
//! See `docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md` §Measurement. The rule
//! this whole tool is shaped around is stated there:
//!
//! > CI fails on a **>10% regression** against the committed baseline, not on
//! > absolute numbers — absolute thresholds fail on CI noise and get disabled,
//! > which is worse than no gate.
//!
//! # What this measures today
//!
//! **Database round-trip latency, and nothing else.** Phase 0 has no product
//! code: there is no HTTP server, no authorization resolver, no serializer, and
//! no connection pool to measure. Rather than invent a synthetic stand-in for
//! those, the harness measures the one component that exists, will dominate the
//! server-side budget, and can be measured honestly today — the SQL in
//! `docs/26-SEARCH-INDEXING-AND-QUERY.md`, executed against a real PostgreSQL
//! as the non-superuser `taskforge_app` role so row-level security is in force.
//!
//! # What this does NOT measure — read this before quoting a number
//!
//! A number from this harness is a **floor**, not an estimate of the figure in
//! the `docs/30` latency table. Everything below is excluded and arrives in
//! Phase 1 with the code that causes it:
//!
//! | Excluded | Why it is excluded | Arrives |
//! | --- | --- | --- |
//! | HTTP framing, TLS, routing, middleware | no API process exists (C-001+) | Phase 1 |
//! | Authorization resolution | the resolver is Phase 1; only its *query* is timed here | Phase 1 |
//! | JSON serialization and response payload assembly | no DTO layer exists | Phase 1 |
//! | Connection-pool checkout and `SET LOCAL` per transaction | see `Session scope` below | Phase 1 |
//! | Concurrency — every case runs single-threaded, one query in flight | a mixed concurrent workload needs the real client (docs/30 §Throughput) | Phase 1 |
//! | Write paths (create / update / transition) | see `Writes` below | Phase 1 |
//! | Client-side row formatting | output is routed to `/dev/null` inside `psql`; rows are still transferred over the wire into the client result set, but are not rendered | n/a |
//! | Cold cache / cold buffer pool | warm-up runs deliberately warm the cache; a cold-start profile is a different measurement | Phase 1 |
//!
//! **Writes.** No write case is measured. An honest write measurement needs a
//! real `COMMIT` (so the WAL flush is included) which mutates the corpus and
//! makes the run non-repeatable, or a `ROLLBACK` which excludes the WAL flush
//! and therefore understates the dominant cost of a write. Neither is worth
//! committing a baseline against, so the write rows of the `docs/30` table are
//! declared in the report's `notMeasured` list instead of being approximated.
//!
//! **Session scope.** The application sets `SET LOCAL taskforge.workspace_id`
//! per transaction (`migrations/0010`). This harness sets it once per session,
//! so the per-transaction `BEGIN`/`SET LOCAL`/`COMMIT` round trips are not in
//! any case's number. The `roundtrip_floor` case exists so that cost has a unit
//! to be read against: every other case should be interpreted as "this much
//! above the protocol floor".
//!
//! # Why `psql` and not a Rust driver
//!
//! The workspace has no PostgreSQL client dependency (see the root
//! `Cargo.toml`), and adding one is a workspace-level decision this task is not
//! entitled to make. The harness therefore drives one long-lived `psql`
//! process over a pipe and reads its `\timing` output, which libpq measures
//! around the query round trip itself. The process is spawned once per run, so
//! process start-up is not in any sample. [`db::Session`] is the seam: when the
//! workspace gains a pooled driver, that one type is replaced and the case
//! catalogue, the report, and the gate are untouched.

mod cases;
mod compare;
mod db;
mod fixtures;
mod report;
mod run;
mod stats;

use clap::{Parser, Subcommand};

/// Exit code for a comparison that found a regression over tolerance.
const EXIT_REGRESSION: i32 = 1;
/// Exit code for a comparison that could not be performed at all — mismatched
/// environment, mismatched schema version, a placeholder baseline, a missing
/// case. Distinct from [`EXIT_REGRESSION`] so CI can tell "the gate says no"
/// from "the gate could not run", which are different bugs.
const EXIT_NOT_COMPARABLE: i32 = 2;

#[derive(Debug, Parser)]
#[command(
    name = "casual-task-loadtest",
    about = "Database latency harness and baseline gate (docs/30 §Measurement)",
    long_about = None,
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Measure every case against a seeded corpus and emit a versioned report.
    Run(run::RunArgs),
    /// Compare a report against a committed baseline; non-zero on regression.
    Compare(compare::CompareArgs),
    /// List the case catalogue and the `docs/30` row each case maps to.
    Cases,
}

fn main() -> std::process::ExitCode {
    match dispatch() {
        Ok(code) => code,
        Err(err) => {
            // `{err:#}` renders the whole anyhow context chain; the top-level
            // message alone is rarely enough to act on in CI logs.
            eprintln!("casual-task-loadtest: {err:#}");
            std::process::ExitCode::from(EXIT_NOT_COMPARABLE as u8)
        }
    }
}

fn dispatch() -> anyhow::Result<std::process::ExitCode> {
    match Cli::parse().command {
        Command::Run(args) => {
            run::execute(&args)?;
            Ok(std::process::ExitCode::SUCCESS)
        }
        Command::Compare(args) => compare::execute(&args),
        Command::Cases => {
            cases::print_catalogue();
            Ok(std::process::ExitCode::SUCCESS)
        }
    }
}
