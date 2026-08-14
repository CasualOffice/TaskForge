//! C-011 scheduled retention, against real PostgreSQL.
//!
//! This is the seam the unit tests cannot prove: the scheduled runner must
//! acquire one session lease, drain more than one bounded batch, and release on
//! the same cancellation signal used by the rest of the worker.

mod schema_harness;

use std::num::NonZeroU32;
use std::time::Duration;

use anyhow::{Context, Result};
use casual_task_model::{ActorType, WorkspaceId, WorkspaceScope};
use casual_task_persistence::{Change, Provenance, Scoped, UnitOfWork, test_support};
use casual_task_worker::dispatcher::CancelOnDrop;
use casual_task_worker::retention::{self, Stopped};
use sqlx::PgPool;
use uuid::Uuid;

async fn seed(pool: &PgPool, count: usize) -> Result<WorkspaceId> {
    let workspace = WorkspaceId::new();
    test_support::insert_workspace(pool, workspace.as_uuid(), "retention").await?;

    for _ in 0..count {
        let aggregate = Uuid::now_v7();
        let mut tx = pool.begin().await?;
        let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
        UnitOfWork::record(
            &mut scoped,
            &Change {
                aggregate_type: "TASK".into(),
                aggregate_id: aggregate,
                project_id: None,
                event_type: "task.created".into(),
                activity_changes: serde_json::json!({}),
                audit_changes: serde_json::json!({}),
                payload: serde_json::json!({"task_id": aggregate}),
                schema_version: 1,
            },
            &Provenance {
                actor: None,
                actor_type: ActorType::System,
                request_id: None,
                correlation_id: None,
                ip: None,
                user_agent: None,
            },
        )
        .await?;
        tx.commit().await?;
    }

    test_support::age_completed_outbox(pool, workspace.as_uuid()).await?;
    Ok(workspace)
}

fn config() -> retention::Config {
    retention::Config::new(
        NonZeroU32::new(2).expect("two is non-zero"),
        Duration::from_millis(50),
        Duration::from_millis(10),
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker; run with --ignored"]
async fn one_scheduled_leader_drains_bounded_batches_and_releases_on_cancel() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = seed(&db.pool, 3).await?;
    let (stop, cancel) = CancelOnDrop::new();

    // Two process-shaped runners, one database lease. The leader-lease gate
    // proves exclusion directly; this pair proves the scheduled seam uses it.
    let first = {
        let pool = db.pool.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { retention::run(&pool, config(), cancel).await })
    };
    let second = {
        let pool = db.pool.clone();
        tokio::spawn(async move { retention::run(&pool, config(), cancel).await })
    };

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let history = test_support::history(&db.pool, workspace.as_uuid()).await?;
            if history.outbox.is_empty() && history.deliveries == 0 {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("the scheduled leader did not drain the eligible outbox")??;

    stop.cancel();
    assert_eq!(first.await??, Stopped::Cancelled);
    assert_eq!(second.await??, Stopped::Cancelled);

    Ok(())
}
