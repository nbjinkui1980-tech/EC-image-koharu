# AR01 R03 Host Origin and CORS Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Protect every API business route and the complete API/MCP/static router with one structurally parsed Host/Origin/CORS policy while preserving session exchange, MCP Bearer-only authentication, readiness ordering, and public static assets.

**Architecture:** R03A moves bootstrap business routes behind the existing Bearer-or-session middleware while leaving `/api/v1/auth/session` self-authenticated. R03B resolves request authority from the single `Host` header and `request.uri().authority()`, rejects absent or conflicting sources, validates bracketed-authority suffixes and IP literals before trusting `axum::http::uri::Authority`, validates authority before Origin, emits exact credentialed CORS/cache headers, and installs the policy outside the fully assembled API/MCP/assets router.

**Tech Stack:** Rust 2024, Axum 0.8, Tokio, `http::uri::Authority`, existing Base64/session security code, raw TCP boundary tests.

**Status:** DRAFT — independent review and explicit user approval are required before execution.

## Global Constraints

- Execute only in `/Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay` on `codex/audit-remediation-sdd`.
- Product-code baseline is `9e46ab84d5afceb2417a55b8cadda000ffd5580f`; preserve `31e4d8c7` and `9e46ab84`. A documentation-only checkpoint after this baseline is allowed.
- Frozen reviewed R03A RED patch digest is `c104c665cffffd917560aad5d4b75e77c2e38ead399a81c47d341c5150cd9883`, computed from `git diff -- crates/koharu-rpc/src/api.rs` before any R03 implementation.
- Follow `docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md` in full.
- The execution handoff must supply independently approved contract and plan SHA-256 values; Task 0 rejects any mismatch before recording evidence.
- `KOHARU_SHARED_TARGET_DIR` and `CARGO_TARGET_DIR` must both equal `/Volumes/G/EC-image-koharu/target`.
- One owner executes R03A then R03B serially. Each task gets its own RED, GREEN, independent review, and commit.
- Do not change dependencies, manifests, lockfiles, generated files, `.omc/`, `.omo/`, `ui/next.config.ts`, Tauri capabilities, UI code, Desktop code, Headless code, or integration-harness code.
- The evidence ledger remains unstaged until the final evidence card.
- Frozen CORS methods: `GET, POST, PUT, PATCH, DELETE`.
- Frozen CORS request headers: `authorization, content-type, accept, last-event-id`.
- Frozen preflight cache policy: `Cache-Control: no-store`.
- Status ordering: Host/Origin rejection `403`; auth rejection `401`; correct auth on an unready app `503`.

### Frozen serving and deployment boundary

- R03 guarantees Host/Origin enforcement only for the shipped `server::*` serving constructors. The public raw `api()`/`router()`/`router_with_session()` surfaces, the public `api` module, and `mcp::mount` remain unsupported serving escape hatches; narrowing them requires a later independent card and is not part of R03.
- Current production Desktop and Headless entrypoints must call `server::*`; Task 3 includes a read-only call-site guard over `crates/koharu/src` so a new direct raw-router serving path blocks closeout.
- Desktop defaults to a loopback listener with same-origin HTTP and the frozen debug origin `http://localhost:3000`. Because the current CLI also accepts `--host` outside Headless mode, R06 must reject or explicitly validate non-loopback Desktop binding before bind; R03 does not claim loopback is already enforced.
- Remote Headless uses a non-loopback or wildcard backend bind, an explicit host allowlist, and a trusted HTTPS reverse proxy for external TLS termination. A proxy may connect through `127.0.0.1` to a backend bound on `0.0.0.0`, but a loopback-only bind does not select the remote exposure profile.
- The trusted reverse proxy must preserve the external `Host` and `Origin` values. R03 validates application-layer authority/Origin/CORS behavior only; it does not claim TLS transport verification. R06 retains validation-before-bind ownership for secret, allowlist, Desktop `--host`, and exposure selection.

---

## File Ownership Map

| File | Responsibility | Task |
|---|---|---|
| `crates/koharu-rpc/src/api.rs` | Put bootstrap business routes behind existing Bearer-or-session auth; retain self-authenticated exchange | R03A |
| `crates/koharu-rpc/src/security.rs` | Structural authority parsing, Host-before-Origin enforcement, exact preflight/CORS behavior | R03B |
| `crates/koharu-rpc/src/server.rs` | Assemble API, MCP, and optional assets before adding the outer Host/Origin layer | R03B |
| `crates/koharu-rpc/tests/origin_host.rs` | Real-listener/raw-TCP transport boundary regression suite | R03B |
| `docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md` | Append evidence only; never stage during R03 | R03A, R03B |

---

### Task 0: Freeze the reviewed execution inputs

**Files:**
- Read: `docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md`
- Read: `docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md`
- Read: `docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md`

**Interfaces:**
- Consumes: approved copies of the contract and this plan.
- Produces: external SHA-256 anchors and an exact pre-edit status record in the ledger.

- [ ] **Step 1: Run the environment preflight**

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
test -f .env
set -a
source .env
set +a
test "$(git branch --show-current)" = codex/audit-remediation-sdd
git merge-base --is-ancestor 9e46ab84d5afceb2417a55b8cadda000ffd5580f HEAD
test "${KOHARU_SHARED_TARGET_DIR:?}" = /Volumes/G/EC-image-koharu/target
test "${CARGO_TARGET_DIR:?}" = "$KOHARU_SHARED_TARGET_DIR"
git diff --cached --quiet
R03_START_SHA="$(git rev-parse HEAD)"
printf 'R03_START_SHA=%s\n' "$R03_START_SHA"
git status --short --untracked-files=all
git diff --name-only 9e46ab84d5afceb2417a55b8cadda000ffd5580f..HEAD
test -z "$(git diff --name-only \
  9e46ab84d5afceb2417a55b8cadda000ffd5580f..HEAD -- . \
  ':!docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md' \
  ':!docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md')"
test -z "$(git log --format= --name-only \
  9e46ab84d5afceb2417a55b8cadda000ffd5580f..HEAD | sed '/^$/d' | sort -u | grep -Ev \
  '^(docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract\.md|docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan\.md)$' || true)"
test -z "$(git status --porcelain=v1 --untracked-files=all | cut -c4- | grep -Ev \
  '^(crates/koharu-rpc/src/api\.rs|docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger\.md|docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract\.md|docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan\.md)$' || true)"
```

Expected: the index is empty; the pre-existing tracked modifications are `crates/koharu-rpc/src/api.rs` and the evidence ledger; the two plan documents may be untracked or may be the only paths committed after the product-code baseline. Any staged path or product/test path committed after the baseline is a hard stop. Preserve the printed `R03_START_SHA` exactly.

- [ ] **Step 2: Record immutable plan identities outside the plans**

```bash
set -euo pipefail
LEDGER=docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
CONTRACT=docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md
PLAN=docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md
: "${R03_APPROVED_CONTRACT_SHA256:?supply the independently approved contract SHA-256}"
: "${R03_APPROVED_PLAN_SHA256:?supply the independently approved plan SHA-256}"
test "$(shasum -a 256 "$CONTRACT" | awk '{print $1}')" = "$R03_APPROVED_CONTRACT_SHA256"
test "$(shasum -a 256 "$PLAN" | awk '{print $1}')" = "$R03_APPROVED_PLAN_SHA256"
test "$(grep -c '^R03_START_SHA=' "$LEDGER" || true)" = 0
test "$(grep -c '^R03_CONTRACT_SHA256=' "$LEDGER" || true)" = 0
test "$(grep -c '^R03_PLAN_SHA256=' "$LEDGER" || true)" = 0
test "$(grep -c '^R03A_RED_PATCH_SHA256=' "$LEDGER" || true)" = 0
R03A_RED_PATCH_SHA256="$(git diff -- crates/koharu-rpc/src/api.rs | shasum -a 256 | awk '{print $1}')"
test "$R03A_RED_PATCH_SHA256" = c104c665cffffd917560aad5d4b75e77c2e38ead399a81c47d341c5150cd9883
printf 'R03_CONTRACT_SHA256=%s\n' "$(shasum -a 256 "$CONTRACT" | awk '{print $1}')"
printf 'R03_PLAN_SHA256=%s\n' "$(shasum -a 256 "$PLAN" | awk '{print $1}')"
printf 'R03A_RED_PATCH_SHA256=%s\n' "$R03A_RED_PATCH_SHA256"
```

The execution handoff must supply the two approved SHA-256 values; Task 0 never derives approval from the files it is about to execute. Append the three printed identity lines verbatim, the reviewer verdict tied to those exact values, the approval timestamp, and the frozen start marker as a literal standalone line `R03_START_SHA=<40-hex HEAD from Step 1>` to the evidence ledger. The four `R03_*` identity markers are write-once: Task 0 is not a resumable append step. If any marker already exists, stop and use the later read-only verification blocks rather than appending duplicates. Do not stage the ledger.

- [ ] **Step 3: Verify the current R03A dirty hunk is test-only**

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
set -a; source .env; set +a
test "$(git branch --show-current)" = codex/audit-remediation-sdd
test "${KOHARU_SHARED_TARGET_DIR:?}" = /Volumes/G/EC-image-koharu/target
test "${CARGO_TARGET_DIR:?}" = "$KOHARU_SHARED_TARGET_DIR"
git diff --cached --quiet
LEDGER=docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
CONTRACT=docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md
PLAN=docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md
test "$(grep -c '^R03_START_SHA=[0-9a-f]\{40\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_CONTRACT_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_PLAN_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03A_RED_PATCH_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(shasum -a 256 "$CONTRACT" | awk '{print $1}')" = \
  "$(sed -n 's/^R03_CONTRACT_SHA256=//p' "$LEDGER")"
test "$(shasum -a 256 "$PLAN" | awk '{print $1}')" = \
  "$(sed -n 's/^R03_PLAN_SHA256=//p' "$LEDGER")"
test "$(sed -n 's/^R03A_RED_PATCH_SHA256=//p' "$LEDGER")" = \
  c104c665cffffd917560aad5d4b75e77c2e38ead399a81c47d341c5150cd9883
R03_START_SHA="$(sed -n 's/^R03_START_SHA=//p' "$LEDGER")"
test "$R03_START_SHA" = "$(git rev-parse HEAD)"
test "$(git diff -- crates/koharu-rpc/src/api.rs | shasum -a 256 | awk '{print $1}')" = \
  "$(sed -n 's/^R03A_RED_PATCH_SHA256=//p' "$LEDGER")"
test -z "$(git status --porcelain=v1 --untracked-files=all | cut -c4- | grep -Ev \
  '^(crates/koharu-rpc/src/api\.rs|docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger\.md|docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract\.md|docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan\.md)$' || true)"
git diff -- crates/koharu-rpc/src/api.rs
```

Expected: all four immutable identity markers exist exactly once, `R03_START_SHA` is the current pre-implementation `HEAD`, both current documents match the reviewed digests, and the exact frozen test-only `api.rs` patch is present. If any identity or patch digest differs, stop; do not execute an unreviewed plan or manufacture a RED from an implementation-bearing baseline.

---

### Task 1: R03A — Authenticate bootstrap business routes

**Files:**
- Modify: `crates/koharu-rpc/src/api.rs`
- Evidence only: `docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md`

**Interfaces:**
- Consumes: existing `bootstrap_api()`, `app_api()`, `require_ready`, `require_auth`, `require_session_auth`, and `handle_session_exchange`.
- Produces: `router_inner(ApiState, SecurityContext, Option<BrowserSessionState>) -> Router` where only `/auth/session` is outside ordinary API auth.

- [ ] **Step 1: Complete the real-boundary RED test without changing product code**

Task 0 Step 3 must have passed immediately before this edit in the same execution session; any digest or dirty-state drift requires rerunning Task 0 Step 3 and stopping before product/test changes.

Inside the existing `#[cfg(test)] mod tests`, add this helper after `get_status`:

```rust
async fn request_head(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    host: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, String) {
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);

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
```

Replace the current incomplete `bootstrap_routes_require_authentication` test with:

```rust
#[tokio::test]
async fn bootstrap_routes_require_authentication() {
    let root = std::env::temp_dir().join(format!("koharu-rpc-auth-{}", Uuid::new_v4()));
    let app = crate::BootstrapManager::new(Arc::new(
        RuntimeManager::new(&root, ComputePolicy::CpuOnly).unwrap(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let policy = crate::security::OriginHostPolicy::for_listener(
        addr,
        false,
        crate::security::RemoteHostPolicy::empty(),
    );
    let session = crate::security::BrowserSessionState::new(None, [0x2C; 32]);
    let cookie = session.session_token_encoded();
    let server = tokio::spawn(crate::server::serve_with_listener_with_session(
        listener,
        app,
        SecurityContext::from_secret(TEST_SECRET),
        policy,
        session,
    ));
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let host = addr.to_string();
    let wrong = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    for path in ["/api/v1/events", "/api/v1/downloads", "/api/v1/operations"] {
        assert_eq!(get_status(addr, path, None, None).await, 401, "{path}");
        assert_eq!(
            get_status(addr, path, None, Some(wrong)).await,
            401,
            "{path}"
        );
    }

    let (status, _) = request_head(
        addr,
        "POST",
        "/api/v1/downloads",
        &host,
        &[("Content-Type", "application/json")],
        r#"{"modelId":"missing:test-model"}"#,
    )
    .await;
    assert_eq!(status, 401);

    let operation_id = format!("auth-cancel-{}", Uuid::new_v4());
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    crate::routes::operations::register_cancel(operation_id.clone(), cancelled.clone());
    let (status, _) = request_head(
        addr,
        "DELETE",
        &format!("/api/v1/operations/{operation_id}"),
        &host,
        &[],
        "",
    )
    .await;
    assert_eq!(status, 401);
    assert!(!cancelled.load(std::sync::atomic::Ordering::Relaxed));
    crate::routes::operations::unregister_cancel(&operation_id);

    let master = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(TEST_SECRET);
    for path in ["/api/v1/events", "/api/v1/downloads", "/api/v1/operations"] {
        assert_eq!(
            get_status(addr, path, None, Some(&master)).await,
            200,
            "{path}"
        );
        assert_eq!(
            get_status(addr, path, Some(&cookie), None).await,
            200,
            "{path}"
        );
    }
    for path in ["/api/v1/meta", "/api/v1/scene.bin"] {
        assert_eq!(
            get_status(addr, path, None, Some(&master)).await,
            503,
            "{path}"
        );
        assert_eq!(
            get_status(addr, path, Some(&cookie), None).await,
            503,
            "{path}"
        );
    }

    server.abort();
    let _ = server.await;
    let _ = std::fs::remove_dir_all(root);
}
```

- [ ] **Step 2: Compile RED-0**

```bash
bun cargo test -p koharu-rpc --lib bootstrap_routes_require_authentication --no-run
```

Expected: exit `0`; the test target compiles.

- [ ] **Step 3: Run RED-1**

```bash
set -euo pipefail
set +e
RED_OUTPUT="$(bun cargo test -p koharu-rpc --lib bootstrap_routes_require_authentication -- --nocapture 2>&1)"
RED_EXIT=$?
set -e
printf '%s\n' "$RED_OUTPUT"
test "$RED_EXIT" = 101
test "$(printf '%s\n' "$RED_OUTPUT" | grep -Ec '^running 1 test$')" = 1
printf '%s\n' "$RED_OUTPUT" | grep -Eq 'test result: FAILED\. 0 passed; 1 failed;'
```

Expected on starting HEAD: exit `101`; exactly one test runs and fails because `/api/v1/events` returns `200` instead of `401`. If it fails for compilation, timeout, or a different first assertion, stop and diagnose before implementation.

- [ ] **Step 4: Replace only `router_inner` with the minimal root-cause fix**

```rust
fn router_inner(
    app: ApiState,
    security: crate::security::SecurityContext,
    session: Option<crate::security::BrowserSessionState>,
) -> Router {
    let (bootstrap, _) = bootstrap_api().split_for_parts();
    let (guarded, _) = app_api().split_for_parts();
    let exchange_security = security.clone();
    let exchange = if let Some(session) = session.clone() {
        Router::new().route(
            "/auth/session",
            axum::routing::post(move |req: Request| {
                let security = exchange_security.clone();
                let session = session.clone();
                async move { handle_session_exchange(req, security, session).await }
            }),
        )
    } else {
        Router::new()
    };
    let guarded = guarded
        .with_state(app.clone())
        .layer(middleware::from_fn_with_state(app.clone(), require_ready));
    let protected = bootstrap.with_state(app).merge(guarded);
    let protected = if let Some(session) = session {
        protected.layer(middleware::from_fn_with_state(
            (security, session),
            crate::security::require_session_auth,
        ))
    } else {
        protected.layer(middleware::from_fn_with_state(
            security,
            crate::security::require_auth,
        ))
    };
    Router::new()
        .nest("/api/v1", exchange.merge(protected))
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
}
```

- [ ] **Step 5: Run GREEN and adjacent authentication regressions**

```bash
set -euo pipefail
rustfmt --edition 2024 crates/koharu-rpc/src/api.rs
assert_green_tests() {
  expected="$1"
  shift
  output="$("$@" 2>&1)"
  printf '%s\n' "$output"
  test "$(printf '%s\n' "$output" | grep -Ec "^running ${expected} tests?$")" = 1
  printf '%s\n' "$output" | grep -Eq "test result: ok\\. ${expected} passed;"
}
assert_green_tests 1 bun cargo test -p koharu-rpc --lib bootstrap_routes_require_authentication -- --nocapture
assert_green_tests 3 bun cargo test -p koharu-rpc --lib session_exchange -- --nocapture
assert_green_tests 1 bun cargo test -p koharu-rpc --lib mcp_remains_master_bearer_only -- --nocapture
bun cargo check -p koharu-rpc --all-targets
bun cargo clippy -p koharu-rpc --all-targets -- -D warnings
bun cargo fmt --all -- --check
git diff --check -- crates/koharu-rpc/src/api.rs
```

Expected: `1/1`, `3/3`, and `1/1` tests pass; check, Clippy, fmt, and diff-check exit `0`.

- [ ] **Step 6: Independent R03A review**

Run this full-scope guard immediately before dispatching review:

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
set -a; source .env; set +a
test "$(git branch --show-current)" = codex/audit-remediation-sdd
test "${KOHARU_SHARED_TARGET_DIR:?}" = /Volumes/G/EC-image-koharu/target
test "${CARGO_TARGET_DIR:?}" = "$KOHARU_SHARED_TARGET_DIR"
git diff --cached --quiet
LEDGER=docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
CONTRACT=docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md
PLAN=docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md
test "$(grep -c '^R03_START_SHA=[0-9a-f]\{40\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_CONTRACT_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_PLAN_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03A_RED_PATCH_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(shasum -a 256 "$CONTRACT" | awk '{print $1}')" = "$(sed -n 's/^R03_CONTRACT_SHA256=//p' "$LEDGER")"
test "$(shasum -a 256 "$PLAN" | awk '{print $1}')" = "$(sed -n 's/^R03_PLAN_SHA256=//p' "$LEDGER")"
test "$(sed -n 's/^R03A_RED_PATCH_SHA256=//p' "$LEDGER")" = c104c665cffffd917560aad5d4b75e77c2e38ead399a81c47d341c5150cd9883
test "$(sed -n 's/^R03_START_SHA=//p' "$LEDGER")" = "$(git rev-parse HEAD)"
test -z "$(git status --porcelain=v1 --untracked-files=all | cut -c4- | grep -Ev \
  '^(crates/koharu-rpc/src/api\.rs|docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger\.md|docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract\.md|docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan\.md)$' || true)"
```

Dispatch a read-only `code-reviewer` with the exact cached-free diff and command outputs. Review requirements:

- `/auth/session` remains outside ordinary API auth but still self-validates.
- GET/POST/DELETE bootstrap business routes are protected.
- unauthenticated DELETE has no cancellation side effect.
- master Bearer and session cookie both reach business handlers/readiness.
- MCP behavior is unchanged.

Unresolved HIGH/CRITICAL is a hard stop. Repair only `api.rs`, rerun Step 5, then request scoped re-review.

- [ ] **Step 7: Stage and commit only R03A**

Rerun the Step 6 full-scope guard verbatim immediately before this block and do not edit any file between that successful guard and staging.

```bash
git add crates/koharu-rpc/src/api.rs
test "$(git diff --cached --name-only)" = crates/koharu-rpc/src/api.rs
git diff --cached --check
git commit -m "fix(rpc): authenticate bootstrap business routes" \
  -m "Co-Authored-By: Codex <noreply@openai.com>"
git rev-parse HEAD
```

Append RED/GREEN/review/commit SHA to the ledger without staging it.

---

### Task 2: R03B — Enforce Host/Origin/CORS on the complete router

**Files:**
- Modify: `crates/koharu-rpc/src/security.rs`
- Modify: `crates/koharu-rpc/src/server.rs`
- Create: `crates/koharu-rpc/tests/origin_host.rs`
- Evidence only: `docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md`

**Interfaces:**
- Consumes: `RemoteHostPolicy::parse`, `OriginHostPolicy::for_listener`, `server::router_for`, and `server::router_with_assets`.
- Produces: structural authority normalization and one outer policy layer shared by API, MCP, and static fallback.

- [ ] **Step 1: Re-run the card preflight after the R03A commit**

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
test -f .env
set -a
source .env
set +a
test "$(git branch --show-current)" = codex/audit-remediation-sdd
git merge-base --is-ancestor 9e46ab84d5afceb2417a55b8cadda000ffd5580f HEAD
test "${KOHARU_SHARED_TARGET_DIR:?}" = /Volumes/G/EC-image-koharu/target
test "${CARGO_TARGET_DIR:?}" = "$KOHARU_SHARED_TARGET_DIR"
LEDGER=docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
CONTRACT=docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md
PLAN=docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md
test "$(grep -c '^R03_START_SHA=[0-9a-f]\{40\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_CONTRACT_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_PLAN_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03A_RED_PATCH_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(shasum -a 256 "$CONTRACT" | awk '{print $1}')" = \
  "$(sed -n 's/^R03_CONTRACT_SHA256=//p' "$LEDGER")"
test "$(shasum -a 256 "$PLAN" | awk '{print $1}')" = \
  "$(sed -n 's/^R03_PLAN_SHA256=//p' "$LEDGER")"
test "$(sed -n 's/^R03A_RED_PATCH_SHA256=//p' "$LEDGER")" = c104c665cffffd917560aad5d4b75e77c2e38ead399a81c47d341c5150cd9883
R03_START_SHA="$(sed -n 's/^R03_START_SHA=//p' \
  "$LEDGER")"
test "$(git rev-parse HEAD^)" = "$R03_START_SHA"
test "$(git log -1 --format=%s)" = "fix(rpc): authenticate bootstrap business routes"
test "$(git diff-tree --no-commit-id --name-only -r HEAD)" = crates/koharu-rpc/src/api.rs
git diff --cached --quiet
git status --short --untracked-files=all
test -z "$(git status --porcelain=v1 --untracked-files=all | cut -c4- | grep -Ev \
  '^(docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger\.md|docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract\.md|docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan\.md)$' || true)"
```

Expected: R03A is the current single-file commit, the index is empty, and only the unstaged ledger plus optional untracked plan documents remain. Any other dirty or staged path is a hard stop.

- [ ] **Step 2: Create the complete real-listener boundary test**

Create `crates/koharu-rpc/tests/origin_host.rs` with exactly:

```rust
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
    let version = if host.is_empty() { "HTTP/1.0" } else { "HTTP/1.1" };
    let mut request = format!("{method} {path} {version}\r\n");
    if !host.is_empty() {
        request.push_str(&format!("Host: {host}\r\n"));
    }
    request.push_str("Connection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
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

    let (status, head) = request_head(server.addr, "GET", "/api/v1/meta", "", &[]).await;
    assert_eq!(status, 403);
    assert!(header(&head, "access-control-allow-origin").is_none());

    let host = server.addr.to_string();
    let (status, head) = request_head(
        server.addr,
        "GET",
        "/api/v1/meta",
        &host,
        &[("Host", &host)],
    )
    .await;
    assert!([400, 403].contains(&status));
    assert!(header(&head, "access-control-allow-origin").is_none());

    let (status, head) = request_head(server.addr, "GET", "/api/v1/meta", &host, &[]).await;
    assert_eq!(status, 401);
    assert!(header(&head, "access-control-allow-origin").is_none());
    assert_eq!(header(&head, "vary"), Some("Origin"));

    let absolute = format!("http://{host}/api/v1/meta");
    let (status, head) = request_head(server.addr, "GET", &absolute, "", &[]).await;
    assert_eq!(status, 401, "URI authority without Host must be accepted");
    assert_eq!(header(&head, "vary"), Some("Origin"));
    assert_eq!(
        request_head(server.addr, "GET", &absolute, &host, &[])
            .await
            .0,
        401,
        "equivalent Host and URI authority must be accepted"
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
        403,
        "conflicting Host and URI authority must fail closed"
    );
    let non_http = format!("ftp://{host}/api/v1/meta");
    assert_eq!(
        request_head(server.addr, "GET", &non_http, "", &[]).await.0,
        403,
        "non-HTTP(S) URI scheme must fail closed"
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

    let (status, head) = request_head(
        server.addr,
        "OPTIONS",
        "/api/v1/meta",
        &host,
        &[
            ("Origin", origin),
            ("Access-Control-Request-Method", "PATCH"),
            ("Access-Control-Request-Headers", "x-evil"),
        ],
    )
    .await;
    assert_eq!(status, 403);
    assert!(header(&head, "access-control-allow-origin").is_none());
    assert_eq!(header(&head, "vary"), Some("Origin"));
    assert_eq!(header(&head, "cache-control"), Some("no-store"));

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
    assert_eq!(header(&head, "vary"), Some("Origin"));
    assert_eq!(header(&head, "cache-control"), Some("no-store"));

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

    let (status, head) = request_head(server.addr, "OPTIONS", "/api/v1/meta", &host, &[]).await;
    assert_eq!(status, 401);
    assert!(header(&head, "access-control-allow-origin").is_none());
    assert_eq!(header(&head, "vary"), Some("Origin"));
}

#[tokio::test]
async fn non_preflight_cors_preserves_existing_vary() {
    for existing in ["Accept-Encoding", "Accept-Encoding, Origin"] {
        let server = spawn_vary_server(existing).await;
        let host = server.addr.to_string();
        let origin = "http://localhost:3000";
        let (status, head) = request_head(
            server.addr,
            "GET",
            "/vary",
            &host,
            &[("Origin", origin)],
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(header(&head, "access-control-allow-origin"), Some(origin));
        assert_eq!(
            header(&head, "access-control-allow-credentials"),
            Some("true")
        );
        assert_eq!(header(&head, "access-control-allow-methods"), Some(METHODS));
        assert_eq!(header(&head, "access-control-allow-headers"), Some(HEADERS));
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
        ("example.com", "https://attacker@example.com"),
        ("example.com", "https://example.com/path"),
        ("example.com", "https://example.com?query"),
        ("example.com", "https://example.com#fragment"),
        ("example.com:", "https://example.com"),
        ("example.com:65536", "https://example.com"),
        ("example.com", "https://example.com:"),
        ("example.com", "https://example.com:65536"),
        ("example.com:444", "https://example.com:444"),
        ("example.com", "https://example.com:444"),
        ("example.com:443", "https://example.com:444"),
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
            403
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
    for path in ["/", "/mcp"] {
        assert_eq!(
            request_head(server.addr, "GET", path, "evil.example", &[])
                .await
                .0,
            403,
            "{path}"
        );
    }

    let host = server.addr.to_string();
    let (status, head) = request_head(server.addr, "GET", "/", &host, &[]).await;
    assert_eq!(status, 200);
    assert_eq!(header(&head, "vary"), Some("Origin"));
    assert_eq!(
        request_head(server.addr, "GET", "/mcp", &host, &[]).await.0,
        401
    );
    let (status, head) = request_head(
        server.addr,
        "GET",
        "/",
        &host,
        &[("Origin", "https://evil.example")],
    )
    .await;
    assert_eq!(status, 403);
    assert!(header(&head, "access-control-allow-origin").is_none());
    assert_eq!(header(&head, "vary"), Some("Origin"));
    assert_eq!(header(&head, "cache-control"), Some("no-store"));
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
        assert_eq!(header(&head, "cache-control"), Some("no-store"));
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
```

- [ ] **Step 3: Compile RED-0**

```bash
bun cargo test -p koharu-rpc --test origin_host --no-run
```

Expected: exit `0`; the test target compiles.

- [ ] **Step 4: Run RED-1**

```bash
set -euo pipefail
set +e
RED_OUTPUT="$(bun cargo test -p koharu-rpc --test origin_host -- --nocapture 2>&1)"
RED_EXIT=$?
set -e
printf '%s\n' "$RED_OUTPUT"
test "$RED_EXIT" = 101
test "$(printf '%s\n' "$RED_OUTPUT" | grep -Ec '^running 6 tests$')" = 1
printf '%s\n' "$RED_OUTPUT" | grep -Eq 'test result: FAILED\. 0 passed; 6 failed;'
```

Expected on the post-R03A baseline: exit `101`; exactly six tests run and all six fail for these existing defects:

- userinfo, empty/bracket-invalid hosts, bracketed non-IP literals, and bracketed IPv6 authorities with illegal suffixes are accepted; the middleware ignores URI authority and does not reject a conflict with `Host`;
- no-Origin forged or missing Host reaches auth instead of returning `403`, while accepted no-Origin responses lack `Vary: Origin`;
- forged Host can reach static `/` and returns `200` instead of `403`;
- accepted preflight emits `Access-Control-Allow-Origin: null` rather than the exact origin and lacks `Cache-Control: no-store`.
- accepted non-preflight CORS does not emit the credentialed headers, and the new implementation must preserve the handler's existing `Vary` value.
- the Desktop production session+assets listener leaves its static fallback and MCP mount outside Host/Origin enforcement.

Any compilation error, zero-test filter, timeout, or unrelated failure is a hard stop.

- [ ] **Step 5: Replace the Origin/Host policy section in `security.rs`**

Keep the existing authentication and browser-session sections. Replace only the Origin/Host imports, constants, structs, and helper functions with:

```rust
use std::net::SocketAddr;

use axum::http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS,
    ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_HEADERS,
    ACCESS_CONTROL_REQUEST_METHOD, CACHE_CONTROL, HOST, ORIGIN, VARY,
};
use axum::http::uri::Authority;
use axum::http::{HeaderValue, Method};

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
            authorities.push(
                parse_authority(entry.trim()).ok_or_else(|| anyhow::anyhow!("invalid host"))?,
            );
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
    let Some(policy) = request.extensions().get::<OriginHostPolicy>() else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(host) = request_authority(&request) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !host_allowed(policy, &host) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let origin = match single_header(request.headers(), ORIGIN) {
        Ok(None) => {
            let mut response = next.run(request).await;
            ensure_vary_origin(response.headers_mut());
            return response;
        }
        Ok(Some(raw)) => match parse_origin(raw) {
            Some(origin) if origin_allowed(policy, &host, &origin) => origin,
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
    if raw.is_empty() || raw.contains('*') || raw.contains('@') {
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
    Some(raw.split_once(':').map(|(_, port)| port))
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

fn single_header(
    headers: &HeaderMap,
    name: axum::http::header::HeaderName,
) -> Result<Option<&str>, ()> {
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
    let default_port = match request.uri().scheme_str() {
        Some("https") => 443,
        Some("http") | None => 80,
        Some(_) => return None,
    };
    let header = match single_header(request.headers(), HOST) {
        Ok(Some(raw)) => Some(parse_authority(raw)?),
        Ok(None) => None,
        Err(()) => return None,
    };
    let uri = match request.uri().authority() {
        Some(raw) => Some(parse_authority(raw.as_str())?),
        None => None,
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

fn host_ip(host: &str) -> Option<std::net::IpAddr> {
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
    let Ok(Some(method)) = single_header(headers, ACCESS_CONTROL_REQUEST_METHOD) else {
        return false;
    };
    if !matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return false;
    }
    let Ok(requested) = single_header(headers, ACCESS_CONTROL_REQUEST_HEADERS) else {
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
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin.parse().unwrap());
    ensure_vary_origin(headers);
    headers.insert(
        ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(ALLOWED_METHODS),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(ALLOWED_HEADERS),
    );
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
    let already_varies = headers.get_all(VARY).iter().filter_map(|value| value.to_str().ok()).any(
        |value| {
            value
                .split(',')
                .map(str::trim)
                .any(|token| token == "*" || token.eq_ignore_ascii_case("Origin"))
        },
    );
    if !already_varies {
        headers.append(VARY, HeaderValue::from_static("Origin"));
    }
}
```

After the production replacement, add this same-file unit regression at the end of `security.rs`. The external absolute-form conflict assertion already provides the failing RED for the missing unified authority resolver; this unit test pins Hyper/Axum's HTTP/2 request representation without adding a seventh `origin_host` integration test:

```rust
#[cfg(test)]
mod authority_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Version;

    fn http2_request(uri: &str, host: Option<&str>) -> Request {
        let mut request = Request::builder()
            .version(Version::HTTP_2)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        if let Some(host) = host {
            request.headers_mut().insert(HOST, host.parse().unwrap());
        }
        request
    }

    #[test]
    fn http2_authority_reconciles_uri_and_host_sources() {
        let uri_only = http2_request("https://example.com/api/v1/meta", None);
        assert_eq!(request_authority(&uri_only).unwrap().host(), "example.com");

        let equivalent = http2_request(
            "https://example.com/api/v1/meta",
            Some("EXAMPLE.COM:443"),
        );
        assert!(request_authority(&equivalent).is_some());

        let conflicting = http2_request(
            "https://example.com/api/v1/meta",
            Some("other.example"),
        );
        assert!(request_authority(&conflicting).is_none());

        let non_http = http2_request("ftp://example.com/api/v1/meta", None);
        assert!(request_authority(&non_http).is_none());
    }
}
```

- [ ] **Step 6: Assemble the complete router before installing policy in `server.rs`**

Add these private helpers after `AssetResolver`:

```rust
fn with_origin_host_policy(router: Router, policy: OriginHostPolicy) -> Router {
    router
        .layer(middleware::from_fn(crate::security::enforce_origin_host))
        .layer(axum::Extension(policy))
}

fn complete_router(
    app: AppState,
    security: SecurityContext,
    session: Option<crate::security::BrowserSessionState>,
) -> Router {
    let api = match session {
        Some(session) => api::router_with_session(app.clone(), security.clone(), session),
        None => api::router(app.clone(), security.clone()),
    };
    crate::mcp::mount(api, app, security)
}
```

Replace the three router constructors with:

```rust
pub fn router_for(app: AppState, security: SecurityContext, policy: OriginHostPolicy) -> Router {
    with_origin_host_policy(complete_router(app, security, None), policy)
}

pub fn router_for_with_session(
    app: AppState,
    security: SecurityContext,
    policy: OriginHostPolicy,
    session: crate::security::BrowserSessionState,
) -> Router {
    with_origin_host_policy(complete_router(app, security, Some(session)), policy)
}

pub fn router_with_assets(
    app: AppState,
    security: SecurityContext,
    policy: OriginHostPolicy,
    resolver: AssetResolver,
) -> Router {
    let complete = complete_router(app, security, None).fallback(move |req: Request<Body>| {
        let resolver = resolver.clone();
        async move { serve_asset(resolver, req).await }
    });
    with_origin_host_policy(complete, policy)
}
```

Keep the existing listener function signatures. Replace only the body of `serve_with_listener_and_assets_with_session` with:

```rust
{
    axum::serve(
        listener,
        with_origin_host_policy(
            complete_router(app, security, Some(session)).fallback(move |req: Request<Body>| {
                let resolver = resolver.clone();
                async move { serve_asset(resolver, req).await }
            }),
            policy,
        ),
    )
    .await?;
    Ok(())
}
```

Do not add a fifth public router variant. The two private helpers remove duplication without widening the public API.

- [ ] **Step 7: Run GREEN and all R03 regressions**

```bash
set -euo pipefail
rustfmt --edition 2024 \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/src/server.rs \
  crates/koharu-rpc/tests/origin_host.rs
assert_green_tests() {
  expected="$1"
  shift
  output="$("$@" 2>&1)"
  printf '%s\n' "$output"
  test "$(printf '%s\n' "$output" | grep -Ec "^running ${expected} tests?$")" = 1
  printf '%s\n' "$output" | grep -Eq "test result: ok\\. ${expected} passed;"
}
assert_green_tests 6 bun cargo test -p koharu-rpc --test origin_host -- --nocapture
assert_green_tests 1 bun cargo test -p koharu-rpc --lib http2_authority_reconciles_uri_and_host_sources -- --nocapture
assert_green_tests 1 bun cargo test -p koharu-rpc --lib bootstrap_routes_require_authentication -- --nocapture
assert_green_tests 3 bun cargo test -p koharu-rpc --lib session_exchange -- --nocapture
assert_green_tests 1 bun cargo test -p koharu-rpc --lib mcp_remains_master_bearer_only -- --nocapture
bun cargo check -p koharu-rpc --all-targets
bun cargo clippy -p koharu-rpc --all-targets -- -D warnings
bun cargo fmt --all -- --check
git diff --check -- \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/src/server.rs \
  crates/koharu-rpc/tests/origin_host.rs
```

Expected: origin/host `6/6`, HTTP/2 authority representation `1/1`, bootstrap `1/1`, session exchange `3/3`, MCP `1/1`; all static checks exit `0`.

- [ ] **Step 8: Independent R03B review**

Run this full-scope guard immediately before dispatching review:

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
set -a; source .env; set +a
test "$(git branch --show-current)" = codex/audit-remediation-sdd
test "${KOHARU_SHARED_TARGET_DIR:?}" = /Volumes/G/EC-image-koharu/target
test "${CARGO_TARGET_DIR:?}" = "$KOHARU_SHARED_TARGET_DIR"
git diff --cached --quiet
LEDGER=docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
CONTRACT=docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md
PLAN=docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md
test "$(grep -c '^R03_START_SHA=[0-9a-f]\{40\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_CONTRACT_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_PLAN_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03A_RED_PATCH_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(shasum -a 256 "$CONTRACT" | awk '{print $1}')" = "$(sed -n 's/^R03_CONTRACT_SHA256=//p' "$LEDGER")"
test "$(shasum -a 256 "$PLAN" | awk '{print $1}')" = "$(sed -n 's/^R03_PLAN_SHA256=//p' "$LEDGER")"
test "$(sed -n 's/^R03A_RED_PATCH_SHA256=//p' "$LEDGER")" = c104c665cffffd917560aad5d4b75e77c2e38ead399a81c47d341c5150cd9883
R03_START_SHA="$(sed -n 's/^R03_START_SHA=//p' "$LEDGER")"
test "$(git rev-parse HEAD^)" = "$R03_START_SHA"
test "$(git log -1 --format=%s)" = "fix(rpc): authenticate bootstrap business routes"
test "$(git diff-tree --no-commit-id --name-only -r HEAD)" = crates/koharu-rpc/src/api.rs
test -z "$(git status --porcelain=v1 --untracked-files=all | cut -c4- | grep -Ev \
  '^(crates/koharu-rpc/src/security\.rs|crates/koharu-rpc/src/server\.rs|crates/koharu-rpc/tests/origin_host\.rs|docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger\.md|docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract\.md|docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan\.md)$' || true)"
```

Dispatch a fresh read-only `code-reviewer` with the exact three-file diff and Step 7 command outputs. Review requirements:

- Request authority is resolved from one valid `Host` header or `request.uri().authority()`; absence, duplicate Host, malformed input, or disagreement between both sources fails closed before Origin/auth/readiness/handler. HTTP/1 absolute-form and the URI representation used for HTTP/2 `:authority` share this path.
- The same-file `HTTP_2` Request regression directly proves authority-only, equivalent dual-source, and conflicting dual-source behavior.
- missing, duplicate, or malformed authority/Origin fails closed.
- userinfo, wildcard, non-HTTP(S), path/query/fragment, empty host, bracketed non-IP literal, bracketed IPv6 suffix junk, and invalid authority are rejected structurally before `Authority::host()` can normalize them.
- empty/out-of-range explicit ports are rejected rather than treated as defaults.
- DNS case, HTTPS effective port `443`, wrong explicit ports, equivalent IPv6 text forms, and bracketed IPv6 loopback are handled correctly.
- preflight and accepted non-preflight responses expose only the exact allowed method/header sets and exact origin with credentials; accepted preflight is `Cache-Control: no-store`.
- every Host-valid response varies on `Origin`; adding CORS preserves existing `Vary` values and adds `Origin` without duplication, including no-Origin and denied-Origin paths.
- API, MCP, and static fallback share the same outer policy.
- valid static assets remain public; MCP remains master-Bearer-only.
- the production `serve_with_listener_and_assets_with_session` path enforces policy on API, MCP, and assets while preserving session exchange.

Unresolved HIGH/CRITICAL is a hard stop. Repair only the three frozen files, rerun Step 7, and request scoped re-review.

- [ ] **Step 9: Stage and commit only R03B**

Rerun the Step 8 full-scope guard verbatim immediately before this block and do not edit any file between that successful guard and staging.

```bash
git add \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/src/server.rs \
  crates/koharu-rpc/tests/origin_host.rs
test "$(git diff --cached --name-only)" = "$(printf '%s\n' \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/src/server.rs \
  crates/koharu-rpc/tests/origin_host.rs)"
git diff --cached --check
git commit -m "fix(rpc): enforce complete host origin policy" \
  -m "Co-Authored-By: Codex <noreply@openai.com>"
git rev-parse HEAD
```

Append RED/GREEN/review/commit SHA to the ledger without staging it.

---

### Task 3: R03 closeout gate

**Files:**
- Read only: all R03 committed files
- Evidence only: `docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md`

**Interfaces:**
- Consumes: committed R03A and R03B.
- Produces: evidence that permits drafting R04; no merge-readiness claim.

- [ ] **Step 1: Run the full R03 crate gate**

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
test -f .env
set -a
source .env
set +a
test "$(git branch --show-current)" = codex/audit-remediation-sdd
test "${KOHARU_SHARED_TARGET_DIR:?}" = /Volumes/G/EC-image-koharu/target
test "${CARGO_TARGET_DIR:?}" = "$KOHARU_SHARED_TARGET_DIR"
git diff --cached --quiet
LEDGER=docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
CONTRACT=docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md
PLAN=docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md
test "$(grep -c '^R03_START_SHA=[0-9a-f]\{40\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_CONTRACT_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_PLAN_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03A_RED_PATCH_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(shasum -a 256 "$CONTRACT" | awk '{print $1}')" = \
  "$(sed -n 's/^R03_CONTRACT_SHA256=//p' "$LEDGER")"
test "$(shasum -a 256 "$PLAN" | awk '{print $1}')" = \
  "$(sed -n 's/^R03_PLAN_SHA256=//p' "$LEDGER")"
test "$(sed -n 's/^R03A_RED_PATCH_SHA256=//p' "$LEDGER")" = c104c665cffffd917560aad5d4b75e77c2e38ead399a81c47d341c5150cd9883
R03_START_SHA="$(sed -n 's/^R03_START_SHA=//p' "$LEDGER")"
git merge-base --is-ancestor "$R03_START_SHA" HEAD
bun cargo fmt --all -- --check
bun cargo check -p koharu-rpc --all-targets
bun cargo clippy -p koharu-rpc --all-targets -- -D warnings
bun cargo test -p koharu-rpc
git diff --check
```

Expected: every command exits `0`. Record exact test counts and command exits in the ledger.

- [ ] **Step 2: Verify commit and worktree boundaries**

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
test -f .env
set -a
source .env
set +a
test "$(git branch --show-current)" = codex/audit-remediation-sdd
test "${KOHARU_SHARED_TARGET_DIR:?}" = /Volumes/G/EC-image-koharu/target
test "${CARGO_TARGET_DIR:?}" = "$KOHARU_SHARED_TARGET_DIR"
LEDGER=docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
CONTRACT=docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md
PLAN=docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md
test "$(grep -c '^R03_START_SHA=[0-9a-f]\{40\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_CONTRACT_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_PLAN_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03A_RED_PATCH_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(shasum -a 256 "$CONTRACT" | awk '{print $1}')" = \
  "$(sed -n 's/^R03_CONTRACT_SHA256=//p' "$LEDGER")"
test "$(shasum -a 256 "$PLAN" | awk '{print $1}')" = \
  "$(sed -n 's/^R03_PLAN_SHA256=//p' "$LEDGER")"
test "$(sed -n 's/^R03A_RED_PATCH_SHA256=//p' "$LEDGER")" = c104c665cffffd917560aad5d4b75e77c2e38ead399a81c47d341c5150cd9883
R03_START_SHA="$(sed -n 's/^R03_START_SHA=//p' \
  "$LEDGER")"
git merge-base --is-ancestor "$R03_START_SHA" HEAD
git log --oneline -4
git status --short --untracked-files=all
git diff --cached --quiet
test -z "$(git status --porcelain=v1 --untracked-files=all | cut -c4- | grep -Ev \
  '^(docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger\.md|docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract\.md|docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan\.md)$' || true)"
EXPECTED_R03_PATHS="$(printf '%s\n' \
  crates/koharu-rpc/src/api.rs \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/src/server.rs \
  crates/koharu-rpc/tests/origin_host.rs | sort)"
test "$(git diff --name-only "$R03_START_SHA"..HEAD | sort)" = "$EXPECTED_R03_PATHS"
test "$(git log --format= --name-only "$R03_START_SHA"..HEAD | sed '/^$/d' | sort -u)" = "$EXPECTED_R03_PATHS"
test "$(rg -l 'server::serve_with_listener' crates/koharu/src --glob '*.rs')" = crates/koharu/src/app.rs
test -z "$(rg -n 'koharu_rpc::(api|router)|api::router|mcp::mount|axum::serve' \
  crates/koharu/src --glob '*.rs' || true)"
```

Expected R03 committed paths only:

```text
crates/koharu-rpc/src/api.rs
crates/koharu-rpc/src/security.rs
crates/koharu-rpc/src/server.rs
crates/koharu-rpc/tests/origin_host.rs
```

The index is empty. The ledger and approved plan documents may remain uncommitted; no other path may be dirty. Both the final tree diff and the union of every committed path in `R03_START_SHA..HEAD` contain exactly the four R03 paths above, regardless of whether scoped follow-up commits were required. The production crate currently has exactly one `server::serve_with_listener*` call-site file and no direct raw API/MCP/Axum serving path. This grep is a closeout snapshot, not a durable prevention guarantee for future embedding; the deferred raw-surface card owns structural API narrowing.

- [ ] **Step 3: Independent whole-R03 review**

Retrieve the single frozen `R03_START_SHA` from the ledger exactly as in Step 2. Dispatch a new read-only `code-reviewer` over `R03_START_SHA..HEAD`, this plan's acceptance criteria, and the full crate-gate output. HIGH/CRITICAL findings require repair in a new narrowly named R03 follow-up commit; do not amend or squash reviewed commits.

Each follow-up is a controlled Task 3 closeout exception to the per-card single-commit rule. It may modify and stage only the four frozen R03 paths. Before every follow-up commit run:

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
set -a; source .env; set +a
test "$(git branch --show-current)" = codex/audit-remediation-sdd
test "${KOHARU_SHARED_TARGET_DIR:?}" = /Volumes/G/EC-image-koharu/target
test "${CARGO_TARGET_DIR:?}" = "$KOHARU_SHARED_TARGET_DIR"
git diff --cached --quiet
LEDGER=docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
CONTRACT=docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md
PLAN=docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md
test "$(grep -c '^R03_START_SHA=[0-9a-f]\{40\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_CONTRACT_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03_PLAN_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(grep -c '^R03A_RED_PATCH_SHA256=[0-9a-f]\{64\}$' "$LEDGER")" = 1
test "$(shasum -a 256 "$CONTRACT" | awk '{print $1}')" = \
  "$(sed -n 's/^R03_CONTRACT_SHA256=//p' "$LEDGER")"
test "$(shasum -a 256 "$PLAN" | awk '{print $1}')" = \
  "$(sed -n 's/^R03_PLAN_SHA256=//p' "$LEDGER")"
test "$(sed -n 's/^R03A_RED_PATCH_SHA256=//p' "$LEDGER")" = c104c665cffffd917560aad5d4b75e77c2e38ead399a81c47d341c5150cd9883
R03_START_SHA="$(sed -n 's/^R03_START_SHA=//p' "$LEDGER")"
git merge-base --is-ancestor "$R03_START_SHA" HEAD
test -z "$(git status --porcelain=v1 --untracked-files=all | cut -c4- | grep -Ev \
  '^(crates/koharu-rpc/src/api\.rs|crates/koharu-rpc/src/security\.rs|crates/koharu-rpc/src/server\.rs|crates/koharu-rpc/tests/origin_host\.rs|docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger\.md|docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract\.md|docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan\.md)$' || true)"
assert_green_tests() {
  expected="$1"
  shift
  output="$("$@" 2>&1)"
  printf '%s\n' "$output"
  test "$(printf '%s\n' "$output" | grep -Ec "^running ${expected} tests?$")" = 1
  printf '%s\n' "$output" | grep -Eq "test result: ok\\. ${expected} passed;"
}
assert_green_tests 6 bun cargo test -p koharu-rpc --test origin_host -- --nocapture
assert_green_tests 1 bun cargo test -p koharu-rpc --lib http2_authority_reconciles_uri_and_host_sources -- --nocapture
assert_green_tests 1 bun cargo test -p koharu-rpc --lib bootstrap_routes_require_authentication -- --nocapture
bun cargo fmt --all -- --check
bun cargo check -p koharu-rpc --all-targets
bun cargo clippy -p koharu-rpc --all-targets -- -D warnings
bun cargo test -p koharu-rpc
git diff --check
git add \
  crates/koharu-rpc/src/api.rs \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/src/server.rs \
  crates/koharu-rpc/tests/origin_host.rs
test -n "$(git diff --cached --name-only)"
test -z "$(git diff --cached --name-only | grep -Ev \
  '^(crates/koharu-rpc/src/api\.rs|crates/koharu-rpc/src/security\.rs|crates/koharu-rpc/src/server\.rs|crates/koharu-rpc/tests/origin_host\.rs)$' || true)"
git diff --cached --check
git commit -m "fix(rpc): close r03 review finding" \
  -m "Co-Authored-By: Codex <noreply@openai.com>"
```

After any follow-up commit, restart Task 3 at Step 1, rerun Steps 1–2 in full, and request a new whole-R03 Step 3 review over the same frozen range. Do not proceed to Step 4 on evidence captured before the latest follow-up commit.

- [ ] **Step 4: Stop at the R03 boundary**

After review approval, append the closeout verdict to the unstaged ledger. Record a deferred, separately reviewed hardening card to narrow the raw router/OpenAPI surface before supporting third-party embedding; this note does not expand or authorize R03 work. Report `R03 COMPLETE — R04 PLAN MAY BE DRAFTED`. Do not implement Desktop/Tauri, UI, Headless, integration harness, raw-router API changes, final smoke, or evidence commit under this plan.

---

## Plan-validation evidence

The preceding revision's test and implementation snippets were exercised against a temporary `git archive` copy of starting HEAD using an explicit one-time export of the approved shared target because ignored `.env` is not present in an archive. The current revision adds explicit non-HTTP(S), digest-binding, test-count, cache, and closeout guards and therefore requires a fresh independent review under its new SHA; the prior sample results below are historical evidence, not approval of this revision. No product code was changed in the execution worktree.

- R03A RED: `1` test ran; exit `101`; `/api/v1/events` returned `200` instead of `401`.
- Frozen R03A RED test-only patch digest: `c104c665cffffd917560aad5d4b75e77c2e38ead399a81c47d341c5150cd9883`.
- R03A GREEN sample: `1/1` passed.
- R03B RED: `6` tests ran; exit `101`; all `6` failed against the archived old implementation, including Host/URI-authority conflict, invalid bracketed host, bracketed-IPv6 suffix, and cache-variant defects.
- R03B GREEN sample: `6/6` passed after applying the exact plan snippets, including URI-authority-only and equivalent/conflicting dual-source cases.
- HTTP/2 Request authority unit regression: `1/1` passed.
- Adjacent sample regressions: session exchange `3/3`, MCP `1/1`.
- Sample static gates: `cargo check -p koharu-rpc --all-targets`, Clippy with `-D warnings`, and workspace fmt check exited `0`.

The executor must reproduce RED and GREEN in the real worktree and record fresh evidence; none of these prior results may substitute for the current SHA review.
