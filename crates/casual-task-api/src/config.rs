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
mod storage_tests {
    use super::*;

    fn with(extra: &[(&'static str, &'static str)]) -> Result<Config, ConfigError> {
        let base: Vec<(&'static str, &'static str)> = vec![
            ("DATABASE_URL", "postgres://localhost/tf"),
            ("TF_SECRET_KEY", "a-secret-key-long-enough-for-the-check"),
            ("TF_PUBLIC_URL", "https://tasks.example.com"),
            ("TF_ATTACHMENT_ORIGIN", "https://files.example.com"),
        ];
        Config::from_source(move |name| {
            extra
                .iter()
                .chain(base.iter())
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn the_backend_defaults_to_the_filesystem() {
        // docs/48: "TF_STORAGE_BACKEND fs | s3 (default fs)".
        let config = with(&[]).expect("defaults are valid");
        assert_eq!(config.storage.backend, StorageBackend::Filesystem);
        assert_eq!(config.storage.path, "./data/attachments");
    }

    #[test]
    fn a_backend_this_build_does_not_have_refuses_to_start() {
        // The failure this parse exists for: `s3` is documented and not built,
        // so accepting it would store files on local disk while the operator
        // believed they were in a bucket.
        assert!(matches!(
            with(&[("TF_STORAGE_BACKEND", "s3")]).err(),
            Some(ConfigError::UnsupportedStorageBackend(_))
        ));
        assert!(with(&[("TF_STORAGE_BACKEND", "nonsense")]).is_err());
    }

    #[test]
    fn an_empty_path_with_the_filesystem_backend_refuses_to_start() {
        assert!(matches!(
            with(&[("TF_STORAGE_PATH", "   ")]).err(),
            Some(ConfigError::MissingStoragePath)
        ));
    }

    #[test]
    fn the_documented_spelling_is_accepted_case_insensitively() {
        assert!(with(&[("TF_STORAGE_BACKEND", "FS")]).is_ok());
        assert!(with(&[("TF_STORAGE_BACKEND", " fs ")]).is_ok());
    }
}

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
mod tests {
    use super::*;

    fn valid() -> Vec<(&'static str, &'static str)> {
        vec![
            ("DATABASE_URL", "postgres://app:pw@localhost/tf"),
            ("TF_PUBLIC_URL", "https://tasks.example.com"),
            ("TF_ATTACHMENT_ORIGIN", "https://files.example.com"),
            ("TF_SECRET_KEY", "0123456789012345678901234567890123"),
        ]
    }

    fn with(overrides: &[(&'static str, &'static str)]) -> Result<Config, ConfigError> {
        let mut env = valid();
        for (key, value) in overrides {
            env.retain(|(k, _)| k != key);
            if !value.is_empty() {
                env.push((key, value));
            }
        }
        Config::from_source(|name| {
            env.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn a_complete_environment_is_accepted() {
        let config = with(&[]).expect("valid");
        assert_eq!(config.bind_addr.port(), 8080);
        assert_eq!(config.pool.max_connections, 32);
    }

    #[test]
    fn a_missing_required_variable_names_itself() {
        // The error text is the entire diagnostic an operator gets from a
        // container that exited before writing a log line.
        for name in [
            "DATABASE_URL",
            "TF_PUBLIC_URL",
            "TF_ATTACHMENT_ORIGIN",
            "TF_SECRET_KEY",
        ] {
            assert_eq!(with(&[(name, "")]).err(), Some(ConfigError::Missing(name)));
        }
    }

    #[test]
    fn a_shared_attachment_origin_refuses_to_start() {
        // docs/28: sharing the origin means a stored HTML or SVG attachment
        // executes with access to application cookies. Starting is worse than
        // not starting.
        assert_eq!(
            with(&[("TF_ATTACHMENT_ORIGIN", "https://tasks.example.com")]).err(),
            Some(ConfigError::SharedAttachmentOrigin)
        );
    }

    #[test]
    fn a_shared_origin_is_caught_through_cosmetic_differences() {
        // The check compares origins, not strings. A trailing slash, a path, or
        // different case would otherwise let the exact misconfiguration it
        // exists to reject sail through.
        for disguise in [
            "https://tasks.example.com/",
            "https://TASKS.example.com",
            "https://tasks.example.com/files",
            "  https://tasks.example.com  ",
        ] {
            assert_eq!(
                with(&[("TF_ATTACHMENT_ORIGIN", disguise)]).err(),
                Some(ConfigError::SharedAttachmentOrigin),
                "{disguise} was accepted as a distinct origin"
            );
        }
    }

    #[test]
    fn a_different_host_is_accepted() {
        // And the check must not be so eager that a correct deployment fails.
        assert!(with(&[("TF_ATTACHMENT_ORIGIN", "https://files.example.com/x")]).is_ok());
        assert!(with(&[("TF_ATTACHMENT_ORIGIN", "https://tasks.example.com:9000")]).is_ok());
    }

    #[test]
    fn a_short_secret_refuses_to_start() {
        assert_eq!(
            with(&[("TF_SECRET_KEY", "too-short")]).err(),
            Some(ConfigError::WeakSecret {
                minimum: MIN_SECRET_LEN
            })
        );
    }

    #[test]
    fn a_malformed_bind_address_says_what_was_expected() {
        let error = with(&[("TF_BIND_ADDR", "8080")]).expect_err("rejected");
        let ConfigError::Invalid { name, reason } = error else {
            panic!("wrong variant")
        };
        assert_eq!(name, "TF_BIND_ADDR");
        assert!(reason.contains("host:port"), "{reason}");
    }

    #[test]
    fn pool_bounds_are_configurable_and_bounded_by_default() {
        // D-039: both bounds stated. A default acquire timeout of "forever"
        // would make the 503 path unreachable.
        let default = PoolConfig::default();
        assert!(default.max_connections > 0);
        assert!(default.acquire_timeout > Duration::ZERO);
        assert!(
            default.acquire_timeout <= Duration::from_secs(5),
            "a caller waiting this long has usually been abandoned by its client"
        );

        let config = with(&[
            ("TF_DB_MAX_CONNECTIONS", "8"),
            ("TF_DB_ACQUIRE_TIMEOUT_SECONDS", "1"),
        ])
        .expect("valid");
        assert_eq!(config.pool.max_connections, 8);
        assert_eq!(config.pool.acquire_timeout, Duration::from_secs(1));
    }

    #[test]
    fn email_is_disabled_by_default_and_that_is_not_an_error() {
        // docs/48: an empty host disables email. Profile 1 is a single node
        // with no relay, and it has to start.
        let config = with(&[]).expect("valid");
        assert!(!config.smtp.enabled());
        assert_eq!(config.smtp.port, 587, "the STARTTLS submission port");
    }

    #[test]
    fn a_relay_without_a_sender_refuses_to_start() {
        // The alternative is a deployment that looks configured and fails on
        // the first password reset — found by the user, not by the operator.
        assert_eq!(
            with(&[("TF_SMTP_HOST", "smtp.example.com")]).err(),
            Some(ConfigError::Missing("TF_SMTP_FROM"))
        );
        assert!(
            with(&[
                ("TF_SMTP_HOST", "smtp.example.com"),
                ("TF_SMTP_FROM", "noreply@example.com"),
            ])
            .is_ok()
        );
    }

    #[test]
    fn a_port_outside_the_tcp_range_is_refused() {
        // 70000 parses as a u32 and truncates to 4464 as a u16. A relay quietly
        // contacted on the wrong port is worse than a refusal to start.
        let error = with(&[("TF_SMTP_PORT", "70000")]).expect_err("rejected");
        assert!(matches!(
            error,
            ConfigError::Invalid {
                name: "TF_SMTP_PORT",
                ..
            }
        ));
    }

    #[test]
    fn a_non_numeric_pool_bound_is_refused_rather_than_defaulted() {
        // Falling back to the default on a typo would mean a deployment that
        // asked for 4 connections silently gets 32.
        let error = with(&[("TF_DB_MAX_CONNECTIONS", "lots")]).expect_err("rejected");
        assert!(matches!(
            error,
            ConfigError::Invalid {
                name: "TF_DB_MAX_CONNECTIONS",
                ..
            }
        ));
    }
}
