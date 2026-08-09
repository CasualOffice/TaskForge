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
//! # Two DSNs, two roles, and why the split is not optional
//!
//! The dispatch loop polls `outbox_delivery` **across tenants**, which needs
//! `taskforge_dispatcher` — a role that bypasses row-level security and is
//! granted on the two outbox tables and nothing else (migration 0014).
//!
//! The search projection reads `task` and writes `task_search`, which that role
//! cannot touch and must not be able to: giving a `BYPASSRLS` role the task
//! tables would hand it every tenant's task text. So the consumer carries a
//! second pool as `taskforge_app`, the ordinary request-serving role, and every
//! statement it issues is tenant-scoped exactly as a request's would be.
//!
//! A deployment that sets only one of the two is refused at startup rather than
//! running half a worker.

use std::process::ExitCode;
use std::sync::Arc;

use casual_task_observability::recorder::Recorder;
use casual_task_worker::dispatcher::{self, CancelOnDrop, Config};
use casual_task_worker::projection::SearchProjection;

/// The application role's DSN — the same `TF_DATABASE_URL` the API uses.
const APP_DSN: &str = "TF_DATABASE_URL";
/// The dispatcher role's DSN. Separate because it is a separate role
/// (migration 0014), not a separate database.
const DISPATCHER_DSN: &str = "TF_DISPATCHER_DATABASE_URL";

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

    let (Ok(app_dsn), Ok(dispatcher_dsn)) = (std::env::var(APP_DSN), std::env::var(DISPATCHER_DSN))
    else {
        // Loud, and specific about which half is missing. A worker that
        // started anyway would poll nothing and report healthy, which is the
        // failure mode `DispatcherRole` exists to make impossible.
        eprintln!(
            "both {APP_DSN} and {DISPATCHER_DSN} are required: the dispatch loop \
             runs as taskforge_dispatcher and the search projection as \
             taskforge_app (migration 0014, docs/48)"
        );
        return ExitCode::FAILURE;
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to start the async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };

    runtime.block_on(async move {
        match serve(&app_dsn, &dispatcher_dsn).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                tracing::error!(%error, "worker stopped");
                ExitCode::FAILURE
            }
        }
    })
}

/// Connect both pools and run the consumers until `SIGTERM`.
async fn serve(app_dsn: &str, dispatcher_dsn: &str) -> Result<(), sqlx::Error> {
    let app = sqlx::PgPool::connect(app_dsn).await?;
    let dispatch = sqlx::PgPool::connect(dispatcher_dsn).await?;
    let metrics = Arc::new(Recorder::new());
    let worker_id = format!(
        "{}-{}",
        hostname().unwrap_or_else(|| "worker".to_owned()),
        std::process::id()
    );

    // The trigger is held by the signal task; dropping it also cancels, so a
    // panicking supervisor cannot orphan the loop.
    let (trigger, cancel) = CancelOnDrop::new();
    tokio::spawn(async move {
        shutdown().await;
        tracing::info!("SIGTERM — draining");
        trigger.cancel();
    });

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        worker = worker_id,
        consumer = casual_task_worker::projection::NAME,
        "taskforge worker starting"
    );

    // One consumer so far. The other five `docs/25` names arrive with C-015 and
    // C-016, and each is another `run` on this same loop.
    dispatcher::run(
        &dispatch,
        Arc::new(SearchProjection::new(app)),
        &worker_id,
        Config::default(),
        cancel,
        metrics,
    )
    .await?;
    Ok(())
}

fn hostname() -> Option<String> {
    std::env::var("HOSTNAME").ok().filter(|h| !h.is_empty())
}

#[cfg(unix)]
async fn shutdown() {
    let mut terminate =
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGTERM; shutdown will be abrupt");
                std::future::pending::<()>().await;
                return;
            }
        };
    tokio::select! {
        _ = terminate.recv() => {},
        result = tokio::signal::ctrl_c() => { let _ = result; },
    }
}

#[cfg(not(unix))]
async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
