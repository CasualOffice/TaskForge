//! The three steps of the upload handshake, and the download
//! (`docs/28` §The handshake).

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_app::attachment::policy;
use casual_task_model::permission;
use casual_task_persistence::attachment::{self, NewAttachment};
use casual_task_persistence::{Change, UnitOfWork};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

use super::guard;
use super::validate;
use super::wire::{AttachmentView, CommitResponse, PresignRequest, PresignResponse};
use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::{self as api_wire, Body, Page, Paged};

fn view(row: &attachment::AttachmentRow) -> AttachmentView {
    AttachmentView {
        id: row.id,
        task_id: row.task_id,
        filename: row.filename.clone(),
        content_type: row.content_type.clone(),
        byte_size: row.byte_size,
        checksum: row.checksum.clone(),
        scan_status: row.scan_status.clone(),
        uploaded_by: row.uploaded_by,
        created_at: api_wire::timestamp(row.created_at),
    }
}

/// `POST /api/v1/tasks/{id}/attachments` — step 1, mint permission to upload.
///
/// Nothing about this response is a capability over anyone else's data: the URL
/// is signed over a key built from three UUIDs (`docs/32`), and the row it
/// reserves is invisible until the scan clears it.
///
/// # Errors
///
/// `404` invisible task, `403` without `task.attachment.create`, `400`/`422`
/// for a field or size refusal.
pub async fn presign(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Body(body): Body<PresignRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let task = guard::task_for(
        &mut scoped,
        &ctx,
        task_id,
        permission::TASK_ATTACHMENT_CREATE,
        &request_id,
    )
    .await?;

    // The workspace's configured ceiling is not implemented yet, so this is the
    // system default; `size_limit` is where a per-workspace setting will arrive
    // and is already clamped to docs/28's 2 GB.
    let max_bytes = policy::size_limit(None);
    validate::presign(
        &body.filename,
        body.byte_size,
        &body.checksum,
        max_bytes,
        &request_id,
    )?;

    let existing = attachment::count_for_task(&mut scoped, task.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "counting attachments failed");
            ApiError::internal(&request_id)
        })?;
    if existing >= policy::MAX_FILES_PER_TASK {
        return Err(ApiError::unprocessable(
            codes::ATTACHMENT_TOO_MANY,
            "This task already has the maximum number of attachments",
            &request_id,
        )
        .with_details(serde_json::json!({ "limit": policy::MAX_FILES_PER_TASK })));
    }

    let id = Uuid::now_v7();
    let key = policy::object_key(ctx.workspace.as_uuid(), task.id, id);
    let upload_url = state
        .storage
        .presign_put(
            &key,
            Duration::from_secs(policy::UPLOAD_TTL_SECONDS.unsigned_abs()),
        )
        .map_err(|error| {
            tracing::error!(%error, "minting the upload URL failed");
            ApiError::internal(&request_id)
        })?;

    let row = attachment::insert(
        &mut scoped,
        &NewAttachment {
            id,
            task_id: task.id,
            object_key: key,
            filename: body.filename.trim().to_owned(),
            byte_size: body.byte_size,
            checksum: body.checksum.clone(),
            uploaded_by: ctx.actor.as_uuid(),
        },
        // The client's DECLARED type, held only until commit.
        //
        // Migration 0006's column comment says "from magic bytes, not the
        // client", and that is what it holds from commit onward — but
        // `docs/28` §Validation also requires commit to reject a "mismatch
        // with the declared type", and the declaration has to survive the two
        // requests in between to be compared against anything.
        //
        // It is safe here for one reason, and only this reason: the row is
        // invisible from the instant it exists (`committed_at IS NULL`), so no
        // read path can serve this value. `commit` overwrites it with the
        // sniffed type before anything can.
        &body.content_type,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "reserving the attachment failed");
        ApiError::internal(&request_id)
    })?;

    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::CREATED,
        axum::Json(PresignResponse {
            attachment_id: row.id,
            upload_url,
            headers: vec![(header::CONTENT_TYPE.to_string(), body.content_type.clone())],
            expires_in: policy::UPLOAD_TTL_SECONDS,
        }),
    )
        .into_response())
}

/// `POST /api/v1/attachments/{id}/commit` — step 3, verify what landed.
///
/// `docs/28`: HEAD the object, check the size, sniff the real type, enqueue the
/// scan, return `202`. It does **not** set `committed_at` — only a `CLEAN` scan
/// does, which is what keeps "the client said it finished" from meaning "the
/// file is safe".
///
/// # Errors
///
/// `404` invisible, `409` when no object was uploaded, `422` on a size or type
/// mismatch.
pub async fn commit(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    // The one read that sees an uncommitted row, by its own name.
    let row = attachment::find_for_commit(&mut scoped, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the attachment failed");
            ApiError::internal(&request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::ATTACHMENT_NOT_FOUND, &request_id))?;

    guard::task_for(
        &mut scoped,
        &ctx,
        row.task_id,
        permission::TASK_ATTACHMENT_CREATE,
        &request_id,
    )
    .await?;

    // Already committed: a retry of a request whose response was lost. Return
    // the current state rather than an error — the client did nothing wrong.
    if row.committed_at.is_some() {
        unit::commit(tx, &request_id).await?;
        return Ok((
            StatusCode::OK,
            axum::Json(CommitResponse {
                attachment_id: row.id,
                scan_status: row.scan_status.clone(),
                content_type: row.content_type.clone(),
            }),
        )
            .into_response());
    }

    let head = state.storage.head(&row.object_key).await.map_err(|error| {
        // Not an internal error: the ordinary case is a client that called
        // commit before its upload finished, or never uploaded at all.
        tracing::debug!(%error, "commit found no object");
        ApiError::missing(codes::ATTACHMENT_NOT_FOUND, &request_id)
    })?;

    if head.byte_size != row.byte_size {
        return Err(ApiError::unprocessable(
            codes::ATTACHMENT_SIZE_MISMATCH,
            "The uploaded object is not the size that was declared",
            &request_id,
        ));
    }

    // The security core: what the bytes ARE, from a bounded prefix. The API
    // never reads the body — `read_prefix` takes a length.
    let prefix = state
        .storage
        .read_prefix(&row.object_key, casual_task_app::attachment::PREFIX)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the object prefix failed");
            ApiError::internal(&request_id)
        })?;
    let sniffed = casual_task_app::attachment::sniff(&prefix);

    let Some(stored_type) = casual_task_app::attachment::stored_type(sniffed) else {
        // Markup. `docs/28`: "A file uploaded as image/png that is actually
        // HTML is rejected — that mismatch is the stored-XSS vector." The
        // object goes immediately; leaving it would leave a reachable file that
        // no row explains.
        let _ = state.storage.delete(&row.object_key).await;
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            codes::ATTACHMENT_TYPE_REFUSED,
            "That file is markup and will not be stored",
            &request_id,
        ));
    };

    // The declared-vs-actual check `docs/28` §Validation requires. `row`'s
    // content_type is still the declaration at this point — see `presign`.
    if !casual_task_app::attachment::agrees(&row.content_type, sniffed) {
        let _ = state.storage.delete(&row.object_key).await;
        return Err(ApiError::unprocessable(
            codes::ATTACHMENT_TYPE_MISMATCH,
            "The uploaded file is not the type that was declared",
            &request_id,
        ));
    }

    let view_before = view(&row);
    attachment::record_verified_type(&mut scoped, row.id, stored_type)
        .await
        .map_err(|error| {
            tracing::error!(%error, "recording the verified type failed");
            ApiError::internal(&request_id)
        })?;

    // ADR-006: the domain change and its history in one transaction. The event
    // is what a scan consumer will claim (D-062's seam).
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "attachment".to_owned(),
            aggregate_id: row.id,
            project_id: Some(row.task_id),
            event_type: "attachment.uploaded".to_owned(),
            activity_changes: serde_json::json!({ "filename": row.filename }),
            audit_changes: serde_json::json!({
                "before": { "content_type": view_before.content_type },
                "after": { "content_type": stored_type },
            }),
            payload: serde_json::json!({
                "attachment_id": row.id,
                "task_id": row.task_id,
                "content_type": stored_type,
            }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the upload failed");
        ApiError::internal(&request_id)
    })?;

    unit::commit(tx, &request_id).await?;

    // 202: verified, not yet visible. docs/28 step 3.
    Ok((
        StatusCode::ACCEPTED,
        axum::Json(CommitResponse {
            attachment_id: row.id,
            scan_status: row.scan_status,
            content_type: stored_type.to_owned(),
        }),
    )
        .into_response())
}

/// `GET /api/v1/attachments/{id}/download` — a redirect to a short-lived URL.
///
/// `docs/28` §Serving downloads: 302 to a pre-signed GET on the **attachment
/// origin**, which is the single most important control here — a stored HTML
/// file that got past every other check still cannot execute in the
/// application's origin.
///
/// # Errors
///
/// `404` invisible, `403` unpermitted, `409` still scanning, `422` infected.
pub async fn download(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let row = guard::downloadable(&mut scoped, &ctx, id, &request_id).await?;
    unit::commit(tx, &request_id).await?;

    let url = state
        .storage
        .presign_get(
            &row.object_key,
            Duration::from_secs(policy::DOWNLOAD_TTL_SECONDS.unsigned_abs()),
        )
        .map_err(|error| {
            tracing::error!(%error, "minting the download URL failed");
            ApiError::internal(&request_id)
        })?;

    let mut response = StatusCode::FOUND.into_response();
    if let Ok(value) = url.parse() {
        response.headers_mut().insert(header::LOCATION, value);
    }
    // Belt and braces on the redirect itself: the object origin sets these too,
    // and a redirect that a proxy decided to cache should not become a sniffing
    // opportunity.
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().expect("static"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "private, no-store".parse().expect("static"),
    );
    Ok(response)
}

/// `GET /api/v1/tasks/{id}/attachments` — the files tab.
///
/// # Errors
///
/// `404` invisible task, `403` without `task.attachment.read`, `400` for a bad
/// page.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    unit::reject_unknown(&params, &["limit", "cursor"], &request_id)?;
    let limit = api_wire::limit(
        params
            .get("limit")
            .map(|raw| {
                raw.parse::<u32>().map_err(|_| {
                    ApiError::bad_request(
                        codes::PAGE_TOO_LARGE,
                        "limit must be a number",
                        &request_id,
                    )
                })
            })
            .transpose()?,
        &request_id,
    )?;
    let after = api_wire::cursor(params.get("cursor").map(String::as_str), &request_id)?;
    let after = after
        .map(|cursor| {
            let key = cursor.keys.first().cloned().unwrap_or_default();
            time::OffsetDateTime::parse(&key, &time::format_description::well_known::Rfc3339)
                .map(|at| (at, cursor.id))
                .map_err(|_| {
                    ApiError::bad_request(codes::BAD_CURSOR, "Malformed cursor", &request_id)
                })
        })
        .transpose()?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let task = guard::task_for(
        &mut scoped,
        &ctx,
        task_id,
        permission::TASK_ATTACHMENT_READ,
        &request_id,
    )
    .await?;

    let mut rows = attachment::list_for_task(&mut scoped, task.id, after, i64::from(limit) + 1)
        .await
        .map_err(|error| {
            tracing::error!(%error, "listing attachments failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = has_more.then(|| rows.last()).flatten().map(|row| {
        casual_task_model::Cursor::new(vec![api_wire::timestamp(row.created_at)], row.id).encode()
    });

    Ok(axum::Json(Paged {
        data: rows.iter().map(view).collect::<Vec<_>>(),
        page: Page {
            next_cursor,
            has_more,
        },
    })
    .into_response())
}
