//! Compiled browser assets and the constrained history fallback (ADR-034).
//!
//! This module prevents an SPA fallback from turning a missing API route or
//! JavaScript asset into successful HTML. Static delivery changes for caching
//! and deployment reasons; it does not belong in the API route inventory.

use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use tower::ServiceBuilder;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use super::AppState;

#[derive(Debug)]
pub(super) struct WebAssets {
    root: PathBuf,
    index: Bytes,
}

impl WebAssets {
    /// Read the entry document at startup so an incomplete image cannot start.
    ///
    /// # Errors
    ///
    /// When `root/index.html` cannot be read.
    pub(super) fn load(root: &Path) -> Result<Self, std::io::Error> {
        Ok(Self {
            root: root.to_path_buf(),
            index: Bytes::from(std::fs::read(root.join("index.html"))?),
        })
    }
}

pub(super) fn attach(routes: Router<AppState>, web: WebAssets) -> Router<AppState> {
    let immutable = HeaderValue::from_static("public, max-age=31536000, immutable");
    let short_cache = HeaderValue::from_static("public, max-age=3600");
    let assets = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            immutable,
        ))
        .service(ServeDir::new(web.root.join("assets")));
    let brand = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            short_cache.clone(),
        ))
        .service(ServeDir::new(web.root.join("brand")));
    let favicon = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            short_cache,
        ))
        .service(ServeFile::new(web.root.join("favicon.svg")));
    let index = web.index;

    routes
        .nest_service("/assets", assets)
        .nest_service("/brand", brand)
        .route_service("/favicon.svg", favicon)
        .fallback(move |method: Method, headers: HeaderMap, uri: Uri| {
            let index = index.clone();
            async move { spa_fallback(method, headers, uri, index) }
        })
}

fn spa_fallback(method: Method, headers: HeaderMap, uri: Uri, index: Bytes) -> Response {
    let path = uri.path();
    let reserved = path == "/metrics" || path.starts_with("/api/") || path.starts_with("/health/");
    let looks_like_asset = path
        .rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'));
    let accepts_html = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().starts_with("text/html"))
        });

    if reserved
        || looks_like_asset
        || !accepts_html
        || !matches!(method, Method::GET | Method::HEAD)
    {
        return StatusCode::NOT_FOUND.into_response();
    }

    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(index)
    };
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
        .into_response()
}
