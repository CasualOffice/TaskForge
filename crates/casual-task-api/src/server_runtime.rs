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
    "/api/v1/tasks/bulk",
    "/api/v1/workflows/{id}",
    "/api/v1/workflows/{id}/statuses",
    "/api/v1/workflows/{id}/statuses/order",
    "/api/v1/workflows/{id}/statuses/{sid}",
    "/api/v1/workflows/{id}/transitions",
    "/api/v1/workflows/{id}/transitions/{tid}",
    "/api/v1/projects/{id}/teams",
    "/api/v1/projects/{id}/teams/{team_id}",
    "/api/v1/projects/{id}/environments",
    "/api/v1/projects/{id}/environments/order",
    "/api/v1/environments/{id}",
    "/api/v1/tasks/{id}/environment",
    "/api/v1/projects/{id}/releases",
    "/api/v1/releases/{id}",
    "/api/v1/reports/run",
    "/api/v1/me",
    "/api/v1/me/queue",
    "/api/v1/me/teams",
    "/api/v1/me/password",
    "/api/v1/me/sessions",
    "/api/v1/me/sessions/{id}",
    "/api/v1/roles",
    "/api/v1/roles/{id}",
    "/api/v1/role-assignments",
    "/api/v1/role-assignments/{id}",
    "/api/v1/permissions/effective",
    "/api/v1/permissions/explain",
    "/api/v1/notifications",
    "/api/v1/notifications/read",
    "/api/v1/tasks/{id}/attachments",
    "/api/v1/attachments/{id}/commit",
    "/api/v1/attachments/{id}/download",
    "/api/v1/tasks/{id}",
    "/api/v1/tasks/{id}/activity",
    "/api/v1/tasks/{id}/custody",
    "/api/v1/tasks/{id}/team",
    "/api/v1/tasks/{id}/promotions",
    "/api/v1/tasks/{id}/verifications",
    "/api/v1/tasks/{id}/dependencies",
    "/api/v1/tasks/{id}/dependencies/{other_id}",
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
