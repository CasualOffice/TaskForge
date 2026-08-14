use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn my_teams_answers_for_the_caller_and_stops_at_the_tenant_boundary() -> Result<()> {
    // The sidebar's list. "Which teams exist here" and "which am I in" are
    // different questions asked by different screens, and answering the second
    // with the first would give a person in three of a hundred teams a hundred
    // rows to read.
    let db = schema_harness::TestDatabase::start().await?;
    let app = app(db.pool.clone());
    let alice = sign_up(&app, &db.pool, "alice@example.test").await?;
    let bob = sign_up(&app, &db.pool, "bob@example.test").await?;
    let (alpha, _) = create_workspace(&app, &alice, "alpha").await?;
    let (beta, _) = create_workspace(&app, &bob, "beta").await?;

    let mut made = Vec::new();
    for (owner, workspace, name) in [
        (&alice, alpha, "Backend"),
        (&alice, alpha, "Android"),
        (&bob, beta, "Someone else's"),
    ] {
        let created = send(
            &app,
            request(
                owner,
                "POST",
                &format!("/api/v1/workspaces/{workspace}/teams"),
            )
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "name": name }).to_string()))?,
        )
        .await?;
        let id: Uuid = json_body(created).await?["id"]
            .as_str()
            .context("team id")?
            .parse()?;
        made.push(id);
    }

    // Nobody is in anything yet, so the honest answer is nothing — not "every
    // team in the workspace".
    let none = send(
        &app,
        request(&alice, "GET", "/api/v1/me/teams")
            .header(WORKSPACE_HEADER, alpha.to_string())
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(none.status(), StatusCode::OK);
    let body = json_body(none).await?;
    assert!(
        body["data"].as_array().context("data")?.is_empty(),
        "a team nobody joined was reported as the caller's: {body}"
    );

    // In one of the two.
    send(
        &app,
        request(
            &alice,
            "POST",
            &format!("/api/v1/teams/{}/members", made[0]),
        )
        .header(WORKSPACE_HEADER, alpha.to_string())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "user_id": alice.user_id }).to_string()))?,
    )
    .await?;

    let mine = send(
        &app,
        request(&alice, "GET", "/api/v1/me/teams")
            .header(WORKSPACE_HEADER, alpha.to_string())
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(mine.status(), StatusCode::OK);
    let body = json_body(mine).await?;
    let listed = body["data"].as_array().context("data")?;
    assert_eq!(listed.len(), 1, "{body}");
    assert_eq!(listed[0]["name"], "Backend", "{body}");

    // And bob, in his own workspace, sees his own — never alice's, and never
    // the team he made in a tenant she cannot reach.
    let his = send(
        &app,
        request(&bob, "GET", "/api/v1/me/teams")
            .header(WORKSPACE_HEADER, beta.to_string())
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(his.status(), StatusCode::OK);
    let body = json_body(his).await?;
    assert!(
        body["data"].as_array().context("data")?.is_empty(),
        "another tenant's team leaked into the caller's list: {body}"
    );

    Ok(())
}
