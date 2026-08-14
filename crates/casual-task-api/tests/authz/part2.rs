use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_password_change_invalidates_existing_sessions() -> Result<()> {
    // docs/40 requires it and live_session implements it, but nothing exercised
    // the clause — it could have been deleted with every test still green.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed(&db.pool, "user@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());
    let (cookie, _) = login(&app, "user@example.com").await?;

    let before = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(before.status(), StatusCode::OK);

    test_support::mark_password_changed(&db.pool, user).await?;

    let after = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        after.status(),
        StatusCode::UNAUTHORIZED,
        "a session created before the password changed still works"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn tombstoning_an_account_kills_its_live_sessions() -> Result<()> {
    // Deactivating someone who is currently signed in has to end the session
    // they are holding, not the next one they create.
    let db = schema_harness::TestDatabase::start().await?;
    let user = seed(&db.pool, "leaver@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());
    let (cookie, _) = login(&app, "leaver@example.com").await?;

    let probe = |app: axum::Router, cookie: String| async move {
        app.oneshot(
            Request::builder()
                .uri("/api/v1/auth/session")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
    };

    assert_eq!(
        probe(app.clone(), cookie.clone()).await?.status(),
        StatusCode::OK
    );
    test_support::tombstone_user(&db.pool, user).await?;
    assert_eq!(
        probe(app, cookie).await?.status(),
        StatusCode::UNAUTHORIZED,
        "a deactivated account kept its live session"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_session_dies_of_idleness_and_of_old_age() -> Result<()> {
    // docs/40: 14 d idle / 30 d absolute. One expires_at column cannot express
    // both — a session used daily would live forever under an idle-only rule,
    // and one left open in a tab survives its whole absolute life under an
    // expiry-only rule. Both bounds are tested because both are easy to lose.
    for (last_seen, created, label) in [
        ("15 days", "15 days", "idle"),
        ("1 hour", "31 days", "absolute"),
    ] {
        let db = schema_harness::TestDatabase::start().await?;
        seed(&db.pool, "user@example.com").await?;
        let app = app_with_protected_route(db.pool.clone());
        let (cookie, _) = login(&app, "user@example.com").await?;

        test_support::age_session(&db.pool, last_seen, created).await?;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "a session past its {label} lifetime still authenticated"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_authentication_attempt_is_recorded() -> Result<()> {
    // docs/40 §What is audited. The unknown-address row is the one that matters:
    // an attacker guessing addresses produces only those, and without them the
    // burst that signals an attack is invisible.
    let db = schema_harness::TestDatabase::start().await?;
    seed(&db.pool, "real@example.com").await?;
    let app = app_with_protected_route(db.pool.clone());

    let attempt = |email: &'static str, password: &'static str| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "email": email, "password": password }).to_string(),
                    ))
                    .expect("request"),
            )
            .await
        }
    };

    attempt("real@example.com", "wrong").await?;
    attempt("nobody@example.com", PASSWORD).await?;
    attempt("real@example.com", PASSWORD).await?;

    assert_eq!(
        test_support::auth_events(&db.pool, "real@example.com").await?,
        vec!["login.succeeded".to_owned(), "login.failed".to_owned()]
    );
    assert_eq!(
        test_support::auth_events(&db.pool, "nobody@example.com").await?,
        vec!["login.failed".to_owned()],
        "an attempt on an unknown address left no trace, so credential \
         stuffing is invisible"
    );
    Ok(())
}
