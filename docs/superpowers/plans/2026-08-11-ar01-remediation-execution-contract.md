# AR01 Remediation Execution Contract

**Status:** DRAFT — independent review and explicit user approval are required before execution.

**Purpose:** This document is the short, authoritative control plane for the remaining AR01 remediation. It defines sequencing, evidence, review, and readiness claims. It is not an implementation plan and does not authorize product-code changes by itself.

## 1. Authoritative baseline

- Worktree: `/Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay`
- Branch: `codex/audit-remediation-sdd`
- Frozen product-code baseline: `9e46ab84d5afceb2417a55b8cadda000ffd5580f`
- Preserved commits:
  - `31e4d8c7 fix(rpc): wire authenticated browser sessions`
  - `9e46ab84 fix(mcp): scope bearer authentication to mcp routes`
- Superseded implementation-plan digest retained for history:
  - `.omx/plans/2026-08-11-ar01-authentication-closeout-implementation-plan.md`
  - SHA-256: `ea3b1ba21efd6e427939a2efa2357e0a89dc7f661e9ff51cdacc7c215efd384c`
- Evidence ledger:
  - `docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md`

The execution owner must compute the SHA-256 of this contract and the active card plan immediately after approval and record both values in the evidence ledger before editing product code. A document cannot contain its own stable digest; the ledger entry is the external approval anchor. Any later plan edit invalidates approval and requires a new independent review and new recorded digest.

## 2. Environment and dirty-state contract

Before every card:

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
test "$(git branch --show-current)" = codex/audit-remediation-sdd
test "${KOHARU_SHARED_TARGET_DIR:?}" = /Volumes/G/EC-image-koharu/target
test "${CARGO_TARGET_DIR:?}" = "$KOHARU_SHARED_TARGET_DIR"
git status --short
```

Rules:

- Reuse the one shared Cargo target above. Do not create per-card target directories.
- Preserve unrelated dirty work. Do not reset, clean, checkout, stash, or broadly format the worktree.
- Before R03A, the only pre-existing tracked dirty files are:
  - `crates/koharu-rpc/src/api.rs` — existing R03A RED test
  - `docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md` — remains unstaged until final evidence commit
- Plan documents may be untracked while under review. They are governance artifacts, not permission to stage product code.
- No generated files, `.omc/`, `.omo/`, `ui/next.config.ts`, dependency manifests, lockfiles, or Tauri capabilities may change unless a later approved card explicitly names them.

## 3. Sequential card model

Cards are authored, reviewed, approved, executed, reviewed again, and committed one at a time:

1. **R03A — bootstrap business-route authentication**
2. **R03B — complete-router Host/Origin/CORS policy**
3. **R04 — Desktop Tauri proof command/state wiring**
4. **R05 — UI authentication gate for children/SSE/updater**
5. **R06 — Headless validation before bind**
6. **R07 — constant-time/integration harness and final evidence**

R03A is specified only by `docs/superpowers/plans/2026-08-11-ar01-r03a-bootstrap-auth-execution-card.md`. R03B is separated into `docs/superpowers/plans/2026-08-11-ar01-r03b-host-origin-cors-scope-card.md` and receives its short execution card only after R03A commits. R04 must not be authored until R03 has passed its closeout review and commit gates. The same predecessor rule applies through R07.

## 4. Per-card mandatory lifecycle

Every card uses one owner and this exact order:

1. **RED-0:** compile the existing harness or new test target without changing product code.
2. **RED-1:** run the exact behavior test; it must execute at least one test and fail for the named defect.
3. **GREEN:** make the smallest root-cause change in only the card's frozen files.
4. **Targeted verification:** rerun the RED test, adjacent contract tests, the touched crate/UI checks, and `git diff --check`.
5. **Independent review:** a read-only `code-reviewer` reviews only the card diff and recorded evidence.
6. **Fix loop:** unresolved HIGH/CRITICAL findings block the card; repair inside the same card and request scoped re-review.
7. **Commit gate:** stage only the frozen card files, run `git diff --cached --check`, verify cached path names, and create one conventional commit with truthful AI co-author attribution.
8. **Ledger:** append RED, GREEN, review verdict, and commit SHA. Keep the ledger unstaged until the final evidence card.

Hard stop conditions:

- RED unexpectedly passes or runs zero tests.
- A required verification command fails.
- The diff contains a file outside the card's frozen set.
- Independent review has unresolved HIGH/CRITICAL findings.
- Shared-target or storage preflight fails.
- Product behavior would require an unapproved dependency, generated-file change, capability expansion, or materially wider interface.

## 5. Security invariants

- Order: Host validation → Origin/CORS validation → endpoint authentication → readiness → handler.
- Missing or wrong credentials return `401`; a valid credential rejected by Host/Origin policy returns `403` before handler side effects.
- `/api/v1/auth/session` validates master Bearer or one-time Desktop proof itself; it is the only API route outside ordinary API auth middleware.
- REST, SSE, Binary, downloads, and operations accept master Bearer or a valid `koharu_session` cookie.
- MCP accepts master Bearer only.
- Static assets remain unauthenticated but are inside Host/Origin enforcement.
- No Origin still requires a valid Host and must not produce CORS response headers.
- No wildcard, `null`, unchecked origin reflection, userinfo, path, query, or fragment is accepted at the authority boundary.
- Credential material never enters URL/query, response body, normal logs, browser storage, console output, or telemetry.

## 6. Readiness claims

- Completing R03 permits drafting R04; it does not make the branch merge-ready.
- `MERGE-READY` requires all cards, final automated gates, independent whole-branch `code-reviewer` and `architect` approval, and recorded Desktop manual smoke PASS.
- If remote HTTPS reverse-proxy smoke is not executed, mark it `PENDING`; the branch cannot be called remote-deployment-ready.
- Docker/T06 is outside this AR01 closeout. Even a merge-ready AR01 branch must not be described as release-ready or Docker-ready on this evidence alone.

## 7. Supersession

The following drafts are retained only as review history and must not be executed:

- `.omx/plans/2026-08-11-ar01-authentication-closeout-plan.md`
- `.omx/plans/2026-08-11-ar01-authentication-closeout-implementation-plan.md`
- `docs/superpowers/plans/2026-08-11-ar01-r03-host-origin-cors-implementation-plan.md`

This contract plus the current short execution card are the only execution authorities.
