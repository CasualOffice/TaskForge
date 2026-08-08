//! Recording metric values, and rendering them as Prometheus exposition text.
//!
//! # Why this is written here rather than taken from a crate
//!
//! The whole point of [`labels`](crate::labels) is that a label value **cannot**
//! be a workspace id: `LabelValue` has no `From<String>`, no `From<Uuid>`, and
//! no constructor taking `impl Display`, so widening a metric to per-tenant
//! cardinality requires naming one of two constructors that say so in their own
//! documentation (`docs/46` §Cardinality discipline, D-042).
//!
//! Every general-purpose Rust metrics facade takes labels as `&str` pairs. Put
//! one underneath this crate and the guard becomes a convention at the call
//! site — the exact thing `docs/10` §3 says not to build: "a rule survives until
//! the eleventh engineer; a compile error survives."
//!
//! So the recording surface here accepts a [`Metric`] and a
//! [`LabelSet`] and nothing else. There is no method
//! that takes a string pair, which is why there is no way to bypass the guard.
//!
//! The cost, stated: the exposition format, the bucket layout, and the
//! concurrency are ours to get right. The format is small and stable, the
//! buckets are declared once below, and the concurrency is a mutex around a map
//! — a scrape happens every fifteen seconds and a counter increment is not on a
//! hot path that a lock-free structure would rescue.
//!
//! # What this does not do
//!
//! **It does not serve HTTP.** `docs/19` puts every HTTP type in
//! `casual-task-api` and `casual-task-lint` enforces it, so the `/metrics`
//! endpoint arrives with the API in C-001. This produces the body it will send.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::labels::LabelSet;
use crate::metrics::{Metric, MetricKind};

/// Histogram bucket upper bounds, in **seconds**.
///
/// Chosen against the targets in `docs/30`, not from a template. That document
/// sets p95 read < 150 ms, so the buckets are dense either side of 0.15: a
/// layout that jumped 0.1 → 1.0 would put the number the project is judged on
/// inside one bucket, and every quantile estimate across it would be an
/// interpolation rather than a measurement.
///
/// The tail runs to 10 s because a request that slow is a distinct failure and
/// its shape matters when diagnosing one; `+Inf` catches the rest.
pub const BUCKETS: &[f64] = &[
    0.005, 0.010, 0.025, 0.050, 0.075, 0.100, 0.150, 0.200, 0.300, 0.500, 0.750, 1.0, 2.5, 5.0,
    10.0,
];

/// A metric was recorded through the wrong method for its declared kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrongKind {
    pub metric: &'static str,
    pub declared: MetricKind,
    pub used: MetricKind,
}

impl std::fmt::Display for WrongKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` is declared {:?} in the registry but was recorded as {:?}",
            self.metric, self.declared, self.used
        )
    }
}

impl std::error::Error for WrongKind {}

#[derive(Debug, Default)]
struct Histogram {
    /// Cumulative counts, parallel to [`BUCKETS`].
    counts: Vec<u64>,
    sum: f64,
    count: u64,
}

#[derive(Debug)]
enum Series {
    Counter(u64),
    /// Stored as bits: a gauge is read and written whole, and `f64` has no
    /// atomic form. The mutex around the map already serialises access.
    Gauge(f64),
    Histogram(Histogram),
}

/// Somewhere to record values, and the source of the `/metrics` body.
///
/// One per process. Cloning is deliberately not offered — two recorders would
/// each hold half the truth and a scrape would report whichever it found.
#[derive(Debug, Default)]
pub struct Recorder {
    series: Mutex<BTreeMap<(&'static str, String), Series>>,
    /// Recording attempts refused for a declared-kind mismatch. Exposed so a
    /// test can assert the count is zero rather than trusting that no call site
    /// ignored a `Result`.
    rejected: AtomicU64,
}

impl Recorder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add to a counter.
    ///
    /// # Errors
    ///
    /// [`WrongKind`] if `metric` is not declared a counter.
    pub fn increment(&self, metric: Metric, labels: &LabelSet, by: u64) -> Result<(), WrongKind> {
        self.check(metric, MetricKind::Counter)?;
        let mut series = self.series.lock().expect("not poisoned");
        match series
            .entry(key(metric, labels))
            .or_insert(Series::Counter(0))
        {
            Series::Counter(n) => *n += by,
            _ => unreachable!("kind checked above"),
        }
        Ok(())
    }

    /// Set a gauge to its current value.
    ///
    /// # Errors
    ///
    /// [`WrongKind`] if `metric` is not declared a gauge.
    pub fn set(&self, metric: Metric, labels: &LabelSet, value: f64) -> Result<(), WrongKind> {
        self.check(metric, MetricKind::Gauge)?;
        let mut series = self.series.lock().expect("not poisoned");
        series.insert(key(metric, labels), Series::Gauge(value));
        Ok(())
    }

    /// Observe one value into a histogram.
    ///
    /// # Errors
    ///
    /// [`WrongKind`] if `metric` is not declared a histogram.
    pub fn observe(&self, metric: Metric, labels: &LabelSet, value: f64) -> Result<(), WrongKind> {
        self.check(metric, MetricKind::Histogram)?;
        let mut series = self.series.lock().expect("not poisoned");
        let entry = series.entry(key(metric, labels)).or_insert_with(|| {
            Series::Histogram(Histogram {
                counts: vec![0; BUCKETS.len()],
                sum: 0.0,
                count: 0,
            })
        });
        match entry {
            Series::Histogram(h) => {
                for (i, bound) in BUCKETS.iter().enumerate() {
                    if value <= *bound {
                        h.counts[i] += 1;
                    }
                }
                h.sum += value;
                h.count += 1;
            }
            _ => unreachable!("kind checked above"),
        }
        Ok(())
    }

    /// How many recordings were refused for a kind mismatch.
    #[must_use]
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    fn check(&self, metric: Metric, used: MetricKind) -> Result<(), WrongKind> {
        if metric.kind() == used {
            return Ok(());
        }
        self.rejected.fetch_add(1, Ordering::Relaxed);
        Err(WrongKind {
            metric: metric.name().as_str(),
            declared: metric.kind(),
            used,
        })
    }

    /// The Prometheus text exposition body.
    ///
    /// Deterministic: series come from a `BTreeMap`, so the output is sorted and
    /// a test can assert on it without sorting first. That also makes a diff
    /// between two scrapes readable during an incident.
    #[must_use]
    pub fn render(&self) -> String {
        let series = self.series.lock().expect("not poisoned");
        let mut out = String::new();
        let mut described: Option<&str> = None;

        for ((name, labels), value) in series.iter() {
            if described != Some(name) {
                // HELP and TYPE are emitted once per metric family, not once
                // per series — repeating them is a parse error in some
                // scrapers rather than merely noisy.
                let metric = crate::metrics::ALL
                    .iter()
                    .find(|m| m.name().as_str() == *name);
                if let Some(m) = metric {
                    out.push_str(&format!("# HELP {name} {}\n", escape_help(m.help())));
                    out.push_str(&format!("# TYPE {name} {}\n", type_name(m.kind())));
                }
                described = Some(name);
            }

            match value {
                Series::Counter(n) => {
                    out.push_str(&format!("{name}{labels} {n}\n"));
                }
                Series::Gauge(v) => {
                    out.push_str(&format!("{name}{labels} {v}\n"));
                }
                Series::Histogram(h) => {
                    for (i, bound) in BUCKETS.iter().enumerate() {
                        out.push_str(&format!(
                            "{name}_bucket{} {}\n",
                            with_le(labels, &format!("{bound}")),
                            h.counts[i]
                        ));
                    }
                    out.push_str(&format!(
                        "{name}_bucket{} {}\n",
                        with_le(labels, "+Inf"),
                        h.count
                    ));
                    out.push_str(&format!("{name}_sum{labels} {}\n", h.sum));
                    out.push_str(&format!("{name}_count{labels} {}\n", h.count));
                }
            }
        }
        out
    }
}

const fn type_name(kind: MetricKind) -> &'static str {
    match kind {
        MetricKind::Counter => "counter",
        MetricKind::Gauge => "gauge",
        MetricKind::Histogram => "histogram",
    }
}

/// `{a="1",b="2"}`, or the empty string when there are no labels.
///
/// Sorted, so the same logical series always renders identically — an unsorted
/// label set would produce two map keys for one series and silently split its
/// counts.
fn key(metric: Metric, labels: &LabelSet) -> (&'static str, String) {
    let mut pairs = labels.pairs();
    pairs.sort_unstable();
    if pairs.is_empty() {
        return (metric.name().as_str(), String::new());
    }
    let rendered = pairs
        .iter()
        .map(|(k, v)| format!("{k}=\"{}\"", escape_label(v)))
        .collect::<Vec<_>>()
        .join(",");
    (metric.name().as_str(), format!("{{{rendered}}}"))
}

/// Insert the `le` bucket bound into an existing rendered label set.
fn with_le(labels: &str, le: &str) -> String {
    if labels.is_empty() {
        return format!("{{le=\"{le}\"}}");
    }
    format!("{},le=\"{le}\"}}", labels.trim_end_matches('}'))
}

/// Exposition escaping for a label value: backslash, double quote, newline.
fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// HELP text escaping: backslash and newline only — a quote is legal there.
fn escape_help(help: &str) -> String {
    help.replace('\\', "\\\\").replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labels::keys;
    use crate::metrics::{
        HTTP_REQUEST_DURATION_SECONDS, OUTBOX_DISPATCH_TOTAL, OUTBOX_LAG_SECONDS,
    };

    #[test]
    fn a_gauge_renders_with_help_and_type() {
        let r = Recorder::new();
        let labels = LabelSet::for_metric(OUTBOX_LAG_SECONDS)
            .with(keys::CONSUMER, "webhook_delivery")
            .expect("declared label");
        r.set(OUTBOX_LAG_SECONDS, &labels, 12.5).expect("a gauge");

        let out = r.render();
        assert!(out.contains("# TYPE outbox_lag_seconds gauge"), "{out}");
        assert!(
            out.contains("outbox_lag_seconds{consumer=\"webhook_delivery\"} 12.5"),
            "{out}"
        );
    }

    #[test]
    fn recording_a_gauge_as_a_counter_is_refused() {
        // D-047 made outbox_lag_seconds a Gauge. Incrementing it would produce a
        // monotonically rising "lag" that never recovers — a plausible-looking
        // series that is wrong in the direction nobody checks.
        let r = Recorder::new();
        let labels = LabelSet::for_metric(OUTBOX_LAG_SECONDS)
            .with(keys::CONSUMER, "sse_fanout")
            .expect("declared label");

        let refused = r.increment(OUTBOX_LAG_SECONDS, &labels, 1);
        assert_eq!(
            refused,
            Err(WrongKind {
                metric: "outbox_lag_seconds",
                declared: MetricKind::Gauge,
                used: MetricKind::Counter,
            })
        );
        assert_eq!(r.rejected(), 1);
        assert_eq!(r.render(), "", "a refused recording still wrote a series");
    }

    #[test]
    fn a_counter_accumulates_per_label_set() {
        let r = Recorder::new();
        for (consumer, outcome, n) in [
            ("webhook_delivery", "dispatched", 3),
            ("webhook_delivery", "failed", 1),
            ("sse_fanout", "dispatched", 5),
        ] {
            let labels = LabelSet::for_metric(OUTBOX_DISPATCH_TOTAL)
                .with(keys::CONSUMER, consumer)
                .expect("declared")
                .with(keys::OUTCOME, outcome)
                .expect("declared");
            r.increment(OUTBOX_DISPATCH_TOTAL, &labels, n)
                .expect("a counter");
        }

        let out = r.render();
        assert!(out.contains(
            "outbox_dispatch_total{consumer=\"webhook_delivery\",outcome=\"dispatched\"} 3"
        ));
        assert!(
            out.contains(
                "outbox_dispatch_total{consumer=\"webhook_delivery\",outcome=\"failed\"} 1"
            )
        );
        assert!(
            out.contains("outbox_dispatch_total{consumer=\"sse_fanout\",outcome=\"dispatched\"} 5")
        );
    }

    #[test]
    fn a_histogram_renders_cumulative_buckets_a_sum_and_a_count() {
        let r = Recorder::new();
        let labels = LabelSet::for_metric(HTTP_REQUEST_DURATION_SECONDS);
        for v in [0.004, 0.120, 3.0] {
            r.observe(HTTP_REQUEST_DURATION_SECONDS, &labels, v)
                .expect("a histogram");
        }

        let out = r.render();
        // Cumulative: the 0.005 bucket holds one, 0.150 holds two, +Inf all three.
        assert!(
            out.contains("http_request_duration_seconds_bucket{le=\"0.005\"} 1"),
            "{out}"
        );
        assert!(
            out.contains("http_request_duration_seconds_bucket{le=\"0.15\"} 2"),
            "{out}"
        );
        assert!(
            out.contains("http_request_duration_seconds_bucket{le=\"+Inf\"} 3"),
            "{out}"
        );
        assert!(
            out.contains("http_request_duration_seconds_count 3"),
            "{out}"
        );
    }

    #[test]
    fn buckets_are_dense_around_the_target_the_project_is_judged_on() {
        // docs/30 sets p95 read < 150 ms. A layout that jumped 0.1 -> 1.0 would
        // put that number inside a single bucket and make every quantile
        // estimate across it an interpolation.
        assert!(BUCKETS.contains(&0.150), "no bucket boundary at the target");
        let near: Vec<_> = BUCKETS
            .iter()
            .filter(|b| **b > 0.05 && **b < 0.35)
            .collect();
        assert!(
            near.len() >= 4,
            "only {} bucket boundaries between 50 ms and 350 ms",
            near.len()
        );
    }

    #[test]
    fn buckets_are_strictly_increasing() {
        // Prometheus requires it, and a bucket list out of order produces
        // cumulative counts that go backwards.
        for pair in BUCKETS.windows(2) {
            assert!(pair[1] > pair[0], "{pair:?} is not increasing");
        }
    }

    #[test]
    fn label_values_are_escaped() {
        // Not reachable through the declared label set today — every value is a
        // source literal — but the renderer must not be the thing that assumes
        // that, because a future bounded constructor could return a value with
        // a quote in it and produce unparseable output.
        assert_eq!(escape_label(r#"a"b\c"#), r#"a\"b\\c"#);
        assert_eq!(escape_label("a\nb"), "a\\nb");
    }

    #[test]
    fn nothing_rendered_looks_like_a_uuid() {
        // docs/46 §Cardinality discipline, asserted on the OUTPUT rather than
        // on the type. The type guard is the real mechanism; this catches a
        // future value that satisfies the type and still carries an id.
        let r = Recorder::new();
        for metric in crate::metrics::ALL {
            let mut labels = LabelSet::for_metric(*metric);
            for key in metric.labels() {
                labels = labels.with(*key, "sample").expect("declared label");
            }
            let _ = match metric.kind() {
                MetricKind::Counter => r.increment(*metric, &labels, 1),
                MetricKind::Gauge => r.set(*metric, &labels, 1.0),
                MetricKind::Histogram => r.observe(*metric, &labels, 0.1),
            };
        }

        let out = r.render();
        let uuid_shaped = out
            .split(|c: char| !c.is_ascii_hexdigit() && c != '-')
            .any(|word| word.len() == 36 && word.split('-').map(str::len).eq([8, 4, 4, 4, 12]));
        assert!(!uuid_shaped, "a rendered label looks like a uuid:\n{out}");
    }

    #[test]
    fn every_declared_metric_can_be_recorded_and_rendered() {
        // A metric in the registry that no method accepts is a metric nobody
        // can emit — declared, dashboarded, and permanently absent.
        let r = Recorder::new();
        for metric in crate::metrics::ALL {
            let mut labels = LabelSet::for_metric(*metric);
            for key in metric.labels() {
                labels = labels.with(*key, "x").expect("declared label");
            }
            let recorded = match metric.kind() {
                MetricKind::Counter => r.increment(*metric, &labels, 1),
                MetricKind::Gauge => r.set(*metric, &labels, 1.0),
                MetricKind::Histogram => r.observe(*metric, &labels, 0.1),
            };
            assert!(recorded.is_ok(), "{}: {recorded:?}", metric.name());
        }

        assert_eq!(r.rejected(), 0);
        let out = r.render();
        for metric in crate::metrics::ALL {
            assert!(
                out.contains(&format!("# TYPE {} ", metric.name())),
                "{} is declared but never rendered",
                metric.name()
            );
        }
    }
}
