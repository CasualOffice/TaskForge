use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_new_routes_are_inside_the_csrf_guard() -> Result<()> {
    // The rule this asserts is about the ROUTER, not the handler: a route
    // registered after `.layer()` escapes both the CSRF guard and the request
    // id, and nothing about the handler would show it.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;

    let response = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/api/v1/workspaces")
            .header(header::COOKIE, &caller.cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": "Acme", "slug": "acme" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a state-changing request succeeded with only a session cookie"
    );
    assert!(
        response.headers().contains_key("x-request-id"),
        "the route is outside the observability layer too"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_list_pages_by_cursor_and_never_by_offset() -> Result<()> {
    // docs/05 §Pagination and docs/26: cursor pagination everywhere, opaque to
    // the client, with the probe row used to answer has_more without a count.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;
    let mut created = Vec::new();
    for slug in ["one", "two", "three"] {
        created.push(create_workspace(&app, &caller, slug).await?.0);
    }
    created.sort_unstable();

    let first = json_body(
        send(
            &app,
            request(&caller, "GET", "/api/v1/workspaces?limit=2").body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(first["data"].as_array().map(Vec::len), Some(2));
    assert_eq!(first["page"]["has_more"], true);
    let cursor = first["page"]["next_cursor"]
        .as_str()
        .context("no cursor on a page that has more")?
        .to_owned();

    let second = json_body(
        send(
            &app,
            request(
                &caller,
                "GET",
                &format!("/api/v1/workspaces?limit=2&cursor={cursor}"),
            )
            .body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(second["data"].as_array().map(Vec::len), Some(1));
    assert_eq!(second["page"]["has_more"], false);
    assert_eq!(second["data"][0]["id"], created[2].to_string());

    // Bounds are enforced rather than clamped: a silently shortened page is
    // indistinguishable to the client from a short last page.
    let over = send(
        &app,
        request(&caller, "GET", "/api/v1/workspaces?limit=101").body(Body::empty())?,
    )
    .await?;
    assert_eq!(over.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_taken_slug_is_refused_and_nothing_is_left_behind() -> Result<()> {
    // The transaction must roll back completely: a workspace row that failed to
    // commit but left an audit row would be history for something that never
    // happened.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let alice = sign_up(&app, &db.pool, "alice@example.test").await?;
    let bob = sign_up(&app, &db.pool, "bob@example.test").await?;
    create_workspace(&app, &alice, "acme").await?;

    let response = send(
        &app,
        request(&bob, "POST", "/api/v1/workspaces")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": "Also Acme", "slug": "acme" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(json_body(response).await?["error"]["code"], "TF-PRJ-0007");

    let mine = json_body(
        send(
            &app,
            request(&bob, "GET", "/api/v1/workspaces").body(Body::empty())?,
        )
        .await?,
    )
    .await?;
    assert_eq!(
        mine["data"].as_array().map(Vec::len),
        Some(0),
        "a failed creation left the caller a member of something"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_bad_slug_or_name_is_refused_before_anything_is_written() -> Result<()> {
    // Every input bounded (AGENTS.md §Engineering priorities 4). A slug reaches
    // a URL, so its character set is decided here rather than inherited from
    // whatever the client sent.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let caller = sign_up(&app, &db.pool, "founder@example.test").await?;

    for (name, slug) in [
        ("Acme", "Not Lowercase"),
        ("Acme", "-leading-dash"),
        ("Acme", ""),
        ("", "acme"),
        ("   ", "acme"),
    ] {
        let response = send(
            &app,
            request(&caller, "POST", "/api/v1/workspaces")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "name": name, "slug": slug }).to_string(),
                ))?,
        )
        .await?;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "accepted name={name:?} slug={slug:?}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn adding_a_member_twice_is_not_an_error() -> Result<()> {
    // A client that retries a request whose response it never saw is doing the
    // right thing; an error there makes correct behaviour look broken.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let owner = sign_up(&app, &db.pool, "owner@example.test").await?;
    let guest = sign_up(&app, &db.pool, "guest@example.test").await?;
    let (workspace, _) = create_workspace(&app, &owner, "acme").await?;
    let uri = format!("/api/v1/workspaces/{workspace}/members");

    let first = send(
        &app,
        request(&owner, "POST", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "user_id": guest.user_id, "member_type": "GUEST" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(json_body(first).await?["member_type"], "GUEST");

    let again = send(
        &app,
        request(&owner, "POST", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": guest.user_id }).to_string()))?,
    )
    .await?;
    assert_eq!(again.status(), StatusCode::OK);
    assert_eq!(
        json_body(again).await?["member_type"],
        "GUEST",
        "a repeat add rewrote the member type it did not ask about"
    );

    // An unknown person is a domain-rule violation, not a 500.
    let nobody = send(
        &app,
        request(&owner, "POST", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": Uuid::now_v7() }).to_string()))?,
    )
    .await?;
    assert_eq!(nobody.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json_body(nobody).await?["error"]["code"], "TF-VAL-0007");

    // And an unknown member type is refused rather than stored.
    let bad = send(
        &app,
        request(&owner, "POST", &uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "user_id": guest.user_id, "member_type": "OWNER" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn creating_a_workspace_makes_the_creator_its_owner() -> Result<()> {
    // The defect this closes: `role_assignment` is the only source of authority
    // (migration 0003), and nothing created one. The workspace committed, its
    // creator was its only member, and every write they could ever attempt was
    // refused — with no way out, because granting requires a grant.
    //
    // Asserted through the HTTP route rather than against the repository,
    // because the guarantee is about what a *request* leaves behind.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let founder = sign_up(&app, &db.pool, "founder@example.com").await?;
    let (workspace, _) = create_workspace(&app, &founder, "acme").await?;

    let grants = test_support::workspace_grants(&db.pool, workspace).await?;
    assert!(
        grants.iter().any(|(principal, role, permission)| {
            *principal == founder.user_id && role == "Owner" && permission == "workspace.owner"
        }),
        "POST /api/v1/workspaces left a workspace with no owner: {grants:?}"
    );

    // Exactly one owner, and it is the creator. A bootstrap that granted the
    // role twice, or to somebody else as well, would pass the assertion above.
    let owners: Vec<Uuid> = grants
        .iter()
        .filter(|(_, _, permission)| permission == "workspace.owner")
        .map(|(principal, _, _)| *principal)
        .collect();
    assert_eq!(owners, vec![founder.user_id]);

    // All five templates, with the sets docs/04 describes.
    let templates = test_support::role_templates(&db.pool, workspace).await?;
    let names: Vec<&str> = templates.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Administrator",
            "Guest",
            "Member",
            "Owner",
            "Project Manager"
        ],
        "the five docs/04 templates were not materialized"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_owner_grant_is_audited_in_the_transaction_that_made_it() -> Result<()> {
    // docs/04 control 7: "Every grant, revoke, role edit, and consent writes an
    // `audit_event` with before/after." A grant nobody can find in the audit
    // trail is a grant nobody can explain during an incident.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let founder = sign_up(&app, &db.pool, "founder@example.com").await?;
    let (workspace, _) = create_workspace(&app, &founder, "acme").await?;

    let audited = test_support::audit_changes(&db.pool, workspace).await?;
    let grant = audited
        .iter()
        .find_map(|entry| entry.get("after")?.get("role_assignment"))
        .context("no audit record names the owner grant")?;

    assert_eq!(grant["role_name"], "Owner");
    assert_eq!(grant["scope_type"], "WORKSPACE");
    assert_eq!(grant["principal_type"], "USER");
    assert_eq!(grant["principal_id"], founder.user_id.to_string());
    assert_eq!(grant["scope_id"], workspace.to_string());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_refused_create_leaves_no_roles_behind() -> Result<()> {
    // The bootstrap runs in the same transaction as the workspace row, so a
    // create that fails after the row is written must leave nothing — no
    // workspace, no templates, no grant. A taken slug is the failure that
    // actually happens.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let founder = sign_up(&app, &db.pool, "founder@example.com").await?;
    let rival = sign_up(&app, &db.pool, "rival@example.com").await?;
    let (workspace, _) = create_workspace(&app, &founder, "acme").await?;

    let refused = send(
        &app,
        request(&rival, "POST", "/api/v1/workspaces")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "name": "Also Acme", "slug": "acme" }).to_string(),
            ))?,
    )
    .await?;
    assert_eq!(refused.status(), StatusCode::CONFLICT);

    // The winner's workspace is untouched: still five templates and one owner,
    // and the loser's rolled-back attempt added nothing to it.
    assert_eq!(
        test_support::role_templates(&db.pool, workspace)
            .await?
            .len(),
        5
    );
    let owners: Vec<Uuid> = test_support::workspace_grants(&db.pool, workspace)
        .await?
        .into_iter()
        .filter(|(_, _, permission)| permission == "workspace.owner")
        .map(|(principal, _, _)| principal)
        .collect();
    assert_eq!(owners, vec![founder.user_id]);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_team_can_be_read_back_and_the_read_stops_at_the_tenant_boundary() -> Result<()> {
    // Team membership used to be write-only: POST added and DELETE removed, and
    // nothing could say who was in a team. A team is a *principal* a grant is
    // assigned to (`docs/04`), so "who does this grant reach?" was unanswerable.
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
    let team: Uuid = json_body(created).await?["id"]
        .as_str()
        .context("team id")?
        .parse()?;

    // Empty before anyone is in it, and a page rather than a bare array.
    let empty = send(
        &app,
        request(&alice, "GET", &format!("/api/v1/teams/{team}/members"))
            .header(WORKSPACE_HEADER, alpha.to_string())
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(empty.status(), StatusCode::OK);
    let body = json_body(empty).await?;
    assert!(
        body["data"].as_array().context("data")?.is_empty(),
        "{body}"
    );
    assert_eq!(body["page"]["has_more"], false, "{body}");

    send(
        &app,
        request(&alice, "POST", &format!("/api/v1/teams/{team}/members"))
            .header(WORKSPACE_HEADER, alpha.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "user_id": alice.user_id }).to_string()))?,
    )
    .await?;

    let listed = send(
        &app,
        request(&alice, "GET", &format!("/api/v1/teams/{team}/members"))
            .header(WORKSPACE_HEADER, alpha.to_string())
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(listed.status(), StatusCode::OK);
    let body = json_body(listed).await?;
    let rows = body["data"].as_array().context("data")?;
    assert_eq!(rows.len(), 1, "{body}");
    assert_eq!(rows[0]["user_id"], alice.user_id.to_string());
    // The fixture names every user `Test`; what matters is that the join
    // reached `user_account` at all rather than returning a bare id.
    assert_eq!(rows[0]["display_name"], "Test");
    assert_eq!(rows[0]["email"], "alice@example.test");
    // No `joined_at`: `team_membership` is (team_id, user_id) and nothing else,
    // so a date here could only be the workspace's, answering another question.
    assert!(rows[0].get("joined_at").is_none(), "{body}");

    // Bob, in beta, cannot read alpha's team — 404, the same answer an absent
    // team gives, so the endpoint is not a team-existence oracle.
    let across = send(
        &app,
        request(&bob, "GET", &format!("/api/v1/teams/{team}/members"))
            .header(WORKSPACE_HEADER, beta.to_string())
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(across.status(), StatusCode::NOT_FOUND);
    Ok(())
}
