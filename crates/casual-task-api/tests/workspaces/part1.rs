use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn creating_a_workspace_makes_the_creator_a_member() -> Result<()> {
    // The unblocking property: without it a signed-in user has no workspace and
    // nothing else in the product is reachable.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;

    let (workspace, _) = create_workspace(&app, &caller, "acme").await?;

    let read = send(
        &app,
        request(&caller, "GET", &format!("/api/v1/workspaces/{workspace}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(read.status(), StatusCode::OK, "the creator cannot read it");
    let body = json_body(read).await?;
    assert_eq!(body["slug"], "acme");
    assert_eq!(body["name"], "Workspace acme");
    // docs/05 §Conventions: RFC 3339, always UTC, always Z.
    assert!(
        body["created_at"]
            .as_str()
            .unwrap_or_default()
            .ends_with('Z'),
        "created_at is not UTC-with-Z: {body}"
    );

    let members = json_body(
        send(
            &app,
            request(
                &caller,
                "GET",
                &format!("/api/v1/workspaces/{workspace}/members"),
            )
            .body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(members["data"][0]["user_id"], caller.user_id.to_string());
    assert_eq!(members["data"][0]["member_type"], "MEMBER");

    let mine = json_body(
        send(
            &app,
            request(&caller, "GET", "/api/v1/workspaces").body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(mine["data"][0]["id"], workspace.to_string());
    assert_eq!(mine["page"]["has_more"], false);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_non_member_is_told_404_and_not_403() -> Result<()> {
    // docs/04: absent and invisible are never disambiguated. A 403 on a real
    // workspace and a 404 on an imaginary one is how workspace ids get
    // enumerated by an authenticated stranger.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.test").await?;
    let stranger = sign_up(&app, &db.pool, "stranger@example.test").await?;
    let (workspace, _) = create_workspace(&app, &owner, "private").await?;

    let imaginary = Uuid::now_v7();
    for (label, id) in [("real", workspace), ("imaginary", imaginary)] {
        for path in [
            format!("/api/v1/workspaces/{id}"),
            format!("/api/v1/workspaces/{id}/members"),
            format!("/api/v1/workspaces/{id}/teams"),
        ] {
            let response =
                send(&app, request(&stranger, "GET", &path).body(Body::empty())?).await?;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{label} {path} answered something other than 404"
            );
            let body = json_body(response).await?;
            assert_eq!(
                body["error"]["code"], "TF-AZN-0008",
                "{label} {path} used a distinguishable error code"
            );
        }
    }

    // And the stranger's own list does not mention it.
    let mine = json_body(
        send(
            &app,
            request(&stranger, "GET", "/api/v1/workspaces").body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(mine["data"].as_array().map(Vec::len), Some(0));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn removing_a_member_stops_their_access_on_the_very_next_request() -> Result<()> {
    // Revocation that takes effect "eventually" is a permission hole with a
    // schedule. The membership check runs on every request precisely so this
    // holds without anything being invalidated first.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.test").await?;
    let guest = sign_up(&app, &db.pool, "guest@example.test").await?;
    let (workspace, _) = create_workspace(&app, &owner, "shared").await?;

    let added = send(
        &app,
        request(
            &owner,
            "POST",
            &format!("/api/v1/workspaces/{workspace}/members"),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "user_id": guest.user_id }).to_string()))?,
    )
    .await?;
    assert_eq!(added.status(), StatusCode::CREATED);

    let before = send(
        &app,
        request(&guest, "GET", &format!("/api/v1/workspaces/{workspace}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(before.status(), StatusCode::OK, "a new member was refused");

    let removed = send(
        &app,
        request(
            &owner,
            "DELETE",
            &format!("/api/v1/workspaces/{workspace}/members/{}", guest.user_id),
        )
        .body(Body::empty())?,
    )
    .await?;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    let after = send(
        &app,
        request(&guest, "GET", &format!("/api/v1/workspaces/{workspace}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(
        after.status(),
        StatusCode::NOT_FOUND,
        "a removed member kept their access"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unknown_json_field_is_refused_with_400() -> Result<()> {
    // docs/05 §Conventions: unknown request fields are "rejected with 400 —
    // silently ignoring a typo'd field is how clients ship bugs that look like
    // server bugs". axum's own Json rejection is a 422 with a bare text body,
    // which is why this endpoint does not use it.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;

    let response = send(
        &app,
        request(&caller, "POST", "/api/v1/workspaces")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": "Acme", "slug": "acme", "plan": "enterprise" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await?;
    assert_eq!(body["error"]["code"], "TF-VAL-0002");
    assert_eq!(body["error"]["details"]["unknown_fields"][0], "plan");
    assert!(
        !body["error"]["request_id"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "the envelope lost its request id: {body}"
    );

    // The same discipline on query parameters.
    let (workspace, _) = create_workspace(&app, &caller, "acme").await?;
    let response = send(
        &app,
        request(
            &caller,
            "GET",
            &format!("/api/v1/workspaces/{workspace}/members?limti=2"),
        )
        .body(Body::empty())?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_rename_is_conditional_on_the_version_the_caller_read() -> Result<()> {
    // docs/05 §Concurrency: 428 without If-Match, 409 against a stale one. A
    // rename that silently accepted an unconditional write would lose the other
    // editor's change without anyone being told.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;
    let (workspace, etag) = create_workspace(&app, &caller, "acme").await?;
    let uri = format!("/api/v1/workspaces/{workspace}");

    let unconditional = send(
        &app,
        request(&caller, "PATCH", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "name": "Renamed" }).to_string()))?,
    )
    .await?;
    assert_eq!(
        unconditional.status(),
        StatusCode::PRECONDITION_REQUIRED,
        "an unconditional PATCH was accepted"
    );

    let ok = send(
        &app,
        request(&caller, "PATCH", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_MATCH, &etag)
            .body(Body::from(json!({ "name": "Renamed" }).to_string()))?,
    )
    .await?;
    assert_eq!(ok.status(), StatusCode::OK);
    let next_etag = ok
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .context("etag")?
        .to_owned();
    assert_ne!(next_etag, etag, "the version did not move");
    assert_eq!(json_body(ok).await?["name"], "Renamed");

    let stale = send(
        &app,
        request(&caller, "PATCH", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_MATCH, &etag)
            .body(Body::from(json!({ "name": "Again" }).to_string()))?,
    )
    .await?;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let body = json_body(stale).await?;
    assert_eq!(body["error"]["code"], "TF-CNC-0001");
    // docs/24: the loser is told what it lost to, so it can re-read and merge.
    assert!(
        body["error"]["details"]["current_version"].is_number(),
        "{body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_mutation_writes_its_history_in_the_same_transaction() -> Result<()> {
    // ADR-006, docs/25: the domain change, its activity row, its audit row and
    // its outbox event commit together or not at all. A membership change with
    // no audit row is exactly what UnitOfWork exists to prevent.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.test").await?;
    let guest = sign_up(&app, &db.pool, "guest@example.test").await?;
    let (workspace, _) = create_workspace(&app, &owner, "acme").await?;

    send(
        &app,
        request(
            &owner,
            "POST",
            &format!("/api/v1/workspaces/{workspace}/members"),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "user_id": guest.user_id }).to_string()))?,
    )
    .await?;
    send(
        &app,
        request(
            &owner,
            "DELETE",
            &format!("/api/v1/workspaces/{workspace}/members/{}", guest.user_id),
        )
        .body(Body::empty())?,
    )
    .await?;

    let history = test_support::history(&db.pool, workspace).await?;
    let expected = vec![
        "workspace.created".to_owned(),
        "workspace.member.added".to_owned(),
        "workspace.member.removed".to_owned(),
    ];
    assert_eq!(history.activity, expected, "activity stream");
    assert_eq!(history.audit, expected, "audit stream");
    assert_eq!(history.outbox, expected, "outbox");
    assert_eq!(
        history.deliveries,
        i64::try_from(expected.len() * casual_task_persistence::CONSUMERS.len())?,
        "one delivery row per consumer per event (docs/25 §Consumer fan-out)"
    );

    // docs/04 §Caching: the epoch moves with every membership change, in the
    // same transaction, so a stale permission-cache entry simply misses.
    assert_eq!(
        test_support::authz_epoch(&db.pool, workspace).await?,
        3,
        "authz_epoch did not move with the two membership changes"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_workspace_cannot_lose_its_last_member() -> Result<()> {
    // Nothing can see a workspace with no members, so nothing can add one back
    // to it. Refusing is the only outcome that does not silently destroy data.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.test").await?;
    let (workspace, _) = create_workspace(&app, &owner, "acme").await?;

    let response = send(
        &app,
        request(
            &owner,
            "DELETE",
            &format!("/api/v1/workspaces/{workspace}/members/{}", owner.user_id),
        )
        .body(Body::empty())?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json_body(response).await?["error"]["code"], "TF-PRJ-0006");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn teams_cannot_be_reached_or_populated_across_a_tenant_boundary() -> Result<()> {
    // `team_membership` carries no workspace_id and therefore no RLS policy
    // (migration 0010). Both halves of its tenant boundary are asserted here:
    // the team must be visible in the caller's workspace, and the user must be
    // a member of it.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let alice = sign_up(&app, &db.pool, "alice@example.test").await?;
    let bob = sign_up(&app, &db.pool, "bob@example.test").await?;
    let (alpha, _) = create_workspace(&app, &alice, "alpha").await?;
    let (beta, _) = create_workspace(&app, &bob, "beta").await?;

    let created = send(
        &app,
        request(&alice, "POST", &format!("/api/v1/workspaces/{alpha}/teams"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "name": "Platform" }).to_string()))?,
    )
    .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let team: Uuid = json_body(created).await?["id"]
        .as_str()
        .context("team id")?
        .parse()?;

    // Bob is a member of beta, and points a beta-scoped request at alpha's team.
    let across = send(
        &app,
        request(&bob, "POST", &format!("/api/v1/teams/{team}/members"))
            .header(WORKSPACE_HEADER, beta.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": bob.user_id }).to_string()))?,
    )
    .await?;
    assert_eq!(
        across.status(),
        StatusCode::NOT_FOUND,
        "a team in another tenant was reachable"
    );

    // Alice can reach her own team, but not put a stranger in it.
    let stranger = send(
        &app,
        request(&alice, "POST", &format!("/api/v1/teams/{team}/members"))
            .header(WORKSPACE_HEADER, alpha.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": bob.user_id }).to_string()))?,
    )
    .await?;
    assert_eq!(stranger.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        json_body(stranger).await?["error"]["code"],
        "TF-VAL-0007",
        "a non-member was added to a team"
    );

    // And the happy path, so the refusals above are not just "everything fails".
    let own = send(
        &app,
        request(&alice, "POST", &format!("/api/v1/teams/{team}/members"))
            .header(WORKSPACE_HEADER, alpha.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": alice.user_id }).to_string()))?,
    )
    .await?;
    assert_eq!(own.status(), StatusCode::CREATED);

    let removed = send(
        &app,
        request(
            &alice,
            "DELETE",
            &format!("/api/v1/teams/{team}/members/{}", alice.user_id),
        )
        .header(WORKSPACE_HEADER, alpha.to_string())
        .body(Body::empty())?,
    )
    .await?;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_path_and_a_header_that_disagree_are_refused() -> Result<()> {
    // Preferring one silently would mean the caller cannot tell which workspace
    // they were answered about.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let alice = sign_up(&app, &db.pool, "alice@example.test").await?;
    let (alpha, _) = create_workspace(&app, &alice, "alpha").await?;
    let (beta, _) = create_workspace(&app, &alice, "beta").await?;

    let response = send(
        &app,
        request(&alice, "GET", &format!("/api/v1/workspaces/{alpha}"))
            .header(WORKSPACE_HEADER, beta.to_string())
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a request naming two different workspaces was answered"
    );
    Ok(())
}
