use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_grants_can_be_listed_narrowed_and_then_revoked_from_the_listing() -> Result<()> {
    // The gap this closes: before `GET /role-assignments`, the id needed to
    // revoke a grant appeared exactly once — in the response to the call that
    // created it. An admin who closed the tab could never take it back.
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (_, reader) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    let (_, writer) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Writer", "permissions": ["task.read", "task.update"] }),
        )
        .await?;
    let ama = member_of(&db.pool, "ama@example.com", admin.workspace, &[]).await?;
    let bo = member_of(&db.pool, "bo@example.com", admin.workspace, &[]).await?;

    for (who, role) in [(ama.user, &reader), (bo.user, &writer)] {
        let (status, body) = admin
            .post(
                "/api/v1/role-assignments",
                &json!({
                    "principal_type": "USER",
                    "principal_id": who,
                    "role_id": role["id"],
                    "scope_type": "WORKSPACE"
                }),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    let (status, all) = admin.get("/api/v1/role-assignments").await?;
    assert_eq!(status, StatusCode::OK, "{all}");
    let rows = all["data"].as_array().expect("data");
    // The admin's own owner grant is in here too, which is the point: the list
    // is every grant, not every grant this call happened to create.
    assert!(rows.len() >= 3, "{all}");

    // Narrowed to one person — the question an admin screen actually asks.
    let (status, hers) = admin
        .get(&format!(
            "/api/v1/role-assignments?principal_id={}",
            ama.user
        ))
        .await?;
    assert_eq!(status, StatusCode::OK, "{hers}");
    let hers_rows = hers["data"].as_array().expect("data");
    assert_eq!(hers_rows.len(), 1, "{hers}");
    assert_eq!(hers_rows[0]["role_id"], reader["id"]);

    // And narrowed to one role.
    let writer_id = writer["id"].as_str().context("role id")?;
    let (_, by_role) = admin
        .get(&format!("/api/v1/role-assignments?role_id={writer_id}"))
        .await?;
    let by_role_rows = by_role["data"].as_array().expect("data");
    assert_eq!(by_role_rows.len(), 1, "{by_role}");
    assert_eq!(by_role_rows[0]["principal_id"], bo.user.to_string());

    // The whole point: revoke using only what the listing gave back.
    let id = hers_rows[0]["id"].as_str().expect("assignment id");
    let (status, body) = admin
        .delete(&format!("/api/v1/role-assignments/{id}"))
        .await?;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, after) = admin
        .get(&format!(
            "/api/v1/role-assignments?principal_id={}",
            ama.user
        ))
        .await?;
    assert!(
        after["data"].as_array().expect("data").is_empty(),
        "the revoked grant is still listed: {after}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_grant_listing_pages_by_cursor_and_never_by_offset() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    let (_, role) = admin
        .post(
            "/api/v1/roles",
            &json!({ "name": "Reader", "permissions": ["task.read"] }),
        )
        .await?;
    for i in 0..3 {
        let who = member_of(&db.pool, &format!("m{i}@example.com"), admin.workspace, &[]).await?;
        admin
            .post(
                "/api/v1/role-assignments",
                &json!({
                    "principal_type": "USER",
                    "principal_id": who.user,
                    "role_id": role["id"],
                    "scope_type": "WORKSPACE"
                }),
            )
            .await?;
    }

    let role_id = role["id"].as_str().context("role id")?;
    let (status, first) = admin
        .get(&format!(
            "/api/v1/role-assignments?role_id={role_id}&limit=2"
        ))
        .await?;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"].as_array().expect("data").len(), 2, "{first}");
    assert_eq!(first["page"]["has_more"], true, "{first}");

    let cursor = first["page"]["next_cursor"].as_str().expect("cursor");
    let (_, second) = admin
        .get(&format!(
            "/api/v1/role-assignments?role_id={role_id}&limit=2&cursor={cursor}"
        ))
        .await?;
    assert_eq!(
        second["data"].as_array().expect("data").len(),
        1,
        "{second}"
    );
    assert_eq!(second["page"]["has_more"], false, "{second}");

    // No overlap: a cursor that resumed at the wrong row would repeat one.
    let seen: Vec<&str> = first["data"]
        .as_array()
        .expect("data")
        .iter()
        .chain(second["data"].as_array().expect("data"))
        .map(|row| row["id"].as_str().expect("id"))
        .collect();
    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        seen.len(),
        "a page repeated a grant: {seen:?}"
    );

    let (status, body) = admin.get("/api/v1/role-assignments?limit=500").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "TF-QRY-0007");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn reading_the_grants_needs_the_same_authority_as_assigning_them() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let admin = admin(&db.pool, "acme").await?;
    // A member with neither `role.assign` nor `role.manage`. Who holds what is
    // the shape of the workspace's authority, not tenant content.
    let nobody = member_of(
        &db.pool,
        "nobody@example.com",
        admin.workspace,
        &["task.read"],
    )
    .await?;
    let (status, body) = nobody.get("/api/v1/role-assignments").await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    Ok(())
}
