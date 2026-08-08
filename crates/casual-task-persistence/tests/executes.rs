//! Does the compiled SQL actually run?
//!
//! Every test up to now asserted the compiler's *output*. That proves the
//! shape and nothing about whether PostgreSQL accepts it — and the interesting
//! failures live there: a text parameter compared against an enum column, a
//! cast that defeats an index, a row-value comparison against mismatched types.
//!
//! These run the real thing: filter AST → validate → resolve → compile →
//! execute, as `taskforge_app` with row-level security applied.

mod schema_harness;

use anyhow::Result;
use casual_task_model::{ProjectId, TeamId, UserId, WorkspaceId, WorkspaceScope};
use casual_task_persistence::{AuthorizedProjectSet, Page, Param, Scoped, compile};
use casual_task_search::filter::{Clause, Field, Node, Operator, Value};
use casual_task_search::{Context, resolve};
use sqlx::{PgPool, Row};
use time::{OffsetDateTime, UtcOffset};

struct Fixture {
    workspace: WorkspaceId,
    project: ProjectId,
    actor: UserId,
}

/// One workspace, one project, three tasks in different states.
async fn seed(pool: &PgPool) -> Result<Fixture> {
    let f = Fixture {
        workspace: WorkspaceId::new(),
        project: ProjectId::new(),
        actor: UserId::new(),
    };
    sqlx::query("INSERT INTO workspace (id, name, slug) VALUES ($1,'W','w')")
        .bind(f.workspace.as_uuid())
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO user_account (id, email, display_name) VALUES ($1,'a@example.com','A')",
    )
    .bind(f.actor.as_uuid())
    .execute(pool)
    .await?;

    let workflow = uuid::Uuid::now_v7();
    let status = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO workflow (id, workspace_id, name) VALUES ($1,$2,'default')")
        .bind(workflow)
        .bind(f.workspace.as_uuid())
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO workflow_status (id, workspace_id, workflow_id, name, state, position, is_initial)
         VALUES ($1,$2,$3,'Todo','PLANNED',1,true)",
    )
    .bind(status)
    .bind(f.workspace.as_uuid())
    .bind(workflow)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO project (id, workspace_id, key, name, workflow_id, created_by)
         VALUES ($1,$2,'WR','Work',$3,$4)",
    )
    .bind(f.project.as_uuid())
    .bind(f.workspace.as_uuid())
    .bind(workflow)
    .bind(f.actor.as_uuid())
    .execute(pool)
    .await?;

    for (n, state, title) in [
        (1i64, "ACTIVE", "first"),
        (2, "PLANNED", "second"),
        (3, "COMPLETED", "third"),
    ] {
        let task = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO task (id, workspace_id, project_id, number, title, status_id, state,
                               reporter_id, position, created_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7::task_state,$8,'a',$8)",
        )
        .bind(task)
        .bind(f.workspace.as_uuid())
        .bind(f.project.as_uuid())
        .bind(n)
        .bind(title)
        .bind(status)
        .bind(state)
        .bind(f.actor.as_uuid())
        .execute(pool)
        .await?;
    }
    Ok(f)
}

async fn app_pool(db: &schema_harness::TestDatabase) -> Result<PgPool> {
    sqlx::query("ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw'")
        .execute(&db.pool)
        .await?;
    Ok(PgPool::connect(&db.app_url()).await?)
}

/// Run a compiled query, binding every parameter in order.
async fn run(
    scoped: &mut Scoped<'_>,
    c: &casual_task_persistence::Compiled,
) -> Result<Vec<String>> {
    let mut q = sqlx::query(&c.sql);
    for p in &c.params {
        q = match p {
            Param::Workspace(w) => q.bind(w.as_uuid()),
            Param::Projects(ps) => q.bind(ps.iter().map(|p| p.as_uuid()).collect::<Vec<_>>()),
            Param::Text(t) => q.bind(t.clone()),
            Param::TextList(v) => q.bind(v.clone()),
        };
    }
    let rows = q.fetch_all(scoped.conn()).await?;
    Ok(rows.iter().map(|r| r.get::<String, _>("title")).collect())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_compiled_filter_executes_and_returns_the_right_rows() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let f = seed(&db.pool).await?;
    let app = app_pool(&db).await?;

    // `state in (ACTIVE)` — an enum column against a bound text parameter, which
    // is exactly the comparison that fails without a parameter cast.
    let filter = Node::Clause(Clause {
        field: Field::State,
        op: Operator::In,
        value: Value::List(vec!["ACTIVE".into()]),
    });
    casual_task_search::filter::validate(&filter).expect("valid");

    let compiled = compile(
        &filter,
        f.workspace,
        &AuthorizedProjectSet::resolved(vec![f.project]),
        &Page::default(),
    );

    let mut tx = app.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(f.workspace)).await?;
    let titles = run(&mut scoped, &compiled).await?;
    tx.rollback().await?;

    assert_eq!(titles, vec!["first".to_owned()], "only the ACTIVE task");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_permission_filter_is_enforced_by_the_database_too() -> Result<()> {
    // The compiler injects the project predicate; RLS enforces the workspace.
    // An empty authorized set must return nothing even though the rows exist
    // and the transaction is correctly scoped.
    let db = schema_harness::TestDatabase::start().await?;
    let f = seed(&db.pool).await?;
    let app = app_pool(&db).await?;

    let compiled = compile(
        &Node::And(Vec::new()),
        f.workspace,
        &AuthorizedProjectSet::resolved(Vec::new()),
        &Page::default(),
    );

    let mut tx = app.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(f.workspace)).await?;
    let titles = run(&mut scoped, &compiled).await?;
    tx.rollback().await?;

    assert!(
        titles.is_empty(),
        "an actor authorized for no project must see nothing, got {titles:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_resolved_symbol_executes_against_a_real_column() -> Result<()> {
    // `@me` resolves to a uuid string, which must survive as a uuid parameter
    // against `reporter_id`. A cast mistake here is invisible until it runs.
    let db = schema_harness::TestDatabase::start().await?;
    let f = seed(&db.pool).await?;
    let app = app_pool(&db).await?;

    let filter = Node::Clause(Clause {
        field: Field::Reporter,
        op: Operator::Eq,
        value: Value::Symbol("@me".into()),
    });
    let ctx = Context::new(
        f.actor,
        Vec::<TeamId>::new(),
        OffsetDateTime::now_utc(),
        UtcOffset::UTC,
    );
    let resolved = resolve(&filter, &ctx).expect("resolves");

    let compiled = compile(
        &resolved,
        f.workspace,
        &AuthorizedProjectSet::resolved(vec![f.project]),
        &Page::default(),
    );

    let mut tx = app.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(f.workspace)).await?;
    let titles = run(&mut scoped, &compiled).await?;
    tx.rollback().await?;

    assert_eq!(titles.len(), 3, "the actor reported all three tasks");
    Ok(())
}
