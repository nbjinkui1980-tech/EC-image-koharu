use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::Request;
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
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
            if trimmed.is_empty() || trimmed.contains('*') || trimmed.contains('@') {
                anyhow::bail!("invalid remote host: {trimmed:?}");
            }
            let authority = trimmed
                .parse::<Authority>()
                .map_err(|_| anyhow::anyhow!("invalid remote host: {trimmed:?}"))?;
            if !has_valid_optional_port(&authority) {
                anyhow::bail!("invalid remote host: {trimmed:?}");
            }
            authorities.insert(normalize_https_authority(&authority));
        }
        Ok(Self { authorities })
    }

    fn accepts(&self, host: &str) -> bool {
        host.parse::<Authority>().is_ok_and(|authority| {
            has_valid_optional_port(&authority)
                && self
                    .authorities
                    .contains(&normalize_https_authority(&authority))
        })
    }
}

fn normalize_https_authority(authority: &Authority) -> String {
    let authority = authority.as_str().to_ascii_lowercase();
    authority
        .strip_suffix(":443")
        .unwrap_or(&authority)
        .to_owned()
}

fn has_valid_optional_port(authority: &Authority) -> bool {
    let raw = authority.as_str();
    let port = if raw.starts_with('[') {
        let Some(end) = raw.find(']') else {
            return false;
        };
        match &raw[end + 1..] {
            "" => return true,
            suffix => suffix.strip_prefix(':'),
        }
    } else {
        raw.rsplit_once(':').map(|(_, port)| port)
    };
    port.is_none_or(|port| !port.is_empty() && port.parse::<u16>().is_ok())
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
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let host = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();

    if request.method() == Method::OPTIONS && !origin.is_empty() {
        if is_allowed(policy, &origin, &host) {
            let mut response = StatusCode::NO_CONTENT.into_response();
            let headers = response.headers_mut();
            headers.insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_str(&origin).expect("Origin is already a valid header value"),
            );
            headers.insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
            );
            headers.insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("Authorization, Content-Type"),
            );
            headers.insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                HeaderValue::from_static("true"),
            );
            return response;
        }
        return StatusCode::FORBIDDEN.into_response();
    }

    if is_allowed(policy, &origin, &host) {
        let mut response = next.run(request).await;
        if !origin.is_empty() {
            response.headers_mut().insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_str(&origin).expect("Origin is already a valid header value"),
            );
            response.headers_mut().insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                HeaderValue::from_static("true"),
            );
        }
        return response;
    }
    StatusCode::FORBIDDEN.into_response()
}

fn is_allowed(policy: Option<&OriginHostPolicy>, origin: &str, host: &str) -> bool {
    let Some(policy) = policy else {
        return true;
    };
    if !is_loopback_host(host, policy.loopback_port) && !policy.remotes.accepts(host) {
        return false;
    }
    if origin.is_empty() {
        return true;
    }
    if origin == "null" || origin == "*" {
        return false;
    }
    if let Some(origin_authority) = origin
        .strip_prefix("http://")
        .and_then(|authority| authority.parse::<Authority>().ok())
    {
        if origin_authority.host() == "127.0.0.1"
            || origin_authority.host().eq_ignore_ascii_case("localhost")
        {
            if !is_loopback_host(host, policy.loopback_port) {
                return false;
            }
            return (is_loopback_origin(origin, policy.loopback_port)
                && origin_authority.as_str().eq_ignore_ascii_case(host))
                || (policy.is_debug && origin == "http://localhost:3000");
        }
        return false;
    }
    if let Some(origin_authority) = origin
        .strip_prefix("https://")
        .and_then(|authority| authority.parse::<Authority>().ok())
    {
        let Ok(host_authority) = host.parse::<Authority>() else {
            return false;
        };
        return normalize_https_authority(&origin_authority)
            == normalize_https_authority(&host_authority)
            && policy.remotes.accepts(host);
    }
    false
}

fn is_loopback_origin(origin: &str, port: u16) -> bool {
    origin == format!("http://127.0.0.1:{port}") || origin == format!("http://localhost:{port}")
}

fn is_loopback_host(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") || host.eq_ignore_ascii_case(&format!("localhost:{port}"))
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

    #[test]
    fn origin_host_policy_covers_security_boundaries() {
        let remote =
            RemoteHostPolicy::parse(&["example.com".into(), "example.com:8443".into()]).unwrap();
        let release = OriginHostPolicy::for_listener(
            "127.0.0.1:4321".parse().unwrap(),
            false,
            remote.clone(),
        );
        let debug = OriginHostPolicy::for_listener("127.0.0.1:4321".parse().unwrap(), true, remote);

        for (name, policy, origin, host, expected) in [
            (
                "missing origin with wrong host",
                &release,
                "",
                "evil.test",
                false,
            ),
            ("null", &release, "null", "127.0.0.1:4321", false),
            ("wildcard", &release, "*", "127.0.0.1:4321", false),
            (
                "loopback port",
                &release,
                "http://127.0.0.1:4321",
                "127.0.0.1:4321",
                true,
            ),
            (
                "wrong loopback port",
                &release,
                "http://localhost:4322",
                "localhost:4321",
                false,
            ),
            (
                "loopback alias mismatch",
                &release,
                "http://localhost:4321",
                "127.0.0.1:4321",
                false,
            ),
            (
                "loopback host mismatch",
                &release,
                "http://localhost:4321",
                "attacker.example:4321",
                false,
            ),
            (
                "release debug origin",
                &release,
                "http://localhost:3000",
                "localhost:4321",
                false,
            ),
            (
                "debug origin",
                &debug,
                "http://localhost:3000",
                "localhost:4321",
                true,
            ),
            (
                "remote https default port",
                &release,
                "https://example.com",
                "example.com",
                true,
            ),
            (
                "remote https explicit port",
                &release,
                "https://example.com:8443",
                "example.com:8443",
                true,
            ),
            (
                "remote https default port normalization",
                &release,
                "https://example.com",
                "example.com:443",
                true,
            ),
            (
                "host mismatch",
                &release,
                "https://example.com",
                "other.example.com",
                false,
            ),
            (
                "explicit port mismatch",
                &release,
                "https://example.com:8443",
                "example.com",
                false,
            ),
        ] {
            assert_eq!(is_allowed(Some(policy), origin, host), expected, "{name}");
        }
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
                    "GET, POST, PUT, PATCH, DELETE, OPTIONS"
                );
                assert_eq!(
                    response.headers()[axum::http::header::ACCESS_CONTROL_ALLOW_HEADERS],
                    "Authorization, Content-Type"
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
