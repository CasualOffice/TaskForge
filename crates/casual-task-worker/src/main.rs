//! # casual-task-worker
//!
//! The worker binary: outbox dispatch, search and state-interval projections,
//! notification fan-out, attachment scanning, and export jobs (`docs/25`,
//! `docs/28`, `docs/38`).
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

use casual_task_worker::attachment_scan::AttachmentScan;
use casual_task_worker::consumers::{NotificationFanout, SseFanout};
use casual_task_worker::dispatcher::{self, Cancel, CancelOnDrop, Consumer};
use casual_task_worker::projection::SearchProjection;
use casual_task_worker::state_interval::StateIntervalProjection;

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
    let search = Arc::new(SearchProjection::new(app_pool.clone()));
    let state_interval = Arc::new(StateIntervalProjection::new(app_pool.clone()));

    // Step 4 of `docs/28`. Built here rather than in the API because the scan
    // is the worker's job and the bytes must never enter a request handler.
    //
    // `TF_CLAMD_ADDR` absent means no scanner, and no scanner means every
    // attachment stays `PENDING` and invisible — D-062, countersigned, and the
    // reason this is `Option` rather than a default that waves files through.
    let scanner: Option<Arc<dyn casual_task_infra::Scanner>> = match std::env::var("TF_CLAMD_ADDR")
    {
        Ok(address) if !address.trim().is_empty() => {
            tracing::info!(%address, "scanning attachments with clamd");
            Some(Arc::new(casual_task_infra::Clamd::new(address)))
        }
        _ => {
            tracing::warn!(
                "TF_CLAMD_ADDR is unset: attachments will be stored and never become visible,                  because nothing can mark them clean (docs/28 step 4, D-062)"
            );
            None
        }
    };
    let object_store = object_store_from_env();
    let attachment_scan = match object_store.as_ref() {
        Some(store) => Some(Arc::new(AttachmentScan::new(
            app_pool.clone(),
            Arc::clone(store),
            scanner,
        ))),
        None => {
            tracing::warn!(
                "no object storage is configured, so attachments cannot be scanned or served"
            );
            None
        }
    };

    let notification_loop = spawn_consumer(
        dispatch_pool.clone(),
        notification,
        worker_id.clone(),
        cancel.clone(),
        Arc::clone(&metrics),
    );
    let sse_loop = spawn_consumer(
        dispatch_pool.clone(),
        sse,
        worker_id.clone(),
        cancel.clone(),
        Arc::clone(&metrics),
    );
    let search_loop = spawn_consumer(
        dispatch_pool.clone(),
        search,
        worker_id.clone(),
        cancel.clone(),
        Arc::clone(&metrics),
    );
    let state_interval_loop = spawn_consumer(
        dispatch_pool.clone(),
        state_interval,
        worker_id.clone(),
        cancel.clone(),
        Arc::clone(&metrics),
    );

    let export_loop = object_store.map(|storage| {
        let dispatch_pool = dispatch_pool.clone();
        let app_pool = app_pool.clone();
        let worker_id = worker_id.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if let Err(error) = casual_task_worker::export::runner::run(
                &dispatch_pool,
                &app_pool,
                storage,
                &worker_id,
                cancel,
            )
            .await
            {
                tracing::error!(%error, %worker_id, "export loop stopped unexpectedly");
            }
        })
    });

    let scan_loop = attachment_scan.map(|consumer| {
        spawn_consumer(dispatch_pool, consumer, worker_id.clone(), cancel, metrics)
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
    let _ = search_loop.await;
    let _ = state_interval_loop.await;
    // Awaited like the others so an in-flight scan finishes rather than being
    // cut off mid-verdict, which would leave an attachment PENDING with the
    // delivery already claimed.
    if let Some(loop_handle) = scan_loop {
        let _ = loop_handle.await;
    }
    if let Some(loop_handle) = export_loop {
        let _ = loop_handle.await;
    }
    ExitCode::SUCCESS
}

fn spawn_consumer<C: Consumer + 'static>(
    pool: sqlx::PgPool,
    consumer: Arc<C>,
    worker_id: String,
    cancel: Cancel,
    metrics: Arc<casual_task_observability::recorder::Recorder>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = dispatcher::run(
            &pool,
            consumer,
            &worker_id,
            dispatcher::Config::default(),
            cancel,
            metrics,
        )
        .await
        {
            tracing::error!(%error, %worker_id, "consumer dispatch loop stopped unexpectedly");
        }
    })
}

/// `TF_SMTP_*`, the same five keys the API reads (`docs/48` §Configuration).
///
/// An empty host disables email, which is a supported deployment.
/// The object store, from `TF_STORAGE_BACKEND` and `TF_STORAGE_PATH`.
///
/// Read here rather than shared with the API's `Config` because the worker has
/// no HTTP configuration and should not have to satisfy it — but the two
/// variables are the same two, so a deployment configures storage once.
fn object_store_from_env() -> Option<std::sync::Arc<dyn casual_task_infra::ObjectStore>> {
    let backend = std::env::var("TF_STORAGE_BACKEND").unwrap_or_default();
    if backend != "fs" {
        return None;
    }
    let path = std::env::var("TF_STORAGE_PATH").ok()?;
    let origin = std::env::var("TF_ATTACHMENT_ORIGIN").ok()?;
    let secret = std::env::var("TF_SECRET_KEY").ok()?;
    Some(std::sync::Arc::new(
        casual_task_infra::FilesystemStore::new(path.into(), origin, secret),
    ))
}

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
