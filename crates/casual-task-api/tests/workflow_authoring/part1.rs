use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_status_is_added_renamed_and_appears_in_the_workflow() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&caller, "WFA").await?;

    let (status, created) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "In Review", "state": "ACTIVE" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    // The representation is the whole workflow, not the status — every
    // authoring call returns the surface a board has to re-render.
    let id = status_named(&created, "In Review")["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let (status, listed) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(status_named(&listed, "In Review")["state"], "ACTIVE");

    let (status, renamed) = caller
        .patch_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{id}"),
            &json!({ "name": "Under Review" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{renamed}");

    let (_, listed) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    assert_eq!(status_named(&listed, "Under Review")["id"], id.as_str());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_status_name_is_unique_inside_one_workflow() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&caller, "WFB").await?;

    let (status, _) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "Triage", "state": "PLANNED" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "Triage", "state": "PLANNED" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0009");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_state_outside_the_five_is_refused() -> Result<()> {
    // `docs/23`: the five permanent states are a closed enum, forever. A
    // workflow author renames and reorders statuses; they never invent a state.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&caller, "WFC").await?;

    let (status, body) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "Blocked", "state": "BLOCKED" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_ne!(
        status,
        StatusCode::CREATED,
        "BLOCKED is a status, not a state"
    );
    assert!(status.is_client_error(), "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn reordering_rewrites_the_whole_order() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&caller, "WFD").await?;

    let (_, before) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    let mut ids: Vec<String> = before["statuses"]
        .as_array()
        .expect("statuses")
        .iter()
        .map(|s| s["id"].as_str().expect("id").to_owned())
        .collect();
    assert!(ids.len() >= 2, "the default workflow has several statuses");
    ids.reverse();

    let (status, body) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses/order"),
            &json!({ "order": ids }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, after) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    let now: Vec<String> = after["statuses"]
        .as_array()
        .expect("statuses")
        .iter()
        .map(|s| s["id"].as_str().expect("id").to_owned())
        .collect();
    assert_eq!(now, ids, "the order the caller sent is the order returned");

    // Positions must stay distinct — `workflow_status` has no unique constraint
    // on `(workflow_id, position)`, and two statuses sharing one makes a board's
    // column order depend on which row the planner returns first.
    let mut positions: Vec<i64> = after["statuses"]
        .as_array()
        .expect("statuses")
        .iter()
        .map(|s| s["position"].as_i64().expect("position"))
        .collect();
    let total = positions.len();
    positions.sort_unstable();
    positions.dedup();
    assert_eq!(positions.len(), total, "two statuses share a position");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_status_holding_tasks_cannot_be_deleted_without_a_target() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, workflow) = a_project(&caller, "WFE").await?;
    let task = a_task(&caller, &project, "Something in the initial status").await?;
    let (on, _) = status_of(&caller, &task).await?;
    demote_initial(&caller, &workflow, &on).await?;

    let (status, body) = caller
        .delete_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{on}"),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0006");

    // And the task is untouched — a refused delete moves nothing.
    let (still_on, _) = status_of(&caller, &task).await?;
    assert_eq!(still_on, on);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn deleting_with_a_target_moves_every_task_and_says_how_many() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, workflow) = a_project(&caller, "WFF").await?;

    let one = a_task(&caller, &project, "First").await?;
    let two = a_task(&caller, &project, "Second").await?;
    let (from, _) = status_of(&caller, &one).await?;

    // Somewhere for them to go.
    let (_, target) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "Parked", "state": "BACKLOG" }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    let to = status_named(&target, "Parked")["id"]
        .as_str()
        .expect("id")
        .to_owned();

    // The initial status cannot be the one deleted — a workflow must keep
    // exactly one — so promote the target first.
    let (status, body) = caller
        .patch_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{to}"),
            &json!({ "is_initial": true }),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = caller
        .delete_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{from}?migrate_to={to}"),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["migrated_tasks"], 2, "{body}");

    for task in [&one, &two] {
        let (now, _) = status_of(&caller, task).await?;
        assert_eq!(now, to, "task {task} did not move");
    }

    // `docs/23`: each move writes an activity event attributed to the acting
    // admin. Lazily remapping on next read would satisfy the assertions above
    // and produce a task whose history does not explain its status.
    let (status, history) = caller.get(&format!("/api/v1/tasks/{one}/activity")).await?;
    assert_eq!(status, StatusCode::OK, "{history}");
    let events = history["data"].as_array().expect("data");
    assert!(
        events.iter().any(|e| {
            e["event_type"]
                .as_str()
                .is_some_and(|t| t.contains("status"))
                || e["changes"].to_string().contains("workflow_migration")
        }),
        "no activity event explains the move: {history}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_migration_target_from_another_workflow_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, workflow) = a_project(&caller, "WFG").await?;
    let (_, elsewhere) = a_project(&caller, "WFH").await?;
    let task = a_task(&caller, &project, "Held").await?;
    let (from, _) = status_of(&caller, &task).await?;
    demote_initial(&caller, &workflow, &from).await?;

    let (_, other) = caller
        .get(&format!("/api/v1/workflows/{elsewhere}"))
        .await?;
    let foreign = other["statuses"].as_array().expect("statuses")[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let (status, body) = caller
        .delete_at(
            &format!("/api/v1/workflows/{workflow}/statuses/{from}?migrate_to={foreign}"),
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-WFL-0008");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn authoring_needs_project_workflow_manage() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&admin, "WFI").await?;

    // Everything except the authoring permission.
    let member = member_of(
        &db.pool,
        "member@example.com",
        admin.workspace,
        &["project.create", "task.create", "task.read", "task.update"],
    )
    .await?;

    let version = version_of(&admin, &workflow).await?;
    let (status, body) = member
        .post_at(
            &format!("/api/v1/workflows/{workflow}/statuses"),
            &json!({ "name": "Sneaky", "state": "PLANNED" }),
            version,
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_transition_is_added_and_the_same_edge_twice_is_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (_, workflow) = a_project(&caller, "WFJ").await?;

    let (_, view) = caller.get(&format!("/api/v1/workflows/{workflow}")).await?;
    let statuses = view["statuses"].as_array().expect("statuses");
    let from = statuses[0]["id"].as_str().expect("id").to_owned();
    let to = statuses[statuses.len() - 1]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let body = json!({ "from": from, "to": to, "required_fields": [] });
    let (status, created) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/transitions"),
            &body,
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let (status, again) = caller
        .post_at(
            &format!("/api/v1/workflows/{workflow}/transitions"),
            &body,
            version_of(&caller, &workflow).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{again}");
    assert_eq!(again["error"]["code"], "TF-WFL-0010");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_environment_is_created_listed_renamed_and_set_on_a_task() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, _) = a_project(&caller, "ENA").await?;
    let task = a_task(&caller, &project, "Fails in staging").await?;

    let (status, created) = caller
        .post(
            &format!("/api/v1/projects/{project}/environments"),
            &json!({ "name": "Staging" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().expect("id").to_owned();

    let (status, listed) = caller
        .get(&format!("/api/v1/projects/{project}/environments"))
        .await?;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        listed["data"]
            .as_array()
            .expect("data")
            .iter()
            .any(|e| e["id"] == id.as_str())
    );

    let (status, renamed) = caller
        .patch(
            &format!("/api/v1/environments/{id}"),
            &json!({ "name": "Stage" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Stage");

    let (status, set) = caller
        .put_at(
            &format!("/api/v1/tasks/{task}/environment"),
            &json!({ "environment_id": id }),
            task_version(&caller, &task).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{set}");

    let (status, read) = caller.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["environment_id"], id.as_str());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_environment_name_is_unique_inside_one_project() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, _) = a_project(&caller, "ENB").await?;

    let make = json!({ "name": "QA" });
    let (status, _) = caller
        .post(&format!("/api/v1/projects/{project}/environments"), &make)
        .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = caller
        .post(&format!("/api/v1/projects/{project}/environments"), &make)
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "TF-PRJ-0009");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_environment_holding_tasks_needs_a_migration_target() -> Result<()> {
    // The same rule as a status, for the same reason: a task pointing at a row
    // that no longer exists is a task whose history does not explain it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = caller(&db.pool, "admin@example.com", "acme", AUTHOR).await?;
    let (project, _) = a_project(&caller, "ENC").await?;
    let task = a_task(&caller, &project, "Reproduces in QA").await?;

    let (_, qa) = caller
        .post(
            &format!("/api/v1/projects/{project}/environments"),
            &json!({ "name": "QA" }),
        )
        .await?;
    let qa_id = qa["id"].as_str().expect("id").to_owned();
    let (status, set) = caller
        .put_at(
            &format!("/api/v1/tasks/{task}/environment"),
            &json!({ "environment_id": qa_id }),
            task_version(&caller, &task).await?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{set}");

    let (status, body) = caller
        .delete(&format!("/api/v1/environments/{qa_id}"))
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-PRJ-0005");

    // The task still points at it.
    let (_, read) = caller.get(&format!("/api/v1/tasks/{task}")).await?;
    assert_eq!(read["environment_id"], qa_id.as_str());
    Ok(())
}
