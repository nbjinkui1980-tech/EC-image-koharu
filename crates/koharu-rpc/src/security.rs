use axum::extract::Request;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;

#[derive(Clone)]
pub struct SecurityContext {
    secret: [u8; 32],
}

impl SecurityContext {
    pub fn from_secret(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    pub fn authorizes_bearer(&self, headers: &HeaderMap) -> bool {
        decode_bearer(headers).is_some_and(|token| token == self.secret)
    }
}

fn decode_bearer(headers: &HeaderMap) -> Option<[u8; 32]> {
    let mut values = headers.get_all(axum::http::header::AUTHORIZATION).iter();
    let first = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let token = first.strip_prefix("Bearer ")?;
    decode_url_safe_no_padding(token)
}

fn decode_url_safe_no_padding(encoded: &str) -> Option<[u8; 32]> {
    if encoded.len() != 43 {
        return None;
    }
    let mut buf = [0u8; 32];
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode_slice(encoded, &mut buf)
        .ok()?;
    Some(buf)
}

pub async fn require_auth(
    axum::extract::State(ctx): axum::extract::State<SecurityContext>,
    request: Request,
    next: Next,
) -> Response {
    if ctx.authorizes_bearer(request.headers()) {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
    )
        .into_response()
}
