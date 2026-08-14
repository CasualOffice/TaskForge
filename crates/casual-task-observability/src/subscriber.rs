//! The tracing subscriber: structured JSON in production, pretty in development.
//!
//! `docs/46-OBSERVABILITY-AND-OPERATIONS.md` §The three signals asks for
//! structured JSON, one line per request plus explicit events, every line
//! carrying `request_id`, `correlation_id`, `workspace_id`, and `actor_id` —
//! which [`CorrelationContext::span`](crate::CorrelationContext::span) supplies.
//!
//! `docs/48-DEPLOYMENT-PROFILES.md` §Configuration defines the switch:
//! `TF_LOG_FORMAT = json | pretty`.
//!
//! **The default is JSON**, not pretty. A deployment that forgets the variable
//! gets machine-parseable logs; a developer who wants the readable form opts in.
//! The reverse default fails in the direction that matters during an incident.
//!
//! Verbosity comes from `RUST_LOG` through
//! [`EnvFilter`] — the ecosystem default rather
//! than a new variable, because `docs/48` does not define one and inventing
//! configuration keys is not this task's to do.
//!
//! ## Not implemented
//!
//! `docs/48` also defines `TF_OTEL_ENDPOINT`, and `docs/46` specifies OTLP
//! traces with tail sampling (1% baseline, 100% for errors and slow requests,
//! 100% for a named workspace under investigation). None of that is here: no
//! `opentelemetry` crate is a workspace dependency. Log lines therefore carry
//! `request_id` and `correlation_id` but not `trace_id`.

use std::env;
use std::str::FromStr;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt, registry};

/// The environment variable that selects the log format (`docs/48`).
pub const LOG_FORMAT_ENV: &str = "TF_LOG_FORMAT";

/// The environment variable that selects verbosity.
pub const LOG_FILTER_ENV: &str = "RUST_LOG";

/// The filter applied when [`LOG_FILTER_ENV`] is unset.
pub const DEFAULT_FILTER: &str = "info";

/// How log lines are rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// One JSON object per line, for a log pipeline. The production default.
    #[default]
    Json,
    /// Human-readable, multi-line, coloured. Development only.
    Pretty,
}

impl LogFormat {
    /// Parse an already-read environment value. `None` (variable unset) yields
    /// the default.
    ///
    /// Separate from [`Self::from_env`] because it is pure, so the parsing rules
    /// are testable without mutating process environment — which is `unsafe` in
    /// Rust 2024 and forbidden by the workspace lints.
    ///
    /// # Errors
    ///
    /// [`LogFormatParseError`] for any value other than `json` or `pretty`.
    /// `docs/48` §Configuration requires startup validation to fail fast and
    /// specifically rather than fall back to a default the operator did not ask
    /// for.
    pub fn from_env_value(value: Option<&str>) -> Result<Self, LogFormatParseError> {
        match value {
            None => Ok(Self::default()),
            Some(raw) => raw.parse(),
        }
    }

    /// Read [`LOG_FORMAT_ENV`].
    ///
    /// # Errors
    ///
    /// [`LogFormatParseError`] if the variable is set to an unrecognized value.
    pub fn from_env() -> Result<Self, LogFormatParseError> {
        let raw = env::var(LOG_FORMAT_ENV).ok();
        Self::from_env_value(raw.as_deref())
    }

    /// The canonical spelling, as `docs/48` writes it.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Pretty => "pretty",
        }
    }
}

impl FromStr for LogFormat {
    type Err = LogFormatParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "pretty" => Ok(Self::Pretty),
            _ => Err(LogFormatParseError {
                value: value.to_owned(),
            }),
        }
    }
}

impl std::fmt::Display for LogFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `TF_LOG_FORMAT` was set to something that is neither `json` nor `pretty`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{LOG_FORMAT_ENV}=`{value}` is not recognized; expected `json` or `pretty` (docs/48)")]
pub struct LogFormatParseError {
    /// What the variable was set to.
    pub value: String,
}

/// Everything the subscriber needs, resolved from the environment once at
/// startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryConfig {
    /// How lines are rendered.
    pub format: LogFormat,
    /// An [`EnvFilter`] directive string, e.g. `info,casual_task_authz=debug`.
    pub filter: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            format: LogFormat::default(),
            filter: DEFAULT_FILTER.to_owned(),
        }
    }
}

impl TelemetryConfig {
    /// Resolve from [`LOG_FORMAT_ENV`] and [`LOG_FILTER_ENV`].
    ///
    /// # Errors
    ///
    /// [`TelemetryError::Format`] if `TF_LOG_FORMAT` is unrecognized. A bad
    /// filter directive is *not* an error here — it is caught by
    /// [`init_with`], which is where it can be reported with the directive text.
    pub fn from_env() -> Result<Self, TelemetryError> {
        Ok(Self {
            format: LogFormat::from_env()?,
            filter: env::var(LOG_FILTER_ENV).unwrap_or_else(|_| DEFAULT_FILTER.to_owned()),
        })
    }
}

/// Why the subscriber could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// `TF_LOG_FORMAT` was unrecognized.
    #[error(transparent)]
    Format(#[from] LogFormatParseError),

    /// The filter directive string did not parse.
    #[error("invalid {LOG_FILTER_ENV} directive `{directive}`: {source}")]
    Filter {
        /// The directive string that failed.
        directive: String,
        /// The parse failure.
        source: tracing_subscriber::filter::ParseError,
    },

    /// A global subscriber was already installed.
    ///
    /// Installation is process-global and once-only; a second call is a bug in
    /// startup ordering, not a condition to recover from.
    #[error("a tracing subscriber is already installed: {0}")]
    AlreadyInstalled(String),
}

/// Install the subscriber using the environment.
///
/// Call once, first thing in `main`, before anything that might log.
///
/// # Errors
///
/// See [`TelemetryError`].
pub fn init() -> Result<(), TelemetryError> {
    init_with(&TelemetryConfig::from_env()?)
}

/// Install the subscriber from an explicit configuration.
///
/// # Errors
///
/// See [`TelemetryError`].
pub fn init_with(config: &TelemetryConfig) -> Result<(), TelemetryError> {
    build(config, std::io::stdout)?
        .try_init()
        .map_err(|e| TelemetryError::AlreadyInstalled(e.to_string()))
}

/// Assemble the subscriber without installing it.
///
/// Split out from [`init_with`] so tests can drive the **same** layer stack over
/// a buffer through `tracing::subscriber::with_default`. Testing a hand-built
/// copy of the stack would assert nothing about what production emits.
///
/// Not public: the writer is an implementation detail of the two binaries, and
/// exposing it would invite a second logging destination that `docs/46` does not
/// describe.
fn build<W>(
    config: &TelemetryConfig,
    writer: W,
) -> Result<impl tracing::Subscriber + Send + Sync + 'static, TelemetryError>
where
    W: for<'w> fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    let filter = EnvFilter::try_new(&config.filter).map_err(|source| TelemetryError::Filter {
        directive: config.filter.clone(),
        source,
    })?;

    let layer = match config.format {
        LogFormat::Json => fmt::layer()
            .json()
            // Event fields stay under `fields`, NOT at the top level.
            //
            // Flattening them was intended to let a pipeline index
            // `correlation_id` without a nested path, which it never did —
            // `correlation_id` is a *span* field, so flattening the event was
            // always the wrong lever for it. What flattening did do was write
            // event fields into the same JSON object as the formatter's own
            // reserved keys, so `info!(message = ..)`, `info!(level = ..)`, or
            // `info!(target = ..)` emitted a duplicate key. Nearly every
            // consumer takes the last value, which means a field innocently
            // named `message` or `level` silently rewrites the line's severity —
            // and docs/46 §Alerts fires on level-derived conditions.
            //
            // Nesting confines that collision to `fields` and costs one path
            // segment in a query.
            .flatten_event(false)
            .with_current_span(true)
            // The ancestor list is REQUIRED, not a luxury.
            //
            // The correlation fields live on the `unit_of_work` span, and
            // `with_current_span` emits only the innermost span. docs/46
            // §Traces specifies child spans for authorization, for each database
            // query, and for each external call — so under the previous
            // `with_span_list(false)` every line emitted inside any of those
            // carried `"span":{"name":"db.query"}` and no correlation id at all.
            // That is the majority of lines the system will emit, and it
            // defeats docs/46 §Correlation, whose whole claim is that one query
            // on a correlation id reconstructs the causal chain.
            //
            // The cost is real and was the original reason for `false`: the
            // ancestor chain repeats on every line. It is the cheaper mistake.
            //
            // This puts the ids under `spans[]`, not at the top level. Lifting
            // them out needs a custom `FormatEvent` that walks the span scope,
            // which is worth doing when there is a pipeline to satisfy.
            .with_span_list(true)
            .with_writer(writer)
            .boxed(),
        // No ANSI: a developer piping to a file or a pager should not get
        // escape codes.
        LogFormat::Pretty => fmt::layer()
            .pretty()
            .with_ansi(false)
            .with_writer(writer)
            .boxed(),
    };

    Ok(registry().with(filter).with(layer))
}

#[cfg(test)]
#[path = "subscriber_tests.rs"]
mod tests;
