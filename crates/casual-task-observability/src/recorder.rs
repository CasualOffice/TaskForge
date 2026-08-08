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
//! buckets are declared once below, and the concurrency is the section that
//! follows — it was got wrong once already.
//!
//! # Concurrency: why this is not one mutex
//!
//! It was one mutex, and that was a defect. Every HTTP request records **twice**
//! — the RED counter and the duration histogram — so a single process-wide lock
//! made this crate a serialisation point on the hot path it exists to measure.
//! `GET /metrics` then took the *same* lock and held it across the whole of
//! [`Recorder::render`], so a scrape stalled every in-flight request for as long
//! as it took to build the body. At a handful of requests per second nobody
//! notices; at the concurrency `docs/30` targets, the wait shows up as latency
//! on every endpoint simultaneously, which reads like a database problem and
//! sends the investigation to the wrong place.
//!
//! Three changes, each aimed at one half of that:
//!
//! - **The value is an atomic, not a field behind a lock.** Recording into a
//!   series that already exists is a `fetch_add`; no thread waits for another.
//! - **The map is split into [`SHARDS`] shards, each behind an `RwLock`, and a
//!   lookup takes a *shared* read lock.** A scrape and any number of recorders
//!   hold theirs at the same time. Only the *first* observation of a series
//!   takes a write lock, and the set of series is bounded by the declared
//!   cardinality — in steady state that path is never taken again.
//! - **[`Recorder::render`] snapshots, then formats.** No lock is held while the
//!   string is built.
//!
//! **The cost, stated** (`docs/10` §4): a scrape is no longer an instantaneous
//! snapshot of every series. Two series in the same body may be microseconds
//! apart, and a histogram's `_sum` may not include an observation whose bucket
//! it does. Prometheus already treats a scrape as a set of independently timed
//! samples, and the alternative is the stall above. What is **not** given up is
//! that a single histogram never renders an *invalid* shape — the write order
//! in `Histogram::observe` guarantees it, and a test hammers it.

use std::collections::{BTreeMap, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

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

/// How many independently locked shards the series map is split into.
///
/// A power of two, and larger than the core count of the machines `docs/48`
/// deploys on, so two threads recording two different series usually contend on
/// nothing at all. It is not tuned to a benchmark: the shard is only held for a
/// hash-map lookup, and past the point where each core can usually find its own,
/// more shards buy nothing and cost a fixed walk on every scrape.
pub const SHARDS: usize = 16;

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

#[derive(Debug)]
struct Histogram {
    /// Cumulative counts, parallel to [`BUCKETS`].
    counts: Vec<AtomicU64>,
    /// `f64` bits: there is no atomic float, and the sum is only ever read as a
    /// whole number of bits (see [`Histogram::read`]).
    sum_bits: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    fn new() -> Self {
        Self {
            counts: BUCKETS.iter().map(|_| AtomicU64::new(0)).collect(),
            sum_bits: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    /// Record one observation.
    ///
    /// **The write order is the correctness argument, not a style choice.** A
    /// Prometheus histogram's bucket counts are cumulative, so they must never
    /// be seen decreasing as `le` rises, and no bucket may exceed `+Inf` — which
    /// is rendered from `count`. Without a lock, a scrape can land in the middle
    /// of this function, so the ordering has to make every intermediate state a
    /// *valid* histogram rather than merely a stale one:
    ///
    /// 1. `count` first, so a reader can never see a bucket that has been
    ///    incremented while `+Inf` has not — a finite bucket above `+Inf` is a
    ///    histogram Prometheus rejects.
    /// 2. Buckets from the **top down**, so the increments a reader can see
    ///    always form a suffix. Bottom-up, a reader walking `le` ascending could
    ///    see `le="0.005" 1` followed by `le="0.010" 0`: cumulative counts going
    ///    backwards.
    ///
    /// The `Release` on each bucket store, paired with the `Acquire` loads in
    /// [`Histogram::read`], is what makes that order visible to another core
    /// rather than merely written in that order here.
    fn observe(&self, value: f64) {
        self.count.fetch_add(1, Ordering::Relaxed);

        // `f64` has no atomic add, so compare-and-swap the bit pattern. The loop
        // serialises concurrent adds but not their *order*, so the last bits of
        // `_sum` may differ between two runs of the same workload — float
        // addition is not associative. Stated rather than hidden: the sum feeds
        // an average, and no alert reads its final ulp.
        let mut current = self.sum_bits.load(Ordering::Relaxed);
        loop {
            let next = (f64::from_bits(current) + value).to_bits();
            match self.sum_bits.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }

        for (i, bound) in BUCKETS.iter().enumerate().rev() {
            // BUCKETS ascends, so the first bound this value exceeds ends the
            // suffix it belongs to.
            if value > *bound {
                break;
            }
            self.counts[i].fetch_add(1, Ordering::Release);
        }
    }

    /// Read the histogram for rendering, in the inverse of the write order.
    ///
    /// Buckets ascending and `count` last: an `Acquire` load that observes a
    /// bucket increment also observes everything [`Histogram::observe`] wrote
    /// before it, which is what turns that function's write order into an
    /// ordering this reader can rely on.
    fn read(&self) -> (Vec<u64>, f64, u64) {
        let counts = self
            .counts
            .iter()
            .map(|c| c.load(Ordering::Acquire))
            .collect();
        let sum = f64::from_bits(self.sum_bits.load(Ordering::Relaxed));
        let count = self.count.load(Ordering::Acquire);
        (counts, sum, count)
    }
}

#[derive(Debug)]
enum Series {
    Counter(AtomicU64),
    /// Stored as bits: a gauge is read and written whole, and `f64` has no
    /// atomic form.
    Gauge(AtomicU64),
    Histogram(Histogram),
}

/// The identity of one time series: metric name, and its rendered label set.
type SeriesKey = (&'static str, String);

/// One independently locked slice of the series map.
///
/// `align(64)` is the point of the type. Two shards sharing a cache line would
/// put their locks' state words on the same line, and two cores recording two
/// unrelated series would bounce that line between them — reintroducing the
/// contention sharding exists to remove, invisibly.
#[derive(Debug, Default)]
#[repr(align(64))]
struct Shard(RwLock<HashMap<SeriesKey, Arc<Series>>>);

/// Somewhere to record values, and the source of the `/metrics` body.
///
/// One per process. Cloning is deliberately not offered — two recorders would
/// each hold half the truth and a scrape would report whichever it found.
#[derive(Debug)]
pub struct Recorder {
    shards: [Shard; SHARDS],
    /// Recording attempts refused for a declared-kind mismatch. Exposed so a
    /// test can assert the count is zero rather than trusting that no call site
    /// ignored a `Result`.
    rejected: AtomicU64,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            shards: std::array::from_fn(|_| Shard::default()),
            rejected: AtomicU64::new(0),
        }
    }
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
        self.with_series(
            key(metric, labels),
            || Series::Counter(AtomicU64::new(0)),
            |series| match series {
                Series::Counter(n) => {
                    n.fetch_add(by, Ordering::Relaxed);
                }
                _ => unreachable!("kind checked above, and the kind follows the name in the key"),
            },
        );
        Ok(())
    }

    /// Set a gauge to its current value.
    ///
    /// # Errors
    ///
    /// [`WrongKind`] if `metric` is not declared a gauge.
    pub fn set(&self, metric: Metric, labels: &LabelSet, value: f64) -> Result<(), WrongKind> {
        self.check(metric, MetricKind::Gauge)?;
        self.with_series(
            key(metric, labels),
            || Series::Gauge(AtomicU64::new(value.to_bits())),
            |series| match series {
                Series::Gauge(bits) => bits.store(value.to_bits(), Ordering::Relaxed),
                _ => unreachable!("kind checked above, and the kind follows the name in the key"),
            },
        );
        Ok(())
    }

    /// Observe one value into a histogram.
    ///
    /// # Errors
    ///
    /// [`WrongKind`] if `metric` is not declared a histogram.
    pub fn observe(&self, metric: Metric, labels: &LabelSet, value: f64) -> Result<(), WrongKind> {
        self.check(metric, MetricKind::Histogram)?;
        self.with_series(
            key(metric, labels),
            || Series::Histogram(Histogram::new()),
            |series| match series {
                Series::Histogram(h) => h.observe(value),
                _ => unreachable!("kind checked above, and the kind follows the name in the key"),
            },
        );
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

    /// Find (or create) a series and hand it to `record`.
    ///
    /// The steady-state path is the read lock: a series exists after its first
    /// observation and is never removed, so the exclusive lock is taken at most
    /// once per series for the life of the process. That is what keeps a scrape
    /// — which holds only read locks — from blocking a request.
    fn with_series<T>(
        &self,
        key: SeriesKey,
        new: impl FnOnce() -> Series,
        record: impl FnOnce(&Series) -> T,
    ) -> T {
        let shard = &self.shards[shard_of(&key)];

        {
            let series = shard.0.read().expect("not poisoned");
            if let Some(existing) = series.get(&key) {
                return record(existing);
            }
        }

        // First observation of this series. The `Arc` is cloned out so the write
        // lock is released before recording, which keeps the exclusive hold to
        // the map insert itself.
        let mut series = shard.0.write().expect("not poisoned");
        let created = Arc::clone(series.entry(key).or_insert_with(|| Arc::new(new())));
        drop(series);
        record(&created)
    }

    /// The Prometheus text exposition body.
    ///
    /// Deterministic: the snapshot is collected into a `BTreeMap`, so the output
    /// is sorted by metric name and then by label set whatever order the shards
    /// happen to hold. A test can assert on it without sorting first, and a diff
    /// between two scrapes is readable during an incident.
    ///
    /// Snapshot first, format second. Formatting under the lock was the defect
    /// this crate's module docs describe: it made the length of a scrape into
    /// added latency on every concurrent request.
    #[must_use]
    pub fn render(&self) -> String {
        let mut snapshot: BTreeMap<SeriesKey, Arc<Series>> = BTreeMap::new();
        for shard in &self.shards {
            let series = shard.0.read().expect("not poisoned");
            snapshot.extend(
                series
                    .iter()
                    .map(|(series_key, value)| (series_key.clone(), Arc::clone(value))),
            );
        }

        let mut out = String::new();
        let mut described: Option<&str> = None;

        for ((name, labels), value) in &snapshot {
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

            match &**value {
                Series::Counter(n) => {
                    out.push_str(&format!("{name}{labels} {}\n", n.load(Ordering::Relaxed)));
                }
                Series::Gauge(bits) => {
                    let v = f64::from_bits(bits.load(Ordering::Relaxed));
                    out.push_str(&format!("{name}{labels} {v}\n"));
                }
                Series::Histogram(h) => {
                    let (counts, sum, count) = h.read();
                    for (i, bound) in BUCKETS.iter().enumerate() {
                        out.push_str(&format!(
                            "{name}_bucket{} {}\n",
                            with_le(labels, &format!("{bound}")),
                            counts[i]
                        ));
                    }
                    out.push_str(&format!(
                        "{name}_bucket{} {count}\n",
                        with_le(labels, "+Inf")
                    ));
                    out.push_str(&format!("{name}_sum{labels} {sum}\n"));
                    out.push_str(&format!("{name}_count{labels} {count}\n"));
                }
            }
        }
        out
    }
}

/// Which shard owns a series. Stable for a given key within a process, which is
/// all that is required — [`Recorder::render`] re-sorts, so the assignment is
/// never visible in the output.
fn shard_of(key: &SeriesKey) -> usize {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    (hasher.finish() % SHARDS as u64) as usize
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
fn key(metric: Metric, labels: &LabelSet) -> SeriesKey {
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
    use std::sync::atomic::AtomicBool;

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

    #[test]
    fn the_body_is_sorted_and_identical_between_two_scrapes_of_one_state() {
        // The series now live in SHARDS separate maps, in whatever order each
        // one's hashing puts them. Sorted output is a promise this crate makes —
        // `docs/50` diffs two scrapes during an incident, and a body that
        // reordered itself between scrapes would make that diff useless. This is
        // the test that keeps the sort where the shards could have removed it.
        let r = Recorder::new();
        for consumer in ["webhook_delivery", "sse_fanout", "search_projection"] {
            let labels = LabelSet::for_metric(OUTBOX_DISPATCH_TOTAL)
                .with(keys::CONSUMER, consumer)
                .expect("declared")
                .with(keys::OUTCOME, "dispatched")
                .expect("declared");
            r.increment(OUTBOX_DISPATCH_TOTAL, &labels, 1)
                .expect("a counter");
        }
        for metric in crate::metrics::ALL {
            let mut labels = LabelSet::for_metric(*metric);
            for key in metric.labels() {
                labels = labels.with(*key, "x").expect("declared label");
            }
            let _ = match metric.kind() {
                MetricKind::Counter => r.increment(*metric, &labels, 1),
                MetricKind::Gauge => r.set(*metric, &labels, 1.0),
                MetricKind::Histogram => r.observe(*metric, &labels, 0.1),
            };
        }

        let first = r.render();
        assert_eq!(first, r.render(), "two scrapes of one state differ");

        let families: Vec<&str> = first
            .lines()
            .filter_map(|l| l.strip_prefix("# TYPE "))
            .filter_map(|l| l.split(' ').next())
            .collect();
        let mut sorted = families.clone();
        sorted.sort_unstable();
        assert_eq!(
            families, sorted,
            "metric families are out of order:\n{first}"
        );

        // And within one family, label sets ascend too — the second half of the
        // ordering, which sorting families alone would not catch.
        let positions: Vec<usize> = ["search_projection", "sse_fanout", "webhook_delivery"]
            .iter()
            .map(|consumer| {
                first
                    .find(&format!(
                        "outbox_dispatch_total{{consumer=\"{consumer}\",outcome=\"dispatched\"}}"
                    ))
                    .unwrap_or_else(|| panic!("{consumer} is missing:\n{first}"))
            })
            .collect();
        assert!(
            positions.windows(2).all(|p| p[0] < p[1]),
            "label sets within a family are out of order:\n{first}"
        );
    }

    #[test]
    fn concurrent_increments_never_lose_one() {
        // The mutex this replaced made a lost update impossible by making every
        // recorder wait. Atomics keep the guarantee without the waiting, and the
        // only way to know they do is to hammer them: N threads x M increments
        // must render as exactly N*M.
        const THREADS: u64 = 16;
        const PER_THREAD: u64 = 5_000;

        let r = Recorder::new();
        let hot = LabelSet::for_metric(OUTBOX_DISPATCH_TOTAL)
            .with(keys::CONSUMER, "sse_fanout")
            .expect("declared")
            .with(keys::OUTCOME, "dispatched")
            .expect("declared");

        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                let (r, hot) = (&r, &hot);
                scope.spawn(move || {
                    for _ in 0..PER_THREAD {
                        r.increment(OUTBOX_DISPATCH_TOTAL, hot, 1)
                            .expect("a counter");
                    }
                });
            }
        });

        let out = r.render();
        assert!(
            out.contains(&format!(
                "outbox_dispatch_total{{consumer=\"sse_fanout\",outcome=\"dispatched\"}} {}\n",
                THREADS * PER_THREAD
            )),
            "lost an increment:\n{out}"
        );
    }

    #[test]
    fn a_scrape_running_throughout_costs_no_observation_and_sees_no_invalid_histogram() {
        // Two properties in one hammer, because they need the same setup.
        //
        // 1. Nothing is lost while a scrape runs. The whole reason `render` no
        //    longer holds an exclusive lock is that recorders must not wait for
        //    it — so a recorder running *during* a scrape must still be counted.
        // 2. No scrape taken mid-observation renders an invalid histogram.
        //    Cumulative bucket counts must never decrease as `le` rises. Without
        //    the write order in `Histogram::observe` this fails within a few
        //    thousand iterations, and the symptom in production is a scraper
        //    dropping the sample rather than an error anyone sees.
        const THREADS: usize = 8;
        const PER_THREAD: usize = 4_000;

        let r = Recorder::new();
        let labels = LabelSet::for_metric(HTTP_REQUEST_DURATION_SECONDS)
            .with(keys::METHOD, "GET")
            .expect("declared")
            .with(keys::ROUTE, "/health/live")
            .expect("declared");
        let stop = AtomicBool::new(false);

        std::thread::scope(|scope| {
            let scraper = scope.spawn(|| {
                let mut scrapes = 0_u64;
                while !stop.load(Ordering::Relaxed) {
                    assert_buckets_never_decrease(&r.render());
                    scrapes += 1;
                }
                scrapes
            });

            let workers: Vec<_> = (0..THREADS)
                .map(|_| {
                    let (r, labels) = (&r, &labels);
                    scope.spawn(move || {
                        // A spread of values, so different threads write
                        // different bucket suffixes and the descending write
                        // order is actually exercised.
                        for i in 0..PER_THREAD {
                            let value = BUCKETS[i % BUCKETS.len()];
                            r.observe(HTTP_REQUEST_DURATION_SECONDS, labels, value)
                                .expect("a histogram");
                        }
                    })
                })
                .collect();

            for worker in workers {
                worker.join().expect("a recording thread panicked");
            }
            stop.store(true, Ordering::Relaxed);
            let scrapes = scraper.join().expect("the scraping thread panicked");
            assert!(
                scrapes > 0,
                "the scraper never ran; the test proved nothing"
            );
        });

        let out = r.render();
        let total = THREADS * PER_THREAD;
        assert!(
            out.contains(&format!(
                "http_request_duration_seconds_count{{method=\"GET\",route=\"/health/live\"}} {total}\n"
            )),
            "lost an observation:\n{out}"
        );
        assert!(
            out.contains(&format!(
                "http_request_duration_seconds_bucket{{method=\"GET\",route=\"/health/live\",le=\"+Inf\"}} {total}\n"
            )),
            "+Inf disagrees with _count:\n{out}"
        );
    }

    /// Every histogram family in a body has non-decreasing cumulative counts.
    fn assert_buckets_never_decrease(body: &str) {
        let mut previous = 0_u64;
        for line in body.lines() {
            let Some(rest) = line.split_once("_bucket{") else {
                previous = 0;
                continue;
            };
            let count: u64 = rest
                .1
                .rsplit(' ')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or_else(|| panic!("unparseable bucket line: {line}"));
            assert!(
                count >= previous,
                "a scrape caught a histogram going backwards ({previous} then {count}):\n{body}"
            );
            previous = count;
        }
    }
}
