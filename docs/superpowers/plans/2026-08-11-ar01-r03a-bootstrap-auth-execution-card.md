# AR01 R03A Bootstrap Authentication Completion Record

**Status:** COMPLETED — NON-EXECUTABLE

**Result:** Bootstrap business routes now require master Bearer or a valid browser-session cookie. `/api/v1/auth/session` remains outside ordinary API authentication and validates its own bootstrap credential.

**Commit:** `057b01e317696c470ca2773c900f3ebb96381fde`

**Changed product file:** `crates/koharu-rpc/src/api.rs`

This record closes R03A. It is not an implementation plan and must not be replayed, amended with new acceptance criteria, or reviewed again as an executable card.

## Frozen acceptance and result

1. **PASS** — `/api/v1/auth/session` remains outside ordinary API auth and continues to validate its own Bearer/proof.
2. **PASS** — GET `/events`, GET/POST `/downloads`, GET `/operations`, and DELETE `/operations/{id}` reject unauthenticated requests with `401`; wrong Bearer credentials are rejected by the shared authentication boundary.
3. **PASS** — Rejected DELETE does not trigger its cancellation flag.
4. **PASS** — Master Bearer and valid `koharu_session` reach bootstrap handlers; protected app routes reach readiness and return `503` while unready.
5. **PASS** — MCP remains master-Bearer-only.
6. **PASS** — The R03A commit contains only `crates/koharu-rpc/src/api.rs`.

These six items are the complete R03A acceptance set. Draft contracts, later cards, ledger formatting, optional hardening, or reviewer preferences cannot add R03A requirements retroactively.

## RED evidence

- Test-only patch SHA-256: `98012a8109e89a3f853b4432b43a3f285762804984c707eefb56c8c1d16b6330`.
- Command: `bun cargo test -p koharu-rpc --lib bootstrap_routes_require_authentication -- --nocapture`.
- Result: exactly one test ran and failed; unauthenticated `/api/v1/events` returned `200` instead of the required `401`.

## GREEN evidence

- `bootstrap_routes_require_authentication`: `1/1` passed.
- `session_exchange`: `3/3` passed.
- `mcp_remains_master_bearer_only`: `1/1` passed.
- Full `bun cargo test -p koharu-rpc`: 32 library tests, 1 binary test, and 4 integration tests passed.
- `bun cargo check -p koharu-rpc --all-targets`: passed.
- Scoped Clippy with only the pre-existing R03B-owned `manual_split_once` rule allowed: passed.
- `bun cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Independent review

- Scope: the frozen six-item acceptance set, the `api.rs` diff, and RED/GREEN evidence.
- Verdict: **APPROVE**.
- Findings: 0 CRITICAL, 0 HIGH, 0 MEDIUM, 0 LOW.

## Closure rule

R03A may be reopened only if:

- `crates/koharu-rpc/src/api.rs` authentication/router behavior changes after commit `057b01e317696c470ca2773c900f3ebb96381fde`; or
- a reproducible request demonstrates that one of the six frozen acceptance items no longer holds.

Missing retrospective governance metadata, changes to draft documents, or additional desirable tests are not grounds to reopen R03A. Final evidence aggregation remains a later evidence-card responsibility.

## R03B boundary

Host, Origin, CORS, authority parsing, response cache headers, and middleware ordering belong exclusively to R03B. R03B must use its own short card, RED, GREEN, independent review, and commit without modifying this completed R03A record.
