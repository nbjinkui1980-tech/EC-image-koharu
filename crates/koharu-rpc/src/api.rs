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
    let security_for_guard = security.clone();
    let session_for_auth = session.clone();
    let bootstrap = {
        let mut r = bootstrap.with_state(app.clone());
        if let Some(s) = session {
            r = r.route(
                "/auth/session",
                axum::routing::post(move |req: Request| {
                    let security = security.clone();
                    let session = s.clone();
                    async move { handle_session_exchange(req, security, session).await }
                }),
            );
        }
        r
    };
    let guarded = guarded
        .with_state(app.clone())
        .layer(middleware::from_fn_with_state(app, require_ready));
    let guarded = if let Some(s) = session_for_auth {
        guarded.layer(middleware::from_fn_with_state(
            (security_for_guard, s),
            crate::security::require_session_auth,
        ))
    } else {
        guarded.layer(middleware::from_fn_with_state(
            security_for_guard,
            crate::security::require_auth,
        ))
    };
    Router::new()
        .nest("/api/v1", bootstrap.merge(guarded))
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
