# AR01 R03B Host/Origin/CORS Execution Card

> **For agentic workers:** Use `superpowers:test-driven-development`. Execute this card once, in order. Do not import requirements from superseded drafts.

**Goal:** Fail closed on malformed or untrusted request authority, emit exact credentialed CORS responses, and place the policy outside the complete API/MCP/static router.

**Architecture:** Replace string splitting in `security.rs` with structured `Authority`/`IpAddr` validation, then assemble API, MCP, and optional assets before adding one outer policy layer in `server.rs`. Reuse the existing router and middleware APIs; add no dependency and no new public abstraction.

**Tech Stack:** Rust 2024, Axum 0.8, Tokio.

**Review state:** FROZEN AFTER ONE IMPLEMENTATION-READINESS REVIEW. That review found two plan contradictions; this version contains only their exact corrections. No second plan review is permitted.

## Requirements summary

- Baseline: R03A commit `057b01e317696c470ca2773c900f3ebb96381fde` remains unchanged.
- Current Host/Origin defects are in `crates/koharu-rpc/src/security.rs:110-177`.
- Current middleware is applied before MCP and asset fallback assembly in `crates/koharu-rpc/src/server.rs:25-57` and `:118-137`.
- Production Desktop/headless callers in `crates/koharu/src/app.rs:118-166` are read-only for R03B.
- No dependency, generated file, UI, R03A, Desktop, Headless, `.omc`, `.omo`, contract, or ledger change is allowed.

## Frozen file boundary

- Modify: `crates/koharu-rpc/src/security.rs`
- Modify: `crates/koharu-rpc/src/server.rs`
- Create/Test: `crates/koharu-rpc/tests/origin_host.rs`

No fourth file may enter the implementation diff or commit.

## Frozen acceptance

1. Missing, duplicate, non-UTF-8, malformed, userinfo-bearing, path/query/fragment-bearing, invalid-port, or conflicting Host/URI authority fails closed before auth or handlers.
2. Only origin-form requests or absolute `http`/`https` request URIs are accepted; other absolute schemes fail closed.
3. Bracketed IPv6 permits only an empty suffix or `:<u16>`; invalid bracket content/suffix fails closed. Equivalent IPv6 spellings compare as `IpAddr`, not text.
4. Loopback Host must match the listener port. Remote HTTPS Host must match an explicitly parsed allowlist authority, with case-insensitive DNS and default `443` equivalence.
5. Missing Origin still requires a valid Host, emits no `Access-Control-Allow-*`, and appends one `Origin` token to `Vary`.
6. `null`, `*`, duplicate, malformed, non-HTTP(S), or Host-mismatched Origin returns `403`, no ACAO, `Vary: Origin`, and `Cache-Control: no-store`. The sole Host-mismatch exception is `Origin: http://localhost:3000` when the listener is loopback and `is_debug=true`.
7. Accepted non-preflight Origin receives exact reflected ACAO, `Access-Control-Allow-Credentials: true`, and one appended/deduplicated `Vary: Origin` without replacing an existing `Vary` value.
8. Accepted preflight returns `204`, `Cache-Control: no-store`, exact reflected ACAO, credentials, methods `GET, POST, PUT, PATCH, DELETE`, and headers `authorization, content-type, accept, last-event-id`. Unsupported requested methods/headers return `403` without ACAO.
9. Policy order is Host → Origin/CORS → endpoint auth → readiness → handler, and the same outer policy covers API, MCP, static fallback, and the Desktop session-assets serving constructor.
10. R03A session exchange/authentication and MCP Bearer-only behavior remain unchanged.

## Task 1: RED — lock the real boundary

Create `crates/koharu-rpc/tests/origin_host.rs` with shared raw-TCP request helpers and exactly these six `#[tokio::test]` functions:

1. `no_origin_still_rejects_forged_host_before_auth`
   - Assert missing/duplicate/forged Host and conflicting Host/absolute URI authority fail before auth.
   - Assert valid listener Host reaches auth (`401`) and valid Bearer reaches readiness (`503`).
   - Assert absolute `ftp` URI fails closed.
2. `preflight_returns_only_exact_credentialed_cors_headers`
   - Assert the sole debug exception (`loopback listener`, `is_debug=true`, `Origin: http://localhost:3000`) receives the exact headers in acceptance item 8.
   - Assert unsupported method/header, duplicate Origin, `null`, `*`, path/query/fragment, and non-HTTP(S) Origin fail closed.
3. `non_preflight_cors_preserves_existing_vary`
   - Start a small handler returning `Vary: Accept-Encoding` and prove accepted Origin appends one `Origin` token without replacement or duplication.
   - Prove a no-Origin response still varies on Origin but has no ACAO.
4. `remote_authority_normalizes_default_port_case_and_ipv6_without_userinfo`
   - Reject empty host, wildcard, userinfo, scheme, path/query/fragment, empty/out-of-range port, empty/invalid bracket host, and bracket suffix junk.
   - Accept DNS case/default `443` equivalence and equivalent bracketed IPv6 forms.
5. `outer_policy_wraps_mcp_and_static_assets`
   - Prove forged Host or denied Origin receives `403` on `/api/v1/meta`, `/mcp`, and `/`; valid Host reaches each route's existing auth/readiness/static behavior.
6. `desktop_session_asset_server_keeps_policy_outermost`
   - Exercise `serve_with_listener_and_assets_with_session`; prove Host/Origin rejection precedes session auth and asset fallback while valid cookie still reaches readiness.

Run RED-0 and RED-1:

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
test "$(git branch --show-current)" = codex/audit-remediation-sdd
test "$(git rev-parse HEAD)" = 057b01e317696c470ca2773c900f3ebb96381fde
git diff --cached --quiet
git diff --quiet -- crates/koharu-rpc/src/security.rs crates/koharu-rpc/src/server.rs
test "$(rg -c '^async fn (no_origin_still_rejects_forged_host_before_auth|preflight_returns_only_exact_credentialed_cors_headers|non_preflight_cors_preserves_existing_vary|remote_authority_normalizes_default_port_case_and_ipv6_without_userinfo|outer_policy_wraps_mcp_and_static_assets|desktop_session_asset_server_keeps_policy_outermost)' crates/koharu-rpc/tests/origin_host.rs)" = 6
bun cargo test -p koharu-rpc --test origin_host --no-run
set +e
RED_OUTPUT="$(bun cargo test -p koharu-rpc --test origin_host -- --nocapture 2>&1)"
RED_STATUS=$?
set -e
test "$RED_STATUS" -ne 0
grep -Fq 'running 6 tests' <<<"$RED_OUTPUT"
grep -Fq 'test result: FAILED. 0 passed; 6 failed' <<<"$RED_OUTPUT"
```

Stop if compilation fails, zero tests run, any test unexpectedly passes, or the first failures do not match the frozen defects.

## Task 2: minimal GREEN

### `crates/koharu-rpc/src/security.rs`

- Parse allowlist, Host, URI authority, and Origin with `axum::http::uri::Authority`; validate explicit ports as `u16` before trusting parsed host text.
- For bracketed authorities, locate `]` and accept only no suffix or `:<u16>`; require bracket contents to parse as `IpAddr`.
- Read Host and Origin with a single-value helper; missing policy, duplicate values, invalid UTF-8, missing authority, and Host/URI conflict return `403`.
- Store the full listener `SocketAddr` in `OriginHostPolicy`; compare IP literals through `IpAddr` and DNS names case-insensitively.
- Validate Host before inspecting Origin. Run `next` only after both boundaries pass.
- Append/deduplicate `Vary: Origin`; never overwrite existing `Vary` values.
- Use fixed allowed method/header constants from acceptance item 8. Reflect only a fully validated Origin.
- Add `Cache-Control: no-store` to preflight and denied-Origin responses.
- Delete the superseded `is_allowed` and `is_loopback_origin` string-splitting logic. Add no new public type.

### `crates/koharu-rpc/src/server.rs`

- Add one private helper that applies `enforce_origin_host` plus `Extension<OriginHostPolicy>`.
- Build API plus MCP first; add optional asset fallback second; apply the policy helper last.
- Preserve every public router/listener signature and all R03A authentication behavior.

## Task 3: GREEN and complete regression

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
rustfmt --edition 2024 crates/koharu-rpc/src/security.rs crates/koharu-rpc/src/server.rs crates/koharu-rpc/tests/origin_host.rs
bun cargo test -p koharu-rpc --test origin_host -- --nocapture
bun cargo test -p koharu-rpc
bun cargo check --workspace --all-targets
bun cargo clippy -p koharu-rpc --all-targets -- -D warnings
bun cargo fmt --all -- --check
git diff --check -- crates/koharu-rpc/src/security.rs crates/koharu-rpc/src/server.rs crates/koharu-rpc/tests/origin_host.rs
test "$(git status --porcelain=v1 -- crates/koharu-rpc/src/security.rs crates/koharu-rpc/src/server.rs crates/koharu-rpc/tests/origin_host.rs | cut -c4- | sort)" = "$(printf '%s\n' crates/koharu-rpc/src/security.rs crates/koharu-rpc/src/server.rs crates/koharu-rpc/tests/origin_host.rs | sort)"
```

Expected: origin/Host suite `6/6`, complete `koharu-rpc` crate regression passes, and every command exits `0`. Workspace integration tests are intentionally excluded because their existing client-auth `401` baseline belongs to R07, not R03B.

## Task 4: one independent implementation review

Dispatch one fresh read-only `code-reviewer` over:

- this frozen post-review card and the pre-review record below;
- the three-file implementation diff;
- RED output and Task 3 output;
- only the ten frozen acceptance items above.

The reviewer may report implementation defects against those frozen items, but may not add new plan requirements. Any HIGH/CRITICAL finding blocks the commit: fix only the reported defect inside the three-file boundary, rerun Task 3, then let the same reviewer verify only that finding. This scoped verification is not a new plan review and cannot add requirements.

## Task 5: single R03B commit

Run only after the implementation reviewer reports no HIGH/CRITICAL:

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
git add crates/koharu-rpc/src/security.rs crates/koharu-rpc/src/server.rs crates/koharu-rpc/tests/origin_host.rs
test "$(git diff --cached --name-only | sort)" = "$(printf '%s\n' crates/koharu-rpc/src/security.rs crates/koharu-rpc/src/server.rs crates/koharu-rpc/tests/origin_host.rs | sort)"
git diff --cached --check
git commit -m "fix(rpc): enforce host and origin policy" \
  -m "Co-Authored-By: Codex <noreply@openai.com>"
```

After the commit, R03B is CLOSED. Proceed to R04 only if the commit contains exactly the three frozen files. Reopen R03B only for a later change to those security/router boundaries or a reproducible violation of the ten frozen acceptance items.

## Risks and mitigations

- **Proxy authority ambiguity:** accept only structurally valid Host/URI authority sources and require equality when both exist.
- **Cache poisoning:** every Origin-dependent path varies on Origin; denied/preflight responses are `no-store`.
- **Middleware bypass:** tests exercise API, MCP, static fallback, and the production session-assets constructor through real listeners.
- **Scope creep:** no dependency or fourth file is permitted; R04 remains separate.

## Single implementation-readiness review record

- Reviewed SHA-256: `20e068bec15f883c9fdc611913a51cc9364d6a0fff274719da3190b065cf04a4`.
- Finding 1: debug `localhost:3000` was not frozen as the sole Host-mismatch exception. Corrected in acceptance item 6 and RED test 2.
- Finding 2: workspace tests have an existing integration-client authentication failure outside R03B. Removed that false gate; complete `koharu-rpc` regression remains mandatory.
- No other acceptance, file, implementation, or governance requirement was added.
