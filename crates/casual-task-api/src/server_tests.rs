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
        "/api/v1/tasks/bulk",
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
fn interned_routes_are_unique() {
    let mut routes = ROUTES.to_vec();
    routes.sort_unstable();
    for pair in routes.windows(2) {
        assert_ne!(pair[0], pair[1], "{} appears twice in ROUTES", pair[0]);
    }
}

#[test]
fn every_route_in_the_source_of_router_is_interned() {
    // The list above is hand-maintained, which is exactly the thing that
    // drifts. This reads the `.route("...")` calls out of this file's own
    // route-construction source, so a route added without a ROUTES entry fails
    // here rather than silently losing its metrics.
    let source = include_str!("server_routes.rs");
    let body = source
        .split_once("pub fn router")
        .and_then(|(_, rest)| rest.split_once(".layer("))
        .map(|(body, _)| body)
        .expect("router() is defined in the route module and its routes precede its layers");
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
    let source = include_str!("server_routes.rs");
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
