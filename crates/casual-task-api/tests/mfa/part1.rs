use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn enrolment_is_two_steps_and_issues_ten_recovery_codes() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "ada@example.com").await?;

    let totp = begin_enrolment(&app, &caller).await?;
    // Present, and NOT confirmed. This is the state a user who closed the tab
    // is left in, and every decision must treat it as "no factor".
    assert_eq!(
        test_support::mfa_factor_state(&db.pool, caller.user_id).await?,
        (true, false)
    );

    let response = app
        .clone()
        .oneshot(json_request(
            &caller,
            "POST",
            "/api/v1/auth/mfa/enrolment/confirm",
            &json!({ "code": code_now(&totp) }),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    let codes = body["recovery_codes"].as_array().context("codes")?;
    assert_eq!(codes.len(), 10, "docs/40 says ten recovery codes");

    assert_eq!(
        test_support::mfa_factor_state(&db.pool, caller.user_id).await?,
        (true, true)
    );
    assert_eq!(
        test_support::recovery_code_counts(&db.pool, caller.user_id).await?,
        (10, 0)
    );

    // The status endpoint agrees, which is what a client renders from.
    let status = app
        .clone()
        .oneshot(request(&caller, "GET", "/api/v1/auth/mfa").body(Body::empty())?)
        .await?;
    let status = json_body(status).await?;
    assert_eq!(status["enrolled"], true);
    assert_eq!(status["pending"], false);
    assert_eq!(status["recovery_codes_remaining"], 10);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unconfirmed_factor_never_satisfies_mfa() -> Result<()> {
    // The failure migration 0016's comment names: "a user who lost the
    // enrolment halfway would otherwise be locked out by a factor they do not
    // have." They must be refused entry AND able to step up with nothing,
    // rather than being trapped.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "ada@example.com").await?;
    let workspace = create_workspace(&app, &caller, "acme").await?;

    // Enrolment begun and abandoned.
    let totp = begin_enrolment(&app, &caller).await?;
    test_support::require_workspace_mfa(&db.pool, workspace, true).await?;

    // Entry is refused, with the step-up code rather than a generic 401.
    let refused = enter_workspace(&app, &caller, workspace).await?;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let body = json_body(refused).await?;
    assert_eq!(body["error"]["code"], "TF-AUT-0005");

    // And a code from the UNCONFIRMED factor does not satisfy the step-up —
    // which is the half that would silently work if `confirmed_at` were only
    // checked at enrolment.
    let step_up = app
        .clone()
        .oneshot(json_request(
            &caller,
            "POST",
            "/api/v1/auth/mfa/step-up",
            &json!({ "code": code_now(&totp) }),
        )?)
        .await?;
    assert_eq!(step_up.status(), StatusCode::UNAUTHORIZED);
    let body = json_body(step_up).await?;
    assert_eq!(body["error"]["code"], "TF-AUT-0006");
    assert!(!test_support::session_mfa_satisfied(&db.pool, caller.user_id).await?);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_code_cannot_be_replayed() -> Result<()> {
    // RFC 6238 §5.2. A code is valid for a whole 30-second step, so an observed
    // one can be presented inside the same window by whoever saw it.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "ada@example.com").await?;
    let (totp, _) = enrol(&app, &caller).await?;

    // Confirmation consumed a step and recorded it. Asserted as the state the
    // replay guard runs on, rather than by replaying the enrolment code: the
    // suite is slow enough that a 30-second boundary usually passes first, so
    // that assertion would be testing the clock rather than the guard.
    assert!(
        test_support::mfa_last_step(&db.pool, caller.user_id)
            .await?
            .is_some(),
        "confirmation did not record the step it consumed"
    );

    // A code from the NEXT step is accepted once...
    let next = OffsetDateTime::now_utc().unix_timestamp() / STEP_SECONDS + 1;
    let code = code_at(&totp, next);
    let first = app
        .clone()
        .oneshot(json_request(
            &caller,
            "POST",
            "/api/v1/auth/mfa/step-up",
            &json!({ "code": code.clone() }),
        )?)
        .await?;
    assert_eq!(first.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        test_support::mfa_last_step(&db.pool, caller.user_id).await?,
        Some(next),
        "the accepted step was not recorded"
    );

    // ...and refused the second time, with the same shape as a wrong code.
    let second = app
        .clone()
        .oneshot(json_request(
            &caller,
            "POST",
            "/api/v1/auth/mfa/step-up",
            &json!({ "code": code }),
        )?)
        .await?;
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
    let body = json_body(second).await?;
    assert_eq!(
        body["error"]["code"], "TF-AUT-0006",
        "a replay must be indistinguishable from a wrong code"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_earlier_step_is_refused_after_a_later_one() -> Result<()> {
    // The monotonic half. Refusing only the exact step already used would let a
    // code captured a few seconds ago be presented after the clock ticks on,
    // which is the same attack with one extra second of patience.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "ada@example.com").await?;
    let (totp, _) = enrol(&app, &caller).await?;

    let now = OffsetDateTime::now_utc().unix_timestamp() / STEP_SECONDS;
    let later = app
        .clone()
        .oneshot(json_request(
            &caller,
            "POST",
            "/api/v1/auth/mfa/step-up",
            &json!({ "code": code_at(&totp, now + 1) }),
        )?)
        .await?;
    assert_eq!(later.status(), StatusCode::NO_CONTENT);

    // The previous step is still inside the ±1 skew window, so the verifier
    // accepts it — and the replay guard is the only thing that refuses it.
    let earlier = app
        .clone()
        .oneshot(json_request(
            &caller,
            "POST",
            "/api/v1/auth/mfa/step-up",
            &json!({ "code": code_at(&totp, now) }),
        )?)
        .await?;
    assert_eq!(
        earlier.status(),
        StatusCode::UNAUTHORIZED,
        "a step below the highest accepted was allowed"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_requiring_workspace_demands_a_step_up_and_then_admits() -> Result<()> {
    // docs/40 §Workspace-level SSO and MFA step-up: the policy is applied at
    // workspace resolution, not at login. The session is already valid here —
    // what changes is which workspace it may enter.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "ada@example.com").await?;
    let open = create_workspace(&app, &caller, "open-co").await?;
    let strict = create_workspace(&app, &caller, "strict-co").await?;
    let (totp, _) = enrol(&app, &caller).await?;

    // Enrolling marked this session, so start from an unsatisfied one.
    let (cookie, csrf) = login(&app, "ada@example.com").await?;
    let caller = Caller {
        user_id: caller.user_id,
        cookie,
        csrf,
    };
    test_support::require_workspace_mfa(&db.pool, strict, true).await?;

    // The open workspace is unaffected — the policy is per workspace.
    assert_eq!(
        enter_workspace(&app, &caller, open).await?.status(),
        StatusCode::OK
    );

    let refused = enter_workspace(&app, &caller, strict).await?;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let body = json_body(refused).await?;
    assert_eq!(body["error"]["code"], "TF-AUT-0005");
    assert_eq!(
        body["error"]["details"]["step_up"],
        "/api/v1/auth/mfa/step-up"
    );

    let step_up = app
        .clone()
        .oneshot(json_request(
            &caller,
            "POST",
            "/api/v1/auth/mfa/step-up",
            &json!({ "code": code_at(&totp, OffsetDateTime::now_utc().unix_timestamp() / STEP_SECONDS + 1) }),
        )?)
        .await?;
    assert_eq!(step_up.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        enter_workspace(&app, &caller, strict).await?.status(),
        StatusCode::OK,
        "a satisfied session was still refused"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_non_member_gets_404_whether_or_not_the_workspace_requires_mfa() -> Result<()> {
    // The step-up check sits AFTER the membership check for this reason: a
    // stranger probing workspace ids must not be able to tell a workspace that
    // demands MFA from one that does not exist.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let strict = create_workspace(&app, &owner, "strict-co").await?;
    test_support::require_workspace_mfa(&db.pool, strict, true).await?;

    let stranger = sign_up(&app, &db.pool, "stranger@example.com").await?;
    let real = enter_workspace(&app, &stranger, strict).await?;
    let imaginary = enter_workspace(&app, &stranger, Uuid::now_v7()).await?;

    assert_eq!(real.status(), StatusCode::NOT_FOUND);
    assert_eq!(imaginary.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        json_body(real).await?["error"]["code"],
        json_body(imaginary).await?["error"]["code"],
        "a requiring workspace is distinguishable from one that does not exist"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_recovery_code_satisfies_a_step_up_exactly_once() -> Result<()> {
    // docs/40: "10 single-use recovery codes shown once." This is the path a
    // person with a lost phone actually takes.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "ada@example.com").await?;
    let workspace = create_workspace(&app, &caller, "acme").await?;
    let (_, codes) = enrol(&app, &caller).await?;

    let (cookie, csrf) = login(&app, "ada@example.com").await?;
    let caller = Caller {
        user_id: caller.user_id,
        cookie,
        csrf,
    };
    test_support::require_workspace_mfa(&db.pool, workspace, true).await?;
    assert_eq!(
        enter_workspace(&app, &caller, workspace).await?.status(),
        StatusCode::UNAUTHORIZED
    );

    let first = app
        .clone()
        .oneshot(json_request(
            &caller,
            "POST",
            "/api/v1/auth/mfa/recovery",
            &json!({ "recovery_code": codes[0] }),
        )?)
        .await?;
    assert_eq!(first.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        enter_workspace(&app, &caller, workspace).await?.status(),
        StatusCode::OK
    );
    assert_eq!(
        test_support::recovery_code_counts(&db.pool, caller.user_id).await?,
        (9, 1),
        "the code was not burned"
    );

    // The same code again, from a fresh session so the outcome is about the
    // code rather than about the session already being satisfied.
    let (cookie, csrf) = login(&app, "ada@example.com").await?;
    let again = Caller {
        user_id: caller.user_id,
        cookie,
        csrf,
    };
    let second = app
        .clone()
        .oneshot(json_request(
            &again,
            "POST",
            "/api/v1/auth/mfa/recovery",
            &json!({ "recovery_code": codes[0] }),
        )?)
        .await?;
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        test_support::recovery_code_counts(&db.pool, again.user_id).await?,
        (9, 1),
        "a refused redemption still burned a code"
    );

    // A different unused code still works — burning one must not burn the set.
    let third = app
        .clone()
        .oneshot(json_request(
            &again,
            "POST",
            "/api/v1/auth/mfa/recovery",
            &json!({ "recovery_code": codes[1] }),
        )?)
        .await?;
    assert_eq!(third.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn removing_a_factor_needs_a_current_proof() -> Result<()> {
    // docs/40 §MFA lists "managing MFA" among the actions needing
    // re-authentication rather than a valid session. Removing a second factor
    // with a stolen cookie alone is the most useful thing an attacker could do
    // with one.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "ada@example.com").await?;
    let (totp, codes) = enrol(&app, &caller).await?;

    let no_proof = app
        .clone()
        .oneshot(json_request(
            &caller,
            "DELETE",
            "/api/v1/auth/mfa",
            &json!({}),
        )?)
        .await?;
    assert_eq!(no_proof.status(), StatusCode::BAD_REQUEST);

    let wrong = app
        .clone()
        .oneshot(json_request(
            &caller,
            "DELETE",
            "/api/v1/auth/mfa",
            &json!({ "code": "000000" }),
        )?)
        .await?;
    assert!(
        matches!(wrong.status(), StatusCode::UNAUTHORIZED),
        "a wrong code removed the factor"
    );
    assert_eq!(
        test_support::mfa_factor_state(&db.pool, caller.user_id).await?,
        (true, true)
    );

    // A recovery code is an accepted proof, which is what someone whose phone
    // is gone has to use to turn the factor off.
    let _ = totp;
    let removed = app
        .clone()
        .oneshot(json_request(
            &caller,
            "DELETE",
            "/api/v1/auth/mfa",
            &json!({ "recovery_code": codes[0] }),
        )?)
        .await?;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        test_support::mfa_factor_state(&db.pool, caller.user_id).await?,
        (false, false)
    );
    // The codes go with it: they are bypasses for a factor that no longer
    // exists, and whoever copied the list must not keep an authenticator.
    assert_eq!(
        test_support::recovery_code_counts(&db.pool, caller.user_id).await?,
        (0, 0)
    );
    Ok(())
}
