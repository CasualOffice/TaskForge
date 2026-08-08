//! A workspace acquires an owner when it is created, and cannot lose its last
//! one (D-054, `docs/04` §Built-in role templates and control 4).
//!
//! # What these tests are actually about
//!
//! `role_assignment` is the only source of authority in the system (migration
//! 0003). Nothing created one, so a workspace committed with **no grants at
//! all**: its creator could see it and could not write to it, and no endpoint
//! could fix that because granting requires a grant. Every test here fails
//! without the bootstrap.
//!
//! The resolver assertions matter more than the row counts. Counting rows
//! proves the inserts ran; asking `casual_task_authz::allows` proves the rows
//! are the shape the resolver actually accepts, which is the claim.

mod schema_harness;

use anyhow::Result;
use casual_task_authz::{Actor, Grant, Principal, ResourceFacts, ResourceScopes, Scope};
use casual_task_model::{ProjectId, UserId, WorkspaceId, WorkspaceScope, permission, template};
use casual_task_persistence::workspace::WorkspaceRecord;
use casual_task_persistence::{Scoped, role, test_support, workspace};
use sqlx::PgPool;
use uuid::Uuid;

/// A person. The password hash is never used — nothing here logs in.
async fn person(pool: &PgPool, email: &str) -> Result<Uuid> {
    let id = Uuid::now_v7();
    test_support::insert_user_with_password(pool, id, email, "not-a-real-hash").await?;
    Ok(id)
}

/// Create a workspace the way the handler does: one transaction holding the
/// row, its membership, and the owner grant.
async fn create_workspace(
    pool: &PgPool,
    creator: Uuid,
    slug: &str,
) -> Result<(WorkspaceId, WorkspaceRecord)> {
    let id = WorkspaceId::new();
    // `for_job`, not `AuthContext::authenticated`: only the API edge may mint an
    // auth context (`docs/32`), and the architecture lint enforces it. A test is
    // not a request.
    let scope = WorkspaceScope::for_job(id);
    let mut tx = pool.begin().await?;
    let mut scoped = Scoped::apply(&mut tx, &scope).await?;

    let unowned = workspace::insert(&mut scoped, slug, slug).await?;
    workspace::insert_member(&mut scoped, creator, "MEMBER").await?;
    // The `Unowned` goes in and a `WorkspaceRecord` comes out. There is no
    // other way to get one, which is the compile-time half of this guarantee.
    let (record, _) = role::bootstrap(&mut scoped, unowned, creator).await?;

    tx.commit().await?;
    Ok((id, record))
}

/// The grants a principal holds, as `casual-task-authz` sees them.
///
/// Read back from the rows and handed to the real resolver, rather than
/// asserted against the repository that wrote them.
async fn grants_of(pool: &PgPool, workspace: WorkspaceId, actor: Uuid) -> Result<Vec<Grant>> {
    let rows = test_support::workspace_grants(pool, workspace.as_uuid()).await?;
    Ok(rows
        .into_iter()
        .filter(|(principal, _, _)| *principal == actor)
        .filter_map(|(principal, _, key)| {
            let found = permission::ALL
                .iter()
                .copied()
                .find(|p| p.as_str() == key)?;
            Some(Grant {
                workspace_id: workspace,
                principal: Principal::User(UserId::from_uuid(principal)),
                scope: Scope::Workspace(workspace),
                permissions: vec![found],
                constraints: Vec::new(),
            })
        })
        .collect())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_new_workspace_has_an_owner() -> Result<()> {
    // The defect, stated as a test. Before the bootstrap this workspace had no
    // role_assignment row of any kind.
    let db = schema_harness::TestDatabase::start().await?;
    let creator = person(&db.pool, "founder@example.com").await?;
    let (workspace, record) = create_workspace(&db.pool, creator, "acme").await?;

    assert_eq!(record.id, workspace.as_uuid());
    let grants = test_support::workspace_grants(&db.pool, workspace.as_uuid()).await?;
    assert!(
        !grants.is_empty(),
        "the workspace committed with no WORKSPACE-scope grant at all — this is \
         the D-054 state: its creator can see it and can never write to it"
    );
    assert!(
        grants
            .iter()
            .any(|(principal, role, key)| *principal == creator
                && role == "Owner"
                && key == "workspace.owner"),
        "the creator does not hold workspace.owner: {grants:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_creator_can_do_everything_according_to_the_real_resolver() -> Result<()> {
    // The claim that matters. Rows in a table are not authority; authority is
    // what `casual_task_authz::allows` returns, and this asks it — for every
    // permission in the closed registry, at workspace scope and at project
    // scope, because docs/04's scope chain is what carries a workspace grant
    // down to a project.
    let db = schema_harness::TestDatabase::start().await?;
    let creator = person(&db.pool, "founder@example.com").await?;
    let (workspace, _) = create_workspace(&db.pool, creator, "acme").await?;

    let grants = grants_of(&db.pool, workspace, creator).await?;
    let actor = Actor::user(UserId::from_uuid(creator));
    let facts = ResourceFacts::default();

    for scoped_to in [
        ResourceScopes::workspace(workspace),
        ResourceScopes::project(workspace, ProjectId::new()),
    ] {
        for wanted in permission::ALL {
            assert!(
                casual_task_authz::allows(&actor, *wanted, &scoped_to, &facts, &grants)
                    .is_allowed(),
                "the workspace's owner may not {wanted}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_stranger_gets_nothing_from_someone_elses_bootstrap() -> Result<()> {
    // The other direction, and the one a bootstrap gets wrong: seeding roles
    // into a workspace must not hand authority to anyone but the creator, and a
    // grant in workspace A must not reach workspace B.
    let db = schema_harness::TestDatabase::start().await?;
    let founder = person(&db.pool, "founder@example.com").await?;
    let stranger = person(&db.pool, "stranger@example.com").await?;
    let (workspace, _) = create_workspace(&db.pool, founder, "acme").await?;
    let (other, _) = create_workspace(&db.pool, stranger, "other").await?;

    let actor = Actor::user(UserId::from_uuid(stranger));
    let facts = ResourceFacts::default();

    // Nothing in the founder's workspace.
    let grants = grants_of(&db.pool, workspace, stranger).await?;
    assert!(
        grants.is_empty(),
        "a stranger holds a grant they were never given"
    );
    assert!(
        !casual_task_authz::allows(
            &actor,
            permission::PROJECT_CREATE,
            &ResourceScopes::workspace(workspace),
            &facts,
            &grants,
        )
        .is_allowed()
    );

    // And their own owner grant does not reach across the boundary.
    let own = grants_of(&db.pool, other, stranger).await?;
    assert!(!own.is_empty(), "the stranger owns their own workspace");
    assert!(
        !casual_task_authz::allows(
            &actor,
            permission::PROJECT_CREATE,
            &ResourceScopes::workspace(workspace),
            &facts,
            &own,
        )
        .is_allowed(),
        "a grant in one workspace authorized an action in another"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_five_templates_are_materialized_with_the_sets_docs_04_describes() -> Result<()> {
    // `role.workspace_id` is NOT NULL, so a template cannot be a global row and
    // a migration cannot seed one. They are materialized per workspace, and the
    // counts come from the table in `casual_task_model::template` so the two
    // cannot drift.
    let db = schema_harness::TestDatabase::start().await?;
    let creator = person(&db.pool, "founder@example.com").await?;
    let (workspace, _) = create_workspace(&db.pool, creator, "acme").await?;

    let mut expected: Vec<(String, i64)> = template::ROLES
        .iter()
        .map(|t| {
            (
                t.name.to_owned(),
                i64::try_from(t.permissions.len()).expect("small"),
            )
        })
        .collect();
    expected.sort();

    let found = test_support::role_templates(&db.pool, workspace.as_uuid()).await?;
    assert_eq!(found, expected);

    // Owner is the whole registry. If this drifts, the workspace's owner
    // silently cannot do something the product offers.
    assert_eq!(
        found
            .iter()
            .find(|(name, _)| name == "Owner")
            .map(|(_, n)| *n),
        Some(i64::try_from(permission::ALL.len()).expect("small"))
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_last_owner_grant_cannot_be_removed() -> Result<()> {
    // docs/04 control 4, migration 0021. Without it, D-054's state is one
    // DELETE away from being real again — and the workspace cannot recover,
    // because granting requires a grant.
    let db = schema_harness::TestDatabase::start().await?;
    let creator = person(&db.pool, "founder@example.com").await?;
    let (workspace, _) = create_workspace(&db.pool, creator, "acme").await?;

    let assignment = test_support::owner_assignment(&db.pool, workspace.as_uuid())
        .await?
        .expect("the workspace has an owner");

    let refused = test_support::delete_role_assignment(&db.pool, assignment).await;
    assert!(
        refused.is_err(),
        "the last owner grant was deleted, leaving a workspace nobody can \
         administer and nothing can repair"
    );

    // Still there, and still an owner.
    assert!(
        test_support::owner_assignment(&db.pool, workspace.as_uuid())
            .await?
            .is_some()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_last_owner_cannot_be_downgraded_either() -> Result<()> {
    // "Removed **or downgraded**". Moving the assignment onto a role without
    // `workspace.owner` is the same outcome by another route.
    let db = schema_harness::TestDatabase::start().await?;
    let creator = person(&db.pool, "founder@example.com").await?;
    let (workspace, _) = create_workspace(&db.pool, creator, "acme").await?;

    let assignment = test_support::owner_assignment(&db.pool, workspace.as_uuid())
        .await?
        .expect("owner");
    let member = test_support::role_by_name(&db.pool, workspace.as_uuid(), "Member")
        .await?
        .expect("the Member template was seeded");

    let refused = test_support::move_role_assignment(&db.pool, assignment, member).await;
    assert!(refused.is_err(), "the last owner was downgraded to Member");
    assert!(
        test_support::owner_assignment(&db.pool, workspace.as_uuid())
            .await?
            .is_some()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_second_owner_makes_the_first_removable() -> Result<()> {
    // The guard must protect the *last* owner, not every owner. A rule that
    // refused every revocation would make ownership permanent, which is a
    // different bug and a worse one to discover in production.
    let db = schema_harness::TestDatabase::start().await?;
    let creator = person(&db.pool, "founder@example.com").await?;
    let successor = person(&db.pool, "successor@example.com").await?;
    let (workspace, _) = create_workspace(&db.pool, creator, "acme").await?;

    let first = test_support::owner_assignment(&db.pool, workspace.as_uuid())
        .await?
        .expect("owner");
    let owner_role = test_support::role_by_name(&db.pool, workspace.as_uuid(), "Owner")
        .await?
        .expect("Owner template");
    test_support::grant_role_at_workspace(&db.pool, workspace.as_uuid(), successor, owner_role)
        .await?;

    let removed = test_support::delete_role_assignment(&db.pool, first).await?;
    assert_eq!(removed, 1, "the first owner could not be removed");
    assert!(
        test_support::owner_assignment(&db.pool, workspace.as_uuid())
            .await?
            .is_some(),
        "the successor is still the owner"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_ordinary_grant_is_freely_removable() -> Result<()> {
    // The guard must not fire on anything else. A trigger that refused every
    // revocation would be discovered the first time an admin removed somebody.
    let db = schema_harness::TestDatabase::start().await?;
    let creator = person(&db.pool, "founder@example.com").await?;
    let colleague = person(&db.pool, "colleague@example.com").await?;
    let (workspace, _) = create_workspace(&db.pool, creator, "acme").await?;

    let member_role = test_support::role_by_name(&db.pool, workspace.as_uuid(), "Member")
        .await?
        .expect("Member template");
    let assignment = test_support::grant_role_at_workspace(
        &db.pool,
        workspace.as_uuid(),
        colleague,
        member_role,
    )
    .await?;

    assert_eq!(
        test_support::delete_role_assignment(&db.pool, assignment).await?,
        1,
        "an ordinary Member grant could not be revoked"
    );
    Ok(())
}
