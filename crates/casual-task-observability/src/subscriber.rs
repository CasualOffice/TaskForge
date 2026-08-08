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
            // The full ancestor list repeats the correlation fields on every
            // line at several times the byte cost; the current span carries
            // them already.
            .with_span_list(false)
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
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{CorrelationContext, Redacted};

    /// A log destination a test can read back.
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Buffer {
        fn contents(&self) -> String {
            String::from_utf8(
                self.0
                    .lock()
                    .expect("no test panics while holding it")
                    .clone(),
            )
            .expect("the formatters emit UTF-8")
        }
    }

    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| io::Error::other("poisoned test buffer"))?
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'w> fmt::MakeWriter<'w> for Buffer {
        type Writer = Self;

        fn make_writer(&'w self) -> Self::Writer {
            self.clone()
        }
    }

    /// Emit one log line through the real layer stack and return it.
    fn capture(format: LogFormat, emit: impl FnOnce()) -> String {
        let buffer = Buffer::default();
        let config = TelemetryConfig {
            format,
            filter: "trace".to_owned(),
        };
        let subscriber = build(&config, buffer.clone()).expect("`trace` is a valid directive");
        tracing::subscriber::with_default(subscriber, emit);
        buffer.contents()
    }

    #[test]
    fn the_default_is_json_not_pretty() {
        // A deployment that forgets TF_LOG_FORMAT must still emit parseable
        // logs; pretty is the opt-in.
        assert_eq!(LogFormat::default(), LogFormat::Json);
        assert_eq!(LogFormat::from_env_value(None), Ok(LogFormat::Json));
        assert_eq!(TelemetryConfig::default().format, LogFormat::Json);
        assert_eq!(TelemetryConfig::default().filter, DEFAULT_FILTER);
    }

    #[test]
    fn both_documented_values_parse() {
        // The two spellings in docs/48 §Configuration.
        assert_eq!(LogFormat::from_env_value(Some("json")), Ok(LogFormat::Json));
        assert_eq!(
            LogFormat::from_env_value(Some("pretty")),
            Ok(LogFormat::Pretty)
        );
        // Tolerated on input, because an operator typing PRETTY in a compose
        // file meant pretty; the canonical spelling is still lowercase.
        assert_eq!(
            LogFormat::from_env_value(Some("  PRETTY ")),
            Ok(LogFormat::Pretty)
        );
        assert_eq!(LogFormat::Pretty.as_str(), "pretty");
        assert_eq!(LogFormat::Json.to_string(), "json");
    }

    #[test]
    fn an_unrecognized_value_fails_rather_than_defaulting() {
        // docs/48: "a misconfigured deployment must not start."
        let err = LogFormat::from_env_value(Some("logfmt")).expect_err("logfmt is not supported");
        assert_eq!(err.value, "logfmt");
        let message = err.to_string();
        assert!(message.contains(LOG_FORMAT_ENV), "{message}");
        assert!(
            message.contains("json") && message.contains("pretty"),
            "{message}"
        );
    }

    #[test]
    fn a_bad_filter_directive_is_reported_with_its_text() {
        let config = TelemetryConfig {
            format: LogFormat::Json,
            filter: "info,casual_task_authz=chatty".to_owned(),
        };
        let err = init_with(&config).expect_err("`chatty` is not a level");
        assert!(matches!(err, TelemetryError::Filter { .. }));
        assert!(err.to_string().contains("chatty"), "{err}");
    }

    #[test]
    fn json_lines_carry_the_correlation_fields() {
        // docs/46 §The three signals: every line carries request_id,
        // correlation_id, workspace_id, actor_id.
        let context = CorrelationContext::at_edge()
            .with_workspace(casual_task_model::WorkspaceId::new())
            .with_actor(casual_task_model::UserId::new());

        let output = capture(LogFormat::Json, || {
            let span = context.span("task.transition");
            let _entered = span.enter();
            tracing::info!(task_id = 42, "status changed");
        });

        let line: serde_json::Value = serde_json::from_str(output.trim())
            .unwrap_or_else(|e| panic!("not JSON: {e}\n{output}"));
        let span_fields = &line["span"];
        assert_eq!(
            span_fields["correlation_id"],
            serde_json::json!(context.correlation_id().to_string())
        );
        assert_eq!(
            span_fields["request_id"],
            serde_json::json!(context.request_id().to_string())
        );
        assert_eq!(
            span_fields["workspace_id"],
            serde_json::json!(context.workspace_id().expect("set above").to_string())
        );
        assert_eq!(
            span_fields["actor_id"],
            serde_json::json!(context.actor_id().expect("set above").to_string())
        );
        // The event's own fields are nested under `fields`, away from the
        // formatter's reserved keys.
        assert_eq!(line["fields"]["task_id"], serde_json::json!(42));
        assert_eq!(
            line["fields"]["message"],
            serde_json::json!("status changed")
        );
    }

    #[test]
    fn an_event_field_cannot_shadow_the_line_level() {
        // A field named `level`, `message`, or `target` is entirely plausible
        // in product code — a notification has a `message`, an audit entry has
        // a `target`. Flattened, each emitted a *duplicate* JSON key, and a
        // consumer taking the last value would read the line's severity as
        // whatever the field happened to say. docs/46 §Alerts fires on
        // level-derived conditions, so that is an alerting failure, not a
        // cosmetic one.
        let output = capture(LogFormat::Json, || {
            tracing::info!(
                level = "SHADOW",
                target = "SHADOW",
                message = "SHADOW",
                "real"
            );
        });

        // `serde_json` resolves duplicate keys last-wins, so parsing catches the
        // flattened case directly: the line's `level` would come back "SHADOW".
        let line: serde_json::Value = serde_json::from_str(output.trim()).expect("JSON");
        assert_eq!(line["level"], serde_json::json!("INFO"), "{output}");
        assert_eq!(
            line["target"],
            serde_json::json!(module_path!()),
            "{output}"
        );
        // And the shadowing values are still present, nested where they belong.
        assert_eq!(line["fields"]["level"], serde_json::json!("SHADOW"));
        assert_eq!(line["fields"]["message"], serde_json::json!("SHADOW"));

        // Not every consumer is last-wins; some take the first value for a
        // duplicated key. Assert on the raw bytes that the reserved `level`
        // comes first, so the line reads as INFO under either rule.
        let reserved = output.find("\"level\":\"INFO\"").expect("reserved level");
        let shadowed = output.find("\"level\":\"SHADOW\"").expect("shadowed level");
        assert!(
            reserved < shadowed,
            "the event's `level` precedes the line's own:\n{output}"
        );
    }

    #[test]
    fn redacted_content_does_not_reach_either_format() {
        // The end-to-end version of the docs/46 §What is not logged rule: it is
        // asserted against bytes the formatter actually produced, not against
        // Display in isolation.
        const TITLE: &str = "Acme Corp Q3 layoff plan";

        for format in [LogFormat::Json, LogFormat::Pretty] {
            let output = capture(format, || {
                let title = Redacted::new(TITLE.to_owned());
                tracing::info!(task_id = 42, task_title = %title, "task created");
            });
            assert!(
                !output.contains(TITLE),
                "customer content reached the {format} log: {output}"
            );
            assert!(
                output.contains(crate::redact::PLACEHOLDER),
                "the placeholder should be visible so the mistake is obvious: {output}"
            );
            assert!(output.contains("42"), "ids are still logged: {output}");
        }
    }

    #[test]
    fn the_filter_suppresses_below_its_level() {
        let buffer = Buffer::default();
        let config = TelemetryConfig {
            format: LogFormat::Json,
            filter: "warn".to_owned(),
        };
        let subscriber = build(&config, buffer.clone()).expect("`warn` is a valid directive");
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("dropped");
            tracing::warn!("kept");
        });
        let output = buffer.contents();
        assert!(!output.contains("dropped"), "{output}");
        assert!(output.contains("kept"), "{output}");
    }
}
