//! The attachment origin, end to end (C-010, `docs/28` §Serving downloads).
//!
//! # Why this suite needs no database
//!
//! That is the point of it. The attachment origin is an object store with a lock
//! on it: no session, no workspace, no rows. Its whole contract is the
//! signature, and a test that had to sign in to reach it would be testing
//! something else.
//!
//! # What is actually being asserted
//!
//! Not "a file comes back" — that is the easy half. The properties that matter
//! are the ones a bug would leave working:
//!
//! - a read capability must not authorize a write, or a shared download link
//!   becomes an upload slot;
//! - an expired URL must be refused even though the signature is genuine;
//! - a forged signature must be refused without saying which part was wrong;
//! - a key that climbs out of the root must not resolve, or `../../etc/passwd`
//!   is readable through a signed URL;
//! - every response must carry `Content-Disposition: attachment` and
//!   `nosniff`, which is what stops a stored HTML file rendering.

use std::sync::Arc;

use anyhow::Result;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use casual_task_api::objects::object_router;

/// The application's origin — the only one a browser may upload from. Distinct
/// from the attachment origin below, which is the separation `docs/28` rests on.
const APP_ORIGIN: &str = "http://127.0.0.1:5173";
use casual_task_infra::FilesystemStore;
use tower::ServiceExt;

const SECRET: &str = "an-object-origin-secret-long-enough";

/// A store over a fresh temporary directory, and the router in front of it.
fn origin() -> (Arc<FilesystemStore>, axum::Router, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("tf-objects-{}", uuid::Uuid::now_v7()));
    let store = Arc::new(FilesystemStore::new(
        root.clone(),
        "http://127.0.0.1:8081".to_owned(),
        SECRET.to_owned(),
    ));
    let router = object_router(Arc::clone(&store), SECRET, APP_ORIGIN);
    (store, router, root)
}

/// A URL for `key`, signed for `method`, valid for an hour.
fn signed(key: &str, method: &str) -> String {
    let expires = time::OffsetDateTime::now_utc().unix_timestamp() + 3600;
    let signature = FilesystemStore::sign(SECRET, key, expires, method);
    format!("/attachments/{key}?expires={expires}&signature={signature}")
}

async fn send(
    router: &axum::Router,
    request: Request<Body>,
) -> Result<(StatusCode, Vec<u8>, axum::http::HeaderMap)> {
    let response = router.clone().oneshot(request).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024).await?;
    Ok((status, bytes.to_vec(), headers))
}

#[tokio::test]
async fn a_signed_put_stores_bytes_that_a_signed_get_returns() -> Result<()> {
    let (_store, router, root) = origin();
    let key = "ws/task/attachment";

    let (status, _, _) = send(
        &router,
        Request::builder()
            .method("PUT")
            .uri(signed(key, "PUT"))
            .body(Body::from("hello attachment"))?,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    let (status, body, headers) = send(
        &router,
        Request::builder()
            .uri(signed(key, "GET"))
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8_lossy(&body), "hello attachment");

    // `docs/28`: every response from this origin is a download, never a page.
    assert_eq!(headers[header::CONTENT_DISPOSITION], "attachment");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::CONTENT_TYPE], "application/octet-stream");

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn a_retried_upload_replaces_rather_than_appends() -> Result<()> {
    // A client that retries a PUT whose response it never saw is doing the right
    // thing. Appending would double the bytes, and the only thing that would
    // notice is `commit`'s size check — a refusal for a correct client.
    let (_store, router, root) = origin();
    let key = "ws/task/retried";

    for _ in 0..3 {
        let (status, _, _) = send(
            &router,
            Request::builder()
                .method("PUT")
                .uri(signed(key, "PUT"))
                .body(Body::from("once"))?,
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
    }

    let (_, body, _) = send(
        &router,
        Request::builder()
            .uri(signed(key, "GET"))
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(
        String::from_utf8_lossy(&body),
        "once",
        "the retries accumulated"
    );

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn a_download_capability_does_not_authorize_an_upload() -> Result<()> {
    // The signature covers the method for exactly this reason: a link handed out
    // to read a file must not accept a replacement of it.
    let (_store, router, root) = origin();
    let key = "ws/task/readonly";

    let (status, _, _) = send(
        &router,
        Request::builder()
            .method("PUT")
            .uri(signed(key, "GET"))
            .body(Body::from("replaced"))?,
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn an_expired_url_is_refused_even_though_the_signature_is_genuine() -> Result<()> {
    let (_store, router, root) = origin();
    let key = "ws/task/expired";
    let expires = time::OffsetDateTime::now_utc().unix_timestamp() - 1;
    let signature = FilesystemStore::sign(SECRET, key, expires, "GET");

    let (status, _, _) = send(
        &router,
        Request::builder()
            .uri(format!(
                "/attachments/{key}?expires={expires}&signature={signature}"
            ))
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn a_forged_signature_and_a_tampered_expiry_are_both_refused() -> Result<()> {
    let (_store, router, root) = origin();
    let key = "ws/task/forged";
    let expires = time::OffsetDateTime::now_utc().unix_timestamp() + 3600;
    let genuine = FilesystemStore::sign(SECRET, key, expires, "GET");

    for uri in [
        format!("/attachments/{key}?expires={expires}&signature=deadbeef"),
        // The expiry is inside the signature, so extending it invalidates it.
        format!(
            "/attachments/{key}?expires={}&signature={genuine}",
            expires + 86_400
        ),
        // A signature minted for a different key.
        format!(
            "/attachments/{key}?expires={expires}&signature={}",
            FilesystemStore::sign(SECRET, "ws/task/other", expires, "GET")
        ),
    ] {
        let (status, _, _) =
            send(&router, Request::builder().uri(&uri).body(Body::empty())?).await?;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
    }

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn a_request_with_no_signature_at_all_is_refused() -> Result<()> {
    // The query is required by the extractor, so this is a 400 rather than a
    // 403 — what matters is that it is not a 200.
    let (_store, router, root) = origin();
    let (status, _, _) = send(
        &router,
        Request::builder()
            .uri("/attachments/ws/task/bare")
            .body(Body::empty())?,
    )
    .await?;
    assert_ne!(status, StatusCode::OK);

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn a_key_that_climbs_out_of_the_root_is_refused_even_when_signed() -> Result<()> {
    // The signature says the *caller* is allowed; it says nothing about whether
    // the key is inside the store. Without the traversal guard, anyone who could
    // mint a URL could read any file the process can.
    let (_store, router, root) = origin();
    let key = "../../etc/passwd";
    let expires = time::OffsetDateTime::now_utc().unix_timestamp() + 3600;
    let signature = FilesystemStore::sign(SECRET, key, expires, "GET");

    let (status, _, _) = send(
        &router,
        Request::builder()
            .uri(format!(
                "/attachments/{key}?expires={expires}&signature={signature}"
            ))
            .body(Body::empty())?,
    )
    .await?;
    assert_ne!(status, StatusCode::OK, "a traversing key was served");

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn a_missing_object_is_404_and_not_an_error() -> Result<()> {
    let (_store, router, root) = origin();
    let (status, _, _) = send(
        &router,
        Request::builder()
            .uri(signed("ws/task/never-uploaded", "GET"))
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

/// # The upload half was unreachable from a browser
///
/// These cover the defect that made `docs/28`'s pipeline complete on the server
/// and unusable from the product. The application and the attachment origin are
/// *deliberately* different origins — that separation is the control the module
/// exists for — and a browser `PUT` carrying `Content-Type` is therefore not a
/// simple request. It asks first, with `OPTIONS`, and a router with no such
/// route answered `405`. Presign returned a URL, the client tried to use it, and
/// the preflight failed before a byte moved.
mod cross_origin {
    use super::*;

    #[tokio::test]
    async fn a_preflight_is_answered_with_the_application_origin() -> Result<()> {
        let (_store, router, _root) = origin();
        let (status, _body, headers) = send(
            &router,
            Request::builder()
                .method("OPTIONS")
                .uri(signed("w/t/a", "PUT"))
                .header("origin", APP_ORIGIN)
                .header("access-control-request-method", "PUT")
                .body(Body::empty())?,
        )
        .await?;

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(headers["access-control-allow-origin"], APP_ORIGIN);
        assert!(
            headers["access-control-allow-methods"]
                .to_str()?
                .contains("PUT"),
            "the preflight must permit the method the upload uses",
        );
        assert_eq!(headers["access-control-allow-headers"], "content-type");
        Ok(())
    }

    #[tokio::test]
    async fn one_origin_and_never_a_wildcard() -> Result<()> {
        // `*` would let any page that obtained a presigned URL spend it. The
        // capability is already narrow — one key, one method, minutes of life —
        // but "narrow" and "anyone may use it" are different properties.
        let (_store, router, _root) = origin();
        let (_status, _body, headers) = send(
            &router,
            Request::builder()
                .method("OPTIONS")
                .uri(signed("w/t/a", "PUT"))
                .header("origin", "https://evil.example")
                .body(Body::empty())?,
        )
        .await?;

        assert_eq!(headers["access-control-allow-origin"], APP_ORIGIN);
        assert_ne!(headers["access-control-allow-origin"], "*");
        Ok(())
    }

    #[tokio::test]
    async fn the_upload_response_carries_the_headers_too() -> Result<()> {
        // Not only the preflight: a browser discards a cross-origin *response*
        // that lacks them, so the bytes would land on disk and the client would
        // still see a failure.
        let (_store, router, _root) = origin();
        let (status, _body, headers) = send(
            &router,
            Request::builder()
                .method("PUT")
                .uri(signed("w/t/a", "PUT"))
                .header("origin", APP_ORIGIN)
                .body(Body::from("bytes"))?,
        )
        .await?;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["access-control-allow-origin"], APP_ORIGIN);
        Ok(())
    }

    #[tokio::test]
    async fn the_preflight_is_not_a_way_past_the_signature() -> Result<()> {
        // It answers before any signature check, which is correct — a preflight
        // carries none, and the browser has not sent the real request yet. What
        // must stay true is that this buys nothing: the PUT it precedes is still
        // refused without a valid signature.
        let (_store, router, _root) = origin();
        let (status, _body, _headers) = send(
            &router,
            Request::builder()
                .method("PUT")
                .uri("/attachments/w/t/a?expires=99999999999&signature=forged")
                .header("origin", APP_ORIGIN)
                .body(Body::from("bytes"))?,
        )
        .await?;

        assert_ne!(
            status,
            StatusCode::OK,
            "a forged signature must not store bytes"
        );
        Ok(())
    }
}
