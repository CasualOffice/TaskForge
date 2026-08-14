use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_blocker_in_an_invisible_project_shows_as_restricted_not_absent() -> Result<()> {
    // docs/03: a blocking task "shows as 'restricted' if the viewer cannot see
    // its project, never as its title".
    //
    // Dropping the edge instead would show a task as blocked by nothing — a
    // card that cannot move with no reason given, which is a worse answer than
    // "something you cannot see".
    let db = schema_harness::TestDatabase::start().await?;
    let author = caller(&db.pool, "author@example.com", "acme", RELATER).await?;
    let (project, open_task) = a_task(&author).await?;
    assert!(!project.is_empty());

    // A private project in the same workspace, and a task in it that blocks the
    // open one. The author can see both; the colleague can see only one.
    let (status, secret_project, _) = author
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "SEC", "name": "Secret", "visibility": "PRIVATE" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{secret_project}");
    let secret_id = secret_project["id"].as_str().expect("id").to_owned();
    let hidden = another_task(&author, &secret_id, "Hidden blocker").await?;

    let (status, body, _) = author
        .post(
            &format!("/api/v1/tasks/{hidden}/dependencies"),
            &serde_json::json!({ "blocks": open_task }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let colleague = member_of(&db.pool, "colleague@example.com", author.workspace, RELATER).await?;
    let (status, relations, _) = colleague
        .get(&format!("/api/v1/tasks/{open_task}/dependencies"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{relations}");

    let blockers = relations["blocked_by"].as_array().expect("array");
    assert_eq!(
        blockers.len(),
        1,
        "the edge was dropped, so the task appears blocked by nothing: {relations}"
    );
    assert_eq!(blockers[0]["restricted"], true);
    assert!(
        blockers[0]["title"].is_null(),
        "a title leaked: {relations}"
    );
    assert!(blockers[0]["key"].is_null(), "a key leaked: {relations}");
    assert!(blockers[0]["id"].is_null(), "an id leaked: {relations}");

    // The author, who can see both, gets the real thing — otherwise this test
    // passes with an endpoint that restricts everybody.
    let (_, theirs, _) = author
        .get(&format!("/api/v1/tasks/{open_task}/dependencies"))
        .await?;
    assert_eq!(theirs["blocked_by"][0]["restricted"], false);
    assert_eq!(theirs["blocked_by"][0]["title"], "Hidden blocker");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_board_knows_a_task_is_blocked_without_asking_per_card() -> Result<()> {
    // §12: the board disables the drop target rather than letting the card
    // spring back. That needs blocked-ness in the LIST response — a per-card
    // fetch on a 200-card board is the N+1 docs/04 §The list problem prevents.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;

    // Before: nothing is blocked.
    let (status, page, _) = caller.get("/api/v1/tasks").await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    for row in page["data"].as_array().expect("data") {
        assert_eq!(row["is_blocked"], false, "{row}");
    }

    let (status, _, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED);

    // After: B is blocked, A is not — in the list, with no extra request.
    let (_, page, _) = caller.get("/api/v1/tasks").await?;
    let rows = page["data"].as_array().expect("data");
    let blocked = rows.iter().find(|r| r["id"] == b.as_str()).expect("B");
    let blocker = rows.iter().find(|r| r["id"] == a.as_str()).expect("A");
    assert_eq!(blocked["is_blocked"], true, "{blocked}");
    assert_eq!(blocker["is_blocked"], false, "{blocker}");

    // And on the single read, so the drawer agrees with the board.
    let (_, single, _) = caller.get(&format!("/api/v1/tasks/{b}")).await?;
    assert_eq!(single["is_blocked"], true, "{single}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_task_blocked_by_something_invisible_still_reports_blocked() -> Result<()> {
    // The two rules meet here: the blocker is withheld from the relations list
    // AND the task is still blocked. A board that showed it as draggable would
    // let the user attempt a transition the gate then refuses.
    let db = schema_harness::TestDatabase::start().await?;
    let author = caller(&db.pool, "author@example.com", "acme", RELATER).await?;
    let (_, open_task) = a_task(&author).await?;
    let (status, secret, _) = author
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "SEC", "name": "Secret", "visibility": "PRIVATE" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{secret}");
    let hidden = another_task(&author, secret["id"].as_str().expect("id"), "Hidden").await?;
    let (status, _, _) = author
        .post(
            &format!("/api/v1/tasks/{hidden}/dependencies"),
            &serde_json::json!({ "blocks": open_task }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED);

    let colleague = member_of(&db.pool, "colleague@example.com", author.workspace, RELATER).await?;
    let (_, single, _) = colleague.get(&format!("/api/v1/tasks/{open_task}")).await?;
    assert_eq!(
        single["is_blocked"], true,
        "a task blocked by something invisible reported as draggable: {single}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_dependency_is_removed_from_either_end_and_the_removal_is_recorded() -> Result<()> {
    // Dependencies were add-only. An edge added by mistake gated the blocked
    // task's transitions forever, and the only escape was
    // `task.dependency.override` — an authority for ignoring a *real* blocker,
    // not a way to correct a wrong one.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, first) = a_task(&caller).await?;
    let second = another_task(&caller, &project, "Second").await?;

    caller
        .post(
            &format!("/api/v1/tasks/{first}/dependencies"),
            &serde_json::json!({ "blocks": second }),
            None,
        )
        .await?;

    // Removed from the *blocked* end, naming the blocker — the direction the
    // edge was not created from. One edge joins a pair, so naming both ends
    // identifies it whichever way round the caller thinks of it.
    let (status, relations, _) = caller
        .delete(&format!("/api/v1/tasks/{second}/dependencies/{first}"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{relations}");
    assert!(
        relations["blocked_by"]
            .as_array()
            .expect("array")
            .is_empty(),
        "the edge survived: {relations}"
    );

    // And it is gone from the other end too, which is the assertion that the
    // row was deleted rather than filtered out of one view.
    let (_, theirs, _) = caller
        .get(&format!("/api/v1/tasks/{first}/dependencies"))
        .await?;
    assert!(
        theirs["blocks"].as_array().expect("array").is_empty(),
        "{theirs}"
    );

    // ADR-006: the removal wrote its history in the same transaction.
    let types =
        casual_task_persistence::test_support::outbox_event_types(&db.pool, second.parse()?)
            .await?;
    assert!(
        types.contains(&"task.dependency.removed".to_owned()),
        "the removal wrote no event: {types:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn removing_an_edge_that_is_not_there_is_404_and_not_a_silent_success() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, first) = a_task(&caller).await?;
    let second = another_task(&caller, &project, "Second").await?;

    // Both tasks are visible and there is simply no edge. A 204 here would tell
    // a client its state matched the server's when it did not.
    let (status, body, _) = caller
        .delete(&format!("/api/v1/tasks/{first}/dependencies/{second}"))
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "TF-TSK-0001");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn removing_a_dependency_needs_task_update() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let owner = caller(&db.pool, "owner@example.com", "acme", RELATER).await?;
    let (project, first) = a_task(&owner).await?;
    let second = another_task(&owner, &project, "Second").await?;
    owner
        .post(
            &format!("/api/v1/tasks/{first}/dependencies"),
            &serde_json::json!({ "blocks": second }),
            None,
        )
        .await?;

    // A reader can see the edge and must not be able to cut it. Adding one
    // needs `task.update` (ADR-019: a dependency gates transitions), and
    // removing one changes the same behaviour in the same way.
    let reader = member_of(
        &db.pool,
        "reader@example.com",
        owner.workspace,
        &["task.read"],
    )
    .await?;
    let (status, body, _) = reader
        .delete(&format!("/api/v1/tasks/{second}/dependencies/{first}"))
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (_, still, _) = owner
        .get(&format!("/api/v1/tasks/{second}/dependencies"))
        .await?;
    assert_eq!(
        still["blocked_by"][0]["id"], first,
        "the refusal still cut the edge"
    );
    Ok(())
}
