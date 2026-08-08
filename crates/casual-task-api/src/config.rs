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

        Ok(Self {
            bind_addr,
            database_url: required("DATABASE_URL")?,
            public_url,
            attachment_origin,
            secret_key,
            pool,
        })
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
