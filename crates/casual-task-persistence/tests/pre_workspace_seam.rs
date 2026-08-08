//! The pre-workspace seam, asserted (ADR-032, migration 0016).
//!
//! ADR-032 records this as "a deliberate hole in the ADR-020 backstop … security
//! -critical logic in SQL, outside the type system and outside
//! `unsafe_code = "forbid"`", and makes three things non-optional. Two are
//! asserted by the schema gate (the pinned `search_path`, the projection). The
//! third is here:
//!
//! > a test asserting it returns zero rows for a revoked or expired credential
//!
//! It runs as **`taskforge_app`**, not as the owner. As the owner, RLS is inert
//! and every assertion below would pass without the function existing at all.

mod schema_harness;

use anyhow::Result;
use casual_task_identity::credential;
use casual_task_model::{WorkspaceId, WorkspaceScope};
use casual_task_persistence::{Scoped, auth};
use uuid::Uuid;

/// A workspace and a token in it. Returns the presented credential.
async fn seed_token(
    pool: &sqlx::PgPool,
    workspace: WorkspaceId,
    slug: &str,
    expires_at: Option<&str>,
    revoked: bool,
) -> Result<String> {
    sqlx::query("INSERT INTO workspace (id, name, slug) VALUES ($1, $2, $2)")
        .bind(workspace.as_uuid())
        .bind(slug)
        .execute(pool)
        .await?;

    let minted = credential::mint()?;
    sqlx::query(
        "INSERT INTO api_token
             (id, workspace_id, principal_type, principal_id, token_selector,
              verifier_hash, name, expires_at, revoked_at)
         VALUES ($1,$2,'SERVICE_ACCOUNT',$3,$4,$5,'ci',
                 CASE WHEN $6::text IS NULL THEN NULL ELSE now() + $6::interval END,
                 CASE WHEN $7 THEN now() ELSE NULL END)",
    )
    .bind(Uuid::now_v7())
    .bind(workspace.as_uuid())
    .bind(Uuid::now_v7())
    .bind(&minted.selector)
    .bind(&minted.verifier_hash)
    .bind(expires_at)
    .bind(revoked)
    .execute(pool)
    .await?;

    Ok(minted.presented)
}

/// Whether a token has been marked used, read as the owner.
///
/// Asserted as a boolean in SQL: sqlx's `time` feature is not enabled, and
/// enabling a feature to decode a value only ever checked for presence would be
/// the tail wagging the dog.
async fn is_marked_used(pool: &sqlx::PgPool, id: Uuid) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT last_used_at IS NOT NULL FROM api_token WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

/// A pool connected as the application role — the one RLS actually applies to.
async fn app_pool(db: &schema_harness::TestDatabase) -> Result<sqlx::PgPool> {
    sqlx::query("ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw'")
        .execute(&db.pool)
        .await?;
    Ok(sqlx::PgPool::connect(&db.app_url()).await?)
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_live_token_is_found_without_any_workspace_scope() -> Result<()> {
    // The capability the seam exists for. No `set_config` has run on this
    // connection, so the RLS policy on api_token hides every row from an
    // ordinary SELECT — and the whole point is that authentication happens
    // before there is a workspace to set.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = WorkspaceId::new();
    let presented = seed_token(&db.pool, workspace, "alpha", Some("1 hour"), false).await?;
    let app = app_pool(&db).await?;
    let (selector, verifier) = credential::split(&presented).expect("well formed");

    let mut conn = app.acquire().await?;

    // The ordinary path sees nothing, which is what makes the seam necessary.
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM api_token")
        .fetch_one(&mut *conn)
        .await?;
    assert_eq!(visible, 0, "RLS is not applying to taskforge_app");

    let found = auth::lookup_token(&mut conn, selector)
        .await?
        .expect("the seam found nothing for a live token");
    assert_eq!(found.workspace_id, workspace.as_uuid());
    assert_eq!(found.principal_type, "SERVICE_ACCOUNT");

    // And the verifier matches through its own, separate door.
    let stored = auth::lookup_token_verifier(&mut conn, selector)
        .await?
        .expect("no verifier hash");
    assert!(credential::verify(verifier, &stored));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_revoked_token_returns_zero_rows() -> Result<()> {
    // ADR-032's named condition. A revoked credential that still authenticates
    // is the failure that makes revocation a lie — and docs/40 rejects JWTs
    // specifically so that revocation is immediate.
    let db = schema_harness::TestDatabase::start().await?;
    let presented = seed_token(&db.pool, WorkspaceId::new(), "alpha", Some("1 hour"), true).await?;
    let app = app_pool(&db).await?;
    let (selector, _) = credential::split(&presented).expect("well formed");
    let mut conn = app.acquire().await?;

    assert_eq!(
        auth::lookup_token(&mut conn, selector).await?,
        None,
        "a revoked token authenticated"
    );
    assert_eq!(
        auth::lookup_token_verifier(&mut conn, selector).await?,
        None,
        "a revoked token's verifier hash is still readable"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_expired_token_returns_zero_rows() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let presented = seed_token(
        &db.pool,
        WorkspaceId::new(),
        "alpha",
        Some("-1 hour"),
        false,
    )
    .await?;
    let app = app_pool(&db).await?;
    let (selector, _) = credential::split(&presented).expect("well formed");
    let mut conn = app.acquire().await?;

    assert_eq!(
        auth::lookup_token(&mut conn, selector).await?,
        None,
        "an expired token authenticated"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unknown_selector_is_indistinguishable_from_a_revoked_one() -> Result<()> {
    // docs/40 §Acceptance gates, the enumeration test in the shape this layer
    // can hold: the seam must not tell a caller whether a credential exists.
    let db = schema_harness::TestDatabase::start().await?;
    let revoked = seed_token(&db.pool, WorkspaceId::new(), "alpha", Some("1 hour"), true).await?;
    let live = seed_token(&db.pool, WorkspaceId::new(), "beta", Some("1 hour"), false).await?;
    let app = app_pool(&db).await?;
    let (revoked_selector, _) = credential::split(&revoked).expect("well formed");
    let (live_selector, _) = credential::split(&live).expect("well formed");
    let unknown = credential::mint()?;
    let mut conn = app.acquire().await?;

    // The positive control, and the reason this test can fail at all. Comparing
    // the revoked answer against the unknown one is satisfied by a seam that
    // returns `None` for everything — including a credential that should
    // authenticate. Establishing that this connection CAN see a live token is
    // what turns the equality below into a statement about revocation rather
    // than a tautology.
    assert!(
        auth::lookup_token(&mut conn, live_selector)
            .await?
            .is_some(),
        "the seam found nothing for a live token, so the comparison below proves nothing"
    );

    let for_revoked = auth::lookup_token(&mut conn, revoked_selector).await?;
    let for_unknown = auth::lookup_token(&mut conn, &unknown.selector).await?;
    assert_eq!(
        for_revoked, for_unknown,
        "a revoked token and an unknown one give different answers"
    );
    assert_eq!(
        for_revoked, None,
        "the shared answer is not the refusal — a revoked token authenticated"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_seam_returns_the_row_of_the_tenant_that_owns_it() -> Result<()> {
    // The seam reads across tenants by design. That is only safe if it returns
    // the RIGHT tenant — a lookup that matched the wrong row would authenticate
    // a caller into someone else's workspace, which is the worst outcome in the
    // system.
    let db = schema_harness::TestDatabase::start().await?;
    let (alpha, beta) = (WorkspaceId::new(), WorkspaceId::new());
    let alpha_token = seed_token(&db.pool, alpha, "alpha", Some("1 hour"), false).await?;
    let beta_token = seed_token(&db.pool, beta, "beta", Some("1 hour"), false).await?;
    let app = app_pool(&db).await?;
    let mut conn = app.acquire().await?;

    for (presented, expected) in [(alpha_token, alpha), (beta_token, beta)] {
        let (selector, _) = credential::split(&presented).expect("well formed");
        let found = auth::lookup_token(&mut conn, selector)
            .await?
            .expect("live token");
        assert_eq!(
            found.workspace_id,
            expected.as_uuid(),
            "the seam returned the wrong tenant's token"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn recording_use_goes_through_the_ordinary_tenant_scoped_path() -> Result<()> {
    // The seam is for the one read that cannot know its workspace. By the time
    // a token is being marked used, the workspace IS known — it came from the
    // lookup — so the write must go through RLS like everything else.
    let db = schema_harness::TestDatabase::start().await?;
    let (owner, intruder) = (WorkspaceId::new(), WorkspaceId::new());
    let presented = seed_token(&db.pool, owner, "alpha", Some("1 hour"), false).await?;
    seed_token(&db.pool, intruder, "beta", Some("1 hour"), false).await?;
    let app = app_pool(&db).await?;
    let (selector, _) = credential::split(&presented).expect("well formed");

    let mut conn = app.acquire().await?;
    let found = auth::lookup_token(&mut conn, selector)
        .await?
        .expect("live token");
    drop(conn);

    // The same write, under another tenant's scope, first. Without this the
    // test proves only that the UPDATE succeeded — which it would do just as
    // happily if `touch_token` took a bare connection and reached across
    // tenants, since the seam's SECURITY DEFINER functions already read every
    // workspace's rows. What must hold is that the scope RESTRICTS the write.
    let mut tx = app.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(intruder)).await?;
    auth::touch_token(&mut scoped, found.id).await?;
    tx.commit().await?;
    assert!(
        !is_marked_used(&db.pool, found.id).await?,
        "a write scoped to another workspace reached this tenant's token row"
    );

    let mut tx = app.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &WorkspaceScope::for_job(owner)).await?;
    auth::touch_token(&mut scoped, found.id).await?;
    tx.commit().await?;
    assert!(
        is_marked_used(&db.pool, found.id).await?,
        "the token was never marked used"
    );
    Ok(())
}
