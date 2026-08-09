//! Where the limiter meets a request (`docs/21` §Enforcement order).
//!
//! # The failure this module prevents
//!
//! A limiter that runs too late to protect anything. `docs/21` fixes the order,
//! cheapest first, "so an attacker cannot make us do expensive work to reject
//! them":
//!
//! ```text
//! 3. authentication   (cheap: one indexed read)
//! 4. rate limit       (bucket check)
//! 5. request parse + field validation
//! 6. authorization
//! ...
//! 9. quota            (needs a count)
//! ```
//!
//! So [`principal_rate_limit`] sits at step 4: after the one indexed read that
//! establishes who is calling, and before the body is parsed, before the
//! permission resolver runs, and before any handler touches a tenant row. A
//! limiter placed after authorization would let a flood cost a permission
//! resolution and a query each, which is the work it exists to prevent.
//!
//! **It authenticates once.** The result is cached in the request extensions
//! and reused by the [`Authenticated`](crate::middleware::Authenticated) and
//! [`WorkspaceMember`](crate::middleware::WorkspaceMember) extractors, so
//! keying per principal costs no extra round trip. Without that cache this
//! layer would double the auth query on every request in the system — a
//! limiter that made the database busier.
//!
//! # Reason to change
//!
//! This file changes when routing or the wire behaviour changes. The published
//! numbers are `super::class`; the algorithm is `super::meter`.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, State};
use axum::http::{HeaderValue, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use casual_task_observability::labels::{LabelSet, keys};
use casual_task_observability::metrics::RATE_LIMIT_HITS_TOTAL;
use casual_task_observability::recorder::Recorder;

use super::class::{self, AUTH, Class, LIMITED_ROUTES};
use super::meter::{Decision, RateLimiter, Scope};
use crate::error::ApiError;
use crate::server::RequestId;

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

/// One limiter per class, keyed per `(workspace, actor)`.
///
/// Separate buckets rather than one shared meter with a per-class cost: a
/// caller who exhausts the bulk budget must still be able to read, because
/// `docs/21` publishes five independent budgets and a client that reads its
/// own `RateLimit-*` headers is entitled to believe them.
#[derive(Clone)]
#[allow(missing_debug_implementations)]
pub struct PrincipalLimits {
    read: Arc<RateLimiter>,
    write: Arc<RateLimiter>,
    search: Arc<RateLimiter>,
    bulk: Arc<RateLimiter>,
    invite: Arc<RateLimiter>,
    /// Service accounts get their own buckets at a higher rate, so an
    /// integration cannot exhaust a human's quota (`docs/21`).
    service: Arc<RateLimiter>,
    pub metrics: Arc<Recorder>,
}

impl PrincipalLimits {
    #[must_use]
    pub fn new(metrics: Arc<Recorder>) -> Self {
        Self {
            read: Arc::new(RateLimiter::per_principal(class::READ)),
            write: Arc::new(RateLimiter::per_principal(class::WRITE)),
            search: Arc::new(RateLimiter::per_principal(class::SEARCH)),
            bulk: Arc::new(RateLimiter::per_principal(class::BULK)),
            invite: Arc::new(RateLimiter::per_principal(class::INVITE)),
            service: Arc::new(RateLimiter::per_principal(
                class::WRITE.for_service_account(),
            )),
            metrics,
        }
    }

    /// The limiter for a class, and the class it publishes.
    fn limiter_for(&self, class: Class, service_account: bool) -> (&RateLimiter, Class) {
        if service_account {
            return (&self.service, class.for_service_account());
        }
        let limiter = match class {
            c if c == class::READ => &self.read,
            c if c == class::WRITE => &self.write,
            c if c == class::SEARCH => &self.search,
            c if c == class::BULK => &self.bulk,
            _ => &self.invite,
        };
        (limiter, class)
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

    write_headers(&mut response, AUTH, decision);
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

/// What [`principal_rate_limit`] needs: a connection to authenticate with, and
/// the buckets.
///
/// A state of its own rather than a field on `AppState`: the limiter needs two
/// things out of it, and widening a struct every test constructs would be a
/// change to every test for no behavioural gain.
#[derive(Clone)]
#[allow(missing_debug_implementations)]
pub struct PrincipalState {
    pub pool: sqlx::PgPool,
    pub limits: PrincipalLimits,
}

/// The per-`(workspace, actor)` limiter — `docs/21` step 4.
///
/// Runs after authentication and before everything expensive. A request with no
/// credential, or one on a route no class governs, passes through untouched:
/// an unauthenticated caller is the auth limiter's problem, and a 401 costs one
/// indexed read, which is the budget `docs/21` already allots to step 3.
pub async fn principal_rate_limit(
    State(state): State<PrincipalState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // The route TEMPLATE, never the request path — a classifier matching
    // attacker-supplied strings is bypassable with a trailing slash.
    let Some(route) = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned())
    else {
        // No matched route: a 404. Not what this limiter is for.
        return next.run(request).await;
    };
    let Some(class) = class::classify(request.method(), &route) else {
        return next.run(request).await;
    };

    let request_id = RequestId::of_request(&request);
    let mut request = request;

    // Step 3 of docs/21's order, done once. The result goes into the extensions
    // so the extractors do not repeat it — without that, keying per principal
    // would double the auth query on every request in the system.
    let identified =
        match crate::middleware::authenticate_request(&state.pool, request.headers()).await {
            Ok(actor) => actor,
            // Not authenticated, or the lookup failed. Either way this layer has no
            // principal to key on and no business answering: the extractor below
            // will produce the documented 401 or 503.
            Err(()) => return next.run(request).await,
        };

    let workspace = crate::middleware::claimed_workspace(request.headers());
    request.extensions_mut().insert(identified);

    let Some(workspace) = workspace else {
        // No workspace claimed. `WorkspaceMember` answers this with a 404
        // (docs/04: the header must not be probeable), and a limiter cannot key
        // on half a pair.
        return next.run(request).await;
    };

    let service_account = matches!(
        identified.actor_type,
        casual_task_model::ActorType::ServiceAccount | casual_task_model::ActorType::Plugin
    );
    let (limiter, published) = state.limits.limiter_for(class, service_account);
    let decision =
        limiter.check_principal_at(workspace, identified.actor_id.as_uuid(), Instant::now());

    let mut response = if let Some(retry_after) = decision.retry_after_seconds {
        record_hit(&state.limits.metrics, Scope::Principal);
        // No workspace or actor id in the line: docs/46 §Cardinality discipline
        // keeps tenant identifiers out of metrics, and the same argument
        // applies to a log an operator greps during a flood.
        tracing::warn!(
            scope = Scope::Principal.label(),
            route,
            retry_after,
            "a request was rate limited"
        );
        ApiError::too_many_requests(&request_id, retry_after).into_response()
    } else {
        next.run(request).await
    };

    write_headers(&mut response, published, decision);
    response
}

/// `docs/05` §Rate limiting: "Returned on success too, so a client can slow
/// down *before* being throttled."
fn write_headers(response: &mut Response, class: Class, decision: Decision) {
    let headers = response.headers_mut();
    headers.insert("ratelimit-limit", HeaderValue::from(class.per_minute()));
    headers.insert("ratelimit-remaining", HeaderValue::from(decision.remaining));
    headers.insert("ratelimit-reset", HeaderValue::from(decision.reset_seconds));
}

#[cfg(test)]
mod tests {
    use super::*;

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

#[cfg(test)]
mod send_check {
    //! The middleware's future must be `Send`.
    //!
    //! Kept because the failure mode is genuinely obscure: `&Request<Body>` is
    //! not `Sync`, so holding one across an `await` makes the future
    //! non-`Send`, and `from_fn_with_state` then rejects the middleware with an
    //! error that names only an unsatisfied `Service` bound and points at the
    //! router. This assertion points at the function instead.

    #[allow(dead_code)]
    fn assert_send<T: Send>(_: T) {}

    #[allow(dead_code)]
    fn the_middleware_future_is_send(
        state: super::PrincipalState,
        request: axum::http::Request<axum::body::Body>,
        next: axum::middleware::Next,
    ) {
        assert_send(super::principal_rate_limit(
            axum::extract::State(state),
            request,
            next,
        ));
    }
}
