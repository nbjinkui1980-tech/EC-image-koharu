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
use axum::middleware;
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;

use crate::AppState;
use crate::api;
use crate::security::{OriginHostPolicy, SecurityContext};

/// Function that maps a URL path (e.g. `"/index.html"`) to `(bytes, mime)`.
/// Returning `None` signals a 404 fall-through.
pub type AssetResolver = Arc<dyn Fn(&str) -> Option<(Vec<u8>, String)> + Send + Sync>;

fn with_origin_host_policy(router: Router, policy: OriginHostPolicy) -> Router {
    router
        .layer(middleware::from_fn(crate::security::enforce_origin_host))
        .layer(axum::Extension(policy))
}

fn complete_router(
    app: AppState,
    security: SecurityContext,
    session: Option<crate::security::BrowserSessionState>,
) -> Router {
    let api = match session {
        Some(session) => api::router_with_session(app.clone(), security.clone(), session),
        None => api::router(app.clone(), security.clone()),
    };
    crate::mcp::mount(api, app, security)
}

/// Wrap the protected router with origin/host policy + mount MCP.
pub fn router_for(app: AppState, security: SecurityContext, policy: OriginHostPolicy) -> Router {
    with_origin_host_policy(complete_router(app, security, None), policy)
}

/// Router for browser clients that can exchange a bootstrap credential for a session cookie.
pub fn router_for_with_session(
    app: AppState,
    security: SecurityContext,
    policy: OriginHostPolicy,
    session: crate::security::BrowserSessionState,
) -> Router {
    with_origin_host_policy(complete_router(app, security, Some(session)), policy)
}

/// Same as `router_for` but installs `resolver` as a fallback.
pub fn router_with_assets(
    app: AppState,
    security: SecurityContext,
    policy: OriginHostPolicy,
    resolver: AssetResolver,
) -> Router {
    let complete = complete_router(app, security, None).fallback(move |req: Request<Body>| {
        let resolver = resolver.clone();
        async move { serve_asset(resolver, req).await }
    });
    with_origin_host_policy(complete, policy)
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
    policy: OriginHostPolicy,
) -> Result<()> {
    axum::serve(listener, router_for(app, security, policy)).await?;
    Ok(())
}

/// Serve HTTP with browser-session authentication enabled.
pub async fn serve_with_listener_with_session(
    listener: TcpListener,
    app: AppState,
    security: SecurityContext,
    policy: OriginHostPolicy,
    session: crate::security::BrowserSessionState,
) -> Result<()> {
    axum::serve(
        listener,
        router_for_with_session(app, security, policy, session),
    )
    .await?;
    Ok(())
}

/// Variant with embedded assets fallback.
pub async fn serve_with_listener_and_assets(
    listener: TcpListener,
    app: AppState,
    security: SecurityContext,
    policy: OriginHostPolicy,
    resolver: AssetResolver,
) -> Result<()> {
    axum::serve(
        listener,
        router_with_assets(app, security, policy, resolver),
    )
    .await?;
    Ok(())
}

/// Serve embedded assets with browser-session authentication enabled.
pub async fn serve_with_listener_and_assets_with_session(
    listener: TcpListener,
    app: AppState,
    security: SecurityContext,
    policy: OriginHostPolicy,
    session: crate::security::BrowserSessionState,
    resolver: AssetResolver,
) -> Result<()> {
    axum::serve(
        listener,
        with_origin_host_policy(
            complete_router(app, security, Some(session)).fallback(move |req: Request<Body>| {
                let resolver = resolver.clone();
                async move { serve_asset(resolver, req).await }
            }),
            policy,
        ),
    )
    .await?;
    Ok(())
}
