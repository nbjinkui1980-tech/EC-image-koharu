# G005 Runtime Geometry Execution Card

> Status: ready for ordered implementation after consensus review.
>
> Authority: this card is subordinate to
> `.omx/plans/2026-08-05-hanonly-functional-delivery-plan.md`. It is not a new
> execution authority and cannot change `.omx/ultragoal/goals.json`.

## Scope And Baseline

- G004 is complete and G005 is already in progress. This card defines only the
  remaining G005 implementation.
- Implementation starts from `49d09067d0b05de06b1846da6e3b720239e2489a`.
- `4356c137512918da267e3e990da458b1dd2ad275` remains the accepted G004 behavior
  baseline. No reset, detached B0, artifact, calibration, custody, or formal
  holdout is required.
- R46-R60 custody, receipt, marker, authorization, and protocol material is
  historical and cannot authorize or block this card.
- The closed Typography authority card completed by `fa3d4290` remains closed.
- Source Gate, PP-OCR scale, detector ownership, protected Latin, incomplete
  coverage rejection, manual sizing, and AllText behavior are regression
  invariants, not redesign targets.

## Completion Matrix

| Product target | Current state | Completion evidence |
|---|---|---|
| Private resolved-layout geometry | PARTIAL | G005 dynamic-layout test is default-active and passes |
| Backend-owned automatic sizing | PARTIAL | Backend owns automatic size; UI contains no HanOnly estimator |
| Explicit unsupported rotation | PARTIAL | Warning/status test is default-active and passes |
| Manual sizing and AllText | COMPLETE invariant | Focused regressions remain green |
| UI Auto with empty numeric value | PARTIAL | Auto input stays empty and does not display or synthesize a size |
| PNG/JPEG/WebP allowlist parity | NOT_IMPLEMENTED | UI and all standard backend image entry points admit the same formats |
| Source Gate/OCR/protected Latin | COMPLETE invariant | Focused regressions remain green |

## Work Packages

### G005-WP1: Dynamic Layout

Production file: `crates/koharu-app/src/renderer.rs`.

- Complete the existing private resolved-layout path; do not add Scene or
  OpenAPI fields.
- A trustworthy shared bubble may allocate deterministic non-overlapping space
  beyond each source anchor while retaining the complete anchor.
- Missing, empty, unsafe, or non-containing bubble evidence stays within the
  source anchor and fails closed where no valid rectangle exists.
- Results must be input-order independent and repeatable.
- The existing default test
  `shared_bubble_keeps_seed_boxes_to_avoid_overlap` encodes the superseded
  seed-box-only behavior. Rename or replace it in the same test module so it
  asserts the new contract: each resolved box contains its source anchor,
  different owners do not overlap, and reversing input order produces the same
  owner-keyed geometry. It must not continue asserting exact seed boxes.
- Remove `#[ignore = "hanonly-pre-b1-red"]` only from
  `hanonly_pre_b1_red_t2_dynamic_layout_contract`, then make that exact test
  pass without weakening its assertions.

Completion: the exact staged test is default-active and green, plus existing
renderer layout regressions pass.

### G005-WP2: One Layout Handoff And Rotation Outcome

Production files:

- `crates/koharu-app/src/pipeline/engines/renderer.rs`
- `crates/koharu-app/src/pipeline/mod.rs`

- Resolver, fit, and post-validation must consume the same private resolved
  layout record and box; do not rebuild geometry at the renderer boundary.
- Unsupported rotation preserves the source ROI, produces no partial rendered
  block or sprite, emits one stable `han_only.unsupported_rotation:` warning per
  unsupported node, and permits supported same-page targets to continue.
- The public job result remains `CompletedWithErrors` when warnings occur.
- Remove the ignore attributes only from
  `hanonly_pre_b1_red_t2_pipeline_layout_handoff_contract` and
  `hanonly_pre_b1_red_t2_rotation_status_contract`, then make both pass without
  weakening their assertions.

Completion: both exact tests are default-active and green; focused HanOnly
pipeline and renderer tests remain green.

### G005-WP3: UI Automatic Non-Authority

Production file: `ui/components/panels/RenderControlsPanel.tsx`.

Test file: `ui/tests/components/RenderControlsPanel.test.tsx`.

- Delete the HanOnly automatic estimator chain rooted at
  `eligibleSourceLayout`, `automaticSourceSize`, and
  `groupedAutomaticSourceSizes`, including its `-5px`, `12..28`, `72px`, source
  box, and grouping calculations.
- Automatic mode keeps the numeric input empty with placeholder `auto` and does
  not display `auto N px` or submit a guessed size.
- Decrement/increment controls are disabled while no explicit manual size
  exists; entering a valid numeric value enters the existing manual path.
- Manual sizing and AllText behavior remain unchanged.

Completion: component tests prove Auto/empty state, no inferred numeric hint or
adjustment, manual transition, and unchanged manual/AllText behavior.

### G005-WP4: Format Allowlist Parity Only

Production files:

- `crates/koharu-app/src/blobs.rs`
- `crates/koharu-rpc/src/routes/pages.rs`
- `ui/lib/io/openFiles.ts` only if verification finds actual UI drift

Test files:

- Existing Rust test modules beside the changed production boundaries
- `ui/tests/lib/io/openFiles.test.ts` only if no existing UI test can prove the
  picker contract

- Reuse one shared Rust byte-sniffed admission boundary for standard images.
- Multipart import, path import, and Blob standard-image decode accept PNG,
  JPEG, and WebP and reject GIF, BMP, unknown, and corrupt formats.
- Filename extensions do not override the decoded byte format.
- Every requested multipart byte payload or path must pass the shared format
  admission preflight before `replace=true` can clear pages or mutate Scene,
  History, epoch, ordering, or Blob references. A rejected GIF, BMP, unknown,
  corrupt, or unreadable input leaves all of those values unchanged.
- Add focused multipart and path-import `replace=true` rejection tests that
  start with an existing page and prove page content/order, epoch/history, and
  referenced blobs are unchanged. This is a narrow no-mutation-before-format-
  admission requirement, not the complete G006 atomic multi-file replacement
  contract.
- Do not implement EXIF orientation normalization, decoded-byte budgets,
  byte-weighted cache eviction, atomic replacement, persistence permits, or
  durability in G005.
- Keep `hanonly_pre_b1_red_t2_blob_decode_budget_contract` and
  `hanonly_pre_b1_red_t2_replace_import_atomicity_contract` ignored for G006.
  G005 does not claim either complete contract; its allowlist work may
  incidentally satisfy the format assertions inside the broader G006 tests.

Completion: focused frontend/backend allowlist tests pass without enabling the
two G006 staged contracts or claiming their complete decoder/persistence scope.

## Stage Ownership

G005 turns exactly these three T2 tests default-active and green:

1. `hanonly_pre_b1_red_t2_dynamic_layout_contract`
2. `hanonly_pre_b1_red_t2_pipeline_layout_handoff_contract`
3. `hanonly_pre_b1_red_t2_rotation_status_contract`

G006 retains these two ignored T2 tests:

1. `hanonly_pre_b1_red_t2_blob_decode_budget_contract`
2. `hanonly_pre_b1_red_t2_replace_import_atomicity_contract`

G007 retains all nine `hanonly-pre-greenc-red` T3 tests. The two already active
G004 Source Gate and PP-OCR T2 tests remain active and green.

## Verification

Each work package runs only its exact focused tests while being implemented.
After all four packages pass, run the completion suite once:

1. The three G005 T2 tests and existing renderer/pipeline regressions.
2. Source Gate and PP-OCR regressions.
3. RenderControlsPanel and format-allowlist tests.
4. Manual sizing and AllText regressions.
5. `bun cargo test --workspace --tests`.
6. `bun cargo check --workspace --all-targets`.
7. `bun run test:ui` and `bun run lint:ui`.
8. `bun run format:check` and `bun run check:generated`.
9. The fixed CPU and actual-Metal affected-path smoke below.
10. The fixed end-to-end visual acceptance below.

### Fixed CPU, Metal, And Visual Acceptance

Verification-only production CLI file:
`crates/koharu-app/bin/pipeline.rs`.

- Add one startup diagnostic at the existing compute-policy selection boundary.
  It must report the requested policy and the actual device returned by
  `koharu_ml::device(cli.cpu)` using stable `cpu` or `metal` values.
- A non-CPU run on this macOS acceptance path must fail before model loading if
  the actual device is not Metal. Do not add a general device inventory,
  receipt, schema, or public API.

Use the tracked input
`test-image/O1CN01LriAra2AloJPVFqEZ_!!2216907268244.webp`, whose required
SHA-256 is
`d5bf9f87a4766e61047ed1c317f96f2d4e9388f974f6d5e1c7c60b19b31da885`.
Run from the reviewed implementation commit and write outside the repository:

```sh
set -u
EXPECTED_INPUT_SHA=d5bf9f87a4766e61047ed1c317f96f2d4e9388f974f6d5e1c7c60b19b31da885
INPUT=test-image/O1CN01LriAra2AloJPVFqEZ_\!\!2216907268244.webp
test "$(shasum -a 256 "$INPUT" | awk '{print $1}')" = "$EXPECTED_INPUT_SHA" || exit 1
RUN_ROOT="/Users/jinkui/ec-image-Koharu/hanonly-g005-smoke/$(git rev-parse HEAD)/$(date -u +%Y%m%dT%H%M%SZ)-$$"
test ! -e "$RUN_ROOT" || exit 1
mkdir -p "$RUN_ROOT/cpu" "$RUN_ROOT/metal" || exit 1
bun cargo run -p koharu-app --bin pipeline -- \
  --input "$INPUT" --output-dir "$RUN_ROOT/cpu" --cpu \
  >"$RUN_ROOT/cpu/stdout.log" 2>"$RUN_ROOT/cpu/stderr.log"
CPU_STATUS=$?
printf '%s\n' "$CPU_STATUS" >"$RUN_ROOT/cpu/exit-status"
bun cargo run --features metal -p koharu-app --bin pipeline -- \
  --input "$INPUT" --output-dir "$RUN_ROOT/metal" \
  >"$RUN_ROOT/metal/stdout.log" 2>"$RUN_ROOT/metal/stderr.log"
METAL_STATUS=$?
printf '%s\n' "$METAL_STATUS" >"$RUN_ROOT/metal/exit-status"
test "$CPU_STATUS" -eq 0 && test "$METAL_STATUS" -eq 0
```

Before accepting either run, verify the input hash, record the implementation
commit and model inventory used by the production CLI, and require exit status
zero plus `=> pipeline succeeded`. The startup diagnostic must report
`requested_compute=cpu actual_compute=cpu` for CPU and
`requested_compute=metal actual_compute=metal` for Metal. `PreferGpu`, feature
presence, or absence of a fallback warning is not proof of actual Metal.

Both output directories must contain decodable, non-empty `source.png`,
`inpainted.png`, `rendered.png`, and parseable `scene.json`; all three images
must retain the source dimensions. Inspect the CPU and Metal outputs side by
side and accept only when the selected Han text is erased and rendered without
clipping or owner overlap, the three protected Latin labels remain unchanged,
no unsupported target is partially rendered, and CPU/Metal resolve the same
target count, text, translation, warning set, source transform, line polygons,
and sprite transform after sorting text nodes by source text plus source
transform and ignoring random IDs. Private owner-keyed resolved-box equivalence
is proved by the default-active G005 dynamic-layout/handoff tests, not inferred
from `scene.json`. Pixel identity between CPU and Metal is not required. Record
the unique run root, exit statuses, model inventory, and SHA-256 values for the
two logs, rendered images, and scene files in the G005 completion report; do not
create a new receipt, schema, or governance artifact.

Do not repeat workspace, Metal, or end-to-end validation after every small
edit. Stop on a reproducible product regression, data-loss risk, or a change
that requires G006 persistence work.

## RALPLAN-DR

### Principles

1. One backend authority for automatic geometry and size.
2. One private resolved-layout record across resolver, fit, and validation.
3. Preserve fail-closed source-selection and protected-content behavior.
4. Keep G005 rendering separate from G006 persistence reliability.

### Decision Drivers

1. Remove current duplicated UI/backend behavior.
2. Turn only the tests owned by G005 green.
3. Avoid retired governance and unnecessary cross-goal implementation.

### Options

- **Selected: four bounded work packages.** Small production ownership sets,
  focused checks per package, and one final suite minimize regression scope.
- **Rejected: restore the archived monolithic B1 plan.** It mixes G005-G007,
  retired custody, persistence, remote governance, and obsolete acceptance.
- **Rejected: enable all five remaining T2 tests at once.** It prematurely
  pulls decoder budgets and atomic persistence into G005.

## ADR

### Decision

Execute G005 through the four work packages above and keep G006/G007 staged
contracts untouched.

### Consequences

- G005 may change rendering, pipeline handoff, UI automatic presentation, and
  format admission only.
- G006 remains responsible for bounded decoding, cache accounting, atomic
  replacement, and durable persistence.
- No new external Scene, OpenAPI, or protocol API, dependency, or evidence
  system is introduced. A minimal workspace-visible Rust helper is allowed only
  when needed to keep the three backend image entry points on one admission
  rule.

### Follow-Ups

- After consensus, `$ultragoal` is the default durable implementation lane.
- `$team` is optional only if UI and Rust work are run as disjoint lanes under
  the same Ultragoal checkpoint; the leader runs the combined verification.
- `$ralph` is an explicit fallback for one-owner sequential implementation.

## Available Agent Types And Staffing

- `executor` (medium): implement one bounded package at a time.
- `test-engineer` (medium): challenge staged-test ownership and regression
  coverage without editing production code.
- `code-reviewer` (high): review the integrated G005 diff.
- `architect` (xhigh): verify backend authority and G005/G006 boundaries.
- `verifier` (high): run and validate the final combined suite.

Recommended team verification: disjoint Rust/UI executors report to one leader;
the leader integrates, then `code-reviewer` and `verifier` inspect the same
commit before G005 completion is considered.

## Consensus Handoff

- Planning artifact: this Execution Card.
- Architect and Critic review the same final document SHA in that order.
- Their agent IDs, decisions, reviewed SHA, and
  `Ralplan-Consensus-Gate: complete` are recorded in the final Git commit
  message, outside the reviewed bytes, only after both approve.
- Execution is not authorized until that ordered consensus commit exists.
- Because `.omx/` is excluded locally, the final commit must force-add exactly
  this card and the archived historical specification, stage the tracked active
  plan normally, and no other path. After commit, verify each reviewed byte set
  with `git show <commit>:<path> | shasum -a 256`; all three values must equal
  the hashes reviewed by Architect and Critic.
