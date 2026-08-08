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
    // See the note in casual-task-api's main: installing the subscriber is what
    // makes the configuration in docs/48 real rather than declared.
    if let Err(error) = casual_task_observability::init() {
        eprintln!("failed to initialise telemetry: {error}");
        return ExitCode::FAILURE;
    }

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "taskforge worker — Phase 0 scaffold, no consumers yet (docs/06)"
    );
    ExitCode::SUCCESS
}
