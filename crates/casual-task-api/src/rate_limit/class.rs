//! The published limits, and which class a request belongs to
//! (`docs/21` §Rate limits).
//!
//! # The failure this module prevents
//!
//! A limit that disagrees with the document that publishes it. `docs/21` is a
//! contract customers read; every number below is transcribed from its table
//! and pinned by a test that names the row it came from, so a "small tuning
//! change" cannot quietly make the product stricter than what was promised.
//!
//! That is not hypothetical here. The first version of the meter refilled by
//! `elapsed × rate` in floating point, and `6 s × (10/60)` is
//! `0.999999999999999` — the token `docs/21` says arrives after six seconds did
//! not arrive, and no reading of the code would have shown it.
//!
//! # Reason to change
//!
//! This file changes when `docs/21`'s table changes, and for no other reason.
//! The algorithm is `super::meter`; the wiring is `super::layer`.

use std::time::Duration;

/// One rate-limit class from `docs/21` §Rate limits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Class {
    /// The numerator of the table's "Sustained" column.
    pub sustained: u32,
    /// The window that rate is published over.
    ///
    /// A minute for most classes and an **hour** for invites, because `docs/21`
    /// publishes "50 / hour" and 50/hour is not a whole number of tokens per
    /// minute. Carrying the window rather than normalising to minutes is what
    /// makes the limit exactly the published one instead of 1/min (60/hour,
    /// too generous) or 0/min (nothing, too strict).
    pub window: Duration,
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
        self.window / self.sustained.max(1)
    }

    /// The value of the `RateLimit-Limit` header: tokens per minute.
    ///
    /// Rounded **down**, so the number a client is told is one it can actually
    /// sustain. Telling an invite client "1 per minute" when the budget is
    /// 50/hour would invite exactly the burst that gets refused.
    #[must_use]
    pub fn per_minute(self) -> u32 {
        let seconds = self.window.as_secs().max(1);
        u32::try_from(u64::from(self.sustained) * 60 / seconds).unwrap_or(u32::MAX)
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
    sustained: 10,
    window: Duration::from_secs(60),
    burst: 5,
};

/// `docs/21`: "Reads | 1,000 / min | 100".
pub const READ: Class = Class {
    sustained: 1_000,
    window: Duration::from_secs(60),
    burst: 100,
};

/// `docs/21`: "Writes | 300 / min | 50".
pub const WRITE: Class = Class {
    sustained: 300,
    window: Duration::from_secs(60),
    burst: 50,
};

/// `docs/21`: "Search | 60 / min | 20".
pub const SEARCH: Class = Class {
    sustained: 60,
    window: Duration::from_secs(60),
    burst: 20,
};

/// `docs/21`: "Bulk | 10 / min | 3".
pub const BULK: Class = Class {
    sustained: 10,
    window: Duration::from_secs(60),
    burst: 3,
};

/// `docs/21`: "Invites | 50 / hour | 10".
///
/// The one class published over an hour rather than a minute, which is why
/// [`Class`] carries its window.
pub const INVITE: Class = Class {
    sustained: 50,
    window: Duration::from_secs(60 * 60),
    burst: 10,
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

/// How much more a service account may spend than a person.
///
/// `docs/21`: "Service accounts get separate, higher, admin-configurable
/// buckets so an integration cannot exhaust a human's quota." The
/// admin-configurable half needs a settings surface that does not exist; this
/// is the "separate, higher" half, and the multiplier is stated here rather
/// than hidden in the layer so the value is reviewable against the document.
pub const SERVICE_ACCOUNT_MULTIPLIER: u32 = 5;

impl Class {
    /// The same class, scaled for a service account.
    #[must_use]
    pub const fn for_service_account(self) -> Self {
        Self {
            sustained: self.sustained * SERVICE_ACCOUNT_MULTIPLIER,
            window: self.window,
            burst: self.burst * SERVICE_ACCOUNT_MULTIPLIER,
        }
    }
}

/// Which published class a request falls in.
///
/// Decided from the route **template** and the method, never from the request
/// path: a classifier that matched attacker-supplied strings would be
/// bypassable with a trailing slash.
///
/// `None` means "not governed by a principal bucket" — the auth routes, which
/// have their own per-IP limiter and run before anybody is authenticated, and
/// the health and metrics endpoints, which an orchestrator polls.
#[must_use]
pub fn classify(method: &axum::http::Method, route: &str) -> Option<Class> {
    use axum::http::Method;

    // Ordered most specific first: `/tasks/bulk` is a write by method and a
    // bulk operation by route, and the narrower limit is the one that applies.
    if route.ends_with("/bulk") {
        return Some(BULK);
    }
    if route == "/api/v1/workspaces/{id}/invitations" && method == Method::POST {
        return Some(INVITE);
    }
    // Search is a read by method, and `docs/21` gives it a much tighter budget
    // because it is the read that costs a `tsquery` rather than an index probe.
    if route == "/api/v1/search" {
        return Some(SEARCH);
    }
    match route {
        // Not governed per principal: no principal yet, or an orchestrator.
        "/health/live" | "/health/ready" | "/metrics" => None,
        route if LIMITED_ROUTES.contains(&route) => None,
        "/api/v1/auth/logout" | "/api/v1/auth/session" => None,
        _ => Some(match *method {
            Method::GET | Method::HEAD | Method::OPTIONS => READ,
            _ => WRITE,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;

    #[test]
    fn every_class_matches_the_row_docs_21_publishes() {
        // docs/21 §Rate limits, transcribed. A "small tuning change" here makes
        // the product stricter than the contract customers read.
        for (class, sustained, burst, row) in [
            (AUTH, 10, 5, "Auth (login, reset)"),
            (READ, 1_000, 100, "Reads"),
            (WRITE, 300, 50, "Writes"),
            (SEARCH, 60, 20, "Search"),
            (BULK, 10, 3, "Bulk"),
        ] {
            assert_eq!(class.sustained, sustained, "{row} sustained");
            assert_eq!(class.window, Duration::from_secs(60), "{row} window");
            assert_eq!(class.burst, burst, "{row} burst");
        }
    }

    #[test]
    fn the_invite_class_is_the_published_hourly_rate() {
        // "Invites | 50 / hour | 10". Carried per minute like every other
        // class; 50/hour is not a whole number per minute, so this pins what
        // was actually chosen rather than letting it drift silently.
        // The class docs/21 publishes over an hour rather than a minute. The
        // window is carried so the limit is exactly 50/hour: normalising to
        // whole tokens per minute gives either 60/hour or nothing.
        assert_eq!(INVITE.burst, 10);
        assert_eq!(INVITE.sustained, 50);
        assert_eq!(INVITE.window, Duration::from_secs(3_600));
        assert_eq!(
            INVITE.emission_interval(),
            Duration::from_secs(72),
            "50 an hour is one every 72 seconds"
        );
    }

    #[test]
    fn a_token_arrives_after_exactly_the_published_gap() {
        // The bug this pins: `6 s × (10/60)` in floating point is
        // 0.999999999999999, so the token docs/21 promises after six seconds
        // did not arrive and the limit was quietly stricter than the document.
        assert_eq!(AUTH.emission_interval(), Duration::from_secs(6));
        assert_eq!(READ.emission_interval(), Duration::from_millis(60));
        assert_eq!(BULK.emission_interval(), Duration::from_secs(6));
    }

    #[test]
    fn an_emptied_bucket_refills_in_burst_times_the_gap() {
        assert_eq!(AUTH.full_refill(), Duration::from_secs(30));
        assert_eq!(BULK.full_refill(), Duration::from_secs(18));
    }

    #[test]
    fn a_service_account_gets_a_separate_higher_bucket() {
        // docs/21: "so an integration cannot exhaust a human's quota". Higher,
        // and never lower — a multiplier below 1 would invert the sentence.
        let multiplier = SERVICE_ACCOUNT_MULTIPLIER;
        assert!(multiplier >= 1, "a multiplier below 1 inverts the sentence");
        let theirs = WRITE.for_service_account();
        assert!(theirs.sustained > WRITE.sustained);
        assert!(theirs.burst > WRITE.burst);
        assert_eq!(
            theirs.window, WRITE.window,
            "scaling must not move the window"
        );
    }

    #[test]
    fn reads_and_writes_are_classified_by_method() {
        assert_eq!(
            classify(&Method::GET, "/api/v1/tasks"),
            Some(READ),
            "a GET is a read"
        );
        for method in [Method::POST, Method::PATCH, Method::DELETE, Method::PUT] {
            assert_eq!(
                classify(&method, "/api/v1/tasks/{id}"),
                Some(WRITE),
                "{method}"
            );
        }
    }

    #[test]
    fn the_narrower_class_wins_where_two_could_apply() {
        // `/tasks/bulk` is a write by method and a bulk operation by route.
        // Applying the write budget would give a caller 300/min of the most
        // expensive operation the API has.
        assert_eq!(classify(&Method::POST, "/api/v1/tasks/bulk"), Some(BULK));
        // Search is a read by method and has its own much tighter budget,
        // because it costs a tsquery rather than an index probe.
        assert_eq!(classify(&Method::GET, "/api/v1/search"), Some(SEARCH));
    }

    #[test]
    fn the_endpoints_an_orchestrator_polls_are_never_governed() {
        // A liveness probe that returns 429 gets the container restarted — the
        // limiter would become the outage it exists to prevent.
        for route in ["/health/live", "/health/ready", "/metrics"] {
            assert_eq!(classify(&Method::GET, route), None, "{route}");
        }
    }

    #[test]
    fn the_auth_routes_are_not_governed_per_principal() {
        // They run before anybody is authenticated, so there is no principal to
        // key on. They have their own per-IP limiter instead.
        for route in LIMITED_ROUTES {
            assert_eq!(classify(&Method::POST, route), None, "{route}");
        }
        assert_eq!(classify(&Method::POST, "/api/v1/auth/logout"), None);
        assert_eq!(classify(&Method::GET, "/api/v1/auth/session"), None);
    }
}
