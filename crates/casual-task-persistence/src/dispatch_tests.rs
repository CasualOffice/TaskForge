use super::*;

#[test]
fn the_backoff_ladder_matches_the_design_record() {
    // docs/25 §Retry and dead-letter, verbatim: 1 s, 4 s, 16 s, 1 m, 5 m,
    // 30 m, then the dead-letter queue.
    let seconds: Vec<i64> = BACKOFF.iter().map(|d| d.whole_seconds()).collect();
    assert_eq!(seconds, vec![1, 4, 16, 60, 300, 1800]);
    assert_eq!(BACKOFF.len(), 6, "six attempts, then dead-letter");
}

#[test]
fn the_ladder_is_monotonic() {
    // A ladder that went backwards would retry a failing consumer harder
    // the longer it stayed broken.
    for pair in BACKOFF.windows(2) {
        assert!(pair[1] > pair[0], "{pair:?} is not increasing");
    }
}

#[test]
fn the_claim_expiry_exceeds_the_longest_plausible_consumer_timeout() {
    // docs/25: longer than any consumer timeout. docs/34 bounds a plugin
    // call at 500 ms and a webhook at 30 s; five minutes clears both by an
    // order of magnitude, which is what stops a slow-but-alive worker from
    // having its work stolen.
    assert!(CLAIM_EXPIRY > time::Duration::seconds(30));
}

#[test]
fn retention_is_long_enough_for_the_runbook_that_reads_history() {
    // RB-01 step 2 reads 30 minutes of dispatch history to decide whether a
    // backlog is draining. A sweep on success would leave it nothing.
    assert!(RETENTION > time::Duration::hours(1));
    assert_eq!(RETENTION.whole_days(), 7, "docs/25 says seven days");
}

#[test]
fn intervals_are_emitted_in_seconds() {
    // Not "1 month" or "30 minutes" — a seconds literal has no locale or
    // month-length ambiguity.
    assert_eq!(pg_interval(time::Duration::minutes(5)), "300 seconds");
    assert_eq!(pg_interval(CLAIM_EXPIRY), "300 seconds");
}
