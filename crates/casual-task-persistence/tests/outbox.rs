//! C-011: the unit of work and the dispatch loop, against a real PostgreSQL.
//!
//! These assert the two guarantees that cannot be checked any other way.
//!
//! **ADR-006** — "domain change + activity + audit + outbox commit in one
//! transaction". A unit test can prove the code *issues* four inserts. Only a
//! database can prove a rollback takes all four back, and that is the half that
//! matters: the failure mode is a task whose history has a hole in it.
//!
//! **D-038** — claim, commit, deliver, record. The rejected design held a
//! transaction across consumer HTTP. The test below proves the claim's lock is
//! gone once its transaction commits, by taking that lock from a second
//! connection while the "HTTP call" is still notionally in flight.
//!
//! `#[ignore]` for the same reason as every other test here: Docker.

mod schema_harness;

use anyhow::Result;
use casual_task_model::{ActorType, WorkspaceId, WorkspaceScope};
use casual_task_persistence::{CONSUMERS, Change, Provenance, Scoped, UnitOfWork, dispatch};
use sqlx::Row;
use uuid::Uuid;

fn a_change(aggregate: Uuid) -> Change {
    Change {
        aggregate_type: "TASK".into(),
        aggregate_id: aggregate,
        project_id: None,
        event_type: "task.status.changed".into(),
        activity_changes: serde_json::json!({"status": {"from": "To Do", "to": "In Progress"}}),
        audit_changes: serde_json::json!({"status": {"from": "TODO", "to": "IN_PROGRESS"}}),
        payload: serde_json::json!({"task_id": aggregate}),
        schema_version: 1,
    }
}

fn nobody() -> Provenance {
    Provenance {
        actor: None,
        actor_type: ActorType::System,
        request_id: None,
        correlation_id: None,
        ip: None,
        user_agent: None,
    }
}

async fn a_workspace(pool: &sqlx::PgPool, slug: &str) -> Result<WorkspaceId> {
    let w = WorkspaceId::new();
    sqlx::query("INSERT INTO workspace (id, name, slug) VALUES ($1, $2, $3)")
        .bind(w.as_uuid())
        .bind(slug)
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(w)
}

/// How many rows each of the four writes left behind, for one aggregate.
async fn history_of(pool: &sqlx::PgPool, aggregate: Uuid) -> Result<(i64, i64, i64, i64)> {
    let row = sqlx::query(
        "SELECT (SELECT count(*) FROM activity_event WHERE aggregate_id = $1),
                (SELECT count(*) FROM audit_event    WHERE target_id    = $1),
                (SELECT count(*) FROM outbox_event   WHERE aggregate_id = $1),
                (SELECT count(*) FROM outbox_delivery d
                   JOIN outbox_event e ON e.id = d.event_id
                  WHERE e.aggregate_id = $1)",
    )
    .bind(aggregate)
    .fetch_one(pool)
    .await?;
    Ok((row.get(0), row.get(1), row.get(2), row.get(3)))
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_committed_change_leaves_activity_audit_outbox_and_every_delivery() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;
    let aggregate = Uuid::now_v7();

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    UnitOfWork::record(&mut scoped, &a_change(aggregate), &nobody()).await?;
    tx.commit().await?;

    let (activity, audit, outbox, deliveries) = history_of(&db.pool, aggregate).await?;
    assert_eq!(activity, 1, "no activity row");
    assert_eq!(audit, 1, "no audit row");
    assert_eq!(outbox, 1, "no outbox event");
    assert_eq!(
        deliveries,
        CONSUMERS.len() as i64,
        "an event reached {deliveries} consumers, not {}",
        CONSUMERS.len()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_rolled_back_change_leaves_no_history_at_all() -> Result<()> {
    // The guarantee ADR-006 actually buys. If the domain write fails after the
    // audit row is inserted, an auditor sees a status change that never
    // happened — worse than a missing one, because it is believed.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;
    let aggregate = Uuid::now_v7();

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    UnitOfWork::record(&mut scoped, &a_change(aggregate), &nobody()).await?;
    tx.rollback().await?;

    assert_eq!(
        history_of(&db.pool, aggregate).await?,
        (0, 0, 0, 0),
        "history survived a rollback: a change that never happened has a record"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_claim_holds_no_lock_once_it_commits() -> Result<()> {
    // D-038, stated as a test. After claiming, the worker does HTTP that may
    // take thirty seconds. If the claim's transaction were still open, this
    // second connection would block on it. It must not.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;
    let aggregate = Uuid::now_v7();

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    UnitOfWork::record(&mut scoped, &a_change(aggregate), &nobody()).await?;
    tx.commit().await?;

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let claimed = dispatch::claim(&mut scoped, "webhook_delivery", "worker-1", 10).await?;
    tx.commit().await?;
    assert_eq!(claimed.len(), 1, "claimed {} deliveries", claimed.len());
    assert_eq!(claimed[0].attempts, 1, "the claim did not count an attempt");

    // The "HTTP call" is now in flight. A second worker must be able to touch
    // the row — blocked here would mean a pinned connection per outstanding
    // delivery, which is the exhaustion D-039 bounds the pool against.
    let mut other = db.pool.begin().await?;
    let mut other_scoped = Scoped::apply(&mut other, &WorkspaceScope::for_job(workspace)).await?;
    let locked = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        sqlx::query("SELECT id FROM outbox_delivery WHERE id = $1 FOR UPDATE")
            .bind(claimed[0].delivery_id)
            .fetch_one(other_scoped.conn()),
    )
    .await;
    assert!(
        locked.is_ok(),
        "a second connection blocked on the claimed row — the claim is holding \
         its transaction open across delivery, which is exactly what D-038 rejects"
    );
    other.rollback().await?;

    // ...and recording the result is its own short transaction.
    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    dispatch::succeeded(&mut scoped, claimed[0].delivery_id).await?;
    tx.commit().await?;

    let dispatched: bool =
        sqlx::query_scalar("SELECT dispatched_at IS NOT NULL FROM outbox_delivery WHERE id = $1")
            .bind(claimed[0].delivery_id)
            .fetch_one(&db.pool)
            .await?;
    assert!(dispatched);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn one_consumer_failing_does_not_affect_the_other_five() -> Result<()> {
    // The reason delivery state is per-consumer at all. Before 0013 these six
    // shared one `attempts` column, so a webhook that was down would drag the
    // search projection through the same backoff.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;
    let aggregate = Uuid::now_v7();

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    UnitOfWork::record(&mut scoped, &a_change(aggregate), &nobody()).await?;
    tx.commit().await?;

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let webhook = dispatch::claim(&mut scoped, "webhook_delivery", "w", 10).await?;
    dispatch::failed(
        &mut scoped,
        webhook[0].delivery_id,
        webhook[0].attempts,
        "502",
    )
    .await?;
    tx.commit().await?;

    // The search projection is untouched and immediately claimable.
    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let search = dispatch::claim(&mut scoped, "search_projection", "w", 10).await?;
    tx.commit().await?;
    assert_eq!(
        search.len(),
        1,
        "a failing webhook blocked the search projection"
    );

    // And the failed webhook is NOT claimable again — it is waiting out its
    // first backoff step. A claim query that ignored next_attempt_at would spin
    // on a dead consumer as fast as the loop allows.
    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let again = dispatch::claim(&mut scoped, "webhook_delivery", "w", 10).await?;
    tx.commit().await?;
    assert!(
        again.is_empty(),
        "a delivery inside its backoff window was claimed anyway"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_ladder_is_walked_to_the_dead_letter_queue() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;
    let aggregate = Uuid::now_v7();

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    UnitOfWork::record(&mut scoped, &a_change(aggregate), &nobody()).await?;
    tx.commit().await?;

    let id: Uuid =
        sqlx::query_scalar("SELECT id FROM outbox_delivery WHERE consumer = 'webhook_delivery'")
            .fetch_one(&db.pool)
            .await?;

    // Baseline first: the gauge reports something while work is outstanding.
    // Without this, a gauge that returned None unconditionally would satisfy
    // the dead-letter assertion at the end of this test — and reporting zero
    // lag forever is a worse failure than reporting none, because it is
    // indistinguishable from health.
    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let lag = dispatch::oldest_pending_seconds(&mut scoped, "webhook_delivery").await?;
    tx.commit().await?;
    assert!(
        lag.is_some_and(|s| s >= 0.0),
        "the lag gauge reported nothing while a delivery was pending: {lag:?}"
    );

    // Attempts 1..=6 back off; the seventh has no rung left and dead-letters.
    for attempt in 1..=dispatch::BACKOFF.len() as i32 {
        let mut tx = db.pool.begin().await?;
        let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
        let dead = dispatch::failed(&mut scoped, id, attempt, "502").await?;
        tx.commit().await?;
        assert!(!dead, "dead-lettered on attempt {attempt}, too early");
    }

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let dead = dispatch::failed(&mut scoped, id, dispatch::BACKOFF.len() as i32 + 1, "502").await?;
    tx.commit().await?;
    assert!(dead, "the ladder never ended — this retries forever");

    // A dead-lettered delivery leaves the pending set entirely. One permanent
    // failure must not hold the lag gauge high forever (D-047).
    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let lag = dispatch::oldest_pending_seconds(&mut scoped, "webhook_delivery").await?;
    tx.commit().await?;
    assert_eq!(
        lag, None,
        "a dead-lettered delivery still counts as pending lag: the primary \
         health signal would stay high forever after one permanent failure"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn deliveries_for_one_aggregate_are_claimed_in_order() -> Result<()> {
    // docs/25 promises per-aggregate ordering. Nothing else provides it, and
    // out-of-order delivery of `task.created` after `task.status.changed` gives
    // a consumer a status change for a task it has never heard of.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;
    let aggregate = Uuid::now_v7();

    for _ in 0..3 {
        let mut tx = db.pool.begin().await?;
        let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
        UnitOfWork::record(&mut scoped, &a_change(aggregate), &nobody()).await?;
        tx.commit().await?;
    }

    // Even asking for ten, only the earliest is claimable: the other two have
    // an undelivered predecessor.
    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let batch = dispatch::claim(&mut scoped, "sse_fanout", "w", 10).await?;
    tx.commit().await?;
    assert_eq!(
        batch.len(),
        1,
        "{} deliveries for one aggregate were claimed at once — they can now be \
         delivered out of order",
        batch.len()
    );

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    dispatch::succeeded(&mut scoped, batch[0].delivery_id).await?;
    let next = dispatch::claim(&mut scoped, "sse_fanout", "w", 10).await?;
    tx.commit().await?;
    assert_eq!(next.len(), 1, "the successor did not become claimable");
    assert_ne!(next[0].delivery_id, batch[0].delivery_id);
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn two_workers_never_claim_the_same_delivery() -> Result<()> {
    // SKIP LOCKED, asserted rather than assumed. Two workers taking the same
    // delivery is a duplicate webhook — tolerable under at-least-once, but only
    // as a rare recovery case, not as the steady state of a two-worker deploy.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;

    // Distinct aggregates, so per-aggregate ordering is not what limits this.
    for _ in 0..8 {
        let mut tx = db.pool.begin().await?;
        let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
        UnitOfWork::record(&mut scoped, &a_change(Uuid::now_v7()), &nobody()).await?;
        tx.commit().await?;
    }

    let mut a = db.pool.begin().await?;
    let mut b = db.pool.begin().await?;
    let mut a_scoped = Scoped::apply(&mut a, &WorkspaceScope::for_job(workspace)).await?;
    let mut b_scoped = Scoped::apply(&mut b, &WorkspaceScope::for_job(workspace)).await?;

    let first = dispatch::claim(&mut a_scoped, "sse_fanout", "worker-a", 4).await?;
    // worker-b runs while worker-a's claim is still uncommitted — the worst
    // case, and the one SKIP LOCKED exists for.
    let second = dispatch::claim(&mut b_scoped, "sse_fanout", "worker-b", 4).await?;
    a.commit().await?;
    b.commit().await?;

    assert_eq!(first.len(), 4);
    assert_eq!(second.len(), 4, "worker-b blocked or came back empty");
    for x in &first {
        assert!(
            !second.iter().any(|y| y.delivery_id == x.delivery_id),
            "both workers claimed {}",
            x.delivery_id
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_abandoned_claim_is_reclaimable_and_a_fresh_one_is_not() -> Result<()> {
    // A worker killed between claim and record leaves a row claimed forever.
    // The expiry is the only thing that gets it delivered.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;
    let aggregate = Uuid::now_v7();

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    UnitOfWork::record(&mut scoped, &a_change(aggregate), &nobody()).await?;
    tx.commit().await?;

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let claimed = dispatch::claim(&mut scoped, "sse_fanout", "doomed-worker", 10).await?;
    tx.commit().await?;
    assert_eq!(claimed.len(), 1);

    // Still fresh: nobody else may take it.
    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let stolen = dispatch::claim(&mut scoped, "sse_fanout", "worker-2", 10).await?;
    tx.commit().await?;
    assert!(
        stolen.is_empty(),
        "a live worker's claim was stolen — every delivery would be duplicated"
    );

    // Age the claim past the expiry rather than sleeping five minutes.
    sqlx::query("UPDATE outbox_delivery SET claimed_at = now() - $1::interval WHERE id = $2")
        .bind(format!(
            "{} seconds",
            dispatch::CLAIM_EXPIRY.whole_seconds() + 60
        ))
        .bind(claimed[0].delivery_id)
        .execute(&db.pool)
        .await?;

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    let recovered = dispatch::claim(&mut scoped, "sse_fanout", "worker-2", 10).await?;
    tx.commit().await?;
    assert_eq!(
        recovered.len(),
        1,
        "an abandoned claim was never reclaimed — this delivery is lost forever"
    );
    assert_eq!(recovered[0].attempts, 2, "the reclaim did not count");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn delivery_rows_are_subject_to_tenant_isolation() -> Result<()> {
    // Migration 0013 creates this table after 0010's catalogue loop, so it does
    // not get a policy for free. A tenant table without one is exactly the
    // silent failure that whole mechanism exists to prevent — and the payload
    // in an outbox delivery is task content.
    let db = schema_harness::TestDatabase::start().await?;
    let alpha = a_workspace(&db.pool, "alpha").await?;
    let beta = a_workspace(&db.pool, "beta").await?;

    for w in [alpha, beta] {
        let mut tx = db.pool.begin().await?;
        let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(w)).await?;
        UnitOfWork::record(&mut scoped, &a_change(Uuid::now_v7()), &nobody()).await?;
        tx.commit().await?;
    }

    // As the application role, which is subject to RLS — the owner is not, and
    // the same assertion run as the owner would pass while proving nothing.
    sqlx::query("ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw'")
        .execute(&db.pool)
        .await?;
    let app = sqlx::PgPool::connect(&db.app_url()).await?;

    let mut tx = app.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(alpha)).await?;
    let visible: i64 = sqlx::query("SELECT count(*) FROM outbox_delivery")
        .fetch_one(scoped.conn())
        .await?
        .get(0);
    tx.rollback().await?;

    assert_eq!(
        visible,
        CONSUMERS.len() as i64,
        "alpha sees {visible} deliveries; it wrote {} and beta's must not be \
         among them",
        CONSUMERS.len()
    );
    Ok(())
}
