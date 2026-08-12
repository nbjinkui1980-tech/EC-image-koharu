# AR01 Authentication Replay Evidence Ledger

Baseline: db2754822e9ec9ef5ad63200e5addebbff21688f
Branch: codex/audit-remediation-sdd
Plan: docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md

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
- R03B commit: `583126bf260d573bfc87176e0ea11d5030013e2a` (`fix(rpc): enforce host and origin policy`), implemented in the shared RPC security core, `server.rs`, and `tests/origin_host.rs`.

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

### R05 — UI authentication gate for children / SSE / Updater

- RED: the old AuthBootstrap swallowed `bootstrapDesktopSession` errors and rendered children while unauthenticated; `UpdaterProvider` wrapped `AuthBootstrap` so the updater started before the gate; `connectEvents` ran unconditionally in Providers before any auth check.
- GREEN: `bootstrapDesktopSession` now properly rejects on failure (dedup + one-time proof guard); AuthBootstrap adds `restart-required` state and connects SSE only after authenticated; Providers swaps AuthBootstrap outside UpdaterProvider.
- Tests: 3 new test files with 13 tests — `auth.test.ts` (5: exchange + bootstrap rejection + one-time guard), `AuthBootstrap.test.tsx` (7: desktop reject/success/re-auth, headless retry, already-authenticated), `providers.test.tsx` (1: updater not mounted before auth).
- Regression: `bun run test:ui` 34 files / 231 tests PASS; `lint:ui` exit 0; `format:check` PASS.
- Commit: `47b14322f407e01517ba7ca9839963d2112fd7c9` (`fix(ui): gate children, SSE, and Updater behind authenticated auth state`), exactly 3 product + 3 test files.

### R06 — Headless validation before bind

- RED: 8 pre-bind tests written; stub `validate_pre_bind` returns `Ok(())` → 5 FAIL (violations not rejected), 3 PASS (happy path + defaults pass-through).
- GREEN: `validate_pre_bind` implemented with checks for Desktop loopback constraint, Desktop headless-only flag rejection, headless secret requirement, and headless remote exposure allowlist requirement. `HeadlessSecurityOptions::resolve()` moved before `TcpListener::bind()`.
- Tests: 8 unit tests in `app::pre_bind_tests`.
- Regression: `koharu` 20/20 PASS (incl. 8 pre_bind + 1 desktop_auth); `koharu-rpc` 43/43 PASS; check, Clippy, fmt PASS.
- Manual QA: 5 CLI scenarios verified — Desktop `--host 0.0.0.0`, `--auth-secret-file`, `--allowed-host` all exit 1 before bind; headless without secret exits 1 before bind; headless with invalid secret decodes before bind.
- Commit: `bf1a3adb9208555ef6276d7c671bbd842bd4da3c` (`fix(app): validate headless secrets and Desktop --host before bind`), exactly `crates/koharu/src/app.rs`.

### R07 — Constant-time comparison, integration harness authentication, and final evidence

- Constant-time: `subtle::ConstantTimeEq` used for all three sensitive comparisons in `crates/koharu-rpc-security/src/lib.rs` — Bearer token verification (`authorizes_bearer`), one-time proof consumption (`consume_proof`), and session cookie validation (`validate_session`). Dependency `9c9f331e` unchanged.
- Integration harness: added Bearer token as `default_headers` on the integration-test `reqwest::Client` so all API requests pass the R03A authentication middleware. All 46 integration tests (binary 6, events 11, LLM 3, meta 8, pipelines 6, projects 8, scene 4) plus 1 platform-skipped (keyring) now pass.
- Harness commit: `1ca5accc1992eea72cf3f5f4a0964178e474a7a` (`fix(test): authenticate integration test harness`), exactly `Cargo.lock`, `tests/integration-tests/Cargo.toml`, `tests/integration-tests/src/client.rs`, `tests/integration-tests/src/harness.rs`.
- Evidence commit: `a248f2552aefd4f349d27483582201eb488b07d1` (`docs(evidence): record AR01 R05-R07 remediation evidence`).

## Branch readiness (REVIEW-READY)

### Final automated gates (post-R07)

| Gate | Result |
|---|---|
| `cargo test -p koharu` | 20/20 PASS |
| `cargo test -p koharu-rpc` | 43/43 PASS (32 lib + 1 binary + 4 openapi + 6 origin_host) |
| `cargo test -p koharu-integration-tests` | 46/47 PASS (1 skipped: keyring) |
| UI `test:ui` | 35 files / 231 tests PASS |
| `cargo check --all-targets` | PASS (koharu, koharu-rpc, integration-tests) |
| `cargo clippy --all-targets -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| UI `lint:ui` | exit 0 (only pre-existing warnings) |
| UI `format:check` | PASS |

### Readiness claims

- **Desktop manual smoke**: PENDING — not executed in this remediation session. R04 Desktop build verified via `KOHARU_CARGO_GUARD_ACTIVE=1 bun run build` during R04/R04-ACL execution; the resulting release binary was confirmed at `/Volumes/G/EC-image-koharu/target/release/koharu` but interactive Desktop smoke (launch → auth bootstrap → app ready) was not performed.
- **Remote HTTPS reverse-proxy smoke**: PENDING — not executed. Headless mode pre-bind validation is exercised by unit tests and CLI manual QA, but a real reverse-proxy deployment with external TLS termination was not tested.
- **Docker**: out of scope per execution contract §6.
- **Release-ready**: NOT CLAIMED — Docker, Desktop smoke, and remote-proxy smoke are all PENDING or out of scope. This branch is review-ready, not deployment-ready.

### Integration status

The remediation branch is merged with `main`; the duplicate RPC security implementation was removed in favor of the shared `koharu-rpc-security` crate, while the structured Host/Origin/CORS boundary tests remain in place. It is **not merge-ready** until the pending Desktop manual smoke and an independent whole-branch review complete. Desktop navigation/ACL proof enforcement remains the separately tracked AR07-T02 prerequisite.

## Structured self-review (Oracle unavailable — manual verification)

Oracle was dispatched three times (whole-branch + three focused sub-reviews) but timed out at 30 minutes each. The following structured self-review covers all security-critical acceptance items from R01-R07. Each finding cites file:line and the specific automated test or manual QA that backs it.

### 1. Constant-time comparison (R07)

| # | Location | Comparison | Verdict |
|---|---|---|---|
| CT1 | `rpc-security/src/lib.rs:29` | `authorizes_bearer`: `token.ct_eq(&self.secret).into()` | ✅ `subtle::ConstantTimeEq` |
| CT2 | `rpc-security/src/lib.rs:355` | `consume_proof`: `stored.ct_eq(candidate).into()` | ✅ guarded by `Mutex<Option>` take |
| CT3 | `rpc-security/src/lib.rs:369` | `validate_session`: `token.ct_eq(&self.session_token).into()` | ✅ |

All three sensitive token comparisons use constant-time equality. Verified by grep — no `==` comparison on secret material found in the security module.

### 2. Pre-bind validation (R06)

| # | Scenario | Test | Verdict |
|---|---|---|---|
| PB1 | Desktop `--host 0.0.0.0` | `app.rs:317` + CLI QA | ✅ `validate_pre_bind` runs before `TcpListener::bind()` (line order verified: check at L113, bind at L131) |
| PB2 | Desktop `--auth-secret-file` | `app.rs:326` + CLI QA | ✅ |
| PB3 | Desktop `--allowed-host` | `app.rs:332` + CLI QA | ✅ |
| PB4 | Headless without secret | `app.rs:300` + CLI QA | ✅ |
| PB5 | Headless remote without allowed-hosts | `app.rs:311` + CLI QA | ✅ |
| PB6 | Headless secret decode | `security.rs:44-67` (resolve) | ✅ `decode_headless_secret` runs before bind |

No code path exists where `TcpListener::bind()` executes before all six checks pass. Verified by line-order inspection: `validate_pre_bind` at L113, `headless_security = ...resolve()` at L120, `TcpListener::bind` at L131.

### 3. Auth gate — fail-closed integrity (R05)

| # | Scenario | Test | Verdict |
|---|---|---|---|
| AG1 | Desktop bootstrap reject → restart-required | `AuthBootstrap.test.tsx` (shows restart-required...) | ✅ children NOT rendered, SSE NOT called |
| AG2 | Desktop bootstrap success → children + SSE | `AuthBootstrap.test.tsx` (mounts neither...) | ✅ `connectEvents` called exactly once |
| AG3 | Headless token reject → stays at form | `AuthBootstrap.test.tsx` (keeps headless...) | ✅ error message visible |
| AG4 | Headless retry after failure | `AuthBootstrap.test.tsx` (keeps headless...) | ✅ second attempt succeeds |
| AG5 | Already-authenticated → skip bootstrap | `AuthBootstrap.test.tsx` (keeps authenticated...) | ✅ no double-bootstrap |
| AG6 | SSE 401 → auth-required → restart-required | `AuthBootstrap.test.tsx` (shows restart-required...) | ✅ Desktop shows alert |
| AG7 | API 401 → auth-required → token form | `AuthBootstrap.test.tsx` (returns headless...) | ✅ headless returns to form |

### 4. Component tree ordering (R05)

| Check | File | Verdict |
|---|---|---|
| `AuthBootstrap` wraps `UpdaterProvider` | `providers.tsx:33-35` | ✅ Updater only mounted after auth |
| SSE only called in `useEffect` when `state === 'authenticated'` | `AuthBootstrap.tsx:57-59` | ✅ `connectEvents()` return used as cleanup |
| `connectEvents` removed from Providers | `providers.tsx` (diff: -`connectEvents`) | ✅ no unconditional SSE |
| `providers.test.tsx` verifies ordering | `tests/app/providers.test.tsx` | ✅ 1/1 PASS |

### 5. Token and secret safety

| Check | Verdict |
|---|---|
| Secret in logs? | ✅ `SecurityContext` stores `[u8; 32]`, no `Debug`/`Display` impl that leaks bytes |
| Secret in error messages? | ✅ CLI errors say "must be 43 characters" / "requires KOHARU_AUTH_SECRET", never print the secret |
| Secret in URL/browser storage? | ✅ `exchangeSession` uses `Authorization: Bearer` header, `credentials: 'same-origin'`, session stored as `HttpOnly; SameSite=Strict` cookie |
| One-time proof double-use? | ✅ `DesktopAuth.take_proof()` via `Mutex<Option>` — second call returns `None` |

### 6. Evidence completeness

All 10 R01-R07 acceptance items (from R03A frozen card + R03B frozen card + R05 frozen card + R06 frozen card + R07 frozen card) are covered by at least one test or manual QA artifact. No item lacks evidence.

### Review verdict

**APPROVE** with zero CRITICAL, zero HIGH, zero MEDIUM findings. Two items remain PENDING (Desktop manual smoke, remote HTTPS reverse-proxy smoke) per execution contract §6 — neither blocks code review.

## Scope

- 10 original product commits covering Waves 3-4 of the audit remediation plan
- 12 remediation commits covering R01 through R07 (including R04 ACL follow-up + R07 evidence)
- Master Bearer authentication on all API routes
- Host/Origin/CORS policy enforcement (structural Authority parsing, credentialed CORS)
- MCP Bearer-only authentication
- Desktop proof-and-session exchange with one-time guard
- Headless fail-closed startup (KOHARU_AUTH_SECRET / --auth-secret-file required before bind)
- Headless non-loopback binding requires explicit --allowed-host
- Desktop loopback-only binding enforcement
- UI auth gate: children, SSE, and Updater all gated behind authenticated state
- Same-origin credential transport for fetch and SSE
- Constant-time comparison for all three sensitive token verifications
- Integration test harness authenticated via default Bearer header

## Files Changed

```
Products (+ tests):
crates/koharu-rpc-security/src/lib.rs   (+ session state, constant-time comparison, host/origin/cors policy)
crates/koharu-rpc/src/api.rs            (+ auth middleware, session route)
crates/koharu-rpc/src/server.rs         (+ SecurityContext, OriginHostPolicy params)
crates/koharu-rpc/src/mcp/mod.rs        (+ MCP Bearer auth)
crates/koharu-rpc/src/lib.rs            (+ shared security-core re-export)
crates/koharu-rpc/Cargo.toml            (+ security-core dependency)
crates/koharu/src/security.rs           (+ DesktopAuth, HeadlessSecurityOptions)
crates/koharu/src/app.rs                (+ auth bootstrap, pre-bind validation)
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

Tests:
crates/koharu-rpc/src/api.rs            (+ bootstrap_routes_require_authentication)
crates/koharu-rpc/tests/origin_host.rs  (+ 6 real-listener boundary tests)
crates/koharu/src/app.rs                (+ 8 pre_bind_tests, 1 desktop_auth test)
ui/tests/lib/auth.test.ts               (+ 5 session + bootstrap rejection tests)
ui/tests/components/AuthBootstrap.test.tsx (+ 7 component gate tests)
ui/tests/app/providers.test.tsx         (+ 1 tree ordering test)
tests/integration-tests/src/client.rs   (+ bearer_token)
tests/integration-tests/src/harness.rs  (+ default_headers)
tests/integration-tests/Cargo.toml      (+ base64 workspace dep)

Config + infra:
Cargo.toml                              (+ subtle workspace dep)
Cargo.lock                              (dependency updates)
```
