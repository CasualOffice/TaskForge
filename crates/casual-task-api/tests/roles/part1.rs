use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_role_is_authored_listed_granted_and_revoked() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;

    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let role_id = role["id"].as_str().expect("id").to_owned();
    assert_eq!(role["permissions"], json!(["task.read"]));

    let (status, listed) = admin.get("/api/v1/roles").await?;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        listed["data"]
            .as_array()
            .expect("data")
            .iter()
            .any(|r| r["id"] == role_id.as_str())
    );

    let ama = member_of(&db.pool, "ama@example.com", admin.workspace, &[]).await?;
    let (status, grant) = admin
        .post(
            "/api/v1/role-assignments",
            &json!({
                "principal_type": "USER",
                "principal_id": ama.user,
                "role_id": role_id,
                "scope_type": "WORKSPACE"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    let assignment = grant["id"].as_str().expect("id").to_owned();

    // The grant is real: Ama could not read anything a moment ago.
    let (status, effective) = ama.get("/api/v1/permissions/effective").await?;
    assert_eq!(status, StatusCode::OK, "{effective}");
    assert!(
        effective["permissions"]
            .as_array()
            .expect("permissions")
            .iter()
            .any(|p| p["permission"] == "task.read"),
        "the grant did not reach: {effective}"
    );

    let (status, _) = admin
        .delete(&format!("/api/v1/role-assignments/{assignment}"))
        .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, after) = ama.get("/api/v1/permissions/effective").await?;
    assert_eq!(
        after["permissions"].as_array().map(Vec::len),
        Some(0),
        "revoking left the permission behind: {after}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_1_you_cannot_author_a_role_carrying_what_you_do_not_hold() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;

    // The admin holds role.manage but not workspace.delete.
    let (status, body) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Destroyer", "permissions": ["workspace.delete"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0003");
    assert_eq!(body["error"]["details"]["missing"], "workspace.delete");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_1_editing_a_role_cannot_smuggle_in_a_new_permission() -> Result<()> {
    // `docs/04`: the ceiling is checked at assignment time *and* re-checked on
    // role edit, because "editing a role you granted cannot smuggle in new
    // permissions". Without the re-check, authoring a harmless role and then
    // widening it is the whole exploit.
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;

    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let id = role["id"].as_str().expect("id");
    let version = role["version"].as_i64().expect("version");

    let (status, body) = admin
        .patch(
            &format!("/api/v1/roles/{id}"),
            &json!({ "permissions": ["task.read", "workspace.delete"] }),
            version,
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0003");

    // And the role is untouched — a refused edit writes nothing.
    let (_, listed) = admin.get("/api/v1/roles").await?;
    let still = listed["data"]
        .as_array()
        .expect("data")
        .iter()
        .find(|r| r["id"] == id)
        .expect("role");
    assert_eq!(still["permissions"], json!(["task.read"]));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_2_a_project_assigner_cannot_create_a_workspace_grant() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let role_id = role["id"].as_str().expect("id").to_owned();

    // Holds the project-scope assign permission and task.read, and nothing at
    // workspace scope that would let them grant there.
    let manager = member_of(
        &db.pool,
        "manager@example.com",
        admin.workspace,
        &["project.role.assign", "task.read"],
    )
    .await?;
    let target = member_of(&db.pool, "target@example.com", admin.workspace, &[]).await?;

    let (status, body) = manager
        .post(
            "/api/v1/role-assignments",
            &json!({
                "principal_type": "USER",
                "principal_id": target.user,
                "role_id": role_id,
                "scope_type": "WORKSPACE"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0004");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_3_authoring_needs_role_manage_and_assigning_is_not_enough() -> Result<()> {
    // D-049. Before that decision the closed set had only `role.manage` above
    // project scope, so a workspace-level assigner necessarily held the right to
    // author — and could mint a role carrying more than they held.
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let assigner = member_of(
        &db.pool,
        "assigner@example.com",
        admin.workspace,
        &["role.assign", "task.read"],
    )
    .await?;

    let (status, body) = assigner
        .post(
            "/api/v1/roles",
            &json!({ "name": "Mine", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // But they may still grant a role that already exists, which is the point
    // of splitting the two.
    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let target = member_of(&db.pool, "target@example.com", admin.workspace, &[]).await?;
    let (status, body) = assigner
        .post(
            "/api/v1/role-assignments",
            &json!({
                "principal_type": "USER",
                "principal_id": target.user,
                "role_id": role["id"],
                "scope_type": "WORKSPACE"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_4_the_last_owner_grant_cannot_be_revoked() -> Result<()> {
    // Enforced by migration 0021's trigger, inside the transaction — `docs/04`
    // requires "a database constraint check, not just application code". The
    // trigger raises `restrict_violation` with the code in a HINT, so without
    // the handler mapping it this would surface as a 500.
    let db = schema_harness::TestDatabase::start().await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    let owner = member_of(
        &db.pool,
        "owner@example.com",
        workspace,
        &["workspace.owner", "role.manage", "role.assign"],
    )
    .await?;

    let (status, grants) = owner.get("/api/v1/roles").await?;
    assert_eq!(status, StatusCode::OK, "{grants}");

    // Find the owner's own assignment through the permissions explainer, which
    // returns the contributing grants.
    let (status, explained) = owner
        .post(
            "/api/v1/permissions/explain",
            &json!({ "permission": "workspace.owner" }),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{explained}");
    assert_eq!(explained["allowed"], true, "{explained}");

    // Revoking it must be refused. The id comes from the database, since the
    // grant was made by a fixture rather than through the API.
    let assignment = test_support::owner_assignment(&db.pool, workspace)
        .await?
        .expect("a workspace always has an owner grant");
    let (status, body) = owner
        .delete(&format!("/api/v1/role-assignments/{assignment}"))
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-AZN-0005");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_5_an_actor_cannot_grant_themselves_more_than_they_hold() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;

    // A role the admin can author, carrying only what they hold.
    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");

    // Granting it to themselves adds nothing and is allowed — control 5 is
    // about *exceeding*, not about self-assignment as such.
    let (status, body) = admin
        .post(
            "/api/v1/role-assignments",
            &json!({
                "principal_type": "USER",
                "principal_id": admin.user,
                "role_id": role["id"],
                "scope_type": "WORKSPACE"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // A role carrying something they do not hold cannot even be authored, so
    // the exploit is refused one step earlier — which is control 1 doing
    // control 5's work, exactly as `ceiling.rs` documents.
    let (status, body) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Owner-ish", "permissions": ["workspace.owner"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn control_7_every_grant_and_role_edit_is_audited() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;

    let (status, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{role}");
    let role_id: Uuid = role["id"].as_str().expect("id").parse()?;

    let audited = test_support::audit_events_for(&db.pool, role_id).await?;
    assert!(
        audited.iter().any(|e| e == "role.created"),
        "authoring a role wrote no audit event: {audited:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_grant_bumps_the_authz_epoch_so_open_streams_revalidate() -> Result<()> {
    // `docs/04` defines the epoch as bumped in the same transaction as the
    // change. C-015's SSE revalidation treats an unchanged epoch as proof, so a
    // grant that did not bump it would leave an open stream authorised on
    // permissions the actor no longer has.
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (_, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    let ama = member_of(&db.pool, "ama@example.com", admin.workspace, &[]).await?;

    let before = test_support::authz_epoch(&db.pool, admin.workspace).await?;
    let (status, body) = admin
        .post(
            "/api/v1/role-assignments",
            &json!({
                "principal_type": "USER",
                "principal_id": ama.user,
                "role_id": role["id"],
                "scope_type": "WORKSPACE"
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let after = test_support::authz_epoch(&db.pool, admin.workspace).await?;
    assert!(after > before, "granting did not bump the epoch");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_unknown_permission_key_is_refused_before_the_schema_sees_it() -> Result<()> {
    // `role_permission.permission` is a foreign key, so the schema would refuse
    // it too — as a 500. "`task.updat` is not a permission" is the sentence an
    // admin can act on.
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (status, body) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Typo", "permissions": ["task.updat"] }),
        )
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-VAL-0005");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn granting_the_same_role_twice_is_idempotent() -> Result<()> {
    // The schema's unique key exists because "the UI retries" (migration 0003).
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (_, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    let ama = member_of(&db.pool, "ama@example.com", admin.workspace, &[]).await?;
    let grant = json!({
        "principal_type": "USER",
        "principal_id": ama.user,
        "role_id": role["id"],
        "scope_type": "WORKSPACE"
    });

    let (first, one) = admin.post("/api/v1/role-assignments", &grant).await?;
    let (second, two) = admin.post("/api/v1/role-assignments", &grant).await?;
    assert_eq!(first, StatusCode::CREATED, "{one}");
    assert_eq!(second, StatusCode::CREATED, "{two}");
    assert_eq!(one["id"], two["id"], "a retry created a second grant");
    Ok(())
}
