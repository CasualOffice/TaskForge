use super::*;

#[test]
fn the_defaults_are_bounded_and_ordered_sensibly() {
    let c = Config::default();
    assert!(c.batch > 0 && c.concurrency > 0);
    // Concurrency above the batch size cannot be used: a poll never has
    // more than `batch` deliveries to run at once.
    assert!(
        c.concurrency as i64 <= c.batch,
        "concurrency exceeds the batch size, so the extra permits are dead"
    );
    // docs/34 bounds a webhook at 30 s. A drain shorter than that abandons
    // deliveries that were about to succeed; longer than the orchestrator's
    // grace period and the process is SIGKILLed mid-drain instead.
    assert!(c.drain >= Duration::from_secs(10));
    assert!(c.drain < Duration::from_secs(30));
}

#[test]
fn the_gauges_are_sampled_far_less_often_than_the_loop_polls() {
    // The defect this bound exists to prevent: `outbox_lag_seconds` is an
    // aggregate over the pending set and `outbox_dlq_depth` over the
    // dead-lettered one, and both were read once per poll. Under a backlog
    // the loop polls with no sleep at all, so the two most expensive queries
    // in the dispatch path ran at the highest rate exactly when the sets
    // they scan were largest.
    let c = Config::default();
    assert!(
        c.metrics_interval >= c.idle * 4,
        "sampling every {:?} against an idle poll of {:?} is not a cadence, \
             it is the poll rate with extra steps",
        c.metrics_interval,
        c.idle
    );
    // And the other direction: RB-01 pages on a 5-minute window and RB-02 on
    // a 15-minute one. A sampling interval near those would delay the page
    // it feeds.
    assert!(
        c.metrics_interval < Duration::from_secs(60),
        "a gauge stale by {:?} is read by an alert that evaluates over five \
             minutes",
        c.metrics_interval
    );
}

#[test]
fn the_drain_is_shorter_than_the_claim_expiry() {
    // Otherwise a drain could still be running when another worker becomes
    // entitled to reclaim the same rows, turning shutdown into a guaranteed
    // double delivery rather than a rare one.
    let drain = Config::default().drain.as_secs() as i64;
    assert!(
        drain < dispatch::CLAIM_EXPIRY.whole_seconds(),
        "drain {drain}s is not shorter than the claim expiry"
    );
}
