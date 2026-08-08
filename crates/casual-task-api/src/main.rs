//! # casual-task-api
//!
//! The deployable API binary. The **only** crate that knows about HTTP.
//!
//! Owns: Axum routers, tower middleware (auth context, request id, rate limit,
//! timeout, compression, tracing), DTOs, the generated OpenAPI document, SSE
//! streams, and the error-to-HTTP mapping (`docs/05-API-SPEC.md`).
//!
//! It is also the only crate permitted to call `AuthContext::authenticated`,
//! which is what makes `WorkspaceScope` unforgeable elsewhere
//! (`docs/32-TENANCY-AND-ISOLATION.md`).
//!
//! Phase 0 scaffold — no routes yet. See `docs/14-EXECUTION-TRACKER.md`.

use std::process::ExitCode;

fn main() -> ExitCode {
    // `deploy/docker-compose.yml` already probes liveness with
    // `taskforge-api --health-check`. Every argument was silently ignored, so
    // that probe passed by doing nothing at all — and a liveness probe that
    // cannot fail is worse than no probe, because it reports healthy straight
    // through an outage and the orchestrator never restarts anything.
    //
    // `/health/live` arrives with the HTTP server (C-001). Until then this
    // refuses rather than lies. Checked before telemetry, because a health
    // probe must not depend on the logger.
    if std::env::args().skip(1).any(|arg| arg == "--health-check") {
        eprintln!(
            "--health-check is not implemented: this is a Phase 0 scaffold with no HTTP \
             server, so there is nothing to report on (docs/46 §Health endpoints, \
             tracker C-001). Refusing to report healthy."
        );
        return ExitCode::FAILURE;
    }

    // First statement in `main`, before anything that might log. An observability
    // crate that no binary installs is a library nobody has run: it means
    // `TF_LOG_FORMAT` is not actually honoured, a bad value is not actually
    // rejected at startup, and the first time any of that is exercised is in
    // production.
    if let Err(error) = casual_task_observability::init() {
        // Deliberately `eprintln!` and not `tracing::error!`: the thing that
        // failed is the logger, so the only reliable destination is stderr.
        eprintln!("failed to initialise telemetry: {error}");
        // docs/48 §Configuration: a misconfigured deployment fails fast and
        // specifically rather than starting with defaults nobody asked for.
        return ExitCode::FAILURE;
    }

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "taskforge api — Phase 0 scaffold, no routes yet (docs/06)"
    );
    ExitCode::SUCCESS
}
