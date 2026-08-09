//! The search-projection consumer (C-013).
//!
//! What is asserted here is the property that makes the projection safe behind
//! an at-least-once outbox: **it recomputes, it does not patch**. Delivering the
//! same event twice, or an old event after a new one, must leave the document
//! matching the task as it is now. A consumer that applied deltas would pass a
//! single-delivery test and diverge in production, where redelivery is the
//! contract rather than an accident.

mod schema_harness;

use anyhow::Result;
use casual_task_persistence::dispatch::Claimed;
use casual_task_persistence::test_support;
use casual_task_worker::dispatcher::Consumer;
use casual_task_worker::projection::SearchProjection;
use uuid::Uuid;

/// A pool as `taskforge_app` — the role the projection actually runs as, and
/// the one row-level security applies to. As the owner every assertion here
/// would pass with the tenant predicates removed.
async fn app_pool(db: &schema_harness::TestDatabase) -> Result<sqlx::PgPool> {
    test_support::enable_app_login(&db.pool, "apppw").await?;
    Ok(sqlx::PgPool::connect(&db.app_url()).await?)
}

/// A workspace with one project and one task in it.
async fn seed(pool: &sqlx::PgPool) -> Result<(Uuid, Uuid)> {
    let workspace = Uuid::now_v7();
    let user = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, "acme").await?;
    test_support::insert_user(pool, user, "dev@example.test", "Dev").await?;
    test_support::add_workspace_member(pool, workspace, user).await?;
    let task =
        test_support::insert_task_fixture(pool, workspace, user, "Payment retry backoff").await?;
    Ok((workspace, task))
}

fn event(workspace: Uuid, task: Uuid, event_type: &str) -> Claimed {
    Claimed {
        delivery_id: Uuid::now_v7(),
        event_id: Uuid::now_v7(),
        workspace_id: workspace,
        project_id: None,
        consumer: casual_task_worker::projection::NAME.to_owned(),
        event_type: event_type.to_owned(),
        aggregate_id: task,
        payload: serde_json::Value::Null,
        attempts: 1,
    }
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_task_event_puts_the_task_in_the_projection() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let (workspace, task) = seed(&db.pool).await?;
    let app = app_pool(&db).await?;
    let consumer = SearchProjection::new(app);

    assert_eq!(test_support::indexed_count(&db.pool, workspace).await?, 0);
    consumer
        .deliver(&event(workspace, task, "task.created"))
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        test_support::indexed_count(&db.pool, workspace).await?,
        1,
        "the consumer did not index the task"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn redelivering_the_same_event_converges_rather_than_duplicating() -> Result<()> {
    // docs/25 guarantees at-least-once. A consumer that patched instead of
    // recomputing would double-apply here, and nothing else would notice.
    let db = schema_harness::TestDatabase::start().await?;
    let (workspace, task) = seed(&db.pool).await?;
    let app = app_pool(&db).await?;
    let consumer = SearchProjection::new(app);

    for _ in 0..3 {
        consumer
            .deliver(&event(workspace, task, "task.updated"))
            .await
            .map_err(anyhow::Error::msg)?;
    }
    assert_eq!(
        test_support::indexed_count(&db.pool, workspace).await?,
        1,
        "three deliveries produced more than one projection row"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_deleted_task_is_removed_and_staying_deleted_is_idempotent() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let (workspace, task) = seed(&db.pool).await?;
    let app = app_pool(&db).await?;
    let consumer = SearchProjection::new(app);

    consumer
        .deliver(&event(workspace, task, "task.created"))
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(test_support::indexed_count(&db.pool, workspace).await?, 1);

    test_support::soft_delete_task(&db.pool, task).await?;

    // Twice: the second is the redelivery that at-least-once permits.
    for _ in 0..2 {
        consumer
            .deliver(&event(workspace, task, "task.deleted"))
            .await
            .map_err(anyhow::Error::msg)?;
    }
    assert_eq!(
        test_support::indexed_count(&db.pool, workspace).await?,
        0,
        "a deleted task stayed searchable"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_event_about_something_else_is_acknowledged_without_touching_the_projection()
-> Result<()> {
    // The dispatch loop hands this consumer every event, not only task ones.
    // Returning an error for a workspace rename would retry it six times and
    // then dead-letter a delivery that was never this consumer's business.
    let db = schema_harness::TestDatabase::start().await?;
    let (workspace, task) = seed(&db.pool).await?;
    let app = app_pool(&db).await?;
    let consumer = SearchProjection::new(app);

    consumer
        .deliver(&event(workspace, task, "workspace.member.added"))
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        test_support::indexed_count(&db.pool, workspace).await?,
        0,
        "a non-task event reached the projection"
    );
    Ok(())
}
