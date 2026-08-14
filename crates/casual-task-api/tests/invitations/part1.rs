use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn inviting_an_existing_account_is_indistinguishable_from_inviting_a_stranger() -> Result<()>
{
    // docs/40 §Acceptance gates: "login, reset, and invite responses are
    // indistinguishable for existing and non-existing accounts, in body, status,
    // and timing envelope". THIS IS THE INVITE HALF — the gate that could not
    // close until this endpoint existed.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    // One address has an account already; the other has never been seen.
    test_support::insert_user(
        &db.pool,
        Uuid::now_v7(),
        "known@example.com",
        "Known Person",
    )
    .await?;

    let started = Instant::now();
    let known =
        status_and_body(invite(&app, &owner, workspace, "known@example.com", None).await?).await;
    let known_elapsed = started.elapsed();

    let started = Instant::now();
    let stranger =
        status_and_body(invite(&app, &owner, workspace, "stranger@example.com", None).await?).await;
    let stranger_elapsed = started.elapsed();

    assert_eq!(known.0, stranger.0, "the status differs");
    assert_eq!(known.1, stranger.1, "the body differs");
    assert_eq!(known.0, StatusCode::ACCEPTED);

    // The envelope, not a tight bound. What must not happen is one branch
    // holding the request open for work the other skips.
    let (slower, faster) = if known_elapsed > stranger_elapsed {
        (known_elapsed, stranger_elapsed)
    } else {
        (stranger_elapsed, known_elapsed)
    };
    assert!(
        slower < faster + Duration::from_millis(500),
        "one branch took {slower:?} and the other {faster:?}; that gap is an account oracle"
    );

    // Both were really invited — a response that refused both would satisfy the
    // comparison above and invite nobody.
    assert_eq!(
        test_support::live_invitation_count(&db.pool, workspace).await?,
        2
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_invitation_creates_the_account_adds_membership_and_burns_once() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    assert_eq!(
        invite(&app, &owner, workspace, "newcomer@example.com", None)
            .await?
            .status(),
        StatusCode::ACCEPTED
    );
    let token = token_from(&mailer.message(0).await?)?;

    // Nobody by that address exists yet — the acceptance is what creates them.
    assert_eq!(
        test_support::user_id_for_email(&db.pool, "newcomer@example.com").await?,
        None
    );

    let response = app
        .clone()
        .oneshot(accept_request(&token, Some("New Comer"))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await?;
    let user_id: Uuid = body["user_id"].as_str().context("user_id")?.parse()?;
    assert_eq!(
        body["workspace_id"].as_str().context("ws")?,
        workspace.to_string()
    );

    assert!(test_support::is_member(&db.pool, workspace, user_id).await?);
    assert_eq!(
        test_support::user_id_for_email(&db.pool, "newcomer@example.com").await?,
        Some(user_id)
    );

    // Single use. A link that works twice is a link that works for whoever
    // reads the mailbox next.
    let second = app.clone().oneshot(accept_request(&token, None)?).await?;
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_expired_invitation_is_refused() -> Result<()> {
    // docs/40 gives an invitation seven days. The clock is moved rather than
    // the test waiting for it.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    invite(&app, &owner, workspace, "late@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;

    assert_eq!(
        test_support::expire_invitations(&db.pool, workspace).await?,
        1
    );

    let response = app.clone().oneshot(accept_request(&token, None)?).await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        test_support::user_id_for_email(&db.pool, "late@example.com").await?,
        None,
        "an expired invitation still created an account"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_revoked_invitation_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    invite(&app, &owner, workspace, "withdrawn@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;

    // Find it through the list, which is the only way an inviter gets the id —
    // the constant 202 deliberately does not return one.
    let listed = app
        .clone()
        .oneshot(
            request(
                &owner,
                "GET",
                &format!("/api/v1/workspaces/{workspace}/invitations"),
            )
            .header("x-workspace-id", workspace.to_string())
            .body(Body::empty())?,
        )
        .await?;
    assert_eq!(listed.status(), StatusCode::OK);
    let body = json_body(listed).await?;
    let id = body["data"][0]["id"].as_str().context("invitation id")?;
    assert_eq!(body["data"][0]["email"], "withdrawn@example.com");

    let revoked = app
        .clone()
        .oneshot(
            request(
                &owner,
                "DELETE",
                &format!("/api/v1/workspaces/{workspace}/invitations/{id}"),
            )
            .header("x-workspace-id", workspace.to_string())
            .body(Body::empty())?,
        )
        .await?;
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let response = app.clone().oneshot(accept_request(&token, None)?).await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_invitation_cannot_be_accepted_by_a_different_account() -> Result<()> {
    // docs/40 §Invitations: "tied to the address". Forwarding the email — which
    // people do, in good faith — must not hand membership to the wrong person.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    invite(&app, &owner, workspace, "intended@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;

    // Somebody else, signed in, holding the forwarded link.
    let bystander = sign_up(&app, &db.pool, "bystander@example.com").await?;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/invitations/accept")
                .header(header::COOKIE, &bystander.cookie)
                .header("x-csrf-token", &bystander.csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "token": token }).to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        !test_support::is_member(&db.pool, workspace, bystander.user_id).await?,
        "a forwarded invitation added the wrong person"
    );

    // And the intended recipient can still use it — a refusal that also burned
    // the invitation would lock out the person it was for.
    let intended = app.clone().oneshot(accept_request(&token, None)?).await?;
    assert_eq!(intended.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn accepting_while_signed_in_as_the_invited_address_works() -> Result<()> {
    // The companion to the refusal above: a check that compared the wrong thing
    // would satisfy that test and break every legitimate acceptance.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    let guest = sign_up(&app, &db.pool, "guest@example.com").await?;
    invite(&app, &owner, workspace, "guest@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/invitations/accept")
                .header(header::COOKIE, &guest.cookie)
                .header("x-csrf-token", &guest.csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "token": token }).to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(test_support::is_member(&db.pool, workspace, guest.user_id).await?);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn inviting_twice_leaves_only_the_newest_link_working() -> Result<()> {
    // Someone re-inviting because the first email was lost must not leave two
    // working links in one inbox.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    invite(&app, &owner, workspace, "twice@example.com", None).await?;
    let first = token_from(&mailer.message(0).await?)?;
    invite(&app, &owner, workspace, "twice@example.com", None).await?;
    let second = token_from(&mailer.message(1).await?)?;
    assert_ne!(first, second);
    assert_eq!(
        test_support::live_invitation_count(&db.pool, workspace).await?,
        1
    );

    let stale = app.clone().oneshot(accept_request(&first, None)?).await?;
    assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);
    let fresh = app.clone().oneshot(accept_request(&second, None)?).await?;
    assert_eq!(fresh.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_forged_verifier_against_a_real_selector_is_refused() -> Result<()> {
    // The reason the verifier is stored hashed at all.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    invite(&app, &owner, workspace, "target@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;
    let (selector, _) = token.split_once('.').context("selector.verifier")?;

    for bad in [
        format!("{selector}.{}", "0".repeat(48)),
        String::new(),
        "nonsense".to_owned(),
        "tf_pat_abc.def".to_owned(),
    ] {
        let response = app.clone().oneshot(accept_request(&bad, None)?).await?;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "accepted {bad:?}"
        );
    }

    assert_eq!(
        test_support::user_id_for_email(&db.pool, "target@example.com").await?,
        None
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_stored_invitation_is_not_a_usable_link() -> Result<()> {
    // docs/40 §Acceptance gates, "token-hash test": a database dump contains no
    // usable credential. Asserted against what is IN the table.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;
    invite(&app, &owner, workspace, "dump@example.com", None).await?;
    let token = token_from(&mailer.message(0).await?)?;
    let (_, verifier) = token.split_once('.').context("selector.verifier")?;

    for stored in test_support::invitation_columns(&db.pool, workspace).await? {
        assert!(
            !stored.contains(verifier),
            "the verifier is recoverable from the stored row: {stored}"
        );
        assert!(
            !stored.contains(&token),
            "the whole token is in the stored row: {stored}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_invitation_in_another_workspace_is_invisible_and_unrevocable() -> Result<()> {
    // docs/04: absent and invisible are never disambiguated, and docs/32 says
    // no data crosses a workspace boundary.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let alice = sign_up(&app, &db.pool, "alice@example.com").await?;
    let alice_ws = create_workspace(&app, &alice, "alice-co").await?;
    invite(&app, &alice, alice_ws, "invitee@example.com", None).await?;

    let mallory = sign_up(&app, &db.pool, "mallory@example.com").await?;
    let mallory_ws = create_workspace(&app, &mallory, "mallory-co").await?;

    // Mallory cannot see Alice's invitations from her own workspace.
    let listed = app
        .clone()
        .oneshot(
            request(
                &mallory,
                "GET",
                &format!("/api/v1/workspaces/{mallory_ws}/invitations"),
            )
            .header("x-workspace-id", mallory_ws.to_string())
            .body(Body::empty())?,
        )
        .await?;
    let body = json_body(listed).await?;
    assert_eq!(body["data"].as_array().context("data")?.len(), 0);

    // Nor revoke one by id, even naming Alice's workspace in the path: the
    // membership check refuses her before the handler runs.
    let denied = app
        .clone()
        .oneshot(
            request(
                &mallory,
                "DELETE",
                &format!(
                    "/api/v1/workspaces/{alice_ws}/invitations/{}",
                    Uuid::now_v7()
                ),
            )
            .header("x-workspace-id", alice_ws.to_string())
            .body(Body::empty())?,
        )
        .await?;
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);

    assert_eq!(
        test_support::live_invitation_count(&db.pool, alice_ws).await?,
        1,
        "Alice's invitation was disturbed"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn inviting_with_a_role_the_inviter_does_not_hold_is_refused() -> Result<()> {
    // docs/04 control 1: you cannot grant what you do not hold. An invitation
    // carrying a role is a DEFERRED GRANT, and without this it would be a way
    // around `role.assign` — the escalation hole D-049 exists to prevent.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    // The INVITER must not be the owner. D-054 grants a workspace creator the
    // Owner role, which is asserted against `permission::ALL` — so the creator
    // legitimately holds every permission and control 1 has nothing to refuse.
    // This test needs someone who may invite and may not delete.
    let inviter = sign_up(&app, &db.pool, "inviter@example.com").await?;
    test_support::add_workspace_member(&db.pool, workspace, inviter.user_id).await?;
    test_support::grant_at_workspace(&db.pool, workspace, inviter.user_id, &["role.assign"])
        .await?;

    // A powerful role exists and somebody else holds it; the inviter does not.
    // Granted to a REAL account: `role_assignment.granted_by` is a foreign key,
    // so a made-up uuid fails the insert rather than the assertion.
    let admin = Uuid::now_v7();
    test_support::insert_user(&db.pool, admin, "admin@example.com", "Admin").await?;
    let powerful =
        test_support::grant_at_workspace(&db.pool, workspace, admin, &["workspace.delete"]).await?;

    let response = invite(
        &app,
        &inviter,
        workspace,
        "escalate@example.com",
        Some(powerful),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        test_support::live_invitation_count(&db.pool, workspace).await?,
        0,
        "a refused invitation was still created"
    );
    Ok(())
}
