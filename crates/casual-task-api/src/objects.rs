//! The attachment origin: the only thing that serves file bytes.
//!
//! # Why this is a second router and a second listener
//!
//! `docs/28` §Serving downloads: "User content is served from a **separate
//! origin** from the application. This is the single most important control
//! here: it means a stored HTML or SVG file cannot execute in the application's
//! origin even if every other check fails." `Config::from_source` already
//! refuses a deployment where `TF_ATTACHMENT_ORIGIN` and `TF_PUBLIC_URL` share
//! an origin — so the separation is enforced at startup and this is the half
//! that makes it real: a different port, and therefore a different origin, even
//! on one node.
//!
//! Nothing here touches the database. It is an object store with a lock on it,
//! which is exactly what S3 is in the profile this replaces, and why a
//! deployment can drop this listener and point `TF_ATTACHMENT_ORIGIN` at a
//! bucket without changing a single handler.
//!
//! # The signature is the authority
//!
//! There is no session here, and there must not be: a presigned URL is
//! *capability* authority, minted by an endpoint that already checked
//! `task.attachment.create` or `task.attachment.read`. The cookie would not
//! travel to another origin anyway. What that means in practice:
//!
//! - the signature covers the **method**, so a read capability cannot write;
//! - it covers the **expiry**, and an expired URL is refused before the disk is
//!   touched;
//! - the comparison is constant-time (`FilesystemStore::verify`);
//! - object keys are `{workspace}/{task}/{attachment}` UUIDs, so a URL is not
//!   guessable and a leaked one expires.
//!
//! # Why the responses are so defensive for a file the caller already asked for
//!
//! `Content-Disposition: attachment` and `X-Content-Type-Options: nosniff` are
//! `docs/28`'s requirement, and they are the in-process half of the origin
//! control: even served from the right origin, a browser that sniffed an
//! uploaded file as HTML would render it. The `Content-Security-Policy` goes
//! further and forbids executing anything at all, which costs nothing for a
//! download and closes the case where a deployment misconfigures its origins.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use casual_task_infra::FilesystemStore;
use serde::Deserialize;

/// A hard ceiling on an upload body.
///
/// A backstop, not the rule: the real limit is the workspace's, checked at
/// pre-sign, and `commit` refuses an object whose size is not the one that was
/// declared. This exists because the object server has no database and
/// therefore cannot know that number — without it a signed URL would accept
/// bytes until the disk filled.
const MAX_UPLOAD: usize = 2 * 1024 * 1024 * 1024;

/// The query a presigned URL carries.
#[derive(Debug, Deserialize)]
struct Signature {
    expires: i64,
    signature: String,
}

#[derive(Clone)]
struct ObjectState {
    store: Arc<FilesystemStore>,
    secret: Arc<str>,
}

/// The attachment origin's router.
///
/// Mounted on its own listener by `main`, never on the application's — see the
/// module documentation.
pub fn object_router(store: Arc<FilesystemStore>, secret: &str) -> Router {
    Router::new()
        .route("/attachments/{*key}", get(fetch).put(store_object))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD))
        .with_state(ObjectState {
            store,
            secret: Arc::from(secret),
        })
}

/// `PUT /attachments/{key}?expires=&signature=` — the upload half of `docs/28`.
///
/// The bytes are written and nothing else happens: no row moves, no scan is
/// queued, no event is recorded. `POST /attachments/{id}/commit` is what turns
/// an object into an attachment, and it re-derives everything it needs from the
/// bytes rather than trusting anything said here.
async fn store_object(
    State(state): State<ObjectState>,
    Path(key): Path<String>,
    Query(query): Query<Signature>,
    body: Body,
) -> Response {
    if let Some(refusal) = refused(&state, &key, &query, "PUT") {
        return refusal;
    }

    let bytes = match axum::body::to_bytes(body, MAX_UPLOAD).await {
        Ok(bytes) => bytes,
        // The body limit, or a client that hung up mid-upload. Either way there
        // is nothing to store and the caller should retry rather than commit.
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "upload too large").into_response(),
    };

    // Truncating, not appending: a retried PUT must leave the object as the
    // sender meant it. `append` would double the bytes, and the only thing that
    // would notice is `commit`'s size check — a confusing refusal for a client
    // that did the right thing.
    match state.store.replace(&key, &bytes).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => {
            tracing::error!(%error, "storing an object failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /attachments/{key}?expires=&signature=` — the download half.
///
/// Streams from disk rather than reading the file into the process: a 2 GiB
/// attachment must not become 2 GiB of resident memory, which is the same
/// reason `ObjectStore::read_prefix` takes a length.
async fn fetch(
    State(state): State<ObjectState>,
    Path(key): Path<String>,
    Query(query): Query<Signature>,
) -> Response {
    if let Some(refusal) = refused(&state, &key, &query, "GET") {
        return refusal;
    }

    let Ok(path) = state.store.resolve(&key) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(file) = tokio::fs::File::open(&path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mut response = Body::from_stream(chunks(file)).into_response();
    let headers = response.headers_mut();
    guard(headers);
    // `application/octet-stream` and never the stored type: the browser is being
    // handed a file to save, and naming a renderable type here would invite it
    // to render one. The real type is on the attachment row, which the
    // application serves and this origin does not read.
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response
}

/// How much of a file is read at a time. 64 KiB is the usual pipe buffer, so a
/// larger chunk buys nothing and a smaller one costs syscalls.
const CHUNK: usize = 64 * 1024;

/// A file as a stream of chunks.
///
/// # Why this is hand-rolled
///
/// `tokio_util::io::ReaderStream` does exactly this, and pulling in
/// `tokio-util` for one adapter is a dependency — and a licence, an audit and a
/// supply-chain surface — for fifteen lines. `futures-core` is already here for
/// the `Stream` trait the SSE endpoint needs, and `tokio`'s `sync` feature is
/// already on. Reading the whole file instead was the other option, and a 2 GiB
/// attachment would then be 2 GiB of resident memory.
fn chunks(mut file: tokio::fs::File) -> ChunkStream {
    let (sender, receiver) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        loop {
            let mut buffer = vec![0_u8; CHUNK];
            match file.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => {
                    buffer.truncate(read);
                    // The receiver is gone: the client hung up mid-download, so
                    // stop reading rather than filling a channel nobody drains.
                    if sender.send(Ok(buffer)).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error)).await;
                    break;
                }
            }
        }
    });
    ChunkStream(receiver)
}

struct ChunkStream(tokio::sync::mpsc::Receiver<Result<Vec<u8>, std::io::Error>>);

impl futures_core::Stream for ChunkStream {
    type Item = Result<Vec<u8>, std::io::Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.get_mut().0.poll_recv(cx)
    }
}

/// The response to send instead, or `None` when the capability is good.
///
/// `Option` rather than `Result`: a `Response` is a large value, and a
/// `Result<(), Response>` makes every success carry the space for one.
fn refused(state: &ObjectState, key: &str, query: &Signature, method: &str) -> Option<Response> {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let good = FilesystemStore::verify(
        &state.secret,
        key,
        query.expires,
        method,
        &query.signature,
        now,
    )
    .is_ok();
    if good {
        return None;
    }
    // One answer for expired, forged, and wrong-method. Distinguishing them
    // would tell a probe which part of a URL to keep guessing at, and none of
    // the three is a case a legitimate client can act on differently: ask the
    // application for a new URL.
    Some((StatusCode::FORBIDDEN, "invalid or expired signature").into_response())
}

/// The headers `docs/28` requires on anything this origin returns.
fn guard(headers: &mut HeaderMap) {
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // Belt and braces for a deployment that got its origins wrong: nothing
    // served here may execute, frame, or fetch anything.
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_response_from_this_origin_is_a_download_and_never_a_page() {
        // The control `docs/28` calls the most important one, asserted on the
        // headers rather than trusted to a comment: a stored HTML file that got
        // past the sniffer must still not render.
        let mut headers = HeaderMap::new();
        guard(&mut headers);
        assert_eq!(headers[header::CONTENT_DISPOSITION], "attachment");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert!(
            headers[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap_or_default()
                .contains("default-src 'none'"),
        );
    }

    #[test]
    fn a_read_capability_does_not_authorize_a_write() {
        // The signature covers the method for exactly this reason. A URL handed
        // out to download a file must not accept a replacement of it.
        let secret = "a-test-secret-key-long-enough";
        let key = "ws/task/attachment";
        let expires = i64::MAX / 2;
        let for_get = FilesystemStore::sign(secret, key, expires, "GET");
        assert!(FilesystemStore::verify(secret, key, expires, "GET", &for_get, 0).is_ok());
        assert!(
            FilesystemStore::verify(secret, key, expires, "PUT", &for_get, 0).is_err(),
            "a GET signature authorized a PUT"
        );
    }

    #[test]
    fn an_expired_signature_is_refused_even_though_it_is_genuine() {
        let secret = "a-test-secret-key-long-enough";
        let key = "ws/task/attachment";
        let signature = FilesystemStore::sign(secret, key, 100, "GET");
        assert!(FilesystemStore::verify(secret, key, 100, "GET", &signature, 99).is_ok());
        assert!(FilesystemStore::verify(secret, key, 100, "GET", &signature, 101).is_err());
    }
}
