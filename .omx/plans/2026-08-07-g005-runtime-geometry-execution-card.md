# G005 Runtime Geometry Execution Card

> Status: draft under renewed consensus review.
>
> Authority: this card is subordinate to
> `.omx/plans/2026-08-05-hanonly-functional-delivery-plan.md`. It is a temporary
> Execution Card, not a second execution authority, and cannot change goals,
> reopen G004, or restore historical custody requirements.

## Scope And Baseline

- G004 is complete; G005 implementation has not started. This card defines
  only the G005 product work and its acceptance checks.
- `49d09067d0b05de06b1846da6e3b720239e2489a` is the product-code parent
  baseline. `d4b55f5f8dc091ad11a8c86df667f5783fae72dc` is the pre-repair planning
  consensus commit.
- Implementation may begin only from the later scoped consensus commit that
  contains these exact repaired card bytes and descends from `d4b55f5f...`.
  Do not reset, clean, or discard unrelated work to reach either baseline.
- `scripts/dev.ts` is unrelated user work and must not be modified, staged, or
  committed by this plan.
- R46-R60 custody, receipt, marker, authorization, and protocol material is
  historical. It cannot authorize or block G005.
- The closed Typography authority card remains closed. Source Gate, PP-OCR,
  protected Latin, incomplete-coverage rejection, manual sizing, AllText, and
  historical project readability are regression invariants.

## Product Contract

1. HanOnly automatic supported-rotation targets use one backend-owned private
   resolved-layout record from allocation through rasterization and
   post-validation.
2. The record preserves the source anchor and may expand within trustworthy
   bubble/page space without colliding with another owner or protected content.
3. Manual sizing, AllText, unrecognized-language input, and unsupported
   rotation keep their existing layout behavior.
4. Unsupported HanOnly rotation preserves the source ROI, renders no partial
   target, emits one stable page/pipeline-step warning, and does not block
   supported same-page targets.
5. New source-image ingress accepts byte-sniffed PNG, JPEG, and WebP only.
   Legacy Blob readers remain able to open existing GIF/BMP projects.
6. G005 guarantees that empty or rejected ingress performs no mutation. G006
   retains successful replacement transaction, persistence, recovery,
   durability, decode-budget, race, and fault semantics.

## Completion Matrix

| Product target | Current state | G005 completion evidence |
|---|---|---|
| Private resolved geometry reaches raster and validation | PARTIAL | Dynamic-layout and pipeline-handoff T2 tests become active and pass with real expansion |
| Backend-owned automatic sizing | PARTIAL | Renderer consumes the private resolved record; UI has no HanOnly estimator |
| Unsupported rotation outcome | PARTIAL | Aggregated warning/status T2 test becomes active and passes |
| Manual sizing and AllText | COMPLETE invariant | Existing focused regressions remain green |
| UI Auto state | PARTIAL | HanOnly recognized automatic input stays empty; unrelated modes keep existing controls |
| New-ingress PNG/JPEG/WebP parity | NOT_IMPLEMENTED | RPC and CLI source ingress share one admission helper; legacy Blob compatibility passes |
| Source Gate/OCR/protected Latin | COMPLETE invariant | Existing focused regressions remain green |

## Work Packages

### G005-WP1: One Private Resolved Geometry

Primary production files:

- `crates/koharu-app/src/renderer.rs`
- `crates/koharu-app/src/pipeline/engines/renderer.rs`
- `crates/koharu-app/src/pipeline/mod.rs`

Requirements:

- Reuse the existing private layout structures and hooks. Add at most one
  crate-private resolved-layout record if no existing type can carry the
  required data; do not add Scene, OpenAPI, RPC, or persisted fields. Any new
  field added to an existing internal struct (e.g., `RenderBlockInput`) must be
  `pub(crate)` or passed through private function signatures; it must not become
  part of a public crate API.
- For HanOnly automatic targets with supported rotation, stop forcing
  `lock_layout_box=true` solely because the source language is recognized or
  the source has one/incomplete line. Preserve locking for manual sizing,
  AllText, unrecognized-language input, and unsupported rotation.
- Resolve owner-keyed geometry deterministically. Each resolved box contains
  its source anchor, remains inside the page/trustworthy bubble, and does not
  overlap another owner or protected Latin support.
- Pass the same resolved record through resolver, fit, line polygon/sprite
  construction, rasterization, and Han post-validation. No downstream stage may
  reconstruct the box from the source transform.
- Validate rendered alpha against the resolved box and page, not only the
  original source bbox. Retain owner, protected-content, bubble, collision, and
  page-bound checks.
- Missing, empty, unsafe, or non-containing bubble evidence keeps the source
  anchor and fails closed if no valid rectangle exists.
- Replace the superseded seed-box-only assertion in
  `shared_bubble_keeps_seed_boxes_to_avoid_overlap` with the new contract.
- Activate only `hanonly_pre_b1_red_t2_dynamic_layout_contract`. It must prove
  at least one resolved box differs from its source box, all anchors are
  contained, owners do not overlap, and reversed input order yields identical
  owner-keyed geometry.
- Activate only `hanonly_pre_b1_red_t2_pipeline_layout_handoff_contract`. It
  must run the production handoff and prove rendered alpha exists outside the
  source bbox but inside the resolved box, with zero protected/other-owner
  overlap and order-independent owner-keyed geometry.

Completion: both exact T2 tests are default-active and green; existing
renderer/pipeline, manual-size, AllText, Source Gate, and protected-Latin
regressions remain green.

### G005-WP2: Unsupported Rotation And Job Status

Primary production files:

- `crates/koharu-app/src/pipeline/engines/renderer.rs`
- `crates/koharu-app/src/pipeline/mod.rs`
- `crates/koharu-rpc/src/routes/pipelines.rs` only if the existing status path
  does not already propagate the warning outcome

Requirements:

- Unsupported nodes are sorted by a stable geometric owner key
  `(page index, source transform, source line polygon)`, retain their source ROI,
  and emit individual body-free diagnostic events. Do not use random `NodeId`
  values as comparison keys.
- Aggregate those diagnostics into exactly one stable
  `han_only.unsupported_rotation:` warning per page/pipeline step. The warning
  must not contain source or translated text.
- Unsupported targets produce no partial block, line, or sprite. Supported
  same-page targets continue through the normal renderer and must actually
  render.
- Existing RPC `JobSummary` and `JobFinished` results must be
  `CompletedWithErrors` when this warning exists. Change RPC production code
  only if a focused test proves the existing propagation is insufficient.
- Activate only `hanonly_pre_b1_red_t2_rotation_status_contract`. Update that
  staged test to run the real renderer step instead of only `aot-inpainting`,
  and remove its obsolete whole-Scene/epoch/file-tree no-change assertion.
  With two unsupported nodes and one supported node, assert instead that:
  - both unsupported source ROIs are unchanged and gain no sprite;
  - the supported node produces a nonempty sprite and rendered output;
  - exactly one sorted, body-free warning is emitted; and
  - the existing RPC path reports `CompletedWithErrors` in both `JobSummary`
    and `JobFinished`.

Completion: the exact T2 and focused RPC test are green without changing
supported rotation, Source Gate, or translation authority.

### G005-WP3: UI Automatic Non-Authority

Production file: `ui/components/panels/RenderControlsPanel.tsx`.

Test file: `ui/tests/components/RenderControlsPanel.test.tsx`.

Requirements:

- Delete the HanOnly automatic estimator chain rooted at
  `eligibleSourceLayout`, `automaticSourceSize`, and
  `groupedAutomaticSourceSizes`, including geometry and fallback arithmetic and
  any helpers (e.g., `isAutomaticSourceNode`, `sameSourceRow`) used only by that
  chain, confirmed by repository search. Do not replace it with another estimator.
- Only `HanOnly + recognized language + automatic/no manual size` hides any
  inferred `N px` hint and disables decrement/increment. The input remains
  empty with placeholder `auto`.
- AllText, unrecognized-language, and legacy paths retain their current
  persisted-or-empty display, enabled controls, and `16px` adjustment fallback
  when no persisted size exists. Explicit manual-size paths remain unchanged.
- Entering a valid numeric value uses the existing manual path.
- Remove `fontSizeAutoHint` locale keys only if repository search proves they
  become unused; no other copy changes.

Completion: component tests cover the exact mode matrix, manual transition,
and unchanged AllText/unrecognized behavior.

### G005-WP4: New Source-Image Admission

Primary production files:

- `crates/koharu-app/src/blobs.rs`
- `crates/koharu-rpc/src/routes/pages.rs`
- `crates/koharu-app/bin/pipeline.rs`
- `ui/lib/io/openFiles.ts` only if verification finds actual UI drift

Requirements:

- Add or reuse one minimal workspace-visible Rust source-image admission helper
  at the Blob boundary. It byte-sniffs and basic-decodes PNG, JPEG, and WebP;
  GIF, BMP, unknown, corrupt, extension-spoofed, and unreadable input fail.
  Workspace-visible here means shared by the library and CLI binary as
  `pub(crate)` or a crate-private item; it is not a new public `koharu-app` or
  workspace API, nor a new Scene/OpenAPI/protocol API.
- Use this helper only for new source-image ingress: multipart import, path
  import, and production CLI `import_page`.
- Do not narrow generic `decode_blob` or `BlobStore::load_image`; existing
  GIF/BMP project blobs must remain readable. Masks keep their existing
  PNG-specific boundary; AI-generated/internal RAW decode paths are unchanged.
- Admission order is fixed: reject an empty request; read every requested path
  or payload; sniff and basic-decode every item; only then permit any Scene,
  History, epoch, ordering, page, or Blob-reference mutation.
- A rejected or empty request leaves those values unchanged. After all inputs
  are admitted, the existing successful replace path may remain unchanged.
- Do not implement the G006 full replacement transaction, rollback,
  persistence permit, recovery, durability, decoded-byte budget, cache
  accounting, race, or fault behavior. If satisfying G005 requires such a
  change, stop and hand the issue to G006.
- Focused tests cover PNG/JPEG/WebP acceptance; GIF/BMP/unknown/corrupt and
  extension spoof rejection; empty/one-invalid replace no-mutation; CLI ingress;
  and legacy GIF/BMP Blob readability.
- Keep `hanonly_pre_b1_red_t2_blob_decode_budget_contract` and
  `hanonly_pre_b1_red_t2_replace_import_atomicity_contract` ignored for G006.

Test files:

- `crates/koharu-app/src/blobs.rs` (unit tests for the admission helper and
  legacy Blob readability).
- `crates/koharu-rpc/tests/pages_admission.rs` (multipart/path import
  acceptance/rejection and replace no-mutation).
- `crates/koharu-app/tests/pipeline_cli_admission.rs` (CLI `import_page`
  admission tests; `bin/pipeline.rs` remains a production touchpoint only).
- `ui/tests/lib/openFiles.test.ts` only if `ui/lib/io/openFiles.ts` is touched.

Completion: all focused ingress/compatibility tests pass and neither G006 T2
contract is enabled or claimed complete.

### G005-WP5: Hermetic CPU And Actual-Metal Smoke

Allowed production touchpoints are limited to:

- `crates/koharu-app/bin/pipeline.rs`
- `crates/koharu-runtime/src/runtime.rs`
- `crates/koharu-runtime/src/packages.rs`
- `crates/koharu-app/src/pipeline/engine.rs`
- the concrete wrappers used by the fixed steps under
  `crates/koharu-app/src/pipeline/engines/`
- device/introspection accessors, only where absent, in the corresponding
  `koharu-ml` model module and `crates/koharu-llm/src/paddleocr_vl.rs`
- only for PaddleOCR-VL llama instance allocation introspection:
  `crates/koharu-llm/build.rs`, `crates/koharu-llm/src/sys/mod.rs`,
  `crates/koharu-llm/src/safe/mod.rs`,
  `crates/koharu-llm/src/safe/model.rs`, and
  `crates/koharu-llm/src/safe/context.rs`

Do not touch unrelated engines. Changes to `build.rs` and `sys/safe` bindings
are limited to exposing existing live-instance allocation data already present
at compile time; no new build dependencies, no new public ABI symbols, no
lifetime or allocation-policy changes, and any new accessor must be
`pub(crate)` read-only. If a required observation cannot be exposed through
these existing runtime/model boundaries, stop instead of introducing a general
evidence framework.

Requirements:

- Add a CLI-only `--data-root` value and use the fixed existing model root
  `/Users/jinkui/Library/Application Support/Koharu` for both runs. This path is
  environment-local for the fixed macOS acceptance machine; the smoke is not
  portable across hosts. The CLI must not replace it with repository `.cache`
  during this acceptance path.
- Add a `koharu-runtime` cached-only preflight called before `prepare()`. It
  verifies the selected native runtime and all fixed-step model artifacts are
  already present. A missing artifact fails before constructing or invoking an
  HTTP download path. Normal application preparation remains unchanged.
- The CLI hashes cached artifacts after the cached-only preflight and before
  model construction. No acceptance run may download or repair a model.
- Reuse a small run-local diagnostic sink at the existing engine boundary.
  Concrete model wrappers emit actual-device data from their held Candle device
  after construction/run; PaddleOCR-VL emits existing llama backend/buffer data
  after inference. Inventory and instance-diagnostic lines are emitted to stderr
  only, are private to this acceptance smoke, and must not be documented or
  consumed as a stable public protocol, receipt, or persisted schema. Do not add
  a public getter to `Engine`, a receipt publisher, or a persisted schema.
- The permitted llama binding change is a narrow read-only interface on the
  same live PaddleOCR-VL model/context instance. It may expose actual
  model/context/compute buffer allocation and backend type after inference; it
  must not alter allocation policy, model lifetime, inference, or general
  logging behavior.

### Fixed Inputs

Tracked source image:

- path: `test-image/O1CN01LriAra2AloJPVFqEZ_!!2216907268244.webp`
- SHA-256: `d5bf9f87a4766e61047ed1c317f96f2d4e9388f974f6d5e1c7c60b19b31da885`

The run creates a unique external mode-0700 root and writes this exact TOML,
including its trailing newline, to `config.toml`:

```toml
[http]
connect_timeout = 20
read_timeout = 300
max_retries = 3

[pipeline]
source_text_policy = "han_only"
detector = "pp-doclayout-v3"
font_detector = "yuzumarker-font-detection"
segmenter = "comic-text-detector-seg"
bubble_segmenter = "speech-bubble-segmentation"
ocr = "paddle-ocr-vl-1.6"
translator = "llm"
typography_planner = "cloud-typography-planner"
inpainter = "lama-manga"
renderer = "koharu-renderer"

[typography_planner]
enabled = false
```

Required config SHA-256:
`b7be0b97709f6836edc5ec98b0fbaf81fc17eb998970478889d39538ac88cefd`.

Both runs pass the same explicit values:

- `--config <run-root>/config.toml`
- `--data-root /Users/jinkui/Library/Application Support/Koharu`
- `--target-lang en`
- `--steps pp-doclayout-v3,comic-text-detector-seg,speech-bubble-segmentation,yuzumarker-font-detection,paddle-ocr-vl-1.6,lama-manga,koharu-renderer`

The CLI must log the effective resolved order, including the implicit
`pp-ocr-v5-source-gate`. CPU and Metal must use identical config, steps, input,
model root, and models. Build the CPU command normally and the Metal command
with `--features metal`. Run both through this stdlib-only deadline wrapper;
record each exit status and fail on timeout or nonzero status:

```sh
run_with_deadline() {
  /usr/bin/perl -e '$seconds = shift @ARGV; alarm $seconds; exec @ARGV' 1800 "$@"
}

run_with_deadline bun cargo run -p koharu-app --bin pipeline -- \
  --input "$INPUT" --output-dir "$RUN_ROOT/cpu" \
  --config "$RUN_ROOT/config.toml" \
  --data-root "/Users/jinkui/Library/Application Support/Koharu" \
  --target-lang en --steps "$STEPS" --cpu

run_with_deadline bun cargo run --features metal -p koharu-app --bin pipeline -- \
  --input "$INPUT" --output-dir "$RUN_ROOT/metal" \
  --config "$RUN_ROOT/config.toml" \
  --data-root "/Users/jinkui/Library/Application Support/Koharu" \
  --target-lang en --steps "$STEPS"
```

### Model Inventory And Instance Evidence

Before model loading, emit one stable sorted model inventory. Each logical
model has one stable `model_id`, backend class, and one or more sorted artifact
records containing canonical local path, byte length, and SHA-256. Config,
preprocessor, label, and weight files may belong to the same model and do not
create device instances. CPU and Metal inventories must be identical; missing,
duplicate, changed, or downloaded artifacts fail.

At the existing CLI/model instance boundaries, emit private run diagnostics:

```text
model_instance_device engine=<id> model=<id> instance=<run-local-id> actual=<cpu|metal>
```

- No receipt, schema, public API, historical harness, or persisted protocol is
  introduced.
- Candle-backed instances report the actual held `Device`.
- In HanOnly, effective-step resolution replaces the direct
  `paddle-ocr-vl-1.6` engine with `pp-ocr-v5-source-gate`; that Source Gate
  constructs and runs its own live `PaddleOcrVl` instance. This internal
  instance is the Paddle evidence target. Do not add a second inference phase
  or change the Source Gate replacement rule.
- That live PaddleOCR-VL/llama.cpp instance reports backend/buffer
  introspection after its Source Gate inference. Metal requires nonzero Metal
  model/context/compute buffer evidence; backend enumeration, feature
  availability, or requested policy alone fails.
- Components that are CPU-only by design are explicitly classified in the
  inventory and may report CPU in both runs.
- In the Metal run every executed Metal-capable instance reports Metal. In the
  CPU run every executed instance reports CPU.
- Every executed model instance references exactly one inventory `model_id`;
  one model may own several artifact records. Missing/duplicate instance IDs,
  an instance without an inventory model, or a required effective-pipeline
  model that never executes fails. A user-requested step removed by HanOnly
  effective-step resolution is not a separate required instance.
- Add focused tests for stable formatting/sorting, Metal available but instance
  CPU rejection, llama enumeration without nonzero Metal buffers rejection,
  and the macOS actual-Metal integration path. Reuse existing low-level backend
  introspection helpers; do not restore historical evidence machinery.

Test files:

- `crates/koharu-app/tests/pipeline_smoke.rs` (smoke-script/CLI invocation
  tests; `bin/pipeline.rs` remains a production touchpoint only).
- `crates/koharu-app/src/pipeline/engine.rs` or the concrete engine wrapper
  modules (unit tests for diagnostic formatting/sorting and device reporting).
- `crates/koharu-llm/src/paddleocr_vl.rs` or its safe/sys bindings (tests for
  llama buffer introspection and Metal rejection).

### Visual Acceptance

Both output directories must contain nonempty, decodable `source.png`,
`inpainted.png`, `rendered.png`, and parseable `scene.json`; all images retain
the source dimensions. Accept only when:

- selected Han text is erased and rendered without clipping or owner overlap;
- protected Latin remains unchanged;
- unsupported targets are not partially rendered;
- CPU and Metal agree on target count, text, translation, warnings, source
  transforms, line polygons, and sprite transforms after sorting by a stable
  geometric owner key `(page index, source transform, source line polygon)`;
  random `NodeId` values are not comparison keys;
- private resolved-box equivalence is proved by the active G005 tests, not
  inferred from `scene.json`;
- pixel identity between CPU and Metal is not required.

Record the reviewed implementation commit, unique run root, config/input/model
hashes, exit statuses, and SHA-256 of logs, rendered outputs, and scenes in
`.omx/state/ralplan-g005-runtime-geometry-smoke-report.md`. Do not create a
receipt or evidence schema.

Completion: `.omx/state/ralplan-g005-runtime-geometry-smoke-report.md` exists
and contains the recorded evidence; both CPU and Metal runs exit zero and pass
visual acceptance.

## Stage Ownership

G005 activates exactly these three T2 tests:

1. `hanonly_pre_b1_red_t2_dynamic_layout_contract`
2. `hanonly_pre_b1_red_t2_pipeline_layout_handoff_contract`
3. `hanonly_pre_b1_red_t2_rotation_status_contract`

G006 retains exactly these two ignored T2 tests:

1. `hanonly_pre_b1_red_t2_blob_decode_budget_contract`
2. `hanonly_pre_b1_red_t2_replace_import_atomicity_contract`

All nine `hanonly-pre-greenc-red` T3 tests remain staged for G007. The two
active G004 Source Gate/PP-OCR T2 tests remain active and green.

## Verification Strategy

Each work package runs its focused tests while being implemented. After all
five packages pass, run the completion suite once:

1. Three G005 T2 tests and focused renderer/pipeline/RPC regressions.
2. Source Gate, PP-OCR, protected-Latin, manual-size, and AllText regressions.
3. RenderControlsPanel and source-image admission/legacy compatibility tests.
4. `bun cargo test --workspace --tests`.
5. `bun cargo check --workspace --all-targets`.
6. `bun run test:ui` and `bun run lint:ui`.
7. `bun run format:check` and `bun run check:generated`.
8. One fixed CPU/actual-Metal end-to-end smoke as defined in G005-WP5.

No calibration, holdout, marker, artifact, new Revision, or historical custody
command is part of G005 acceptance.

## Stop Conditions

Stop G005 implementation on:

- a reproducible product regression or protected-content violation;
- data-loss risk during ingress;
- a required change to successful replacement persistence/rollback semantics,
  which belongs to G006;
- inability to prove an executed Metal-capable model instance used Metal;
- need for a new public Scene/OpenAPI/protocol field or dependency.

Do not stop for retired custody, historical receipt format, or superseded
Revision requirements.

## RALPLAN-DR

### Principles

1. One backend authority owns automatic geometry and size.
2. One private resolved-layout record crosses resolver, fit, raster, and
   validation.
3. New ingress is strict while historical stored content remains readable.
4. G005 owns pre-mutation admission; G006 owns successful replacement
   transaction and durability.
5. Runtime observability stays private and minimal; no historical protocol or
   evidence framework returns.

### Top Decision Drivers

1. Make real expanded geometry reach output pixels without weakening owner or
   protected-content safety.
2. Prove actual model execution on CPU/Metal rather than requested policy.
3. Avoid breaking existing projects while aligning all new source ingress.

### Options

- **Selected: revise this single Execution Card.** It preserves one authority
  and closes the known implementation and acceptance gaps.
- **Rejected: add an appendix or second plan.** That creates competing scope and
  another status source.
- **Rejected: restore historical holdout/evidence harnesses.** Only existing
  low-level device introspection may be reused; the governance machinery is
  retired.

### Pre-Mortem

1. **Tests pass but product remains source-bbox locked.** Prevention: the
   handoff test requires real alpha outside source bbox and inside the resolved
   box through the production pipeline.
2. **Metal reports a false green.** Prevention: validate each executed model
   instance and require nonzero llama Metal buffers, not availability flags.
3. **Allowlist breaks old projects.** Prevention: constrain admission to new
   source ingress and retain explicit legacy GIF/BMP Blob-load tests.

### Expanded Test Plan

- Unit: deterministic layout allocation, admission sniff/decode, warning
  aggregation, model diagnostic sorting/classification.
- Integration: resolved record reaches raster/post-validation; RPC warning
  result; RPC/CLI admission no-mutation; legacy Blob readability; actual device
  matching.
- End to end: one hermetic CPU/Metal image run with fixed config/input/model
  inventory and visual acceptance.
- Observability: stable effective-step order, model inventory, per-instance
  device lines, timeout/exit status, and ordinary completion-report hashes.

## ADR

### Decision

Execute G005 through the five bounded work packages above. Keep the G006/G007
contracts staged and use one hermetic CPU/actual-Metal smoke at completion.

### Consequences

- G005 may change private renderer geometry, warning propagation, UI automatic
  presentation, new source-image admission, and minimal CLI model diagnostics.
- G006 remains responsible for successful replacement transaction, decode
  budgets, cache accounting, persistence, recovery, durability, race, and fault
  behavior.
- No new dependency or external Scene/OpenAPI/protocol/evidence surface is
  introduced.

### Follow-Ups

- After consensus commit, use `$ralph` for one-owner sequential implementation,
  or `$team` only when Rust and UI lanes are disjoint and one leader owns final
  integration.
- Use `executor` per bounded package, `test-engineer` for stage ownership,
  `code-reviewer` for the integrated diff, `architect` for final boundary
  review, and `verifier` for the completion suite.

## Consensus Handoff

- The prior `d4b55f5f...` consensus is superseded only for this repaired card.
- Planner, then Architect, then Critic review the same final card bytes in that
  order. Any plan-byte change after Architect approval restarts Architect then
  Critic review.
- After both approve, create
  `.omx/state/ralplan-g005-runtime-geometry-consensus.json` binding the plan
  SHA-256, planning-context path/SHA-256, reviewer IDs, decisions, order, and
  `gate_complete=true`.
- That state record is not an execution authority and must initially record
  `execution_authorized=false`.
- Execution becomes authorized only after a scoped Git commit contains the
  exact reviewed card bytes, descends from `d4b55f5f...`, excludes unrelated
  `scripts/dev.ts`, and satisfies:

  ```sh
  git show <consensus-commit>:.omx/plans/2026-08-07-g005-runtime-geometry-execution-card.md \
    | shasum -a 256
  ```

  The result must equal the Architect/Critic-reviewed plan SHA. Record the
  consensus commit separately; do not edit the reviewed plan bytes merely to
  change its status.
