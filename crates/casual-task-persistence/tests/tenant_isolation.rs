//! The cross-tenant property, asserted from Rust against a real PostgreSQL
//! (C-005, and the seam C-001's repositories will be built on).
//!
//! `scripts/verify-schema.sh` already gates this from shell, as the
//! `taskforge_app` role. What it cannot do is exercise the *code path* the
//! product will use — `Scoped::apply` setting the GUC that migration 0010's
//! policy reads. A drift between the constant in that module and the name the
//! migration formats into every policy would leave both the shell gate and the
//! application "working": the gate passes because it sets the GUC itself, and
//! the application returns zero rows without erroring.
//!
//! `#[ignore]` for the same reason as the schema harness beside it: Docker.

mod schema_harness;

use anyhow::Result;
use casual_task_model::{WorkspaceId, WorkspaceScope};
use casual_task_persistence::Scoped;
use sqlx::Row;

/// Two workspaces, each with one tag. `tag` is the smallest table carrying
/// `workspace_id`, so it exercises the policy with the least setup.
async fn seed_two_tenants(pool: &sqlx::PgPool) -> Result<(WorkspaceId, WorkspaceId)> {
    let (a, b) = (WorkspaceId::new(), WorkspaceId::new());
    for (w, name) in [(a, "alpha"), (b, "beta")] {
        sqlx::query("INSERT INTO workspace (id, name, slug) VALUES ($1, $2, $3)")
            .bind(w.as_uuid())
            .bind(name)
            .bind(name)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO tag (id, workspace_id, name) VALUES ($1, $2, $3)")
            .bind(uuid::Uuid::now_v7())
            .bind(w.as_uuid())
            .bind(format!("{name}-tag"))
            .execute(pool)
            .await?;
    }
    Ok((a, b))
}

/// Count tags visible to the current session.
async fn visible_tags(conn: &mut sqlx::PgConnection) -> Result<i64> {
    Ok(sqlx::query("SELECT count(*) FROM tag")
        .fetch_one(conn)
        .await?
        .get(0))
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_scoped_transaction_sees_only_its_own_tenant() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let (a, b) = seed_two_tenants(&db.pool).await?;

    // The seeding above ran as the owner, who is NOT subject to RLS. Everything
    // below runs as taskforge_app, which is.
    sqlx::query("ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw'")
        .execute(&db.pool)
        .await?;
    let app = sqlx::PgPool::connect(&db.app_url()).await?;

    for (scope_of, expected_name) in [(a, "alpha-tag"), (b, "beta-tag")] {
        let mut tx = app.begin().await?;
        let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(scope_of)).await?;

        assert_eq!(
            visible_tags(scoped.conn()).await?,
            1,
            "a scoped transaction must see exactly its own tenant's row"
        );
        let name: String = sqlx::query("SELECT name FROM tag")
            .fetch_one(scoped.conn())
            .await?
            .get(0);
        assert_eq!(name, expected_name, "and it must be the right one");
        tx.rollback().await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unscoped_transaction_sees_nothing_rather_than_everything() -> Result<()> {
    // The direction of failure is the whole point. docs/32 and migration 0010's
    // NULLIF exist so that a missing scope yields no rows instead of every row.
    let db = schema_harness::TestDatabase::start().await?;
    seed_two_tenants(&db.pool).await?;
    sqlx::query("ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw'")
        .execute(&db.pool)
        .await?;
    let app = sqlx::PgPool::connect(&db.app_url()).await?;

    let mut tx = app.begin().await?;
    assert_eq!(
        visible_tags(&mut tx).await?,
        0,
        "an unscoped session must fail closed"
    );
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_scope_does_not_survive_the_transaction() -> Result<()> {
    // `set_config(..., true)` is transaction-local. On a pooled connection a
    // session-level setting would outlive the request and the next checkout
    // would inherit another tenant's scope — which is a cross-tenant leak that
    // only appears under load, and the reason the flag is not optional.
    let db = schema_harness::TestDatabase::start().await?;
    let (a, _) = seed_two_tenants(&db.pool).await?;
    sqlx::query("ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw'")
        .execute(&db.pool)
        .await?;

    // A single-connection pool, so the second transaction is guaranteed to be
    // the same physical connection the first one scoped.
    let app = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&db.app_url())
        .await?;

    let mut first = app.begin().await?;
    let mut scoped = Scoped::apply(&mut first, &WorkspaceScope::for_job(a)).await?;
    assert_eq!(visible_tags(scoped.conn()).await?, 1);
    first.commit().await?;

    let mut second = app.begin().await?;
    assert_eq!(
        visible_tags(&mut second).await?,
        0,
        "the previous transaction's scope leaked onto a pooled connection"
    );
    second.rollback().await?;
    Ok(())
}
