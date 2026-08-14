use std::collections::BTreeSet;

use super::*;

/// Every row of `docs/46` §Domain metrics, as `(name, kind, labels the doc
/// requires "by …")`.
///
/// The earlier version of this list held names only, and was missing four
/// of them — `db_query_duration`, `plugin_call_errors_total`,
/// `plugin_call_timeouts_total`, and `automation_depth_exceeded_total`
/// could all have been deleted from the registry with this test still
/// green, which is the exact regression it exists to catch. It also could
/// not see a metric changing from a Histogram to a Gauge, or losing the
/// label the doc asks for it to be broken down by.
const DOCUMENTED: &[(&str, MetricKind, &[LabelKey])] = &[
    // Gauge since D-047 was Accepted. docs/46 previously asked for
    // `p50/p95/max` here and flagged the contradiction itself: the help
    // text described the age of the single oldest pending event, which is
    // a gauge quantity, while the registry declared a histogram. There is
    // only ever one oldest, so the percentiles would have been taken over
    // repeated readings of the same number.
    ("outbox_lag_seconds", MetricKind::Gauge, &[keys::CONSUMER]),
    ("outbox_dlq_depth", MetricKind::Gauge, &[keys::CONSUMER]),
    (
        "outbox_dispatch_total",
        MetricKind::Counter,
        &[keys::CONSUMER, keys::OUTCOME],
    ),
    // Gauge because "how stale is search right now" is a point-in-time
    // reading.
    ("search_projection_lag_seconds", MetricKind::Gauge, &[]),
    ("authz_resolution_duration", MetricKind::Histogram, &[]),
    ("authz_cache_hit_ratio", MetricKind::Gauge, &[]),
    ("authz_epoch_bumps_total", MetricKind::Counter, &[]),
    // "by permission", "by reason" — docs/46 names the breakdown, so the
    // label is part of the contract, not an implementation detail.
    (
        "permission_denied_total",
        MetricKind::Counter,
        &[keys::PERMISSION],
    ),
    (
        "transition_rejected_total",
        MetricKind::Counter,
        &[keys::REASON],
    ),
    (
        "plugin_call_duration",
        MetricKind::Histogram,
        &[keys::INSTALLATION],
    ),
    (
        "plugin_call_errors_total",
        MetricKind::Counter,
        &[keys::INSTALLATION],
    ),
    (
        "plugin_call_timeouts_total",
        MetricKind::Counter,
        &[keys::INSTALLATION],
    ),
    (
        "plugin_circuit_state",
        MetricKind::Gauge,
        &[keys::INSTALLATION],
    ),
    ("automation_runs_total", MetricKind::Counter, &[]),
    ("automation_depth_exceeded_total", MetricKind::Counter, &[]),
    ("sse_connections_active", MetricKind::Gauge, &[]),
    ("db_pool_utilization", MetricKind::Gauge, &[keys::POOL]),
    (
        "db_query_duration",
        MetricKind::Histogram,
        &[keys::STATEMENT],
    ),
    ("attachment_scan_queue_depth", MetricKind::Gauge, &[]),
    ("rate_limit_hits_total", MetricKind::Counter, &[]),
];

/// Registered metrics that `docs/46` §Domain metrics does not name.
///
/// docs/46 asks for "RED per endpoint" without naming the series, so these
/// two spellings are this crate's choice. They are listed rather than
/// tolerated silently, so that a *third* invented metric fails the reverse
/// check below and has to be argued for.
const UNDOCUMENTED_BY_DESIGN: &[&str] = &["http_requests_total", "http_request_duration_seconds"];

#[test]
fn every_documented_metric_is_registered_with_the_documented_shape() {
    for (name, kind, required_labels) in DOCUMENTED {
        let metric = ALL
            .iter()
            .find(|m| m.name().as_str() == *name)
            .unwrap_or_else(|| {
                panic!("docs/46 §Domain metrics lists `{name}`, which is not in the registry")
            });
        assert_eq!(
            metric.kind(),
            *kind,
            "`{name}` is registered as {:?}; docs/46 describes a {kind:?}",
            metric.kind()
        );
        for label in *required_labels {
            assert!(
                metric.labels().contains(label),
                "docs/46 breaks `{name}` down by `{}`, which it does not declare",
                label.as_str()
            );
        }
    }
}

#[test]
fn no_metric_is_registered_that_the_design_record_does_not_name() {
    // The reverse direction. Without it the registry can grow series that
    // no document justifies, and every one of those is a dashboard, an
    // alert, and a cardinality cost nobody reviewed.
    let documented: BTreeSet<_> = DOCUMENTED
        .iter()
        .map(|(n, _, _)| *n)
        .chain(UNDOCUMENTED_BY_DESIGN.iter().copied())
        .collect();
    for metric in ALL {
        assert!(
            documented.contains(metric.name().as_str()),
            "`{}` is registered but appears in neither docs/46 §Domain metrics nor \
                 the UNDOCUMENTED_BY_DESIGN allow-list. Add it to the document, or say \
                 here why it is exempt.",
            metric.name().as_str()
        );
    }
}

#[test]
fn metric_names_are_unique() {
    let mut seen = BTreeSet::new();
    for metric in ALL {
        assert!(
            seen.insert(metric.name()),
            "duplicate metric name `{}`",
            metric.name()
        );
    }
}

#[test]
fn counters_end_in_total() {
    // Prometheus convention; a counter without it reads as a gauge on a
    // dashboard and gets summed instead of rated.
    for metric in ALL {
        if metric.kind() == MetricKind::Counter {
            assert!(
                metric.name().as_str().ends_with("_total"),
                "counter `{}` must end in `_total`",
                metric.name()
            );
        }
    }
}

#[test]
fn names_are_valid_exposition_identifiers() {
    for metric in ALL {
        let name = metric.name().as_str();
        assert!(!name.is_empty());
        assert!(
            name.starts_with(|c: char| c.is_ascii_lowercase()),
            "`{name}` must start with a lowercase letter"
        );
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "`{name}` must be lower snake_case"
        );
        assert!(
            !metric.help().is_empty(),
            "`{name}` has no HELP text; an undescribed series is an unusable one"
        );
    }
}

#[test]
fn a_metric_declares_no_duplicate_label_keys() {
    for metric in ALL {
        let unique: BTreeSet<_> = metric.labels().iter().collect();
        assert_eq!(
            unique.len(),
            metric.labels().len(),
            "metric `{}` declares a duplicate label key",
            metric.name()
        );
    }
}
