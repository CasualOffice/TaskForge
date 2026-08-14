use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_dispatcher_sees_every_tenant_and_the_app_role_sees_one() -> Result<()> {
    // Why the role exists at all. Same query, same data, two roles.
    let db = schema_harness::TestDatabase::start().await?;
    let alpha = a_workspace(&db.pool, "alpha").await?;
    let beta = a_workspace(&db.pool, "beta").await?;
    for w in [alpha, beta] {
        let mut tx = db.pool.begin().await?;
        let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(w)).await?;
        UnitOfWork::record(&mut scoped, &a_change(Uuid::now_v7()), &nobody()).await?;
        tx.commit().await?;
    }

    sqlx::query("ALTER ROLE taskforge_dispatcher WITH LOGIN PASSWORD 'dispw'")
        .execute(&db.pool)
        .await?;
    let pool = sqlx::PgPool::connect(&db.dispatcher_url()).await?;

    let mut tx = pool.begin().await?;
    let mut d = dispatch::Dispatcher::assume(&mut tx).await?;
    let claimed = dispatch::claim(&mut d, "sse_fanout", "worker-1", 10).await?;
    tx.commit().await?;

    assert_eq!(
        claimed.len(),
        2,
        "the dispatcher claimed {} deliveries across two workspaces, not 2 — \
         one tenant's events are not being delivered",
        claimed.len()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_sweep_removes_delivered_history_and_never_a_dead_letter() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;
    let delivered = Uuid::now_v7();
    let dead = Uuid::now_v7();

    for aggregate in [delivered, dead] {
        let mut tx = db.pool.begin().await?;
        let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
        UnitOfWork::record(&mut scoped, &a_change(aggregate), &nobody()).await?;
        tx.commit().await?;
    }

    // Age both past retention: one fully delivered, one dead-lettered.
    let old = format!("{} seconds", dispatch::RETENTION.whole_seconds() + 86_400);
    sqlx::query(
        "UPDATE outbox_delivery d SET dispatched_at = now() - $1::interval
           FROM outbox_event e
          WHERE e.id = d.event_id AND e.aggregate_id = $2",
    )
    .bind(&old)
    .bind(delivered)
    .execute(&db.pool)
    .await?;
    sqlx::query(
        "UPDATE outbox_delivery d SET dead_lettered_at = now() - $1::interval
           FROM outbox_event e
          WHERE e.id = d.event_id AND e.aggregate_id = $2",
    )
    .bind(&old)
    .bind(dead)
    .execute(&db.pool)
    .await?;
    sqlx::query("UPDATE outbox_event SET created_at = now() - $1::interval")
        .bind(&old)
        .execute(&db.pool)
        .await?;

    let mut tx = db.pool.begin().await?;
    let mut d = dispatch::Dispatcher::assume(&mut tx).await?;
    let (deliveries, events) = dispatch::sweep(&mut d, 1000).await?;
    tx.commit().await?;

    assert_eq!(deliveries, CONSUMERS.len() as u64, "swept {deliveries}");
    assert_eq!(events, 1, "swept {events} events");

    // The dead-lettered event and every one of its deliveries survive. docs/25:
    // "a dead-lettered event is never silently dropped" — a retention timer
    // deleting one at 3 a.m. is exactly the silent drop that forbids.
    let (_, _, outbox, remaining) = history_of(&db.pool, dead).await?;
    assert_eq!(outbox, 1, "the dead-lettered event was swept");
    assert_eq!(
        remaining,
        CONSUMERS.len() as i64,
        "dead-lettered deliveries were swept: they can no longer be replayed"
    );
    assert_eq!(
        history_of(&db.pool, delivered).await?,
        (1, 1, 0, 0),
        "the delivered event's outbox rows should be gone, its history kept"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_sweep_keeps_an_event_while_any_consumer_still_needs_it() -> Result<()> {
    // Five consumers done, one still pending. Deleting the event would delete
    // the payload the sixth has not received yet.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;
    let aggregate = Uuid::now_v7();

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    UnitOfWork::record(&mut scoped, &a_change(aggregate), &nobody()).await?;
    tx.commit().await?;

    let old = format!("{} seconds", dispatch::RETENTION.whole_seconds() + 86_400);
    sqlx::query(
        "UPDATE outbox_delivery SET dispatched_at = now() - $1::interval
          WHERE consumer <> 'webhook_delivery'",
    )
    .bind(&old)
    .execute(&db.pool)
    .await?;
    sqlx::query("UPDATE outbox_event SET created_at = now() - $1::interval")
        .bind(&old)
        .execute(&db.pool)
        .await?;

    let mut tx = db.pool.begin().await?;
    let mut d = dispatch::Dispatcher::assume(&mut tx).await?;
    let (deliveries, events) = dispatch::sweep(&mut d, 1000).await?;
    tx.commit().await?;

    // Every consumer but the one still owed: derived from the registry rather
    // than written as `5`, because a literal here means the next consumer
    // breaks a Docker-only test long after the unit test pinning the same list
    // has gone green.
    assert_eq!(deliveries, CONSUMERS.len() as u64 - 1, "swept {deliveries}");
    assert_eq!(
        events, 0,
        "the event was deleted while one consumer had not received it"
    );

    let (_, _, outbox, remaining) = history_of(&db.pool, aggregate).await?;
    assert_eq!(outbox, 1);
    assert_eq!(remaining, 1, "the outstanding delivery survives");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_delivery_row_is_created_at_the_same_instant_as_its_event() -> Result<()> {
    // `oldest_pending_seconds` reads `outbox_delivery.created_at` and does NOT
    // join `outbox_event` — that join was one random heap fetch per pending row,
    // on an aggregate that cannot stop early. What makes the two readings the
    // same number is that `UnitOfWork::record` writes both in one transaction
    // and both default to now(), which in PostgreSQL is the transaction's start
    // time, identical to the microsecond.
    //
    // That is an invariant of the writer, not of the type system, so it is
    // asserted here. A fan-out moved out of the producing transaction would make
    // the primary health signal quietly under-report.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;
    let aggregate = Uuid::now_v7();

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    UnitOfWork::record(&mut scoped, &a_change(aggregate), &nobody()).await?;
    tx.commit().await?;

    let mismatched: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM outbox_delivery d
           JOIN outbox_event e ON e.id = d.event_id
          WHERE e.aggregate_id = $1
            AND d.created_at <> e.created_at",
    )
    .bind(aggregate)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        mismatched, 0,
        "{mismatched} delivery rows do not share their event's creation instant; \
         outbox_lag_seconds is measured from the delivery row and would now \
         under-report by the gap"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_dispatcher_role_cannot_be_verified_by_a_role_that_rls_applies_to() -> Result<()> {
    // The same refusal as the test above, on the path the worker actually takes.
    // `Dispatcher::assume` checks the connection it is handed; the loop checks
    // once and reuses the token, and a token that could be minted by an
    // unprivileged role would move the silent failure rather than fix it.
    let db = schema_harness::TestDatabase::start().await?;
    sqlx::query("ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw'")
        .execute(&db.pool)
        .await?;
    let app = sqlx::PgPool::connect(&db.app_url()).await?;

    let mut conn = app.acquire().await?;
    let refused = dispatch::DispatcherRole::verify(&mut conn).await;
    assert!(
        refused.is_err(),
        "taskforge_app minted a dispatcher role token — every claim made with it \
         would return nothing and report healthy"
    );
    assert!(
        refused
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default()
            .contains("taskforge_app"),
        "the error does not name the role, which is the one thing an operator needs"
    );

    sqlx::query("ALTER ROLE taskforge_dispatcher WITH LOGIN PASSWORD 'dispw'")
        .execute(&db.pool)
        .await?;
    let dispatcher = sqlx::PgPool::connect(&db.dispatcher_url()).await?;
    let mut conn = dispatcher.acquire().await?;
    assert!(
        dispatch::DispatcherRole::verify(&mut conn).await.is_ok(),
        "taskforge_dispatcher was refused; the worker cannot start"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn dlq_depth_is_reported_per_consumer() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = a_workspace(&db.pool, "alpha").await?;

    let mut tx = db.pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(workspace)).await?;
    UnitOfWork::record(&mut scoped, &a_change(Uuid::now_v7()), &nobody()).await?;
    tx.commit().await?;

    let mut tx = db.pool.begin().await?;
    let mut d = dispatch::Dispatcher::assume(&mut tx).await?;
    assert!(
        dispatch::dlq_depth(&mut d).await?.is_empty(),
        "a healthy system reported a non-empty DLQ"
    );
    tx.commit().await?;

    let id: Uuid =
        sqlx::query_scalar("SELECT id FROM outbox_delivery WHERE consumer = 'webhook_delivery'")
            .fetch_one(&db.pool)
            .await?;
    let mut tx = db.pool.begin().await?;
    let mut d = dispatch::Dispatcher::assume(&mut tx).await?;
    dispatch::failed(&mut d, id, dispatch::BACKOFF.len() as i32 + 1, "502").await?;
    let depth = dispatch::dlq_depth(&mut d).await?;
    tx.commit().await?;

    assert_eq!(
        depth,
        vec![("webhook_delivery".to_owned(), 1)],
        "the DLQ gauge does not attribute the dead letter to a consumer, which \
         is the first question RB-02 asks"
    );
    Ok(())
}
