# AR01 R05 UI Authentication Gate Execution Card

**Status:** CLOSED — implemented and verified by `47b14322f407e01517ba7ca9839963d2112fd7c9`; retained as the historical execution card.

> **For agentic workers:** Execute once in strict order: RED → minimal GREEN → regression+build → independent review → one commit. This is R05 only — R06, dependency upgrades, generated files, or backend refactors are out of scope.

**Goal:** Add regression tests proving that Desktop bootstrap failure propagates correctly to AuthBootstrap, children/SSE/Updater are gated on authentication success, and headless retry after failure works. Fix any product-code bug discovered during test development — the existing gate is mostly correct but needs verified coverage.

**Architecture:** Expand existing test files with Vitest + `@testing-library/react`. Mock `@tauri-apps/api/core`, `@/lib/events`, and `fetch` at the boundary. Product-code changes are ONLY permitted when a test uncovers a genuine fail-closed gap in the gate; no speculative "improvement" edits.

**Tech Stack:** React 19, Vitest, testing-library, TypeScript. No new dependencies.

**Baseline:** The current `HEAD` of `codex/audit-remediation-sdd`. R04 commit `6958c1e8` is the feature baseline.

## Frozen file boundary

- Test (create/expand): `ui/tests/components/AuthBootstrap.test.tsx`
- Test (expand): `ui/tests/lib/auth.test.ts`
- Product (diagnostic only — edit only if a test-proven bug exists): `ui/components/AuthBootstrap.tsx`
- Product (diagnostic only): `ui/lib/auth.ts`

No other file may enter the implementation diff or commit. The existing `providers.test.tsx` is adequate; do not expand it.

## Frozen acceptance

1. Desktop bootstrap rejection (Tauri invoke fails) → AuthBootstrap shows `restart-required`, `connectEvents` never called, children never rendered.
2. Desktop bootstrap rejection (exchangeSession fails after invoke succeeds) → AuthBootstrap shows `restart-required`, `authenticated` stays `false`.
3. Desktop bootstrap success → children render, `connectEvents` called exactly once.
4. Headless exchange rejection → error message visible, stays at token form, children not rendered.
5. Headless retry with valid token after a failed attempt → `authenticated`, children render.
6. Already-authenticated on mount → children render immediately, no bootstrap call, SSE connects.
7. SSE 401 on Desktop → `auth-required` listener fires → state becomes `restart-required` (existing test covers this).
8. API 401 on headless → `auth-required` listener fires → state returns to token form (existing test covers this).

## Task 1: auth.test.ts — expand desktop bootstrap rejection coverage

Add these tests to `ui/tests/lib/auth.test.ts`:

### Test A: `stays unauthenticated and rejects when the desktop invoke call fails`

```typescript
it('stays unauthenticated and rejects when the desktop invoke call fails', async () => {
  const invoke = vi.fn().mockRejectedValue(new Error('IPC failed'))
  vi.doMock('@tauri-apps/api/core', () => ({ invoke }))

  await expect(bootstrapDesktopSession()).rejects.toThrow('IPC failed')
  expect(isAuthenticated()).toBe(false)
  expect(invoke).toHaveBeenCalledTimes(1)
})
```

### Test B: `stays unauthenticated and rejects when the session exchange fails after a successful invoke`

```typescript
it('stays unauthenticated and rejects when the session exchange fails after a successful invoke', async () => {
  const invoke = vi.fn().mockResolvedValue('desktop-proof')
  vi.doMock('@tauri-apps/api/core', () => ({ invoke }))
  vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 500 }))

  await expect(bootstrapDesktopSession()).rejects.toThrow('auth exchange failed: 500')
  expect(isAuthenticated()).toBe(false)
  expect(invoke).toHaveBeenCalledTimes(1)
})
```

### Test C (already exists — keep): `requests the desktop bootstrap proof at most once per page lifetime`

Do not modify. It already verifies:
- First call succeeds
- Second call rejects with `desktop restart required`
- Invoke called exactly once

## Task 2: AuthBootstrap.test.tsx — expand component gate coverage

Add these tests to `ui/tests/components/AuthBootstrap.test.tsx`:

### Test A: `shows restart-required without mounting children or SSE when desktop bootstrap rejects`

```typescript
it('shows restart-required without mounting children or SSE when desktop bootstrap rejects', async () => {
  mocks.bootstrapDesktopSession.mockRejectedValue(new Error('IPC failed'))

  render(<AuthBootstrap>ready</AuthBootstrap>)

  await screen.findByRole('alert')
  expect(screen.getByText('Authentication expired. Restart Koharu.')).toBeInTheDocument()
  expect(screen.queryByText('ready')).not.toBeInTheDocument()
  expect(mocks.connectEvents).not.toHaveBeenCalled()
})
```

### Test B: `keeps headless client at token form with error after a rejected exchange, then authenticates on retry`

```typescript
it('keeps headless client at token form with error after a rejected exchange, then authenticates on retry', async () => {
  mocks.desktop = false
  mocks.exchangeSession.mockRejectedValueOnce(new Error('Bad token'))
  render(<AuthBootstrap>ready</AuthBootstrap>)

  const input = screen.getByPlaceholderText('Enter authentication token')
  fireEvent.change(input, { target: { value: 'bad' } })
  fireEvent.submit(input.closest('form')!)

  await waitFor(() =>
    expect(screen.getByText('Authentication failed')).toBeInTheDocument(),
  )
  expect(screen.queryByText('ready')).not.toBeInTheDocument()

  mocks.exchangeSession.mockResolvedValueOnce()
  fireEvent.change(input, { target: { value: 'good' } })
  fireEvent.submit(input.closest('form')!)

  await screen.findByText('ready')
  expect(mocks.exchangeSession).toHaveBeenCalledTimes(2)
  expect(mocks.connectEvents).toHaveBeenCalledTimes(1)
})
```

### Test C (keep existing): `shows restart-required without requesting a second desktop proof`
### Test D (keep existing): `mounts neither children nor SSE before desktop authentication succeeds`
### Test E (keep existing): `returns headless clients to token entry after a runtime 401`
### Test F (keep existing): `keeps an authenticated client mounted without bootstrapping again`

## Product-code guard

The existing `AuthBootstrap.tsx` and `auth.ts` implementations correctly enforce items 1-8 before any edit. Run the expanded test suite first as RED (some tests fail before any product changes — verify existing tests all pass, verify new tests selectively fail for the right reasons).

If ALL tests pass (both new and existing), no product-code change is needed — the gate is already correct. R05 then delivers the test layer only.

If a new test reveals a genuine fail-closed gap (e.g., `restart-required` not shown when it should be, or children rendered during `pending`), fix ONLY that gap with the smallest possible change. Do not refactor adjacent code.

## Task 3: RED-GREEN cycle

### Step 1: Run existing tests to confirm baseline

```bash
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
bun run --cwd ui test -- tests/components/AuthBootstrap.test.tsx --run
bun run --cwd ui test -- tests/lib/auth.test.ts --run
bun run --cwd ui test -- tests/app/providers.test.tsx --run
```

Expected: all existing tests pass. Any pre-existing failure in another test file is out of scope.

### Step 2: Add new tests (RED)

Add the new test cases from Tasks 1 and 2. Do NOT change product code yet.

```bash
bun run --cwd ui test -- tests/lib/auth.test.ts --run
```

Expected for auth.test.ts `stays unauthenticated and rejects when the desktop invoke call fails`: the `vi.doMock` for `@tauri-apps/api/core` may require adjusting — this test uses `vi.doMock` which applies at module-import time. If the test fails for compilation/mock reasons, fix the mock setup, not the product code.

### Step 3: Verify new AuthBootstrap tests pass or fail correctly

```bash
bun run --cwd ui test -- tests/components/AuthBootstrap.test.tsx --run
```

Expected: the existing 4 tests pass. The new test A "shows restart-required..." SHOULD pass immediately (the product code already handles this). The new test B "keeps headless client..." SHOULD pass immediately. If either fails, it indicates the product code needs a fix — proceed to Task 4.

### Step 4: Minimal GREEN (only if a test-proven bug exists)

If any new test fails for a reason that traces to a product-code gap:

1. Document the gap in the evidence ledger
2. Make the minimal fix in `AuthBootstrap.tsx` or `auth.ts`
3. Re-run ALL tests — new + existing
4. Do not change any other file

If all new tests pass without product-code changes, skip this step.

## Task 4: Complete UI regression and build

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay

# Auth gate tests
bun run --cwd ui test -- tests/components/AuthBootstrap.test.tsx --run
bun run --cwd ui test -- tests/lib/auth.test.ts --run
bun run --cwd ui test -- tests/app/providers.test.tsx --run

# Full regression
bun run test:ui -- --run

# Lint and format
bun run lint:ui
bun run format:check

# Build
bun run build

# Verify only test files changed (unless a product bug was fixed)
git diff --check -- ui/tests/components/AuthBootstrap.test.tsx ui/tests/lib/auth.test.ts ui/components/AuthBootstrap.tsx ui/lib/auth.ts
```

Expected: every command exits 0. The diff must NOT include any generated file, dependency manifest, or file outside the frozen boundary.

## Task 5: Independent implementation review

Dispatch one fresh read-only `code-reviewer` over:
- The test-only diff (or test + minimal product fix diff)
- RED/GREEN/Task 4 output
- Only the eight frozen acceptance items above

The reviewer may report implementation defects against those frozen items but may not add new requirements. Any HIGH/CRITICAL blocks the commit. Fix inside the frozen file boundary, rerun Task 4, re-review.

## Task 6: One conventional commit

```bash
set -euo pipefail
cd /Users/jinkui/ec-image-Koharu/EC-image-koharu-ar01-replay
git add ui/tests/components/AuthBootstrap.test.tsx ui/tests/lib/auth.test.ts
# Only if product code was changed:
# git add ui/components/AuthBootstrap.tsx ui/lib/auth.ts
git diff --cached --check
git commit -m "test(ui): cover auth gate bootstrap failure and retry paths" \
  -m "Co-Authored-By: Codex <noreply@openai.com>"
git rev-parse HEAD
```

After the commit, R05 is CLOSED. Proceed to R06 only after the commit is verified.

## Risks and mitigations

- **vi.doMock timing:** `vi.doMock` for `@tauri-apps/api/core` must be called before the module under test is imported. The auth.test.ts needs careful import ordering.
- **Test isolation:** Each test resets mocks in `beforeEach`/`afterEach`. Module-level state (`authenticated`, `desktopProofRequested`) persists across tests — rely on `notifyAuthenticationRequired()` in `afterEach` to reset.
- **Scope creep:** No backend, Tauri, dependency, or generated-file change. R06 is separate.
