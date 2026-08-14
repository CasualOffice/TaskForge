use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_invited_role_is_granted_on_acceptance_and_credited_to_the_inviter() -> Result<()> {
    // The companion to the refusal above, and the audit property: `granted_by`
    // is the INVITER. Recording the acceptor would read, years later, as a
    // self-grant.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    // The inviter holds the role they are handing out, and `role.assign`.
    let role = test_support::grant_at_workspace(
        &db.pool,
        workspace,
        owner.user_id,
        &["role.assign", "task.read"],
    )
    .await?;

    let response = invite(&app, &owner, workspace, "colleague@example.com", Some(role)).await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let token = token_from(&mailer.message(0).await?)?;

    let accepted = app.clone().oneshot(accept_request(&token, None)?).await?;
    assert_eq!(accepted.status(), StatusCode::OK);
    let body = json_body(accepted).await?;
    let user_id: Uuid = body["user_id"].as_str().context("user_id")?.parse()?;

    let grants = test_support::workspace_grants_for_user(&db.pool, workspace, user_id).await?;
    assert_eq!(grants.len(), 1, "the invited role was not granted");
    assert_eq!(grants[0].0, role);
    assert_eq!(
        grants[0].1, owner.user_id,
        "the grant was credited to the acceptor rather than the inviter"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_role_from_another_workspace_is_refused_at_invite_time() -> Result<()> {
    // Refused when the invitation is written, not when it is accepted — the
    // invitee would otherwise join with no role and nobody would know why.
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let alice = sign_up(&app, &db.pool, "alice@example.com").await?;
    let alice_ws = create_workspace(&app, &alice, "alice-co").await?;
    let other_ws = create_workspace(&app, &alice, "other-co").await?;
    let foreign =
        test_support::grant_at_workspace(&db.pool, other_ws, alice.user_id, &["task.read"]).await?;
    test_support::grant_at_workspace(&db.pool, alice_ws, alice.user_id, &["role.assign"]).await?;

    let response = invite(&app, &alice, alice_ws, "x@example.com", Some(foreign)).await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(response).await?;
    assert_eq!(body["error"]["code"], "TF-VAL-0007");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_malformed_address_is_refused_before_anything_is_written() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let mailer = Arc::new(Recording::default());
    let app = app(db.pool.clone(), mailer.clone());

    let owner = sign_up(&app, &db.pool, "owner@example.com").await?;
    let workspace = create_workspace(&app, &owner, "acme").await?;

    // The header-injection case first: it must never reach the mailer.
    for bad in ["nope", "a@b@c.com", "user@example.com\r\nBcc: x@y.com", ""] {
        let response = invite(&app, &owner, workspace, bad, None).await?;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "accepted {bad:?}"
        );
    }
    assert_eq!(
        test_support::live_invitation_count(&db.pool, workspace).await?,
        0
    );
    assert_eq!(mailer.count(), 0, "a malformed address reached the mailer");
    Ok(())
}
