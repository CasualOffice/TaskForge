use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn requiring_mfa_without_holding_a_factor_is_refused() -> Result<()> {
    // docs/40 §MFA: "the enforcing admin must already have MFA enrolled, so
    // nobody can lock themselves out while locking others in." Without this the
    // first person to use the feature is locked out by it.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    let refused = app
        .clone()
        .oneshot(json_request(
            &owner,
            "PUT",
            &format!("/api/v1/workspaces/{workspace}/mfa-requirement"),
            &json!({ "required": true }),
        )?)
        .await?;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(refused).await?["error"]["code"], "TF-AUT-0005");

    // With a factor, it is allowed — and the caller can still enter afterwards,
    // which is the whole point of the rule.
    enrol(&app, &owner).await?;
    let allowed = app
        .clone()
        .oneshot(json_request(
            &owner,
            "PUT",
            &format!("/api/v1/workspaces/{workspace}/mfa-requirement"),
            &json!({ "required": true }),
        )?)
        .await?;
    assert_eq!(allowed.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        enter_workspace(&app, &owner, workspace).await?.status(),
        StatusCode::OK,
        "the admin locked themselves out by enabling the requirement"
    );

    // Turning it OFF needs no factor: it can only widen access, and demanding
    // one would be the same lockout with the opposite sign.
    let stranger = sign_up(&app, &db.pool, "other@example.com").await?;
    let _ = stranger;
    let off = app
        .clone()
        .oneshot(json_request(
            &owner,
            "PUT",
            &format!("/api/v1/workspaces/{workspace}/mfa-requirement"),
            &json!({ "required": false }),
        )?)
        .await?;
    assert_eq!(off.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn break_glass_clears_a_factor_and_writes_the_audit_row() -> Result<()> {
    // docs/40 §Acceptance gates: "an owner locked out ... can recover through
    // the documented path, and the recovery is audited."
    //
    // This runs the REAL BINARY, not the functions behind it. A documented
    // recovery path that nobody executes is a path that has rotted by the time
    // it is needed, and the argument-parsing is exactly the part that rots.
    let db = schema_harness::TestDatabase::start().await?;
    test_support::enable_app_login(&db.pool).await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "owner@example.com").await?;
    enrol(&app, &caller).await?;
    assert_eq!(
        test_support::mfa_factor_state(&db.pool, caller.user_id).await?,
        (true, true)
    );

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_casual-task-api"))
        .arg("--break-glass-clear-mfa")
        .arg("owner@example.com")
        .env("DATABASE_URL", db.app_url())
        .output()?;
    assert!(
        output.status.success(),
        "break-glass failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        test_support::mfa_factor_state(&db.pool, caller.user_id).await?,
        (false, false),
        "the factor survived break-glass"
    );
    assert_eq!(
        test_support::recovery_code_counts(&db.pool, caller.user_id).await?,
        (0, 0)
    );

    // Audited, which is the half the acceptance gate actually names.
    let events = test_support::auth_events(&db.pool, "owner@example.com").await?;
    assert!(
        events.iter().any(|e| e == "mfa.break_glass"),
        "the recovery was not audited: {events:?}"
    );

    // And the owner can sign in with their password alone afterwards — the
    // recovery is only useful if it ends with them back in the product.
    let (cookie, csrf) = login(&app, "owner@example.com").await?;
    let recovered = Caller {
        user_id: caller.user_id,
        cookie,
        csrf,
    };
    let status = app
        .clone()
        .oneshot(request(&recovered, "GET", "/api/v1/auth/mfa").body(Body::empty())?)
        .await?;
    assert_eq!(json_body(status).await?["enrolled"], false);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn break_glass_refuses_an_address_it_does_not_know() -> Result<()> {
    // A typo at 3 a.m. must fail loudly rather than exiting 0 having done
    // nothing, which would read as "the factor is cleared".
    let db = schema_harness::TestDatabase::start().await?;
    test_support::enable_app_login(&db.pool).await?;

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_casual-task-api"))
        .arg("--break-glass-clear-mfa")
        .arg("nobody@example.com")
        .env("DATABASE_URL", db.app_url())
        .output()?;
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no active account"),
        "the failure did not say why"
    );

    // And with no address at all.
    let missing = std::process::Command::new(env!("CARGO_BIN_EXE_casual-task-api"))
        .arg("--break-glass-clear-mfa")
        .env("DATABASE_URL", db.app_url())
        .output()?;
    assert!(!missing.status.success());
    Ok(())
}
