# Repository Guidelines

## Project Structure & Module Organization

- `crates/` is the Rust 2024 workspace; each `koharu-*` crate owns one subsystem. Key boundaries are `koharu-core`, `koharu-app`, and `koharu-rpc`.
- `ui/` contains the Next.js 16/React 19 frontend: routes in `app/`, components in `components/`, hooks in `hooks/`, and API/state helpers in `lib/`.
- `tests/integration-tests/` exercises the backend. Crate tests live beside source or in crate-level `tests/`; Vitest files live under `ui/tests/`.
- Localized docs live in `docs/<locale>/`; utilities and generators live in `scripts/`. Do not commit `target/`, `ui/.next/`, or `ui/out/`.

## Build, Test, and Development Commands

- `bun install` installs workspace dependencies from `bun.lock`.
- `bun run dev` launches the Tauri desktop app; `bun run build` creates the release binary.
- Use `bun cargo ...` instead of raw Cargo locally so platform features are configured correctly.
- `bun cargo check --workspace --all-targets` checks Rust; `bun cargo test --workspace --tests` runs tests.
- `bun run test:ui`, `bun run lint:ui`, and `bun run format:check` validate the frontend.
- `bun run check:generated` regenerates OpenAPI/Orval output and rejects drift.

## File-Scoped Commands

| Task         | Command                                                  |
| ------------ | -------------------------------------------------------- |
| Rust test    | `bun cargo test -p koharu-app test_name`                 |
| UI test      | `bun run --cwd ui test -- tests/components/Foo.test.tsx` |
| Format files | `bunx oxfmt path/to/file.ts`                             |

## Coding Style & Naming Conventions

- Follow `rustfmt`, Clippy, Oxfmt, and Oxlint; do not restate or bypass their configuration.
- Rust uses `snake_case` modules/functions and `UpperCamelCase` types. React components use `PascalCase.tsx`; hooks use `useThing.ts`; tests use `.test.ts` or `.test.tsx`.
- Do not edit `ui/lib/api/generated.ts` or generated schemas manually. Change the OpenAPI source or Orval config, then regenerate.

## Karpathy Coding Contract

- Apply these rules to every implementation, bug fix, review, and refactor, even when the `karpathy-guidelines` skill is unavailable.
- Before editing, state material assumptions, ambiguities, the simplest viable approach, and verifiable success criteria. Ask only when ambiguity would change behavior, scope, or risk.
- Implement only requested behavior. Reuse existing code; do not add speculative features, single-use abstractions, or unnecessary configurability.
- Make surgical changes: every changed line must trace to the request. Do not refactor adjacent code or remove pre-existing dead code; remove only orphans created by the current change.
- For nontrivial work, use a short goal-to-verification plan. Reproduce bugs with a focused regression test, then loop until the targeted checks pass.
- If the implementation grows beyond the simplest adequate solution, stop and simplify it before continuing.

## Testing Guidelines

- Add focused regression tests for behavior changes. Prefer deterministic MSW handlers and fixtures registered by each test.
- Run checks matching the touched area; run `bun run build` for desktop integration changes.
- Model, keyring, and GPU tests may need platform services; document skipped baselines.

## Commit & Pull Request Guidelines

- Use conventional prefixes seen in history: `feat:`, `fix:`, `refactor:`, `ci:`, and `chore(deps):`.
- Keep one goal per PR. Include a summary, validation commands, relevant issues, and screenshots or clips for visible UI changes.
- Do not add compatibility shims, drive-by refactors, or handwritten changelog entries.

## Commit Attribution

- AI-assisted commits must use the active agent's truthful identity: `Co-Authored-By: <agent name> <agent email>`.
