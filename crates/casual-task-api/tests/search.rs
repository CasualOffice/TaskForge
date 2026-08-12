//! Full-text search and the filter grammar, end to end (C-013).
//!
//! The test that matters most here is the permission one. `docs/26`
//! §Acceptance gates asks for it in exactly these words — "search never returns
//! a task from an inaccessible project, **including for tasks whose text
//! matches strongly**" — because the classic failure is to search first and
//! filter afterwards, which collapses page sizes, breaks cursors, and leaks the
//! existence of matching work.
//!
//! The projection is populated through `test_support::index_task` rather than
//! by running a dispatch loop: the subject of these tests is the query path.
//! The consumer that keeps the projection current has its own test in
//! `casual-task-worker`.

mod schema_harness;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::auth::SESSION_COOKIE;
use casual_task_api::middleware::WORKSPACE_HEADER;
use casual_task_api::server::{AppState, router};
use casual_task_identity::password;
use casual_task_observability::recorder::Recorder;
use casual_task_persistence::test_support;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "a sufficiently long password";
const SECRET: &str = "a-test-secret-key-long-enough-for-hmac";

const MEMBER: &[&str] = &["project.create", "task.create", "task.read", "task.update"];

struct Caller {
    app: Router,
    cookie: String,
    csrf: String,
    workspace: Uuid,
    user: Uuid,
    pool: sqlx::PgPool,
}

impl Caller {
    async fn get(&self, uri: &str) -> Result<(StatusCode, serde_json::Value)> {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::COOKIE, &self.cookie)
                    .header(WORKSPACE_HEADER, self.workspace.to_string())
                    .body(Body::empty())?,
            )
            .await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        Ok((status, body))
    }

    async fn post(&self, uri: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::COOKIE, &self.cookie)
                    .header("x-csrf-token", &self.csrf)
                    .header(WORKSPACE_HEADER, self.workspace.to_string())
                    .header("idempotency-key", Uuid::now_v7().to_string())
                    .body(Body::from(body.to_string()))?,
            )
            .await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        anyhow::ensure!(status == StatusCode::CREATED, "create failed: {value}");
        Ok(value)
    }

    /// Create a task and put it in the search projection, as the worker would.
    async fn indexed_task(&self, project: Uuid, title: &str, description: &str) -> Result<Uuid> {
        let body = self
            .post(
                &format!("/api/v1/projects/{project}/tasks"),
                &serde_json::json!({ "title": title, "description": description }),
            )
            .await?;
        let id: Uuid = body["id"].as_str().context("task id")?.parse()?;
        anyhow::ensure!(
            test_support::index_task(&self.pool, self.workspace, id).await?,
            "the task was not indexed"
        );
        Ok(id)
    }

    async fn project(&self, key: &str, visibility: &str) -> Result<Uuid> {
        let body = self
            .post(
                "/api/v1/projects",
                &serde_json::json!({ "key": key, "name": key, "visibility": visibility }),
            )
            .await?;
        Ok(body["id"].as_str().context("project id")?.parse()?)
    }
}

fn state(pool: sqlx::PgPool) -> AppState {
    AppState {
        storage: std::sync::Arc::new(casual_task_infra::FilesystemStore::new(
            std::env::temp_dir().join("tf-test-objects"),
            "https://files.example.test".to_owned(),
            "test-object-signing-secret".to_owned(),
        )),
        pool,
        metrics: Arc::new(Recorder::new()),
        secret_key: SECRET.into(),
        broadcast: casual_task_api::sse::local_hub(),
        public_url: "https://tasks.example.test".into(),
        mailer: Arc::new(casual_task_infra::mail::LoggingMailer),
    }
}

async fn signed_in(
    pool: &sqlx::PgPool,
    email: &str,
    workspace: Uuid,
    permissions: &[&str],
) -> Result<Caller> {
    let user = Uuid::now_v7();
    test_support::insert_user_with_password(
        pool,
        user,
        email,
        &password::hash_chosen(PASSWORD).expect("hashes"),
    )
    .await?;
    test_support::add_workspace_member(pool, workspace, user).await?;
    if !permissions.is_empty() {
        test_support::grant_at_workspace(pool, workspace, user, permissions).await?;
    }

    let app = router(state(pool.clone()));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "email": email, "password": PASSWORD }).to_string(),
                ))?,
        )
        .await?;
    anyhow::ensure!(response.status() == StatusCode::OK, "login failed");
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with(SESSION_COOKIE))
        .and_then(|c| c.split(';').next())
        .context("session cookie")?
        .to_owned();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    let csrf = body["csrf_token"]
        .as_str()
        .context("csrf token")?
        .to_owned();

    Ok(Caller {
        app,
        cookie,
        csrf,
        workspace,
        user,
        pool: pool.clone(),
    })
}

async fn fresh(pool: &sqlx::PgPool, email: &str, slug: &str) -> Result<Caller> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, slug).await?;
    signed_in(pool, email, workspace, MEMBER).await
}

/// The task ids a response returned, in order.
fn ids(body: &serde_json::Value) -> Vec<String> {
    body["data"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["id"].as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------

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
