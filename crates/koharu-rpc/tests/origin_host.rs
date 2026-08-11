use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use base64::Engine;
use koharu_rpc::security::{OriginHostPolicy, RemoteHostPolicy, SecurityContext};
use koharu_rpc::server::{self, AssetResolver};
use koharu_runtime::{ComputePolicy, RuntimeManager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

const TEST_SECRET: [u8; 32] = [0x2A; 32];
const METHODS: &str = "GET, POST, PUT, PATCH, DELETE";
const HEADERS: &str = "authorization, content-type, accept, last-event-id";

struct TestServer {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
    root: std::path::PathBuf,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn spawn_server(
    debug: bool,
    remotes: RemoteHostPolicy,
    policy_ip: Option<IpAddr>,
    assets: bool,
) -> TestServer {
    let root = std::env::temp_dir().join(format!("koharu-rpc-origin-{}", Uuid::new_v4()));
    let app = koharu_rpc::BootstrapManager::new(Arc::new(
        RuntimeManager::new(&root, ComputePolicy::CpuOnly).unwrap(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let policy_addr = SocketAddr::new(policy_ip.unwrap_or(addr.ip()), addr.port());
    let policy = OriginHostPolicy::for_listener(policy_addr, debug, remotes);
    let security = SecurityContext::from_secret(TEST_SECRET);
    let router = if assets {
        let resolver: AssetResolver = Arc::new(|path| {
            (path == "index.html").then(|| (b"<main>Koharu</main>".to_vec(), "text/html".into()))
        });
        server::router_with_assets(app, security, policy, resolver)
    } else {
        server::router_for(app, security, policy)
    };
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    TestServer { addr, task, root }
}

async fn spawn_session_asset_server() -> (TestServer, String) {
    let root = std::env::temp_dir().join(format!("koharu-rpc-origin-{}", Uuid::new_v4()));
    let app = koharu_rpc::BootstrapManager::new(Arc::new(
        RuntimeManager::new(&root, ComputePolicy::CpuOnly).unwrap(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let policy = OriginHostPolicy::for_listener(addr, false, RemoteHostPolicy::empty());
    let security = SecurityContext::from_secret(TEST_SECRET);
    let session = koharu_rpc::security::BrowserSessionState::new(None, [0x2C; 32]);
    let cookie = session.session_token_encoded();
    let resolver: AssetResolver = Arc::new(|path| {
        (path == "index.html").then(|| (b"<main>Koharu</main>".to_vec(), "text/html".into()))
    });
    let task = tokio::spawn(async move {
        server::serve_with_listener_and_assets_with_session(
            listener, app, security, policy, session, resolver,
        )
        .await
        .unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    (TestServer { addr, task, root }, cookie)
}

async fn spawn_vary_server(vary: &'static str) -> TestServer {
    let root = std::env::temp_dir().join(format!("koharu-rpc-origin-{}", Uuid::new_v4()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let policy = OriginHostPolicy::for_listener(addr, true, RemoteHostPolicy::empty());
    let router = axum::Router::new()
        .route(
            "/vary",
            axum::routing::get(move || async move {
                let mut response = axum::response::Response::new(axum::body::Body::empty());
                response.headers_mut().insert(
                    axum::http::header::VARY,
                    axum::http::HeaderValue::from_static(vary),
                );
                response
            }),
        )
        .layer(axum::middleware::from_fn(
            koharu_rpc::security::enforce_origin_host,
        ))
        .layer(axum::Extension(policy));
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    TestServer { addr, task, root }
}

async fn request_head(
    addr: SocketAddr,
    method: &str,
    path: &str,
    host: &str,
    headers: &[(&str, &str)],
) -> (u16, String) {
    let version = if host.is_empty() {
        "HTTP/1.0"
    } else {
        "HTTP/1.1"
    };
    let mut request = format!("{method} {path} {version}\r\n");
    if !host.is_empty() {
        request.push_str(&format!("Host: {host}\r\n"));
    }
    request.push_str("Connection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");

    request_bytes(addr, request.as_bytes()).await
}

async fn request_bytes(addr: SocketAddr, request: &[u8]) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request).await.unwrap();
    let mut response = Vec::new();
    loop {
        let mut chunk = [0; 1024];
        let read = tokio::time::timeout(std::time::Duration::from_secs(2), stream.read(&mut chunk))
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
    let head = String::from_utf8_lossy(&response).into_owned();
    let status = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, head)
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (candidate, value) = line.split_once(':')?;
        candidate
            .eq_ignore_ascii_case(name)
            .then(|| value.trim_end_matches('\r').trim())
    })
}

fn header_values<'a>(head: &'a str, name: &str) -> Vec<&'a str> {
    head.lines()
        .filter_map(|line| {
            let (candidate, value) = line.split_once(':')?;
            candidate
                .eq_ignore_ascii_case(name)
                .then(|| value.trim_end_matches('\r').trim())
        })
        .collect()
}

fn master_header() -> String {
    format!(
        "Bearer {}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(TEST_SECRET)
    )
}

#[tokio::test]
async fn no_origin_still_rejects_forged_host_before_auth() {
    let server = spawn_server(false, RemoteHostPolicy::empty(), None, false).await;
    let (status, head) =
        request_head(server.addr, "GET", "/api/v1/meta", "evil.example", &[]).await;
    assert_eq!(status, 403);
    assert!(header(&head, "access-control-allow-origin").is_none());

    assert_eq!(
        request_head(server.addr, "GET", "/api/v1/meta", "", &[])
            .await
            .0,
        403
    );

    let mut non_utf8 = b"GET /api/v1/meta HTTP/1.1\r\nHost: ".to_vec();
    non_utf8.push(0xff);
    non_utf8.extend_from_slice(b"\r\nConnection: close\r\n\r\n");
    assert!([400, 403].contains(&request_bytes(server.addr, &non_utf8).await.0));

    let host = server.addr.to_string();
    assert!(
        [400, 403].contains(
            &request_head(
                server.addr,
                "GET",
                "/api/v1/meta",
                &host,
                &[("Host", &host)],
            )
            .await
            .0
        )
    );

    let (status, head) = request_head(server.addr, "GET", "/api/v1/meta", &host, &[]).await;
    assert_eq!(status, 401);
    assert!(header(&head, "access-control-allow-origin").is_none());
    assert_eq!(header(&head, "vary"), Some("Origin"));

    let wrong_port = format!("127.0.0.1:{}", server.addr.port() + 1);
    assert_eq!(
        request_head(server.addr, "GET", "/api/v1/meta", &wrong_port, &[])
            .await
            .0,
        403
    );

    let origin = format!("http://{host}");
    assert_eq!(
        request_head(
            server.addr,
            "OPTIONS",
            "*",
            &host,
            &[
                ("Origin", &origin),
                ("Access-Control-Request-Method", "GET"),
            ],
        )
        .await
        .0,
        403
    );
    let (status, head) = request_head(
        server.addr,
        "GET",
        "/api/v1/meta",
        &host,
        &[("Origin", &origin)],
    )
    .await;
    assert_eq!(status, 401);
    assert_eq!(
        header(&head, "access-control-allow-origin"),
        Some(origin.as_str())
    );

    let absolute = format!("http://{host}/api/v1/meta");
    assert_eq!(
        request_head(server.addr, "GET", &absolute, "", &[]).await.0,
        401
    );
    assert_eq!(
        request_head(server.addr, "GET", &absolute, &host, &[])
            .await
            .0,
        401
    );
    assert_eq!(
        request_head(
            server.addr,
            "GET",
            "http://evil.example/api/v1/meta",
            &host,
            &[],
        )
        .await
        .0,
        403
    );
    let non_http = format!("ftp://{host}/api/v1/meta");
    assert_eq!(
        request_head(server.addr, "GET", &non_http, "", &[]).await.0,
        403
    );

    let master = master_header();
    assert_eq!(
        request_head(
            server.addr,
            "GET",
            "/api/v1/meta",
            &host,
            &[("Authorization", &master)],
        )
        .await
        .0,
        503
    );
}

#[tokio::test]
async fn preflight_returns_only_exact_credentialed_cors_headers() {
    let server = spawn_server(true, RemoteHostPolicy::empty(), None, false).await;
    let host = server.addr.to_string();
    let origin = "http://localhost:3000";
    let (status, head) = request_head(
        server.addr,
        "OPTIONS",
        "/api/v1/meta",
        &host,
        &[
            ("Origin", origin),
            ("Access-Control-Request-Method", "PATCH"),
            (
                "Access-Control-Request-Headers",
                "authorization, content-type",
            ),
        ],
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(header(&head, "access-control-allow-origin"), Some(origin));
    assert_eq!(header(&head, "vary"), Some("Origin"));
    assert_eq!(header(&head, "cache-control"), Some("no-store"));
    assert_eq!(
        header(&head, "access-control-allow-credentials"),
        Some("true")
    );
    assert_eq!(header(&head, "access-control-allow-methods"), Some(METHODS));
    assert_eq!(header(&head, "access-control-allow-headers"), Some(HEADERS));

    for headers in [
        vec![
            ("Origin", origin),
            ("Access-Control-Request-Method", "TRACE"),
        ],
        vec![
            ("Origin", origin),
            ("Access-Control-Request-Method", "PATCH"),
            ("Access-Control-Request-Headers", "x-evil"),
        ],
    ] {
        let (status, head) =
            request_head(server.addr, "OPTIONS", "/api/v1/meta", &host, &headers).await;
        assert_eq!(status, 403, "{headers:?}");
        assert!(header(&head, "access-control-allow-origin").is_none());
        assert_eq!(header(&head, "vary"), Some("Origin"));
        assert_eq!(header(&head, "cache-control"), Some("no-store"));
    }

    let (status, head) = request_head(
        server.addr,
        "GET",
        "/api/v1/meta",
        &host,
        &[("Origin", origin), ("Origin", origin)],
    )
    .await;
    assert_eq!(status, 403);
    assert!(header(&head, "access-control-allow-origin").is_none());

    for invalid in [
        "null",
        "*",
        "ftp://localhost:3000",
        "http://localhost:3000/path",
        "http://localhost:3000?query",
        "http://localhost:3000#fragment",
    ] {
        let (status, head) = request_head(
            server.addr,
            "GET",
            "/api/v1/meta",
            &host,
            &[("Origin", invalid)],
        )
        .await;
        assert_eq!(status, 403, "{invalid}");
        assert!(header(&head, "access-control-allow-origin").is_none());
        assert_eq!(header(&head, "vary"), Some("Origin"));
        assert_eq!(header(&head, "cache-control"), Some("no-store"));
    }
}

#[tokio::test]
async fn non_preflight_cors_preserves_existing_vary() {
    for existing in ["Accept-Encoding", "Accept-Encoding, Origin"] {
        let server = spawn_vary_server(existing).await;
        let host = server.addr.to_string();
        let origin = "http://localhost:3000";
        let (status, head) =
            request_head(server.addr, "GET", "/vary", &host, &[("Origin", origin)]).await;
        assert_eq!(status, 200);
        assert_eq!(header(&head, "access-control-allow-origin"), Some(origin));
        assert_eq!(
            header(&head, "access-control-allow-credentials"),
            Some("true")
        );
        let vary = header_values(&head, "vary");
        let tokens: Vec<_> = vary
            .iter()
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .collect();
        assert_eq!(
            tokens
                .iter()
                .filter(|value| value.eq_ignore_ascii_case("Origin"))
                .count(),
            1
        );
        assert!(
            tokens
                .iter()
                .any(|value| value.eq_ignore_ascii_case("Accept-Encoding"))
        );
    }

    let server = spawn_vary_server("Accept-Encoding").await;
    let host = server.addr.to_string();
    let (status, head) = request_head(server.addr, "GET", "/vary", &host, &[]).await;
    assert_eq!(status, 200);
    assert!(header(&head, "access-control-allow-origin").is_none());
    assert!(header_values(&head, "vary").iter().any(|value| {
        value
            .split(',')
            .map(str::trim)
            .any(|token| token.eq_ignore_ascii_case("Origin"))
    }));
}

#[tokio::test]
async fn remote_authority_normalizes_default_port_case_and_ipv6_without_userinfo() {
    for invalid in [
        "",
        "*",
        "attacker@example.com",
        "https://example.com",
        "example.com/path",
        "example.com?query",
        "example.com#fragment",
        "example.com:",
        "example.com:65536",
        ":443",
        "[]",
        "[not-an-ip]:443",
        "[2001:db8::1]junk",
        "[2001:db8::1]junk:443",
        "[::1]junk",
        "[::1]junk:443",
    ] {
        assert!(
            RemoteHostPolicy::parse(&[invalid.into()]).is_err(),
            "{invalid}"
        );
    }
    for valid in ["[::1]", "[::1]:443", "[2001:db8::1]:443"] {
        assert!(RemoteHostPolicy::parse(&[valid.into()]).is_ok(), "{valid}");
    }

    let remotes = RemoteHostPolicy::parse(&["example.com".into()]).unwrap();
    let server = spawn_server(false, remotes, Some("0.0.0.0".parse().unwrap()), false).await;
    let master = master_header();
    for (host, origin) in [
        ("EXAMPLE.COM:443", "https://example.com"),
        ("example.com", "https://EXAMPLE.COM:443"),
    ] {
        assert_eq!(
            request_head(
                server.addr,
                "GET",
                "/api/v1/meta",
                host,
                &[("Origin", origin), ("Authorization", &master)],
            )
            .await
            .0,
            503
        );
    }
    for (host, origin) in [
        ("attacker@example.com", "https://example.com"),
        ("example.com/path", "https://example.com"),
        ("example.com?query", "https://example.com"),
        ("example.com#fragment", "https://example.com"),
        ("example.com", "https://attacker@example.com"),
        ("example.com", "https://example.com/path"),
        ("example.com", "https://example.com?query"),
        ("example.com", "https://example.com#fragment"),
        ("example.com:", "https://example.com"),
        ("example.com:65536", "https://example.com"),
        ("example.com", "https://example.com:"),
        ("example.com", "https://example.com:65536"),
        ("example.com:444", "https://example.com:444"),
        ("127.0.0.1", "http://127.0.0.1"),
    ] {
        assert_eq!(
            request_head(
                server.addr,
                "GET",
                "/api/v1/meta",
                host,
                &[("Origin", origin), ("Authorization", &master)],
            )
            .await
            .0,
            403,
            "{host} {origin}"
        );
    }

    let remotes = RemoteHostPolicy::parse(&["[2001:0db8::1]:443".into()]).unwrap();
    let server = spawn_server(false, remotes, Some("0.0.0.0".parse().unwrap()), false).await;
    for (host, origin) in [
        ("[2001:db8::1]junk", "https://[2001:db8::1]:443"),
        ("[2001:db8::1]junk:443", "https://[2001:db8::1]:443"),
        ("[2001:db8::1]:443", "https://[2001:db8::1]junk"),
        ("[2001:db8::1]:443", "https://[2001:db8::1]junk:443"),
    ] {
        assert_eq!(
            request_head(
                server.addr,
                "GET",
                "/api/v1/meta",
                host,
                &[("Origin", origin), ("Authorization", &master)],
            )
            .await
            .0,
            403,
            "{host} {origin}"
        );
    }
    assert_eq!(
        request_head(
            server.addr,
            "GET",
            "/api/v1/meta",
            "[2001:DB8::1]",
            &[
                ("Origin", "https://[2001:db8::1]:443"),
                ("Authorization", &master),
            ],
        )
        .await
        .0,
        503
    );

    let server = spawn_server(
        false,
        RemoteHostPolicy::empty(),
        Some("::1".parse().unwrap()),
        false,
    )
    .await;
    let host = format!("[::1]:{}", server.addr.port());
    let origin = format!("http://[0:0:0:0:0:0:0:1]:{}", server.addr.port());
    assert_eq!(
        request_head(
            server.addr,
            "GET",
            "/api/v1/meta",
            &host,
            &[("Origin", &origin), ("Authorization", &master)],
        )
        .await
        .0,
        503
    );
}

#[tokio::test]
async fn outer_policy_wraps_mcp_and_static_assets() {
    let server = spawn_server(false, RemoteHostPolicy::empty(), None, true).await;
    for path in ["/api/v1/meta", "/mcp", "/"] {
        assert_eq!(
            request_head(server.addr, "GET", path, "evil.example", &[])
                .await
                .0,
            403,
            "{path}"
        );
    }

    let host = server.addr.to_string();
    assert_eq!(
        request_head(server.addr, "GET", "/api/v1/meta", &host, &[])
            .await
            .0,
        401
    );
    let (status, head) = request_head(server.addr, "GET", "/", &host, &[]).await;
    assert_eq!(status, 200);
    assert_eq!(header(&head, "vary"), Some("Origin"));
    assert_eq!(
        request_head(server.addr, "GET", "/mcp", &host, &[]).await.0,
        401
    );
    for path in ["/api/v1/meta", "/mcp", "/"] {
        let (status, head) = request_head(
            server.addr,
            "GET",
            path,
            &host,
            &[("Origin", "https://evil.example")],
        )
        .await;
        assert_eq!(status, 403, "{path}");
        assert!(header(&head, "access-control-allow-origin").is_none());
    }
}

#[tokio::test]
async fn desktop_session_asset_server_keeps_policy_outermost() {
    let (server, cookie) = spawn_session_asset_server().await;
    for path in ["/", "/mcp", "/api/v1/meta"] {
        assert_eq!(
            request_head(server.addr, "GET", path, "evil.example", &[])
                .await
                .0,
            403,
            "forged Host reached {path}"
        );
    }

    let host = server.addr.to_string();
    for path in ["/", "/mcp", "/api/v1/meta"] {
        let (status, head) = request_head(
            server.addr,
            "GET",
            path,
            &host,
            &[("Origin", "https://evil.example")],
        )
        .await;
        assert_eq!(status, 403, "evil Origin reached {path}");
        assert!(header(&head, "access-control-allow-origin").is_none());
    }

    assert_eq!(
        request_head(server.addr, "GET", "/", &host, &[]).await.0,
        200
    );
    let cookie_header = format!("koharu_session={cookie}");
    assert_eq!(
        request_head(
            server.addr,
            "GET",
            "/api/v1/meta",
            &host,
            &[("Cookie", &cookie_header)],
        )
        .await
        .0,
        503
    );
    assert_eq!(
        request_head(
            server.addr,
            "GET",
            "/mcp",
            &host,
            &[("Cookie", &cookie_header)],
        )
        .await
        .0,
        401
    );

    let master = master_header();
    let (status, head) = request_head(
        server.addr,
        "POST",
        "/api/v1/auth/session",
        &host,
        &[("Authorization", &master), ("Content-Length", "0")],
    )
    .await;
    assert_eq!(status, 204);
    assert!(header(&head, "set-cookie").is_some());
}
