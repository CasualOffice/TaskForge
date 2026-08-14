use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_tasks_history_is_readable_and_newest_first() -> Result<()> {
    // Every change has written an activity record in the same transaction as
    // the change since C-011 (ADR-006). Until now nothing read them: the data
    // accumulated and the History tab had nothing to call.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (_, task) = a_task(&caller).await?;

    // A second change, so ordering means something.
    let (status, _, etag) = caller.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, updated, _) = caller
        .patch(
            &format!("/api/v1/tasks/{task}"),
            &serde_json::json!({ "title": "Renamed" }),
            etag.as_deref(),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let (status, page, _) = caller
        .get(&format!("/api/v1/tasks/{task}/activity"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    let entries = page["data"].as_array().expect("data");
    assert!(entries.len() >= 2, "history is missing entries: {page}");

    // Newest first (docs/05: the History tab reads top-down).
    assert_eq!(entries[0]["event_type"], "task.updated");
    assert_eq!(
        entries[entries.len() - 1]["event_type"],
        "task.created",
        "the oldest entry is not the create"
    );
    // The actor is resolved for rendering, not left as a bare id.
    assert!(
        entries[0]["actor_name"].is_string(),
        "no actor name to render: {}",
        entries[0]
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn history_pages_by_cursor_without_repeating_or_skipping() -> Result<()> {
    // docs/26 bans OFFSET. activity_event is partitioned by occurred_at, so the
    // cursor carries the timestamp as well as the id — an id-only cursor cannot
    // be resumed without scanning every partition.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (_, task) = a_task(&caller).await?;

    for n in 0..4 {
        let (_, current, etag) = caller.get(&format!("/api/v1/tasks/{task}")).await?;
        assert!(current["id"].is_string());
        let (status, body, _) = caller
            .patch(
                &format!("/api/v1/tasks/{task}"),
                &serde_json::json!({ "title": format!("Rename {n}") }),
                etag.as_deref(),
            )
            .await?;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let mut seen: Vec<String> = Vec::new();
    let mut next: Option<String> = None;
    for _ in 0..6 {
        let uri = next.map_or_else(
            || format!("/api/v1/tasks/{task}/activity?limit=2"),
            |c| format!("/api/v1/tasks/{task}/activity?limit=2&cursor={c}"),
        );
        let (status, page, _) = caller.get(&uri).await?;
        assert_eq!(status, StatusCode::OK, "{page}");
        for entry in page["data"].as_array().expect("data") {
            seen.push(entry["id"].as_str().expect("id").to_owned());
        }
        next = page["page"]["next_cursor"].as_str().map(ToOwned::to_owned);
        if next.is_none() {
            break;
        }
    }

    let unique: std::collections::HashSet<&String> = seen.iter().collect();
    assert_eq!(
        unique.len(),
        seen.len(),
        "an entry was served twice: {seen:?}"
    );
    assert!(seen.len() >= 5, "paging lost entries: {seen:?}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn history_of_an_invisible_task_is_404_and_never_403() -> Result<()> {
    // docs/04: absent and invisible are one answer. The activity stream is the
    // most attractive read in the product for this mistake — it names actors,
    // statuses and titles, keyed by an id the caller supplies.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = caller(&db.pool, "owner@example.com", "acme", RELATER).await?;
    let (_, task) = a_task(&owner).await?;
    let outsider = caller(&db.pool, "outsider@example.com", "other", RELATER).await?;

    let (real, _, _) = outsider
        .get(&format!("/api/v1/tasks/{task}/activity"))
        .await?;
    let (imaginary, _, _) = outsider
        .get(&format!("/api/v1/tasks/{}/activity", Uuid::now_v7()))
        .await?;
    assert_eq!(real, StatusCode::NOT_FOUND);
    assert_eq!(
        real, imaginary,
        "another tenant's task is distinguishable from one that does not exist"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn history_needs_task_history_read_and_a_grant_is_the_only_source() -> Result<()> {
    // docs/25 §The three streams assigns this read to `task.history.read`, not
    // `audit.read` — gating it on the latter would hide a user's own task
    // history behind an administrator's permission.
    let db = schema_harness::TestDatabase::start().await?;
    let author = caller(&db.pool, "author@example.com", "acme", RELATER).await?;
    let (_, task) = a_task(&author).await?;

    // A colleague who can SEE the task but was never granted history.
    let colleague = member_of(
        &db.pool,
        "colleague@example.com",
        author.workspace,
        &["task.read"],
    )
    .await?;
    let (status, body, _) = colleague
        .get(&format!("/api/v1/tasks/{task}/activity"))
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0001");

    // And the holder is allowed — otherwise the test above passes with the
    // endpoint refusing everyone.
    let (status, page, _) = author
        .get(&format!("/api/v1/tasks/{task}/activity"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_dependency_is_added_and_reads_back_from_both_ends() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, first) = a_task(&caller).await?;
    let second = another_task(&caller, &project, "Second").await?;

    // "first blocks second".
    let (status, relations, _) = caller
        .post(
            &format!("/api/v1/tasks/{first}/dependencies"),
            &serde_json::json!({ "blocks": second }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{relations}");
    assert_eq!(relations["blocks"][0]["id"], second);
    assert_eq!(relations["blocks"][0]["key"], "WR-2");
    assert_eq!(relations["blocks"][0]["state"], "BACKLOG");
    assert_eq!(relations["blocks"][0]["restricted"], false);
    assert!(
        relations["blocked_by"]
            .as_array()
            .expect("array")
            .is_empty()
    );

    // The other end sees the mirror image. Getting this backwards would draw
    // every arrow the wrong way and look entirely plausible.
    let (status, theirs, _) = caller
        .get(&format!("/api/v1/tasks/{second}/dependencies"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{theirs}");
    assert_eq!(theirs["blocked_by"][0]["id"], first);
    assert!(theirs["blocks"].as_array().expect("array").is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_dependency_that_would_close_a_loop_is_refused() -> Result<()> {
    // ADR-019, and the part that must be impossible to get wrong. A cycle makes
    // "what is blocking this?" non-terminating, and the transition gate, the
    // board and My Work all walk that graph.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;
    let c = another_task(&caller, &project, "C").await?;

    // A blocks B blocks C.
    for (blocker, blocked) in [(&a, &b), (&b, &c)] {
        let (status, body, _) = caller
            .post(
                &format!("/api/v1/tasks/{blocker}/dependencies"),
                &serde_json::json!({ "blocks": blocked }),
                None,
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    // C blocking A would close the loop, two hops away from the edge itself.
    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{c}/dependencies"),
            &serde_json::json!({ "blocks": a }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-TSK-0003");
    // docs/03 / §12: the refusal names the loop. "Invalid dependency" tells the
    // user nothing they can act on.
    let cycle = body["error"]["details"]["cycle"]
        .as_array()
        .expect("the refusal names the cycle");
    assert!(
        cycle.len() >= 3,
        "the path does not describe a loop: {body}"
    );
    assert_eq!(
        cycle.first(),
        cycle.last(),
        "a named cycle must start and end at the same task: {body}"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("→"),
        "the message does not render the path: {body}"
    );

    // And nothing was written: the refusal is the whole statement, not a check
    // followed by an insert.
    let (_, relations, _) = caller
        .get(&format!("/api/v1/tasks/{c}/dependencies"))
        .await?;
    assert!(
        relations["blocks"].as_array().expect("array").is_empty(),
        "a refused dependency was stored anyway: {relations}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_shortest_possible_cycle_is_refused_too() -> Result<()> {
    // The one-hop case and the zero-hop case. A reachability check that starts
    // its walk one step too late lets both of these through.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;

    // A task cannot block itself.
    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": a }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-TSK-0003");

    // A blocks B, then B blocks A.
    let (status, _, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{b}/dependencies"),
            &serde_json::json!({ "blocks": a }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-TSK-0003");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_same_dependency_twice_is_not_an_error_and_not_a_duplicate() -> Result<()> {
    // The drawer's button is idempotent, and a duplicate is not a cycle. The
    // second call must not be refused as one.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;

    let (first, _, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(first, StatusCode::CREATED);

    let (second, relations, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(second, StatusCode::OK, "a repeat was refused: {relations}");
    assert_eq!(
        relations["blocks"].as_array().expect("array").len(),
        1,
        "the edge was stored twice"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_dependency_on_a_task_in_another_workspace_is_404() -> Result<()> {
    // Absent and invisible are one answer, so a caller cannot discover task ids
    // by proposing dependencies on them.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = caller(&db.pool, "owner@example.com", "acme", RELATER).await?;
    let (_, theirs) = a_task(&owner).await?;

    let stranger = caller(&db.pool, "stranger@example.com", "other", RELATER).await?;
    let (project, mine) = a_task(&stranger).await?;
    assert!(!project.is_empty());

    let (real, body, _) = stranger
        .post(
            &format!("/api/v1/tasks/{mine}/dependencies"),
            &serde_json::json!({ "blocks": theirs }),
            None,
        )
        .await?;
    let (imaginary, _, _) = stranger
        .post(
            &format!("/api/v1/tasks/{mine}/dependencies"),
            &serde_json::json!({ "blocks": Uuid::now_v7() }),
            None,
        )
        .await?;
    assert_eq!(real, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(real, imaginary, "a foreign task id is distinguishable");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn adding_a_dependency_needs_task_update() -> Result<()> {
    // A dependency changes how a task behaves — it gates its transitions
    // (ADR-019) — so it is a task update. There is no `task.dependency.add` in
    // the closed registry.
    let db = schema_harness::TestDatabase::start().await?;
    let author = caller(&db.pool, "author@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&author).await?;
    let b = another_task(&author, &project, "B").await?;

    let reader = member_of(
        &db.pool,
        "reader@example.com",
        author.workspace,
        &["task.read"],
    )
    .await?;
    let (status, body, _) = reader
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0001");

    // Reading relations only needs to see the task.
    let (status, _, _) = reader
        .get(&format!("/api/v1/tasks/{a}/dependencies"))
        .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_request_naming_neither_direction_or_both_is_refused() -> Result<()> {
    // Picking a direction silently is how a Relations panel ends up drawing the
    // arrow backwards.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;

    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({}),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b, "blocked_by": b }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn adding_a_dependency_writes_its_history_in_the_same_transaction() -> Result<()> {
    // ADR-006, and the join between the two features in this file: the edge and
    // its activity record commit together, so the History tab shows it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", RELATER).await?;
    let (project, a) = a_task(&caller).await?;
    let b = another_task(&caller, &project, "B").await?;

    let (status, _, _) = caller
        .post(
            &format!("/api/v1/tasks/{a}/dependencies"),
            &serde_json::json!({ "blocks": b }),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, page, _) = caller.get(&format!("/api/v1/tasks/{a}/activity")).await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    let entries = page["data"].as_array().expect("data");
    assert_eq!(
        entries[0]["event_type"], "task.dependency.added",
        "the dependency left no history: {page}"
    );
    assert_eq!(entries[0]["changes"]["direction"], "blocks");
    Ok(())
}
