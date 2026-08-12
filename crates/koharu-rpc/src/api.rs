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
    let bootstrap = bootstrap.with_state(app.clone());
    let guarded = guarded
        .with_state(app.clone())
        .layer(middleware::from_fn_with_state(app, require_ready));
    let protected =
        crate::security::protect_api_routes(bootstrap.merge(guarded), security, session);
    Router::new()
        .nest("/api/v1", protected)
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
}
