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

/// CSP delivered with HTML responses and mirrored in `tauri.conf.json`. The
/// first five directives are the SPEC AR-07 frozen baseline; the rest are the
/// minimal allowances the UI needs (inline Next bootstrap scripts, React
/// inline styles, blob/data images and fonts, Sentry egress).
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' https://*.sentry.io https://*.ingest.sentry.io";

fn enforce_origin_host(router: Router, policy: OriginHostPolicy) -> Router {
    router
        .layer(middleware::from_fn(crate::security::enforce_origin_host))
        .layer(axum::Extension(policy))
}

/// Wrap the protected router with origin/host policy + mount MCP.
pub fn router_for(app: AppState, security: SecurityContext, policy: OriginHostPolicy) -> Router {
    enforce_origin_host(
        crate::mcp::mount(api::router(app.clone(), security.clone()), app, security),
        policy,
    )
}

/// Router for browser clients that can exchange a bootstrap credential for a session cookie.
pub fn router_for_with_session(
    app: AppState,
    security: SecurityContext,
    policy: OriginHostPolicy,
    session: crate::security::BrowserSessionState,
) -> Router {
    enforce_origin_host(
        crate::mcp::mount(
            api::router_with_session(app.clone(), security.clone(), session),
            app,
            security,
        ),
        policy,
    )
}

/// Same as `router_for` but installs `resolver` as a fallback.
pub fn router_with_assets(
    app: AppState,
    security: SecurityContext,
    policy: OriginHostPolicy,
    resolver: AssetResolver,
) -> Router {
    let router = crate::mcp::mount(api::router(app.clone(), security.clone()), app, security)
        .fallback(move |req: Request<Body>| {
            let resolver = resolver.clone();
            async move { serve_asset(resolver, req).await }
        });
    enforce_origin_host(router, policy)
}

fn router_with_assets_with_session(
    app: AppState,
    security: SecurityContext,
    policy: OriginHostPolicy,
    session: crate::security::BrowserSessionState,
    resolver: AssetResolver,
) -> Router {
    let router = crate::mcp::mount(
        api::router_with_session(app.clone(), security.clone(), session),
        app,
        security,
    )
    .fallback(move |req: Request<Body>| {
        let resolver = resolver.clone();
        async move { serve_asset(resolver, req).await }
    });
    enforce_origin_host(router, policy)
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
        if mime == "text/html" {
            resp.headers_mut().insert(
                axum::http::header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(CONTENT_SECURITY_POLICY),
            );
        }
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
        router_with_assets_with_session(app, security, policy, session, resolver),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod csp_tests {
    use super::*;

    fn test_resolver() -> AssetResolver {
        Arc::new(|path: &str| match path {
            "index.html" => Some((
                b"<html><head></head><body>app</body></html>".to_vec(),
                "text/html".to_string(),
            )),
            "app.js" => Some((b"content".to_vec(), "text/javascript".to_string())),
            _ => None,
        })
    }

    // AR07-T01 RED: HTML responses must carry the SPEC AR-07 frozen CSP
    // directives; today serve_asset sets only Content-Type.
    #[tokio::test]
    async fn csp_html_response_carries_frozen_directives() {
        let response = serve_asset(
            test_resolver(),
            Request::builder().uri("/").body(Body::empty()).unwrap(),
        )
        .await;
        let csp = response
            .headers()
            .get(axum::http::header::CONTENT_SECURITY_POLICY)
            .and_then(|value| value.to_str().ok())
            .expect("HTML response must carry a Content-Security-Policy header");
        for directive in [
            "default-src 'self'",
            "object-src 'none'",
            "base-uri 'none'",
            "frame-ancestors 'none'",
            "form-action 'none'",
        ] {
            assert!(
                csp.contains(directive),
                "missing frozen directive: {directive}"
            );
        }
    }

    // Lock: non-HTML assets keep serving normally.
    #[tokio::test]
    async fn csp_non_html_asset_serves_normally() {
        let response = serve_asset(
            test_resolver(),
            Request::builder()
                .uri("/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}
