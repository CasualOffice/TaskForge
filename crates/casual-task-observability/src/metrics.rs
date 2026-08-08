//! The metric-name registry.
//!
//! Every metric in `docs/46-OBSERVABILITY-AND-OPERATIONS.md` §Domain metrics is
//! a constant here, plus the RED pair the same document specifies per endpoint.
//! Recording sites reference the constant, never a string literal, so a typo is
//! a compile error rather than a dashboard series that silently never appears —
//! which is the failure mode that only shows up during the incident the metric
//! was for.
//!
//! Each constant also declares its legal label keys. [`LabelSet`](crate::LabelSet)
//! rejects anything else, which is where the cardinality rule is enforced; see
//! [`crate::labels`].
//!
//! **This is a registry, not an exporter.** Nothing here records a value. No
//! Prometheus client library is a workspace dependency, so binding these names
//! to a collector and a `/metrics` endpoint is a separate, unimplemented task.

use std::fmt;

use crate::labels::{LabelKey, keys};

/// A metric name.
///
/// A newtype over `&'static str` so that a metric name cannot be constructed
/// from request data, and so the registry is compile-time data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetricName(&'static str);

impl MetricName {
    /// Declare a metric name.
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The name as exported.
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for MetricName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// What kind of instrument a metric is.
///
/// Recorded here because it decides the naming convention (a counter ends in
/// `_total`) and because a collector needs it at registration time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetricKind {
    /// Monotonically increasing; name ends in `_total`.
    Counter,
    /// A point-in-time value that may go up or down.
    Gauge,
    /// A distribution — the `p50/p95/max` readings `docs/46` asks for.
    Histogram,
}

/// One registered metric: its name, instrument kind, legal labels, and why an
/// operator would look at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metric {
    name: MetricName,
    kind: MetricKind,
    labels: &'static [LabelKey],
    help: &'static str,
}

impl Metric {
    /// Register a metric. Only called from this module.
    const fn new(
        name: &'static str,
        kind: MetricKind,
        labels: &'static [LabelKey],
        help: &'static str,
    ) -> Self {
        Self {
            name: MetricName::new(name),
            kind,
            labels,
            help,
        }
    }

    /// The exported name.
    pub const fn name(&self) -> MetricName {
        self.name
    }

    /// The instrument kind.
    pub const fn kind(&self) -> MetricKind {
        self.kind
    }

    /// The label keys this metric may carry. Anything else is a
    /// [`CardinalityError`](crate::CardinalityError).
    pub const fn labels(&self) -> &'static [LabelKey] {
        self.labels
    }

    /// The `HELP` text — what an operator learns from this series.
    pub const fn help(&self) -> &'static str {
        self.help
    }
}

impl fmt::Display for Metric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.name, f)
    }
}

// ---------------------------------------------------------------------------
// Outbox — docs/46 calls outbox lag "the primary health signal" (docs/25).
// ---------------------------------------------------------------------------

/// The primary health signal: it moves first under database pressure, consumer
/// failure, or a dead worker (`docs/46` §Domain metrics).
pub const OUTBOX_LAG_SECONDS: Metric = Metric::new(
    "outbox_lag_seconds",
    MetricKind::Histogram,
    &[keys::CONSUMER],
    "Age of the oldest undispatched outbox event, by consumer",
);

/// Events that gave up. `docs/46` alerts on any sustained increase.
pub const OUTBOX_DLQ_DEPTH: Metric = Metric::new(
    "outbox_dlq_depth",
    MetricKind::Gauge,
    &[keys::EVENT_TYPE],
    "Events in the dead letter queue; never expected to be non-zero",
);

/// How stale search is (`docs/26`).
pub const SEARCH_PROJECTION_LAG_SECONDS: Metric = Metric::new(
    "search_projection_lag_seconds",
    MetricKind::Gauge,
    &[],
    "Delay between a task commit and its search projection",
);

// ---------------------------------------------------------------------------
// Authorization — docs/04. Cost paid on every request.
// ---------------------------------------------------------------------------

/// Permission resolution cost, paid on every request (`docs/04`).
pub const AUTHZ_RESOLUTION_DURATION: Metric = Metric::new(
    "authz_resolution_duration",
    MetricKind::Histogram,
    &[keys::OUTCOME],
    "Time to resolve an effective permission set",
);

/// Cache effectiveness for the resolver. A drop here explains a latency rise.
pub const AUTHZ_CACHE_HIT_RATIO: Metric = Metric::new(
    "authz_cache_hit_ratio",
    MetricKind::Gauge,
    &[],
    "Fraction of permission resolutions served from cache",
);

/// Cache churn; a spike means a mass permission change (`docs/46`).
pub const AUTHZ_EPOCH_BUMPS_TOTAL: Metric = Metric::new(
    "authz_epoch_bumps_total",
    MetricKind::Counter,
    &[keys::REASON],
    "Permission cache epoch invalidations",
);

/// A burst signals compromise or misconfiguration (`docs/46` alerts on 10×
/// baseline for one actor).
pub const PERMISSION_DENIED_TOTAL: Metric = Metric::new(
    "permission_denied_total",
    MetricKind::Counter,
    &[keys::PERMISSION],
    "Authorization denials, by permission key",
);

// ---------------------------------------------------------------------------
// Workflow — docs/23.
// ---------------------------------------------------------------------------

/// Workflow friction (`docs/23`). The reason is a closed set of codes, never
/// free text, or this metric becomes a cardinality bomb.
pub const TRANSITION_REJECTED_TOTAL: Metric = Metric::new(
    "transition_rejected_total",
    MetricKind::Counter,
    &[keys::REASON],
    "Rejected status transitions, by rejection reason code",
);

// ---------------------------------------------------------------------------
// Plugins — docs/34. Per-installation, at a stated cardinality cost.
// ---------------------------------------------------------------------------

/// Per-plugin latency (`docs/34`). See
/// [`LabelValue::plugin_installation`](crate::LabelValue::plugin_installation)
/// for the cardinality cost of the `installation` label.
pub const PLUGIN_CALL_DURATION: Metric = Metric::new(
    "plugin_call_duration",
    MetricKind::Histogram,
    &[keys::INSTALLATION, keys::EXTENSION_POINT, keys::OUTCOME],
    "Plugin call latency, by installation and extension point",
);

/// Plugin calls that returned an error.
pub const PLUGIN_CALL_ERRORS_TOTAL: Metric = Metric::new(
    "plugin_call_errors_total",
    MetricKind::Counter,
    &[keys::INSTALLATION, keys::EXTENSION_POINT],
    "Plugin calls that failed",
);

/// Plugin calls that exceeded their deadline. Separate from errors because the
/// operator action differs: a timeout points at the integration, not the payload.
pub const PLUGIN_CALL_TIMEOUTS_TOTAL: Metric = Metric::new(
    "plugin_call_timeouts_total",
    MetricKind::Counter,
    &[keys::INSTALLATION, keys::EXTENSION_POINT],
    "Plugin calls that exceeded their timeout budget",
);

/// Which integrations are down (`docs/46` runbook 3, "plugin circuit storm").
pub const PLUGIN_CIRCUIT_STATE: Metric = Metric::new(
    "plugin_circuit_state",
    MetricKind::Gauge,
    &[keys::INSTALLATION, keys::CIRCUIT_STATE],
    "Circuit breaker state per plugin installation",
);

// ---------------------------------------------------------------------------
// Automation — docs/36.
// ---------------------------------------------------------------------------

/// Automation execution volume (`docs/36`).
pub const AUTOMATION_RUNS_TOTAL: Metric = Metric::new(
    "automation_runs_total",
    MetricKind::Counter,
    &[keys::OUTCOME],
    "Automation rule executions, by outcome",
);

/// Runaway rules. `docs/46` alerts on any occurrence.
pub const AUTOMATION_DEPTH_EXCEEDED_TOTAL: Metric = Metric::new(
    "automation_depth_exceeded_total",
    MetricKind::Counter,
    &[],
    "Automation chains stopped by the recursion depth limit",
);

// ---------------------------------------------------------------------------
// Runtime saturation.
// ---------------------------------------------------------------------------

/// Live-update load (`docs/27`).
pub const SSE_CONNECTIONS_ACTIVE: Metric = Metric::new(
    "sse_connections_active",
    MetricKind::Gauge,
    &[],
    "Open server-sent-event streams",
);

/// Database saturation. `docs/46` pages at > 90% for 5 minutes.
pub const DB_POOL_UTILIZATION: Metric = Metric::new(
    "db_pool_utilization",
    MetricKind::Gauge,
    &[keys::POOL],
    "Fraction of the connection pool checked out",
);

/// Query latency by *named statement*, never by SQL text — statement names are
/// `&'static`, SQL text with inlined literals would be unbounded.
pub const DB_QUERY_DURATION: Metric = Metric::new(
    "db_query_duration",
    MetricKind::Histogram,
    &[keys::STATEMENT, keys::POOL],
    "Database query latency by named statement",
);

/// Files stuck invisible (`docs/28`). `docs/46` tickets at > 100 for 15 minutes.
pub const ATTACHMENT_SCAN_QUEUE_DEPTH: Metric = Metric::new(
    "attachment_scan_queue_depth",
    MetricKind::Gauge,
    &[],
    "Uploaded attachments awaiting a malware scan verdict",
);

/// Who is being throttled (`docs/21`).
///
/// `docs/46` §Domain metrics writes this one "by workspace", and §Cardinality
/// discipline forbids a raw `workspace_id` label. Those two lines are in
/// tension. This registry resolves it the way §Cardinality discipline requires —
/// [`keys::WORKSPACE_BUCKET`] and [`keys::WORKSPACE_INVESTIGATION`], both bounded
/// — and the resolution is recorded as an open question in the task report
/// rather than settled silently. **The cost:** this metric answers "is
/// throttling concentrated?" and not "which tenant?"; the latter is a log query.
pub const RATE_LIMIT_HITS_TOTAL: Metric = Metric::new(
    "rate_limit_hits_total",
    MetricKind::Counter,
    &[
        keys::SCOPE_KIND,
        keys::WORKSPACE_BUCKET,
        keys::WORKSPACE_INVESTIGATION,
    ],
    "Requests rejected by a rate limit, by limiter scope and tenant bucket",
);

// ---------------------------------------------------------------------------
// RED, per endpoint (docs/46 §The three signals).
// ---------------------------------------------------------------------------

/// Request rate and errors. `route` is the route *template*; the resolved path
/// carries ids and would be unbounded.
pub const HTTP_REQUESTS_TOTAL: Metric = Metric::new(
    "http_requests_total",
    MetricKind::Counter,
    &[keys::METHOD, keys::ROUTE, keys::STATUS_CLASS],
    "HTTP requests served, by route template and status class",
);

/// Request duration — the SLO in `docs/30` is stated against this.
pub const HTTP_REQUEST_DURATION_SECONDS: Metric = Metric::new(
    "http_request_duration_seconds",
    MetricKind::Histogram,
    &[keys::METHOD, keys::ROUTE, keys::STATUS_CLASS],
    "HTTP request latency, by route template",
);

/// Every registered metric.
///
/// Exists so the invariants are asserted over the whole registry rather than
/// per constant: a metric added without a test still cannot carry a tenant label
/// or a mis-suffixed name.
pub const ALL: &[Metric] = &[
    OUTBOX_LAG_SECONDS,
    OUTBOX_DLQ_DEPTH,
    SEARCH_PROJECTION_LAG_SECONDS,
    AUTHZ_RESOLUTION_DURATION,
    AUTHZ_CACHE_HIT_RATIO,
    AUTHZ_EPOCH_BUMPS_TOTAL,
    PERMISSION_DENIED_TOTAL,
    TRANSITION_REJECTED_TOTAL,
    PLUGIN_CALL_DURATION,
    PLUGIN_CALL_ERRORS_TOTAL,
    PLUGIN_CALL_TIMEOUTS_TOTAL,
    PLUGIN_CIRCUIT_STATE,
    AUTOMATION_RUNS_TOTAL,
    AUTOMATION_DEPTH_EXCEEDED_TOTAL,
    SSE_CONNECTIONS_ACTIVE,
    DB_POOL_UTILIZATION,
    DB_QUERY_DURATION,
    ATTACHMENT_SCAN_QUEUE_DEPTH,
    RATE_LIMIT_HITS_TOTAL,
    HTTP_REQUESTS_TOTAL,
    HTTP_REQUEST_DURATION_SECONDS,
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// The metrics `docs/46` §Domain metrics names explicitly. If a row is
    /// deleted from the registry, this fails.
    const DOCUMENTED: &[&str] = &[
        "outbox_lag_seconds",
        "outbox_dlq_depth",
        "search_projection_lag_seconds",
        "authz_resolution_duration",
        "authz_cache_hit_ratio",
        "authz_epoch_bumps_total",
        "permission_denied_total",
        "transition_rejected_total",
        "plugin_call_duration",
        "plugin_circuit_state",
        "automation_runs_total",
        "sse_connections_active",
        "db_pool_utilization",
        "attachment_scan_queue_depth",
        "rate_limit_hits_total",
    ];

    #[test]
    fn every_documented_metric_is_registered() {
        let registered: BTreeSet<_> = ALL.iter().map(|m| m.name().as_str()).collect();
        for name in DOCUMENTED {
            assert!(
                registered.contains(name),
                "docs/46 §Domain metrics lists `{name}`, which is not in the registry"
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
}
