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

    if !security.authorizes_bearer(request.headers()) && !session.has_proof() {
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
    use crate::security::SecurityContext;

    const TEST_SECRET: [u8; 32] = [0x2A; 32];

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
