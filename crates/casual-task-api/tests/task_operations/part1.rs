use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_patch_updates_fields_clears_with_null_and_moves_the_version() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}");

    let (status, body, next) = caller
        .patch(
            &uri,
            &serde_json::json!({
                "title": "Ship it properly",
                "description": "the long version",
                "priority": "HIGH",
                "type": "BUG",
            }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["title"], "Ship it properly");
    assert_eq!(body["priority"], "HIGH");
    assert_eq!(body["type"], "BUG");
    let next = next.context("no ETag on a successful patch")?;
    assert_ne!(next, etag, "the version did not move");

    // docs/05 §Conventions: `null` clears, absent leaves alone. Both in one
    // request, so a handler that collapsed them would fail here.
    let (status, body, _) = caller
        .patch(
            &uri,
            &serde_json::json!({ "description": null }),
            Some(&next),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["description"].is_null(), "null did not clear: {body}");
    assert_eq!(
        body["title"], "Ship it properly",
        "an absent field was not left alone"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_patch_without_if_match_is_428_and_a_stale_one_is_409() -> Result<()> {
    // docs/05 §Concurrency. A client that forgets If-Match has a bug, and
    // failing loudly in development beats losing a user's edit in production.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}");
    let rename = serde_json::json!({ "title": "renamed" });

    let (status, body, _) = caller.patch(&uri, &rename, None).await?;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["error"]["code"], "TF-CNC-0002");

    let (status, body, _) = caller.patch(&uri, &rename, Some("\"nonsense\"")).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-CNC-0003");

    // Win once, then replay the same stale tag.
    let (status, _, _) = caller.patch(&uri, &rename, Some(&etag)).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, body, _) = caller.patch(&uri, &rename, Some(&etag)).await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "TF-CNC-0001");
    // docs/24: the loser is told what it lost to, so it can merge.
    assert!(
        body["error"]["details"]["current_version"].is_number(),
        "{body}"
    );
    assert!(body["error"]["details"]["current"].is_object(), "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_patch_naming_a_status_is_sent_to_the_transition_endpoint() -> Result<()> {
    // docs/23: "Status is never written through PATCH /tasks/{id}. Attempting it
    // returns 400 TF-WFL-0001." The field is DECLARED so the error can say that,
    // rather than deny_unknown_fields calling it a field nobody has heard of.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}");

    for body in [
        serde_json::json!({ "status_id": Uuid::now_v7() }),
        serde_json::json!({ "state": "COMPLETED" }),
    ] {
        let (status, answer, _) = caller.patch(&uri, &body, Some(&etag)).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        assert_eq!(
            answer["error"]["code"], "TF-WFL-0001",
            "a status write was not pointed at the transition endpoint: {answer}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_delete_is_a_tombstone_and_the_task_stops_being_readable() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(
        &db.pool,
        "dev@example.test",
        "acme",
        &[MEMBER, &["task.delete"]].concat(),
    )
    .await?;
    let (_, task, etag) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}");

    let (status, body, _) = caller.delete(&uri, None).await?;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");

    let (status, body, _) = caller.delete(&uri, Some(&etag)).await?;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // docs/03: a delete is a tombstone, not a DELETE. The row survives and the
    // read path does not see it.
    assert!(
        test_support::task_is_deleted(&db.pool, task).await?,
        "the row was hard-deleted"
    );
    let (status, _, _) = caller.get(&uri).await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a deleted task stayed readable"
    );

    let types = test_support::outbox_event_types(&db.pool, task).await?;
    assert!(
        types.contains(&"task.deleted".to_owned()),
        "the delete wrote no event: {types:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_transition_moves_the_task_and_writes_its_history() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;
    let todo = status_ids["Todo"];

    let (status, body, next) = caller
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": todo, "comment": "starting" }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status_id"], todo.to_string());
    // docs/23: state is written in the same statement as status_id, so the two
    // can never disagree. Read from the row, not from the response.
    let (stored_status, stored_state) = test_support::task_status_and_state(&db.pool, task).await?;
    assert_eq!(stored_status, todo);
    assert_eq!(stored_state, "PLANNED", "the derived state drifted");
    assert_eq!(body["state"], "PLANNED");
    assert_ne!(next.context("etag")?, etag);

    // ADR-006: the domain change and all three history rows commit together.
    let (activity, audit, outbox, deliveries) =
        test_support::history_counts(&db.pool, task).await?;
    assert_eq!((activity, audit, outbox), (2, 2, 2), "create + transition");
    // Derived from the registry, not a literal. As a literal this said `2 * 6`
    // and adding a seventh consumer broke it here — in a Docker-only suite,
    // after the unit test that pins the same list had already been updated. The
    // property is "one row per consumer per event"; the number of consumers is
    // not this test's business.
    assert_eq!(
        deliveries,
        2 * i64::try_from(casual_task_persistence::CONSUMERS.len()).expect("consumer count"),
        "one delivery row per consumer per event"
    );
    assert_eq!(
        test_support::outbox_event_types(&db.pool, task).await?,
        vec!["task.created".to_owned(), "task.status.changed".to_owned()]
    );
    // docs/23 §What commits lists the comment among the rows one transaction
    // writes.
    assert_eq!(test_support::comment_count(&db.pool, task).await?, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_status_with_no_edge_from_here_is_refused() -> Result<()> {
    // docs/23 step 4, TF-WFL-0002. The default workflow has no Backlog -> Done
    // edge; work has to pass through Todo and In Progress.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;

    let (status, body, _) = caller
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0002");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn closing_needs_task_close_and_reopening_needs_task_reopen() -> Result<()> {
    // docs/23 §Closing and reopening: closing "requires task.close AND a valid
    // transition edge; both, not either". The default workflow carries the
    // permission on the edge, so this exercises step 5 — TF-WFL-0003, a 403 and
    // not a 422, because the answer is "you may not", not "that is impossible".
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;
    let uri = format!("/api/v1/tasks/{task}");
    let transitions = format!("/api/v1/tasks/{task}/transitions");

    // Backlog -> Todo -> In Progress, neither of which needs a permission.
    let mut tag = etag;
    for name in ["Todo", "In Progress"] {
        let (status, body, next) = caller
            .post_conditional(
                &transitions,
                &serde_json::json!({ "to_status_id": status_ids[name] }),
                Some(&tag),
            )
            .await?;
        assert_eq!(status, StatusCode::OK, "moving to {name}: {body}");
        tag = next.context("etag")?;
    }

    let (status, body, _) = caller
        .post_conditional(
            &transitions,
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            Some(&tag),
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0003");
    assert_eq!(
        body["error"]["details"]["required_permission"],
        "task.close"
    );

    // With the permission, the same move succeeds — so the refusal above was
    // the permission and not the edge.
    let closer = member_of(
        &db.pool,
        "closer@example.test",
        caller.workspace,
        &[MEMBER, &["task.close", "task.reopen"]].concat(),
    )
    .await?;
    let (_, current, tag) = closer.get(&uri).await?;
    assert_eq!(current["status_id"], status_ids["In Progress"].to_string());
    let (status, body, next) = closer
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            tag.as_deref(),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "COMPLETED");

    // And reopening writes a DISTINCT event — docs/23: "how often does work
    // come back?" is a question a generic status-change event cannot serve.
    let (status, body, _) = closer
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["In Progress"] }),
            next.as_deref(),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let types = test_support::outbox_event_types(&db.pool, task).await?;
    assert_eq!(
        types.last().map(String::as_str),
        Some("task.reopened"),
        "leaving a terminal state wrote a generic event: {types:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_move_to_the_status_it_already_has_is_a_no_op() -> Result<()> {
    // docs/23 §Concurrency: "moving to a status the task is already in is a
    // no-op that returns 200 without writing an event. This makes client
    // retries safe without an idempotency key."
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;

    let before = test_support::history_counts(&db.pool, task).await?;
    let (status, body, next) = caller
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Backlog"] }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        next.as_deref(),
        Some(etag.as_str()),
        "a no-op moved the version"
    );
    assert_eq!(
        test_support::history_counts(&db.pool, task).await?,
        before,
        "a no-op transition wrote history"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_blocked_task_cannot_move_until_the_blocker_resolves_or_is_overridden() -> Result<()> {
    // docs/23 step 7, TF-WFL-0005. The error names the blockers the actor can
    // see, which is what makes it actionable rather than merely a refusal.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (project, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;

    let (_, blocker_body, _) = caller
        .post(
            &format!("/api/v1/projects/{project}/tasks"),
            &serde_json::json!({ "title": "Do this first" }),
            Some(&key()),
        )
        .await?;
    let blocker: Uuid = blocker_body["id"].as_str().context("blocker id")?.parse()?;
    test_support::add_blocker(&db.pool, caller.workspace, blocker, task).await?;

    let transitions = format!("/api/v1/tasks/{task}/transitions");
    let (status, body, _) = caller
        .post_conditional(
            &transitions,
            &serde_json::json!({ "to_status_id": status_ids["Todo"] }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0005");
    assert_eq!(
        body["error"]["details"]["blocked_by"][0],
        blocker.to_string(),
        "the blocker was not named: {body}"
    );

    // Cancel is the wildcard edge and opts out of dependency gating entirely,
    // so a blocked task can still be abandoned.
    let (status, body, _) = caller
        .post_conditional(
            &transitions,
            &serde_json::json!({ "to_status_id": status_ids["Canceled"] }),
            Some(&etag),
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "cancel is gated by a blocker: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_first_failure_in_the_documented_order_is_the_one_reported() -> Result<()> {
    // docs/23: "the first failure is the one reported — so the error a user sees
    // is the most actionable one, not whichever check happened to run first."
    //
    // Each request below violates SEVERAL rules at once and must report the
    // earliest. A handler that checked permission before version, or version
    // before visibility, would pass every single-violation test and fail here.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;

    // Step 1 beats everything: an invisible task with a stale version and an
    // unreachable target is a 404, not a 409 or a 422.
    let stranger = signed_in(&db.pool, "other@example.test", "other", MEMBER).await?;
    let (status, body, _) = stranger
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            Some("\"999\""),
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "step 1 did not win: {body}");

    // Step 2 beats step 4: a stale version AND an unreachable status is a 409.
    let (status, body, _) = caller
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            Some("\"999\""),
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "step 2 did not beat step 4: {body}"
    );

    // Step 3 beats step 4: no task.transition AND an unreachable status is a
    // 403 naming the missing grant, not a 422 about the edge.
    let onlooker = member_of(
        &db.pool,
        "onlooker@example.test",
        caller.workspace,
        &["task.read"],
    )
    .await?;
    let (status, body, _) = onlooker
        .post_conditional(
            &format!("/api/v1/tasks/{task}/transitions"),
            &serde_json::json!({ "to_status_id": status_ids["Done"] }),
            Some(&etag),
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "step 3 did not beat step 4: {body}"
    );
    assert_eq!(body["error"]["code"], "TF-AZN-0001");
    Ok(())
}
