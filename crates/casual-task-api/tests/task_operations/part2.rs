use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_dependency_override_requires_and_audits_one_visible_reason() -> Result<()> {
    // docs/23's dependency gate acceptance path: the permission alone is not
    // enough. The exceptional move needs an explanation, and the explanation
    // must survive in immutable audit history beside the blockers it bypassed.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "lead@example.test", "acme", OVERRIDER).await?;
    let (project, task, etag) = a_task(&caller).await?;
    let status_ids = statuses(&db.pool, caller.workspace).await?;

    let (_, blocker_body, _) = caller
        .post(
            &format!("/api/v1/projects/{project}/tasks"),
            &serde_json::json!({ "title": "Restore production first" }),
            Some(&key()),
        )
        .await?;
    let blocker: Uuid = blocker_body["id"].as_str().context("blocker id")?.parse()?;
    test_support::add_blocker(&db.pool, caller.workspace, blocker, task).await?;

    let transitions = format!("/api/v1/tasks/{task}/transitions");
    for omitted in [serde_json::Value::Null, serde_json::json!("   ")] {
        let mut request = serde_json::json!({ "to_status_id": status_ids["Todo"] });
        if !omitted.is_null() {
            request["comment"] = omitted;
        }
        let (status, body, _) = caller
            .post_conditional(&transitions, &request, Some(&etag))
            .await?;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"]["code"], "TF-WFL-0005");
        assert_eq!(body["error"]["details"]["missing_fields"][0], "comment");
        assert_eq!(
            body["error"]["details"]["blocked_by"][0],
            blocker.to_string()
        );
    }
    assert_eq!(
        test_support::history_counts(&db.pool, task).await?.0,
        1,
        "a refused override wrote activity"
    );

    let reason = "Production recovery is blocked; incident commander approved the bypass";
    let (status, body, _) = caller
        .post_conditional(
            &transitions,
            &serde_json::json!({
                "to_status_id": status_ids["Todo"],
                "comment": reason,
            }),
            Some(&etag),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(test_support::comment_count(&db.pool, task).await?, 1);

    let audit = test_support::audit_changes(&db.pool, task).await?;
    assert_eq!(audit[0]["dependency_override"]["reason"], reason);
    assert_eq!(
        audit[0]["dependency_override"]["blocked_by"][0],
        blocker.to_string()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn assigning_is_idempotent_and_unassigning_removes_it() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, _) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}/assignees");
    let body = serde_json::json!({ "user_id": caller.user });

    let (status, answer, _) = caller.post(&uri, &body, None).await?;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    assert_eq!(answer["assignees"][0], caller.user.to_string());

    // A retry of a request whose response was never seen is doing the right
    // thing; an error there makes correct behaviour look broken.
    let (status, answer, _) = caller.post(&uri, &body, None).await?;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        test_support::task_assignees(&db.pool, task).await?.len(),
        1,
        "a retry assigned the same person twice"
    );

    let (status, answer, _) = caller
        .delete(&format!("{uri}/{}", caller.user), None)
        .await?;
    assert_eq!(status, StatusCode::NO_CONTENT, "{answer}");
    assert!(
        test_support::task_assignees(&db.pool, task)
            .await?
            .is_empty()
    );

    // Unassigning someone who is not assigned is a 404, not a silent success.
    let (status, _, _) = caller
        .delete(&format!("{uri}/{}", caller.user), None)
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn work_cannot_be_assigned_to_someone_who_cannot_see_the_project() -> Result<()> {
    // TF-TSK-0005. The invariant that matters is not "has a membership row" —
    // a WORKSPACE-visible project usually has none — but "can see it at all".
    // A stranger and another tenant's user are refused for the same reason.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let outsider = signed_in(&db.pool, "outsider@example.test", "other", MEMBER).await?;
    let (_, task, _) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}/assignees");

    for (label, user) in [
        ("another tenant's member", outsider.user),
        ("nobody at all", Uuid::now_v7()),
    ] {
        let (status, body, _) = caller
            .post(&uri, &serde_json::json!({ "user_id": user }), None)
            .await?;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{label} was assignable: {body}"
        );
        assert_eq!(body["error"]["code"], "TF-TSK-0005");
    }
    assert!(
        test_support::task_assignees(&db.pool, task)
            .await?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_task_can_be_tagged_and_an_unusable_tag_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, _) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}/tags");

    let tag = test_support::insert_tag(&db.pool, caller.workspace, None, "security").await?;
    let (status, body, _) = caller
        .post(&uri, &serde_json::json!({ "tag_id": tag }), None)
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["name"], "security");

    // Idempotent, like assigning.
    let (status, _, _) = caller
        .post(&uri, &serde_json::json!({ "tag_id": tag }), None)
        .await?;
    assert_eq!(status, StatusCode::OK);

    // The activity stream holds the tag's NAME, not its id: docs/25 wants a
    // stream that still reads correctly after the tag is renamed or deleted.
    let types = test_support::outbox_event_types(&db.pool, task).await?;
    assert!(types.contains(&"task.tagged".to_owned()), "{types:?}");

    // A tag from another workspace is refused — and so is one that does not
    // exist, with the same answer.
    let elsewhere = signed_in(&db.pool, "elsewhere@example.test", "other", MEMBER).await?;
    let foreign = test_support::insert_tag(&db.pool, elsewhere.workspace, None, "security").await?;
    for id in [foreign, Uuid::now_v7()] {
        let (status, body, _) = caller
            .post(&uri, &serde_json::json!({ "tag_id": id }), None)
            .await?;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"]["code"], "TF-VAL-0007");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_new_route_sits_inside_the_csrf_guard() -> Result<()> {
    // The rule is about the ROUTER, not the handlers: a route registered after
    // `.layer()` escapes both the CSRF guard and the request id, and nothing
    // about a handler would show it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, etag) = a_task(&caller).await?;

    for (method, uri, body) in [
        ("PATCH", format!("/api/v1/tasks/{task}"), "{}"),
        ("DELETE", format!("/api/v1/tasks/{task}"), ""),
        (
            "POST",
            format!("/api/v1/tasks/{task}/transitions"),
            "{\"to_status_id\":\"00000000-0000-7000-8000-000000000001\"}",
        ),
        (
            "POST",
            format!("/api/v1/tasks/{task}/assignees"),
            "{\"user_id\":\"00000000-0000-7000-8000-000000000001\"}",
        ),
        (
            "DELETE",
            format!("/api/v1/tasks/{task}/assignees/{}", caller.user),
            "",
        ),
        (
            "POST",
            format!("/api/v1/tasks/{task}/tags"),
            "{\"tag_id\":\"00000000-0000-7000-8000-000000000001\"}",
        ),
    ] {
        // Everything a real request has, except the CSRF token.
        let request = Request::builder()
            .method(method)
            .uri(&uri)
            .header(header::COOKIE, &caller.cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_MATCH, &etag)
            .header(WORKSPACE_HEADER, caller.workspace.to_string())
            .body(Body::from(body))?;
        let response = caller.app.clone().oneshot(request).await?;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{method} {uri} accepted a state change with no CSRF token"
        );
        assert!(
            response.headers().contains_key("x-request-id"),
            "{method} {uri} is outside the observability layer too"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_assignee_set_can_be_read_back_without_writing_to_it() -> Result<()> {
    // It was write-only: `POST` returned the set and `DELETE` did not, so the
    // only way to learn who was on a task was to assign someone. A task surface
    // cannot answer "who is working on this" without this read.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(&db.pool, "dev@example.test", "acme", MEMBER).await?;
    let (_, task, _) = a_task(&caller).await?;
    let uri = format!("/api/v1/tasks/{task}/assignees");

    let (status, empty, _) = caller.get(&uri).await?;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert!(
        empty["assignees"]
            .as_array()
            .context("assignees")?
            .is_empty(),
        "{empty}"
    );

    caller
        .post(&uri, &serde_json::json!({ "user_id": caller.user }), None)
        .await?;

    let (status, one, _) = caller.get(&uri).await?;
    assert_eq!(status, StatusCode::OK, "{one}");
    assert_eq!(one["assignees"].as_array().context("assignees")?.len(), 1);
    assert_eq!(one["assignees"][0], caller.user.to_string());

    // And after unassigning, the read is what shows it — not the write's echo.
    caller
        .delete(&format!("{uri}/{}", caller.user), None)
        .await?;
    let (_, gone, _) = caller.get(&uri).await?;
    assert!(
        gone["assignees"]
            .as_array()
            .context("assignees")?
            .is_empty(),
        "{gone}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn reading_assignees_of_an_invisible_task_is_404_and_never_403() -> Result<()> {
    // `docs/04`: absent and invisible are one answer. An assignee list that
    // 403'd would confirm the task exists.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = signed_in(&db.pool, "owner@example.test", "acme", MEMBER).await?;
    let (_, task, _) = a_task(&owner).await?;
    let elsewhere = signed_in(&db.pool, "other@example.test", "other", MEMBER).await?;

    let (status, body, _) = elsewhere
        .get(&format!("/api/v1/tasks/{task}/assignees"))
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "TF-TSK-0001");
    Ok(())
}
