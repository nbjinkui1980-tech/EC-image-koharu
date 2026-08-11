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

use std::net::{IpAddr, SocketAddr};

use axum::http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD,
    CACHE_CONTROL, HOST, ORIGIN, VARY,
};
use axum::http::uri::Authority;
use axum::http::{HeaderName, HeaderValue, Method};

const ALLOWED_METHODS: &str = "GET, POST, PUT, PATCH, DELETE";
const ALLOWED_HEADERS: &str = "authorization, content-type, accept, last-event-id";
const ALLOWED_HEADER_NAMES: &[&str] = &["authorization", "content-type", "accept", "last-event-id"];

#[derive(Clone, Debug, Default)]
pub struct RemoteHostPolicy {
    authorities: Vec<Authority>,
}

impl RemoteHostPolicy {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn parse(raw: &[String]) -> anyhow::Result<Self> {
        let mut authorities = Vec::new();
        for entry in raw {
            let authority = parse_authority(entry.trim())
                .ok_or_else(|| anyhow::anyhow!("invalid remote host: {entry:?}"))?;
            authorities.push(authority);
        }
        Ok(Self { authorities })
    }

    fn accepts_https(&self, host: &Authority) -> bool {
        self.authorities
            .iter()
            .any(|allowed| authorities_match(allowed, host, 443))
    }
}

#[derive(Clone)]
pub struct OriginHostPolicy {
    listener: SocketAddr,
    is_debug: bool,
    remotes: RemoteHostPolicy,
}

impl OriginHostPolicy {
    pub fn for_listener(addr: SocketAddr, debug: bool, remotes: RemoteHostPolicy) -> Self {
        Self {
            listener: addr,
            is_debug: debug,
            remotes,
        }
    }
}

pub async fn enforce_origin_host(request: Request, next: Next) -> Response {
    let Some(policy) = request.extensions().get::<OriginHostPolicy>().cloned() else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(host) = request_authority(&request) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !host_allowed(&policy, &host) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let origin = match single_header(request.headers(), &ORIGIN) {
        Ok(None) => {
            let mut response = next.run(request).await;
            ensure_vary_origin(response.headers_mut());
            return response;
        }
        Ok(Some(raw)) => match parse_origin(raw) {
            Some(origin) if origin_allowed(&policy, &host, &origin) => origin,
            _ => return origin_forbidden(),
        },
        Err(()) => return origin_forbidden(),
    };

    if request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key(ACCESS_CONTROL_REQUEST_METHOD)
    {
        if !valid_preflight(request.headers()) {
            return origin_forbidden();
        }
        let mut response = StatusCode::NO_CONTENT.into_response();
        add_cors_headers(response.headers_mut(), &origin.raw);
        response.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static(ALLOWED_METHODS),
        );
        response.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static(ALLOWED_HEADERS),
        );
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    } else {
        let mut response = next.run(request).await;
        add_cors_headers(response.headers_mut(), &origin.raw);
        response
    }
}

struct ParsedOrigin {
    raw: String,
    https: bool,
    authority: Authority,
}

fn parse_authority(raw: &str) -> Option<Authority> {
    if raw.is_empty() || raw != raw.trim() || raw.contains(['*', '@', '/', '?', '#']) {
        return None;
    }
    let bracketed = raw.starts_with('[');
    if let Some(port) = explicit_port(raw)? {
        port.parse::<u16>().ok()?;
    }
    let authority: Authority = raw.parse().ok()?;
    if authority.host().is_empty() || (bracketed && host_ip(authority.host()).is_none()) {
        return None;
    }
    Some(authority)
}

fn explicit_port(raw: &str) -> Option<Option<&str>> {
    if let Some(bracketed) = raw.strip_prefix('[') {
        let (_, suffix) = bracketed.split_once(']')?;
        return match suffix {
            "" => Some(None),
            suffix => Some(Some(suffix.strip_prefix(':')?)),
        };
    }
    if raw.contains(['[', ']']) || raw.matches(':').count() > 1 {
        return None;
    }
    Some(raw.rsplit_once(':').map(|(_, port)| port))
}

fn parse_origin(raw: &str) -> Option<ParsedOrigin> {
    let (https, authority) = raw
        .strip_prefix("http://")
        .map(|authority| (false, authority))
        .or_else(|| {
            raw.strip_prefix("https://")
                .map(|authority| (true, authority))
        })?;
    Some(ParsedOrigin {
        raw: raw.to_owned(),
        https,
        authority: parse_authority(authority)?,
    })
}

fn single_header<'a>(headers: &'a HeaderMap, name: &HeaderName) -> Result<Option<&'a str>, ()> {
    let mut values = headers.get_all(name).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    let first = first.to_str().map_err(|_| ())?;
    if values.next().is_some() {
        return Err(());
    }
    Ok(Some(first))
}

fn request_authority(request: &Request) -> Option<Authority> {
    let (uri, default_port) = match (request.uri().scheme_str(), request.uri().authority()) {
        (None, None) if request.uri().path().starts_with('/') => (None, 80),
        (Some("http"), Some(authority)) => (Some(parse_authority(authority.as_str())?), 80),
        (Some("https"), Some(authority)) => (Some(parse_authority(authority.as_str())?), 443),
        _ => return None,
    };
    let header = match single_header(request.headers(), &HOST) {
        Ok(Some(raw)) => Some(parse_authority(raw)?),
        Ok(None) => None,
        Err(()) => return None,
    };
    match (header, uri) {
        (Some(header), Some(uri)) => {
            authorities_match(&header, &uri, default_port).then_some(header)
        }
        (Some(header), None) => Some(header),
        (None, Some(uri)) => Some(uri),
        (None, None) => None,
    }
}

fn authority_port(authority: &Authority, default: u16) -> u16 {
    authority.port_u16().unwrap_or(default)
}

fn authorities_match(left: &Authority, right: &Authority, default_port: u16) -> bool {
    hosts_match(left.host(), right.host())
        && authority_port(left, default_port) == authority_port(right, default_port)
}

fn host_ip(host: &str) -> Option<IpAddr> {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
        .parse()
        .ok()
}

fn hosts_match(left: &str, right: &str) -> bool {
    match (host_ip(left), host_ip(right)) {
        (Some(left), Some(right)) => left == right,
        (None, None) => left.eq_ignore_ascii_case(right),
        _ => false,
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host_ip(host).is_some_and(|ip| ip.is_loopback())
}

fn host_allowed(policy: &OriginHostPolicy, host: &Authority) -> bool {
    if policy.listener.ip().is_loopback() {
        is_loopback_host(host.host()) && host.port_u16() == Some(policy.listener.port())
    } else {
        policy.remotes.accepts_https(host)
    }
}

fn origin_allowed(policy: &OriginHostPolicy, host: &Authority, origin: &ParsedOrigin) -> bool {
    if policy.listener.ip().is_loopback() {
        if origin.https {
            return false;
        }
        if policy.is_debug && origin.raw == "http://localhost:3000" {
            return true;
        }
        return authorities_match(host, &origin.authority, 80);
    }
    origin.https && authorities_match(host, &origin.authority, 443)
}

fn valid_preflight(headers: &HeaderMap) -> bool {
    let Ok(Some(method)) = single_header(headers, &ACCESS_CONTROL_REQUEST_METHOD) else {
        return false;
    };
    if !matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return false;
    }
    let Ok(requested) = single_header(headers, &ACCESS_CONTROL_REQUEST_HEADERS) else {
        return false;
    };
    requested.is_none_or(|requested| {
        requested.split(',').all(|name| {
            let name = name.trim().to_ascii_lowercase();
            !name.is_empty() && ALLOWED_HEADER_NAMES.contains(&name.as_str())
        })
    })
}

fn add_cors_headers(headers: &mut HeaderMap, origin: &str) {
    headers.insert(
        ACCESS_CONTROL_ALLOW_ORIGIN,
        origin.parse().expect("validated Origin is a header value"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    ensure_vary_origin(headers);
}

fn origin_forbidden() -> Response {
    let mut response = StatusCode::FORBIDDEN.into_response();
    ensure_vary_origin(response.headers_mut());
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn ensure_vary_origin(headers: &mut HeaderMap) {
    let already_varies = headers
        .get_all(VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|token| token == "*" || token.eq_ignore_ascii_case("Origin"))
        });
    if !already_varies {
        headers.append(VARY, HeaderValue::from_static("Origin"));
    }
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
