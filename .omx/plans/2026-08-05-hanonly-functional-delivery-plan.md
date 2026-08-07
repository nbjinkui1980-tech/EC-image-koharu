# HanOnly Functional Delivery Plan

This is the sole active execution plan for G005-G009. G004 is complete. Revision 46-60 custody, holdout, authorization, and protocol documents are historical evidence, not delivery gates.

## Product Invariants

- Source Gate selects only safely proven Han or Han-mixed source text.
- Pure Latin protected regions remain unchanged.
- Ambiguous ownership, unsupported rotation, and incomplete source coverage fail closed.
- Existing project/session data remains compatible unless a goal explicitly changes its schema.

## Delivery Order

1. **G005 - Visual rendering:** implement the approved rendering behavior with focused regression tests and CPU/Metal smoke coverage.
2. **G006 - Integration reliability:** verify current supported platforms with integration, race, and fault tests. Windows is required only when it is an active release target or suitable CI is available.
3. **G007 - Planner source style and frozen sprite:** implement strict source style classification, run-local Planner contracts, immutable frozen sprites, exact post-inpaint Renderer composition, and turn the nine staged T3 tests green.
4. **G008 - Release verification:** run focused tests, workspace checks, CPU and actual-Metal smoke tests, and end-to-end visual acceptance.
5. **G009 - Final acceptance:** complete final code review, builds, and functional acceptance.

## Required Checks

- Focused regression tests for each changed behavior.
- `bun cargo test --workspace --tests`
- `bun cargo check --workspace --all-targets`
- `bun run test:ui`
- `bun run lint:ui`
- `bun run format:check`
- `bun run check:generated`
- CPU and actual-Metal smoke checks for affected image paths.
- End-to-end visual acceptance before G008/G009 completion.

## Stop Rules

- Stop on a reproducible product regression, data-loss risk, or unsupported platform dependency.
- Historical custody commands cannot authorize, mutate, or block functional delivery.
- Do not create new custody Revisions or formal holdout markers.
