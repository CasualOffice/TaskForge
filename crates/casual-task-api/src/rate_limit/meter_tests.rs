use super::*;
use crate::rate_limit::class::AUTH;
use axum::http::HeaderValue;

fn headers_from(ip: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", HeaderValue::from_str(ip).expect("valid"));
    headers
}
#[test]
fn a_burst_is_spent_and_then_refused() {
    let limiter = RateLimiter::per_ip(AUTH);
    let now = Instant::now();
    let headers = headers_from("203.0.113.9");

    for spent in 1..=AUTH.burst {
        let decision = limiter.check_at(&headers, now);
        assert!(decision.allowed, "refused request {spent} of the burst");
        assert_eq!(decision.remaining, AUTH.burst - spent);
        assert_eq!(decision.retry_after_seconds, None);
    }

    let refused = limiter.check_at(&headers, now);
    assert!(!refused.allowed, "the burst was not a limit");
    assert_eq!(refused.remaining, 0);
    assert_eq!(
        refused.retry_after_seconds,
        Some(6),
        "10/min is one token per six seconds"
    );
}

#[test]
fn one_address_cannot_spend_anothers_tokens() {
    // The property that makes this worth having at all. A shared bucket
    // would mean one attacker locks every user out of logging in — the
    // failure mode docs/21 names for a per-account-only limit, inverted.
    let limiter = RateLimiter::per_ip(AUTH);
    let now = Instant::now();
    let attacker = headers_from("203.0.113.9");
    let ordinary = headers_from("198.51.100.4");

    for _ in 0..AUTH.burst + 3 {
        let _ = limiter.check_at(&attacker, now);
    }
    assert!(!limiter.check_at(&attacker, now).allowed);

    let decision = limiter.check_at(&ordinary, now);
    assert!(
        decision.allowed,
        "a second address was refused because the first exhausted its bucket"
    );
    assert_eq!(decision.remaining, AUTH.burst - 1);
}

#[test]
fn the_bucket_refills_at_the_documented_rate() {
    let limiter = RateLimiter::per_ip(AUTH);
    let start = Instant::now();
    let headers = headers_from("203.0.113.9");

    for _ in 0..AUTH.burst {
        assert!(limiter.check_at(&headers, start).allowed);
    }
    assert!(!limiter.check_at(&headers, start).allowed);

    // Five seconds is not yet a token at 10/min.
    assert!(
        !limiter
            .check_at(&headers, start + Duration::from_secs(5))
            .allowed,
        "a token appeared before six seconds had passed"
    );
    // Six is.
    assert!(
        limiter
            .check_at(&headers, start + Duration::from_secs(6))
            .allowed,
        "the bucket never refilled: one refusal would be permanent"
    );

    // And a full window restores the whole burst, not just one token.
    let later = start + AUTH.full_refill() + Duration::from_secs(60);
    for spent in 1..=AUTH.burst {
        assert!(
            limiter.check_at(&headers, later).allowed,
            "only {spent} of the burst came back after a full window"
        );
    }
}

#[test]
fn sustained_throughput_matches_the_documented_rate() {
    // The other half of "10 / min": after the burst, a client that keeps
    // asking gets ten per minute and not eleven.
    let limiter = RateLimiter::per_ip(AUTH);
    let start = Instant::now();
    let headers = headers_from("203.0.113.9");
    for _ in 0..AUTH.burst {
        assert!(limiter.check_at(&headers, start).allowed);
    }

    let mut allowed = 0;
    for second in 1..=60 {
        if limiter
            .check_at(&headers, start + Duration::from_secs(second))
            .allowed
        {
            allowed += 1;
        }
    }
    assert_eq!(
        allowed, AUTH.sustained,
        "a drained bucket admitted {allowed} requests in the following minute"
    );
}

#[test]
fn requests_without_a_usable_address_share_one_bucket() {
    // Stated as a test because it is a real cost, not an accident: with no
    // X-Forwarded-For there is nothing to key on, and the safe direction is
    // one shared bucket rather than an exemption an attacker can ask for by
    // omitting a header.
    let limiter = RateLimiter::per_ip(AUTH);
    let now = Instant::now();
    let none = HeaderMap::new();
    let mut garbage = HeaderMap::new();
    garbage.insert(
        "x-forwarded-for",
        HeaderValue::from_static("not-an-address"),
    );

    for _ in 0..AUTH.burst {
        assert!(limiter.check_at(&none, now).allowed);
    }
    assert!(
        !limiter.check_at(&garbage, now).allowed,
        "an unparseable address got its own bucket, so sending garbage is a bypass"
    );
}

#[test]
fn the_first_hop_is_what_is_keyed_on() {
    let limiter = RateLimiter::per_ip(AUTH);
    let now = Instant::now();
    let mut chained = HeaderMap::new();
    chained.insert(
        "x-forwarded-for",
        HeaderValue::from_static("203.0.113.9, 198.51.100.4"),
    );

    for _ in 0..AUTH.burst {
        assert!(limiter.check_at(&chained, now).allowed);
    }
    assert!(!limiter.check_at(&chained, now).allowed);
    // The same first hop through a different proxy chain is the same client.
    let mut other_chain = HeaderMap::new();
    other_chain.insert(
        "x-forwarded-for",
        HeaderValue::from_static("203.0.113.9, 192.0.2.7"),
    );
    assert!(
        !limiter.check_at(&other_chain, now).allowed,
        "changing a later hop created a new bucket, which any client can do"
    );
}

#[test]
fn the_tracked_map_never_exceeds_its_cap() {
    // The memory-exhaustion primitive, asserted away. Spraying addresses
    // must cost the attacker a packet each and cost us nothing unbounded.
    let limiter = RateLimiter::per_ip(AUTH);
    let now = Instant::now();
    for n in 0..(MAX_TRACKED_KEYS as u64 + 5_000) {
        let ip = IpAddr::from(((n as u32) | 0x0100_0000).to_be_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_str(&ip.to_string()).expect("valid"),
        );
        let _ = limiter.check_at(&headers, now);
    }

    let state = limiter.state.lock().expect("not poisoned");
    assert!(
        state.tracked.len() <= MAX_TRACKED_KEYS,
        "the limiter tracked {} keys; the cap is {MAX_TRACKED_KEYS}",
        state.tracked.len()
    );
    assert!(
        state.overflow.is_some(),
        "the cap was reached but nothing was charged to the overflow bucket, \
             so the excess went unlimited"
    );
}

#[test]
fn an_already_tracked_client_keeps_its_bucket_when_the_map_is_full() {
    // The overflow policy must not let a flood of new addresses reset an
    // attacker's own bucket by evicting it.
    let limiter = RateLimiter::per_ip(AUTH);
    let now = Instant::now();
    let known = headers_from("203.0.113.9");
    for _ in 0..AUTH.burst {
        assert!(limiter.check_at(&known, now).allowed);
    }

    for n in 0..(MAX_TRACKED_KEYS as u64 + 1_000) {
        let ip = IpAddr::from(((n as u32) | 0x0a00_0000).to_be_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_str(&ip.to_string()).expect("valid"),
        );
        let _ = limiter.check_at(&headers, now);
    }

    assert!(
        !limiter.check_at(&known, now).allowed,
        "a flood of new addresses restored an exhausted client's bucket"
    );
}

#[test]
fn a_full_bucket_is_swept_because_it_carries_no_information() {
    // Overflow policy step 1. A bucket that has fully refilled decides
    // exactly what a brand new one would, so dropping it is free — and it is
    // what keeps the map from staying saturated after a flood ends.
    let limiter = RateLimiter::per_ip(AUTH);
    let start = Instant::now();
    for n in 0..MAX_TRACKED_KEYS as u64 {
        let ip = IpAddr::from(((n as u32) | 0x0a00_0000).to_be_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_str(&ip.to_string()).expect("valid"),
        );
        let _ = limiter.check_at(&headers, start);
    }
    assert_eq!(
        limiter.state.lock().expect("not poisoned").tracked.len(),
        MAX_TRACKED_KEYS
    );

    // Long enough that every one of them has refilled completely.
    let later = start + AUTH.full_refill() + SWEEP_INTERVAL + Duration::from_secs(1);
    let fresh = headers_from("203.0.113.9");
    assert!(limiter.check_at(&fresh, later).allowed);

    let state = limiter.state.lock().expect("not poisoned");
    assert!(
        state.tracked.len() < MAX_TRACKED_KEYS,
        "the sweep freed nothing, so the map stays saturated forever after \
             one flood"
    );
}
