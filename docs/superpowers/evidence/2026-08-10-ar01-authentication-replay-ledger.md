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

## Remediation cards

### R01 — authenticated browser-session wiring

- RED: `CARGO_TARGET_DIR=/tmp/koharu-sdd-ar01-r01 bun cargo test -p koharu-rpc session_exchange_rejects_missing_credential -- --nocapture` exited 101; unauthenticated exchange returned 204 instead of 401.
- GREEN: `CARGO_TARGET_DIR=/tmp/koharu-sdd-ar01-r01 bun cargo test -p koharu-rpc session_exchange` passed 3/3; covers missing credential rejection, proof single use, master exchange, and session-aware listener mounting.
- GREEN: `CARGO_TARGET_DIR=/tmp/koharu-sdd-ar01-r01 bun cargo test -p koharu-rpc` passed 29 library tests, 1 binary test, and 4 integration tests.
- GREEN: `CARGO_TARGET_DIR=/tmp/koharu-sdd-ar01-r01 bun cargo check -p koharu -p koharu-rpc --all-targets`, file-scoped rustfmt, and `git diff --check` passed.
- Review: scoped independent review APPROVE; R04 Tauri command registration remains a separate remediation card.
- Commit: `31e4d8c71e51b846e4bb87d2655da8dc2e0930f5` (`fix(rpc): wire authenticated browser sessions`).

### R02 — MCP Bearer scope

- RED: `CARGO_TARGET_DIR=/tmp/koharu-sdd-ar01-r02 bun cargo test -p koharu-rpc session_cookie_reaches_api_readiness_without_master_bearer -- --nocapture` exited 101; valid cookie request to `/api/v1/meta` returned 401 rather than readiness 503.
- GREEN: the same command passed after restricting `mcp_auth` to the `/mcp` subrouter.
- GREEN: `CARGO_TARGET_DIR=/tmp/koharu-sdd-ar01-r02 bun cargo test -p koharu-rpc mcp_remains_master_bearer_only -- --nocapture` passed; no credential and cookie returned 401 while master Bearer passed the gate.
- GREEN: `CARGO_TARGET_DIR=/tmp/koharu-sdd-ar01-r02 bun cargo test -p koharu-rpc` passed 31 library tests, 1 binary test, and 4 integration tests; `git diff --check` passed.
- Review: scoped independent review APPROVE; Host/Origin policy remains R03.
- Commit: `9e46ab84d5afceb2417a55b8cadda000ffd5580f` (`fix(mcp): scope bearer authentication to mcp routes`).

### R03 — Bootstrap authentication and Host/Origin/CORS

- R03A RED/GREEN and independent review evidence is frozen in `2026-08-11-ar01-r03a-bootstrap-auth-execution-card.md`.
- R03A commit: `057b01e317696c470ca2773c900f3ebb96381fde` (`fix(rpc): authenticate bootstrap business routes`), exactly `crates/koharu-rpc/src/api.rs`.
- R03B RED: the real-listener `origin_host` suite ran 6 tests and failed 0/6 against the old Host/Origin/CORS boundary.
- R03B GREEN: the same suite passed 6/6; the complete `koharu-rpc` regression, check, Clippy, fmt, and diff gates passed before commit.
- R03B commit: `583126bf260d573bfc87176e0ea11d5030013e2a` (`fix(rpc): enforce host and origin policy`), exactly `security.rs`, `server.rs`, and `tests/origin_host.rs`.

### R04 — Desktop Tauri proof command/state wiring

- R04 execution contract SHA-256: ae6630679ea459ed7d4b87919bd957b273561f5dfbbf5ee53a5f2bf60820de59
- R04 approved card SHA-256: 35c94f362e2f2f9649b4fe9e14bbec8d3a4cb81cad684701c6630f0359c05a53
- RED: `bun cargo test -p koharu desktop_auth_command_uses_managed_one_time_proof -- --nocapture` ran 1 test and exited 101 because `desktop_bootstrap_proof` was not registered.
- GREEN: the same command passed 1/1; `bun cargo test -p koharu` passed 12/12 and `bun cargo test -p koharu-rpc session_exchange -- --nocapture` passed 3/3.
- Gates: `bun cargo check -p koharu -p koharu-rpc --all-targets`, Clippy `-D warnings`, workspace fmt check, scoped diff checks, and the post-deslop rerun all passed.
- Desktop build: `KOHARU_CARGO_GUARD_ACTIVE=1 bun run build` passed after installing the frozen Bun dependencies; release binary: `/Volumes/G/EC-image-koharu/target/release/koharu`.
- Review: independent code-reviewer APPROVE with zero findings; independent architect CLEAR; standard deslop found no removable slop and made no changes.
- Commit: `6958c1e8a24e7826a2b5916b14d719db4a03261f` (`fix(desktop): register bootstrap proof command`), exactly the three approved product files.
- ACL follow-up RED: the production-context IPC test ran 1 test and failed 0/1 with `desktop_bootstrap_proof not allowed` for `http://127.0.0.1:4000`.
- ACL follow-up GREEN: the same remote-origin test passed 1/1 after adding the single-command app permission and attaching it to the existing `main`/loopback capability.
- ACL follow-up gates: `koharu` 12/12, RPC session exchange 3/3, check, Clippy `-D warnings`, fmt, scoped diff checks, release ACL inspection, and Desktop build passed; no Cargo.lock, Tauri schema, or autogenerated drift.
- ACL follow-up review: independent code-reviewer APPROVE with zero findings; independent architect CLEAR.
- ACL follow-up commit: `1dd6b20f178671fd8910817e2f74f6527f33324b` (`fix(desktop): allow bootstrap proof from release origin`), exactly three product files.

## Scope

- 10 original product commits covering Waves 3-4 of the audit remediation plan
- 6 replay/remediation commits covering R01 through R04, including the R04 ACL follow-up
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
crates/koharu/capabilities/default.json (+ Desktop proof command permission)
crates/koharu/permissions/desktop_bootstrap_proof.toml (+ app command ACL)
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
