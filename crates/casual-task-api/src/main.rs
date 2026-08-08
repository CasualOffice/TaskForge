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

    let state = AppState {
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: config.secret_key.clone().into(),
        public_url: config.public_url.clone().into(),
        mailer,
    };
    match casual_task_api::serve(listener, state).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "server failed");
            ExitCode::FAILURE
        }
    }
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
