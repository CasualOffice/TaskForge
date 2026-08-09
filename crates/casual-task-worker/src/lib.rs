//! # casual-task-worker
//!
//! The worker: outbox dispatch, search projection, notification fan-out,
//! webhook delivery, scan coordination, automation execution, retention sweeps,
//! and rank compaction (`docs/25`, `docs/36`, `docs/46`).
//!
//! Runs embedded in the API process on the single-node profile
//! (`TF_WORKER_EMBEDDED=true`) and as a separate binary above it
//! (`docs/48-DEPLOYMENT-PROFILES.md`).
//!
//! # Why this is a library as well as a binary
//!
//! The dispatch loop's contract is about process death — killed mid-batch,
//! rows left claimed, another worker arriving later. That cannot be asserted
//! from inside `main`, and the acceptance gate `docs/25` names ("kill the
//! dispatcher mid-batch; assert every event is delivered, some twice, none
//! lost") is an integration test, which can only reach a library target.
//!
//! Embedding on the single-node profile needs the same thing: the API process
//! calls [`dispatcher::run`] directly rather than shelling out to a binary.

pub mod dispatcher;
pub mod projection;
