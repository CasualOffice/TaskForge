use super::*;

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn html_uploaded_as_a_png_is_rejected_at_commit() -> Result<()> {
    // docs/28 §Acceptance gates, the type-confusion test. This is the
    // stored-XSS vector the whole sniffing step exists for.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    let html = b"<html><script>alert(document.cookie)</script></html>";
    let id = presign(
        &caller,
        task,
        "innocent.png",
        "image/png",
        html.len() as i64,
    )
    .await?;
    caller.upload(caller.workspace, task, id, html).await?;

    let (status, body) = caller
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "markup was accepted: {body}"
    );
    assert_eq!(body["error"]["code"], "TF-ATT-0002");

    // The object is gone: leaving it would leave a reachable file no row
    // explains.
    let path = caller
        .root
        .join(caller.workspace.to_string())
        .join(task.to_string())
        .join(id.to_string());
    assert!(!path.exists(), "the refused object was left on disk");

    // And it never became visible.
    let (_, listed) = caller
        .get(&format!("/api/v1/tasks/{task}/attachments"))
        .await?;
    assert_eq!(listed["data"].as_array().map(Vec::len), Some(0), "{listed}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_declared_type_that_contradicts_the_bytes_is_rejected() -> Result<()> {
    // The other half of docs/28 §Validation: not markup, but still a lie. A PDF
    // declared as a PNG is refused, because the declaration is what pinned the
    // upload policy.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    let pdf = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n";
    let id = presign(&caller, task, "shot.png", "image/png", pdf.len() as i64).await?;
    caller.upload(caller.workspace, task, id, pdf).await?;

    let (status, body) = caller
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-ATT-0003");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_uncommitted_attachment_is_absent_from_every_read_path() -> Result<()> {
    // docs/28 §The invariant and its acceptance gate. The row exists from
    // pre-sign; nothing may see it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    let id = presign(&caller, task, "chart.png", "image/png", png.len() as i64).await?;

    // The row is there.
    assert!(
        test_support::attachment_exists(&db.pool, id).await?,
        "the pre-sign did not reserve a row"
    );
    // And it is in no read path.
    let (_, listed) = caller
        .get(&format!("/api/v1/tasks/{task}/attachments"))
        .await?;
    assert_eq!(listed["data"].as_array().map(Vec::len), Some(0), "{listed}");

    let (status, _) = caller
        .get(&format!("/api/v1/attachments/{id}/download"))
        .await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an uncommitted file was reachable"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_committed_file_is_not_downloadable_until_a_scan_clears_it() -> Result<()> {
    // D-062, the fail-closed default. Commit verifies; it does not make the file
    // available. Without a scanner the attachment stays PENDING forever, and
    // PENDING is a 409 rather than a 404 so the uploader is told to wait.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRxxxx";
    let id = presign(&caller, task, "chart.png", "image/png", png.len() as i64).await?;
    caller.upload(caller.workspace, task, id, png).await?;

    let (status, body) = caller
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["scan_status"], "PENDING");
    // The stored type came from the bytes, not the declaration.
    assert_eq!(body["content_type"], "image/png");

    // Still not downloadable, and still not listed. docs/28 §The invariant
    // makes this a 404 rather than a "wait": an uncommitted row is invisible to
    // EVERY read path, and a download is one. The friendlier 409 would mean this
    // endpoint reads rows the partial index exists to hide.
    let (status, refused) = caller
        .get(&format!("/api/v1/attachments/{id}/download"))
        .await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an unscanned file was served: {refused}"
    );

    let (_, listed) = caller
        .get(&format!("/api/v1/tasks/{task}/attachments"))
        .await?;
    assert_eq!(listed["data"].as_array().map(Vec::len), Some(0), "{listed}");

    // A CLEAN verdict is what makes it visible — the transition only
    // `mark_scanned` can perform.
    test_support::set_scan_verdict(&db.pool, caller.workspace, id, "CLEAN").await?;

    let (_, listed) = caller
        .get(&format!("/api/v1/tasks/{task}/attachments"))
        .await?;
    assert_eq!(listed["data"][0]["id"], id.to_string(), "{listed}");

    let response = caller
        .raw_get(&format!("/api/v1/attachments/{id}/download"))
        .await?;
    assert_eq!(response.status(), StatusCode::FOUND);
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .context("no redirect")?;
    // docs/28: the separate origin is "the single most important control here".
    assert!(
        location.starts_with("https://files.example.test/"),
        "the download was served from the application origin: {location}"
    );
    assert!(location.contains("signature="), "{location}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn concurrent_commit_requests_emit_one_scan_event() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;
    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRxxxx";
    let id = presign(&caller, task, "chart.png", "image/png", png.len() as i64).await?;
    caller.upload(caller.workspace, task, id, png).await?;
    let before = test_support::history_counts(&db.pool, id).await?;

    let request = || {
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/attachments/{id}/commit"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, &caller.cookie)
            .header("x-csrf-token", &caller.csrf)
            .header(WORKSPACE_HEADER, caller.workspace.to_string())
            .body(Body::from("{}"))
    };
    let (one, two) = tokio::join!(
        caller.app.clone().oneshot(request()?),
        caller.app.clone().oneshot(request()?),
    );
    for response in [one?, two?] {
        assert!(
            matches!(response.status(), StatusCode::ACCEPTED | StatusCode::OK),
            "concurrent commit returned {}",
            response.status()
        );
    }

    let after = test_support::history_counts(&db.pool, id).await?;
    assert_eq!(after.0 - before.0, 1, "duplicate activity event");
    assert_eq!(after.1 - before.1, 1, "duplicate audit event");
    assert_eq!(after.2 - before.2, 1, "duplicate outbox event");
    assert_eq!(
        after.3 - before.3,
        casual_task_persistence::CONSUMERS.len() as i64,
        "duplicate consumer deliveries"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_infected_file_is_never_served() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;
    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRxxxx";
    let id = presign(&caller, task, "chart.png", "image/png", png.len() as i64).await?;
    caller.upload(caller.workspace, task, id, png).await?;
    caller
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;

    // Neither verdict commits the row, so neither is reachable at all — the
    // invariant does the work before the verdict check gets a chance to.
    for verdict in ["INFECTED", "FAILED"] {
        test_support::set_scan_verdict(&db.pool, caller.workspace, id, verdict).await?;
        let (status, body) = caller
            .get(&format!("/api/v1/attachments/{id}/download"))
            .await?;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{verdict} was served: {body}"
        );
        let (_, listed) = caller
            .get(&format!("/api/v1/tasks/{task}/attachments"))
            .await?;
        assert_eq!(
            listed["data"].as_array().map(Vec::len),
            Some(0),
            "{verdict} was listed: {listed}"
        );
    }

    // And the second gate, on a row that IS committed: a file cleared once and
    // re-scanned as infected stops being served. This is the only way
    // `scanned_clean` is reachable, which is why it is asserted here.
    test_support::set_scan_verdict(&db.pool, caller.workspace, id, "CLEAN").await?;
    let (status, _) = caller
        .get(&format!("/api/v1/attachments/{id}/download"))
        .await?;
    assert_eq!(status, StatusCode::FOUND, "a clean file was not served");

    test_support::set_scan_verdict(&db.pool, caller.workspace, id, "INFECTED").await?;
    let (status, body) = caller
        .get(&format!("/api/v1/attachments/{id}/download"))
        .await?;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a re-scan that found malware kept serving the file: {body}"
    );
    assert_eq!(body["error"]["code"], "TF-ATT-0006");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_attachment_in_another_workspace_is_404_and_never_403() -> Result<()> {
    // docs/28 §Acceptance gates, the cross-tenant test: a pre-signed URL for
    // workspace A cannot be minted or used by a member of workspace B.
    let db = schema_harness::TestDatabase::start().await?;
    let owner = fresh(&db.pool, "owner@example.test", "acme").await?;
    let stranger = fresh(&db.pool, "stranger@example.test", "other").await?;
    let task = a_task(&owner, "WR").await?;

    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRxxxx";
    let id = presign(&owner, task, "chart.png", "image/png", png.len() as i64).await?;
    owner.upload(owner.workspace, task, id, png).await?;
    owner
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    test_support::set_scan_verdict(&db.pool, owner.workspace, id, "CLEAN").await?;

    // The owner can reach it, so the fixture is real.
    let response = owner
        .raw_get(&format!("/api/v1/attachments/{id}/download"))
        .await?;
    assert_eq!(response.status(), StatusCode::FOUND);

    // A member of another workspace cannot — and cannot tell it exists.
    for uri in [
        format!("/api/v1/attachments/{id}/download"),
        format!("/api/v1/attachments/{}/download", Uuid::now_v7()),
    ] {
        let (status, body) = stranger.get(&uri).await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
    }

    // Nor can they mint an upload against the other tenant's task.
    let (status, body) = stranger
        .post(
            &format!("/api/v1/tasks/{task}/attachments"),
            &serde_json::json!({
                "filename": "x.png", "content_type": "image/png",
                "byte_size": 10, "checksum": SHA,
            }),
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_traversing_filename_cannot_reach_the_object_key() -> Result<()> {
    // The key is three UUIDs, so a filename cannot address storage at all — and
    // the filename is refused separately, so both are true.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    for filename in ["../../etc/passwd", "a/b.png", "..", "with\\slash.png"] {
        let (status, body) = caller
            .post(
                &format!("/api/v1/tasks/{task}/attachments"),
                &serde_json::json!({
                    "filename": filename, "content_type": "image/png",
                    "byte_size": 10, "checksum": SHA,
                }),
            )
            .await?;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "accepted {filename:?}: {body}"
        );
    }

    // An accepted upload's key is the three ids and nothing else.
    let id = presign(&caller, task, "ordinary.png", "image/png", 10).await?;
    let key = test_support::attachment_object_key(&db.pool, id).await?;
    assert_eq!(key, format!("{}/{task}/{id}", caller.workspace));
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn commit_refuses_a_size_that_does_not_match_and_an_upload_that_never_happened() -> Result<()>
{
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    // Declared 999, uploaded 16.
    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    let id = presign(&caller, task, "chart.png", "image/png", 999).await?;
    caller.upload(caller.workspace, task, id, png).await?;
    let (status, body) = caller
        .post(
            &format!("/api/v1/attachments/{id}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "TF-ATT-0009");

    // Committing something that was never uploaded.
    let missing = presign(&caller, task, "ghost.png", "image/png", 10).await?;
    let (status, body) = caller
        .post(
            &format!("/api/v1/attachments/{missing}/commit"),
            &serde_json::json!({}),
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "TF-ATT-0005");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_attachment_routes_sit_inside_the_csrf_guard() -> Result<()> {
    // A route registered after `.layer()` escapes the guard, and nothing about
    // a handler would show it.
    let db = schema_harness::TestDatabase::start().await?;
    let caller = fresh(&db.pool, "dev@example.test", "acme").await?;
    let task = a_task(&caller, "WR").await?;

    for (uri, body) in [
        (
            format!("/api/v1/tasks/{task}/attachments"),
            r#"{"filename":"a.png","content_type":"image/png","byte_size":1,"checksum":""#
                .to_owned()
                + SHA
                + r#""}"#,
        ),
        (
            format!("/api/v1/attachments/{}/commit", Uuid::now_v7()),
            "{}".to_owned(),
        ),
    ] {
        let response = caller
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&uri)
                    .header(header::COOKIE, &caller.cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(WORKSPACE_HEADER, caller.workspace.to_string())
                    .body(Body::from(body))?,
            )
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{uri} accepted a state change with no CSRF token"
        );
    }
    Ok(())
}
