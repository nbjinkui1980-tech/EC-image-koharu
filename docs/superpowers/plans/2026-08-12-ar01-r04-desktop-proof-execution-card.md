# AR01 R04 Desktop Proof Execution Card

> **For agentic workers:** REQUIRED SUB-SKILL: Use `oh-my-codex:ralph`. Execute once in order: RED → minimal GREEN → regression/build → independent review → isolated commit.

**Goal:** Register the existing one-time Desktop bootstrap proof as a real Tauri command backed by the same `DesktopAuth` instance used to create the RPC browser session.

**Architecture:** Add one private generic Builder helper in `app.rs`; production and the Tauri mock-runtime test both call it. Reuse Tauri 2.11's built-in `test` feature and existing `DesktopAuth`; add no dependency, capability, or public abstraction.

**Tech Stack:** Rust 2024, Tauri 2.11, Tokio.

**Review state:** CORRECTED REVIEW CANDIDATE — execute only after the same reviewer clears the five recorded findings and the user approves the resulting SHA-256.

## Frozen boundary

- Modify: `crates/koharu/Cargo.toml`
- Modify: `crates/koharu/src/app.rs`
- Modify: `crates/koharu/src/security.rs`

No fourth file, `Cargo.lock`, capability, UI, RPC, generated file, ledger, `.omc`, or `.omo` may enter the implementation diff or commit. Baseline HEAD is `583126bf260d573bfc87176e0ea11d5030013e2a`.

## Frozen acceptance

1. Tauri IPC command `desktop_bootstrap_proof` is registered on the production Builder.
2. The Builder manages the same `DesktopAuth` value whose cloned `BrowserSessionState` is passed to the RPC server.
3. A non-`main` window is rejected without consuming the proof.
4. The `main` window receives a 43-character URL-safe no-padding proof; the cloned RPC session accepts the decoded 32 bytes.
5. A second `main` invocation fails with `proof already consumed`.
6. No secret/proof enters logs, URLs, browser storage, error bodies, or capabilities; R05/R06 remain out of scope.

## Task 0: preflight

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
set -a
source .env
set +a
test "$(git branch --show-current)" = codex/audit-remediation-sdd
test "$(git rev-parse HEAD)" = 583126bf260d573bfc87176e0ea11d5030013e2a
test "${KOHARU_SHARED_TARGET_DIR:?}" = /Volumes/G/EC-image-koharu/target
test "${CARGO_TARGET_DIR:?}" = "$KOHARU_SHARED_TARGET_DIR"
CONTRACT_SHA="$(shasum -a 256 docs/superpowers/plans/2026-08-11-ar01-remediation-execution-contract.md | awk '{print $1}')"
CARD_SHA="$(shasum -a 256 docs/superpowers/plans/2026-08-12-ar01-r04-desktop-proof-execution-card.md | awk '{print $1}')"
test "$CONTRACT_SHA" = ae6630679ea459ed7d4b87919bd957b273561f5dfbbf5ee53a5f2bf60820de59
test "${R04_APPROVED_CARD_SHA:?must be supplied by the approval prompt}" = "$CARD_SHA"
git diff --cached --quiet
test "$(git diff HEAD --name-only | sort)" = \
  docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
```

Before any product edit, use `apply_patch` to append these two approval anchors to the unstaged evidence ledger, using the already verified values from the shell:

```text
- R04 execution contract SHA-256: ae6630679ea459ed7d4b87919bd957b273561f5dfbbf5ee53a5f2bf60820de59
- R04 approved card SHA-256: value of R04_APPROVED_CARD_SHA supplied by the approval prompt
```

Then verify the exact values and restore the pre-edit gates:

```bash
grep -Fq -- '- R04 execution contract SHA-256: ae6630679ea459ed7d4b87919bd957b273561f5dfbbf5ee53a5f2bf60820de59' \
  docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
grep -Fq -- "- R04 approved card SHA-256: $R04_APPROVED_CARD_SHA" \
  docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md
git diff --cached --quiet
git diff --quiet -- \
  crates/koharu/Cargo.toml \
  crates/koharu/src/app.rs \
  crates/koharu/src/security.rs
```

Stop if any command fails. Preserve the unstaged ledger and untracked plans.

## Task 1: real IPC RED

### Step 1 — enable only Tauri's built-in test module

Append to `crates/koharu/Cargo.toml`:

```toml
[dev-dependencies]
tauri = { workspace = true, features = ["test"] }
```

This is a test-only feature on the existing direct dependency. `Cargo.lock` must remain byte-identical.

### Step 2 — add one failing test

Add `#[cfg(test)] mod tests` at the end of `app.rs`. The test builds `tauri::test::mock_builder()` without a handler, creates a `main` webview, sends this real IPC request, and expects success:

```rust
fn invoke(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
) -> Result<String, serde_json::Value> {
    tauri::test::get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: "desktop_bootstrap_proof".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
    )
    .map(|body| body.deserialize::<String>().unwrap())
}

#[test]
fn desktop_auth_command_uses_managed_one_time_proof() {
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    invoke(&main).expect("desktop_bootstrap_proof command must be registered");
}
```

Run RED-0, then RED-1:

```bash
bun cargo test -p koharu desktop_auth_command_uses_managed_one_time_proof --no-run
set +e
RED_OUTPUT="$(bun cargo test -p koharu desktop_auth_command_uses_managed_one_time_proof -- --nocapture 2>&1)"
RED_STATUS=$?
set -e
test "$RED_STATUS" -ne 0
grep -Fq 'running 1 test' <<<"$RED_OUTPUT"
grep -Fq 'Command desktop_bootstrap_proof not found' <<<"$RED_OUTPUT"
grep -Fq 'test result: FAILED. 0 passed; 1 failed' <<<"$RED_OUTPUT"
```

Any compile failure, zero-test result, unrelated failure, or unexpected pass is a hard stop.

## Task 2: minimal GREEN

### Step 1 — make the existing command runtime-generic

In `security.rs`, change only its window parameter:

```rust
window: tauri::Window<impl tauri::Runtime>,
```

Keep the existing main-window guard, `take_proof`, encoding, and error strings unchanged.

### Step 2 — add and use one private Builder helper

In `app.rs`, add:

```rust
fn desktop_builder<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    desktop_auth: crate::security::DesktopAuth,
) -> tauri::Builder<R> {
    builder
        .manage(desktop_auth)
        .invoke_handler(tauri::generate_handler![
            crate::security::desktop_bootstrap_proof
        ])
}
```

Replace only `tauri::Builder::default()` with `desktop_builder(tauri::Builder::default(), desktop_auth)`. The server clones must still be obtained before moving `desktop_auth` into this helper.

### Step 3 — finish the same IPC test

Replace the RED test module with:

```rust
#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::desktop_builder;

    fn invoke(window: &tauri::WebviewWindow<tauri::test::MockRuntime>) -> Result<String, String> {
        tauri::test::get_ipc_response(
            window,
            tauri::webview::InvokeRequest {
                cmd: "desktop_bootstrap_proof".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.into(),
            },
        )
        .map(|body| body.deserialize::<String>().unwrap())
        .map_err(|value| value.as_str().unwrap().to_owned())
    }

    #[test]
    fn desktop_auth_command_uses_managed_one_time_proof() {
        let desktop_auth = crate::security::DesktopAuth::generate().unwrap();
        let session = desktop_auth.browser_session_state();
        let app = desktop_builder(tauri::test::mock_builder(), desktop_auth)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        let other = tauri::WebviewWindowBuilder::new(&app, "other", Default::default())
            .build()
            .unwrap();
        assert_eq!(invoke(&other), Err("unauthorized window".into()));

        let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let encoded = invoke(&main).unwrap();
        assert_eq!(encoded.len(), 43);
        let decoded: [u8; 32] = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .unwrap()
            .try_into()
            .unwrap();
        assert!(session.consume_proof(&decoded));
        assert_eq!(invoke(&main), Err("proof already consumed".into()));
    }
}
```

The final test remains one function named `desktop_auth_command_uses_managed_one_time_proof`; add no second test or helper abstraction beyond `invoke`.

Run GREEN:

```bash
GREEN_OUTPUT="$(bun cargo test -p koharu desktop_auth_command_uses_managed_one_time_proof -- --nocapture 2>&1)"
grep -Fq 'running 1 test' <<<"$GREEN_OUTPUT"
grep -Fq 'test result: ok. 1 passed; 0 failed' <<<"$GREEN_OUTPUT"
```

Expected: exactly `1 passed; 0 failed` in the `koharu` library target. The binary target may report `0 tests` after the library test has run; that is not the filtered-test hard stop.

## Task 3: regression and Desktop build

Run sequentially:

```bash
set -euo pipefail
GREEN_OUTPUT="$(bun cargo test -p koharu desktop_auth_command_uses_managed_one_time_proof -- --nocapture 2>&1)"
grep -Fq 'running 1 test' <<<"$GREEN_OUTPUT"
grep -Fq 'test result: ok. 1 passed; 0 failed' <<<"$GREEN_OUTPUT"
bun cargo test -p koharu
RPC_OUTPUT="$(bun cargo test -p koharu-rpc session_exchange -- --nocapture 2>&1)"
grep -Fq 'running 3 tests' <<<"$RPC_OUTPUT"
grep -Fq 'test result: ok. 3 passed; 0 failed' <<<"$RPC_OUTPUT"
bun cargo check -p koharu -p koharu-rpc --all-targets
bun cargo clippy -p koharu -p koharu-rpc --all-targets -- -D warnings
bun cargo fmt --all -- --check
bun run build
git diff --check -- \
  crates/koharu/Cargo.toml \
  crates/koharu/src/app.rs \
  crates/koharu/src/security.rs
test -z "$(git diff --name-only -- Cargo.lock crates/koharu/capabilities)"
test "$(git diff HEAD --name-only | sort)" = "$(printf '%s\n' \
  crates/koharu/Cargo.toml \
  crates/koharu/src/app.rs \
  crates/koharu/src/security.rs \
  docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md | sort)"
git diff --cached --quiet
```

Every command must exit `0`; otherwise stop before review or commit.

## Task 4: one independent implementation review

Dispatch one fresh read-only `code-reviewer` over the three-file diff and RED/GREEN/Task 3 outputs. The review is limited to the six frozen acceptance items, shared-state identity, Tauri IPC reachability, secret exposure, file scope, and test truthfulness.

Any HIGH/CRITICAL blocks the commit. Fix only that finding inside the frozen three files, rerun Task 3, and request scoped verification from the same reviewer. New features, UI bootstrap, headless flow, capability changes, or unrelated cleanup are scope creep.

## Task 5: isolated commit

```bash
set -euo pipefail
test "$(git diff HEAD --name-only | sort)" = "$(printf '%s\n' \
  crates/koharu/Cargo.toml \
  crates/koharu/src/app.rs \
  crates/koharu/src/security.rs \
  docs/superpowers/evidence/2026-08-10-ar01-authentication-replay-ledger.md | sort)"
git diff --cached --quiet
git add \
  crates/koharu/Cargo.toml \
  crates/koharu/src/app.rs \
  crates/koharu/src/security.rs
test "$(git diff --cached --name-only | sort)" = "$(printf '%s\n' \
  crates/koharu/Cargo.toml \
  crates/koharu/src/app.rs \
  crates/koharu/src/security.rs | sort)"
git diff --cached --check
git commit -m "fix(desktop): register bootstrap proof command" \
  -m "Co-Authored-By: Codex <noreply@openai.com>"
test "$(git diff-tree --no-commit-id --name-only -r HEAD | sort)" = "$(printf '%s\n' \
  crates/koharu/Cargo.toml \
  crates/koharu/src/app.rs \
  crates/koharu/src/security.rs | sort)"
```

After the executable commit-path gate passes, append RED/GREEN/review/commit-SHA evidence to the existing ledger but keep the ledger unstaged. Then R04 is CLOSED and R05 may be drafted.

## Implementation-readiness evidence

- Disposable `git archive` RED prototype: one test ran and failed with `Command desktop_bootstrap_proof not found`; exit `101`.
- Disposable GREEN prototype: the same test passed `1/1`, including non-main rejection, same proof accepted by the cloned RPC session, and second-call rejection.
- Disposable GREEN static gates: `bun cargo check -p koharu --all-targets`, Clippy `-D warnings`, and workspace fmt check passed.
- Disposable prototypes did not modify the execution worktree and are not Task 3 evidence; all gates rerun during execution.
