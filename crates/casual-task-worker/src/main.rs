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

    // The dispatch loop is implemented (C-011) and the first consumer now
    // exists (C-016, `notify::NotificationFanout`) — but the loop still cannot
    // be started here, and the reason is a configuration decision rather than
    // missing code.
    //
    // `dispatch::claim` runs as a role that bypasses row-level security
    // (migration 0014, `DispatcherRole::verify` refuses anything else), and the
    // consumer's own reads run as `taskforge_app`. That is two DSNs. `docs/48`
    // §Configuration names one `DATABASE_URL` and no second one, so there is
    // nowhere documented for the dispatcher's credentials to come from, and
    // inventing an environment variable here would settle a deployment question
    // in a binary. Tracked as **D-060**.
    //
    // Starting the loop with the application role would fail `verify` on the
    // first poll and restart-loop; starting it with no consumers would poll an
    // empty table forever and look like a working worker. Neither is honest, so
    // neither is done.
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        consumer = casual_task_worker::consumers::notification::NAME,
        "taskforge worker — dispatch loop and notification fan-out built; \
         not started: the dispatcher DSN is undecided (D-060)"
    );
    ExitCode::SUCCESS
}
