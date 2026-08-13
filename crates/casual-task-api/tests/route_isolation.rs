//! Tenant-isolation acceptance coverage derived from the production route table.
//!
//! A hand-picked endpoint test leaves the next route unprotected. This gate
//! instead classifies every route exported by the server and drives every
//! tenant-scoped one as an authenticated non-member. Adding a route without
//! classifying it therefore fails this test.

mod schema_harness;

use std::sync::Arc;

use anyhow::Result;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::middleware::WORKSPACE_HEADER;
use casual_task_api::server::{AppState, ROUTES, router};
use casual_task_identity::password;
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";
const SECRET: &str = "a-test-secret-key-long-enough-for-hmac";

/// Routes that deliberately do not resolve a workspace.
const NON_TENANT_ROUTES: &[&str] = &[
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
    "/api/v1/auth/password-reset",
    "/api/v1/auth/password-reset/confirm",
    "/api/v1/auth/invitations/accept",
    "/api/v1/workspaces",
    "/api/v1/me",
    "/api/v1/me/password",
    "/api/v1/me/sessions",
    "/api/v1/me/sessions/{id}",
    "unmatched",
];

fn state(pool: sqlx::PgPool) -> AppState {
    AppState {
        storage: Arc::new(casual_task_infra::FilesystemStore::new(
            std::env::temp_dir().join("tf-route-isolation-objects"),
            "https://files.example.test".to_owned(),
            "test-object-signing-secret".to_owned(),
        )),
        broadcast: casual_task_api::sse::local_hub(),
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: SECRET.into(),
        public_url: "https://tasks.example.test".into(),
        mailer: Arc::new(casual_task_infra::mail::LoggingMailer),
    }
}

async fn login(app: &axum::Router) -> Result<(String, String)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "email": "outsider@example.com",
                        "password": PASSWORD,
                    })
                    .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "login failed");

    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(SESSION_COOKIE))
        .and_then(|value| value.split(';').next())
        .expect("session cookie")
        .to_owned();
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&body)?;
    let csrf = body["csrf_token"].as_str().expect("csrf token").to_owned();
    Ok((cookie, csrf))
}

fn materialize(template: &str, workspace_id: Uuid) -> String {
    template
        .split('/')
        .map(|segment| {
            if segment == "{workspace_id}" {
                workspace_id.to_string()
            } else if segment.starts_with('{') && segment.ends_with('}') {
                Uuid::now_v7().to_string()
            } else {
                segment.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[test]
fn every_route_is_explicitly_classified() {
    for route in ROUTES {
        let non_tenant = NON_TENANT_ROUTES.contains(route);
        let tenant = !non_tenant && *route != "unmatched";
        assert!(
            non_tenant || tenant,
            "{route} has no tenant-isolation classification"
        );
    }
    for route in NON_TENANT_ROUTES {
        assert!(
            ROUTES.contains(route),
            "stale route classification: {route}"
        );
    }
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_tenant_route_hides_a_workspace_from_a_non_member() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let outsider = Uuid::now_v7();
    test_support::insert_user_with_password(
        &db.pool,
        outsider,
        "outsider@example.com",
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
    let private_workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, private_workspace, "private").await?;

    let app_state = state(db.pool.clone());
    let (cookie, csrf) = login(&router(app_state.clone())).await?;
    let methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ];

    for template in ROUTES
        .iter()
        .filter(|route| !NON_TENANT_ROUTES.contains(route))
    {
        let uri = materialize(template, private_workspace);
        let mut registered = 0;
        for method in &methods {
            // A fresh router gives each probe a fresh rate limiter. The test is
            // checking authority, not the aggregate request budget.
            let response = router(app_state.clone())
                .oneshot(
                    Request::builder()
                        .method(method.clone())
                        .uri(&uri)
                        .header(header::COOKIE, &cookie)
                        .header(WORKSPACE_HEADER, private_workspace.to_string())
                        .header("x-csrf-token", &csrf)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))?,
                )
                .await?;
            if response.status() == StatusCode::METHOD_NOT_ALLOWED {
                continue;
            }
            registered += 1;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{method} {template} exposed a private workspace to a non-member"
            );
        }
        assert!(
            registered > 0,
            "{template} is listed but no method serves it"
        );
    }
    Ok(())
}
