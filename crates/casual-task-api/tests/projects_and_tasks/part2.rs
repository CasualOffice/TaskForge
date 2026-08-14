use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_retried_create_returns_the_first_response_rather_than_a_second_task() -> Result<()> {
    // docs/24: "a timeout that actually succeeded produces a duplicate task,
    // and the user has no way to tell". The key is what makes the retry safe,
    // and the request hash is what catches the client that reuses one key for
    // two different tasks.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;
    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    let uri = format!(
        "/api/v1/projects/{}/tasks",
        project["id"].as_str().expect("id")
    );
    let idempotency = key();
    let body = serde_json::json!({ "title": "Ship it" });

    let (status, first, _) = caller.post(&uri, &body, Some(&idempotency)).await?;
    assert_eq!(status, StatusCode::CREATED);
    let (status, replay, _) = caller.post(&uri, &body, Some(&idempotency)).await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(replay["id"], first["id"], "the retry created a second task");

    let (_, page, _) = caller.get("/api/v1/tasks").await?;
    assert_eq!(page["data"].as_array().map(Vec::len), Some(1));

    // The same key with a different body is the client bug docs/24 names.
    let (status, body, _) = caller
        .post(
            &uri,
            &serde_json::json!({ "title": "Something else" }),
            Some(&idempotency),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-IDM-0002");

    // And a create with no key at all is refused: docs/05 requires one.
    let (status, body, _) = caller
        .post(&uri, &serde_json::json!({ "title": "x" }), None)
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-IDM-0003");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_list_pages_by_cursor_and_never_repeats_or_skips_a_row() -> Result<()> {
    // docs/26 bans OFFSET because it "duplicates or skips rows under concurrent
    // writes". This asserts the keyset actually works — the second page is a
    // real query against a real database, which is where a cursor whose type
    // cast is missing fails and no unit test can see it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;
    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    let uri = format!(
        "/api/v1/projects/{}/tasks",
        project["id"].as_str().expect("id")
    );
    for n in 0..5 {
        let (status, body, _) = caller
            .post(
                &uri,
                &serde_json::json!({ "title": format!("task {n}") }),
                Some(&key()),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    let mut seen: Vec<String> = Vec::new();
    let mut next: Option<String> = None;
    for _ in 0..5 {
        let uri = next.map_or_else(
            || "/api/v1/tasks?limit=2".to_owned(),
            |c| format!("/api/v1/tasks?limit=2&cursor={c}"),
        );
        let (status, page, _) = caller.get(&uri).await?;
        assert_eq!(status, StatusCode::OK, "{page}");
        for row in page["data"].as_array().expect("data").iter() {
            seen.push(row["id"].as_str().expect("id").to_owned());
        }
        next = page["page"]["next_cursor"].as_str().map(ToOwned::to_owned);
        if next.is_none() {
            break;
        }
    }

    assert_eq!(seen.len(), 5, "paging saw {} of 5 tasks", seen.len());
    let unique: std::collections::HashSet<&String> = seen.iter().collect();
    assert_eq!(unique.len(), 5, "a row was served twice: {seen:?}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_full_task_page_resolves_authority_once() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;
    let (_, project, _) = caller
        .post(
            "/api/v1/projects",
            &serde_json::json!({ "key": "WR", "name": "Work" }),
            Some(&key()),
        )
        .await?;
    let project_id = project["id"].as_str().expect("id").parse()?;
    test_support::insert_task_page(&db.pool, caller.workspace, project_id, caller.user, 100)
        .await?;

    let before = metric_count(
        &caller.metrics.render(),
        "authz_resolution_duration_count{outcome=\"cache_miss\"}",
    );
    let (status, body, _) = caller.get("/api/v1/tasks?limit=100").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"].as_array().expect("data").len(), 100);
    let after = metric_count(
        &caller.metrics.render(),
        "authz_resolution_duration_count{outcome=\"cache_miss\"}",
    );
    assert_eq!(
        after - before,
        1,
        "one list page must perform one authorization resolution"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_page_size_and_the_query_parameters_are_bounded() -> Result<()> {
    // docs/26 §Query limits caps a page at 100. Clamping instead of refusing
    // would tell a client that asked for 500 there were only 100 rows.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    for uri in ["/api/v1/tasks?limit=101", "/api/v1/projects?limit=101"] {
        let (status, body, _) = caller.get(uri).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(body["error"]["code"], "TF-QRY-0007");
    }
    // TF-QRY-0001, not the generic TF-VAL-0002: since C-012 the list endpoint
    // reads unrecognised query parameters as filter fields, so a typo'd `limit`
    // genuinely *is* an unknown filter field, and that code's docs URL points at
    // the grammar the client needs. The property docs/05 requires is unchanged —
    // the typo is refused rather than silently ignored.
    let (status, body, _) = caller.get("/api/v1/tasks?limt=10").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-QRY-0001");
    // And it names the key that was wrong: "one of your parameters is unknown"
    // makes a client bisect its own query string.
    assert_eq!(body["error"]["details"]["field"], "limt", "{body}");

    let (status, body, _) = caller.get("/api/v1/tasks?cursor=!!!nonsense!!!").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-QRY-0006");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_tenant_route_refuses_a_request_with_no_workspace() -> Result<()> {
    // The structural rule: every one of these takes `WorkspaceMember`, which is
    // the only thing that mints an AuthContext. Without a workspace header
    // there is no membership to validate, and docs/04 makes that a 404 rather
    // than a 400 so the header cannot be probed.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;
    let id = Uuid::now_v7();

    for uri in [
        "/api/v1/projects".to_owned(),
        format!("/api/v1/projects/{id}"),
        "/api/v1/tasks".to_owned(),
        format!("/api/v1/tasks/{id}"),
    ] {
        let response = caller
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&uri)
                    .header(header::COOKIE, &caller.cookie)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }

    // And with no credential at all, 401 — before any tenant row is touched.
    let response = caller
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/projects")
                .header(WORKSPACE_HEADER, caller.workspace.to_string())
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_create_without_a_csrf_token_is_refused() -> Result<()> {
    // The new routes are registered BEFORE the layers in server.rs. If one were
    // appended after `.layer()` it would escape the CSRF guard entirely, and
    // nothing else in the suite would notice.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let response = caller
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/projects")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, &caller.cookie)
                .header(WORKSPACE_HEADER, caller.workspace.to_string())
                .header("idempotency-key", key())
                .body(Body::from(
                    serde_json::json!({ "key": "WR", "name": "Work" }).to_string(),
                ))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a state-changing request succeeded with only a session cookie"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_first_project_in_a_workspace_brings_the_default_workflow_with_it() -> Result<()> {
    // docs/23: the default workflow "works with zero configuration". Nothing
    // else creates one, so a project create in a fresh workspace either
    // provisions it or fails — and the second project must reuse it rather than
    // making another.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "member@example.com", "acme", MEMBER).await?;

    let mut workflows = Vec::new();
    for project_key in ["WR", "OPS"] {
        let (status, project, _) = caller
            .post(
                "/api/v1/projects",
                &serde_json::json!({ "key": project_key, "name": project_key }),
                Some(&key()),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{project}");
        workflows.push(project["workflow_id"].as_str().expect("id").to_owned());
    }
    assert_eq!(
        workflows[0], workflows[1],
        "the second project created a second default workflow"
    );

    let statuses = test_support::workflow_status_names(&db.pool, workflows[0].parse()?).await?;
    assert_eq!(
        statuses,
        vec![
            "Backlog".to_owned(),
            "Todo".to_owned(),
            "In Progress".to_owned(),
            "Blocked".to_owned(),
            "Done".to_owned(),
            "Canceled".to_owned(),
        ],
        "the default workflow is not the one docs/23 draws"
    );
    Ok(())
}
