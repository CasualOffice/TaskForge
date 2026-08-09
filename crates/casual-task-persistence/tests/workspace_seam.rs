//! The membership seam, asserted as the role row-level security applies to.
//!
//! Migration 0019 exists because `workspace_membership` carries `workspace_id`
//! and therefore a policy (migration 0010), while the two reads that establish
//! a tenant scope necessarily run before any workspace has been set. Everything
//! here runs as **`taskforge_app`**, not as the owner: as the owner RLS is inert
//! and every assertion below would pass with no seam at all — which is exactly
//! how the broken version shipped.

mod schema_harness;

use anyhow::Result;
use casual_task_persistence::{test_support, workspace};
use uuid::Uuid;

/// A pool connected as the application role — the one RLS actually applies to.
async fn app_pool(db: &schema_harness::TestDatabase) -> Result<sqlx::PgPool> {
    sqlx::query("ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw'")
        .execute(&db.pool)
        .await?;
    Ok(sqlx::PgPool::connect(&db.app_url()).await?)
}

/// A user, two workspaces, membership of the first only.
async fn seed(pool: &sqlx::PgPool) -> Result<(Uuid, Uuid, Uuid)> {
    let user = Uuid::now_v7();
    test_support::insert_user(pool, user, "member@example.test", "Member").await?;
    let mine = Uuid::now_v7();
    let theirs = Uuid::now_v7();
    test_support::insert_workspace(pool, mine, "mine").await?;
    test_support::insert_workspace(pool, theirs, "theirs").await?;
    test_support::add_workspace_member(pool, mine, user).await?;
    Ok((user, mine, theirs))
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unscoped_membership_read_sees_nothing_which_is_why_the_seam_exists() -> Result<()> {
    // The bug migration 0019 fixes, reproduced. `SELECT ... FROM
    // workspace_membership` on a connection with no `taskforge.workspace_id`
    // returns zero rows for the application role, so the membership check that
    // mints an AuthContext answered `false` for everyone — and no one could
    // enter any workspace at all.
    let db = schema_harness::TestDatabase::start().await?;
    let (user, _, _) = seed(&db.pool).await?;
    let app = app_pool(&db).await?;

    assert_eq!(
        test_support::unscoped_membership_count(&db.pool, user).await?,
        1,
        "the row is there for the owner, for whom RLS is inert"
    );
    assert_eq!(
        test_support::unscoped_membership_count(&app, user).await?,
        0,
        "if this is non-zero the policy has been removed, and tenant isolation \
         on workspace_membership with it"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_seam_answers_membership_for_the_application_role() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let (user, mine, theirs) = seed(&db.pool).await?;
    let app = app_pool(&db).await?;
    let mut conn = app.acquire().await?;

    assert!(
        workspace::is_member(&mut conn, user, mine).await?,
        "a member was refused their own workspace; nobody can sign in"
    );
    assert!(
        !workspace::is_member(&mut conn, user, theirs).await?,
        "the seam admitted a non-member"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_seam_lists_only_the_caller_s_own_workspaces() -> Result<()> {
    // The seam is filtered by user and never by workspace, so it cannot be
    // pointed at someone else. This is the property that keeps it from becoming
    // a cross-tenant read (docs/32 §The user_account exception).
    let db = schema_harness::TestDatabase::start().await?;
    let (user, mine, _) = seed(&db.pool).await?;
    let stranger = Uuid::now_v7();
    test_support::insert_user(&db.pool, stranger, "stranger@example.test", "Stranger").await?;

    let app = app_pool(&db).await?;
    let mut conn = app.acquire().await?;

    let found = workspace::list_for_user(&mut conn, user, None, 10).await?;
    assert_eq!(
        found.iter().map(|w| w.id).collect::<Vec<_>>(),
        vec![mine],
        "the workspace list is not exactly the caller's memberships"
    );

    assert!(
        workspace::list_for_user(&mut conn, stranger, None, 10)
            .await?
            .is_empty(),
        "a person with no memberships was shown a workspace"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_soft_deleted_workspace_is_unreachable_not_merely_hidden() -> Result<()> {
    // docs/32 §Deletion: soft delete starts a 30-day grace window in which the
    // workspace is "restorable, hidden, billing stopped". Hidden has to mean
    // its members cannot enter it either, or the deletion did nothing.
    let db = schema_harness::TestDatabase::start().await?;
    let (user, mine, _) = seed(&db.pool).await?;
    sqlx::query("UPDATE workspace SET deleted_at = now() WHERE id = $1")
        .bind(mine)
        .execute(&db.pool)
        .await?;

    let app = app_pool(&db).await?;
    let mut conn = app.acquire().await?;

    assert!(!workspace::is_member(&mut conn, user, mine).await?);
    assert!(
        workspace::list_for_user(&mut conn, user, None, 10)
            .await?
            .is_empty()
    );
    Ok(())
}
