use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::extract::Request;
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD,
    CACHE_CONTROL, HOST, ORIGIN, VARY,
};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct SecurityContext {
    secret: [u8; 32],
}

impl SecurityContext {
    pub fn from_secret(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    pub fn authorizes_bearer(&self, headers: &HeaderMap) -> bool {
        decode_bearer(headers).is_some_and(|token| token.ct_eq(&self.secret).into())
    }
}

fn decode_bearer(headers: &HeaderMap) -> Option<[u8; 32]> {
    let mut values = headers.get_all(axum::http::header::AUTHORIZATION).iter();
    let first = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    decode_url_safe_no_padding(first.strip_prefix("Bearer ")?)
}

fn decode_url_safe_no_padding(encoded: &str) -> Option<[u8; 32]> {
    if encoded.len() != 43 {
        return None;
    }
    let mut buf = [0u8; 32];
    URL_SAFE_NO_PAD.decode_slice(encoded, &mut buf).ok()?;
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

const ALLOWED_METHODS: &str = "GET, POST, PUT, PATCH, DELETE";
const ALLOWED_HEADERS: &str = "authorization, content-type, accept, last-event-id";

#[derive(Clone, Debug, Default)]
pub struct RemoteHostPolicy {
    authorities: Vec<Authority>,
}

impl RemoteHostPolicy {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn parse(raw: &[String]) -> anyhow::Result<Self> {
        raw.iter()
            .map(|entry| {
                parse_authority(entry.trim())
                    .ok_or_else(|| anyhow::anyhow!("invalid remote host: {entry:?}"))
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .map(|authorities| Self { authorities })
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
        Err(())
    } else {
        Ok(Some(first))
    }
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

fn authorities_match(left: &Authority, right: &Authority, default_port: u16) -> bool {
    hosts_match(left.host(), right.host())
        && left.port_u16().unwrap_or(default_port) == right.port_u16().unwrap_or(default_port)
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
        !origin.https
            && ((policy.is_debug && origin.raw == "http://localhost:3000")
                || authorities_match(host, &origin.authority, 80))
    } else {
        origin.https && authorities_match(host, &origin.authority, 443)
    }
}
fn valid_preflight(headers: &HeaderMap) -> bool {
    let Ok(Some(method)) = single_header(headers, &ACCESS_CONTROL_REQUEST_METHOD) else {
        return false;
    };
    if !ALLOWED_METHODS.split(", ").any(|allowed| allowed == method) {
        return false;
    }
    single_header(headers, &ACCESS_CONTROL_REQUEST_HEADERS).is_ok_and(|requested| {
        requested.is_none_or(|requested| {
            requested.split(',').all(|name| {
                ALLOWED_HEADERS
                    .split(", ")
                    .any(|allowed| allowed.eq_ignore_ascii_case(name.trim()))
            })
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
    if !headers
        .get_all(VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|token| token == "*" || token.eq_ignore_ascii_case("Origin"))
        })
    {
        headers.append(VARY, HeaderValue::from_static("Origin"));
    }
}

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
        if let Some(value) = part.trim().strip_prefix("koharu_session=") {
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
        .and_then(|value| value.to_str().ok());
    if authorizes_session(cookie, &session) {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
    )
        .into_response()
}

pub async fn session_exchange(
    request: Request,
    security: SecurityContext,
    session: BrowserSessionState,
) -> Response {
    use axum::http::header;

    let authorized = security.authorizes_bearer(request.headers())
        || decode_bearer(request.headers()).is_some_and(|proof| session.consume_proof(&proof));
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

/// Protect routes that must accept only the master Bearer credential.
pub fn protect_bearer_only_routes(router: axum::Router, security: SecurityContext) -> axum::Router {
    router.layer(axum::middleware::from_fn(
        move |request: Request, next: Next| {
            let security = security.clone();
            async move {
                if security.authorizes_bearer(request.headers()) {
                    return next.run(request).await;
                }
                StatusCode::UNAUTHORIZED.into_response()
            }
        },
    ))
}

/// Apply the production API authentication boundary, including browser-session exchange.
pub fn protect_api_routes(
    router: axum::Router,
    security: SecurityContext,
    session: Option<BrowserSessionState>,
) -> axum::Router {
    if let Some(session) = session {
        let exchange_security = security.clone();
        let exchange_session = session.clone();
        router
            .layer(axum::middleware::from_fn_with_state(
                (security, session),
                require_session_auth,
            ))
            .route(
                "/auth/session",
                axum::routing::post(move |request: Request| {
                    let security = exchange_security.clone();
                    let session = exchange_session.clone();
                    async move { session_exchange(request, security, session).await }
                }),
            )
    } else {
        router.layer(axum::middleware::from_fn_with_state(security, require_auth))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::middleware;
    use axum::routing::get;
    use tower::ServiceExt;

    const TEST_SECRET: [u8; 32] = [0x2A; 32];

    fn bearer(secret: [u8; 32]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let value = format!("Bearer {}", URL_SAFE_NO_PAD.encode(secret));
        headers.insert(axum::http::header::AUTHORIZATION, value.parse().unwrap());
        headers
    }

    fn protected_test_routes() -> Router {
        Router::new()
            .route("/operations", get(|| async { StatusCode::IM_A_TEAPOT }))
            .route("/meta", get(|| async { StatusCode::IM_A_TEAPOT }))
            .route("/events", get(|| async { StatusCode::IM_A_TEAPOT }))
            .route("/blobs/{id}", get(|| async { StatusCode::IM_A_TEAPOT }))
            .route("/downloads", get(|| async { StatusCode::IM_A_TEAPOT }))
    }

    fn request(path: &str, headers: HeaderMap) -> Request {
        let mut request = Request::builder()
            .uri(path)
            .body(axum::body::Body::empty())
            .unwrap();
        *request.headers_mut() = headers;
        request
    }

    #[tokio::test]
    async fn protected_api_router_applies_bearer_auth_to_every_route() {
        let app = Router::new().nest(
            "/api/v1",
            protect_api_routes(
                protected_test_routes(),
                SecurityContext::from_secret(TEST_SECRET),
                None,
            ),
        );

        for path in [
            "/api/v1/operations",
            "/api/v1/meta",
            "/api/v1/events",
            "/api/v1/blobs/0000",
            "/api/v1/downloads",
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(request(path, HeaderMap::new()))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED,
                "{path}"
            );
            assert_eq!(
                app.clone()
                    .oneshot(request(path, bearer(TEST_SECRET)))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::IM_A_TEAPOT,
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn protected_api_router_mounts_exchange_and_accepts_issued_session() {
        let session = BrowserSessionState::new(Some([0x2B; 32]), [0x2C; 32]);
        let app = Router::new().nest(
            "/api/v1",
            protect_api_routes(
                protected_test_routes(),
                SecurityContext::from_secret(TEST_SECRET),
                Some(session),
            ),
        );

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/auth/session")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let exchange = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/auth/session")
                    .header(
                        axum::http::header::AUTHORIZATION,
                        format!("Bearer {}", URL_SAFE_NO_PAD.encode([0x2B; 32])),
                    )
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(exchange.status(), StatusCode::NO_CONTENT);
        let cookie = exchange
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/meta")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    }

    #[tokio::test]
    async fn bearer_only_router_rejects_session_cookie() {
        let app = protect_bearer_only_routes(
            Router::new().route("/mcp", get(|| async { StatusCode::IM_A_TEAPOT })),
            SecurityContext::from_secret(TEST_SECRET),
        );
        let session = BrowserSessionState::new(None, [0x2C; 32]);

        let cookie_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .header(
                        axum::http::header::COOKIE,
                        format!("koharu_session={}", session.session_token_encoded()),
                    )
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cookie_response.status(), StatusCode::UNAUTHORIZED);

        let bearer_response = app
            .oneshot(request("/mcp", bearer(TEST_SECRET)))
            .await
            .unwrap();
        assert_eq!(bearer_response.status(), StatusCode::IM_A_TEAPOT);
    }

    #[test]
    fn bearer_auth_rejects_missing_malformed_wrong_and_duplicate_headers() {
        let context = SecurityContext::from_secret(TEST_SECRET);
        assert!(!context.authorizes_bearer(&HeaderMap::new()));

        let mut malformed = HeaderMap::new();
        malformed.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer !!!".parse().unwrap(),
        );
        assert!(!context.authorizes_bearer(&malformed));
        assert!(!context.authorizes_bearer(&bearer([0x2B; 32])));

        let mut duplicate = bearer(TEST_SECRET);
        duplicate.append(
            axum::http::header::AUTHORIZATION,
            "Bearer extra".parse().unwrap(),
        );
        assert!(!context.authorizes_bearer(&duplicate));
        assert!(context.authorizes_bearer(&bearer(TEST_SECRET)));
    }

    #[test]
    fn session_and_proof_are_validated_without_replay() {
        let proof = [0x2B; 32];
        let state = BrowserSessionState::new(Some(proof), [0x2C; 32]);
        assert!(state.consume_proof(&proof));
        assert!(!state.consume_proof(&proof));
        assert!(state.validate_session(&state.session_token_encoded()));
        assert!(!state.validate_session(&URL_SAFE_NO_PAD.encode([0x2D; 32])));
    }

    #[test]
    fn session_cookie_uses_first_matching_value() {
        let state = BrowserSessionState::new(None, [0x2C; 32]);
        let valid = state.session_token_encoded();

        assert!(!authorizes_session(
            Some(&format!("koharu_session=invalid; koharu_session={valid}")),
            &state,
        ));
        assert!(authorizes_session(
            Some(&format!(
                "other=value; koharu_session={valid}; koharu_session=invalid"
            )),
            &state,
        ));
    }

    #[test]
    fn remote_host_policy_rejects_empty_and_wildcard_authorities() {
        assert!(RemoteHostPolicy::parse(&[String::new()]).is_err());
        assert!(RemoteHostPolicy::parse(&["*.example.com".into()]).is_err());
        assert!(RemoteHostPolicy::parse(&["https://example.com".into()]).is_err());
        assert!(RemoteHostPolicy::parse(&["example.com/path".into()]).is_err());
        assert!(RemoteHostPolicy::parse(&["example.com:".into()]).is_err());
        assert!(RemoteHostPolicy::parse(&["example.com:not-a-port".into()]).is_err());
        assert!(RemoteHostPolicy::parse(&["example.com".into()]).is_ok());
        assert!(RemoteHostPolicy::parse(&["example.com:443".into()]).is_ok());
    }

    #[tokio::test]
    async fn origin_host_middleware_handles_options() {
        let policy = OriginHostPolicy::for_listener(
            "127.0.0.1:4321".parse().unwrap(),
            false,
            RemoteHostPolicy::empty(),
        );
        let app = Router::new()
            .route("/", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn(enforce_origin_host))
            .layer(axum::Extension(policy));

        for (origin, expected) in [
            ("http://localhost:4321", StatusCode::NO_CONTENT),
            ("null", StatusCode::FORBIDDEN),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::OPTIONS)
                        .uri("/")
                        .header(axum::http::header::ORIGIN, origin)
                        .header(axum::http::header::HOST, "localhost:4321")
                        .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
                        .header(ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{origin}");
            if expected == StatusCode::NO_CONTENT {
                assert_eq!(
                    response.headers()[axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN],
                    origin
                );
                assert_eq!(
                    response.headers()[axum::http::header::ACCESS_CONTROL_ALLOW_METHODS],
                    "GET, POST, PUT, PATCH, DELETE"
                );
                assert_eq!(
                    response.headers()[axum::http::header::ACCESS_CONTROL_ALLOW_HEADERS],
                    "authorization, content-type, accept, last-event-id"
                );
                assert_eq!(
                    response.headers()[axum::http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS],
                    "true"
                );
            } else {
                assert!(
                    response
                        .headers()
                        .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                        .is_none()
                );
            }
        }

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(axum::http::header::ORIGIN, "http://localhost:4321")
                    .header(axum::http::header::HOST, "localhost:4321")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "http://localhost:4321"
        );
        assert_eq!(
            response.headers()[axum::http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS],
            "true"
        );
    }

    #[tokio::test]
    async fn remote_default_https_port_works_for_requests_and_preflight() {
        let policy = OriginHostPolicy::for_listener(
            "0.0.0.0:4321".parse().unwrap(),
            false,
            RemoteHostPolicy::parse(&["example.com:443".into()]).unwrap(),
        );
        let app = Router::new()
            .route("/", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn(enforce_origin_host))
            .layer(axum::Extension(policy));

        for (method, expected) in [
            (Method::GET, StatusCode::OK),
            (Method::OPTIONS, StatusCode::NO_CONTENT),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/")
                        .header(axum::http::header::ORIGIN, "https://example.com")
                        .header(axum::http::header::HOST, "example.com")
                        .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
    }

    #[tokio::test]
    async fn session_exchange_accepts_master_and_one_time_proof() {
        let session = BrowserSessionState::new(Some([0x2B; 32]), [0x2C; 32]);
        let master_request = Request::builder()
            .header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", URL_SAFE_NO_PAD.encode(TEST_SECRET)),
            )
            .body(axum::body::Body::empty())
            .unwrap();
        let response = session_exchange(
            master_request,
            SecurityContext::from_secret(TEST_SECRET),
            session.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let proof_request = Request::builder()
            .header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", URL_SAFE_NO_PAD.encode([0x2B; 32])),
            )
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            session_exchange(
                proof_request,
                SecurityContext::from_secret(TEST_SECRET),
                session.clone(),
            )
            .await
            .status(),
            StatusCode::NO_CONTENT,
        );

        let replay_request = Request::builder()
            .header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", URL_SAFE_NO_PAD.encode([0x2B; 32])),
            )
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            session_exchange(
                replay_request,
                SecurityContext::from_secret(TEST_SECRET),
                session,
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED,
        );
    }
}
