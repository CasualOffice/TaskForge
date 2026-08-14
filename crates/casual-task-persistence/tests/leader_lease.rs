//! C-011 scheduled-job leadership, against real PostgreSQL.
//!
//! A mock cannot prove the property: advisory locks belong to a database
//! session, and returning that session to a pool is the exact failure this test
//! guards against.

mod schema_harness;

use std::time::Duration;

use anyhow::{Context, Result};
use casual_task_persistence::leader_lease::LeaderLease;

const JOB: &str = "leader-lease-acceptance";

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn one_instance_leads_and_both_release_paths_free_the_lock() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;

    let mut first = LeaderLease::try_acquire(&db.pool, JOB)
        .await?
        .context("the first instance did not acquire an unheld lease")?;
    assert_eq!(first.name(), JOB);
    first.heartbeat().await?;

    assert!(
        LeaderLease::try_acquire(&db.pool, JOB).await?.is_none(),
        "two sessions held the same scheduled-job lease"
    );

    first.release().await?;
    let second = LeaderLease::try_acquire(&db.pool, JOB)
        .await?
        .context("explicit release left the advisory lock held")?;

    // Cancellation and panic do not await an unlock. close-on-drop must close
    // the session rather than return its still-held lock to the pool.
    drop(second);
    let third = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(lease) = LeaderLease::try_acquire(&db.pool, JOB).await? {
                return Ok::<_, sqlx::Error>(lease);
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("drop did not release the advisory lock")??;
    third.release().await?;

    Ok(())
}
