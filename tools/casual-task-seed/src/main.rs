//! # casual-task-seed
//!
//! The deterministic reference corpus generator (F-006).
//!
//! `docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md` §Reference capacity defines
//! one workspace — 2,000,000 tasks, 200 projects, 500 users, 20,000,000
//! activity events — and every latency gate in that document is measured
//! against it. This tool produces that workspace, and produces it *identically*
//! every time: a regression check compares today's number against a number
//! recorded weeks ago, and the comparison only means something if both ran over
//! the same rows.
//!
//! Two decisions follow from that and are worth stating up front:
//!
//! * **The clock is frozen.** `--now` defaults to a fixed instant, not the
//!   wall clock, because "due next week" has to mean the same thing on every
//!   run. The cost is that the corpus ages: regenerate it, or pass `--now`, if
//!   the overdue tail matters to what is being measured.
//! * **The output is `COPY` text, not `INSERT`.** At two million rows the
//!   difference is not a percentage.
//!
//! ```text
//! cargo run --release -p casual-task-seed -- --scale tiny --out ./corpus
//! cd corpus && ./load.sh "$DATABASE_URL"
//! ```

mod copy;
mod det;
mod extras;
mod labels;
mod loader;
mod scale;
mod tasks;
mod vocab;
mod world;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::copy::Sink;
use crate::scale::{Plan, Scale};

/// The corpus clock. Fixed, so that "overdue" and "due this week" mean the same
/// thing on every run — see the note about determinism above.
const DEFAULT_NOW: &str = "2026-06-01T00:00:00Z";

#[derive(Parser, Debug)]
#[command(
    name = "casual-task-seed",
    about = "Generate the deterministic TaskForge reference corpus as PostgreSQL COPY files"
)]
struct Args {
    /// Corpus size. `reference` is the gated corpus from docs/30.
    #[arg(long, value_enum, default_value = "tiny")]
    scale: Scale,

    /// Corpus seed. The same seed always produces byte-identical output.
    #[arg(long, default_value_t = 20_260_101)]
    seed: u64,

    /// Output directory. Created if absent; existing corpus files are replaced.
    #[arg(long, default_value = "target/corpus")]
    out: PathBuf,

    /// The instant the corpus treats as "now", RFC 3339. Changing it changes
    /// every generated timestamp and identifier — deliberately.
    #[arg(long, default_value = DEFAULT_NOW)]
    now: String,

    /// Print an Argon2id hash of this password and exit, generating no corpus.
    ///
    /// `scripts/dev-up.sh` needs one account it can log in as, and the hash
    /// parameters are a security decision (`docs/40`: 64 MiB, t=3, p=4) that
    /// belongs in `casual-task-identity` and nowhere else. A script that
    /// shelled out to a generic Argon2 tool would pick its own parameters, and
    /// the demo login would then be protected differently from every real one.
    #[arg(long)]
    hash_password: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(password) = args.hash_password.as_deref() {
        // `hash_chosen`, not `hash_generated`: this is a password a person
        // types, so it must meet the same minimum length every real chosen
        // password does. A demo account exempt from the policy is a demo of a
        // different system.
        println!("{}", casual_task_identity::password::hash_chosen(password)?);
        return Ok(());
    }

    let now_ms = parse_now(&args.now)?;
    let plan = Plan::for_scale(args.scale);

    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("creating output directory {}", args.out.display()))?;
    clean(&args.out)?;

    let started = Instant::now();
    let mut sink = Sink::create(&args.out).context("opening the corpus files")?;

    let world = world::build(&mut sink, &plan, args.seed, now_ms);
    let sample = tasks::generate(&mut sink, &world, args.seed);
    extras::generate(&mut sink, &world, &plan, args.seed, &sample.ids);
    let counts = sink.finish().context("flushing the corpus files")?;

    loader::write(
        &args.out,
        &loader::Summary {
            plan: &plan,
            seed: args.seed,
            now: now_ms,
            workspace_id: world.workspace_id,
            counts: &counts,
        },
    )
    .context("writing the loader")?;

    report(&args, &plan, &counts, started.elapsed());
    // Printed last and to stderr, so it survives `| head` and is visible in a
    // log where the count table has scrolled past. A corpus that quietly came
    // up short is the failure this exists to prevent.
    for note in &world.notes {
        eprintln!("warning: {note}");
    }
    Ok(())
}

fn parse_now(raw: &str) -> Result<i64> {
    let t = OffsetDateTime::parse(raw, &Rfc3339)
        .with_context(|| format!("--now must be RFC 3339, got {raw:?}"))?;
    Ok((t.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// Replace a previous corpus rather than merge with it. A stale `.copy` file
/// from a larger scale would still be listed in the loader of the smaller one
/// and would load rows nothing else references.
fn clean(dir: &std::path::Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ours = name.ends_with(".copy")
            || name == loader::LOADER_FILE
            || name == loader::SCRIPT_FILE
            || name == loader::MANIFEST_FILE;
        if ours && path.is_file() {
            std::fs::remove_file(&path)
                .with_context(|| format!("removing stale {}", path.display()))?;
        }
    }
    Ok(())
}

fn report(
    args: &Args,
    plan: &Plan,
    counts: &std::collections::BTreeMap<&'static str, u64>,
    elapsed: std::time::Duration,
) {
    println!(
        "casual-task-seed: scale={} seed={} clock={} out={}",
        plan.scale.as_str(),
        args.seed,
        args.now,
        args.out.display()
    );
    println!();

    let width = counts.keys().map(|k| k.len()).max().unwrap_or(10);
    // Reported in load order, not alphabetically: this is the order the rows go
    // in, and reading it that way makes a missing dependency obvious.
    let mut total = 0;
    for table in copy::Table::ALL {
        let rows = counts.get(table.name()).copied().unwrap_or(0);
        total += rows;
        println!("  {:<width$}  {:>12}", table.name(), rows, width = width);
    }
    println!("  {:<width$}  {:>12}", "TOTAL", total, width = width);
    println!();

    let bytes: u64 = std::fs::read_dir(&args.out)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();
    println!(
        "{total} rows in {:.2}s, {:.1} MiB on disk",
        elapsed.as_secs_f64(),
        bytes as f64 / (1024.0 * 1024.0)
    );
    println!(
        "load with:  cd {} && ./{} \"$DATABASE_URL\"",
        args.out.display(),
        loader::SCRIPT_FILE
    );
}
