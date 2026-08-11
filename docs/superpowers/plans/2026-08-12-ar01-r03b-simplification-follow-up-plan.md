# AR01 R03B-S Simplification Follow-up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to execute this single-owner cleanup card in order. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the three duplication/single-use-helper findings from the post-R03B Ponytail review without changing Host/Origin/CORS behavior.

**Architecture:** Keep commit `583126bf260d573bfc87176e0ea11d5030013e2a` as the immutable R03B security baseline. Perform a two-file behavior-preserving cleanup, reuse the existing six real-listener regression tests, and create a separate refactor commit.

**Tech Stack:** Rust 2024, Axum 0.8, Tokio.

## Global Constraints

- This is `R03B-S`, not a reopening or amendment of the committed R03B implementation.
- Modify only `crates/koharu-rpc/src/security.rs` and `crates/koharu-rpc/tests/origin_host.rs`.
- Do not modify `server.rs`, dependencies, generated files, UI, ledger, `.omc`, `.omo`, or existing plan history.
- Add no abstraction, dependency, feature, public API, test function, or acceptance criterion.
- This review found only unnecessary complexity. Do not manufacture a failing behavior test: lock the current behavior GREEN, simplify, then prove it remains GREEN.
- Preserve all ten frozen R03B acceptance items and all six existing `origin_host` tests.

---

### Task 1: Freeze the post-R03B baseline

**Files:**
- Read: `crates/koharu-rpc/src/security.rs`
- Read: `crates/koharu-rpc/tests/origin_host.rs`

**Interfaces:**
- Consumes: committed R03B behavior at `583126bf260d573bfc87176e0ea11d5030013e2a`
- Produces: fresh pre-edit GREEN evidence for the existing six boundary tests

- [ ] **Step 1: Verify branch, baseline, index, and product-file cleanliness**

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
test "$(git branch --show-current)" = codex/audit-remediation-sdd
test "$(git rev-parse HEAD)" = 583126bf260d573bfc87176e0ea11d5030013e2a
git diff --cached --quiet
git diff --quiet -- \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/tests/origin_host.rs
```

Expected: every command exits `0`. Unrelated ledger and plan changes may remain unstaged.

- [ ] **Step 2: Record the behavior-lock GREEN baseline**

```bash
bun cargo test -p koharu-rpc --test origin_host -- --nocapture
```

Expected: exactly `6 passed; 0 failed`; a zero-test result or any failure is a hard stop.

---

### Task 2: Remove duplicated policy lists and the single-use port helper

**Files:**
- Modify: `crates/koharu-rpc/src/security.rs:71-73,256-263,306-321`
- Test: `crates/koharu-rpc/tests/origin_host.rs`

**Interfaces:**
- Consumes: `ALLOWED_METHODS`, `ALLOWED_HEADERS`, `Authority::port_u16`, and the existing six boundary tests
- Produces: the same private Host/Origin/CORS behavior with one source of truth per allowlist

- [ ] **Step 1: Delete the duplicate header-name constant**

Delete:

```rust
const ALLOWED_HEADER_NAMES: &[&str] = &["authorization", "content-type", "accept", "last-event-id"];
```

Keep `ALLOWED_METHODS` and `ALLOWED_HEADERS` as the exact response-header values required by R03B.

- [ ] **Step 2: Validate requested methods and headers from the existing response constants**

Replace the duplicated `matches!` method list and `ALLOWED_HEADER_NAMES` lookup in `valid_preflight` with:

```rust
if !ALLOWED_METHODS.split(", ").any(|allowed| allowed == method) {
    return false;
}

requested.is_none_or(|requested| {
    requested.split(',').all(|name| {
        ALLOWED_HEADERS
            .split(", ")
            .any(|allowed| allowed.eq_ignore_ascii_case(name.trim()))
    })
})
```

An empty or unknown requested header still fails because it matches no allowed token.

- [ ] **Step 3: Inline the single-use `authority_port` helper**

Delete `authority_port` and leave `authorities_match` as:

```rust
fn authorities_match(left: &Authority, right: &Authority, default_port: u16) -> bool {
    hosts_match(left.host(), right.host())
        && left.port_u16().unwrap_or(default_port)
            == right.port_u16().unwrap_or(default_port)
}
```

- [ ] **Step 4: Run the focused regression immediately**

```bash
bun cargo test -p koharu-rpc --test origin_host -- --nocapture
```

Expected: exactly `6 passed; 0 failed`.

---

### Task 3: Reuse the test header parser

**Files:**
- Modify: `crates/koharu-rpc/tests/origin_host.rs:168-185`

**Interfaces:**
- Consumes: existing `header_values(head, name) -> Vec<&str>`
- Produces: unchanged `header(head, name) -> Option<&str>` without a second parser body

- [ ] **Step 1: Delegate the single-value helper to the existing parser**

Replace `header` with:

```rust
fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    header_values(head, name).into_iter().next()
}
```

Do not add a generic iterator abstraction; this code is test-only and the existing `Vec` is sufficient.

- [ ] **Step 2: Run the focused regression again**

```bash
bun cargo test -p koharu-rpc --test origin_host -- --nocapture
```

Expected: exactly `6 passed; 0 failed`.

---

### Task 4: Complete regression and independent simplification review

**Files:**
- Verify: `crates/koharu-rpc/src/security.rs`
- Verify: `crates/koharu-rpc/tests/origin_host.rs`

**Interfaces:**
- Consumes: the two-file cleanup diff
- Produces: verified behavior preservation and a read-only over-engineering verdict

- [ ] **Step 1: Run all required gates sequentially**

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
rustfmt --edition 2024 \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/tests/origin_host.rs
bun cargo test -p koharu-rpc --test origin_host -- --nocapture
bun cargo test -p koharu-rpc
bun cargo check -p koharu-rpc --all-targets
bun cargo clippy -p koharu-rpc --all-targets -- -D warnings
bun cargo fmt --all -- --check
git diff --check -- \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/tests/origin_host.rs
test "$(git diff --name-only -- \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/tests/origin_host.rs | sort)" = "$(printf '%s\n' \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/tests/origin_host.rs | sort)"
```

Expected: six focused tests pass, complete `koharu-rpc` regression passes, and every static gate exits `0`.

- [ ] **Step 2: Perform one fresh read-only `ponytail-review`**

Review only the two-file diff against these three accepted findings:

1. no duplicated method/header allowlist values;
2. no single-use `authority_port` wrapper;
3. no duplicate response-header parser body.

Any newly proposed feature, abstraction, dependency, acceptance criterion, or unrelated cleanup is out of scope. If the reviewer finds remaining complexity in these exact edits, simplify it and rerun Task 4 Step 1.

---

### Task 5: Create the isolated cleanup commit

**Files:**
- Commit: `crates/koharu-rpc/src/security.rs`
- Commit: `crates/koharu-rpc/tests/origin_host.rs`

**Interfaces:**
- Consumes: passing Task 4 evidence and a clear simplification review
- Produces: one behavior-preserving conventional commit

- [ ] **Step 1: Stage exactly two files and verify the index**

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
git add \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/tests/origin_host.rs
test "$(git diff --cached --name-only | sort)" = "$(printf '%s\n' \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/tests/origin_host.rs | sort)"
git diff --cached --check
```

- [ ] **Step 2: Commit without amending R03B**

```bash
git commit -m "refactor(rpc): simplify origin policy helpers" \
  -m "Co-Authored-By: Codex <noreply@openai.com>"
```

- [ ] **Step 3: Verify the commit boundary**

```bash
test "$(git diff-tree --no-commit-id --name-only -r HEAD | sort)" = "$(printf '%s\n' \
  crates/koharu-rpc/src/security.rs \
  crates/koharu-rpc/tests/origin_host.rs | sort)"
git status --short
```

Expected: the new commit contains exactly two files; unrelated ledger and plan changes remain unstaged.

## Self-review

- Spec coverage: all three Ponytail findings map to Tasks 2 and 3; no R03B security acceptance item changes.
- Placeholder scan: no TBD, TODO, deferred implementation, or unspecified test step.
- Type consistency: all names and signatures match commit `583126bf`.
- Complexity ceiling: expected reduction is approximately nine lines with no new abstraction.
