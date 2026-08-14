use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_project_and_a_task_in_it_can_be_created_and_read_back() -> Result<()> {
    // The whole point of C-006 and C-008: before this, the product could log in
    // and nothing else.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let (status, project, etag) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["key"], "WR");
    assert_eq!(project["version"], 1);
    assert_eq!(etag.as_deref(), Some("\"1\""), "a create returns its ETag");
    let project_id = project["id"].as_str().expect("id").to_owned();

    let (status, task, etag) = caller
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "Ship the thing", "priority": "HIGH" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    // The task enters the default workflow's initial status, and the state it
    // maps to is written with it (docs/23).
    assert_eq!(task["state"], "BACKLOG");
    assert_eq!(
        task["key"], "WR-1",
        "the human key spans project and number"
    );
    assert_eq!(task["number"], 1);
    assert_eq!(task["priority"], "HIGH");
    assert_eq!(etag.as_deref(), Some("\"1\""));
    let task_id = task["id"].as_str().expect("id").to_owned();

    let (status, read_back, etag) = caller
        .get(&format!("/api/v1/projects/{project_id}"))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_back["id"], project["id"]);
    assert_eq!(etag.as_deref(), Some("\"1\""), "a read returns an ETag");

    let (status, read_back, etag) = caller.get(&format!("/api/v1/tasks/{task_id}")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_back["title"], "Ship the thing");
    assert_eq!(etag.as_deref(), Some("\"1\""));

    // And both appear in their lists.
    let (status, page, _) = caller.get("/api/v1/projects").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["data"][0]["id"], project["id"]);
    assert_eq!(page["page"]["has_more"], false);

    let (status, page, _) = caller.get("/api/v1/tasks").await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["data"][0]["id"], task["id"]);
    assert_eq!(page["data"][0]["key"], "WR-1");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_create_writes_its_activity_audit_and_outbox_rows_in_the_same_transaction() -> Result<()>
{
    // ADR-006. Without this, a create that returned 201 and wrote no history
    // would pass every other test in this file.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    let project_id: Uuid = project["id"].as_str().expect("id").parse()?;

    let (activity, audit, outbox, deliveries) =
        test_support::history_counts(&db.pool, project_id).await?;
    assert_eq!(activity, 1, "no activity row for the project create");
    assert_eq!(audit, 1, "no audit row for the project create");
    assert_eq!(outbox, 1, "no outbox event for the project create");
    assert_eq!(
        deliveries,
        i64::try_from(casual_task_persistence::CONSUMERS.len()).expect("consumer count"),
        "one delivery row per consumer, written in the producing transaction"
    );
    assert_eq!(
        test_support::outbox_event_types(&db.pool, project_id).await?,
        vec!["project.created".to_owned()],
        "docs/25 names the event type"
    );

    let (_, task, _) = caller
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "t" }),
            Some(&key()),
        )
        .await?;
    let task_id: Uuid = task["id"].as_str().expect("id").parse()?;
    let (activity, audit, outbox, _) = test_support::history_counts(&db.pool, task_id).await?;
    assert_eq!((activity, audit, outbox), (1, 1, 1));
    assert_eq!(
        test_support::outbox_event_types(&db.pool, task_id).await?,
        vec!["task.created".to_owned()]
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_project_in_another_workspace_is_404_and_never_403() -> Result<()> {
    // docs/04: absent and invisible are never disambiguated. A 403 here would
    // confirm the project exists, which is how project ids get enumerated —
    // and the ids are in every task key the other tenant publishes.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = caller(&db.pool, "owner@example.com", "acme", MEMBER).await?;
    let stranger = caller(&db.pool, "stranger@example.com", "other", MEMBER).await?;

    let (_, project, _) = owner
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
            Some(&key()),
        )
        .await?;
    let project_id = project["id"].as_str().expect("id").to_owned();

    let (real, _, _) = stranger
        .get(&format!("/api/v1/projects/{project_id}"))
        .await?;
    let (imaginary, _, _) = stranger
        .get(&format!("/api/v1/projects/{}", Uuid::now_v7()))
        .await?;
    assert_eq!(real, StatusCode::NOT_FOUND);
    assert_eq!(
        real, imaginary,
        "a project in another workspace is distinguishable from one that does \
         not exist"
    );

    // And it is absent from the stranger's list, rather than merely unreadable.
    let (status, page, _) = stranger.get("/api/v1/projects").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["data"].as_array().map(Vec::len), Some(0));

    // The task in it is invisible for the same reason.
    let (_, task, _) = owner
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "secret" }),
            Some(&key()),
        )
        .await?;
    let task_id = task["id"].as_str().expect("id");
    let (status, _, _) = stranger.get(&format!("/api/v1/tasks/{task_id}")).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Creating into someone else's project is the same answer, not a 403.
    let (status, _, _) = stranger
        .post(
            &format!("/api/v1/projects/{project_id}/tasks"),
            &serde_json::json!({ "title": "intruder" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_private_project_is_invisible_to_a_fellow_member() -> Result<()> {
    // The other half of docs/04's visibility rule, inside one workspace. A
    // workspace-scoped grant does not confer visibility of a private project —
    // that is how "Member everywhere except this one project" is expressed.
    let db = schema_harness::TestDatabase::start().await?;
    let author = caller(&db.pool, "author@example.com", "acme", MEMBER).await?;
    // A second member of the SAME workspace, holding every permission these
    // endpoints use.
    let colleague = member_of(&db.pool, "colleague@example.com", author.workspace, MEMBER).await?;

    let (_, private, _) = author
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "SEC", "name": "Secret", "visibility": "PRIVATE" }),
            Some(&key()),
        )
        .await?;
    let private_id = private["id"].as_str().expect("id").to_owned();

    // The author can see it: creating something you cannot read back is a bug.
    let (status, _, _) = author
        .get(&format!("/api/v1/projects/{private_id}"))
        .await?;
    assert_eq!(status, StatusCode::OK);

    // The colleague holds every permission and still cannot see it.
    let (status, _, _) = colleague
        .get(&format!("/api/v1/projects/{private_id}"))
        .await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a PRIVATE project was visible to a workspace member who is not in it"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_patch_without_if_match_is_428_and_a_stale_one_is_409() -> Result<()> {
    // docs/05: 428 rather than silently accepting an unconditional write, and
    // 409 rather than the silent overwrite ADR-023 exists to prevent. Both are
    // easy to lose, and losing either is invisible until someone's edit
    // vanishes in production.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    let uri = format!("/api/v1/projects/{}", project["id"].as_str().expect("id"));

    let (status, body, _) = caller
        .patch(&uri, &serde_json::json!({ "name": "Renamed" }), None)
        .await?;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["error"]["code"], "TF-CNC-0002");

    // The current version is 1, so 7 is stale.
    let (status, body, _) = caller
        .patch(
            &uri,
            &serde_json::json!({ "name": "Renamed" }),
            Some("\"7\""),
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "TF-CNC-0001");
    assert_eq!(body["error"]["details"]["your_version"], 7);
    assert_eq!(body["error"]["details"]["current_version"], 1);
    assert_eq!(
        body["error"]["details"]["current"]["name"], "Work",
        "docs/24: the conflict body carries the current representation so the \
         client can show what changed"
    );

    // The refused writes changed nothing.
    let (_, unchanged, _) = caller.get(&uri).await?;
    assert_eq!(unchanged["name"], "Work");
    assert_eq!(unchanged["version"], 1);

    // And the correct tag succeeds, bumping the version.
    let (status, updated, etag) = caller
        .patch(
            &uri,
            &serde_json::json!({ "name": "Renamed" }),
            Some("\"1\""),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "Renamed");
    assert_eq!(updated["version"], 2);
    assert_eq!(etag.as_deref(), Some("\"2\""));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_patch_cannot_change_the_key_and_can_clear_a_description() -> Result<()> {
    // ADR-007 makes the key immutable, and docs/05 §Conventions makes `null`
    // mean "clear" while absent means "leave alone". Both are one-line rules
    // that a PATCH implementation gets wrong by default.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work", "description": "notes" }),
            Some(&key()),
        )
        .await?;
    let uri = format!("/api/v1/projects/{}", project["id"].as_str().expect("id"));

    let (status, body, _) = caller
        .patch(&uri, &serde_json::json!({ "key": "OPS" }), Some("\"1\""))
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-PRJ-0003");

    // An empty patch leaves the description alone.
    let (status, unchanged, _) = caller
        .patch(&uri, &serde_json::json!({}), Some("\"1\""))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unchanged["description"], "notes");

    // An explicit null clears it.
    let (status, cleared, _) = caller
        .patch(
            &uri,
            &serde_json::json!({ "description": null }),
            Some("\"2\""),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert!(cleared["description"].is_null());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn authority_comes_from_a_grant_and_nowhere_else() -> Result<()> {
    // migration 0003: "role_assignment is the ONLY source of authority in the
    // system. No permission is granted anywhere else — not by a boolean column,
    // not by an is_admin flag, and not by project membership."
    let db = schema_harness::TestDatabase::start().await?;
    let ungranted = caller(&db.pool, "member@example.com", "acme", &[]).await?;

    let (status, body, _) = ungranted
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0001");

    // Reading is not blocked by the same rule: docs/04 gives visibility an
    // implicit read grant, so a member with no grants still sees the workspace.
    let (status, page, _) = ungranted.get("/api/v1/projects").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["data"].as_array().map(Vec::len), Some(0));

    // A grant that carries project.create but not task.create authorizes one
    // and refuses the other — the resolver is consulted per permission, not
    // per endpoint family.
    let partial = caller(&db.pool, "partial@example.com", "beta", &["project.create"]).await?;
    let (status, project, _) = partial
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let (status, body, _) = partial
        .post(
            &format!(
                "/api/v1/projects/{}/tasks",
                project["id"].as_str().expect("id")
            ),
            &serde_json::json!({ "title": "t" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0001");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_duplicate_key_is_409_and_a_malformed_one_is_400() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let (status, _, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Other" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "TF-PRJ-0002");

    for bad in ["wr", "W", "W-R", "TOOLONGAKEY1"] {
        let (status, body, _) = caller
            .post(
                "/api/v1/projects",
                &serde_json::json!({ "key": bad, "name": "x" }),
                Some(&key()),
            )
            .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} was accepted");
        assert_eq!(body["error"]["code"], "TF-PRJ-0004");
    }

    // An unknown field is a 400 and names itself, rather than being ignored.
    let (status, body, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "OPS", "name": "x", "visibilty": "TEAM" }),
            Some(&key()),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-VAL-0002");
    Ok(())
}
