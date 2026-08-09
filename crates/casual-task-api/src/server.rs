//! The router, the middleware, and shutdown.
//!
//! # Health endpoints are two questions, not one (`docs/46`)
//!
//! `/health/live` answers "is this process wedged?" and touches nothing else.
//! A liveness probe that checks the database restarts every API instance during
//! a database outage — removing the only thing that could still serve cached
//! reads, and adding a thundering herd of reconnects to an already struggling
//! server.
//!
//! `/health/ready` answers "should traffic come here?" and *does* check the
//! database, because an instance that cannot reach it should leave the load
//! balancer rotation without dying.
//!
//! # Shutdown (D-041)
//!
//! `SIGTERM` stops accepting connections and lets in-flight requests finish,
//! bounded. Unbounded would mean one slow request holds the deploy open until
//! the orchestrator `SIGKILL`s the process mid-write.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{MatchedPath, State};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use casual_task_observability::labels::{LabelSet, keys};
use casual_task_observability::metrics::{HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_TOTAL};
use casual_task_observability::recorder::Recorder;
use sqlx::PgPool;

use crate::error::ApiError;

/// The header carrying a request id in and out.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// The request id, put into extensions so handlers and extractors can put the
/// **same** value in an error body that the response header carries.
///
/// It existed only as a header before, so every error body carried a hardcoded
/// literal — `"auth"`, `"login"` — while the header carried a real id. `docs/05`
/// promises "a `request_id` the user can quote to support"; two different values
/// for one request makes that promise false in the exact situation it is for.
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

impl RequestId {
    /// The id, or a marker when the observability layer did not run — which
    /// happens only in a test that builds a bare handler.
    #[must_use]
    pub fn of(parts: &axum::http::request::Parts) -> String {
        parts
            .extensions
            .get::<Self>()
            .map_or_else(|| "unknown".to_owned(), |id| id.0.clone())
    }

    /// The same, from headers alone — for a handler that took `HeaderMap`
    /// rather than `Parts`.
    #[must_use]
    pub fn of_parts(headers: &axum::http::HeaderMap) -> String {
        headers
            .get(REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty() && v.len() <= 128)
            .map_or_else(|| "unknown".to_owned(), ToOwned::to_owned)
    }

    /// The same, from a whole request.
    #[must_use]
    pub fn of_request(request: &Request<axum::body::Body>) -> String {
        request
            .extensions()
            .get::<Self>()
            .map_or_else(|| "unknown".to_owned(), |id| id.0.clone())
    }
}

/// Extracted so a handler can build an error body carrying the **same** id the
/// response header will.
///
/// Reading it from the request headers instead — as the login handler does —
/// finds an id only when the client happened to send one, because `observe`
/// puts the minted id in extensions and on the *response*. A handler that took
/// the header would therefore report `"unknown"` to every user who did not
/// already know their own request id.
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for RequestId {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<Self>()
            .cloned()
            .unwrap_or_else(|| Self("unknown".to_owned())))
    }
}

/// How long shutdown waits for in-flight requests.
///
/// Shorter than Kubernetes' default 30-second `SIGKILL` grace, because being
/// killed mid-request is what this exists to avoid.
pub const DRAIN: Duration = Duration::from_secs(20);

/// Shared, immutable per-request state.
#[derive(Clone)]
#[allow(missing_debug_implementations)]
pub struct AppState {
    pub pool: PgPool,
    /// Live-update fan-out (C-015). A trait object rather than the concrete hub
    /// so `docs/48`'s Redis implementation, when it exists, replaces it without
    /// touching a handler — and so a test can supply its own.
    pub broadcast: Arc<dyn casual_task_infra::broadcast::Broadcast>,
    pub metrics: Arc<Recorder>,
    /// `TF_SECRET_KEY`. Used for the CSRF binding and nothing else — ADR-032:
    /// "TF_SECRET_KEY is not a cookie signature."
    pub secret_key: Arc<str>,
    /// `TF_PUBLIC_URL`. `docs/48`: "used in emails and OIDC redirects". The
    /// reset link is built from it, so a deployment that sets it wrongly sends
    /// links to the wrong host rather than merely rendering one oddly.
    pub public_url: Arc<str>,
    /// Where attachment bytes live. `Arc<dyn ObjectStore>` for the reason
    /// `Mailer` is one: `TF_STORAGE_BACKEND` picks the backend once at startup
    /// and no handler branches on it again, so the filesystem profile runs the
    /// identical handshake S3 does (`docs/28` §Local deployment).
    pub storage: Arc<dyn casual_task_infra::ObjectStore>,
    /// Where outbound mail goes. `Arc<dyn Mailer>` and not an `Option`: an
    /// empty `TF_SMTP_HOST` selects the no-op implementation at startup
    /// (`docs/48`, D-046), so no handler has to ask whether email is on.
    pub mailer: Arc<dyn casual_task_infra::Mailer>,
}

/// Build the router.
///
/// **Every route is registered before the layers, and must stay that way.** In
/// axum a route added *after* `.layer()` is not wrapped by it — so a route
/// appended to the returned `Router` silently escapes both the CSRF guard and
/// the request id. `docs/05` says "every unsafe method without a valid token is
/// rejected", and that holds only while this ordering does.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/metrics", get(metrics))
        .route(
            "/api/v1/auth/login",
            axum::routing::post(crate::auth::login),
        )
        .route(
            "/api/v1/auth/logout",
            axum::routing::post(crate::auth::logout),
        )
        .route("/api/v1/auth/session", get(crate::middleware::whoami))
        // MFA (C-001, docs/40 §MFA). All under /auth because they are about
        // the credential rather than about a workspace — the one exception is
        // the per-workspace requirement toggle, which lives with the workspace
        // it configures.
        .route(
            "/api/v1/auth/mfa",
            get(crate::mfa::status).delete(crate::mfa::disable),
        )
        .route(
            "/api/v1/auth/mfa/enrolment",
            axum::routing::post(crate::mfa::begin),
        )
        .route(
            "/api/v1/auth/mfa/enrolment/confirm",
            axum::routing::post(crate::mfa::confirm),
        )
        .route(
            "/api/v1/auth/mfa/step-up",
            axum::routing::post(crate::mfa::step_up),
        )
        .route(
            "/api/v1/auth/mfa/recovery",
            axum::routing::post(crate::mfa::verify_recovery_code),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/mfa-requirement",
            axum::routing::put(crate::mfa::set_requirement),
        )
        // Both reset routes are registered HERE, above the layers, like every
        // other route: one registered below them escapes the CSRF guard and the
        // request id. They pass the CSRF guard because they carry no session
        // cookie — there is nothing to forge with, which is the same reason
        // login does.
        .route(
            "/api/v1/auth/password-reset",
            axum::routing::post(crate::password_reset::request),
        )
        .route(
            "/api/v1/auth/password-reset/confirm",
            axum::routing::post(crate::password_reset::confirm),
        )
        // C-006 / C-008. Every one of these takes `WorkspaceMember`, which is
        // the only thing that mints an `AuthContext` — so none of them can
        // reach a tenant row without a validated membership (`docs/32`).
        .route(
            "/api/v1/projects",
            get(crate::projects::list).post(crate::projects::create),
        )
        .route(
            "/api/v1/projects/{id}",
            get(crate::projects::read).patch(crate::projects::update),
        )
        .route(
            "/api/v1/projects/{id}/tasks",
            axum::routing::post(crate::tasks::create),
        )
        .route(
            "/api/v1/tasks/{id}/attachments",
            get(crate::attachments::list).post(crate::attachments::presign),
        )
        .route(
            "/api/v1/attachments/{id}/commit",
            axum::routing::post(crate::attachments::commit),
        )
        .route(
            "/api/v1/attachments/{id}/download",
            get(crate::attachments::download),
        )
        .route("/api/v1/tasks", get(crate::tasks::list))
        // C-016. Both take `WorkspaceMember`, and both scope every statement to
        // the caller's own user id — a notification is the one tenant row whose
        // owner is not implied by the workspace.
        .route("/api/v1/workflows/{id}", get(crate::workflows::read))
        // Workflow authoring (`docs/23` §Editing a workflow). A status delete
        // carries `?migrate_to=` because a status holding tasks cannot simply
        // vanish — every task on it moves in the same transaction, attributed
        // to the admin who asked.
        .route(
            "/api/v1/workflows/{id}/statuses",
            get(crate::workflows::list_statuses).post(crate::workflows::create_status),
        )
        .route(
            "/api/v1/workflows/{id}/statuses/order",
            axum::routing::post(crate::workflows::reorder_statuses),
        )
        .route(
            "/api/v1/workflows/{id}/statuses/{sid}",
            axum::routing::patch(crate::workflows::update_status)
                .delete(crate::workflows::delete_status),
        )
        .route(
            "/api/v1/workflows/{id}/transitions",
            axum::routing::post(crate::workflows::create_transition),
        )
        .route(
            "/api/v1/workflows/{id}/transitions/{tid}",
            axum::routing::patch(crate::workflows::update_transition)
                .delete(crate::workflows::delete_transition),
        )
        // Environments. Also an authorization scope (`Scope::Environment`), so
        // these are part of the permission model and not merely a task field.
        // A project involves many teams (`docs/03`). Authority is
        // `project.member.manage`, evaluated against the project's EXISTING
        // teams — evaluating against the incoming one would let anyone holding
        // a grant on a team add that team to any project they can see.
        .route(
            "/api/v1/projects/{id}/teams",
            get(crate::project_teams::list).post(crate::project_teams::add),
        )
        .route(
            "/api/v1/projects/{id}/teams/{team_id}",
            axum::routing::delete(crate::project_teams::remove),
        )
        .route(
            "/api/v1/projects/{id}/environments",
            get(crate::environments::list).post(crate::environments::create),
        )
        .route(
            "/api/v1/environments/{id}",
            axum::routing::patch(crate::environments::rename).delete(crate::environments::delete),
        )
        .route(
            "/api/v1/tasks/{id}/environment",
            axum::routing::put(crate::environments::set_on_task),
        )
        .route(
            "/api/v1/permissions/effective",
            get(crate::permissions::effective),
        )
        .route(
            "/api/v1/permissions/explain",
            axum::routing::post(crate::permissions::explain),
        )
        .route("/api/v1/notifications", get(crate::notifications::list))
        .route(
            "/api/v1/notifications/read",
            axum::routing::post(crate::notifications::mark_read),
        )
        .route(
            "/api/v1/tasks/{id}",
            get(crate::tasks::read)
                .patch(crate::tasks::update)
                .delete(crate::tasks::delete),
        )
        // docs/23: the ONLY door to a status change. A `PATCH` naming
        // `status_id` is refused with TF-WFL-0001 and pointed here.
        .route(
            "/api/v1/tasks/{id}/transitions",
            axum::routing::post(crate::tasks::transition),
        )
        .route(
            "/api/v1/tasks/{id}/assignees",
            axum::routing::post(crate::tasks::assign),
        )
        .route(
            "/api/v1/tasks/{id}/assignees/{user_id}",
            axum::routing::delete(crate::tasks::unassign),
        )
        // C-009 — comments. Visibility is decided by the task, never by the
        // comment: a comment carries no permission of its own.
        // C-011 — the History tab. Every change has written an activity record
        // in the same transaction as the change since C-011; this is the first
        // thing that reads them.
        .route("/api/v1/tasks/{id}/activity", get(crate::activity::stream))
        // C-008 — the Relations panel. The write is docs/05's; the read shape
        // is chosen (see the module docs) because docs/05 specifies none.
        .route(
            "/api/v1/tasks/{id}/dependencies",
            get(crate::dependencies::read).post(crate::dependencies::add),
        )
        .route(
            "/api/v1/tasks/{id}/comments",
            get(crate::comments::thread).post(crate::comments::create),
        )
        .route(
            "/api/v1/comments/{id}",
            axum::routing::patch(crate::comments::edit),
        )
        .route(
            "/api/v1/tasks/{id}/tags",
            get(crate::tasks::tags_of).post(crate::tasks::tag),
        )
        .route(
            "/api/v1/tasks/{id}/tags/{tag_id}",
            axum::routing::delete(crate::tasks::untag),
        )
        // The vocabulary, as distinct from its use above. `tag.manage` authors
        // it; `task.update` applies it. Without a list nothing can render a
        // picker, which is why the write endpoint beside it was unreachable
        // from a browser for its whole life.
        .route(
            "/api/v1/tags",
            get(crate::tags::list).post(crate::tags::create),
        )
        // ADR-018 caps depth at 1, so this is a list and never a tree. A read
        // and only a read: `docs/03` says the rollup is displayed, never
        // enforced, and there is no verb here that could enforce one.
        .route(
            "/api/v1/tasks/{id}/subtasks",
            get(crate::tasks::subtasks_of),
        )
        // Milestones. Authored per project, read with the tasks they are about.
        // Closing one moves no task — see `crate::milestones`.
        .route(
            "/api/v1/projects/{id}/milestones",
            get(crate::milestones::list).post(crate::milestones::create),
        )
        .route(
            "/api/v1/milestones/{id}",
            axum::routing::patch(crate::milestones::update),
        )
        // C-015. Registered here with every other route — above the layers, so
        // it is wrapped by CSRF, the rate limiter and `observe` like anything
        // else. A stream that escaped those would be an unmetered, unlimited,
        // unidentified connection.
        .route("/api/v1/stream", get(crate::sse::stream))
        // C-021 — export. Registered here with every other route, above the
        // layers, for the reason this function's docs give.
        .route(
            "/api/v1/exports",
            axum::routing::post(crate::exports::create),
        )
        .route("/api/v1/exports/{id}", get(crate::exports::read))
        .route(
            "/api/v1/exports/{id}/download",
            get(crate::exports::download),
        )
        // C-002 — workspaces, membership, teams. Registered HERE, above the
        // layers, for the reason this function's docs give: a route appended to
        // the returned Router escapes the CSRF guard and the request id.
        .route(
            "/api/v1/workspaces",
            get(crate::workspaces::list).post(crate::workspaces::create),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}",
            get(crate::workspaces::read).patch(crate::workspaces::rename),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/members",
            get(crate::workspaces::list_members).post(crate::workspaces::add_member),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/members/{user_id}",
            axum::routing::delete(crate::workspaces::remove_member),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/invitations",
            get(crate::invitations::list).post(crate::invitations::create),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/invitations/{id}",
            axum::routing::delete(crate::invitations::revoke),
        )
        // Accepting is NOT under /workspaces: the acceptor may not be a member
        // of one yet, and may have no account at all. It sits beside the other
        // credential-bearing, unauthenticated endpoints instead.
        .route(
            "/api/v1/auth/invitations/accept",
            axum::routing::post(crate::invitations::accept),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/teams",
            get(crate::workspaces::list_teams).post(crate::workspaces::create_team),
        )
        .route(
            "/api/v1/teams/{team_id}/members",
            axum::routing::post(crate::workspaces::add_team_member),
        )
        .route(
            "/api/v1/teams/{team_id}/members/{user_id}",
            axum::routing::delete(crate::workspaces::remove_team_member),
        )
        // CSRF sits over every route, so a route added later cannot be added
        // beside it. docs/05: "every unsafe method without a valid token is
        // rejected" — every, not most.
        //
        // Under `observe`, so that a CSRF rejection still gets a request id and
        // still counts in the metrics. A refusal nobody can measure is a
        // refusal nobody notices.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::csrf_guard,
        ))
        // Outside CSRF, inside `observe`. Outside, because `docs/21`
        // §Enforcement order puts the cheapest checks first and a bucket check
        // is cheaper than an HMAC; inside, so a 429 still gets a request id and
        // still lands in the RED metrics — a refusal nobody can measure is a
        // refusal nobody notices.
        //
        // Its state is built here rather than added to `AppState`, so the
        // limiter's lifetime is the router's: every test gets its own, and no
        // other construction site of `AppState` has to change.
        .layer(axum::middleware::from_fn_with_state(
            crate::rate_limit::RateLimitState::auth(Arc::clone(&state.metrics)),
            crate::rate_limit::rate_limit,
        ))
        // The per-`(workspace, actor)` limiter, OUTSIDE the auth-class one so
        // it is the first bucket a request meets, and outside CSRF for the same
        // reason that one is: `docs/21` §Enforcement order runs the cheapest
        // check first.
        //
        // This is step 4 of that order. It authenticates once — step 3, "cheap:
        // one indexed read" — and puts the answer in the request extensions, so
        // the extractors below it do not repeat the query. Placed any lower it
        // would be limiting requests that had already cost a permission
        // resolution and a tenant read, which is the work it exists to prevent.
        .layer(axum::middleware::from_fn_with_state(
            crate::rate_limit::PrincipalState {
                pool: state.pool.clone(),
                limits: crate::rate_limit::PrincipalLimits::new(Arc::clone(&state.metrics)),
            },
            crate::rate_limit::principal_rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(state.clone(), observe))
        .with_state(state)
}

/// Liveness. Deliberately touches nothing — see the module docs.
async fn live() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Readiness. Checks the database, and reports **503** when it cannot be
/// reached rather than 500: this is "do not send me traffic", not "I am
/// broken", and the distinction is what keeps a load balancer from removing
/// every instance during a brief database blip.
async fn ready(State(state): State<AppState>) -> Response {
    // Through casual-task-persistence: docs/19 puts every query there, and
    // casual-task-lint enforces it. Even a one-line `SELECT 1` — the rule is
    // not worth a hole.
    match casual_task_persistence::health::ping(&state.pool).await {
        Ok(()) => (StatusCode::OK, "ready").into_response(),
        Err(error) => {
            tracing::warn!(%error, "readiness check failed");
            // No request id here: readiness is probed by infrastructure, not by
            // a user who could quote one.
            ApiError::unavailable("health", 5).into_response()
        }
    }
}

/// The Prometheus scrape endpoint (F-009).
///
/// The body comes from `casual-task-observability`; this is the only part that
/// had to live in an HTTP crate, which is why the recorder was written without
/// it.
async fn metrics(State(state): State<AppState>) -> Response {
    (
        StatusCode::OK,
        [(
            "content-type",
            // Version 0.0.4 of the exposition format, which is what every
            // Prometheus scraper still negotiates.
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(),
    )
        .into_response()
}

/// Assign a request id, record RED metrics, and echo the id back.
///
/// The id is echoed on **every** response, including errors, because `docs/05`
/// promises the user something to quote to support — and the responses they
/// need it for are exactly the failing ones.
async fn observe(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // An inbound id is honoured so a trace spans a proxy; one is minted when
    // absent. It is not trusted for anything but correlation.
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .map_or_else(|| uuid::Uuid::now_v7().to_string(), ToOwned::to_owned);

    // The route TEMPLATE, never the resolved path: `docs/46` §Cardinality
    // discipline. `/api/v1/tasks/{id}` is one series; the resolved path is one
    // series per task.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned());
    let method = request.method().clone();

    // Into extensions BEFORE the inner layers run, so an error body built deep
    // in an extractor carries the same id the response header will.
    let mut request = request;
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));

    let started = std::time::Instant::now();
    let mut response = next.run(request).await;
    let elapsed = started.elapsed();

    record(&state.metrics, &method, &route, response.status(), elapsed);

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}

/// RED: rate, errors, duration.
fn record(
    metrics: &Recorder,
    method: &axum::http::Method,
    route: &str,
    status: StatusCode,
    elapsed: Duration,
) {
    // Bounded label values only. `method` is a fixed set; `route` is a template
    // from the router, not from the request; `status_class` collapses 200..599
    // to five values. None of them can carry an id.
    let status_class = match status.as_u16() {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    };
    let Some(method) = declared_method(method) else {
        // An unrecognised verb is not worth a metric series; it is worth a log.
        tracing::debug!(%method, "unrecognised HTTP method");
        return;
    };
    // `route` is a String from the router. It is bounded by the number of
    // routes, but LabelValue takes only &'static str — the cardinality guard —
    // so it is interned against the router's own table.
    let Some(route) = declared_route(route) else {
        return;
    };

    let labels = LabelSet::for_metric(HTTP_REQUESTS_TOTAL)
        .with(keys::METHOD, method)
        .and_then(|l| l.with(keys::ROUTE, route))
        .and_then(|l| l.with(keys::STATUS_CLASS, status_class));
    if let Ok(labels) = labels {
        let _ = metrics.increment(HTTP_REQUESTS_TOTAL, &labels, 1);
    }

    let labels = LabelSet::for_metric(HTTP_REQUEST_DURATION_SECONDS)
        .with(keys::METHOD, method)
        .and_then(|l| l.with(keys::ROUTE, route));
    if let Ok(labels) = labels {
        let _ = metrics.observe(
            HTTP_REQUEST_DURATION_SECONDS,
            &labels,
            elapsed.as_secs_f64(),
        );
    }
}

/// Map a method to a `&'static str`, or refuse it.
const fn declared_method(method: &axum::http::Method) -> Option<&'static str> {
    match *method {
        axum::http::Method::GET => Some("GET"),
        axum::http::Method::POST => Some("POST"),
        axum::http::Method::PUT => Some("PUT"),
        axum::http::Method::PATCH => Some("PATCH"),
        axum::http::Method::DELETE => Some("DELETE"),
        axum::http::Method::HEAD => Some("HEAD"),
        axum::http::Method::OPTIONS => Some("OPTIONS"),
        _ => None,
    }
}

/// Every route this server serves, as `&'static str`.
///
/// The cardinality guard in `casual-task-observability` accepts only
/// `&'static str`, and the router hands back a `String`. Interning it here
/// rather than widening the guard means an unrouted path — which is attacker
/// controlled — cannot become a metric series.
pub const ROUTES: &[&str] = &[
    "/health/live",
    "/health/ready",
    "/metrics",
    "/api/v1/auth/login",
    "/api/v1/auth/logout",
    "/api/v1/auth/session",
    "/api/v1/auth/mfa",
    "/api/v1/auth/mfa/enrolment",
    "/api/v1/auth/mfa/enrolment/confirm",
    "/api/v1/auth/mfa/step-up",
    "/api/v1/auth/mfa/recovery",
    "/api/v1/workspaces/{workspace_id}/mfa-requirement",
    "/api/v1/stream",
    "/api/v1/exports",
    "/api/v1/exports/{id}",
    "/api/v1/exports/{id}/download",
    "/api/v1/auth/password-reset",
    "/api/v1/auth/password-reset/confirm",
    // The route TEMPLATE, never the resolved path — `{id}` is one series, and
    // `/api/v1/projects/<uuid>` would be one series per project.
    "/api/v1/projects",
    "/api/v1/projects/{id}",
    "/api/v1/projects/{id}/tasks",
    "/api/v1/tasks",
    "/api/v1/workflows/{id}",
    "/api/v1/workflows/{id}/statuses",
    "/api/v1/workflows/{id}/statuses/order",
    "/api/v1/workflows/{id}/statuses/{sid}",
    "/api/v1/workflows/{id}/transitions",
    "/api/v1/workflows/{id}/transitions/{tid}",
    "/api/v1/projects/{id}/teams",
    "/api/v1/projects/{id}/teams/{team_id}",
    "/api/v1/projects/{id}/environments",
    "/api/v1/environments/{id}",
    "/api/v1/tasks/{id}/environment",
    "/api/v1/permissions/effective",
    "/api/v1/permissions/explain",
    "/api/v1/notifications",
    "/api/v1/notifications/read",
    "/api/v1/tasks/{id}/attachments",
    "/api/v1/attachments/{id}/commit",
    "/api/v1/attachments/{id}/download",
    "/api/v1/tasks/{id}",
    "/api/v1/tasks/{id}/activity",
    "/api/v1/tasks/{id}/dependencies",
    "/api/v1/tasks/{id}/comments",
    "/api/v1/comments/{id}",
    "/api/v1/tasks/{id}/tags/{tag_id}",
    "/api/v1/tags",
    "/api/v1/tasks/{id}/subtasks",
    "/api/v1/projects/{id}/milestones",
    "/api/v1/milestones/{id}",
    "/api/v1/tasks/{id}/transitions",
    "/api/v1/tasks/{id}/assignees",
    "/api/v1/tasks/{id}/assignees/{user_id}",
    "/api/v1/tasks/{id}/tags",
    "/api/v1/workspaces",
    "/api/v1/workspaces/{workspace_id}",
    "/api/v1/workspaces/{workspace_id}/members",
    "/api/v1/workspaces/{workspace_id}/members/{user_id}",
    "/api/v1/workspaces/{workspace_id}/invitations",
    "/api/v1/workspaces/{workspace_id}/invitations/{id}",
    "/api/v1/auth/invitations/accept",
    "/api/v1/workspaces/{workspace_id}/teams",
    "/api/v1/teams/{team_id}/members",
    "/api/v1/teams/{team_id}/members/{user_id}",
    "unmatched",
];

fn declared_route(route: &str) -> Option<&'static str> {
    ROUTES.iter().copied().find(|known| *known == route)
}

/// Serve until `SIGTERM` or `SIGINT`, then drain.
///
/// # Errors
///
/// If the listener cannot be bound, or the server fails while running.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: AppState,
) -> Result<(), std::io::Error> {
    // The hub is closed when the signal arrives, BEFORE axum stops accepting.
    // D-041: a live stream must be *closed*, not dropped mid-frame — a client
    // that sees end-of-stream reconnects, and one whose socket vanishes
    // mid-event sees a parse error and may not.
    //
    // Held separately from `state` because `router` consumes it.
    let broadcast = Arc::clone(&state.broadcast);
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let open = broadcast.subscriber_count();
            broadcast.close_all();
            tracing::info!(open, "closed live streams for shutdown");
        })
        .await
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            // Cannot install the handler: better to keep serving than to exit.
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGTERM; shutdown will be abrupt");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!(drain_seconds = DRAIN.as_secs(), "shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_the_router_serves_is_interned() {
        // The metric label must be a &'static str, so a route missing from
        // ROUTES silently loses its metrics. Listed here as the one place the
        // two tables are compared.
        for route in [
            "/health/live",
            "/health/ready",
            "/metrics",
            "/api/v1/auth/login",
            "/api/v1/auth/logout",
            "/api/v1/auth/session",
            "/api/v1/stream",
            "/api/v1/exports",
            "/api/v1/exports/{id}",
            "/api/v1/exports/{id}/download",
            "/api/v1/auth/password-reset",
            "/api/v1/auth/password-reset/confirm",
            "/api/v1/projects",
            "/api/v1/projects/{id}",
            "/api/v1/projects/{id}/tasks",
            "/api/v1/tasks",
            "/api/v1/tasks/{id}",
            "/api/v1/tasks/{id}/transitions",
            "/api/v1/tasks/{id}/assignees",
            "/api/v1/tasks/{id}/assignees/{user_id}",
            "/api/v1/tasks/{id}/tags",
            "/api/v1/workspaces",
            "/api/v1/workspaces/{workspace_id}",
            "/api/v1/workspaces/{workspace_id}/members",
            "/api/v1/workspaces/{workspace_id}/members/{user_id}",
            "/api/v1/workspaces/{workspace_id}/invitations",
            "/api/v1/workspaces/{workspace_id}/invitations/{id}",
            "/api/v1/auth/invitations/accept",
            "/api/v1/workspaces/{workspace_id}/teams",
            "/api/v1/teams/{team_id}/members",
            "/api/v1/teams/{team_id}/members/{user_id}",
        ] {
            assert!(
                declared_route(route).is_some(),
                "{route} is served but not in ROUTES, so it records no metrics"
            );
        }
    }

    #[test]
    fn every_route_in_the_source_of_router_is_interned() {
        // The list above is hand-maintained, which is exactly the thing that
        // drifts. This reads the `.route("...")` calls out of this file's own
        // source, so a route added to `router` without a ROUTES entry fails
        // here rather than silently losing its metrics.
        let source = include_str!("server.rs");
        let body = source
            .split_once("pub fn router")
            .and_then(|(_, rest)| rest.split_once(".layer("))
            .map(|(body, _)| body)
            .expect("router() is defined in this file and its routes precede its layers");
        let mut seen = 0;
        // Odd-indexed segments of a `"`-split are the insides of string
        // literals; every path in `router()` is one.
        for literal in body.split('"').skip(1).step_by(2) {
            if !literal.starts_with('/') {
                continue;
            }
            seen += 1;
            assert!(
                declared_route(literal).is_some(),
                "{literal} is registered in router() but missing from ROUTES, \
                 so every request to it records no metrics"
            );
        }
        assert!(seen >= 8, "only found {seen} routes; the scan is broken");
    }

    #[test]
    fn every_interned_route_is_actually_registered() {
        // The guard for the failure that produced it: a merge dropped the
        // comment routes from `router()` while leaving the module, the handlers
        // and the tests in place. Every comment request 404'd, and the only
        // symptom was six integration tests failing with an unhelpful `null`
        // body — nothing pointed at the router.
        //
        // ROUTES exists for metric labels, so it and the router are two lists
        // that must agree. Comparing them here means a route lost from either
        // side fails a unit test that NAMES the route, instead of an
        // integration suite that reports a status code.
        let source = include_str!("server.rs");
        let router_block = source
            .split("pub fn router(")
            .nth(1)
            .expect("router() exists");
        let router_block = &router_block[..router_block.find("\n}").unwrap_or(router_block.len())];

        for route in ROUTES {
            if *route == "unmatched" {
                continue;
            }
            assert!(
                router_block.contains(&format!("\"{route}\"")),
                "{route} is interned in ROUTES but not registered in router(); \
                 requests to it 404 and record no metrics"
            );
        }
    }

    #[test]
    fn an_unrouted_path_cannot_become_a_metric_series() {
        // The path is attacker-controlled. Without interning, every 404 to a
        // random URL would create a time series.
        assert_eq!(declared_route("/../../etc/passwd"), None);
        assert_eq!(declared_route("/api/v1/tasks/018f2c"), None);
    }

    #[test]
    fn the_drain_is_shorter_than_the_orchestrator_kill_grace() {
        // Kubernetes defaults to 30 s. A drain longer than that is not a drain;
        // it is a SIGKILL with extra steps.
        assert!(DRAIN < Duration::from_secs(30));
    }

    #[test]
    fn methods_map_to_bounded_labels() {
        assert_eq!(declared_method(&axum::http::Method::GET), Some("GET"));
        assert_eq!(
            declared_method(&axum::http::Method::from_bytes(b"PROPFIND").expect("valid")),
            None,
            "an arbitrary verb would be an unbounded label"
        );
    }
}
