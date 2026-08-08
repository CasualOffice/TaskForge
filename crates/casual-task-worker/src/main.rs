//! # casual-task-worker
//!
//! The worker binary: outbox dispatch, search projection, notification fan-out,
//! webhook delivery, scan coordination, automation execution, retention sweeps,
//! and rank compaction (`docs/25`, `docs/36`, `docs/46`).
//!
//! Runs embedded in the API process on the single-node profile
//! (`TF_WORKER_EMBEDDED=true`) and as a separate binary above it
//! (`docs/48-DEPLOYMENT-PROFILES.md`).
//!
//! Phase 0 scaffold — no consumers yet. See `docs/14-EXECUTION-TRACKER.md`.

use std::process::ExitCode;

fn main() -> ExitCode {
    // See the note in casual-task-api's main: the image gate checks that the
    // binary executes, and running a worker to completion is not that question.
    if std::env::args().skip(1).any(|arg| arg == "--version") {
        println!("taskforge-worker {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    // See the note in casual-task-api's main: installing the subscriber is what
    // makes the configuration in docs/48 real rather than declared.
    if let Err(error) = casual_task_observability::init() {
        eprintln!("failed to initialise telemetry: {error}");
        return ExitCode::FAILURE;
    }

    // The dispatch loop is implemented (C-011) but has no consumers to run:
    // the six named in docs/25 arrive with C-013, C-015 and C-016. Starting a
    // loop with nothing to deliver would poll an empty table forever and look
    // like a working worker, so it is not started. Stated, not hidden.
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "taskforge worker — dispatch loop built (C-011); no consumers registered yet"
    );
    ExitCode::SUCCESS
}
