//! Axum router assembly + OpenAPI descriptor.
//!
//! Each domain registers its routes; this module stitches them into one
//! `OpenApiRouter<ApiState>` + the OpenAPI doc for static export.

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use utoipa_axum::router::OpenApiRouter;

use crate::AppState;
use crate::routes;
use crate::{binary, events};

const MAX_BODY_SIZE: usize = 1024 * 1024 * 1024;

/// State threaded through every `State<ApiState>` extractor.
pub type ApiState = AppState;

fn bootstrap_api() -> OpenApiRouter<ApiState> {
    OpenApiRouter::default()
        .merge(routes::downloads::router())
        .merge(routes::operations::router())
        .merge(events::router())
}

fn app_api() -> OpenApiRouter<ApiState> {
    OpenApiRouter::default()
        .merge(routes::history::router())
        .merge(routes::pages::router())
        .merge(routes::projects::router())
        .merge(routes::config::router())
        .merge(routes::meta::router())
        .merge(routes::fonts::router())
        .merge(routes::llm::router())
        .merge(routes::ai::router())
        .merge(routes::pipelines::router())
        .merge(binary::router())
}

/// Build the router + OpenAPI doc. Called by the bin and by `router()`.
pub fn api() -> (Router<ApiState>, utoipa::openapi::OpenApi) {
    bootstrap_api().merge(app_api()).split_for_parts()
}

async fn require_ready(State(app): State<ApiState>, request: Request, next: Next) -> Response {
    if !app.is_ready() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            crate::ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "app is still bootstrapping",
            ),
        )
            .into_response();
    }
    next.run(request).await
}

const CONTROL_BODY_LIMIT: usize = 1024 * 1024;
const MASK_BODY_LIMIT: usize = 64 * 1024 * 1024;
const ARCHIVE_BODY_LIMIT: usize = 512 * 1024 * 1024;

/// Route-tier body limits: control 1 MiB, mask uploads 64 MiB, archive import
/// 512 MiB. Inserted per request as a `DefaultBodyLimit` extension, which the
/// Bytes/Json extractors honor. The Multipart extractor ignores it — the
/// multipart import budget lives in routes/pages.rs instead.
async fn tiered_body_limit(mut request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let limit = if path == "/api/v1/projects/import" {
        ARCHIVE_BODY_LIMIT
    } else if path.starts_with("/api/v1/pages/") && path.contains("/masks/") {
        MASK_BODY_LIMIT
    } else {
        CONTROL_BODY_LIMIT
    };
    DefaultBodyLimit::max(limit).apply(&mut request);
    next.run(request).await
}

/// Ready-to-serve router with required authentication.
pub fn router(app: ApiState, security: crate::security::SecurityContext) -> Router {
    router_inner(app, security, None)
}

/// Router that accepts either master Bearer or a valid browser session cookie.
pub fn router_with_session(
    app: ApiState,
    security: crate::security::SecurityContext,
    session: crate::security::BrowserSessionState,
) -> Router {
    router_inner(app, security, Some(session))
}

fn router_inner(
    app: ApiState,
    security: crate::security::SecurityContext,
    session: Option<crate::security::BrowserSessionState>,
) -> Router {
    let (bootstrap, _) = bootstrap_api().split_for_parts();
    let (guarded, _) = app_api().split_for_parts();
    let bootstrap = bootstrap.with_state(app.clone());
    let guarded = guarded
        .with_state(app.clone())
        .layer(middleware::from_fn_with_state(app, require_ready));
    let protected =
        crate::security::protect_api_routes(bootstrap.merge(guarded), security, session);
    // Layer order: the global 1 GiB backstop runs first (outermost); the tier
    // middleware runs after it and overwrites the extension with the tier limit.
    Router::new()
        .nest("/api/v1", protected)
        .layer(middleware::from_fn(tiered_body_limit))
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
}

#[cfg(test)]
mod body_limit_tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use koharu_app::{App, AppConfig};
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use uuid::Uuid;

    const TEST_SECRET: [u8; 32] = [0x2A; 32];
    const MIB: usize = 1024 * 1024;

    struct TestDir(std::path::PathBuf);

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn spawn_test_server() -> (u16, tokio::task::JoinHandle<()>, TestDir) {
        let root =
            TestDir(std::env::temp_dir().join(format!("koharu-body-limit-{}", Uuid::new_v4())));
        std::fs::create_dir_all(&root.0).expect("create test root");
        let runtime = RuntimeManager::new(root.0.join("runtime"), ComputePolicy::CpuOnly)
            .expect("create runtime");
        runtime.prepare().await.expect("prepare runtime");
        let runtime = Arc::new(runtime);
        let app = Arc::new(
            App::new(AppConfig::default(), runtime.clone(), true, "test").expect("create app"),
        );
        let state = crate::BootstrapManager::new(runtime);
        assert!(state.set_app(app).is_ok(), "set test app");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let router = router(
            state,
            crate::security::SecurityContext::from_secret(TEST_SECRET),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve");
        });
        (port, server, root)
    }

    fn bearer() -> String {
        format!("Bearer {}", URL_SAFE_NO_PAD.encode(TEST_SECRET))
    }

    fn mask_path() -> String {
        format!("/api/v1/pages/{}/masks/segment", koharu_core::PageId::new())
    }

    fn parse_status(head: &[u8]) -> Option<u16> {
        let line = head.split(|b| *b == b'\r').next()?;
        let text = std::str::from_utf8(line).ok()?;
        text.split_whitespace().nth(1)?.parse().ok()
    }

    // Hand-rolled HTTP/1.1 client: writes the head, streams `body_len` bytes
    // from a fixed 1 MiB chunk, then reads the response head. The server may
    // reject early (413) and close mid-write — EPIPE on write is expected then.
    async fn raw_request(
        port: u16,
        method: &str,
        path: &str,
        content_type: &str,
        body_len: usize,
    ) -> u16 {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: {}\r\nContent-Type: {content_type}\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n",
            bearer()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        // Heap-allocated chunk: a stack array this size would live inside the
        // async future and overflow the small tokio test-thread stack.
        let chunk = vec![0x78u8; 64 * 1024];
        let mut remaining = body_len;
        while remaining > 0 {
            let n = remaining.min(chunk.len());
            match stream.write(&chunk[..n]).await {
                Ok(0) => break,
                Ok(written) => remaining -= written,
                Err(_) => break,
            }
        }
        let mut response = Vec::new();
        let mut buf = [0u8; 4096];
        tokio::time::timeout(std::time::Duration::from_secs(120), async {
            loop {
                let read = stream.read(&mut buf).await.unwrap_or(0);
                if read == 0 {
                    break;
                }
                response.extend_from_slice(&buf[..read]);
                if response.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
        })
        .await
        .expect("response timed out");
        parse_status(&response).expect("response head")
    }

    // AR05-T01 RED: tiered body limits must reject over-limit bodies with 413
    // before the handler; today everything inherits the 1 GiB global limit.
    #[tokio::test]
    async fn body_limit_control_tier_413() {
        let (port, server, _root) = spawn_test_server().await;
        let status =
            raw_request(port, "PATCH", "/api/v1/config", "application/json", MIB + 1).await;
        server.abort();
        assert_eq!(
            status, 413,
            "control tier must reject >1 MiB before the handler"
        );
    }

    #[tokio::test]
    async fn body_limit_mask_tier_413() {
        let (port, server, _root) = spawn_test_server().await;
        let status = raw_request(port, "PUT", &mask_path(), "image/png", 64 * MIB + 1).await;
        server.abort();
        assert_eq!(
            status, 413,
            "mask tier must reject >64 MiB before the handler"
        );
    }

    #[tokio::test]
    async fn body_limit_archive_tier_413() {
        let (port, server, _root) = spawn_test_server().await;
        let status = raw_request(
            port,
            "POST",
            "/api/v1/projects/import",
            "application/zip",
            512 * MIB + 1,
        )
        .await;
        server.abort();
        assert_eq!(
            status, 413,
            "archive tier must reject >512 MiB before the handler"
        );
    }

    // Lock: exactly-at-limit bodies reach the handler (its own 4xx, not 413),
    // and a small control request behaves normally.
    #[tokio::test]
    async fn body_limit_at_tier_passes() {
        let (port, server, _root) = spawn_test_server().await;
        let control = raw_request(port, "PATCH", "/api/v1/config", "application/json", MIB).await;
        assert_ne!(control, 413, "at-limit control body must reach the handler");
        let mask = raw_request(port, "PUT", &mask_path(), "image/png", 64 * MIB).await;
        assert_ne!(mask, 413, "at-limit mask body must reach the handler");
        let archive = raw_request(
            port,
            "POST",
            "/api/v1/projects/import",
            "application/zip",
            512 * MIB,
        )
        .await;
        assert_ne!(archive, 413, "at-limit archive body must reach the handler");
        let small = raw_request(port, "PATCH", "/api/v1/config", "application/json", 2).await;
        assert_ne!(
            small, 413,
            "small control request must pass the limit layer"
        );
        server.abort();
    }
}
