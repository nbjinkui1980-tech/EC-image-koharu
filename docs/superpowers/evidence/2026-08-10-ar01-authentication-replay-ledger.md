# AR01 Authentication Replay Evidence Ledger

Baseline: db2754822e9ec9ef5ad63200e5addebbff21688f
Branch: codex/audit-remediation-sdd
Plan: docs/superpowers/plans/2026-08-10-ar01-authentication-replay-plan.md

## Commits

| # | SHA | Task | Description |
|---|-----|------|-------------|
| 1 | ab462ca6 | T01A | feat(rpc): add protected API router core |
| 2 | 638e3abf | T01B | feat(rpc): require credentials on API servers |
| 3 | a4ce5f2a | T03 | fix(rpc): enforce host and origin policy |
| 4 | 630e2ade | T02 | fix(mcp): require master bearer authentication |
| 5 | 9c9f331e | T04A | chore(deps): add constant-time comparison primitive |
| 6 | c100e062 | T04-RPC | feat(rpc): exchange credentials for browser sessions |
| 7 | e7469577 | T04-Desktop | feat(desktop): bootstrap authenticated browser sessions |
| 8 | 57b5a540 | T04B | feat(ui): bootstrap authenticated sessions |
| 9 | b1f38810 | T04C | fix(ui): unify authenticated API and event transport |
| 10 | 3ed6df12 | T05 | fix(app): fail closed for headless authentication |

## Verification

- cargo check -p koharu -p koharu-rpc --all-targets: PASS
- cargo fmt --all -- --check: PASS
- cargo test -p koharu-rpc --lib: 26/26 PASS
- cargo test -p koharu: 0/0 PASS (no tests in this crate)

## Scope

- 10 product commits covering Waves 3-4 of the audit remediation plan
- Master Bearer authentication on all API routes
- Host/Origin/CORS policy enforcement
- MCP Bearer-only authentication
- Desktop proof-and-session exchange
- Headless fail-closed startup with KOHARU_AUTH_SECRET and --auth-secret-file
- UI auth bootstrap with session cookie exchange
- Same-origin credential transport for fetch and SSE

## Files Changed

```
crates/koharu-rpc/src/security.rs       (+ session state, host policy)
crates/koharu-rpc/src/api.rs            (+ auth middleware, session route)
crates/koharu-rpc/src/server.rs         (+ SecurityContext, OriginHostPolicy params)
crates/koharu-rpc/src/mcp/mod.rs        (+ MCP Bearer auth)
crates/koharu-rpc/src/lib.rs            (+ pub mod security)
crates/koharu-rpc/Cargo.toml            (+ subtle, getrandom deps)
crates/koharu/src/security.rs           (+ DesktopAuth, HeadlessSecurityOptions)
crates/koharu/src/app.rs                (+ auth bootstrap, headless auth)
crates/koharu/src/cli.rs                (+ --auth-secret-file, --allowed-host)
crates/koharu/src/lib.rs                (+ pub mod security)
crates/koharu/Cargo.toml                (+ base64 dep)
ui/lib/auth.ts                          (+ session exchange, auth events)
ui/components/AuthBootstrap.tsx          (+ auth gate component)
ui/app/providers.tsx                    (+ AuthBootstrap wrapper)
ui/lib/api/fetch.ts                     (+ credentials: same-origin, 401 notify)
ui/lib/events.ts                        (+ credentials: same-origin, 401 notify)
tests/integration-tests/src/harness.rs  (+ SecurityContext param)
crates/koharu-rpc/src/routes/history.rs (+ auth test updates)
Cargo.toml                              (+ subtle workspace dep)
Cargo.lock                              (dependency updates)
```
