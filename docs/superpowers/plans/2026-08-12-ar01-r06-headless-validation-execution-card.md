# AR01 R06 Headless Validation Before Bind Execution Card

**Status:** CLOSED — implemented by `bf1a3adb9208555ef6276d7c671bbd842bd4da3c`; the integration regression introduced by `67ba1b2f` was restored and verified by `68f7b68844357738a56a036a1db47a0c9c74ceec`. Retained as the historical execution card.

> **For agentic workers:** Execute once in strict order: RED → minimal GREEN → regression+build → independent review → one commit. This is R06 only — R07, dependency upgrades, generated files, or UI/backend refactors are out of scope.

**Goal:** Add pre-bind validation so the server never opens a listening socket before secrets and host policy are verified. Reject misconfigured Desktop `--host`, headless-only flags in Desktop mode, and headless remote exposure without explicit allowed hosts.

**Architecture:** Move `HeadlessSecurityOptions::resolve()` before `TcpListener::bind()` in `app.rs`, add a pre-bind guard block that validates Desktop loopback constraint and headless-only flag rejection.

**Tech Stack:** Rust 2024, Tokio, anyhow. No new dependencies.

**Baseline:** Commit `47b14322` (R05) on `codex/audit-remediation-sdd`.

## Frozen file boundary

- Modify: `crates/koharu/src/app.rs`
- Modify (if needed for testability): `crates/koharu/src/security.rs`
- Create/Modify test: `crates/koharu/src/app.rs` (add `#[cfg(test)] mod pre_bind_tests` or expand existing test module)

No other file may enter the implementation diff or commit. `Cargo.toml`, `Cargo.lock`, generated files, UI code, RPC code, `.omc`, `.omo`, and ledgers are out of scope.

## Frozen acceptance

1. Headless mode with no `KOHARU_AUTH_SECRET` and no `--auth-secret-file` → process exits non-zero before any `TcpListener::bind()`.
2. Headless mode with invalid secret (wrong length, bad base64) → exits non-zero before bind.
3. Headless mode with non-loopback `--host` (e.g. `0.0.0.0`) and empty `--allowed-host` → exits non-zero before bind.
4. Desktop mode with non-loopback `--host` (anything other than `127.0.0.1`, `::1`, `localhost`) → exits non-zero before bind.
5. Desktop mode with `--auth-secret-file` → exits non-zero before bind (headless-only flag).
6. Desktop mode with `--allowed-host` → exits non-zero before bind (headless-only flag).
7. Headless with valid secret + loopback host + non-empty allowed-hosts → listener binds, server starts.
8. Headless with valid secret + non-loopback host + matching allowed-hosts → listener binds.
9. Desktop with `--host 127.0.0.1` and no headless flags → listener binds, Tauri starts (existing behavior preserved).

## Task 1: RED — add failing pre-bind guard tests

Add a `#[cfg(test)] mod pre_bind_tests` at the end of `crates/koharu/src/app.rs`. Each test calls a new `validate_pre_bind` function (initially a stub that always returns `Ok(())`) with representative CLI-like structs.

### Step 1 — define the pre-bind input struct

```rust
#[derive(Default)]
struct PreBindInput {
    headless: bool,
    host: String,
    allowed_hosts: Vec<String>,
    auth_secret_file: Option<String>,
    has_env_secret: bool,
}
```

### Step 2 — stub function

```rust
fn validate_pre_bind(_input: &PreBindInput) -> anyhow::Result<()> {
    Ok(())
}
```

### Step 3 — RED test cases

```rust
#[cfg(test)]
mod pre_bind_tests {
    use super::*;

    #[test]
    fn headless_without_secret_fails_before_bind() {
        let input = PreBindInput { headless: true, host: "127.0.0.1".into(), ..Default::default() };
        assert!(validate_pre_bind(&input).is_err());
    }

    #[test]
    fn headless_with_non_loopback_host_and_empty_allowed_fails() {
        let input = PreBindInput {
            headless: true,
            host: "0.0.0.0".into(),
            has_env_secret: true,
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_err());
    }

    #[test]
    fn desktop_with_non_loopback_host_fails_before_bind() {
        let input = PreBindInput { host: "0.0.0.0".into(), ..Default::default() };
        assert!(validate_pre_bind(&input).is_err());
    }

    #[test]
    fn desktop_with_auth_secret_file_fails() {
        let input = PreBindInput {
            auth_secret_file: Some("/tmp/secret".into()),
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_err());
    }

    #[test]
    fn desktop_with_allowed_hosts_fails() {
        let input = PreBindInput { allowed_hosts: vec!["example.com".into()], ..Default::default() };
        assert!(validate_pre_bind(&input).is_err());
    }

    #[test]
    fn headless_with_secret_and_loopback_passes() {
        let input = PreBindInput {
            headless: true,
            host: "127.0.0.1".into(),
            has_env_secret: true,
            allowed_hosts: vec!["example.com:443".into()],
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_ok());
    }

    #[test]
    fn headless_with_secret_allowed_hosts_and_wildcard_host_passes() {
        let input = PreBindInput {
            headless: true,
            host: "0.0.0.0".into(),
            has_env_secret: true,
            allowed_hosts: vec!["example.com".into()],
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_ok());
    }

    #[test]
    fn desktop_with_defaults_passes() {
        let input = PreBindInput { host: "127.0.0.1".into(), ..Default::default() };
        assert!(validate_pre_bind(&input).is_ok());
    }
}
```

### Step 4 — RED-0 and RED-1

```bash
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
bun cargo test -p koharu pre_bind --no-run
# Expected: compiles
bun cargo test -p koharu pre_bind -- --nocapture
# Expected: 5 FAILED (headless/desktop violation tests), 3 PASSED (happy path + defaults)
```

## Task 2: GREEN — implement `validate_pre_bind`

Replace the stub with the real implementation. `validate_pre_bind` checks in this exact order:

```rust
fn validate_pre_bind(input: &PreBindInput) -> anyhow::Result<()> {
    // 1. Desktop must not receive headless-only flags
    if !input.headless {
        if input.auth_secret_file.is_some() {
            anyhow::bail!("--auth-secret-file is only valid with --headless");
        }
        if !input.allowed_hosts.is_empty() {
            anyhow::bail!("--allowed-host is only valid with --headless");
        }
    }

    // 2. Desktop must bind to loopback only
    if !input.headless && !is_loopback(&input.host) {
        anyhow::bail!(
            "Desktop mode only supports loopback binding. \
             Use --headless for remote exposure with --allowed-host."
        );
    }

    // 3. Headless requires a secret
    if input.headless {
        let has_file = input.auth_secret_file.is_some();
        if !input.has_env_secret && !has_file {
            anyhow::bail!(
                "headless mode requires KOHARU_AUTH_SECRET or --auth-secret-file"
            );
        }
        if input.has_env_secret && has_file {
            anyhow::bail!(
                "KOHARU_AUTH_SECRET and --auth-secret-file are mutually exclusive"
            );
        }

        // 4. Headless remote exposure requires explicit allowed hosts
        if !is_loopback(&input.host) && input.allowed_hosts.is_empty() {
            anyhow::bail!(
                "Non-loopback headless binding requires --allowed-host. \
                 Specify at least one remote host that is allowed to connect."
            );
        }
    }

    Ok(())
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "localhost")
}
```

Then integrate into `run()`: call `validate_pre_bind` BEFORE `TcpListener::bind()`. Construct `PreBindInput` from the parsed CLI args and env:

```rust
// In run(), BEFORE TcpListener::bind():
validate_pre_bind(&PreBindInput {
    headless: cli.headless,
    host: bind_host.to_string(),
    allowed_hosts: cli.allowed_host.clone(),
    auth_secret_file: cli.auth_secret_file.clone(),
    has_env_secret: std::env::var("KOHARU_AUTH_SECRET").is_ok(),
})?;
```

Also restructure the headless block to call `HeadlessSecurityOptions::resolve()` BEFORE bind (the `validate_pre_bind` call already covers secret format check at a high level; `resolve()` does the detailed byte-level decode):

```rust
// Move headless secret resolution BEFORE listener bind
let headless_security = if cli.headless {
    Some(crate::security::HeadlessSecurityOptions {
        secret_from_env: std::env::var("KOHARU_AUTH_SECRET").ok(),
        secret_file: cli.auth_secret_file.clone(),
        allowed_hosts: cli.allowed_host.clone(),
    }.resolve()?)
} else {
    None
};

// THEN bind the listener
let listener = ...;

// THEN use the pre-resolved security
if let Some(headless) = headless_security {
    // spawn server with headless.security, headless.session, headless.remote_policy
}
```

### GREEN verification:

```bash
bun cargo test -p koharu pre_bind -- --nocapture
# Expected: 8 passed; 0 failed
```

## Task 3: regression and build

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
bun cargo test -p koharu
bun cargo test -p koharu-rpc
bun cargo check -p koharu -p koharu-rpc --all-targets
bun cargo clippy -p koharu -p koharu-rpc --all-targets -- -D warnings
bun cargo fmt --all -- --check
git diff --check -- crates/koharu/src/app.rs crates/koharu/src/security.rs
```

Expected: all tests pass, all checks pass. The `koharu` crate's existing `desktop_auth_command_uses_managed_one_time_proof` test must still pass.

## Task 4: independent implementation review

Dispatch one fresh read-only `code-reviewer` over:
- The `app.rs` diff (and `security.rs` if touched)
- RED/GREEN/Task 3 output
- Only the nine frozen acceptance items above

Any HIGH/CRITICAL blocks the commit. Fix inside the frozen file boundary, rerun Task 3, re-review.

## Task 5: one conventional commit

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
git add crates/koharu/src/app.rs
# Only if security.rs was changed:
# git add crates/koharu/src/security.rs
git diff --cached --check
git commit -m "fix(app): validate headless secrets and Desktop --host before bind" \
  -m "Co-Authored-By: Codex <noreply@openai.com>"
git rev-parse HEAD
```

After the commit, R06 is CLOSED. R07 may then be drafted.

## Risks and mitigations

- **HEAD reset**: The current `app.rs` HEAD matches R05 commit `47b14322`. Any drift blocks RED.
- **Port binding testability**: The `validate_pre_bind` function is pure (no I/O), so unit tests run instantly without binding real ports. The integration test (actual `run()`) is exercised by the existing `desktop_auth_command_uses_managed_one_time_proof` test.
- **is_loopback completeness**: Only `127.0.0.1`, `::1`, and `localhost` are recognized. `0.0.0.0` is correctly NOT recognized as loopback. DNS names that resolve to loopback (e.g. hostname → 127.0.0.1) are NOT covered — this is intentional; structural validation at the bind string level is sufficient for R06.
- **Scope creep**: No dependency, capability, RPC, UI, or generated file change. R07 is separate.
