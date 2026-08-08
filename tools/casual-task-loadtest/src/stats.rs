//! Percentile computation.
//!
//! The percentile *definition* is part of the report's schema version, not an
//! implementation detail: two runs computed with different definitions are not
//! comparable, and the gate compares numbers across months. Changing the method
//! below is a `schemaVersion` bump (see [`crate::report::SCHEMA_VERSION`]).
//!
//! The method is **nearest-rank**: the p-th percentile is the smallest observed
//! sample at or above rank `ceil(p/100 · n)`. It is chosen over linear
//! interpolation because every reported number is then a value that was
//! actually observed — an interpolated p99 of 8.4 ms when no sample was between
//! 6 ms and 11 ms invites exactly the wrong conclusion about what a user waited.

/// Summary of one case's samples, in microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub min_us: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
    pub mean_us: u64,
}

/// Smallest number of samples at which the p99 nearest-rank estimate has at
/// least ten samples in the tail it summarises. Below this the p99 is reported
/// but flagged, because a single scheduling hiccup moves it entirely.
pub const P99_CONFIDENCE_MIN_SAMPLES: usize = 1_000;

/// Smallest number of samples at which the p95 estimate is worth gating on:
/// 200 samples puts ten of them above the p95, so one outlier cannot set it.
pub const P95_CONFIDENCE_MIN_SAMPLES: usize = 200;

/// Summarise raw samples. Returns `None` for an empty input rather than
/// inventing a zero, because a case that produced no samples must surface as a
/// missing case in the report and fail the gate, not as a suspiciously fast one.
pub fn summarise(samples: &[u64]) -> Option<Summary> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();

    // Sum cannot overflow u64 in any plausible run: u64::MAX microseconds is
    // ~584,000 years, and the sample count is bounded by --iterations.
    let sum: u128 = sorted.iter().map(|&v| u128::from(v)).sum();
    let mean = (sum / sorted.len() as u128) as u64;

    Some(Summary {
        min_us: sorted[0],
        p50_us: nearest_rank(&sorted, 50.0),
        p95_us: nearest_rank(&sorted, 95.0),
        p99_us: nearest_rank(&sorted, 99.0),
        max_us: sorted[sorted.len() - 1],
        mean_us: mean,
    })
}

/// Nearest-rank percentile over an already-sorted slice. `percentile` is in
/// `0.0..=100.0`. Panics only on an empty slice, which callers exclude.
fn nearest_rank(sorted: &[u64], percentile: f64) -> u64 {
    debug_assert!(!sorted.is_empty());
    let n = sorted.len();
    let rank = (percentile / 100.0 * n as f64).ceil() as usize;
    // rank is 1-based; clamp guards both p=0 (rank 0) and float rounding at
    // p=100 producing n+1.
    let index = rank.clamp(1, n) - 1;
    sorted[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_has_no_summary() {
        assert_eq!(summarise(&[]), None);
    }

    #[test]
    fn single_sample_is_every_percentile() {
        let s = summarise(&[42]).expect("one sample summarises");
        assert_eq!(s.min_us, 42);
        assert_eq!(s.p50_us, 42);
        assert_eq!(s.p95_us, 42);
        assert_eq!(s.p99_us, 42);
        assert_eq!(s.max_us, 42);
        assert_eq!(s.mean_us, 42);
    }

    #[test]
    fn nearest_rank_matches_the_textbook_definition() {
        // 1..=100: nearest rank p50 = element 50, p95 = 95, p99 = 99.
        let samples: Vec<u64> = (1..=100).collect();
        let s = summarise(&samples).expect("summary");
        assert_eq!(s.p50_us, 50);
        assert_eq!(s.p95_us, 95);
        assert_eq!(s.p99_us, 99);
        assert_eq!(s.min_us, 1);
        assert_eq!(s.max_us, 100);
        assert_eq!(s.mean_us, 50); // 5050/100 = 50.5, truncated
    }

    #[test]
    fn every_reported_percentile_is_an_observed_sample() {
        // The property that distinguishes nearest-rank from interpolation.
        let samples = vec![1, 1, 1, 1, 1, 1, 1, 1, 1, 900];
        let s = summarise(&samples).expect("summary");
        assert!(samples.contains(&s.p50_us));
        assert!(samples.contains(&s.p95_us));
        assert!(samples.contains(&s.p99_us));
        assert_eq!(s.p95_us, 900, "the outlier is in the top 5% of ten samples");
    }

    #[test]
    fn input_order_does_not_change_the_summary() {
        let ascending: Vec<u64> = (1..=500).collect();
        let mut shuffled = ascending.clone();
        shuffled.reverse();
        assert_eq!(summarise(&ascending), summarise(&shuffled));
    }

    #[test]
    fn summarise_does_not_mutate_the_caller_slice() {
        let samples = vec![9, 3, 7];
        let _ = summarise(&samples);
        assert_eq!(samples, vec![9, 3, 7]);
    }
}
