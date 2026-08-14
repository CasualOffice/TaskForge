use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_report_groups_by_the_dimension_asked_for_and_keeps_the_null_slice() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let caller = signed_in(&db.pool, "lead@example.test", workspace, MEMBER).await?;

    let (status, project) = caller
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;

    for kind in ["BUG", "BUG", "FEATURE"] {
        let (status, made) = caller
            .send_json(
                "POST",
                &format!("/api/v1/projects/{project_id}/tasks"),
                &json!({ "title": "Something", "type": kind }),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{made}");
    }

    let (status, report) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(total_for(&report, Some("BUG")), 2, "{report}");
    assert_eq!(total_for(&report, Some("FEATURE")), 1, "{report}");
    assert_eq!(report["total"], 3, "{report}");

    // Nothing has been handed to a team yet, so every task is untriaged — and
    // that is the slice, not a gap in the data. A report that filtered out the
    // null group would hide the queue `docs/45` makes a place.
    let (status, by_team) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "team" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{by_team}");
    assert_eq!(total_for(&by_team, None), 3, "{by_team}");

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_report_counts_only_what_the_viewer_can_see() -> Result<()> {
    // `docs/38`: "aggregate numbers are not comparable between viewers. A
    // manager's '47 open' and a guest's '12 open' are both right." The failure
    // this pins is silent — a leaked row still produces a plausible number, and
    // nobody audits a total.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let owner = signed_in(&db.pool, "owner@example.test", workspace, MEMBER).await?;

    let (status, project) = owner
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": "PV", "name": "Private", "visibility": "PRIVATE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;
    for _ in 0..3 {
        owner
            .send_json(
                "POST",
                &format!("/api/v1/projects/{project_id}/tasks"),
                &json!({ "title": "Secret", "type": "BUG" }),
            )
            .await?;
    }

    let (status, mine) = owner
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(total_for(&mine, Some("BUG")), 3, "{mine}");

    // Another member of the same workspace, with the same permissions, who was
    // never added to a private project.
    let outsider = signed_in(&db.pool, "outsider@example.test", workspace, MEMBER).await?;
    let (status, theirs) = outsider
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{theirs}");
    assert_eq!(
        theirs["total"], 0,
        "a private project's tasks reached another member's report: {theirs}"
    );
    assert_eq!(theirs["scope"]["projects"], 0, "{theirs}");

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_filter_narrows_a_report_the_same_way_it_narrows_a_list() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let caller = signed_in(&db.pool, "lead@example.test", workspace, MEMBER).await?;

    let (_, project) = caller
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
        )
        .await?;
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;
    for (kind, priority) in [("BUG", "URGENT"), ("BUG", "LOW"), ("FEATURE", "URGENT")] {
        caller
            .send_json(
                "POST",
                &format!("/api/v1/projects/{project_id}/tasks"),
                &json!({ "title": "Something", "type": kind, "priority": priority }),
            )
            .await?;
    }

    let (status, urgent) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type", "filter": { "priority": "URGENT" } }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{urgent}");
    assert_eq!(urgent["total"], 2, "{urgent}");
    assert_eq!(total_for(&urgent, Some("BUG")), 1, "{urgent}");

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_measure_outside_the_set_and_an_unknown_dimension_are_both_refused() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let caller = signed_in(&db.pool, "lead@example.test", workspace, MEMBER).await?;

    // Outside the closed set — refused by name rather than answered with a
    // count somebody would quote. `cycle_time` and `time_in_state` used to be
    // here and are built now; what has to stay true is that a *name* the server
    // does not know never falls through to a default.
    let (status, refused) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "assignee", "measure": "reopen_rate" }),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    // `time_in_state` is built, and needs the state named. A request without
    // one is refused for the missing field, not answered for a state nobody
    // chose — "how long in which state" has no sensible default.
    let (status, needs_state) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "assignee", "measure": "time_in_state" }),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{needs_state}");
    assert_eq!(needs_state["error"]["code"], "TF-VAL-0003", "{needs_state}");

    // And with one, it answers.
    let (status, answered) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "assignee", "measure": "time_in_state", "state": "ACTIVE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{answered}");
    assert_eq!(answered["unit"], "seconds", "{answered}");

    // And a dimension outside the closed set never reaches the compiler.
    let (status, bad) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "t.title) FROM task; --" }),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_state_projection_is_rebuilt_from_history_and_survives_redelivery() -> Result<()> {
    // The property the whole projection rests on. Outbox delivery is
    // at-least-once, so a consumer that appended an interval per event would
    // double a task's history the first time one was redelivered — and every
    // duration measure would be quietly wrong, with nothing on screen to say
    // so. Rebuilding from the audit stream is idempotent by construction, and
    // this is the assertion that says so.
    use casual_task_model::{WorkspaceId, WorkspaceScope};
    use casual_task_persistence::{Scoped, state_interval};

    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let caller = signed_in(&db.pool, "lead@example.test", workspace, MEMBER).await?;

    let (status, project) = caller
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;

    let (status, task) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{project_id}/tasks"),
            &json!({ "title": "Login crashes on rotate" }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let task_id: Uuid = task["id"].as_str().context("task id")?.parse()?;

    let scope = WorkspaceScope::for_job(WorkspaceId::from_uuid(workspace));
    let rebuild_once = || async {
        let mut tx = db.pool.begin().await.expect("begin");
        let mut scoped = Scoped::apply(&mut tx, &scope).await.expect("scope");
        state_interval::rebuild(&mut scoped, task_id)
            .await
            .expect("rebuild");
        let rows = state_interval::for_task(&mut scoped, task_id)
            .await
            .expect("read");
        tx.commit().await.expect("commit");
        rows
    };

    let first = rebuild_once().await;
    assert!(
        !first.is_empty(),
        "a created task has been somewhere, so it has at least one interval"
    );
    // Exactly one open interval: the task is in a state right now, and only
    // one. A second open row would double-count it in every aggregate, which
    // is why the schema makes it a unique index rather than a hope.
    assert_eq!(
        first.iter().filter(|row| row.exited_at.is_none()).count(),
        1,
        "{first:?}"
    );

    // The same delivery again, twice more. Converges or the projection is
    // unusable under the delivery guarantee it actually has.
    let second = rebuild_once().await;
    let third = rebuild_once().await;
    assert_eq!(first.len(), second.len(), "redelivery changed the series");
    assert_eq!(second.len(), third.len(), "redelivery changed the series");
    assert_eq!(
        third.iter().filter(|row| row.exited_at.is_none()).count(),
        1,
        "redelivery left more than one open interval: {third:?}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn cancelled_work_never_counts_as_completed() -> Result<()> {
    // `docs/38`: "CANCELED never counts as completed. Cycle time and throughput
    // exclude it entirely. Collapsing the two is the most common metric bug in
    // trackers" — and it flatters, which is why nobody catches it: abandoned
    // work is fast, so counting it makes a team look quick.
    use casual_task_model::{WorkspaceId, WorkspaceScope};
    use casual_task_persistence::{Scoped, state_interval};

    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let caller = signed_in(&db.pool, "lead@example.test", workspace, MEMBER).await?;

    let (_, project) = caller
        .send_json(
            "POST",
            "/api/v1/projects",
            &json!({ "key": "WR", "name": "Work", "visibility": "WORKSPACE" }),
        )
        .await?;
    let project_id: Uuid = project["id"].as_str().context("project id")?.parse()?;

    let (_, made) = caller
        .send_json(
            "POST",
            &format!("/api/v1/projects/{project_id}/tasks"),
            &json!({ "title": "Abandoned" }),
        )
        .await?;
    let task_id: Uuid = made["id"].as_str().context("task id")?.parse()?;

    // Written straight into the projection: driving a task through a real
    // workflow to CANCELED takes a transition path this test is not about, and
    // the assertion is about what the *measure* does with the interval. The
    // statements live in `test_support` because `docs/19` keeps SQL in the
    // persistence crate.
    test_support::insert_state_interval(
        &db.pool,
        task_id,
        workspace,
        project_id,
        "ACTIVE",
        2,
        Some(1),
    )
    .await?;
    test_support::insert_state_interval(
        &db.pool, task_id, workspace, project_id, "CANCELED", 1, None,
    )
    .await?;

    // A day of "cycle time" is sitting in the table, and it must not be counted.
    let (status, cycle) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type", "measure": "cycle_time" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{cycle}");
    assert_eq!(
        cycle["groups"].as_array().context("groups")?.len(),
        0,
        "a cancelled task produced a cycle time: {cycle}"
    );
    assert_eq!(cycle["unit"], "seconds", "{cycle}");

    // Nor as throughput.
    let (status, shipped) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type", "measure": "throughput" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{shipped}");
    assert_eq!(
        shipped["total"], 0,
        "a cancelled task was counted as shipped: {shipped}"
    );

    // The same task completed instead is counted, so the zeros above are the
    // rule working rather than the query finding nothing.
    test_support::move_intervals(&db.pool, task_id, "CANCELED", "COMPLETED").await?;

    let (_, now_counted) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type", "measure": "cycle_time" }),
        )
        .await?;
    let seconds = now_counted["groups"][0]["total"].as_i64().unwrap_or(0);
    assert!(
        seconds > 60_000,
        "a day of cycle time should be about 86400 seconds: {now_counted}"
    );

    let (_, throughput) = caller
        .send_json(
            "POST",
            "/api/v1/reports/run",
            &json!({ "group_by": "type", "measure": "throughput" }),
        )
        .await?;
    assert_eq!(throughput["total"], 1, "{throughput}");

    // And `state_interval` still owns the series: rebuilding from history
    // replaces what this test wrote by hand.
    let scope = WorkspaceScope::for_job(WorkspaceId::from_uuid(workspace));
    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &scope).await?;
    state_interval::rebuild(&mut scoped, task_id).await?;
    tx.commit().await?;

    Ok(())
}
