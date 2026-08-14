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

    let line: serde_json::Value =
        serde_json::from_str(output.trim()).unwrap_or_else(|e| panic!("not JSON: {e}\n{output}"));
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
fn correlation_survives_a_nested_span() {
    // The regression this guards is the common case, not an edge case:
    // docs/46 §Traces puts a child span around authorization, around every
    // database query, and around every external call, so most log lines are
    // emitted from inside one. Emitting only the *current* span dropped the
    // correlation fields from all of them, and the existing test could not
    // see it because it logged directly inside `unit_of_work`.
    let context = CorrelationContext::at_edge()
        .with_workspace(casual_task_model::WorkspaceId::new())
        .with_actor(casual_task_model::UserId::new());

    let output = capture(LogFormat::Json, || {
        let root = context.span("task.transition");
        let _entered = root.enter();
        let child = tracing::info_span!("db.query", statement = "update_task");
        let _in_child = child.enter();
        tracing::info!("status changed");
    });

    let line: serde_json::Value =
        serde_json::from_str(output.trim()).unwrap_or_else(|e| panic!("not JSON: {e}\n{output}"));

    // The innermost span is the child — that is what `span` means.
    assert_eq!(
        line["span"]["name"],
        serde_json::json!("db.query"),
        "{output}"
    );

    // …and the correlation fields are still reachable, from the ancestor
    // list, which is why it is enabled.
    let spans = line["spans"].as_array().expect("span list");
    let root = spans
        .iter()
        .find(|s| s["name"] == serde_json::json!("unit_of_work"))
        .unwrap_or_else(|| panic!("no unit_of_work span in the ancestor list:\n{output}"));
    for field in ["correlation_id", "request_id", "workspace_id", "actor_id"] {
        assert!(
            root.get(field).is_some_and(|v| !v.is_null()),
            "docs/46 §The three signals: every line carries {field}, and this one \
                 does not:\n{output}"
        );
    }
    assert_eq!(
        root["correlation_id"],
        serde_json::json!(context.correlation_id().to_string())
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
