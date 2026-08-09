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
//! # Two DSNs, and why (D-060)
//!
//! `DISPATCHER_DATABASE_URL` polls the outbox across every tenant, so it
//! connects as a role that **bypasses row-level security** (migration 0014) and
//! `DispatcherRole::verify` refuses anything else. `DATABASE_URL` is the
//! ordinary application role, and it is what the consumers read tenant data
//! with — a notification fan-out that read assignees as the dispatcher would be
//! reading them with RLS switched off.
//!
//! Both are required here. That is the resolution of D-060: this binary could
//! not start at all while `docs/48` named only one connection, because the two
//! roles are deliberately different and neither can do the other's job.

use std::process::ExitCode;
use std::sync::Arc;

use casual_task_worker::consumers::{NotificationFanout, SseFanout};
use casual_task_worker::dispatcher::{self, CancelOnDrop};

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

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("cannot start the async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run())
}

/// Refuse to start rather than run half a worker.
///
/// `docs/48`: "A misconfigured deployment must not start." A worker that came
/// up without a dispatcher DSN would poll nothing forever and look healthy,
/// which is the failure this binary spent Phase 1 in.
fn required(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} is required"))
}

async fn run() -> ExitCode {
    let (app_dsn, dispatcher_dsn, public_url) = match (
        required("DATABASE_URL"),
        required("DISPATCHER_DATABASE_URL"),
        required("TF_PUBLIC_URL"),
    ) {
        (Ok(app), Ok(dispatcher), Ok(public_url)) => (app, dispatcher, public_url),
        (app, dispatcher, public_url) => {
            for missing in [app.err(), dispatcher.err(), public_url.err()]
                .into_iter()
                .flatten()
            {
                eprintln!("configuration is not valid: {missing}");
                tracing::error!(%missing, "configuration is not valid");
            }
            return ExitCode::FAILURE;
        }
    };

    let app_pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&app_dsn)
        .await
    {
        Ok(pool) => pool,
        Err(error) => {
            tracing::error!(%error, "cannot connect as the application role");
            return ExitCode::FAILURE;
        }
    };
    let dispatch_pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&dispatcher_dsn)
        .await
    {
        Ok(pool) => pool,
        Err(error) => {
            tracing::error!(%error, "cannot connect as the dispatcher role");
            return ExitCode::FAILURE;
        }
    };

    // Proven once, at startup. A DSN that does not bypass RLS claims nothing
    // and would otherwise look like an idle worker rather than a broken one.
    match dispatch_pool.acquire().await {
        Ok(mut conn) => {
            if let Err(error) =
                casual_task_persistence::dispatch::DispatcherRole::verify(&mut conn).await
            {
                tracing::error!(
                    %error,
                    "DISPATCHER_DATABASE_URL does not connect as a role that bypasses \
                     row-level security (migration 0014)"
                );
                return ExitCode::FAILURE;
            }
        }
        Err(error) => {
            tracing::error!(%error, "the dispatcher pool has no connections");
            return ExitCode::FAILURE;
        }
    }

    let metrics = Arc::new(casual_task_observability::recorder::Recorder::new());
    let mailer = match casual_task_infra::mail::from_config(&smtp_from_env()) {
        Ok(mailer) => mailer,
        Err(error) => {
            tracing::error!(%error, "the mail transport is not valid");
            return ExitCode::FAILURE;
        }
    };

    // A local hub. In a separate worker process, SSE fan-out publishes to
    // subscribers that are in the *API* process — which this cannot reach
    // without Redis. `docs/48` already requires Redis at ≥ 2 processes for
    // exactly this; said out loud so an operator running Profile 2 does not
    // conclude live updates are broken.
    tracing::warn!(
        "running as a separate process: SSE fan-out is delivered into this \
         process's hub and will not reach browsers connected to the API until \
         a shared broadcast exists (docs/48)"
    );
    let broadcast: Arc<dyn casual_task_infra::broadcast::Broadcast> =
        Arc::new(casual_task_infra::broadcast::LocalBroadcast::new());

    let (handle, cancel) = CancelOnDrop::new();
    let worker_id = format!("worker-{}", std::process::id());
    let notification = Arc::new(NotificationFanout::new(
        app_pool.clone(),
        Arc::clone(&mailer),
        public_url,
    ));
    let sse = Arc::new(SseFanout::new(Arc::clone(&broadcast)));

    let notification_loop = tokio::spawn({
        let (pool, cancel, metrics, id) = (
            dispatch_pool.clone(),
            cancel.clone(),
            Arc::clone(&metrics),
            worker_id.clone(),
        );
        async move {
            dispatcher::run(
                &pool,
                notification,
                &id,
                dispatcher::Config::default(),
                cancel,
                metrics,
            )
            .await
        }
    });
    let sse_loop = tokio::spawn({
        let (pool, cancel, metrics, id) = (dispatch_pool, cancel, metrics, worker_id.clone());
        async move {
            dispatcher::run(
                &pool,
                sse,
                &id,
                dispatcher::Config::default(),
                cancel,
                metrics,
            )
            .await
        }
    });

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        worker_id,
        "taskforge worker running"
    );

    // SIGTERM stops the loops and lets in-flight deliveries drain, which is
    // what `CancelOnDrop` is for (D-041).
    shutdown_signal().await;
    tracing::info!("shutting down");
    drop(handle);
    let _ = notification_loop.await;
    let _ = sse_loop.await;
    ExitCode::SUCCESS
}

/// `TF_SMTP_*`, the same five keys the API reads (`docs/48` §Configuration).
///
/// An empty host disables email, which is a supported deployment.
fn smtp_from_env() -> casual_task_infra::SmtpConfig {
    let get = |name: &str| std::env::var(name).unwrap_or_default();
    casual_task_infra::SmtpConfig {
        host: get("TF_SMTP_HOST"),
        port: get("TF_SMTP_PORT")
            .parse()
            .unwrap_or(casual_task_infra::SmtpConfig::DEFAULT_PORT),
        user: get("TF_SMTP_USER"),
        password: get("TF_SMTP_PASS"),
        from: get("TF_SMTP_FROM"),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGTERM; shutdown will be abrupt");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
