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

fn main() {
    println!("taskforge worker — Phase 0 scaffold, no consumers yet (docs/06)");
}
