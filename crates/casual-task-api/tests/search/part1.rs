use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_task_is_found_by_words_from_its_title() -> Result<()> {
    // The capability C-013 exists for: before it, GET /tasks filtered by
    // project only and a user could not find a task by typing.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let project = caller.project("WR", "WORKSPACE").await?;

    let wanted = caller
        .indexed_task(project, "Payment retry backoff", "the exponential ladder")
        .await?;
    let other = caller
        .indexed_task(project, "Rename the sidebar", "cosmetic only")
        .await?;

    let (status, body) = caller.get("/api/v1/tasks?q=payment%20retry").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        ids(&body),
        vec![wanted.to_string()],
        "search did not find the task by its title: {body}"
    );

    // The description is weight C and is searchable too.
    let (_, body) = caller.get("/api/v1/tasks?q=exponential").await?;
    assert_eq!(ids(&body), vec![wanted.to_string()]);

    // A term in neither task matches nothing — the projection is not returning
    // everything and letting the client sort it out.
    let (_, body) = caller.get("/api/v1/tasks?q=zylophage").await?;
    assert!(ids(&body).is_empty(), "{body}");

    // And the other task is findable by its own words, so the first assertion
    // was not passing because only one row was ever indexed.
    let (_, body) = caller.get("/api/v1/tasks?q=sidebar").await?;
    assert_eq!(ids(&body), vec![other.to_string()]);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn search_never_returns_a_task_from_a_project_the_actor_cannot_see() -> Result<()> {
    // docs/26 §Acceptance gates, in its own words: "including for tasks whose
    // text matches strongly". The match here is stronger than the visible one —
    // the term is in the title AND the description — so a query that ranked
    // first and filtered afterwards would put it at the top.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = fresh(&db.pool, "owner@example.test", "acme").await?;
    let open = owner.project("WR", "WORKSPACE").await?;
    let private = owner.project("SEC", "PRIVATE").await?;

    let visible = owner
        .indexed_task(open, "Payment notes", "a passing mention of retry")
        .await?;
    let hidden = owner
        .indexed_task(
            private,
            "Payment retry payment retry",
            "payment retry payment",
        )
        .await?;

    // The owner sees both: they are a member of the private project by having
    // created it, so the fixture is proven to have indexed both rows.
    let (_, body) = owner.get("/api/v1/tasks?q=payment%20retry").await?;
    let seen = ids(&body);
    assert!(seen.contains(&hidden.to_string()), "fixture: {body}");
    assert!(seen.contains(&visible.to_string()), "fixture: {body}");

    // A colleague in the same workspace cannot see the private project.
    let colleague = signed_in(&db.pool, "colleague@example.test", owner.workspace, MEMBER).await?;
    let (status, body) = colleague.get("/api/v1/tasks?q=payment%20retry").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        ids(&body),
        vec![visible.to_string()],
        "search leaked a task from an invisible project: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn results_are_ranked_and_the_cursor_resumes_on_the_rank() -> Result<()> {
    // Ranking is the half of search a boolean match test never checks, and the
    // rank cursor is the half a first-page test never checks.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let project = caller.project("WR", "WORKSPACE").await?;

    // Weight A is the title, weight C the description: a title hit must outrank
    // a description hit.
    let strong = caller
        .indexed_task(project, "Payment retry", "unrelated body")
        .await?;
    let weak = caller
        .indexed_task(project, "Unrelated title", "a note about payment retry")
        .await?;

    let (_, body) = caller.get("/api/v1/tasks?q=payment%20retry").await?;
    assert_eq!(
        ids(&body),
        vec![strong.to_string(), weak.to_string()],
        "a description match outranked a title match: {body}"
    );

    // Page one of one, resumed by the rank cursor.
    let (_, page) = caller
        .get("/api/v1/tasks?q=payment%20retry&limit=1")
        .await?;
    assert_eq!(ids(&page), vec![strong.to_string()]);
    assert_eq!(page["page"]["has_more"], true);
    let cursor = page["page"]["next_cursor"]
        .as_str()
        .context("no cursor on a page with more")?
        .to_owned();

    let (status, page) = caller
        .get(&format!(
            "/api/v1/tasks?q=payment%20retry&limit=1&cursor={cursor}"
        ))
        .await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "the rank cursor did not resume: {page}"
    );
    assert_eq!(
        ids(&page),
        vec![weak.to_string()],
        "the second page did not continue from the rank: {page}"
    );
    assert_eq!(page["page"]["has_more"], false);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_soft_deleted_task_leaves_the_projection() -> Result<()> {
    // Search is eventually consistent, but "eventually" must not mean "never":
    // a deleted task that stayed indexed would keep appearing in results.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let project = caller.project("WR", "WORKSPACE").await?;
    let task = caller
        .indexed_task(project, "Payment retry backoff", "body")
        .await?;

    let (_, body) = caller.get("/api/v1/tasks?q=payment").await?;
    assert_eq!(ids(&body), vec![task.to_string()]);

    test_support::soft_delete_task(&db.pool, task).await?;
    // Re-running the projection is what the worker does on `task.deleted`.
    assert!(
        !test_support::index_task(&db.pool, caller.workspace, task).await?,
        "a deleted task still qualified for the projection"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_filter_grammar_reaches_beyond_project_id() -> Result<()> {
    // C-013's other half: status, assignee, priority, dates, tags — the closed
    // field set docs/26 declares.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = signed_in(
        &db.pool,
        "dev@example.test",
        {
            let workspace = Uuid::now_v7();
            test_support::insert_workspace(&db.pool, workspace, "acme").await?;
            workspace
        },
        &[MEMBER, &["task.assign"]].concat(),
    )
    .await?;
    let project = caller.project("WR", "WORKSPACE").await?;

    let urgent = caller
        .post(
            &format!("/api/v1/projects/{project}/tasks"),
            &serde_json::json!({ "title": "Urgent one", "priority": "URGENT" }),
        )
        .await?["id"]
        .as_str()
        .context("id")?
        .parse::<Uuid>()?;
    let quiet = caller
        .post(
            &format!("/api/v1/projects/{project}/tasks"),
            &serde_json::json!({ "title": "Quiet one", "priority": "LOW", "type": "BUG" }),
        )
        .await?["id"]
        .as_str()
        .context("id")?
        .parse::<Uuid>()?;

    // priority=>=HIGH — the ordered-enum comparison, against the enum's
    // declared order rather than alphabetical.
    let (status, body) = caller.get("/api/v1/tasks?priority=%3E%3DHIGH").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ids(&body), vec![urgent.to_string()], "{body}");

    // type=BUG
    let (_, body) = caller.get("/api/v1/tasks?type=BUG").await?;
    assert_eq!(ids(&body), vec![quiet.to_string()], "{body}");

    // state=BACKLOG matches both; the point is that it compiles and runs.
    let (status, body) = caller.get("/api/v1/tasks?state=BACKLOG").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ids(&body).len(), 2, "{body}");

    // assignee=@me — the symbol resolves to the caller before compilation.
    let (status, body) = caller.get("/api/v1/tasks?assignee=@me").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(ids(&body).is_empty(), "nobody is assigned yet: {body}");

    caller
        .post(
            &format!("/api/v1/tasks/{urgent}/assignees"),
            &serde_json::json!({ "user_id": caller.user }),
        )
        .await?;
    let (_, body) = caller.get("/api/v1/tasks?assignee=@me").await?;
    assert_eq!(ids(&body), vec![urgent.to_string()], "{body}");

    // assignee= (empty) is is_empty — the unassigned bucket.
    let (_, body) = caller.get("/api/v1/tasks?assignee=").await?;
    assert_eq!(ids(&body), vec![quiet.to_string()], "{body}");

    // Two clauses at once, which is what a saved view looks like.
    let (_, body) = caller
        .get("/api/v1/tasks?assignee=@me&priority=URGENT")
        .await?;
    assert_eq!(ids(&body), vec![urgent.to_string()], "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_documented_limits_are_refused_with_their_own_codes() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;

    for (uri, code, why) in [
        (
            "/api/v1/tasks?colour=red".to_owned(),
            "TF-QRY-0001",
            "an unlisted field must be a 400, not a slow query (docs/26)",
        ),
        (
            "/api/v1/tasks?title=%3Eabc".to_owned(),
            "TF-QRY-0003",
            "an operator the field forbids",
        ),
        (
            "/api/v1/tasks?sort=colour".to_owned(),
            "TF-QRY-0002",
            "an unsortable field",
        ),
        (
            "/api/v1/tasks?sort=rank".to_owned(),
            "TF-QRY-0002",
            "sort=rank without q has nothing to rank",
        ),
        (
            format!("/api/v1/tasks?q={}", "x".repeat(257)),
            "TF-QRY-0008",
            "docs/26 bounds a search term at 256 characters",
        ),
        (
            "/api/v1/tasks?limit=101".to_owned(),
            "TF-QRY-0007",
            "docs/26 caps a page at 100",
        ),
    ] {
        let (status, body) = caller.get(&uri).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {body}");
        assert_eq!(body["error"]["code"], code, "{why}: {body}");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn project_id_still_works_and_so_does_its_grammar_spelling() -> Result<()> {
    // `project_id` shipped in C-006 and a name is a contract; `project` is what
    // the grammar calls it. Both must reach the same clause.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let one = caller.project("WR", "WORKSPACE").await?;
    let two = caller.project("OPS", "WORKSPACE").await?;
    let here = caller.indexed_task(one, "Here", "body").await?;
    caller.indexed_task(two, "There", "body").await?;

    for uri in [
        format!("/api/v1/tasks?project_id={one}"),
        format!("/api/v1/tasks?project={one}"),
    ] {
        let (status, body) = caller.get(&uri).await?;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(ids(&body), vec![here.to_string()], "{uri}: {body}");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_word_finds_its_task_before_it_has_been_finished() -> Result<()> {
    // The failure this prevents, and the one people actually hit: every
    // keystroke before the last returning nothing. `plainto_tsquery` alone
    // matches whole lexemes, so `backu` found no task and `backup` found one —
    // a search box that answers only completed words.
    //
    // D-069: served by a `:*` on the final token rather than by the trigram
    // index `docs/26` names, because that index's plan shape is unmeasured
    // under D-043 and this one leaves the plan alone. Typo tolerance
    // (`bakcup`) is deliberately still absent and stays with D-069.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let project = caller.project("OPS", "WORKSPACE").await?;
    let task = caller
        .indexed_task(project, "Backup restore drill", "a note")
        .await?;

    for typing in ["b", "bac", "backu", "backup"] {
        let (status, body) = caller.get(&format!("/api/v1/tasks?q={typing}")).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            ids(&body),
            vec![task.to_string()],
            "typing {typing:?} did not reach the task: {body}"
        );
    }

    // Only the LAST token is a prefix; the earlier ones are finished words. So
    // a second term still narrows rather than widening.
    let (_, body) = caller.get("/api/v1/tasks?q=restore%20backu").await?;
    assert_eq!(ids(&body), vec![task.to_string()], "{body}");

    // A word that is not a prefix of anything still matches nothing — the
    // point is to find sooner, not to find everything.
    let (_, body) = caller.get("/api/v1/tasks?q=zylophage").await?;
    assert!(ids(&body).is_empty(), "{body}");

    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn tsquery_syntax_in_the_typing_is_text_not_operators() -> Result<()> {
    // `to_tsquery` parses its argument as tsquery *syntax*, so `&`, `|`, `!`,
    // the parens and `:` are operators unless they never arrive. Somebody
    // typing "a & b" or "!" must get an answer, not a 500 — and the search box
    // is exactly where punctuation gets typed by accident.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let project = caller.project("OPS", "WORKSPACE").await?;
    caller
        .indexed_task(project, "Backup restore drill", "a note")
        .await?;

    // The property is that punctuation is *survivable*, not that it is
    // accepted. Two layers already refuse some of it: `parse_url` rejects what
    // the URL grammar does not allow with a documented `TF-QRY-0003`, which is
    // a fine answer. What must never happen is a 500 — a tsquery syntax error
    // reaching the database is a crash caused by somebody typing a bracket.
    for hostile in [
        "%21%21%21",
        "%26",
        "a%20%26%20b",
        "%28%29",
        "%3A%2A",
        "-",
        "%7C%7C",
        "a%3Ab",
        "%27",
    ] {
        let (status, body) = caller.get(&format!("/api/v1/tasks?q={hostile}")).await?;
        assert!(
            !status.is_server_error(),
            "q={hostile} reached the database as syntax: {status} {body}"
        );
    }
    Ok(())
}
