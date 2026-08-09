//! The API binary.
//!
//! Startup order is deliberate and each step is a refusal, not a warning:
//!
//! 1. `--health-check` short-circuits, before telemetry — a liveness probe must
//!    not depend on the logger it is checking.
//! 2. Telemetry, so everything after this is observable.
//! 3. Configuration, which fails fast and specifically (`docs/48`).
//! 4. The database pool, bounded (D-039).
//! 5. **The superuser check.** `docs/48`: "the API refuses to start if
//!    `current_setting('is_superuser')` is on."
//! 6. Bind and serve.

use std::process::ExitCode;
use std::sync::Arc;

use casual_task_api::{AppState, Config};
use casual_task_observability::recorder::Recorder;
use sqlx::postgres::PgPoolOptions;

fn main() -> ExitCode {
    // `deploy/docker-compose.yml` probes liveness with `--health-check`.
    // Handled before anything else can fail: a probe that depends on
    // configuration reports unhealthy during a misconfiguration, which is true
    // but useless — the orchestrator then restarts a container that will never
    // become healthy, forever.
    if std::env::args().skip(1).any(|arg| arg == "--health-check") {
        return health_check();
    }

    // `--version` exists for the image gate, and the gate needed it. That check
    // used to run this binary with NO arguments and require exit 0, which only
    // ever passed because the binary was a scaffold that printed a line and
    // stopped. A server that exits 0 when started with no configuration is not
    // a healthy binary; it is one that failed to notice. `--version` answers
    // what the gate actually wants to know — does this binary execute in this
    // image, on this architecture, with its libraries — and answers nothing
    // else.
    if std::env::args().skip(1).any(|arg| arg == "--version") {
        println!("taskforge-api {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    if let Err(error) = casual_task_observability::init() {
        eprintln!("failed to initialise telemetry: {error}");
        return ExitCode::FAILURE;
    }

    match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime.block_on(run()),
        Err(error) => {
            tracing::error!(%error, "cannot start the async runtime");
            ExitCode::FAILURE
        }
    }
}

/// Probe the live endpoint of an already-running process.
///
/// Deliberately talks HTTP to `/health/live` rather than checking anything
/// itself. A probe that re-derives health from configuration answers a
/// different question than "is the server responding", and the whole point of
/// a liveness probe is the second one.
fn health_check() -> ExitCode {
    let addr = std::env::var("TF_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_owned());
    let port = addr.rsplit(':').next().unwrap_or("8080");
    let url = format!("127.0.0.1:{port}");

    // A bare TCP connect, so the probe needs no HTTP client dependency in the
    // image. It answers "is something accepting connections on the port this
    // process was told to bind", which is what liveness means here — readiness
    // is a separate endpoint with a separate answer.
    match std::net::TcpStream::connect_timeout(
        &match url.parse() {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!("--health-check: cannot parse {url}: {error}");
                return ExitCode::FAILURE;
            }
        },
        std::time::Duration::from_secs(2),
    ) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("--health-check: {url} is not accepting connections: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            // The message names the variable and what goes wrong. This is the
            // entire diagnostic for a container that exits during startup.
            tracing::error!(%error, "configuration is not valid");
            eprintln!("configuration is not valid: {error}");
            return ExitCode::FAILURE;
        }
    };

    let pool = match PgPoolOptions::new()
        .max_connections(config.pool.max_connections)
        .acquire_timeout(config.pool.acquire_timeout)
        .connect(&config.database_url)
        .await
    {
        Ok(pool) => pool,
        Err(error) => {
            tracing::error!(%error, "cannot connect to the database");
            return ExitCode::FAILURE;
        }
    };

    if let Err(reason) = refuse_superuser(&pool).await {
        tracing::error!("{reason}");
        eprintln!("{reason}");
        return ExitCode::FAILURE;
    }

    let listener = match tokio::net::TcpListener::bind(config.bind_addr).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(%error, addr = %config.bind_addr, "cannot bind");
            return ExitCode::FAILURE;
        }
    };

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        addr = %config.bind_addr,
        max_connections = config.pool.max_connections,
        "taskforge api listening"
    );

    // Built BEFORE the listener binds. `docs/48`: "A misconfigured deployment
    // must not start." A relay whose address does not parse is discovered here,
    // by the operator, rather than by the first person who forgets a password.
    let mailer = match casual_task_infra::mail::from_config(&config.smtp) {
        Ok(mailer) => mailer,
        Err(error) => {
            tracing::error!(%error, "the mail transport is not valid");
            eprintln!("TF_SMTP_* is not valid: {error}");
            return ExitCode::FAILURE;
        }
    };
    if !config.smtp.enabled() {
        // Said once, loudly, at startup. `docs/48` makes this a supported
        // deployment, but an operator who set the variables and typoed the
        // name would otherwise learn about it from a support ticket.
        tracing::warn!(
            "TF_SMTP_HOST is empty: email is disabled and password-reset links will not be sent"
        );
    }

    // One hub for the process. The SSE consumer publishes into it and every
    // stream handler subscribes to it, which is only the same hub because both
    // are handed this one value — `docs/48`: fan-out is single-instance until
    // Redis exists.
    let broadcast: Arc<dyn casual_task_infra::broadcast::Broadcast> =
        Arc::new(casual_task_infra::broadcast::LocalBroadcast::new());

    // The backend is chosen ONCE, here, from TF_STORAGE_BACKEND. No handler
    // branches on it again, which is what keeps the single-node profile running
    // the identical handshake S3 would (`docs/28` §Local deployment).
    let storage: std::sync::Arc<dyn casual_task_infra::ObjectStore> = match config.storage.backend {
        casual_task_api::config::StorageBackend::Filesystem => {
            std::sync::Arc::new(casual_task_infra::FilesystemStore::new(
                std::path::PathBuf::from(&config.storage.path),
                config.attachment_origin.clone(),
                config.secret_key.clone(),
            ))
        }
        // Unreachable: `Config::from_source` refuses this value rather than
        // letting a deployment believe its files are in a bucket.
        casual_task_api::config::StorageBackend::S3 => {
            eprintln!("TF_STORAGE_BACKEND=s3 is not implemented");
            return ExitCode::FAILURE;
        }
    };

    let state = AppState {
        pool,
        storage,
        metrics: Arc::new(Recorder::new()),
        secret_key: config.secret_key.clone().into(),
        public_url: config.public_url.clone().into(),
        mailer,
        broadcast,
    };
    // The outbox dispatcher, in this process (D-060, `docs/48` Profile 1).
    //
    // Held for its `Drop`: dropping the handle cancels the loops, so a panic in
    // `serve` below cannot leave orphaned dispatchers polling a database whose
    // API is gone.
    let _worker = start_embedded_worker(&config, &state).await;

    match casual_task_api::serve(listener, state).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "server failed");
            ExitCode::FAILURE
        }
    }
}

/// Start the dispatch loop inside the API process, if this deployment wants it.
///
/// Returns the cancellation handle. `None` when the loop is not running, for
/// one of two reasons, and **both are said out loud**: a silent no-op here is
/// how a deployment ends up with notifications and SSE that work in tests and
/// nowhere else, which is exactly the state D-060 described.
async fn start_embedded_worker(
    config: &casual_task_api::Config,
    state: &AppState,
) -> Option<casual_task_worker::dispatcher::CancelOnDrop> {
    use casual_task_worker::consumers::{NotificationFanout, SseFanout};
    use casual_task_worker::dispatcher::{self, CancelOnDrop};

    if !config.worker_embedded {
        tracing::info!(
            "TF_WORKER_EMBEDDED=false: this process serves requests only; \
             a separate worker must run the dispatch loop"
        );
        return None;
    }
    let Some(dsn) = config.dispatcher_database_url.as_deref() else {
        // Not fatal. An API that serves requests is more useful than one that
        // refuses to start, and the operator is told precisely what is off and
        // what it costs them.
        tracing::warn!(
            "DISPATCHER_DATABASE_URL is not set: the outbox dispatcher is NOT \
             running, so notifications and live updates will not be delivered. \
             It needs the taskforge_dispatcher role (migration 0014), not the \
             application role."
        );
        return None;
    };

    // A small pool of its own. The loop is a handful of connections and must
    // not compete with request serving for the API's.
    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(config.pool.acquire_timeout)
        .connect(dsn)
        .await
    {
        Ok(pool) => pool,
        Err(error) => {
            tracing::error!(%error, "the dispatcher cannot connect; the loop is not running");
            return None;
        }
    };

    // Verified once, here, so a misconfigured DSN is a startup message rather
    // than a loop that claims nothing forever: `claim` polls across tenants and
    // needs a role that bypasses row-level security.
    if let Err(error) = casual_task_persistence::dispatch::DispatcherRole::verify(&mut *match pool
        .acquire()
        .await
    {
        Ok(conn) => conn,
        Err(error) => {
            tracing::error!(%error, "the dispatcher cannot acquire a connection");
            return None;
        }
    })
    .await
    {
        tracing::error!(
            %error,
            "DISPATCHER_DATABASE_URL does not connect as a role that bypasses \
             row-level security; the loop is not running"
        );
        return None;
    }

    let (handle, cancel) = CancelOnDrop::new();
    let worker_id = format!("api-{}", std::process::id());

    // One loop per consumer: each claims its own delivery rows, and a consumer
    // that is slow or failing must not hold up the others (`docs/25`).
    let notification = std::sync::Arc::new(NotificationFanout::new(
        state.pool.clone(),
        std::sync::Arc::clone(&state.mailer),
        config.public_url.clone(),
    ));
    let sse = std::sync::Arc::new(SseFanout::new(std::sync::Arc::clone(&state.broadcast)));
    // Its own pool, as `taskforge_app` and NOT the dispatcher's: the projection
    // writes tenant rows, and the dispatcher role bypasses row-level security
    // and is granted on the two outbox tables and nothing else (migration 0014).
    let search = std::sync::Arc::new(casual_task_worker::projection::SearchProjection::new(
        state.pool.clone(),
    ));

    for (name, spawn) in [
        (
            casual_task_worker::consumers::notification::NAME,
            Loop::Notification(notification),
        ),
        ("sse_fanout", Loop::Sse(sse)),
        // Without this loop the index is never written and search returns
        // nothing — while every task write succeeds and every gate passes,
        // because the projection's own tests drive the consumer directly.
        (casual_task_worker::projection::NAME, Loop::Search(search)),
    ] {
        let pool = pool.clone();
        let cancel = cancel.clone();
        let metrics = std::sync::Arc::clone(&state.metrics);
        let worker_id = worker_id.clone();
        tokio::spawn(async move {
            let outcome = match spawn {
                Loop::Notification(consumer) => {
                    dispatcher::run(
                        &pool,
                        consumer,
                        &worker_id,
                        dispatcher::Config::default(),
                        cancel,
                        metrics,
                    )
                    .await
                }
                Loop::Sse(consumer) => {
                    dispatcher::run(
                        &pool,
                        consumer,
                        &worker_id,
                        dispatcher::Config::default(),
                        cancel,
                        metrics,
                    )
                    .await
                }
                Loop::Search(consumer) => {
                    dispatcher::run(
                        &pool,
                        consumer,
                        &worker_id,
                        dispatcher::Config::default(),
                        cancel,
                        metrics,
                    )
                    .await
                }
            };
            match outcome {
                Ok(stopped) => tracing::info!(consumer = name, ?stopped, "dispatch loop stopped"),
                Err(error) => tracing::error!(%error, consumer = name, "dispatch loop failed"),
            }
        });
    }

    tracing::info!(worker_id, "the embedded outbox dispatcher is running");
    Some(handle)
}

/// The consumers the embedded worker runs. An enum rather than a boxed trait
/// object because `Consumer` takes `self` by reference in an `async fn` and is
/// therefore not dyn-compatible on the pinned toolchain.
enum Loop {
    Notification(std::sync::Arc<casual_task_worker::consumers::NotificationFanout>),
    Sse(std::sync::Arc<casual_task_worker::consumers::SseFanout>),
    Search(std::sync::Arc<casual_task_worker::projection::SearchProjection>),
}

/// Refuse to serve as a superuser (`docs/48`, migration 0012).
///
/// A superuser bypasses **every** row-level security policy unconditionally and
/// is unaffected by the `REVOKE`s that make audit history append-only. Connected
/// as one, the application still works — every request succeeds, every test
/// passes, and tenant isolation and audit immutability are both silently inert.
/// There is no symptom until a customer sees another customer's tasks.
///
/// That is precisely the failure that has to be impossible rather than
/// documented, so it is checked here and the process exits.
async fn refuse_superuser(pool: &sqlx::PgPool) -> Result<(), String> {
    let is_superuser = casual_task_persistence::health::is_superuser(pool)
        .await
        .map_err(|error| format!("cannot determine the connected role: {error}"))?;

    if is_superuser {
        return Err(
            "refusing to start: DATABASE_URL connects as a SUPERUSER. A superuser bypasses every \
             row-level security policy and is unaffected by the REVOKEs that make audit history \
             append-only, so tenant isolation and audit immutability would both be silently \
             inert. Connect as taskforge_app (migration 0012, docs/52)."
                .to_owned(),
        );
    }
    Ok(())
}
