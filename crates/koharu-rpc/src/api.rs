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
    let exchange_security = security.clone();
    let exchange = if let Some(session) = session.clone() {
        Router::new().route(
            "/auth/session",
            axum::routing::post(move |req: Request| {
                let security = exchange_security.clone();
                let session = session.clone();
                async move { handle_session_exchange(req, security, session).await }
            }),
        )
    } else {
        Router::new()
    };
    let guarded = guarded
        .with_state(app.clone())
        .layer(middleware::from_fn_with_state(app.clone(), require_ready));
    let protected = bootstrap.with_state(app).merge(guarded);
    let protected = if let Some(session) = session {
        protected.layer(middleware::from_fn_with_state(
            (security, session),
            crate::security::require_session_auth,
        ))
    } else {
        protected.layer(middleware::from_fn_with_state(
            security,
            crate::security::require_auth,
        ))
    };
    Router::new()
        .nest("/api/v1", exchange.merge(protected))
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
}

async fn handle_session_exchange(
    request: Request,
    security: crate::security::SecurityContext,
    session: crate::security::BrowserSessionState,
) -> Response {
    use axum::http::header;

    let authorized = security.authorizes_bearer(request.headers())
        || crate::security::decode_bearer(request.headers())
            .is_some_and(|proof| session.consume_proof(&proof));
    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();
    }

    let token = session.session_token_encoded();
    let cookie = format!("koharu_session={token}; HttpOnly; SameSite=Strict; Path=/");
    (
        StatusCode::NO_CONTENT,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::SET_COOKIE, cookie.as_str()),
        ],
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base64::Engine;
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use uuid::Uuid;

    use crate::security::SecurityContext;

    const TEST_SECRET: [u8; 32] = [0x2A; 32];

    async fn exchange_status(
        addr: std::net::SocketAddr,
        credential: Option<&str>,
    ) -> (u16, String) {
        let mut request = format!(
            "POST /api/v1/auth/session HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 0\r\nConnection: close\r\n"
        );
        if let Some(credential) = credential {
            request.push_str(&format!("Authorization: Bearer {credential}\r\n"));
        }
        request.push_str("\r\n");

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        loop {
            let mut chunk = [0; 1024];
            let read =
                tokio::time::timeout(std::time::Duration::from_secs(2), stream.read(&mut chunk))
                    .await
                    .expect("response headers timed out")
                    .unwrap();
            if read == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..read]);
            if response.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let headers = String::from_utf8_lossy(&response).into_owned();
        let status = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        (status, headers)
    }

    async fn get_status(
        addr: std::net::SocketAddr,
        path: &str,
        cookie: Option<&str>,
        bearer: Option<&str>,
    ) -> u16 {
        let mut request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
        if let Some(cookie) = cookie {
            request.push_str(&format!("Cookie: koharu_session={cookie}\r\n"));
        }
        if let Some(bearer) = bearer {
            request.push_str(&format!("Authorization: Bearer {bearer}\r\n"));
        }
        request.push_str("\r\n");

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        loop {
            let mut chunk = [0; 1024];
            let read =
                tokio::time::timeout(std::time::Duration::from_secs(2), stream.read(&mut chunk))
                    .await
                    .expect("response headers timed out")
                    .unwrap();
            if read == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..read]);
            if response.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&response)
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap()
    }

    async fn request_head(
        addr: std::net::SocketAddr,
        method: &str,
        path: &str,
        host: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> (u16, String) {
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(body);

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        loop {
            let mut chunk = [0; 1024];
            let read =
                tokio::time::timeout(std::time::Duration::from_secs(2), stream.read(&mut chunk))
                    .await
                    .expect("response headers timed out")
                    .unwrap();
            if read == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..read]);
            if response.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let headers = String::from_utf8_lossy(&response).into_owned();
        let status = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        (status, headers)
    }

    #[tokio::test]
    async fn session_exchange_rejects_missing_credential() {
        let root = std::env::temp_dir().join(format!("koharu-rpc-session-{}", Uuid::new_v4()));
        let app = crate::BootstrapManager::new(Arc::new(
            RuntimeManager::new(&root, ComputePolicy::CpuOnly).unwrap(),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let session = crate::security::BrowserSessionState::new(Some([0x2B; 32]), [0x2C; 32]);
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                super::router_with_session(app, SecurityContext::from_secret(TEST_SECRET), session),
            )
            .await
            .unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let (status, headers) = exchange_status(addr, None).await;
        assert_eq!(status, 401);
        assert!(!headers.contains("set-cookie:"));

        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn session_exchange_accepts_master_or_one_time_proof() {
        let root = std::env::temp_dir().join(format!("koharu-rpc-session-{}", Uuid::new_v4()));
        let app = crate::BootstrapManager::new(Arc::new(
            RuntimeManager::new(&root, ComputePolicy::CpuOnly).unwrap(),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let proof = [0x2B; 32];
        let session = crate::security::BrowserSessionState::new(Some(proof), [0x2C; 32]);
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                super::router_with_session(app, SecurityContext::from_secret(TEST_SECRET), session),
            )
            .await
            .unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let proof = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(proof);
        let (status, headers) = exchange_status(addr, Some(&proof)).await;
        assert_eq!(status, 204);
        assert!(headers.contains("set-cookie:"));
        assert!(headers.contains("HttpOnly; SameSite=Strict; Path=/"));

        assert_eq!(exchange_status(addr, Some(&proof)).await.0, 401);

        let master = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(TEST_SECRET);
        assert_eq!(exchange_status(addr, Some(&master)).await.0, 204);

        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn session_exchange_is_mounted_by_session_server() {
        let root = std::env::temp_dir().join(format!("koharu-rpc-session-{}", Uuid::new_v4()));
        let app = crate::BootstrapManager::new(Arc::new(
            RuntimeManager::new(&root, ComputePolicy::CpuOnly).unwrap(),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let security = SecurityContext::from_secret(TEST_SECRET);
        let policy = crate::security::OriginHostPolicy::for_listener(
            addr,
            false,
            crate::security::RemoteHostPolicy::empty(),
        );
        let server = tokio::spawn(crate::server::serve_with_listener_with_session(
            listener,
            app,
            security,
            policy,
            crate::security::BrowserSessionState::new(None, [0x2C; 32]),
        ));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let master = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(TEST_SECRET);
        assert_eq!(exchange_status(addr, Some(&master)).await.0, 204);

        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn session_cookie_reaches_api_readiness_without_master_bearer() {
        let root = std::env::temp_dir().join(format!("koharu-rpc-session-{}", Uuid::new_v4()));
        let app = crate::BootstrapManager::new(Arc::new(
            RuntimeManager::new(&root, ComputePolicy::CpuOnly).unwrap(),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let security = SecurityContext::from_secret(TEST_SECRET);
        let policy = crate::security::OriginHostPolicy::for_listener(
            addr,
            false,
            crate::security::RemoteHostPolicy::empty(),
        );
        let session = crate::security::BrowserSessionState::new(None, [0x2C; 32]);
        let cookie = session.session_token_encoded();
        let server = tokio::spawn(crate::server::serve_with_listener_with_session(
            listener, app, security, policy, session,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        assert_eq!(
            get_status(addr, "/api/v1/meta", Some(&cookie), None).await,
            503
        );

        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn bootstrap_routes_require_authentication() {
        let root = std::env::temp_dir().join(format!("koharu-rpc-auth-{}", Uuid::new_v4()));
        let app = crate::BootstrapManager::new(Arc::new(
            RuntimeManager::new(&root, ComputePolicy::CpuOnly).unwrap(),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let policy = crate::security::OriginHostPolicy::for_listener(
            addr,
            false,
            crate::security::RemoteHostPolicy::empty(),
        );
        let session = crate::security::BrowserSessionState::new(None, [0x2C; 32]);
        let cookie = session.session_token_encoded();
        let server = tokio::spawn(crate::server::serve_with_listener_with_session(
            listener,
            app,
            SecurityContext::from_secret(TEST_SECRET),
            policy,
            session,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let host = addr.to_string();
        let wrong = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        for path in ["/api/v1/events", "/api/v1/downloads", "/api/v1/operations"] {
            assert_eq!(get_status(addr, path, None, None).await, 401, "{path}");
            assert_eq!(
                get_status(addr, path, None, Some(wrong)).await,
                401,
                "{path}"
            );
        }

        let (status, _) = request_head(
            addr,
            "POST",
            "/api/v1/downloads",
            &host,
            &[("Content-Type", "application/json")],
            r#"{"modelId":"missing:test-model"}"#,
        )
        .await;
        assert_eq!(status, 401);

        let operation_id = format!("auth-cancel-{}", Uuid::new_v4());
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        crate::routes::operations::register_cancel(operation_id.clone(), cancelled.clone());
        let (status, _) = request_head(
            addr,
            "DELETE",
            &format!("/api/v1/operations/{operation_id}"),
            &host,
            &[],
            "",
        )
        .await;
        assert_eq!(status, 401);
        assert!(!cancelled.load(std::sync::atomic::Ordering::Relaxed));
        crate::routes::operations::unregister_cancel(&operation_id);

        let master = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(TEST_SECRET);
        for path in ["/api/v1/events", "/api/v1/downloads", "/api/v1/operations"] {
            assert_eq!(
                get_status(addr, path, None, Some(&master)).await,
                200,
                "{path}"
            );
            assert_eq!(
                get_status(addr, path, Some(&cookie), None).await,
                200,
                "{path}"
            );
        }
        for path in ["/api/v1/meta", "/api/v1/scene.bin"] {
            assert_eq!(
                get_status(addr, path, None, Some(&master)).await,
                503,
                "{path}"
            );
            assert_eq!(
                get_status(addr, path, Some(&cookie), None).await,
                503,
                "{path}"
            );
        }

        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_remains_master_bearer_only() {
        let root = std::env::temp_dir().join(format!("koharu-rpc-session-{}", Uuid::new_v4()));
        let app = crate::BootstrapManager::new(Arc::new(
            RuntimeManager::new(&root, ComputePolicy::CpuOnly).unwrap(),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let security = SecurityContext::from_secret(TEST_SECRET);
        let policy = crate::security::OriginHostPolicy::for_listener(
            addr,
            false,
            crate::security::RemoteHostPolicy::empty(),
        );
        let session = crate::security::BrowserSessionState::new(None, [0x2C; 32]);
        let cookie = session.session_token_encoded();
        let server = tokio::spawn(crate::server::serve_with_listener_with_session(
            listener, app, security, policy, session,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        assert_eq!(get_status(addr, "/mcp", None, None).await, 401);
        assert_eq!(get_status(addr, "/mcp", Some(&cookie), None).await, 401);
        let master = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(TEST_SECRET);
        assert_ne!(get_status(addr, "/mcp", None, Some(&master)).await, 401);

        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn security_context_rejects_missing_header() {
        let ctx = SecurityContext::from_secret(TEST_SECRET);
        let headers = axum::http::HeaderMap::new();
        assert!(!ctx.authorizes_bearer(&headers));
    }

    #[test]
    fn security_context_rejects_malformed_bearer() {
        let ctx = SecurityContext::from_secret(TEST_SECRET);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer !!!".parse().unwrap());
        assert!(!ctx.authorizes_bearer(&headers));
    }

    #[test]
    fn security_context_rejects_wrong_secret() {
        let ctx = SecurityContext::from_secret(TEST_SECRET);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "authorization",
            "Bearer AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                .parse()
                .unwrap(),
        );
        assert!(!ctx.authorizes_bearer(&headers));
    }

    #[test]
    fn security_context_accepts_correct_bearer() {
        let ctx = SecurityContext::from_secret(TEST_SECRET);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "authorization",
            "Bearer KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio"
                .parse()
                .unwrap(),
        );
        assert!(ctx.authorizes_bearer(&headers));
    }

    #[test]
    fn security_context_rejects_cookie_only() {
        let ctx = SecurityContext::from_secret(TEST_SECRET);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("cookie", "koharu_session=test".parse().unwrap());
        assert!(!ctx.authorizes_bearer(&headers));
    }

    #[test]
    fn security_context_rejects_duplicate_auth_headers() {
        let ctx = SecurityContext::from_secret(TEST_SECRET);
        let mut headers = axum::http::HeaderMap::new();
        headers.append(
            "authorization",
            "Bearer KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio"
                .parse()
                .unwrap(),
        );
        headers.append("authorization", "Bearer extra".parse().unwrap());
        assert!(!ctx.authorizes_bearer(&headers));
    }
}
