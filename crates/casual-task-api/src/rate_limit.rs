//! Per-IP rate limiting for the authentication endpoints (`docs/21` §Rate
//! limits, `docs/40` §Local authentication).
//!
//! # The hole this fills
//!
//! `casual-task-identity`'s password backoff is **per account**. It slows an
//! attacker who guesses one account's password repeatedly, and it does nothing
//! at all against credential stuffing — one attempt each against ten thousand
//! accounts never increments any single account's counter. `docs/40` says
//! "rate limited per account **and** per IP"; this is the per-IP half, and until
//! it existed `POST /api/v1/auth/login` had no limit of any kind.
//!
//! # The numbers are `docs/21`'s, not this module's
//!
//! Auth class: **10 / min sustained, burst 5** — see [`AUTH`]. A token bucket,
//! as `docs/21` §Rate limits specifies: the bucket holds `burst` tokens and
//! refills at `sustained` per minute, so a client may spend five immediately and
//! then one every six seconds.
//!
//! # In-process only, and what that means for an operator
//!
//! State lives in this process. `docs/48` Profile 1 is one binary and PostgreSQL
//! with **no Redis**, and it must work — so a shared limiter is not an option
//! here.
//!
//! **With more than one API instance the limit is per instance.** Two instances
//! behind a round-robin load balancer admit up to twice the configured rate; N
//! instances admit N times. `docs/48` already says Redis becomes *required* at
//! ≥ 2 API instances "because rate limits and SSE fan-out need shared state",
//! and this module is the reason the first half of that sentence is true. An
//! operator running Profile 2 without Redis has a limiter that is weaker by
//! exactly their instance count, and nothing will tell them so at runtime.
//!
//! # The state is bounded, because otherwise it is a weapon
//!
//! A map keyed by client IP that grows without limit is a memory-exhaustion
//! primitive handed to the attacker the limiter exists to stop: spraying
//! addresses costs them one packet each and costs us an allocation each.
//!
//! So the map is capped at [`MAX_TRACKED_KEYS`], and the **overflow policy** is
//! stated rather than implied:
//!
//! 1. When the map is full, entries whose bucket has fully refilled are dropped.
//!    That is free, not a heuristic: a full bucket is indistinguishable from one
//!    that was never created, so forgetting it changes no decision.
//! 2. If the map is still full after that sweep, a request whose key is not
//!    already tracked is charged to a single **shared overflow bucket**.
//!
//! The cost of step 2, plainly: while the map is saturated, previously-unseen
//! clients share one bucket and can throttle each other. That is the deliberate
//! direction — an attacker rotating through addresses is collectively limited
//! rather than being handed unlimited attempts *and* unlimited memory — but a
//! legitimate client arriving during such a flood can be refused. Reaching that
//! state requires [`MAX_TRACKED_KEYS`] distinct addresses to have each spent a
//! token inside one refill window, which is an attack, not a login peak.
//!
//! Sweeping is rate-limited to once per [`SWEEP_INTERVAL`] so a saturated map
//! cannot turn every request into an O(n) scan.
//!
//! # Why there is no new dependency
//!
//! `docs/48`'s table says in-process limits "fall back to in-process (moka)".
//! What is needed here is a bounded map with a documented overflow policy and a
//! token bucket — about eighty lines — and a new crate would have to clear
//! `cargo deny check licenses` to buy it. The deviation from that parenthetical
//! is deliberate and is recorded here rather than left to be discovered.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, State};
use axum::http::{HeaderMap, HeaderValue, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use casual_task_observability::labels::{LabelSet, keys};
use casual_task_observability::metrics::RATE_LIMIT_HITS_TOTAL;
use casual_task_observability::recorder::Recorder;

use crate::error::ApiError;
use crate::server::RequestId;

/// One rate-limit class from `docs/21` §Rate limits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Class {
    /// Tokens added per minute — the table's "Sustained" column.
    pub sustained_per_minute: u32,
    /// Bucket capacity — the table's "Burst" column.
    pub burst: u32,
}

impl Class {
    /// The gap between two tokens: 60 s / sustained. Six seconds for [`AUTH`].
    ///
    /// Integer nanoseconds, not a rate in floating point. The first version of
    /// this module carried `tokens: f64` and refilled by `elapsed * rate`, and
    /// `6 s × (10/60)` is `0.999999999999999`, so the token that `docs/21` says
    /// arrives after six seconds did not arrive — the limit was silently
    /// stricter than the document, in a way no reading of the code would show.
    #[must_use]
    pub fn emission_interval(self) -> Duration {
        Duration::from_secs(60) / self.sustained_per_minute.max(1)
    }

    /// How far ahead of real time the meter may run — the burst, expressed as
    /// time. Also how long an emptied bucket takes to come all the way back,
    /// which is what `RateLimit-Reset` counts down to.
    #[must_use]
    pub fn full_refill(self) -> Duration {
        self.emission_interval() * self.burst
    }
}

/// `docs/21`: "Auth (login, reset) | 10 / min **per IP and per account** | 5".
pub const AUTH: Class = Class {
    sustained_per_minute: 10,
    burst: 5,
};

/// The routes the auth class governs.
///
/// A closed list, matched against the router's own `MatchedPath` template. A
/// request that matched no route is not governed — it is a 404, and 404s are
/// not what this limiter is for.
///
/// Health endpoints are **absent on purpose**. An orchestrator probes
/// `/health/live` and `/health/ready` every second or two; throttling those
/// turns the limiter into the outage it was added to prevent, because a
/// liveness probe that returns 429 gets the container restarted.
pub const LIMITED_ROUTES: &[&str] = &["/api/v1/auth/login"];

/// The most distinct keys tracked at once. See the module docs for the overflow
/// policy.
///
/// 65,536 entries is roughly 6 MB at this entry size — small enough to be
/// uninteresting, large enough that only an address-rotating flood reaches it.
pub const MAX_TRACKED_KEYS: usize = 65_536;

/// The shortest gap between two sweeps of a saturated map.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

/// What a client is limited by.
///
/// Only [`Scope::Ip`] exists today. The per-workspace and per-actor classes in
/// `docs/21` need an authenticated actor, and they are **not implemented** —
/// said plainly rather than left to be inferred from a `scope_kind` label that
/// only ever reports one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Keyed on the client address.
    Ip,
}

impl Scope {
    /// The `scope_kind` metric label (`docs/46`). A closed set, which is why it
    /// can be a `&'static str` and therefore a legal [`LabelValue`].
    ///
    /// [`LabelValue`]: casual_task_observability::labels::LabelValue
    const fn label(self) -> &'static str {
        match self {
            Self::Ip => "ip",
        }
    }
}

/// Who a bucket belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Ip(IpAddr),
    /// No usable client address: no `X-Forwarded-For`, or one that did not
    /// parse.
    ///
    /// Every such request shares one bucket. That is the safe direction — an
    /// attacker cannot escape the limiter by omitting a header — but it has a
    /// real cost on a deployment with no reverse proxy in front: all direct
    /// clients share a single auth bucket and can throttle each other.
    /// `docs/48` puts a reverse proxy ahead of the API in every profile, and
    /// this is one of the things that assumption is buying.
    Unattributed,
}

/// One client's token bucket, stored as the single instant a token bucket can
/// be reduced to.
///
/// This is the GCRA formulation: instead of a token count that has to be
/// refilled on a schedule, keep the **theoretical arrival time** — the moment
/// the meter would be empty if every token spent so far were spread out at the
/// emission interval. `tat <= now` means full; `tat - now` is how long until it
/// is full again; and a request is admitted when spending it would not push the
/// meter more than [`Class::full_refill`] ahead of now.
///
/// One `Instant` per tracked client, and every operation is integer duration
/// arithmetic — which is the point. It is the same limit as a refilling
/// counter, without the rounding that made the counter disagree with `docs/21`.
#[derive(Debug, Clone, Copy)]
struct Meter {
    tat: Instant,
}

impl Meter {
    /// A client that has spent nothing.
    fn full(now: Instant) -> Self {
        Self { tat: now }
    }

    /// Whether this meter carries no information — see overflow policy step 1.
    /// A fully-decayed meter decides exactly what a brand new one would.
    fn is_full(self, now: Instant) -> bool {
        self.tat <= now
    }

    /// How long until the meter is fully decayed.
    fn until_full(self, now: Instant) -> Duration {
        self.tat.saturating_duration_since(now)
    }
}

/// What the limiter decided, and the numbers the response headers need.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    /// Whether the request may proceed.
    pub allowed: bool,
    /// Whole tokens left after this request.
    pub remaining: u32,
    /// Seconds until the bucket is full again — `RateLimit-Reset`.
    pub reset_seconds: u32,
    /// Seconds until at least one token exists. `Some` exactly when the request
    /// was refused, so a 429 cannot be built without it.
    pub retry_after_seconds: Option<u32>,
}

/// The limiter for one class.
#[derive(Debug)]
pub struct RateLimiter {
    class: Class,
    scope: Scope,
    state: Mutex<Buckets>,
}

#[derive(Debug)]
struct Buckets {
    tracked: HashMap<Key, Meter>,
    /// Where requests go when [`MAX_TRACKED_KEYS`] is reached.
    overflow: Option<Meter>,
    last_sweep: Option<Instant>,
}

impl RateLimiter {
    /// A limiter for `class`, keyed by client address.
    #[must_use]
    pub fn per_ip(class: Class) -> Self {
        Self {
            class,
            scope: Scope::Ip,
            state: Mutex::new(Buckets {
                tracked: HashMap::new(),
                overflow: None,
                last_sweep: None,
            }),
        }
    }

    /// What this limiter keys on, for the `scope_kind` metric label.
    #[must_use]
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// Spend a token for `headers`' client, at `now`.
    ///
    /// `now` is a parameter rather than read inside, so the window behaviour is
    /// testable without a test that sleeps for it. A test that waits out a real
    /// six-second window is a test that gets marked `#[ignore]` and then stops
    /// being run.
    pub fn check_at(&self, headers: &HeaderMap, now: Instant) -> Decision {
        let key = client_ip(headers).map_or(Key::Unattributed, Key::Ip);
        let class = self.class;
        // A std Mutex, not tokio's: the critical section is float arithmetic on
        // one entry and there is no await inside it. A tokio mutex here would
        // add a scheduler interaction to every login for no gain.
        let mut state = self.state.lock().unwrap_or_else(|poisoned| {
            // A panic inside the arithmetic above would poison this forever and
            // take every login down with it. The state is a cache of counters,
            // not a ledger: recovering is strictly better than refusing.
            poisoned.into_inner()
        });

        let meter = state.meter_for(key, now);

        // Spending one token moves the meter forward by one emission interval,
        // starting from now if it had already decayed past it.
        let spent = meter.tat.max(now) + class.emission_interval();
        // The request is admitted unless that would put the meter further ahead
        // than the burst allows. `checked_sub` because `Instant` has no epoch
        // guarantee: on a process younger than the tolerance the subtraction
        // would underflow, and the honest reading of that is "nothing has been
        // spent yet", not a panic.
        let earliest = spent.checked_sub(class.full_refill());
        let allowed = earliest.is_none_or(|earliest| earliest <= now);

        if allowed {
            meter.tat = spent;
        }
        let until_full = meter.until_full(now);
        let retry_after = earliest.map(|earliest| earliest.saturating_duration_since(now));
        drop(state);

        Decision {
            allowed,
            // Whole tokens left: how much of the tolerance the meter is not
            // currently using, in emission intervals. Zero on a refusal by
            // construction — there was no token to give.
            remaining: if allowed {
                whole_intervals(class.full_refill().saturating_sub(until_full), class)
            } else {
                0
            },
            reset_seconds: whole_seconds_up(until_full),
            // `Some` exactly when refused, so `ApiError::too_many_requests`
            // cannot be reached without a value. At least one second: a
            // Retry-After of 0 tells a client to retry immediately, which is the
            // flood this exists to stop.
            retry_after_seconds: (!allowed)
                .then(|| whole_seconds_up(retry_after.unwrap_or_default()).max(1)),
        }
    }
}

impl Buckets {
    /// The meter this key should be charged against, applying the overflow
    /// policy in the module docs.
    fn meter_for(&mut self, key: Key, now: Instant) -> &mut Meter {
        if self.tracked.contains_key(&key) {
            return self.tracked.get_mut(&key).expect("checked above");
        }

        if self.tracked.len() >= MAX_TRACKED_KEYS {
            // Step 1: drop what carries no information. Rate-limited, so a
            // saturated map does not make every request an O(n) scan.
            let due = self
                .last_sweep
                .is_none_or(|at| now.saturating_duration_since(at) >= SWEEP_INTERVAL);
            if due {
                self.tracked.retain(|_, meter| !meter.is_full(now));
                self.last_sweep = Some(now);
            }
        }

        if self.tracked.len() >= MAX_TRACKED_KEYS {
            // Step 2: the shared meter. Bounded memory, and an address-rotating
            // flood is limited collectively rather than not at all.
            return self.overflow.get_or_insert_with(|| Meter::full(now));
        }

        self.tracked.entry(key).or_insert_with(|| Meter::full(now))
    }
}

/// How many whole emission intervals fit in `spare` — whole tokens.
fn whole_intervals(spare: Duration, class: Class) -> u32 {
    let interval = class.emission_interval().as_nanos();
    if interval == 0 {
        return 0;
    }
    u32::try_from(spare.as_nanos() / interval).unwrap_or(u32::MAX)
}

/// `d` in whole seconds, rounded **up**.
///
/// Up, not down: a `Retry-After` of 5 for a 5.4-second wait invites a retry that
/// is refused again, and a client that trusts the header then treats the second
/// refusal as the server misbehaving.
fn whole_seconds_up(d: Duration) -> u32 {
    let seconds = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    u32::try_from(seconds).unwrap_or(u32::MAX)
}

/// The client IP from `X-Forwarded-For`, or `None`.
///
/// The first hop only, and only when it parses as an address — the header is
/// attacker-controlled and a two-hop chain sends
/// `X-Forwarded-For: 203.0.113.9, 198.51.100.4`.
///
/// **This duplicates `crate::auth::client_ip`,** which is private to that module
/// and cannot be called from here. The duplication is deliberate and temporary:
/// `auth.rs` is being edited concurrently, so widening that function's
/// visibility would be a conflict for no behavioural gain today. The two must
/// not drift — the follow-up is to delete the copy in `auth.rs` and call this
/// one, which is the direction that leaves the parser in a module that is not
/// about login.
///
/// A spoofed value is not a security problem for this limiter in the way it
/// would be for authorization: an attacker who rotates the header rotates
/// through buckets, which is exactly what [`MAX_TRACKED_KEYS`] and the overflow
/// bucket bound. It is not usable as an identity and is not used as one.
#[must_use]
pub fn client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let raw = headers.get("x-forwarded-for")?.to_str().ok()?;
    raw.split(',').next()?.trim().parse::<IpAddr>().ok()
}

/// The state the layer carries: the limiter, and somewhere to report to.
#[derive(Clone)]
#[allow(missing_debug_implementations)]
pub struct RateLimitState {
    pub limiter: Arc<RateLimiter>,
    pub metrics: Arc<Recorder>,
}

impl RateLimitState {
    /// The auth-class limiter, per `docs/21`.
    #[must_use]
    pub fn auth(metrics: Arc<Recorder>) -> Self {
        Self {
            limiter: Arc::new(RateLimiter::per_ip(AUTH)),
            metrics,
        }
    }
}

/// The middleware. Governs [`LIMITED_ROUTES`] and passes everything else
/// through untouched.
pub async fn rate_limit(
    State(state): State<RateLimitState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // The route TEMPLATE from the router, not the request path: a limiter that
    // matched on attacker-supplied strings would be bypassable by a trailing
    // slash. A request that matched no route is a 404 and is not governed.
    let governed = request
        .extensions()
        .get::<MatchedPath>()
        .is_some_and(|path| LIMITED_ROUTES.contains(&path.as_str()));
    if !governed {
        return next.run(request).await;
    }

    let request_id = RequestId::of_request(&request);
    let decision = state.limiter.check_at(request.headers(), Instant::now());

    let mut response = if let Some(retry_after) = decision.retry_after_seconds {
        record_hit(&state.metrics, state.limiter.scope());
        // docs/40 §What is audited: a burst of refusals is the clearest signal
        // of an attack. The address is deliberately absent from the message and
        // present in the log, where docs/46 allows an identifier.
        tracing::warn!(
            scope = state.limiter.scope().label(),
            retry_after,
            "a request was rate limited"
        );
        ApiError::too_many_requests(&request_id, retry_after).into_response()
    } else {
        next.run(request).await
    };

    // docs/05 §Rate limiting: "Returned on success too, so a client can slow
    // down *before* being throttled."
    let headers = response.headers_mut();
    headers.insert(
        "ratelimit-limit",
        HeaderValue::from(AUTH.sustained_per_minute),
    );
    headers.insert("ratelimit-remaining", HeaderValue::from(decision.remaining));
    headers.insert("ratelimit-reset", HeaderValue::from(decision.reset_seconds));
    response
}

/// `docs/46` §Domain metrics: `rate_limit_hits_total` by limiter scope.
///
/// `scope_kind` only. The metric also declares `workspace_bucket` and
/// `workspace_investigation`, and neither is knowable here: this limiter runs
/// before authentication, on an endpoint whose entire purpose is that the caller
/// has no identity yet. Attaching a made-up tenant would be worse than omitting
/// one.
fn record_hit(metrics: &Recorder, scope: Scope) {
    match LabelSet::for_metric(RATE_LIMIT_HITS_TOTAL).with(keys::SCOPE_KIND, scope.label()) {
        Ok(labels) => {
            if let Err(error) = metrics.increment(RATE_LIMIT_HITS_TOTAL, &labels, 1) {
                tracing::error!(%error, "recording a rate limit hit");
            }
        }
        // Unreachable: scope_kind is declared on this metric and the value is a
        // &'static str. Logged rather than unwrapped — a metric is not worth
        // turning a 429 into a panic.
        Err(error) => tracing::warn!(%error, "rate limit label rejected"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_from(ip: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_str(ip).expect("valid"));
        headers
    }

    #[test]
    fn the_class_is_the_one_docs_21_names() {
        // The numbers are the design record's, and a test is what keeps them
        // from drifting into "whatever felt right during a refactor".
        assert_eq!(
            AUTH.sustained_per_minute, 10,
            "docs/21 §Rate limits: 10/min"
        );
        assert_eq!(AUTH.burst, 5, "docs/21 §Rate limits: burst 5");
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
            allowed, AUTH.sustained_per_minute,
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

    #[test]
    fn health_routes_are_not_in_the_governed_set() {
        // An orchestrator probes these every second. A 429 on a liveness probe
        // is a container restart, which turns a limiter into an outage.
        assert!(!LIMITED_ROUTES.contains(&"/health/live"));
        assert!(!LIMITED_ROUTES.contains(&"/health/ready"));
        assert!(!LIMITED_ROUTES.contains(&"/metrics"));
        assert!(
            LIMITED_ROUTES.contains(&"/api/v1/auth/login"),
            "the endpoint this exists for is not governed"
        );
    }

    #[test]
    fn no_route_outside_the_auth_class_is_governed_by_it() {
        // The auth class is 10/min. docs/21 gives reads 1,000/min and writes
        // 300/min, and putting a project list or a task read under the auth
        // bucket would throttle the product into uselessness while looking like
        // a security improvement. Those classes are keyed per (workspace, actor)
        // and are not implemented; until they are, everything outside auth is
        // deliberately ungoverned, and this test is what stops a route being
        // added to the wrong list by reflex.
        for route in LIMITED_ROUTES {
            assert!(
                route.starts_with("/api/v1/auth/"),
                "{route} is under the auth class (10/min); docs/21 gives \
                 non-auth endpoints their own, much larger, per-(workspace, \
                 actor) limits"
            );
        }
    }

    #[test]
    fn every_governed_route_is_one_the_router_serves() {
        // A typo here is a limit that silently governs nothing.
        for route in LIMITED_ROUTES {
            assert!(
                crate::server::ROUTES.contains(route),
                "{route} is rate limited but is not a route this server serves"
            );
        }
    }
}
