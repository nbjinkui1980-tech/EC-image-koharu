//! Typed API errors. All handlers return `ApiResult<T>`; errors land as JSON
//! with `status` + `message`.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status: status.as_u16(),
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn service_unavailable(err: anyhow::Error) -> Self {
        Self::server_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "service temporarily unavailable",
            err,
        )
    }

    pub fn internal(err: anyhow::Error) -> Self {
        Self::server_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error",
            err,
        )
    }

    fn server_error(status: StatusCode, message: &'static str, err: anyhow::Error) -> Self {
        tracing::error!(
            http.status = status.as_u16(),
            error.causes = err.chain().count(),
            "request failed"
        );
        Self::new(status, message)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use super::ApiError;

    const PRIVATE_DETAIL: &str =
        "/Users/alice/private/project.khr: provider body token=sk-test-secret";

    async fn response_body(error: ApiError) -> String {
        let body = error.into_response().into_body();
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn api_error_internal_response_is_stable_and_redacted() {
        let error = anyhow::anyhow!(PRIVATE_DETAIL).context("nested cause");
        let body = response_body(ApiError::internal(error)).await;

        assert_eq!(body, r#"{"status":500,"message":"internal server error"}"#);
        assert!(!body.contains(PRIVATE_DETAIL));
        assert!(!body.contains("nested cause"));
    }

    #[tokio::test]
    async fn api_error_service_unavailable_response_is_stable_and_redacted() {
        let body = response_body(ApiError::service_unavailable(anyhow::anyhow!(
            PRIVATE_DETAIL
        )))
        .await;

        assert_eq!(
            body,
            r#"{"status":503,"message":"service temporarily unavailable"}"#
        );
        assert!(!body.contains(PRIVATE_DETAIL));
    }

    #[tokio::test]
    async fn api_error_explicit_client_message_is_preserved() {
        let error = ApiError::bad_request("invalid page selection");
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            r#"{"status":400,"message":"invalid page selection"}"#
        );
    }
}
