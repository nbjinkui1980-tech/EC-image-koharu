//! Server bootstrap — attaches the router to an axum listener.
//!
//! Also exposes an `AssetResolver` hook so the Tauri binary can bolt its
//! embedded frontend onto unmatched routes.

use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header::CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

use crate::AppState;
use crate::api;
use crate::security::SecurityContext;

/// Function that maps a URL path (e.g. `"/index.html"`) to `(bytes, mime)`.
/// Returning `None` signals a 404 fall-through.
pub type AssetResolver = Arc<dyn Fn(&str) -> Option<(Vec<u8>, String)> + Send + Sync>;

/// Wrap the protected router with CORS + mount MCP.
pub fn router_for(app: AppState, security: SecurityContext) -> Router {
    let base = api::router(app.clone(), security.clone()).layer(CorsLayer::very_permissive());
    crate::mcp::mount(base, app)
}

/// Same as `router_for` but installs `resolver` as a fallback.
pub fn router_with_assets(
    app: AppState,
    security: SecurityContext,
    resolver: AssetResolver,
) -> Router {
    router_for(app, security).fallback(move |req: Request<Body>| {
        let resolver = resolver.clone();
        async move { serve_asset(resolver, req).await }
    })
}

async fn serve_asset(resolver: AssetResolver, req: Request<Body>) -> Response {
    if req.method() != axum::http::Method::GET {
        return (StatusCode::METHOD_NOT_ALLOWED, "method not allowed").into_response();
    }
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some((bytes, mime)) = resolver(path)
        && let Ok(header) = HeaderValue::from_str(&mime)
    {
        let mut resp = Response::new(Body::from(bytes));
        resp.headers_mut().insert(CONTENT_TYPE, header);
        return resp;
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
}

/// Serve HTTP on an already-bound listener with required auth.
pub async fn serve_with_listener(
    listener: TcpListener,
    app: AppState,
    security: SecurityContext,
) -> Result<()> {
    axum::serve(listener, router_for(app, security)).await?;
    Ok(())
}

/// Variant with embedded assets fallback.
pub async fn serve_with_listener_and_assets(
    listener: TcpListener,
    app: AppState,
    security: SecurityContext,
    resolver: AssetResolver,
) -> Result<()> {
    axum::serve(listener, router_with_assets(app, security, resolver)).await?;
    Ok(())
}
