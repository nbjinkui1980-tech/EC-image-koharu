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

pub(crate) fn decode_bearer(headers: &HeaderMap) -> Option<[u8; 32]> {
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

// ── Origin / Host policy ─────────────────────────────────────────────────

use std::collections::BTreeSet;
use std::net::SocketAddr;

use axum::http::{HeaderValue, Method};

#[derive(Clone, Debug, Default)]
pub struct RemoteHostPolicy {
    authorities: BTreeSet<String>,
}

impl RemoteHostPolicy {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn parse(raw: &[String]) -> anyhow::Result<Self> {
        let mut authorities = BTreeSet::new();
        for entry in raw {
            let trimmed = entry.trim();
            if trimmed.is_empty() || trimmed.contains('*') {
                anyhow::bail!("invalid remote host: {trimmed:?}");
            }
            authorities.insert(trimmed.to_ascii_lowercase());
        }
        Ok(Self { authorities })
    }

    fn accepts(&self, host: &str) -> bool {
        self.authorities.contains(&host.to_ascii_lowercase())
    }
}

#[derive(Clone)]
pub struct OriginHostPolicy {
    loopback_port: u16,
    is_debug: bool,
    remotes: RemoteHostPolicy,
}

impl OriginHostPolicy {
    pub fn for_listener(addr: SocketAddr, debug: bool, remotes: RemoteHostPolicy) -> Self {
        Self {
            loopback_port: addr.port(),
            is_debug: debug,
            remotes,
        }
    }
}

pub async fn enforce_origin_host(request: Request, next: Next) -> Response {
    let policy: Option<&OriginHostPolicy> = request.extensions().get();

    let origin = request
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let host = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if request.method() == Method::OPTIONS && !origin.is_empty() {
        let allowed = is_allowed(policy, origin, host);
        if allowed {
            return (
                StatusCode::NO_CONTENT,
                [(
                    axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
                    HeaderValue::from_static("null"),
                )],
            )
                .into_response();
        }
        return StatusCode::FORBIDDEN.into_response();
    }

    if is_allowed(policy, origin, host) {
        return next.run(request).await;
    }
    StatusCode::FORBIDDEN.into_response()
}

fn is_allowed(policy: Option<&OriginHostPolicy>, origin: &str, host: &str) -> bool {
    let Some(policy) = policy else {
        return true;
    };
    if origin.is_empty() {
        return true;
    }
    if origin == "null" || origin == "*" {
        return false;
    }
    if let Some(origin_host) = origin
        .strip_prefix("http://")
        .and_then(|o| o.rsplitn(2, ':').nth(1))
    {
        if origin_host == "127.0.0.1" || origin_host == "localhost" {
            return is_loopback_origin(origin, policy.loopback_port)
                || (policy.is_debug && origin == "http://localhost:3000");
        }
        return false;
    }
    if let Some(origin_host) = origin
        .strip_prefix("https://")
        .and_then(|o| o.rsplitn(2, ':').nth(1))
    {
        let host_base = host.rsplitn(2, ':').nth(1).unwrap_or(host);
        return origin_host == host_base && policy.remotes.accepts(host);
    }
    false
}

fn is_loopback_origin(origin: &str, port: u16) -> bool {
    origin == format!("http://127.0.0.1:{port}") || origin == format!("http://localhost:{port}")
}

// ── Browser session state ─────────────────────────────────────────────────

use std::sync::Arc;
use std::sync::Mutex;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct BrowserSessionState {
    session_token: [u8; 32],
    proof: Arc<Mutex<Option<[u8; 32]>>>,
}

impl BrowserSessionState {
    pub fn new(proof: Option<[u8; 32]>, session: [u8; 32]) -> Self {
        Self {
            session_token: session,
            proof: Arc::new(Mutex::new(proof)),
        }
    }

    pub fn consume_proof(&self, candidate: &[u8; 32]) -> bool {
        let mut guard = self.proof.lock().unwrap();
        match guard.as_ref() {
            Some(stored) if stored.ct_eq(candidate).into() => {
                *guard = None;
                true
            }
            _ => false,
        }
    }

    pub fn session_token_encoded(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.session_token)
    }

    pub fn validate_session(&self, cookie_value: &str) -> bool {
        decode_url_safe_no_padding(cookie_value)
            .is_some_and(|token| token.ct_eq(&self.session_token).into())
    }
}

pub fn generate_token() -> [u8; 32] {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("CSPRNG failure");
    buf
}

pub fn authorizes_session(cookie_header: Option<&str>, state: &BrowserSessionState) -> bool {
    let Some(cookie_str) = cookie_header else {
        return false;
    };
    for part in cookie_str.split(';') {
        let trimmed = part.trim();
        if let Some(value) = trimmed.strip_prefix("koharu_session=") {
            return state.validate_session(value);
        }
    }
    false
}

pub async fn require_session_auth(
    axum::extract::State((ctx, session)): axum::extract::State<(
        SecurityContext,
        BrowserSessionState,
    )>,
    request: Request,
    next: Next,
) -> Response {
    if ctx.authorizes_bearer(request.headers()) {
        return next.run(request).await;
    }
    let cookie = request
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok());
    if authorizes_session(cookie, &session) {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
    )
        .into_response()
}
