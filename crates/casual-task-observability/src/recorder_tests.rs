use std::sync::atomic::AtomicBool;

use super::*;
use crate::labels::keys;
use crate::metrics::{HTTP_REQUEST_DURATION_SECONDS, OUTBOX_DISPATCH_TOTAL, OUTBOX_LAG_SECONDS};

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
    assert!(
        out.contains(
            "outbox_dispatch_total{consumer=\"webhook_delivery\",outcome=\"dispatched\"} 3"
        )
    );
    assert!(
        out.contains("outbox_dispatch_total{consumer=\"webhook_delivery\",outcome=\"failed\"} 1")
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
