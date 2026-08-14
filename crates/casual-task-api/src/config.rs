//! Configuration, validated at startup (`docs/48` §Configuration).
//!
//! > "Startup validation fails fast and specifically."
//!
//! Every value here is read once, checked once, and then exists as a typed
//! field. Nothing reads an environment variable later: a value that could be
//! missing at request time is a 500 waiting for the first request that needs
//! it, hours after the deploy that caused it.
//!
//! # Refusing to start is the feature
//!
//! `docs/48` requires `TF_ATTACHMENT_ORIGIN` to differ from `TF_PUBLIC_URL`,
//! because sharing an origin lets a stored HTML or SVG attachment execute with
//! access to application cookies (`docs/28`). A deployment that starts with
//! them equal is silently insecure; one that refuses to start is a five-minute
//! problem. The same reasoning covers the missing-secret cases below.

use std::net::SocketAddr;
use std::time::Duration;

/// Why the process will not start.
///
/// Each variant names the variable **and** what goes wrong if it were allowed
/// through, because this text is the entire diagnostic an operator gets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// `TF_STORAGE_BACKEND` names a backend that is not built.
    #[error(
        "TF_STORAGE_BACKEND={0} is not a backend this build has. `fs` is \
         implemented; `s3` is documented in docs/48 and not yet built, and \
         starting with it would silently store files on local disk."
    )]
    UnsupportedStorageBackend(String),
    /// `TF_STORAGE_PATH` is empty with the filesystem backend selected.
    #[error("TF_STORAGE_PATH must be set when TF_STORAGE_BACKEND is `fs`")]
    MissingStoragePath,
    #[error("{0} is required and not set")]
    Missing(&'static str),
    #[error("{name} is not valid: {reason}")]
    Invalid { name: &'static str, reason: String },
    #[error(
        "TF_ATTACHMENT_ORIGIN must not equal TF_PUBLIC_URL. They share an origin, so a stored \
         HTML or SVG attachment would execute with access to application cookies (docs/28)."
    )]
    SharedAttachmentOrigin,
    #[error(
        "TF_SECRET_KEY must be at least {minimum} characters. It binds the CSRF token \
         (ADR-032); a short one is guessable and the binding then proves nothing."
    )]
    WeakSecret { minimum: usize },
}

/// The minimum accepted `TF_SECRET_KEY` length.
///
/// 32 characters. `deploy/.env.example` generates 48 with
/// `openssl rand -base64 48`; the floor is lower so an operator with their own
/// key management is not forced to regenerate, but not so low that a
/// hand-typed value passes.
pub const MIN_SECRET_LEN: usize = 32;

/// Everything the process needs, validated.
#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    /// `TF_OBJECT_BIND_ADDR` — where this process serves the attachment origin.
    ///
    /// `None` when nothing is served here, which is the S3 profile: the bucket
    /// answers the presigned URL and this process never sees a byte of file
    /// content. Set it for the single-node profile, and set it to a **different
    /// port** from `TF_BIND_ADDR` — a different port is a different origin,
    /// which is the whole control `docs/28` §Serving downloads rests on.
    pub object_bind_addr: Option<SocketAddr>,
    pub database_url: String,
    pub public_url: String,
    pub attachment_origin: String,
    pub secret_key: String,
    pub pool: PoolConfig,
    /// `TF_SMTP_*`. An empty host disables email (`docs/48`, D-046).
    pub smtp: casual_task_infra::SmtpConfig,
    /// `TF_STORAGE_*`. Where attachment bytes live (`docs/48`, `docs/28`).
    pub storage: StorageConfig,
    /// `DISPATCHER_DATABASE_URL` — the outbox dispatcher's connection (D-060).
    ///
    /// A **second DSN**, and it has to be: `dispatch::claim` polls across every
    /// tenant, so it runs as a role that bypasses row-level security
    /// (migration 0014), and `DispatcherRole::verify` refuses anything else.
    /// The role serving requests is deliberately not that role — that is the
    /// whole point of migration 0012.
    ///
    /// `None` disables the embedded worker, and the process says so at startup
    /// rather than polling an empty table forever. `deploy/docker-compose.yml`
    /// has set this variable since it was written; nothing read it until now,
    /// which is why the dispatch loop had never run outside a test.
    pub dispatcher_database_url: Option<String>,
    /// `TF_WORKER_EMBEDDED` (`docs/48`, default **true**).
    ///
    /// Profile 1 is one binary: the API process runs the dispatch loop itself.
    /// Setting it false is how Profile 2 moves the loop into its own container
    /// without the API also running one — two dispatchers are not wrong (the
    /// claim is `FOR UPDATE SKIP LOCKED`), but they are twice the polling for
    /// no gain.
    pub worker_embedded: bool,
}

/// Object storage selection (`docs/48` §Configuration).
///
/// Both keys were documented in `docs/48` and `docs/52` and set in
/// `deploy/docker-compose.yml` from the beginning, and **nothing read them** —
/// so a deployment could set `TF_STORAGE_BACKEND=s3`, see it accepted, and get
/// the filesystem. Configuration that is documented and unread is worse than
/// undocumented: it reports success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageConfig {
    /// `fs` or `s3`. `docs/48` defaults it to `fs`.
    pub backend: StorageBackend,
    /// `TF_STORAGE_PATH`, the filesystem root. Meaningful only for `fs`.
    pub path: String,
}

/// The backends `TF_STORAGE_BACKEND` names.
///
/// A closed enum rather than a string: `s3` is **not implemented**, and an
/// operator who asks for it must be refused at startup rather than silently
/// served the filesystem. That is the whole reason this parses instead of
/// defaulting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBackend {
    Filesystem,
    S3,
}

/// Connection pool bounds (D-039).
///
/// A pool is a queue with a fixed number of servers, and both bounds have to be
/// stated or the queue is unbounded in one direction. `docs/30` accepts a 503
/// under saturation; what it does not accept is a request waiting indefinitely
/// for a connection while its client has already given up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolConfig {
    /// Maximum connections.
    pub max_connections: u32,
    /// How long a request waits for one before the answer is **503**, not a
    /// hang. Short on purpose: a caller that has been waiting five seconds for
    /// a connection has usually been abandoned by its client already, and
    /// serving it then costs a connection nobody is waiting for.
    pub acquire_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            // PostgreSQL's own default `max_connections` is 100 and the worker
            // needs some. Leaving headroom here is what stops the API from
            // being the reason the dispatcher cannot connect.
            max_connections: 32,
            acquire_timeout: Duration::from_secs(3),
        }
    }
}

impl Config {
    /// Read and validate from the environment.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] on the first problem, naming the variable.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_source(|key| std::env::var(key).ok())
    }

    /// The same validation, against any source.
    ///
    /// Split out so the rules are testable without mutating process-wide
    /// environment state — which is shared between concurrently running tests
    /// and makes them flaky in a way that looks like a real bug.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] on the first problem.
    pub fn from_source<F>(get: F) -> Result<Self, ConfigError>
    where
        F: Fn(&'static str) -> Option<String>,
    {
        let required = |name: &'static str| get(name).ok_or(ConfigError::Missing(name));

        let bind = get("TF_BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8080".to_owned());
        let bind_addr = bind.parse().map_err(|e| ConfigError::Invalid {
            name: "TF_BIND_ADDR",
            reason: format!("{e} (expected host:port, e.g. 0.0.0.0:8080)"),
        })?;

        let secret_key = required("TF_SECRET_KEY")?;
        if secret_key.len() < MIN_SECRET_LEN {
            return Err(ConfigError::WeakSecret {
                minimum: MIN_SECRET_LEN,
            });
        }

        // Where the attachment origin is served from, when this process serves
        // it. Absent means it is served elsewhere — an S3 bucket, a CDN, a
        // second deployment — and this process binds one listener as before.
        let object_bind_addr = match get("TF_OBJECT_BIND_ADDR") {
            None => None,
            Some(raw) => Some(raw.parse().map_err(|e| ConfigError::Invalid {
                name: "TF_OBJECT_BIND_ADDR",
                reason: format!("{e} (expected host:port, e.g. 0.0.0.0:8081)"),
            })?),
        };

        let public_url = required("TF_PUBLIC_URL")?;
        let attachment_origin = required("TF_ATTACHMENT_ORIGIN")?;
        if origin_of(&attachment_origin) == origin_of(&public_url) {
            return Err(ConfigError::SharedAttachmentOrigin);
        }

        let pool = PoolConfig {
            max_connections: parse_or("TF_DB_MAX_CONNECTIONS", &get, 32)?,
            acquire_timeout: Duration::from_secs(u64::from(parse_or(
                "TF_DB_ACQUIRE_TIMEOUT_SECONDS",
                &get,
                3,
            )?)),
        };

        // `docs/48` §Configuration: "TF_SMTP_HOST/PORT/USER/PASS/FROM — empty
        // host disables email (D-046)". Absent and empty are the same thing
        // here, so a compose file with `TF_SMTP_HOST=` behaves as the operator
        // reading that line expects.
        let smtp = casual_task_infra::SmtpConfig {
            host: get("TF_SMTP_HOST").unwrap_or_default(),
            port: u16::try_from(parse_or(
                "TF_SMTP_PORT",
                &get,
                u32::from(casual_task_infra::SmtpConfig::DEFAULT_PORT),
            )?)
            .map_err(|_| ConfigError::Invalid {
                name: "TF_SMTP_PORT",
                reason: "a TCP port is at most 65535".to_owned(),
            })?,
            user: get("TF_SMTP_USER").unwrap_or_default(),
            password: get("TF_SMTP_PASS").unwrap_or_default(),
            from: get("TF_SMTP_FROM").unwrap_or_default(),
        };
        // A relay with nothing to send *from* is a deployment where the first
        // person to forget their password discovers the misconfiguration.
        // `docs/48`: "A misconfigured deployment must not start."
        if smtp.enabled() && smtp.from.trim().is_empty() {
            return Err(ConfigError::Missing("TF_SMTP_FROM"));
        }

        // `docs/48` defaults the backend to `fs`. An unrecognised value is a
        // refusal, not a fallback: "TF_STORAGE_BACKEND=s4" quietly writing to
        // local disk is the failure this parse exists to prevent.
        let storage = {
            let raw = get("TF_STORAGE_BACKEND").unwrap_or_else(|| "fs".to_owned());
            let backend = match raw.trim().to_ascii_lowercase().as_str() {
                "fs" => StorageBackend::Filesystem,
                other => return Err(ConfigError::UnsupportedStorageBackend(other.to_owned())),
            };
            let path = get("TF_STORAGE_PATH").unwrap_or_else(|| "./data/attachments".to_owned());
            if backend == StorageBackend::Filesystem && path.trim().is_empty() {
                return Err(ConfigError::MissingStoragePath);
            }
            StorageConfig { backend, path }
        };

        Ok(Self {
            bind_addr,
            object_bind_addr,
            database_url: required("DATABASE_URL")?,
            public_url,
            attachment_origin,
            secret_key,
            pool,
            smtp,
            storage,
            // Absent is a supported deployment, not a misconfiguration: an
            // operator who has not created the dispatcher role yet gets an API
            // that serves requests and says the loop is off, rather than one
            // that refuses to start.
            dispatcher_database_url: get("DISPATCHER_DATABASE_URL")
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty()),
            // docs/48 §Configuration: "true | false (default true)".
            worker_embedded: get("TF_WORKER_EMBEDDED")
                .is_none_or(|v| !v.trim().eq_ignore_ascii_case("false")),
        })
    }
}

#[cfg(test)]
#[path = "config_storage_tests.rs"]
mod storage_tests;

fn parse_or<F>(name: &'static str, get: &F, default: u32) -> Result<u32, ConfigError>
where
    F: Fn(&'static str) -> Option<String>,
{
    match get(name) {
        None => Ok(default),
        Some(raw) => raw.parse().map_err(|e| ConfigError::Invalid {
            name,
            reason: format!("{e} (expected a positive integer, got {raw:?})"),
        }),
    }
}

/// Scheme and authority, lowercased — what the browser calls an origin.
///
/// Compared this way rather than by string equality so that a trailing slash,
/// a path, or a difference in case does not make two identical origins look
/// distinct and pass the check that exists to reject them.
fn origin_of(url: &str) -> String {
    let lowered = url.trim().trim_end_matches('/').to_ascii_lowercase();
    match lowered.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{authority}")
        }
        None => lowered,
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
