//! The three endpoints `docs/38` §Export is a job, not a request specifies.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_persistence::export as store;
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::exports::wire::{CreateRequest, ExportView};
use crate::json::ValidJson;
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;

/// How long a download URL is valid (`docs/38`: "Signed download URL, 1 hour").
const DOWNLOAD_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

/// `POST /api/v1/exports` → `202 Accepted`.
///
/// # Errors
///
/// `400` for an unknown format, an unknown column, or a filter the grammar
/// rejects; `500` on a database failure.
pub async fn create(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    ValidJson(body): ValidJson<CreateRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    // Everything refusable is refused HERE, not an hour later in a worker.
    let format = casual_task_worker::export::Format::parse(&body.format).ok_or_else(|| {
        ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "format must be csv or jsonl",
            &request_id,
        )
    })?;

    let columns = match &body.columns {
        None => None,
        Some(requested) => {
            for name in requested {
                if casual_task_worker::export::Column::parse(name).is_none() {
                    return Err(ApiError::bad_request(
                        codes::UNKNOWN_FIELD,
                        "unknown export column",
                        &request_id,
                    )
                    .with_details(serde_json::json!({ "field": name })));
                }
            }
            Some(serde_json::json!(requested))
        }
    };

    // The filter is parsed and validated now, through the list endpoint's own
    // parser, so a malformed query is a 400 the user sees immediately rather
    // than a failed job they discover later.
    let pairs: Vec<(String, String)> = body
        .filter
        .split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    casual_task_search::parse_url(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str()))).map_err(
        |error| {
            ApiError::bad_request(
                crate::error::Code::from_registry(error.code()),
                "The query could not be understood",
                &request_id,
            )
            .with_details(serde_json::json!({ "field": error.field() }))
        },
    )?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let job = store::insert(
        &mut scoped,
        &store::NewJob {
            id: Uuid::now_v7(),
            requested_by: ctx.actor.as_uuid(),
            filter_query: body.filter.clone(),
            format: format.as_str().to_owned(),
            columns,
        },
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "queueing an export failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    // 202, not 201: docs/05 reserves 201 for a resource that now exists in the
    // state the caller asked for. An export exists as an intention.
    Ok((StatusCode::ACCEPTED, axum::Json(ExportView::of(&job))).into_response())
}

/// `GET /api/v1/exports/{id}`.
///
/// # Errors
///
/// `404` when the export does not exist, belongs to another workspace, or was
/// requested by someone else.
pub async fn read(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let job = mine(&mut scoped, &ctx, id, &request_id).await?;
    unit::commit(tx, &request_id).await?;

    Ok(axum::Json(ExportView::of(&job)).into_response())
}

/// `GET /api/v1/exports/{id}/download` → `302` to a signed URL.
///
/// # Errors
///
/// `404` as above; `409` when the export is not finished, because "not ready"
/// is a different fact from "not yours" and a client can act on it.
pub async fn download(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;
    let job = mine(&mut scoped, &ctx, id, &request_id).await?;
    unit::commit(tx, &request_id).await?;

    let Some(key) = job.object_key.filter(|_| job.status == "succeeded") else {
        return Err(ApiError::conflict(
            codes::EXPORT_NOT_READY,
            "The export is not ready to download",
            &request_id,
        ));
    };

    // Redirect to the storage origin, never stream through here: docs/28's rule
    // for attachments, and the same reason applies — an export can be hundreds
    // of megabytes and the API process must not carry it.
    let url = state
        .storage
        .presign_get(&key, DOWNLOAD_TTL)
        .map_err(|error| {
            tracing::error!(%error, "signing an export download failed");
            ApiError::internal(&request_id)
        })?;
    Ok((StatusCode::FOUND, [(header::LOCATION, url)]).into_response())
}

/// The caller's own export, or `404`.
///
/// Requester-only, deliberately. `docs/38` does not say who may download an
/// artefact, and an export is bulk tenant data sitting behind one URL — so the
/// conservative reading is that it belongs to the person who asked for it. A
/// workspace admin who needs someone else's export can run the same filter.
///
/// Absent and not-yours return the same `404`, per `docs/04`.
async fn mine(
    scoped: &mut casual_task_persistence::Scoped<'_>,
    ctx: &Context,
    id: Uuid,
    request_id: &str,
) -> Result<store::JobRow, ApiError> {
    let job = store::read(scoped, id).await.map_err(|error| {
        tracing::error!(%error, "reading an export failed");
        ApiError::internal(request_id)
    })?;
    job.filter(|job| job.requested_by == ctx.actor.as_uuid())
        .ok_or_else(|| ApiError::missing(codes::NOT_FOUND, request_id))
}
