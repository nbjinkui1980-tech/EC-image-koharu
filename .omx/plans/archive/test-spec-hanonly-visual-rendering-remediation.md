# Historical test specification: HanOnly visual rendering remediation

> Status: historical pre-functional-delivery specification; non-gating.
>
> This document cannot authorize or block G005-G009 and cannot restore R46-R60
> custody, Revision, holdout, authorization, receipt, marker, remote Ruleset, or
> other retired governance requirements. Current execution authority is
> `.omx/plans/2026-08-05-hanonly-functional-delivery-plan.md` together with its
> explicitly subordinate Execution Cards. The body below is preserved as a
> historical design and test snapshot.

## Purpose

Revision 50 is the current operative B0 Source Gate / PP-OCR detector-support contract. It inherits the ordered `S25L4/S25L5/S25L6/S25L7` candidates but replaces Revision 49 scalar/line/order coverage proxies with complete detector ownership and actual detector-to-Scene-to-downstream raster equivalence. The artifact requires integer `plan_revision: 50` and a versioned detector-support/raster preimage. Revision 49 remains immutable non-authorizing evidence: calibration passed, selected `S25L4`, and all eight CPU/Metal holdout cells failed. Revisions 46 through 49 are historical evidence or inherited contracts only; every inherited Revision 46 requirement below remains normative unless the Revision 50 section explicitly supersedes it.

## Revision 50 G004 B0 tests and evidence

1. PP observation tests prove all raw detector rectangles and `Option<TextLine>` results are retained; every source-scaled raw quad is captured as eight pre-quantization `f32::to_bits` values in exact `RotatedRect::corners()` order. Tests retain raw outputs, canonical lines whose each occurrence stores both occurrence index and actual canonical `corners().to_bits`, and recognition slots including `None`; each canonical occurrence bits-equals its indexed raw occurrence, flattened canonical identities/bits equal the raw multiset including duplicates, and recognition length equals line length. Input order cannot change canonical support; missing recognition, unassigned/shared segments, rotated/non-finite/non-positive geometry, and ambiguous topology reject. One `#[doc(hidden)]` workspace observation method is authorized; existing `word_boxes` delegates to its unchanged projection, and no external API changes.
2. Source Gate tests prove: one axis-aligned detector with numeric/neutral PP text and Han-only VL promotes; PP/VL line or scalar differences do not reject when support is complete; PP order changes preserve decisions; pure or VL-observed protected Latin rejects; incomplete support, multiple VL-only components, Han/Latin co-ownership, and ambiguous Latin reject.
3. An end-to-end test proves pixel equality from selected raw detector through emitted Scene, `eligible_text_lines`, and actual `line_support_mask`, using the existing shared raster implementation. It emits exact `detector-support-raster-preimage-v1` bytes under `.omx/plans/hanonly-r50-b0-evidence-contract.json`; validator tests reject unknown/missing/reordered semantic fields, non-integer geometry, forged equality/subset/verdict fields, byte-length drift, and SHA-256 drift.
4. The only target mask is `DimensionAndMaskValidatedTarget.agreed_mask`. Validator tests recompute coverage from actual downstream support and reject one uncovered target pixel, any protected support, unsupported rotation, non-unique target ownership, unmatched selected nodes, forged stored pass fields, or CPU/Metal disagreement.
5. Diagnostics use the closed `hanonly-r50-diagnostic-index-v1` and `hanonly-r50-cell-diagnostic-v1` schemas in `.omx/plans/hanonly-r50-b0-evidence-contract.json`. One logical index spans calibration and holdout; phase/candidate are per-record fields. Tests require one strictly sorted unique `phase/candidate_id/device/entry_id` record for every expected cell, exact B0/manifest/fixture/candidate/device binding, referenced diagnostic/device/log path plus recomputed byte length and SHA-256, and exactly one durable `captured_unclassified -> passed|failed` transition. Every publication retains an immutable monotonically numbered generation bound to the prior generation path/length/SHA; validator tests walk the full chain, require one changed cell per generation, and reject terminal rewrites. The pre-seal holdout hash must be null; the first holdout capture atomically binds the sealed hash, every later generation must preserve it, and missing/early/changed/rebound hashes reject. Tests also require raw detector indices to be unique and exactly `0..raw_detector_count-1`, with each embedded detector preimage length/hash independently recomputed. The validator rejects unknown, duplicate, missing, reordered, orphan, temporary, non-terminal, conflicting, or terminal-rewritten records and forged stored hashes or pass fields. Deterministic fault tests cover capture write/sync/publish and terminal transition failures.
6. R49 h01-h04 and x02-x05 run only as disclosed regression/challenge. Formal holdout assets are independently sealed after clean B0 SHA and selected calibration candidate freeze under a separate holdout manifest/hash, and stay unseen by implementation personnel until all eight cells terminate. The calibration manifest/hash remains immutable; a no-model `seal-holdout` step publishes the holdout extension before the pre-holdout anti-fixture check.
   The disclosed R49 h04 `product-id` is pure Latin: R50 challenge validation requires it to be absent from automatic targets and retained in protected support, without changing any R49 corpus byte.
7. Pre-B0 gates retain both B0-owned tests passing, default workspace tests/checks, generated/format/policy/anti-fixture/Revision 50 marker inventory, and exactly five T2 plus nine T3 staged RED. Formal calibration is `r50-c01` through `r50-c04`, including c04, across all four candidates and both devices. Formal holdout is sealed `r50-h01` through `r50-h04` on only the frozen candidate.
8. Any calibration, holdout, diagnostic, authorization, permission, device, model, hash, canonical-JSON, or cleanliness failure is terminal for Revision 50: preserve diagnostics, refuse artifact freeze, keep G004 in progress and G005 pending, and do not retune or reuse failed holdout.

Revision 46 additionally proves that normal HanOnly execution inside `pipeline::run` is exactly `spec.options.region.is_none()`: selected font/Typography producers are conditionally ordered before the selected inpainter, zero-inpainter pages perform zero builder/reservation/raster/publication work, and one-inpainter pages invoke one sole crate-private builder exactly once for the current `PageId` immediately before inpaint. After every selected upstream producer has committed and immediately before that builder, the run captures `B_prepare` from observable Scene bytes/epoch, History epoch/canonical-log bytes, persisted sprite inventory, and `reachable_blob_state(B_prepare)`: the canonical sorted exact-reference/length/byte-hash map reachable from current Scene, committed/replayable History op roots, and current undo/redo op trees. Physical unreachable CAS is excluded from equality and separately audited. The builder completes final translation identity, layout, cluster/control coverage, glyph validation, fill rasterization, nonempty alpha, checked complete-page retained logical-sprite reservation, independently checked actual-scale transient raster construction, serial per-target scratch release, and atomic immutable sprite publication before any inpainter run/apply; a selected Renderer later persists/composites the same frozen sprite exactly once with zero layout/raster rebuild or public render-contract change. HTTP `start_pipeline` rejects `req.region.is_some()` before session/job/cancel/event/task effects, and direct `pipeline::run` rejects `spec.options.region.is_some()` before registry/order/page/Scene/run-state/engine/warning/blob/history effects. Public `StartPipelineRequest.region`, `PipelineRunOptions.region`, serialization, and `options_from_request` shapes remain unchanged; their behavior/response descriptions are truth-synced, HTTP 400 is documented, and normal OpenAPI regeneration may change generated documentation artifacts. The explicit region-bearing direct repair route in `routes/pages.rs` is the sole allowed direct engine ingress outside this matrix only after a pre-side-effect guard proves URL role `MaskRole::Segment`, successful registry resolution, and exact descriptor output `[Artifact::Inpainted]`; accepted repair retains its direct inpainter path with zero builder/publication/accessor, while all rejected engine classes return one stable HTTP 400 before body/blob/Scene/engine/apply. The exact public `EngineCtx` field set, crate-root re-export, and by-value `Engine::run(&self, EngineCtx<'_>)` signature remain unchanged; one crate-private Tokio task-local `Arc<PipelineRunState>` per accepted run stores immutable objects by `PageId` and is proved unavailable outside the current-page accessor scope, isolated across pages and sequential/concurrent/nested runs, restored after nested scopes, dropped at run completion, and not inherited by spawned tasks. Pixel payloads release after successful composite, valid no-Renderer completion, or failed inpaint with zero sprite persistence and never accumulate across pages. Every deterministic hard pre-write `geometry_or_font` failure (translation-identity mismatch, final-layout failure after local fallback, cluster/control-coverage failure, glyph-validity failure, fill-traversal failure, or nonempty-alpha validation failure) and every recoverable `geometry_or_font` failure (checked logical/actual-raster arithmetic or `usize` conversion failure, retained owner-rectangle/page-cap reservation failure, or fallible raster-surface-construction failure) leaves final Scene/History/reachable-blob/sprite state exactly equal to `B_prepare`, adds no builder publication, inpainter run/apply, erase, or downstream persistence, and retains upstream producer commits already in that baseline. Process-level allocator OOM/abort is outside recoverable `geometry_or_font` semantics. It also proves that automatic source-color/stroke classification is admitted by deterministic `J -> T -> Wterm -> Wmeta` page/target accounting before traversal, P1 requested/reserved values are exact conservative bounds while consumed records only real declared operations, all six P0 terminal tuples and page aggregation semantics are bound into the source-contract preimage, every strict-color/P95/pair decision and both contract-hash preimages have one closed interpretation, each staged RED executes by one uniquely resolved full test name, valid complexity rejection preserves the complete ROI before erase, every successful target proves complete pre-inpaint translation/cluster/glyph/fill/alpha rasterization with glyph zero rejected as missing, guarded reports retain local text-derived proof, sanitized exports retain only random content-independent correlation IDs and verdicts, and elapsed time never affects correctness.

Immediately after upstream producer commits and before builder work, acquire History then Scene for one coherent pre-permit staging snapshot; capture authoritative `expected_epoch` plus the read-only Scene/History inputs defining `B_prepare`; release Scene then History before any permit acquisition.

Revision 46 closes four additional hard gates. P-1 reads only O(1) page-node count `J`, reserves `J`, and charges the one node traversal that derives automatic-target count `T` before P0 target terminalization or canonical ranking; total `choose2_checked` covers `T=0/1` without unsigned underflow. The repair route accepts a pipeline only when the URL role is `MaskRole::Segment` and its resolved descriptor produces exactly `[Artifact::Inpainted]`, with rejection before body decode/blob/Scene/engine/apply side effects; `PutMaskParams.pipeline` type/serialization shape remains unchanged while public behavior/400 documentation is corrected and regenerated. Every successful target proves before inpaint that the complete expected translation reached the shared raster path through expected/frozen/pre-inpaint-raster-entry UTF-8 digests, reversible insertion-only Planner breaks, complete shaped-cluster or layout-control byte coverage, nonzero representable glyph IDs, successful fill-pass traversal of every shaped glyph, and nonempty alpha. The builder checked-reserves the complete retained logical sprite set with `logical_width*logical_height*4`, checked `usize` conversion, owner-rectangle containment, and checked total not exceeding page RGBA bytes. Independently, for each target at the actual existing 2-4x scale, it checked-computes raster width/height/surface bytes, uses the existing fallible surface constructor, rasterizes serially, and releases that target's supersampled/copy/downsample scratch before the next target; at most one target scratch surface is live. Renderer proves same frozen sprite identity and zero raster calls. A translation-identity mismatch, final-layout failure after local fallback, cluster/control-coverage failure, glyph-validity failure, fill-traversal failure, or nonempty-alpha validation failure is a deterministic hard pre-write `geometry_or_font` failure. Only checked logical/actual-raster arithmetic or `usize` conversion failure, retained owner-rectangle/page-cap reservation failure, or fallible raster-surface-construction failure is a recoverable `geometry_or_font` failure. Both classes preserve exact equality to `B_prepare`, retain upstream producer commits, and produce zero publication, inpainter run/apply, erase, or downstream persistence. Process-level allocator OOM/abort is excluded. Exact final replay and per-target omission inequality remain post-composite proofs. Guarded local reports store no raw text but may retain unsalted text-derived digests/counts; exported `R/A/C/G`, corpus, and annotation contain neither raw text nor text-derived digests/lengths/counts.

Revision 46 also closes the platform/backend gap. The History COW boundary is accepted only through one private `Committed|Unchanged|Indeterminate` adapter whose verdict comes from exact canonical bytes plus platform-required sync/reopen evidence, never from `rename` return alone. Unix requires parent-directory `sync_all`; Windows relinquishes the old writer before replacement and requires canonical-file `sync_all`, exact-new byte verification, a usable append writer, and no temp, without claiming directory fsync. Every sprite and final Rendered CAS object uses the separate Blob-local `durable_put_exact` helper: common same-directory temp create/write/flush/sync/exact verification and handle release, then Unix rename/canonical verification/parent-directory sync or Windows overwrite rename/canonical reopen/file sync/exact verification. Blob/CAS failure before History replacement preserves `B_prepare` reachable state, may leave only verified `unreachable_cas`, and does not poison the session. One ProjectSession-owned persistence control gives poison and every library-mediated canonical/success publication a total order: the atomic flag is a fast reject only; a non-cloneable mutex-backed permit spans under-gate poison recheck, CAS promotion, History then Scene publication, and synchronous response/Dirty/autosave/job success. History `Indeterminate` alone stores poison before permit release and returns exactly `project persistence is indeterminate; close and reopen the project`; no `.await` or inner-lock-to-gate acquisition is allowed. Public write-capable ProjectSession fields become private and direct callers migrate to read-only/gated methods; BlobStore public put signatures remain unchanged and auto-gated. One shared pure `backend_admission` decision runs after region rejection at both RPC and direct-run ingress. Unsupported RPC skips Registry but still creates one job and emits no warning; direct `pipeline::run` emits the sole stable warning and returns `warning_count=1`, producing `CompletedWithErrors`. Admitted unknown IDs retain synchronous HTTP 400. Normal HanOnly admits explicit CPU on every target and requested Metal only on macOS; CUDA, Vulkan, and every other non-CPU path outside macOS fail closed. A real Windows History/Blob behavior test is an unconditional prerequisite of the sole required `HanOnly Production Policy` context.

Builder failures are disjoint. A translation-identity mismatch, final-layout failure after local fallback, cluster/control-coverage failure, glyph-validity failure, fill-traversal failure, or nonempty-alpha validation failure is a deterministic hard pre-write `geometry_or_font` failure. Only checked logical/actual-raster arithmetic or `usize` conversion failure, retained owner-rectangle/page-cap reservation failure, or fallible raster-surface-construction failure is a recoverable `geometry_or_font` failure. Both classes drop unpublished state, preserve exact final Scene/History/reachable-blob/sprite equality to `B_prepare`, retain upstream producer commits, and produce zero publication, inpainter run/apply, erase, or downstream persistence. Process allocator OOM/abort is excluded.

Prove that HanOnly successful modes remove complete source ink, place complete translated ink at the largest locally safe size, preserve exact source-derived color and deterministic stroke width, and actually composite each pre-inpaint-frozen sprite into final Rendered RGBA; prove that unsupported modes preserve their complete source ROI and report explicit failure. Also prove geometry-relative crop/size policy across independent source images, Planner suggestions only inside local safety rules, independently classified protected Latin, one trusted-target closure plus cross-platform common/generator-lock equality and same-target reproducibility through the exact PR candidate head, distinct required-check test-merge identity, and builder-local atomic failure relative to `B_prepare` with zero inpainter run/apply whenever translation, layout, glyph, fill, alpha, retained reservation, actual-scale raster construction, erase, or ownership safety cannot be established. Earlier successful producer commits in `B_prepare` remain intact. Human review is limited to successful-mode inpaint texture plausibility.

This is the normative Revision 46 test contract linked by the main remediation plan. Revision 29 artifacts and verdicts are historical, and Revisions 30 through 45 cannot satisfy this specification.

### Revision 46 logic-correction delta

The following rules are normative everywhere in this specification and supersede any older contradictory wording without changing either frozen canonical JSON preimage or either corresponding hash:

1. Direct page repair is legal only for URL `MaskRole::Segment` plus a resolved descriptor whose exact output is `[Artifact::Inpainted]`; the successful Segment repair behavior remains unchanged.
2. Aggregate CTD ink is converted to per-node support by forbidden subtraction followed by stable 8-connected components. Exactly one eligible anchor/contact owner receives a whole component; zero owners drop it; multiple owners fail the page before inference.
3. Missing, empty, or non-containing bubble evidence limits layout to the original source anchor. It never authorizes the full base page.
4. The shared staged Batch/History commit boundary is implemented and accepted in GREEN-B. GREEN-C reuses it.
5. Static inpainter `needs`/`produces` descriptors and AllText order are unchanged. Only accepted normal HanOnly runs receive private conditional order edges.
6. A Renderer+Inpainter row is one `B_prepare`-scoped page transaction. Inpainted Scene changes, sprite bytes, normalized frozen `sprite_transform`, `rendered_direction`, and final Rendered output commit once only after every engine/blob/cancel/epoch check succeeds.
7. Render admission reserves all simultaneously live buffers, including retained logical sprite, supersampled Pixmap, unavoidable copy, downsample output, masks, staged Scene/blob state, and transaction buffers. Supported inputs cannot rely on allocator OOM as a normal boundary.
8. Every ink-bearing shaped cluster/run contributes nonzero final logical-sprite alpha. Ligatures and combining marks are judged at the shaped cluster/run boundary; legal controls/whitespace are classified separately.
9. Successful visual targets provide two blind independently prepared source-ink masks. They must agree exactly before runtime outputs are opened. Before opening runtime output, derive `M_delta={p in clean_reference_edit_roi | Source[p] != Clean[p]}` from the independently prepared Clean reference; require nonempty `M_delta` containing the agreed mask, run the same frozen residue criteria over both masks, and allow only the existing pinned Source Gate OCR model/config to add a rejection for a positive-area candidate intersecting `M_delta`.
10. `hanonly-test-evidence` is observational only. A default-feature run and an evidence-feature run of the same existing public-output contract must be byte-identical on public Scene/blob/Rendered bytes, status, warnings, transforms, directions, and error tokens.
11. The production-policy scanner is defense in depth. The required job also runs the existing ordinary adversarial test with a CI-runtime cryptographic seed and post-build generated dimensions, geometry, node order, text length, color, transform, and supported in-memory format variants.
12. Public failure remains `geometry_or_font`; a private typed discriminant separates content/geometry, cluster contribution, memory arithmetic, reservation, and surface-construction causes.
13. GREEN-B History commit is cloned Scene/op staging plus whole-log copy-on-write under item 20's outer persistence permit. Before permit acquisition, acquire History then Scene once, capture authoritative `expected_epoch` and coherent read-only staging inputs, release Scene then History, finish all async/decode/engine work, clone/apply staged Scene/op state, compute the proposed next epoch/undo/redo, and serialize the complete frame. Under the permit, recheck poison only, promote required Blob objects without History/Scene locks, release Blob-local/cache locks, acquire History exactly once within the permit-held publication boundary, perform exactly one `current_epoch == expected_epoch` comparison, then acquire Scene and revalidate required state. Within the permit-held publication boundary, Blob promotion and Blob-local lock release are the first persistence actions after the poison-only recheck; History acquisition and epoch comparison follow them. This ordering constraint does not apply to the required pre-permit coherent staging snapshot. A stale epoch at this point publishes no Scene/History/success state, never poisons, and may leave only exact verified `unreachable_cas`. Read the old canonical generation into immutable bytes and parse it with existing compatible startup semantics; `old_valid_log_bytes` is its last-complete-frame prefix and excludes only an accepted malformed/truncated trailing frame from the new generation. Write, flush, and `sync_all` `old_valid_log_bytes + new_frame` in one task-owned same-directory complete temp; strictly replay that immutable temp to exact EOF from a separate fresh clone; require exactly the new next-epoch frame to apply once and the result to equal staged Scene/next epoch. Flush/sync and relinquish the old writer, then invoke pinned Rust 1.97 `std::fs::rename` through one private platform adapter. Unix `Committed` requires rename, parent-directory `sync_all`, exact-new canonical bytes, no temp, and a synced reopened append writer. Windows `Committed` requires rename, exact-new canonical bytes, canonical-file `sync_all`, no temp, and a usable append writer. `Unchanged` requires exact raw-old canonical bytes, no temp, and a synced reopened old writer. Every other observation is fatal `Indeterminate`: while the permit remains held, its ProjectSession caller sets `persistence_poisoned` and returns exactly `project persistence is indeterminate; close and reopen the project`, without removing the session or publishing success. Release Scene then History; only a `Committed` path may publish synchronous response/Dirty/autosave/job success before releasing the permit. Read-only Scene/project-summary/diagnosis/export access remains gate-free and cannot clear poison. Apply, apply-if-epoch, undo, redo, snapshot/compact, autosave, page import/mask repair, public/direct Blob puts, pipeline CAS promotion, and pipeline History commit all use the same control. Pipeline poison is `Failed`, never a warning or `CompletedWithErrors`. Autosave logs once and exits. Explicit close removes the session, joins autosave, skips `FlushNow` and final compact, and drops the lock; only a later open creates an unpoisoned session after strict-first canonical recovery. `Committed` alone publishes staged Scene/epoch/undo/redo; `Unchanged` preserves `B_prepare`. Restart first strictly replays immutable canonical bytes on a disposable fresh clone; only a malformed/truncated trailing-frame failure may discard that clone and use existing compatible replay on a second fresh clone, which can denote only the old generation. Every other replay failure rejects open; probe state is never reused and the new frame cannot apply twice. No stronger hardware/power-loss claim is made beyond successful host sync contracts.
14. Every operative `B_prepare` blob equality uses `reachable_blob_state`; verified immutable unreachable CAS is excluded, cannot be read by Scene/replay/undo/redo, and is reported only by a separate orphan/integrity audit. Task-owned temp files must be removed.
15. The required metamorphic job resolves the unchanged ordinary short ID to exactly one module-qualified full libtest name through `--list`, runs only that name with `--exact --nocapture`, and proves started=1, passed=1, failed=0. Missing, renamed, deleted, duplicate, root-unqualified, or zero-test output fails.
16. Integration/source-policy exclusively owns every production and test hunk in `op.rs`, production-owned `history.rs`, the `blobs.rs` durable-promotion/bound-control/permit-aware helper, whole-file task ownership of `session.rs`, `app.rs`, and `autosave.rs`, `PersistenceControl`, permit/accessors/field encapsulation, `pipeline/mod.rs` shared backend admission/direct guard/gated transaction/fatal propagation, and `routes/pipelines.rs` preflight/job behavior, plus Batch staging, History commit/gate/race/fault tests, Windows prerequisite coverage, and import/GREEN-C reuse. The exact current direct-field caller closure is `crates/koharu-app/src/session.rs`, `crates/koharu-app/src/app.rs`, `crates/koharu-app/src/ai.rs`, `crates/koharu-app/bin/pipeline.rs`, `crates/koharu-app/src/pipeline/mod.rs`, `crates/koharu-rpc/src/binary.rs`, `crates/koharu-rpc/src/mcp/mod.rs`, `crates/koharu-rpc/src/psd_export.rs`, `crates/koharu-rpc/src/routes/pages.rs`, `crates/koharu-rpc/src/routes/projects.rs`, `tests/integration-tests/tests/binary.rs`, `tests/integration-tests/tests/pipelines.rs`, and `tests/integration-tests/tests/scene.rs`. Every migration hunk and the same-module `ProjectSession::open_untrusted` structural audit is exclusively integration/source-policy owned. Planner/provenance owns no hunk, sign-off, or checkpoint in those surfaces.
17. One shared pure `#[doc(hidden)] pub backend_admission(source_text_policy, cpu, target_os)` decision is total and compile-target-driven. Because `pipeline` is public, it is an explicit additive downstream-public Rust helper, not a private/workspace-only API; no existing signature, request/options/status/OpenAPI field, Scene, Engine, `EngineCtx`, or Renderer contract changes. AllText is admitted for both CPU flag values. `HanOnly + cpu=true` accepts everywhere; `HanOnly + cpu=false + macOS` accepts and remains subject to actual-Metal T6 proof; every other HanOnly non-CPU target returns exact reason `han_only.unsupported_backend: runtime backend is not accepted by Revision 46; retry with explicit CPU`. RPC executes region rejection then this decision before request-step Registry preflight. Unsupported RPC records zero Registry calls, emits no warning, and still creates exactly one job; direct `pipeline::run` repeats the same decision before `infos_for_spec` and emits the sole exact warning with `warning_count=1`, so the job becomes `CompletedWithErrors` once. Admitted unknown IDs retain synchronous HTTP 400 before job creation. Direct callers cannot bypass admission.
18. `.github/workflows/test.yml` contains one `windows-2022` prerequisite running exact default-feature `history::tests::windows_durable_replace_generation_contract` and one required `HanOnly Production Policy` job with exact `needs: [hanonly-windows-history-contract]`, job-level `if: ${{ always() }}`, and first-step rejection unless the prerequisite result is exactly `success`. The checker rejects every mutation that could turn Windows failure/cancellation/skip into a missing or successful required context.
19. Blob/CAS uses exactly one separate helper in existing `blobs.rs`, `durable_put_exact`, and never reuses the History adapter or adds a module/dependency. The ProjectSession store is bound to item 20's exact control. Public `BlobStore::{put_bytes,put_webp,put_raw}` signatures remain unchanged and auto-acquire the bound control; crate-private permit-aware variants require permit/store control identity and avoid recursive acquisition under an outer transaction. Standalone stores own an independent local control and cannot become a ProjectSession store. Common behavior is same-directory temp `create_new`, complete write/flush/temp `sync_all`, exact temp length/bytes/hash verification, and handle release. Unix then renames, verifies exact canonical bytes/hash, and syncs the parent directory. Windows releases related handles, performs overwrite rename, reopens and `sync_all`s the canonical file, and verifies exact canonical length/bytes/hash, with no directory-fsync claim. Existing-canonical reuse/repair and certain-owner temp cleanup are explicit. Single puts hold the permit through cache publication. Blob/CAS failure before History replacement leaves `B_prepare` reachable state unchanged and does not poison; exact verified promoted objects may appear only in sorted `unreachable_cas`.
20. `ProjectSession` owns one private `Arc<PersistenceControl>` with an optional Acquire fast-reject `AtomicBool` and one non-reentrant synchronous mutex that alone linearizes persistence. Every operation acquires the gate, reloads poison under it, creates a non-cloneable identity-bound permit, and holds it through canonical, in-memory, and synchronous success publication. The one normative order is: pre-permit coherent History-then-Scene snapshot and `expected_epoch` capture; Scene-then-History staging-guard release; lock-free staging completion; permit acquisition; poison-only under-gate recheck; Blob promotion with no History/Scene lock; Blob-local/cache-lock release; exactly one permit-held post-Blob History acquisition; exactly one `current_epoch == expected_epoch` comparison; Scene acquisition and required-state revalidation; History canonical plus Scene/epoch/undo/redo publication; Scene-then-History release; synchronous response/Dirty/autosave/job success; permit release. No staging guard crosses permit acquisition. No permit-held pre-Blob History acquisition or epoch read/validation, atomic epoch mirror, additional gate, additional permit-held History phase, or compatibility shim is allowed. A stale epoch after verified Blob promotion leaves only exact verified unreachable CAS, poison false, and no Scene/History/success publication. No gate acquisition while holding History, Scene, Blob-cache, autosave, Registry, job-map, or event-bus locks; no `.await` under a permit. `ProjectSession.scene/history/blobs` become private and former direct callers use narrow read-only/gated methods. Same-module `ProjectSession::open_untrusted` is covered by structural policy because privacy alone cannot prevent bypass. Removing those public fields is an explicit downstream-breaking Rust helper/API change with no compatibility shim; existing request/schema, Scene data model, Engine, `EngineCtx`, Renderer, and BlobStore public put signatures remain unchanged. History `Indeterminate` stores poison under the still-held permit before release and suppresses every success. Deterministic barriers/channels prove two total orders without sleeps: poison wins between another writer's fast read and gate acquisition, or an already-permitted writer completes canonical plus success publication while the poisoner blocks and poisons only after release.

Only the D0 provenance preflight may assert the selected approved regression container's manifest-declared raw SHA-256, approved decoded-pixel identity, and decoded dimensions. No behavioral assertion may encode that image's dimensions, bboxes, card layout, fixed padding, fixed font-size deduction, text/line count, crop filename/bounds, source hash, or NodeId.

## T0. Layered baseline and diagnostics

Extend the existing ignored model-backed pipeline harness in `crates/koharu-app/src/pipeline/mod.rs`; do not add a second CLI or committed scene/image fixture.

For each run, record into a caller-visible absolute run-specific evidence directory that persists after the command exits:

- decoded Source and independently approved clean-reference hashes and dimensions;
- raw Segment mask, final erase mask, Inpainted image, and Rendered image hashes;
- per-node recognition anchor, selected segment-component extent, final erase-support extent, protected-overlap pixel count, dynamic layout region, crop-padding decision, source-size estimate, blanket deduction, candidate cap, independent safe size, grouped size, raw final-size input `f_t_raw`, rounded/frozen integer-page-pixel `F_t`, derived `W_t`, preflight publication state, `B_prepare` equality/upstream-retention verdicts, checked logical sprite dimensions/bytes/reservation, actual raster scale/dimensions/surface bytes, peak-live-scratch count, per-target scratch-release verdict, frozen sprite identity, payload-release verdict, final sprite alpha bounds, rotation support reason, source `F_s/W_s`, reduced stroke ratio, guarded-only expected/frozen/pre-inpaint-raster-entry UTF-8 SHA-256 values, renderer-input byte/scalar counts, insertion-only break transcript digest, layout-cluster/control coverage digest and counts, shaped/representable/missing/fill-visited glyph counts, coverage/completion/Renderer-zero-raster verdicts, one random content-independent opaque target-correlation ID, actual persisted Source/Inpainted-base/Brush/ordered-sprite hashes/transforms and page-node order, independently recomputed protected-support hash, exact-composite/omission verdicts, and per-field Planner outcome;
- per-node 8-connected component IDs and zero/exact-one/multi-owner terminal states; per-ink-bearing-shaped-cluster/run alpha-contribution counts; normalized frozen `sprite_transform` and `rendered_direction`; private failure discriminant plus unchanged public token; complete peak-live memory terms/reserved/consumed/released bytes; page-transaction stage/commit/rollback verdict; dual blind-mask hashes/equality and full-edit-ROI residue verdict; and default/evidence public-output equivalence verdict;
- per-page dynamic-resolver elapsed time, page-pixel count, target count, and owner-mask bytes;
- per-page `classifier_page_node_count=J`, derived `classifier_automatic_target_count=T` when enumeration is admitted, and `classifier_p0_phase=node_enumeration|target_terminalization|canonical_ranking`; per-automatic-target `classifier_budget_version`, `classifier_terminal_state`, core count, planned/evaluated candidate counts, width-probe count, every exact target/page preflight/evaluation/total requested-reserved-consumed field, independent actual-operation counter totals, target/page limits, exact limit kind, and elapsed microseconds; P1 requested/reserved is the conservative calculator bound while consumed is only real declared work and may be lower; `p0_metadata_unbounded` records no target when enumeration or terminal-slot reservation was not admitted, and elapsed time is diagnostic only;
- raw Source Gate requested/load/backend-device/model/executable evidence plus every runtime node anchor/rotation/selected bit and the canonical production-closure schema/common/generator-lock/trusted-target hashes;
- pre-render clean-reference residual metrics and requested/actual final AOT backend names;
- a non-executable mode-0600 `evidence-ledger.json` containing only the verified input path/hash, visual-manifest path/hash, frozen repository Source Gate fixture-manifest hash, and canonical evidence root for fresh-shell rehydration.

Evidence has two noninterchangeable classes. Detailed T0/T5/T6 reports and the opaque-ID-to-local-target map are mode-0600 regular files beneath the descriptor-guarded mode-0700 external evidence root; they may contain unsalted text-derived digests and byte/scalar/cluster/glyph counts but never raw source or translated text. After validating the manifest target schema but before reading or hashing any target text, generate one `target_correlation_id` per manifest target from 128 bits of operating-system cryptographic randomness and encode it as exactly 32 lowercase hexadecimal characters. IDs are unique within the acceptance run, stable for the same target across its 40 cells, nondeterministic across reruns, and encode no text, NodeId, path, position, length, digest, image content, or other target content; the mapping never leaves the guarded root. Exported `R` and `A` may contain only correlation-ID-keyed booleans for translation identity, cluster/control coverage, glyph coverage, fill traversal, exact composite, and omission plus unrelated production-closure fields. Exported `C/G`, the CI corpus, workflow annotation, and closure summaries contain no target text-proof fields or mapping. Every exporter applies a closed-schema field-and-value scan that rejects raw text, stable NodeId, source/OCR/translated line counts, text/byte/scalar/cluster/glyph counts, text lengths, unsalted text-derived digests, guarded report paths, or the guarded mapping. No keyed-digest dependency is introduced.

Before creating the implementation worktree, create one owned mode-0700 `HANONLY_ORIGINAL_SNAPSHOT_DIR` beneath the preexisting shared external evidence base, outside every worktree. Persist mode-0600 `head.txt`, NUL status, tracked/staged binary diffs, untracked path/blob identities, exact `typography.rs` binary patch/blob hash, `pre-edit-sha256.json`, and exact `pre-edit-Cargo.lock` bytes plus SHA-256/owner/mode/`(st_dev,st_ino,type)` metadata. Compute any path key as the first 16 lowercase hex characters of SHA-256 over repository-relative path bytes. `pre-edit-sha256.json` covers every planned Cargo/config/toolchain manifest plus `Cargo.lock`. All snapshot files are written/fsynced before any implementation-branch manifest, lock, config, toolchain, or source edit. The implementation-time checker descriptor-walks and validates this external snapshot; D0 does not need to create it or add it to the closed ledger. No automatic trap may delete the snapshot or D0 run root; only nested scratch data may be removed. The verifier owns explicit cleanup after baseline, reports, contact sheets, original-worktree equality, and governance evidence are accepted and checkpointed.

The first D0 scaffolding hunk adds only the test-only Python-standard-library files `scripts/hanonly_evidence_ledger.py` and `scripts/hanonly_evidence_ledger_test.py`; neither is imported by production code and neither adds a package or runtime dependency. `python3 -m unittest scripts/hanonly_evidence_ledger_test.py` must pass before any evidence directory or production hunk is created. D0 exposes exactly four modes: `freeze-history-registry`, `create`, `rehydrate`, and `seal-holdout`. `freeze-history-registry` is mandatory before the first B0 production-code edit; `seal-holdout` remains unavailable until candidate freeze. All four use the same `os.open(..., dir_fd=...)`, `os.mkdir(..., dir_fd=...)`, and `os.rename(..., src_dir_fd=..., dst_dir_fd=...)` descriptor walker with `O_DIRECTORY|O_NOFOLLOW`, same-descriptor regular-file/SHA-256, mode/owner, closed-schema, root-containment, and NUL-output functions. Startup fails before mutation unless Python exposes all required `dir_fd` operations and flags on the current platform.

`seal-holdout` accepts exactly one current-user mode-0700 intake directory below the canonical external evidence root and outside every worktree. Its only entries are mode-0600 `seal-input.json`, `custody-attestation.json`, `operator-attestation.json`, `erase-mask-lane-attestation.json`, `residual-mask-lane-attestation.json`, and `assets/`; the closed input, custody, lane, entry, target, enum/boolean/ID, relative-path, asset, output-manifest, output-ledger, canonicalization, deterministic field mapping, durability, idempotency, and negative-case schemas are exactly the Revision 50 evidence contract. Before the first B0 production-code edit, D0 creates and freezes one mode-0600 `hanonly-r50-historical-root-registry-v1`; its closed sorted canonical roots enumerate every R46-R49 evidence/corpus root, the R50 calibration and pre-holdout roots, and disclosed challenge roots. Its path/length/SHA are bound into the calibration ledger and later B0-preflight attestation, so a holdout operator cannot supply or omit roots. The registry inventory records raw SHA and, for every decodable image/mask, decoded kind, dimensions, and BLAKE3. The B0 preflight attestation also records exact `CODEX_THREAD_ID` and requires equality with a `Codex-Thread-ID` B0 commit trailer. After calibration and candidate freeze, three distinct native subagents independently publish the operator, erase-mask, and residual-mask lane attestations. Each binds issuer `CODEX_THREAD_ID`, parent leader thread, native role, launch submission ID, exact B0/candidate/seal-input and its assigned raw/decoded asset hash map. Custody binds each separate file's path/length/SHA and requires all three issuer thread/submission pairs distinct, all share the leader parent, and none equals the implementation thread. It recomputes the authoritative registry inventory and requires every formal source raw SHA, decoded BLAKE3, and `r50-h01` through `r50-h04` identity absent; `r49-h01` through `r49-h04` and `x01` through `x05` are explicitly disclosed and excluded. Missing lane file/receipt, self-issued or duplicate identity, unbound asset, incomplete/drifted registry, raw or decoded-equivalent reuse, or `runtime_output_opened:true` rejects. All three seal outputs live only in a dedicated current-user mode-0700 `source-gate-selection/holdout-seal/` directory; Rust preflight alone descriptor-walks the held parent, creates an absent child with `mkdirat`, reopens with `O_DIRECTORY|O_NOFOLLOW`, verifies owner/mode/type, and fsyncs child plus parent. Existing empty/exact-output states resume; symlink, collision, wrong owner/mode/type, or unknown entry rejects. Its only final entries are decode attestation, manifest, and ledger, plus at most the one deterministic temp legal for the current publication step. Before Python seal, the existing Rust evidence decoder runs a no-model preflight over the same held source/clean/mask bytes and writes the closed mode-0600 `hanonly-r50-holdout-decode-attestation-v1`. Every attested asset has exact raw hash, `rgba8_image|binary_mask`, positive dimensions and decoded BLAKE3; the sorted key set equals all and only source/clean/erase/residual paths referenced by seal input. The attestation binds the running B0 test-executable SHA and exact `["hanonly-test-evidence"]` feature set, and uses deterministic create-new/temp/fsync/rename/directory-fsync recovery. Python descriptor-validates and binds custody, all three lane attestations, and decode attestation but does not decode images or invoke PIL/b3sum. Seal validates four ordered IDs `r50-h01` through `r50-h04`, `runtime_output_opened:false`, exact B0 and frozen selected candidate, every raw hash and attested decoded digest, contained half-open rectangles, no unknown fields or filesystem entries, and zero symlink/path escape. It writes no asset and never changes input; it publishes only the two mode-0600 canonical JSON outputs through the contract's recoverable manifest-first/ledger-second state machine, each binding the custody-attestation SHA. Unit tests cover absent/empty/existing/symlink/collision directory states, every allowed file start state and crash point, missing/swapped/extra asset coverage, wrong kind/dimension/hash, duplicate IDs, duplicate/self implementation/preparation identities, missing/mismatched lane receipts, incomplete/drifted historical registry, raw or decoded-equivalent disclosed/previously-used source, wrong enum/boolean, output collision with same/different bytes, ledger-only, wrong/unknown temp, symlink collision, and each write/fsync/rename/directory-fsync fault. G004 B0 authorization recomputes custody, lane records, registry and inventory, reruns the same no-model decoder in memory from held descriptors, and requires canonical attestation byte equality, test-executable hash equality, and exact feature equality. Later IMPL authorization does not claim a rerun; it requires exact immutable B0 artifact/custody/lane/decode-attestation bytes, B0 commit, recorded G004 authorization digest, and eight-path byte identity.

Before `create` performs its first `mkdir`, it proves canonical `pwd -P`, requires `expected_base` to equal the caller-supplied canonical `HANONLY_SHARED_EVIDENCE_BASE`, requires that base to be absolute, owned by the current user, mode `0700`, outside the canonical path of the original, implementation, acceptance, and PR linked worktrees, and validates the direct run-id child shape. It walks every existing filesystem-root/base/input/visual-manifest component from held directory descriptors using one child name at a time with `O_DIRECTORY|O_NOFOLLOW`; absolute-path validation is never followed by an absolute-path mutation. It opens the approved input and visual manifest with final-component `O_NOFOLLOW`, reads each descriptor exactly once into immutable bytes, and computes SHA-256 from those bytes. One bounded magic-selected header parser accepts only the approved regression containers: JPEG SOF dimensions or WebP `VP8`, `VP8L`, or `VP8X` dimensions. It rejects truncation, malformed lengths, unsupported format/chunk, duplicate or contradictory dimension records, and trailing records that alter the selected dimensions, and requires `790x1023` from the same input bytes used for SHA-256; `sips` and `/dev/fd` path traversal are not used. The selected input raw SHA-256 must equal the `regression` entry's declared `sha256`, while the ignored Rust harness must decode those same immutable bytes and require the entry's approved `decoded_rgba_blake3`; the JPEG and decoded-equivalent WebP may therefore have different raw hashes but count as one regression identity. It also descriptor-walks and same-descriptor hashes the fixed tracked file `crates/koharu-app/tests/fixtures/source-gate-deterministic-recall/fixture-manifest.json`, and invokes `git status --porcelain=v1 -- <fixed-path>` through `subprocess.run` with an argument vector and `shell=False`; any output, missing tracked file, path/type/hash drift, or attempted task edit fails before directory creation.

`freeze-history-registry` derives the canonical project parent as the parent of canonical `repo_root`, descriptor-enumerates every direct child matching exactly `EC-image-koharu-hanonly-r(46|47|48|49|50)-*` or `hanonly-r(46|47|48|49|50)-*`, and descriptor-enumerates every existing direct child of canonical `HANONLY_SHARED_EVIDENCE_BASE`; it additionally includes fixed disclosed roots `hanonly-r49-corpus` and `hanonly-r49-intake` when present. It rejects any matching path omitted from the canonical sorted set. Each root item has exactly `canonical_path`, `st_dev`, `st_ino`, `kind`, and `root_inventory_sha256`; the registry has exactly `contract`, `plan_revision`, `project_parent`, `shared_evidence_base`, `root_name_rules`, `roots`, and `registry_inventory_sha256`. Each root inventory covers every regular artifact/log/manifest/ledger/source/image/mask using relative path, byte length, raw SHA-256, and, for existing no-model-decoder-supported images/masks, decoded kind, dimensions, and BLAKE3; unsupported non-image files carry null decoded fields. It publishes mode-0600 `hanonly-r50-historical-root-registry.json` below the shared evidence base through one deterministic hash-named temp and file/parent fsync state machine. A second descriptor enumeration immediately before publication and every later validation must reproduce the exact matching root set and identities, except the exact current R50 run root created after registry freeze and separately bound by the calibration ledger; any other added, missing, renamed, replaced, or omitted matching root rejects.

After all read-only checks pass, `create` keeps the already-existing shared-base descriptor open and creates only the direct mode-0700 run root relative to it; it never creates or repairs the shared base or any ancestor. It descriptor-opens the frozen history registry and binds its canonical path, byte length, and SHA-256. It writes/fsyncs the temporary ledger through a final `os.open(temp_child, ..., dir_fd=run_fd)`, renames it with `os.rename(temp_child, "evidence-ledger.json", src_dir_fd=run_fd, dst_dir_fd=run_fd)`, then fsyncs the final file, run directory, and shared-base directory; it never returns to the validated absolute path for `mkdir`, write, rename, recovery, or fsync. The deterministic temporary name is `.evidence-ledger.<expected-ledger-sha256>.tmp`. The exact closed JSON keys are `version`, `visual_input`, `visual_input_sha256`, `visual_manifest`, `visual_manifest_sha256`, `source_gate_fixture_manifest_sha256`, `historical_root_registry`, `historical_root_registry_byte_length`, `historical_root_registry_sha256`, and `evidence_root`. Values are data only and are never sourced or evaluated.

`create` is an idempotent transaction for the exact same arguments and `run_id`. Its only accepted run-root states, enumerated from the held run descriptor, are: absent; an owned mode-0700 empty directory; exactly one owned mode-0600 regular deterministic temp; or exactly one owned mode-0600 regular final ledger. For a temp state, remove only that deterministic task-owned temp relative to the verified run descriptor and rewrite it from the recomputed canonical expected bytes. For a final state, require byte-for-byte equality with those expected bytes, then repeat final-file/run-directory/base-directory fsync. Any unknown entry, symlink, additional file, wrong owner/mode/type, temp name/hash mismatch, final-byte mismatch, or changed invocation fails without overwrite or cleanup. A failure after run creation, write, temp fsync, rename, final fsync, run-directory fsync, or base-directory fsync emits zero bytes; a same-argument retry must converge through this state machine to one exact durable final ledger and no temp.

Immediately before output, `create` fresh-walks from a separately held filesystem-root descriptor to every emitted absolute path and the fixed fixture path, using `O_DIRECTORY|O_NOFOLLOW` per component, and requires `(st_dev, st_ino, file-type)` equality against the still-held repository, input, visual-manifest, fixture-manifest, history-registry, evidence-base, run-root, and final-ledger descriptors. It also rechecks owner/mode and exact final-ledger bytes. Namespace replacement therefore fails with zero output even when the old held object remained writable. Only after all fsyncs and this final identity proof does it emit exactly nine buffered NUL-delimited values: input path/hash, visual-manifest path/hash, evidence root, fixture-manifest hash, and history-registry path/byte-length/hash. `rehydrate` performs the same root-based fresh walk, identity/type/hash/mode/owner comparisons, exact root-set re-enumeration, and final-ledger byte validation before its nine-value output. The ignored manifest harness validates the current Revision 50 manifest schema, inheriting Revision 46 fields unless explicitly superseded, before any calibration output is viewed.

Caller-shell preflight shape:

```sh
set -euo pipefail
HANONLY_VISUAL_INPUT=/Users/jinkui/Desktop/test.jpeg
HANONLY_VISUAL_MANIFEST=/absolute/path/to/hanonly-visual-manifest.json
HANONLY_SHARED_EVIDENCE_BASE="${HANONLY_SHARED_EVIDENCE_BASE:?absolute external evidence base required}"
repo_root="$(pwd -P)"
test "$(git rev-parse --show-toplevel)" = "$repo_root"
source_gate_fixture_manifest="$repo_root/crates/koharu-app/tests/fixtures/source-gate-deterministic-recall/fixture-manifest.json"
history_registry_values=()
while IFS= read -r -d '' value; do
  history_registry_values+=("$value")
done < <(
  python3 scripts/hanonly_evidence_ledger.py freeze-history-registry \
    --repo-root "$repo_root" \
    --expected-base "$HANONLY_SHARED_EVIDENCE_BASE"
)
test "${#history_registry_values[@]}" -eq 3
history_registry="${history_registry_values[0]}"
history_registry_byte_length="${history_registry_values[1]}"
history_registry_sha256="${history_registry_values[2]}"
run_id="$(date -u +%Y%m%dT%H%M%SZ)-88718dd92986-$$"
ledger_values=()
while IFS= read -r -d '' value; do
  ledger_values+=("$value")
done < <(
  python3 scripts/hanonly_evidence_ledger.py create \
    --repo-root "$repo_root" \
    --expected-base "$HANONLY_SHARED_EVIDENCE_BASE" \
    --run-id "$run_id" \
    --input "$HANONLY_VISUAL_INPUT" \
    --expected-input-sha256 88718dd929860574afcb5fb89dd826bc6527e2af7172df239fa28b70b0d9cdb3 \
    --expected-input-size 790x1023 \
    --manifest "$HANONLY_VISUAL_MANIFEST" \
    --historical-root-registry "$history_registry" \
    --expected-historical-root-registry-byte-length "$history_registry_byte_length" \
    --expected-historical-root-registry-sha256 "$history_registry_sha256" \
    --source-gate-fixture-manifest "$source_gate_fixture_manifest"
)
test "${#ledger_values[@]}" -eq 9
HANONLY_VISUAL_INPUT="${ledger_values[0]}"
HANONLY_VISUAL_INPUT_SHA256="${ledger_values[1]}"
HANONLY_VISUAL_MANIFEST="${ledger_values[2]}"
HANONLY_VISUAL_MANIFEST_SHA256="${ledger_values[3]}"
HANONLY_VISUAL_EVIDENCE_ROOT="${ledger_values[4]}"
HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256="${ledger_values[5]}"
HANONLY_R50_HISTORICAL_ROOT_REGISTRY="${ledger_values[6]}"
HANONLY_R50_HISTORICAL_ROOT_REGISTRY_BYTE_LENGTH="${ledger_values[7]}"
HANONLY_R50_HISTORICAL_ROOT_REGISTRY_SHA256="${ledger_values[8]}"
export HANONLY_VISUAL_INPUT HANONLY_VISUAL_INPUT_SHA256
export HANONLY_VISUAL_MANIFEST HANONLY_VISUAL_MANIFEST_SHA256
export HANONLY_VISUAL_EVIDENCE_ROOT HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256
export HANONLY_R50_HISTORICAL_ROOT_REGISTRY HANONLY_R50_HISTORICAL_ROOT_REGISTRY_BYTE_LENGTH
export HANONLY_R50_HISTORICAL_ROOT_REGISTRY_SHA256
readonly HANONLY_VISUAL_INPUT HANONLY_VISUAL_INPUT_SHA256
readonly HANONLY_VISUAL_MANIFEST HANONLY_VISUAL_MANIFEST_SHA256
readonly HANONLY_VISUAL_EVIDENCE_ROOT HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256
readonly HANONLY_R50_HISTORICAL_ROOT_REGISTRY HANONLY_R50_HISTORICAL_ROOT_REGISTRY_BYTE_LENGTH
readonly HANONLY_R50_HISTORICAL_ROOT_REGISTRY_SHA256
```

Ledger regression fixtures run all four modes against absolute paths containing spaces, single and double quotes, `$()`, backticks, semicolons, and glob characters. Registry freeze, creation, and fresh-Bash rehydration preserve every byte exactly, recompute input/manifest/fixture/registry hashes from the same `O_NOFOLLOW` descriptors, derive JPEG dimensions from the exact hashed bytes, and leave a precreated command-execution sentinel untouched. Registry fixtures cover every root-name rule, aliasing device/inode, omission, extra matching root, post-freeze replacement, raw image identity, decoded-equivalent JPEG/WebP identity, unsupported files with null decoded fields, and the one exact post-freeze current-run exception. JPEG parser fixtures cover baseline/progressive SOF, legal metadata segments, truncation, malformed lengths, missing/duplicate/contradictory SOF, non-JPEG bytes, and dimension mismatch. Creation-mode negatives cover a relative, root, wrong-owner, wrong-mode, symlinked, or worktree-contained expected base; malformed/non-direct run ID; input/visual-manifest/fixed-fixture-manifest/history-registry reached through an intermediate or final symlink; hash/type/dimension mismatch; dirty/untracked/replaced fixture manifest; NUL/CR/LF path data; and a foreign/mismatched/extra-entry existing root. Pre-mutation failures assert that no evidence-base or run directory was created. Recovery positives cover the exact empty/temp/final states. Fault injection covers partial write, temp fsync, rename, final-file fsync, run-directory fsync, and base-directory fsync; every failed attempt emits zero NUL values, and retry with identical arguments yields one exact final ledger, no temp, and nine values once. Rehydration negatives cover an environment and ledger root that disagree with the D0 canonical external base/run root, an intermediate root symlink, wrong mode/owner, symlinked ledger, unknown/missing schema keys, invalid hash, input/visual-manifest/fixed-fixture-manifest/history-registry drift, and a replaced base/root/ledger component between validation and use. Race fixtures pause after each descriptor check and immediately before output, replace the named path or ancestor, and require zero output plus no mutation through the replacement object; successful output requires the fresh-walk `(st_dev, st_ino, type)` identities to equal all held descriptors. No operation may mutate a child through a path whose parent descriptor is not still held.

Guarded diagnostic records must omit source and translated text bodies. Every matched target in every guarded runtime cell must contain a stable node ID plus positive integer `source_line_count`, `ocr_line_count`, and `translated_line_count`; the validator recomputes each count from raw node/recognition/translation evidence before accepting it. These fields are mandatory only for local correlation and detailed validation. They are forbidden from `R/A/C/G`, the CI corpus, workflow annotation, and closure summaries, and never generate the `line_count` corpus category; that category is populated solely from checker-owned synthetic syntax-coupling sentinels so the required static rule remains testable without publishing real target counts.

RED for the approved input must show source-outline pixels outside the final erase mask, source-like post-inpaint pixels under the independent residual oracle, tight final layout regions, absolute C2 crop growth, backend blanket `-5px`, a UI-guessed automatic pixel value disconnected from private runtime geometry, and the existing small/black/no-stroke result. GREEN must show full target-ink support, a passing pre-render residual oracle, zero protected overlap, geometry-relative backend crop/size policy, no UI automatic-pixel estimate, preserved Auto/empty UI state, expanded dynamic regions where safe, and complete sprites.

## T1. Erase geometry

Add table-driven tests beside the existing mask tests in:

- `crates/koharu-app/src/pipeline/engines/ctd_segment.rs`;
- `crates/koharu-app/src/pipeline/engines/support.rs`;
- `crates/koharu-app/src/pipeline/engines/lama.rs`;
- `crates/koharu-app/src/pipeline/engines/aot.rs`;
- `crates/koharu-app/src/pipeline/engines/flux2_klein.rs`;
- conditionally `crates/koharu-ml/src/inpainting/mask.rs` only if D1 requires a shared expansion correction.

Required deterministic cases:

| Case | Required assertion |
|---|---|
| glyph interior plus outline outside OCR bbox | refined/expanded component is retained outside recognition support but inside dynamic erase support |
| thin/thick stroke and bounded-shadow candidate | before color classification, candidate support covers all observed ink; thin/thick supported strokes continue into the strict source-style oracle, while shadow must finish as `UnsupportedSourceColor(Shadow)` with zero final erase/block/sprite pixels, byte-identical ROI, one warning, and RPC `CompletedWithErrors` |
| protected Latin adjacent, enclosing, and interleaved | final erase-mask overlap with protected support is exactly zero |
| adjacent non-target source node | its support remains zero in the final erase mask |
| two eligible nodes with disjoint 8-connected CTD components | after forbidden subtraction, each complete component has exactly one anchor/contact owner and the per-node union equals the aggregate selected support under both node orders |
| zero-owner CTD component | the component is dropped and never reaches color classification, expansion, or inference |
| one 8-connected component contacting multiple eligible nodes | the page fails before inference/blob/Scene mutation; the component is never distance-split or assigned by input order |
| page-edge target | every nonzero mask pixel remains inside page bounds |
| bubble present/empty/missing | canonical support remains bounded and backend-independent; each backend-specific final mask remains safe |
| zero eligible Han targets | existing no-op/zero-target cleanup behavior remains unchanged |
| eligible Han targets but no trustworthy component after forbidden subtraction | HanOnly returns an error before inference, Inpainted/Rendered blob creation, or Scene ops |
| repair region | existing explicit region clipping semantics remain unchanged |
| AllText | byte-for-byte mask behavior remains unchanged for the existing fixtures |

Scale each geometry fixture at 0.5x, 1x, 2x, and 4x. Assert normalized mask/support bounds and areas scale approximately with the input. Tolerance must be derived from raster quantization (`ceil`/`floor`), never a tuned sample-specific padding.

All three inpaint dispatchers must derive identical canonical target support, identical forbidden support, and zero protected overlap for the same fixture. Final-mask bytes are not required to match: AOT/Lama intentionally use glyph-only expansion while Flux uses region-fill expansion. Each backend-specific final mask must remain a subset of the same canonical allowed support after forbidden subtraction.

The shared inpaint-mask handoff must distinguish prepared mask, no eligible Han targets, and unsafe empty-or-ambiguous ownership after forbidden subtraction. Starting from CTD's aggregate `GrayImage`, production subtracts forbidden support, extracts stable-raster-order 8-connected nonzero components, and assigns a complete component only when its eligible anchor/contact owner set contains exactly one NodeId. Tests must prove no eligible Han targets keep the existing zero-target/no-inference behavior, zero-owner components drop, and nonzero eligible targets whose mask becomes empty or whose component has multiple owners fail before inference, blob creation, or Scene ops. This is a private implementation detail, not a new public API.

## T2. Dynamic layout region and maximum safe size

Add parameterized tests to `crates/koharu-app/src/renderer.rs` and `crates/koharu-app/src/pipeline/engines/renderer.rs`.

Test additions also include `ui/tests/components/RenderControlsPanel.test.tsx` and the new file `ui/tests/lib/io/openFiles.test.ts`. The former proves HanOnly automatic UI non-authority plus unchanged manual/AllText behavior; the latter proves the PNG/JPEG/WebP allowlist and input contract.

Revision 46/G002 staged seven pre-B1 RED tests. Revision 49 reclassifies only the two Source Gate / PP-OCR recall tests as B0/G004-owned; the remaining five stay B1-owned. At the start of G004 each of the seven still appears exactly once with an immediately adjacent `#[ignore = "hanonly-pre-b1-red"]`; no other test may use that reason, and no existing ignored test, whole module, or crate may be covered by it:

| Phase owner | Path | Exact test ID |
|---|---|---|
| B1 | `crates/koharu-app/src/renderer.rs` | `hanonly_pre_b1_red_t2_dynamic_layout_contract` |
| B1 | `crates/koharu-app/src/pipeline/engines/renderer.rs` | `hanonly_pre_b1_red_t2_pipeline_layout_handoff_contract` |
| B0/G004 | `crates/koharu-app/src/pipeline/engines/source_language_gate.rs` | `hanonly_pre_b1_red_t2_source_gate_ratio_contract` |
| B0/G004 | `crates/koharu-ml/src/pp_ocr_v5.rs` | `hanonly_pre_b1_red_t2_crop_local_ppocr_contract` |
| B1 | `crates/koharu-app/src/blobs.rs` | `hanonly_pre_b1_red_t2_blob_decode_budget_contract` |
| B1 | `crates/koharu-rpc/src/routes/pages.rs` | `hanonly_pre_b1_red_t2_replace_import_atomicity_contract` |
| B1 | `crates/koharu-app/src/pipeline/mod.rs` | `hanonly_pre_b1_red_t2_rotation_status_contract` |

Before B0 selection, each short ID is used only in one `--list --ignored` discovery that must resolve exactly one module-qualified libtest name whose final segment equals the ID; only that full name executes through `--exact --ignored`, and the log must prove exactly one started and failed test. Zero/duplicate matches, list/compile failure, a short-filter execution, setup failure before one test starts, a pass, or multiple tests cannot satisfy RED. The default workspace suite still passes. G004 then removes only the two B0/G004-owned ignore attributes, proves those two tests fail unignored under default execution, and implements Source Gate / PP-OCR recall until those two pass; the remaining five T2 ignores stay through B0. B1's first test-only hunk removes only the remaining five B1-owned attributes, proves those exact IDs still fail under default execution, and does not rename them; only then may B1 production hunks make them pass.

The runtime layout resolver must be tested with:

- landscape, portrait, square, and extreme aspect-ratio pages;
- horizontal and vertical writing;
- small, medium, and large source anchors;
- single line and multi-line translations;
- short, equal-length, 2x, and 3x translations;
- one node, grouped nodes, and irregularly distributed nodes;
- protected Latin adjacent, partially surrounding, and interleaved;
- page-edge anchors and insufficient free space;
- bubble present, empty, and missing;
- manual and automatic font sizes mixed;
- reversed input order and ten repeated runs.
- at least four decoded page-dimension bins, including a short side below 720, 720-1439, 1440-2159, and at least 2160 pixels;
- source crop candidates from at least four distinct decoded-source hashes, never only the approved image or its crops;
- nonzero rotation, which must follow explicit unsupported semantics in this bounded correction.

The resolver contract is exact:

1. Rasterize every finite Source transform and obstacle to a clamped integer half-open box using `floor(min)` and `ceil(max)`; reject non-finite, empty, or out-of-page target anchors.
2. Build the base allowed mask as page pixels minus protected Latin and non-target Source support. A nonempty connected bubble component is usable for a target only when it contains the complete target anchor box; a component may be shared by multiple targets. Missing, empty, or non-containing bubble evidence means the target domain is exactly the original source anchor mask. It never falls back to the full base page and never invents a padding box.
3. Within a trustworthy shared bubble only, assign every allowed bubble pixel to the eligible target domain with minimum Manhattan distance to that target's anchor box. Resolve ties by the stable key `(anchor.top, anchor.left, anchor.bottom, anchor.right, NodeId)`, never input order. In the anchor-only fallback no ownership expansion runs. Reject overlapping/degenerate anchors when each full anchor cannot remain owned by itself.
4. In each owner mask, enumerate maximal all-owned axis-aligned rectangles with the existing integer pixels using a row-histogram/monotonic-stack search. Keep only rectangles containing the full anchor whose rectangle center lies inside the anchor. Select by descending `(area, primary-axis extent, secondary-axis extent)`, then ascending Manhattan displacement between rectangle and anchor centers, then lexicographic `(top, left, bottom, right)`. Horizontal writing uses width as the primary extent; vertical writing uses height.
5. Convert the selected integer half-open rectangle directly to the one private runtime transform. If none exists, fail before raster/blob/Scene mutation. The owner mask is discarded after this reduction and is never used as a second fit contract.

On small generated masks, compare the selected rectangle with an exhaustive oracle implementing the same locality filter and score. This proves global maximality under the declared score, exact bubble fallback, stable tie-breaking, integer rounding, anchor locality, and input-order independence without sample-specific constants.

Required assertions:

1. recognition/source anchors are unchanged;
2. one per-target resolved layout is computed once into a crate-private HanOnly record/map and later embedded in the qualifying page's `PageId`-keyed frozen object; the exact public `EngineCtx` field set/re-export and by-value `Engine::run` signature plus public `RenderBlockInput`, `PageRenderOptions`, and `Renderer::render_page(...)` remain unchanged, while existing `source_transform` and `transform` retain their meanings;
3. both local fit and post-render validation consume the exact same resolved axis-aligned box;
4. runtime layout regions are inside the page and contain no protected/non-target/other-owner pixels;
5. regions are deterministic and input-order independent;
6. horizontal regions expand preferentially on the horizontal axis; vertical regions on the vertical axis;
7. every non-transparent sprite pixel lies in its dynamic safe region;
8. no sprite pixel overlaps protected Latin, other source nodes, or another target sprite;
9. the resolved-rectangle center and final nontransparent sprite center both lie inside the original source anchor;
10. the backend exclusively computes `G/Csrc/S0/Smax` from the resolved layout in the crate-private HanOnly record: for horizontal writing `cross_extent=resolved_region.height`, for vertical writing `cross_extent=resolved_region.width`, and `G=floor(min(page_width,page_height,cross_extent))` with checked finite positive inputs/conversion; `G` is only a geometry-derived search ceiling, no `GLOBAL_CAP_PX` or renamed fixed pixel cap exists in the HanOnly automatic backend dataflow, and `Csrc=min(source_estimate,G)`, `S0=largest_fit(Csrc)`, and `Smax=largest_fit(G)` use the same region/font/writing/stroke/effect/padding/raster predicate so source estimates below/equal/above `G` assert `S0<=Smax`;
11. the backend exclusively computes every group value: `G0=min(S0_i)` and `G1=min(selected_i)`; every `selected_i>=S0_i`, proving `G1>=G0`;
12. manual sizes remain excluded from automatic grouping and remain authoritative subject to existing safety checks;
13. 0.5x/1x/2x/4x equivalent geometry produces approximately proportional regions, sizes, alpha bounds, and locality;
14. no fit at the readability floor fails before blob/Scene mutation;
15. production Source Gate no longer selects absolute C2; Revision 50 candidates are predeclared as `S25L4`, `S25L5`, `S25L6`, and `S25L7`, each using `padding=max(short_side*1/4,long_side*L)` with `L=1/25,1/20,3/50,7/100`, followed only by raster quantization and page clipping;
16. exactly four non-regression calibration hashes select the smallest candidate passing the recomputed Source Gate target/protected/rotation oracle on validator-derived CPU and actual Metal, the raw load/node/executable/model evidence and result are frozen in `crop-policy-selection.json`, and exactly four disjoint holdout hashes pass the same two-device oracle without build/model drift or retuning; `test.jpeg` is regression-only and the five existing crops sharing one source hash count as one source;
17. the UI receives none of `G/Csrc/S0/Smax/G0/G1` and performs no source/OCR-box, prediction, detection, or fallback automatic pixel estimation. For HanOnly with no manual size, `RenderControlsPanel` displays the existing Auto state with no numeric value; `eligibleSourceLayout`, `automaticSourceSize`, and `groupedAutomaticSourceSizes` plus their `-5px`, `12..28`, `72px`, and source-box cap/deduction are absent from the HanOnly automatic path. Manual-size and AllText behavior remain unchanged; UI/backend parity is asserted only for PNG/JPEG/WebP allowlists;
18. a validated same-node transient reflow may change target line count without changing original translation bytes or OCR ownership; unvalidated mismatches still restore source safely;
19. nonzero rotation carries private reason `Rotation`, leaves the source ROI unchanged, emits no rendered block/sprite or partial persistence, reaches the existing warning sink exactly once per node through `ctx.warn` before engine run or the same `EngineWarningSink` after apply, uses message prefix `han_only.unsupported_rotation:`, increments `warning_count`, and yields RPC `CompletedWithErrors`; a same-page non-rotation unsupported node remains tracing-only.

For locality, the alpha bbox is the smallest integer half-open page box containing every nonzero-alpha pixel. Its center is `((left+right)/2, (top+bottom)/2)` in page coordinates and is inside the source anchor only when `anchor.left <= cx < anchor.right` and `anchor.top <= cy < anchor.bottom`. Rectangle centers use the same half-open containment rule.

Keep the existing backend tests for detected-first English, prediction/detection source-estimate ordering, grouping, source-relative one-raster behavior, effect ink bounds, page overflow, protected overlap, other-node overlap, target overlap, and atomic failure. Replace backend `-5`/fixed-cap assertions with zero blanket deduction and backend-only geometry/fit tables. Replace UI `-5`/cap assertions with absence of HanOnly automatic pixel estimation plus Auto/empty-state tests; keep separate PNG/JPEG/WebP allowlist parity tests. Replace the tight-source-bbox invariant only with the dynamic-safe-region invariant.

GREEN-B, before either replacement route is accepted as atomic, must implement the single shared staged `Op::Batch` plus whole-log copy-on-write History commit boundary under item 20. Acquire History then Scene before the permit, capture coherent `expected_epoch` and read-only staging inputs, release Scene then History, finish async/body/decode work, apply the complete nested Batch only to staged Scene/op clones, and compute staged next epoch/undo/redo plus the complete serialized frame. Then acquire the one persistence permit and recheck poison only; promote all Blob objects through permit-aware item 19 without History/Scene locks; release Blob-local/cache locks; acquire History exactly once within the permit-held publication boundary and perform exactly one `current_epoch == expected_epoch` comparison; then acquire Scene and revalidate required state. A stale epoch at that sole post-promotion check leaves only exact verified unreachable CAS, poison false, and zero Scene/History/success publication. Read the current canonical old generation once into immutable bytes and parse it with existing compatible startup semantics: accept complete decodable frames and ignore only a malformed or truncated trailing frame. Record exact raw bytes and `old_valid_log_bytes`, the accepted last-complete-frame prefix. Write solely `old_valid_log_bytes + new_frame` to one same-directory task-owned `create_new` temp; flush and `sync_all`; then strictly replay that immutable temp to exact EOF from a separate fresh clone. Exactly the new next-epoch frame must apply once, and replayed Scene/epoch must equal staged Scene/next epoch. The adapter flushes/syncs and relinquishes the old writer before replacement. On Unix, `Committed` requires rename, parent-directory sync, exact-new canonical bytes, no temp, and a synced reopened append writer. On Windows, `Committed` requires rename, canonical-file sync, exact-new bytes, no temp, and a usable append writer. `Unchanged` requires exact raw-old bytes, no temp, and a synced reopened old writer. Every other observation is fatal `Indeterminate`; its caller stores `ProjectSession.persistence_poisoned` while still holding the permit and returns exactly `project persistence is indeterminate; close and reopen the project`. Release Scene then History; only `Committed` may synchronously publish response/Dirty/autosave/job success before permit release. Read-only diagnosis/export remains available and gate-free. Pipeline poison is `Failed`; autosave logs once and exits; explicit close joins autosave, skips final writes, releases the lock, and only a later strict-recovery open constructs an unpoisoned session. Restart uses strict-first disposable-clone replay and only legacy-tail fallback on a second fresh clone. No staging guard may cross permit acquisition; no permit-held pre-Blob History acquisition or epoch read/validation, additional permit-held History phase, inverse/truncate rollback, generation concatenation, noncanonical-temp recovery, partially mutated probe reuse, guessing, in-process poison reset, `.await` under permit, inner-lock-to-gate acquisition, or double apply is permitted. Both replacement routes must finish validation/staging before the gate, then use one permit through CAS, History, memory, and synchronous success publication. GREEN-C marker/style validation reuses this boundary and adds no second algorithm. The existing `hanonly_pre_b1_red_t2_replace_import_atomicity_contract` absorbs common and Unix adapter cases without adding a staged ID; ordinary default-feature `history::tests::windows_durable_replace_generation_contract` runs on real `windows-2022` and proves the History and Blob/CAS Unix/Windows matrices, old-writer/related-handle release, overwrite replacement, exact-new verification, canonical sync, append continuity, failed-replace exact-old `Unchanged`, poison lifecycle, strict/legacy-tail restart, and no task temp.

## T3. Typography Planner and style precedence

Add/adjust tests in `crates/koharu-app/src/typography.rs`, `crates/koharu-app/src/pipeline/engine.rs`, `crates/koharu-app/src/pipeline/mod.rs`, `crates/koharu-app/src/pipeline/engines/typography.rs`, `crates/koharu-app/src/renderer.rs`, `crates/koharu-app/src/pipeline/engines/renderer.rs`, `crates/koharu-core/src/op.rs`, `crates/koharu-app/src/history.rs`, `crates/koharu-app/src/blobs.rs`, `crates/koharu-app/src/session.rs`, `crates/koharu-app/src/app.rs`, `crates/koharu-app/src/autosave.rs`, `crates/koharu-rpc/src/routes/pipelines.rs`, `crates/koharu-rpc/src/routes/history.rs`, `crates/koharu-rpc/src/mcp/mod.rs`, and the existing untrusted archive-import tests in `crates/koharu-rpc/src/routes/projects.rs`. The exact direct-field caller migration/test closure is `crates/koharu-app/src/session.rs`, `crates/koharu-app/src/app.rs`, `crates/koharu-app/src/ai.rs`, `crates/koharu-app/bin/pipeline.rs`, `crates/koharu-app/src/pipeline/mod.rs`, `crates/koharu-rpc/src/binary.rs`, `crates/koharu-rpc/src/mcp/mod.rs`, `crates/koharu-rpc/src/psd_export.rs`, `crates/koharu-rpc/src/routes/pages.rs`, `crates/koharu-rpc/src/routes/projects.rs`, `tests/integration-tests/tests/binary.rs`, `tests/integration-tests/tests/pipelines.rs`, and `tests/integration-tests/tests/scene.rs`; the ordinary lifecycle/source-policy contract also audits same-module `ProjectSession::open_untrusted`. `history.rs` is an expected production edit with colocated unit tests, not a test-only addition. Integration/source-policy exclusively owns its adapter/tests together with `blobs.rs` bound control/durable promotion, whole-file `session.rs`/`app.rs`/`autosave.rs`, every permit/accessor/field-encapsulation/direct-caller migration, the 13-file closure, `pipeline/mod.rs` admission/gated transaction/fatal propagation, and `routes/pipelines.rs` preflight/job behavior. `ProjectSession.scene/history/blobs` become private; their removal is an explicit downstream-breaking Rust helper/API change, while existing request/schema, Scene, Engine, `EngineCtx`, Renderer, and BlobStore public put signatures remain unchanged. `routes/pipelines.rs` owns the pre-job HTTP region guard, shared admission before conditional Registry preflight, unsupported job lifecycle, and admitted synchronous unknown-ID 400 while preserving request/options/schema/conversion shapes. `pipeline/engine.rs` receives only unchanged-public-contract and producer/order tests; `pipeline/mod.rs` exclusively owns the first-executable direct-run region guard, the explicit additive downstream-public `#[doc(hidden)] pub` admission helper, plus the crate-private task-local key/state/accessor/scope and its lifetime/isolation tests.

The following nine tests are the complete pre-GREEN-C RED inventory. At `B0_SHA` and throughout B1 each appears exactly once with an immediately adjacent `#[ignore = "hanonly-pre-greenc-red"]`; no other test may use that reason:

| Path | Exact test ID |
|---|---|
| `crates/koharu-app/src/typography.rs` | `hanonly_pre_greenc_red_t3_transient_planner_hint_contract` |
| `crates/koharu-app/src/pipeline/mod.rs` | `hanonly_pre_greenc_red_t3_run_state_lifetime_contract` |
| `crates/koharu-app/src/renderer.rs` | `hanonly_pre_greenc_red_t3_planner_font_outcome_contract` |
| `crates/koharu-app/src/renderer.rs` | `hanonly_pre_greenc_red_t3_source_color_contract` |
| `crates/koharu-core/src/op.rs` | `hanonly_pre_greenc_red_t3_marker_batch_atomicity_contract` |
| `crates/koharu-app/src/session.rs` | `hanonly_pre_greenc_red_t3_untrusted_marker_lifecycle_contract` |
| `crates/koharu-rpc/src/routes/history.rs` | `hanonly_pre_greenc_red_t3_http_marker_rejection_contract` |
| `crates/koharu-rpc/src/mcp/mod.rs` | `hanonly_pre_greenc_red_t3_mcp_marker_rejection_contract` |
| `crates/koharu-renderer/tests/rendering.rs` | `hanonly_pre_greenc_red_t3_source_color_probe_contract` |

Before B0 selection, each short ID is used only in one `--list --ignored` discovery that must resolve exactly one module-qualified libtest name whose final segment equals the ID; only that full name executes through `--exact --ignored`, and the log must prove exactly one started and failed test. GREEN-C's first test-only hunk removes only these nine attributes, proves the same IDs still fail under default execution, and does not rename them; only then may GREEN-C production hunks make them pass. Extend the existing run-state lifetime staged test without renaming it to lock the exact public `EngineCtx` fields/re-export and by-value `Engine::run` shape; require `pipeline::run` with `spec.options.region.is_some()` to return one deterministic error as its first executable validation before registry/order/page/Scene/run-state/engine/warning/blob/history access; require current-page task-local access to fail before/after scope and for wrong/prior/post-run pages; builder/inpainter/Renderer to observe `Arc::ptr_eq`, one current-page frozen-object identity, and the same frozen sprite identity; sequential/concurrent runs to remain distinct; nested same-task scopes to shadow then restore; `tokio::spawn` not to inherit access; no state-derived mutable handle to outlive the awaited scoped future; and all page records to drop with the run Arc. For each builder injection it snapshots pre-upstream state, commits representative upstream producers, captures `B_prepare`, proves final Scene/History/reachable-blob/sprite state equals `B_prepare` and differs from the pre-upstream snapshot where a producer commit was expected, and proves no producer commit was rolled back. It also records retained logical bytes separately from actual raster scale/dimensions/surface bytes, enforces peak live scratch `<=1`, and proves per-target scratch release on success and either builder failure class. Its two-page `Scope::WholeProject` table covers one inpainter without Renderer, one with Renderer, and page-one inpainter failure after publication followed by zero sprite/blob persistence, pixel-payload release, and independent page-two success. It also proves successful Renderer/no-Renderer terminal paths release pixel payloads and no sprite bytes accumulate across pages while small immutable diagnostic metadata may remain until run drop. UI RED tests are added after B1 and before the corresponding production hunk, proved RED normally, and never use Vitest `.skip` as a substitute for this Rust inventory.

Extend the existing staged T3 source-color-contract test with every zero/one-producer engine-spec permutation, ambiguous multiple-producer rejection before engine load, and the total scheduling matrix for accepted `spec.options.region.is_none()` runs: present font/Typography producers precede builder/inpaint; no Renderer/no inpainter is valid with zero builder/reservation/raster/publication; no Renderer/one inpainter is valid with exactly one complete builder publication immediately before inpaint; Renderer/no inpainter rejects before engine load with zero builder/publication; Renderer/one inpainter uses exactly one such publication and both inpaint and Renderer consume the same frozen object/sprite identity. Region-bearing direct runs are rejected before this matrix, and the direct `pages.rs` repair route never satisfies or violates a matrix row. The builder must prove pre-inpaint translation/layout/raster, complete canonical-target retained logical-sprite reservation, independently checked actual-scale raster arithmetic, serial one-target scratch lifetime/release, `B_prepare` capture/equality and upstream-commit retention for both builder failure classes, atomic publication, exactly one raster invocation per successful target, and failure before inpainter `engine.run`/`app.apply`; Renderer must have zero translation rebuild, layout, shaping, glyph, raster, fit, group, Planner-size, rounding, and width calls. Also prove the exact P-1/P0/P1/E formula and term tables, including `Wterm=J+T`, total `Wpair=choose2_checked(T)`, `Wmeta=checked(Wterm+Wpair)`, `P_i=4*A+19*X_i+81_920`, `Wfill=1_028*N+3`, and `Wstroke=3_071*N+12*M+7`; `T=0/1/2`, maximum successful `choose2_checked(6_074_001_000)`, overflow at `6_074_001_001`, separate `Wterm+Wpair` addition overflow, and all other `J/A/X/T` checked add/multiply boundaries; all six phase-specific P0 requested/reserved/consumed/target-record tuples; every page evaluation/preflight/total checked sum; P1 exact requested/reserved upper-bound versus actual consumed semantics; `|B|<8X_i`, `H=1`, `H=64`, `H=65`, and semantic early-stop cases with independently counted `consumed<reserved`; injected padding/no-op/synthetic counter inflation hard failure; preflight exact-bound/bound-plus-one based on requested/reserved rather than consumed equality; `H=64/K=4096`; `H=65/K=4225` selecting `core_count`; arithmetic-before-target-before-page precedence; exact requested/reserved/consumed null/zero rules; identical final page totals under reversed input order; empty/nonopaque `Q/B`; proof that P1 performs no membership/P95/order/`bg(q)`/candidate work and E rejection leaves all such counters/allocations zero; one shared immutable `bg(q)` table under reversed candidate order; same-bin P95 ranks for sample sizes `1/2/19/20/21/100`; integer `+127` blend boundaries; all-channel residuals; exact pair/compatibility order; cross-pair minimum-residual fail-closed ties; unique shortest path, same-core multipath, cross-core tie, and unreachable-core rejection; topology unique/no/multiple-width intervals; absence of `U*M` enumeration; `F_t` rounding at `0.49/0.50/0.51/1.49/1.50`, nonfinite/nonpositive/overflow, one frozen Planner/group outcome, pre-erase `W_t`, no partial map/sprite publication, hard geometry/font pre-write failure, unsupported color/width preservation, and zero Renderer rebuild; exact canonical-JSON digest agreement between Rust and TypeScript plus wrong-key/type/float/array-order/escape/whitespace/newline/formula-drift/P0-terminal/page-accounting negatives; visual-manifest/preimage separation; the one exact complexity warning; accounting-invariant hard abort; and elapsed-verdict independence. Process-level allocator OOM/abort is never represented as a recoverable test verdict. This does not add or rename a staged ID.

Extend the two existing staged source-color contract/probe tests listed above: the app contract gains the `J -> T -> Wterm -> Wmeta` P-1/P0 phase table, expected/frozen/pre-inpaint-raster-entry translation-digest chain, `B_prepare` plus upstream-retention proof, complete-target retained logical-sprite reservation, independently checked actual-scale raster dimensions/bytes, peak-live-scratch and release proof, atomic publish, one builder raster per successful target, and zero Renderer raster; the renderer probe exercises the same shared raster path with insertion-only break reconstruction, cluster-interval/control-byte coverage, glyph-zero `.notdef`, a nonzero out-of-range shaped glyph that errors rather than skips, multi-glyph fill-pass traversal, missing-glyph rejection, nonempty-alpha validation failure, logical-versus-raster arithmetic, fallible surface-construction failure, serial scratch lifetime, and transcript digest stability. Low-level conversion remains defense in depth. The tests construct through existing public layout/renderer types but add no public production evidence API. Existing `han_only_visual_manifest_matrix` and `han_only_visual_runtime_matrix` consume the private test transcript for real targets. The sixteen staged IDs and three ordinary work-budget IDs are not added, removed, moved, or renamed.

The following ordinary, non-ignored platform/lifecycle tests are additive and are neither staged IDs nor source-color work-budget IDs:

- existing `history::tests::windows_durable_replace_generation_contract` is extended in place to cover Windows History and Blob/CAS handle release, overwrite rename, canonical-file sync, exact bytes/hash, temp absence, and append continuity;
- new `session::tests::persistence_poison_lifecycle_contract` covers apply/apply-if-epoch/undo/redo/snapshot/compact/autosave, public/direct Blob puts, RPC page import/mask repair, pipeline sprite/final-CAS promotion, History commit, synchronous response/Dirty/autosave/job publication, permit/control identity, every former public-field caller, read-only allowance, poison-aware close, lock release, and strict close/reopen recovery; barriers/channels prove poison-wins-before-gate and permitted-publication-wins-before-poison orderings without sleep/time;
- new `routes::pipelines::tests::backend_admission_preflight_contract` covers the pure table, unsupported zero-Registry job path with one direct-run warning/`CompletedWithErrors`, direct-run duplication guard, and admitted unknown-ID synchronous HTTP 400.

The exact Windows prerequisite command remains `cargo test -p koharu-app --lib history::tests::windows_durable_replace_generation_contract -- --exact --nocapture`; the job/test ID is not renamed and no second required context is added.

The same IDs now prove four additional boundaries. First, every ink-bearing shaped cluster/run records a nonzero contribution to the final logical-sprite alpha after clipping/downsample; ligatures and combining marks remain one shaped contribution group, and only enumerated legal controls/whitespace may contribute zero. Second, the frozen object binds immutable sprite bytes, normalized `sprite_transform`, and `rendered_direction`; Renderer is their sole writer and persists all three with final Rendered output in one staged commit while preserving `Node.transform` and source/OCR geometry. Third, peak-live accounting reserves the sum of every simultaneously live retained sprite, Pixmap, unavoidable copy, downsample output, mask, staged Scene/blob, and transaction buffer before any allocation; fault tests cover each term and remove or make fallible the current full-surface copy path. Fourth, the private typed failure discriminant is asserted for every injected cause while the public result remains exactly `geometry_or_font`.

Extend existing `hanonly_pre_greenc_red_t3_run_state_lifetime_contract` without renaming it with a public-output equivalence mode. The final matrix runs it once without `hanonly-test-evidence` and once with the feature on the same deterministic in-memory input, writes closed public-only reports, and compares exact Scene bytes, reachable blob bytes/hashes, Rendered RGBA, status, warning order/text, sprite transform, rendered direction, and public error token. Only feature-gated diagnostic fields may differ; any public difference proves the feature changed behavior and fails acceptance.

The same existing run-state contract and ordinary `routes::pipelines::tests::backend_admission_preflight_contract` prove one shared pure `backend_admission(source_text_policy, cpu, target_os)` table. AllText is admitted for both CPU flags. HanOnly explicit CPU is admitted for macOS, Windows, Linux, and `other`; HanOnly non-CPU is admitted only for macOS. The HTTP ingress runs region validation and shared admission before request-step Registry preflight. For an unsupported request it records zero Registry calls, still creates one job, and delegates the sole warning to direct `pipeline::run`; that second shared check runs before `infos_for_spec` and returns exactly one stable warning, `warning_count=1`, and `CompletedWithErrors` with zero page/run-state/model/engine/Scene/blob/History/erase/render effects. The route precheck emits no warning. For an admitted request, an unknown step ID still fails synchronously with the existing HTTP 400 before job/cancel/event/task creation. Direct `pipeline::run` tests independently prove unsupported short-circuiting even without RPC. Requested Metal on macOS proceeds to the existing actual-Metal T6 boundary; no test self-reports an actual backend before model load.

After B1 and before the first GREEN-C production hunk, add and prove these ordinary non-ignored tests RED under normal execution:

| Layer | Exact test ID | Required proof |
|---|---|---|
| Integration | `han_only_source_color_work_budget_pipeline_contract` | P0 admission/terminalization, P1 target/page rejection, E page rejection, and full admission all finish before their authorized traversal/allocation; no candidate allocation occurs before E; every emitted record has exact null/zero fields and identical final page totals; checked retained logical-sprite area/byte/`usize`/page-cap exact/overflow plus actual-scale raster width/height/byte/`usize` exact/overflow and fallible-surface rejection finish in builder before inpainter run/apply, preserve `B_prepare`, retain upstream commits, release scratch, and record one pre-inpaint raster trace with zero Renderer raster |
| End-to-end | `han_only_source_color_work_budget_completed_with_errors` | every budget rejection preserves Source/Inpainted/Rendered ROI equality, has zero erase/block/sprite, exact one-warning dedupe, and `CompletedWithErrors`; `p0_metadata_unbounded` instead hard-fails with no target record or mutation |
| Stress | `han_only_source_color_work_budget_adversarial_stress` | generated high-core, high-candidate, large-`U`, arithmetic-overflow, and aggregate-page cases terminate through checked accounting; reversed target/input order yields identical non-time diagnostics, final page totals, reasons, and pixels; elapsed time has no authority and no `U*M` width work exists |

GREEN-C requires these three IDs plus the staged source-color-contract ID to pass. None may use `#[ignore]`, `.skip`, elapsed thresholds, external images, or fixture-specific constants. The frozen sixteen-ID matrix remains exactly unchanged.

Planner line cases:

| Case | Result |
|---|---|
| originally single-line translation, one safe region, insertion-only line breaks | a fresh per-run HanOnly hint reaches local validation; deleting only inserted `\n` bytes reconstructs the original exactly; Scene translation/style/marker remain unchanged |
| multiple Source lines or safe regions | original translation line ownership is preserved |
| replacement/removal/reordering of any original byte, original multiline text, or non-reversible whitespace | local original-translation fallback; pipeline continues |
| candidate fits dynamic region | planned line breaks render |
| candidate fails local raster fit | original translation is restored and locally auto-laid out |
| both candidate and fallback fail | atomic no-fit error before persistence |
| Planner timeout/provider/strict JSON failure | existing warning plus no HanOnly runtime hint and zero HanOnly Planner ops; local render continues |

Planner font-size cases:

- Every HanOnly Planner `fontSize` value is normalized to an integer and parsed into a transient per-field outcome. `G=floor(min(page_width,page_height,cross_extent))`, where `cross_extent` is resolved-region height for horizontal writing and width for vertical writing; all inputs and conversion are checked finite positive values. `G` has no fixed global pixel cap. `Csrc=min(source_estimate,G)`, `S0=largest_fit(Csrc)`, and `Smax=largest_fit(G)` use the exact same resolved region, font, writing mode, stroke/effects/padding, and raster-fit predicate; tests cover source estimates below, equal to, and above `G`.
- Manual size present: `ignored_by_policy_manual`; manual behavior stays authoritative.
- Nonfinite, nonpositive, or globally out-of-range `p`: `rejected_unsafe`; final automatic fallback is `S0`.
- `p < S0`: `ignored_by_policy_would_reduce`; final size is `S0`.
- `S0 <= p <= Smax`: `accepted`; selected independent size is `p`.
- `p > Smax`: `rejected_unsafe`; final size is `S0`; silent clamping is forbidden.
- Group-min uses `G0=min(S0_i)` and `G1=min(selected_i)` and asserts `G1>=G0`.
- All outcomes are observable in test diagnostics; none writes a HanOnly Scene style/marker or claims `typography_plan_verified`.
- AllText retains its existing behavior.
- Manual size remains highest priority.
- HanOnly source-estimate precedence remains unchanged; source estimates above geometry-derived `G` are intentionally bounded by `G`, the blanket `-5px`, `GLOBAL_CAP_PX`, and any renamed fixed cap are absent, and the exact `G0/G1` group proof replaces the prior informal cap statement.

Style precedence cases:

1. The complete HanOnly Planner hint (`original_translation`, reversible proposed lines, proposed style, proposed font size, and per-field outcomes) lives only in the crate-private task-local `Arc<PipelineRunState>` keyed by page/node. The task-local accessor is unavailable outside each related awaited current-page engine scope; the sole builder's direct private Arc/`PageId` handoff is the only exception. All hints and frozen page records drop after that run, produce no Scene op, and never set `typography_plan_verified`.
2. After D3 proves the current-version lifecycle and finds no ambiguous baseline, a HanOnly stored style with `typography_plan_verified=false` is the explicit/manual source and wins for font, color, stroke, effect, and alignment under `manual_override`. A stored style with `true` remains Planner-owned and is ignored entirely as a HanOnly render-style input: it sits in no fallback tier and must not affect output. A focused regression supplies marked black/no-stroke style and proves the automatic path derives the same source-only `SourceColorContract` when `fontPrediction` is present, absent, delayed, or contradictory. If source color cannot be represented by one opaque solid fill plus zero or one opaque solid stroke, the node becomes `unsupported_source_color`; prediction, a same-run Planner hint, default black, and contrasting stroke may not turn that result into a rendered target. The same fixture proves AllText behavior is unchanged.
3. A shared raw-JSON parser rejects `typographyPlanVerified` whenever the field is present at the exact forward-authoritative pointers `/addPage/page/nodes/<id>/kind/text/typographyPlanVerified`, `/addNode/node/kind/text/typographyPlanVerified`, or `/updateNode/patch/data/text/typographyPlanVerified`, for both `true` and `false`, before typed deserialization. It recurses through `/batch/ops/<index>` at arbitrary tested nesting depth and explicitly ignores the actual overwritten inverse paths `/removePage/prev_page`, `/removeNode/prev_node`, and `/updateNode/prev`. Tests derive representative JSON by serializing each `Op`, then mutate those exact keys; marker omission is the only accepted external authoritative shape. Generated schemas may still expose the full Scene/patch field for serialization compatibility, but tests and documentation must make the external apply boundary runtime-restrictive rather than accepting explicit marker values. If source comments are changed for OpenAPI clarity, `bun run check:generated` must show only the normal regenerated artifacts, with no added or removed marker field.
4. A private marker guard runs inside actual `Op::apply` before any non-Batch mutation and is shared with `Op::validate`; direct `History::apply` therefore cannot bypass it. `Op::Batch` must be atomic even when an early child is valid and a later child fails: a shared private staging helper clones the input `Scene` and the Batch ops, applies every cloned child sequentially through the full real apply path against the cloned Scene, and recursively stages nested Batch children. Only complete success assigns both the staged Scene and the now-populated staged ops back to the real arguments. `Op::validate` invokes the same staging helper on clones and discards the result. Marker, structural, inverse-population, and nested-Batch errors therefore mutate neither the real Scene nor the caller's ops.
5. A non-Planner text/translation/style patch applied while a persisted style is Planner-owned clears that style when no replacement style is supplied, or atomically replaces it with the supplied manual style; it always clears the marker. `capture_prev_text` must capture the implicitly changed style and marker so undo restores and redo re-invalidates the exact state. A trusted marker-only `false` against marked state follows the same capture/clear rule.
6. A trusted persisted Planner install or full-node internal add may set `true` only with a nonempty style in the same patch/node. A marker-only `true` fails on actual apply as well as preflight. Trusted undo may restore `true` only with the captured Planner style; redo must reproduce invalidation.
7. Renderer output patches omit both style and marker when rendering does not change them; Renderer never asserts or clears trust.
8. Untrusted project opening clears both persisted Planner style and the marker before compaction, while preserving translation bytes. Test the existing import route as well as `ProjectSession::open_untrusted`.
9. `typography_plan_verified=true` means only that the currently stored Scene style came from the last atomically validated persisted Planner op or trusted inverse restore; it does not attest visual quality, final line acceptance, final font size, or a HanOnly runtime hint. Marker `false` without the audited current-version transition and style presence is not evidence of provenance.
10. Usable `fontPrediction` font identity, direction, finite positive font-size evidence, and effect metadata may beat conflicting transient HanOnly Planner values for those non-color fields. Predicted fill/stroke RGB and stroke width remain consistency diagnostics only and never select automatic render bytes.
11. Transient Planner font/effect/alignment may fill locally validated missing non-color fields. Planner fill/stroke values are recorded as ignored-by-policy diagnostics for non-manual HanOnly and never enter `AutomaticStrict`.
12. Defaults apply only to non-color fields after manual, usable prediction, and validated transient Planner sources are unavailable. Automatic fill/stroke have no default or contrasting fallback: they come exactly from `SourceColorContract`, or the node is preserved as `UnsupportedSourceColor`.
13. AllText keeps its existing request, Scene-op, line validation, font-size, and render behavior; add an explicit AllText before/after regression alongside the new marker lifecycle tests.

Color-contract cases:

- For accepted normal HanOnly execution inside `pipeline::run`, exactly `spec.options.region.is_none()`, zero or one selected producer is permitted for each of `FontPredictions`, `TypographyStyles`, and `Inpainted`; multiple selected producers for any ordering-critical artifact fail before engine load. Every registered engine's static `needs`/`produces` descriptor remains byte-for-byte unchanged. `pipeline::run` adds private HanOnly-only conditional edges and omits absent terms: with both optional producers, `font < typography < builder-layout/raster/freeze < inpainter < renderer-persist/composite`; with only font, `font < builder-layout/raster/freeze < inpainter < renderer-persist/composite`; with only Typography, `typography < builder-layout/raster/freeze < inpainter < renderer-persist/composite`; with neither, `builder-layout/raster/freeze < inpainter < renderer-persist/composite`; omit the Renderer term when Renderer is absent. AllText uses the unchanged global DAG. The total count matrix is authoritative per page: no Renderer/no inpainter is valid with zero builder/reservation/raster/publication; no Renderer/one inpainter is valid with exactly one complete `PageId` frozen-sprite publication immediately before inpaint; Renderer/no inpainter rejects before engine load with zero builder/publication; Renderer/one inpainter is valid only with that one current-page publication and same-object/sprite consumption inside one `B_prepare`-scoped transaction. The direct Segment route in `crates/koharu-rpc/src/routes/pages.rs` is outside this matrix only when its pre-side-effect guard proves `role == MaskRole::Segment`, `Registry::find(params.pipeline)` succeeds, and `engine_info.produces == &[Artifact::Inpainted]`. It performs that guard before body decode/blob persistence, Scene read/snapshot/apply, engine cache/load/run, or `app.apply`; unknown IDs, non-Segment roles, and any descriptor producing Renderer/Typography/other artifacts return one stable HTTP 400. An accepted repair retains the direct region-bearing `EngineCtx` path, performs zero builder/publication, and bypasses task-local access before lookup. Missing task-local state is not a repair fallback. `PutMaskParams.pipeline`, `StartPipelineRequest.region`, `PipelineRunOptions.region`, serialization, and conversion shapes remain; comments and localized HTTP API docs state that `POST /pipelines` rejects non-null region and directs localized repair to `PUT /pages/{id}/masks/{role}?pipeline=...`; the route documents HTTP 400 and normal OpenAPI regeneration updates generated documentation without adding/removing the nullable field. The exact public `EngineCtx` field set, crate-root re-export, and by-value `Engine::run(&self, EngineCtx<'_>)` signature remain unchanged; `PipelineRunState` is never added to `EngineCtx`, `PipelineRunOptions`, `RenderBlockInput`, `PageRenderOptions`, Scene, OpenAPI, or another public carrier.

  The current-page builder snapshot is one coherent pre-permit History-then-Scene snapshot: immediately after selected upstream producer commits and before builder work, capture authoritative `expected_epoch` plus the read-only Scene/History inputs defining `B_prepare`, then release Scene followed by History before any permit acquisition.

  Existing `pipeline/mod.rs` exclusively defines one crate-private `tokio::task_local!` key and `PipelineRunState`, creates exactly one `Arc<PipelineRunState>` per accepted run, stores immutable objects by `PageId`, and scopes every related awaited Typography, selected inpainter, selected Renderer, and feature-gated AOT-observation future with the run Arc plus current page around `engine.run(ctx)`. Production access is only through a crate-private non-panicking closure accessor using `try_with`; it errors before/after scope, for wrong or completed-prior pages, and after run completion. The only complete builder is crate-private `pipeline/engines/renderer.rs::prepare_and_freeze_hanonly_render_inputs(...)`; on a one-inpainter page `pipeline/mod.rs` passes the same Arc and `PageId` directly and calls the builder from the same page snapshot immediately before the selected inpainter, and on a no-inpainter page it is not called. Immediately after selected upstream producer commits and before this call, capture `B_prepare` as observable Scene bytes/epoch, History epoch/canonical-log bytes, persisted sprite inventory, and canonical `reachable_blob_state(B_prepare)` rooted in current Scene, committed/replayable History ops, and current undo/redo trees. This direct builder handoff is the sole non-accessor exception. The builder owns final resolved layout, `G/Csrc/S0/Smax/G0/G1`, final font/effects, Planner outcome, `F_t/W_t`, source-color mode, expected/frozen/pre-inpaint-raster-entry translation digests, byte/scalar counts, insertion-only break transcript, cluster/control transcript, glyph/fill evidence, nonempty alpha, retained logical-sprite reservation, actual raster scale/dimensions/surface bytes, serial scratch lifetime/release, immutable sprite bytes/placement, and final safety. The test-only evidence harness separately correlates those guarded records to its random IDs; no production run-state or renderer field stores an ID. The builder, selected inpainter, and selected Renderer must observe `Arc::ptr_eq`, the same current-page frozen-object identity, and the same frozen sprite identity. A failure from either builder class before publication creates no page entry, has zero inpainter run/apply, and leaves final Scene/History/reachable-blob/sprite state equal to `B_prepare` while retaining all upstream commits already present there; an inpainter failure after publication skips Renderer, persists zero sprite/blob bytes, and releases the pixel payload before independent page-two work. Successful Renderer or valid no-Renderer completion also releases its pixel payload; no sprite bytes accumulate across pages. Transient raster scratch is released after each target success or either builder failure class, with at most one target scratch surface live. Concurrent invocations use distinct Arcs; nested same-task invocations shadow then restore the outer Arc; `tokio::spawn` does not inherit it; no Arc/state-derived mutable handle escapes; and all remaining records drop at run completion. After `Inpainted`, a selected Renderer passes the same frozen sprite to a crate-private HanOnly entry in existing `crates/koharu-app/src/renderer.rs` and persists/composites it exactly once with zero layout, shaping, glyph, raster, fit, group, Planner-size, rounding, or width work. It must not add fields to or repurpose any public engine/render carrier or method. AllText keeps its existing public renderer-time path. A missing, partial, duplicate, wrong-page, inconsistent, or matrix-forbidden publication, repair accessor use, post-run survival, pixel-payload leak, scratch accumulation, public shape drift, stale behavior documentation/generated artifacts, scope leak, cross-page/run contamination, spawned access, escaping mutable handle, and every Renderer layout/raster/recomputation invocation are hard failures.
- **Complete target-text raster proof.** Before Planner mutation, independently hash the exact Scene translation UTF-8 bytes as `expected_translation_utf8_sha256`. The sole builder hashes its accepted final input as `frozen_renderer_input_utf8_sha256` and independently hashes the exact bytes entering its shared pre-inpaint layout/raster path as `renderer_entry_utf8_sha256`; this legacy field name now denotes pre-inpaint raster entry, not Renderer-engine work. With no Planner breaks all three digests and byte/scalar counts match. With accepted insertion-only breaks, frozen and raster-entry values match each other, the break transcript names only inserted `\n` byte offsets, and removing exactly those bytes reconstructs the expected digest, byte length, scalar order, and every non-newline byte. Any deletion, replacement, reorder, alternate whitespace, invalid UTF-8 boundary, duplicate/out-of-range offset, or digest mismatch returns the existing `geometry_or_font` builder error before sprite allocation, publication, inpainter run/apply, or erase and preserves exact equality to `B_prepare` without rolling back upstream commits.
- Before any inpainter run/apply, the builder lays out the final input and builds a canonical transcript from each line's original-text byte range and every shaped glyph's cluster value. Convert sorted cluster starts plus the next start/line end into nonoverlapping half-open cluster intervals; every non-control input scalar byte must lie in at least one interval. Only the closed legal no-ink control allowlist, including accepted inserted LF and legitimate whitespace already classified by layout, may appear once in deterministic layout-control records; arbitrary control characters are not exempt. No other byte may be classified as control, and no gap, overlap conflict, or out-of-range interval is accepted. Hash that transcript as `layout_cluster_coverage_sha256` and require `cluster_coverage_complete=true`. A shaped glyph with `glyph_id==0` belonging to any non-control cluster is `.notdef`, increments `missing_glyph_count`, and returns the same hard pre-write error. Separately require every nonzero glyph ID to fit `u16`; low-level `render_line` returns an error on conversion failure as defense in depth. A static renderer contract test proves the shared fill `render_pass` iterates every layout line and every glyph exactly once; a successful returned fill pass therefore records `fill_raster_visited_glyph_count=shaped_glyph_count`, `missing_glyph_count=0`, `fill_raster_visit_complete=true`, and nonempty alpha. Stroke may add a second pass but cannot satisfy the fill count. Existing tests distinguish absent-scalar `.notdef` failure, fallback-font nonzero success, ligature/combining clusters that need not map one glyph per scalar, legal no-ink controls, nonzero `u16` overflow, fill failure, and empty alpha. These private/test-only records change no public renderer API; only RPC behavior documentation/OpenAPI responses regenerate. Exact persisted-input RGBA replay and per-target omission inequality remain independently mandatory after Renderer persists/composites the frozen sprite.
- **Checked frozen-sprite reservation and raster scratch.** For the complete canonical successful-target set, before any retained sprite allocation or raster call, require `logical_sprite_bytes_i=checked(logical_width_i*logical_height_i*4)`, checked conversion of all logical dimensions and bytes to `usize`, and proof that the logical surface lies wholly inside target `i`'s disjoint owner rectangle. Checked-sum all retained `logical_sprite_bytes_i` and require the total to be at most `checked(page_width*page_height*4)`. Reserve the complete retained logical set atomically. Separately, for each target at the actual existing `raster_scale_i` in `2..=4`, require `raster_width_i=checked(logical_width_i*raster_scale_i)`, `raster_height_i=checked(logical_height_i*raster_scale_i)`, `raster_surface_bytes_i=checked(raster_width_i*raster_height_i*4)`, and checked `usize` conversion of every raster dimension and byte count. Construct the transient surface through the existing fallible surface constructor, rasterize serially exactly once, downsample/freeze the logical sprite, and release that target's supersampled/copy/downsample scratch before the next target starts. Do not sum transient surfaces as retained bytes or keep multiple target scratch surfaces live. Only checked logical/actual-raster arithmetic or `usize` conversion failure, retained owner-rectangle/page-cap reservation failure, or fallible raster-surface-construction failure is recoverable; it releases all unpublished reservation/scratch and returns the same `geometry_or_font` builder result with final state equal to `B_prepare`; process-level allocator OOM/abort is outside this recoverable contract. Publication occurs only after all target raster/alpha/safety checks; unpublished buffers are dropped on failure. Renderer success, no-Renderer completion, and inpainter failure all release pixel payloads on their terminal path, with inpainter failure proving zero sprite/blob persistence. Terminal non-pixel metadata may remain until run drop.
- Axis-aligned means both node and text rotation are finite zero. Rotation is terminally classified before Source Gate and is excluded from color-mode completeness. Every Source-Gate-accepted non-manual axis-aligned HanOnly target first receives one provisional source partition/width result. The complete no-side-effect preflight then resolves final layout/font/effects, per-target fit, Planner-size outcome, group-min result, `f_t_raw/F_t`, `W_t`, and final raster/probe/ownership/safety. Only after all targets reach one terminal result does it atomically publish the run-local `AutomaticStrict(SourceColorContract)` or `UnsupportedSourceColor(SourceColorReason)` map, before any erase/inpaint write. A missing Segment/source/layout/font input, incomplete accepted-node bijection, unknown reason, rotation in the map, partial publication, or second inconsistent classification fails before inference, blob creation, or Scene ops.
- Freeze `source-color-contract-v2` with exact constants `COLOR_UNIFORMITY_P95_MAX_CHANNEL=4`, `COLOR_MIN_CLUSTER_DISTANCE_L1=24`, and `COLOR_AA_BLEND_RESIDUAL_MAX_CHANNEL=1` plus the following complete partition/width algorithm before viewing any regression, calibration, or holdout output. The only source coordinate is page-space `RenderBlockInput.source_transform`; `transform` is layout-only. Convert `source_transform` with `left/top=floor`, inclusive `right/bottom=ceil-1`, then page-clip. Define `source_geometry_estimate` as bbox height for horizontal writing and width for vertical writing; compute checked `F_s=floor(source_geometry_estimate+0.5)` and require `F_s>=1` before any automatic result. Tests make `source_transform` and `transform` disagree and require the former, and cover `0.49/0.50/0.51/1.49/1.50`, nonfinite, nonpositive, and overflow.
- Let `Ω` be the target-owned integer page-pixel domain after subtracting page-outside, protected, other-source, and neighboring target ownership. Let `Q` be the nonempty intersection of `Ω`, the integer `source_transform` bbox, and acceptance's blind-agreed union `M`; production uses its independently derived complete per-node source-ink support and never reads either manifest mask. Let `B=(Dilate8(Q,1)\Q)∩Ω`; every `Q/B` source pixel must have alpha `255`. Build a 5-bit RGB histogram over `Q` with key `(r>>3,g>>3,b>>3)`, order occupied bins by `(-count,key_r,key_g,key_b)`, and choose one per-channel lower-median representative per occupied bin. P1 derives only these domains, counts, and representatives; direct cores, P95, background witnesses, candidates, alpha witnesses, labels, and topology are E-only.
- For representative `h`, define complete same-bin samples `S_h={q∈Q | bin(rgb(q))=bin_h}` and integer deviation `d_h(q)=max(|r(q)-r(rep_h)|,|g(q)-g(rep_h)|,|b(q)-b(rep_h)|)`. `S_h` is nonempty by occupied-bin construction. E builds one 256-slot histogram over the complete multiset `{d_h(q)|q∈S_h}`. Checked `rank95=(95*|S_h|+99)/100`; `p95_h` is the smallest slot whose cumulative count reaches `rank95`. Direct core `D_h={q∈S_h | d_h(q)<=4}`. Representative `h` may label a candidate only when `D_h` is nonempty and `p95_h<=COLOR_UNIFORMITY_P95_MAX_CHANNEL`. The percentile excludes `B`, reconstructed non-core pixels, model data, clean references, rendered output, and other targets; it uses no interpolation, floating point, rescaling, or per-channel percentile.
- After E admission, build exactly one immutable candidate-independent table `bg(q)=argmin_{b∈B}(max(|q_x-b_x|,|q_y-b_y|),b_y,b_x)` for every `q∈Q`. Its inputs are only frozen `Q/B` coordinates. Every candidate reads that same table; candidate representatives, semantic labels, model/text data, candidate order, or output cannot rebuild, filter, mutate, or replace it. Missing/duplicate entries or candidate-specific selection is semantic failure or `accounting_invariant_failure` as applicable.
- All color reconstruction uses checked integers. For ordered `(foreground,background)` and `a∈1..254`, each channel is `blend_c=(a*foreground_c+(255-a)*background_c+127)/255` with floor integer division. This freezes round-to-nearest/half-up; divisor 255 makes an exact integer-numerator half impossible. Residual is `max(|source_r-blend_r|,|source_g-blend_g|,|source_b-blend_b|)` and must be `<=COLOR_AA_BLEND_RESIDUAL_MAX_CHANNEL`. No float, gamma conversion, premultiplication shortcut, SIMD-specific rounding, truncation-before-sum, or renderer blend rule may replace it.
- Fill-only has exact pair order `0:(fill,bg(q))->{fill}`. Fill/stroke has exact pair order `0:(fill,bg(q))->{fill}`, `1:(stroke,bg(q))->{stroke}`, `2:(fill,stroke)->{fill,stroke}`; the first endpoint is alpha-weighted foreground and no `(stroke,fill)` pair exists. For each non-direct pixel evaluate every permitted pair and every `a`. Determine minimum residual without pair order. If more than one pair order attains that minimum, fail `AmbiguousSemanticColorWitness`; pair order never resolves a cross-pair semantic tie. Within the one winning pair, smallest alpha is the deterministic diagnostic witness. Pair 2 allows both labels and therefore enters the two-label shortest-path proof.
- Exhaustively enumerate every one-core fill candidate and every ordered distinct fill/stroke representative pair. Each fill representative must be `L1>=24` from every `bg(q)`; stroke candidates require both representatives separately `L1>=24` from every `bg(q)` and fill/stroke `L1>=24`. One-core candidates follow E-derived core order; stroke candidates are lexicographic `(fill_core_order,stroke_core_order)`. Enumeration order is diagnostic only. Exactly one complete candidate yields `AutomaticStrict`; zero candidates or more than one, including output-identical but semantically different partitions, yields fail-closed `UnsupportedSourceColor`.
- After one provisional source partition and topology-unique `W_s` exist, resolve the final no-side-effect target-size preflight. Let `f_t_raw` be the finite positive final backend-selected size after the exact per-target fit, Planner-size outcome, and group-min resolution. Define `F_t=checked_u64(floor(f_t_raw+0.5))` in integer page pixels before raster/device scale; nonfinite, nonpositive, or overflow is invalid. Freeze that same integer and use it in the builder's sole pre-inpaint raster. In a no-stroke candidate require `W_s=W_t=0` and `stroke_ratio` absent or `null`; serialization, diagnostics, and aggregation must never contain `0/0`, `0/F_s`, or any ratio. In a stroke candidate require `F_s>1`; derive scalar `U=min(F_s-1,floor((min(width(Ω_bbox),height(Ω_bbox))-1)/2))`, then use only the single rectangular Chebyshev-distance interval proof defined below. No `1..=U` or equivalent width enumeration is allowed. Exactly one interval-proved `W_s` and checked `W_t=floor((2*W_s*F_t+F_s)/(2*F_s))` are single-sided page-pixel radii, `W_t>=1`, and only this branch records reduced `W_s/F_s`. The exact rectangle/font/writing/effects/padding/color/`F_t/W_t` combination must pass raster-fit, probe, ownership, complete translation/cluster/glyph/fill traversal, checked retained logical-sprite reservation, independently checked actual-scale raster dimensions/bytes/construction, serial scratch release, nonempty alpha, and safety before `AutomaticStrict` plus frozen-sprite publication and erase. A translation-identity mismatch, final-layout failure after local fallback, cluster/control-coverage failure, glyph-validity failure, fill-traversal failure, or nonempty-alpha validation failure is a deterministic hard pre-write `geometry_or_font` failure. Only checked logical/actual-raster arithmetic or `usize` conversion failure, retained owner-rectangle/page-cap reservation failure, or fallible raster-surface-construction failure is a recoverable `geometry_or_font` failure. Either builder class preserves exact final equality to `B_prepare`, retains upstream commits, and has zero publication/inpainter/erase/downstream persistence; process allocator OOM/abort is outside the result. Source-color or `F_s/W_s/F_t/W_t` representability failure is the existing unsupported-preserve-ROI result. Renderer receives frozen sprite bytes with `F_t` and `width_px=W_t` metadata and only persists/composites once; it cannot layout, rasterize, rerun fit/grouping/Planner-size selection/rounding/clamping/width derivation, or revalidate through a second pixel path. Outline alone maps width to full width `2*W_t*raster_scale`, while bitmap uses radius `ceil(W_t*raster_scale)`. Tests cover unique thin/thick strokes, no-width and multiple-width intervals, multiple complete core partitions, AA ties/unreachable cores/residual overflow, empty/clipped domains, `F_s=1` stroke, `F_t` boundaries, width overflow, exact fill/stroke probes, bounded JPEG-like noise, gradient/multicolor/pattern/nonopaque source, and terminal shadow/glow unsupported. Prediction, Planner, default black/default width, and `contrasting_stroke_color` are forbidden source-style authorities; Planner participates only in the bounded final-size outcome.
- Freeze `source-color-work-budget-v1` with `SOURCE_COLOR_MAX_CORES=64`, `SOURCE_COLOR_MAX_CANDIDATES=4096`, `SOURCE_COLOR_TARGET_WORK_UNITS=16_777_216`, and `SOURCE_COLOR_PAGE_WORK_UNITS=134_217_728`. Revision 46 changes no numeric limit. `color_constant_set_sha256` and `source_color_contract_sha256` are computed only from the two closed canonical-JSON preimages below; no fixture identity, filename, machine speed, model output, elapsed runtime, or manifest role enters either digest or verdict.
- For one page let `J=page.nodes.len()` be the O(1) page-node count, let `T` be the automatic axis-aligned Source-Gate-accepted target count derived only by the admitted P-1 enumeration, let `A=checked_u64(page_width*page_height)`, canonical key `(anchor.top,anchor.left,anchor.bottom,anchor.right,NodeId)`, and `X_i` the checked area of target `i`'s clipped integer `source_transform` bbox. All arithmetic is checked `u64`; overflow before a stage admission performs no work from that stage.
- **P-1/P0 metadata admission.** Read only `J` before admission. If `J>SOURCE_COLOR_PAGE_WORK_UNITS`, emit `p0_metadata_unbounded` at phase `node_enumeration` with page-preflight requested/reserved/consumed `J/0/0`; hard-fail before reading a node, creating a target record/warning, or mutating state. Otherwise reserve `J`, traverse every page node exactly once under phase `node_enumeration`, apply the existing automatic-target predicate, materialize that collection, and derive `T`. Checked-compute `Wterm=J+T`. Overflow emits `p0_arithmetic_overflow/target_terminalization` with `null/J/J`; `Wterm` above the page limit emits `p0_metadata_unbounded/target_terminalization` with `Wterm/J/J`; neither may traverse targets or emit target records because terminal slots were not admitted. On success atomically extend the reservation to `Wterm`, traverse the `T` targets exactly once to establish one terminal slot per target, and enter phase `canonical_ranking`. Define total checked `choose2_checked(T)` as `0` for `T<2`; for even `T`, checked-multiply `(T/2)*(T-1)`; for odd `T`, checked-multiply `T*((T-1)/2)`. This division-before-multiplication rule never evaluates unsigned `T-1` for `T<2`. Checked-compute `Wpair=choose2_checked(T)` and then `Wmeta=checked(Wterm+Wpair)`. Either pair multiplication or final addition overflow emits `p0_arithmetic_overflow/canonical_ranking` with `null/Wterm/Wterm`; page-limit excess emits `p0_page_limit/canonical_ranking` with `Wmeta/Wterm/Wterm`; both emit one existing budget-unsupported record/warning per target and perform zero pair comparisons. On admission atomically extend the same reservation to `Wmeta`, compare every unordered canonical-key pair exactly once, and derive canonical ranks. Admitted P0 requested/reserved/consumed is exactly `Wmeta`, independently of input order. No path adds `J`, `Wterm`, and `Wmeta` as separate reservations.
- **P1 target-data admission.** For each canonically ranked target compute without pixel traversal:

  ```text
  P_i = 4*A + 19*X_i + 81_920
  81_920 = 32_768 + 768*64
  ```

  `P_i` is the exact calculator value and conservative requested/reserved upper bound. It is not a promise that every admitted target will execute every possible operation.

  | Term | Exact requested/reserved upper-bound operation |
  |---|---|
  | first `A` | one ownership-distance comparison attributed to target `i` for each page pixel |
  | second `A` | materialize `Ω_i` after protected/non-target/other-owner subtraction |
  | third `A` | initialize/compact target-local `Q/B` membership storage |
  | fourth `A` | row-major `B` compaction and validation |
  | first `X_i` | form `Q_i=Ω_i∩bbox_i∩source_ink_i` |
  | `8*X_i` | fixed-order eight-neighbor probes constructing `B_i` |
  | second `X_i` | `Q_i` alpha validation and fixed 5-bit histogram insertion |
  | second `8*X_i` | upper-bound `B_i` alpha validation because `|B_i|<=8X_i` |
  | third `X_i` | after `H<=64`, populate the admitted bins' three 256-slot channel histograms |
  | `32_768` | full histogram-bin scan |
  | `768*64` | 256 slots times three channels times at most 64 lower-median representatives |

  Overflow computing `P_i` yields target `arithmetic_overflow`; `P_i>target limit` yields `target_work_units`; neither target enters pixel preflight, and its existing source support remains an obstacle. Checked-sum `Ppage_requested=Wmeta+ΣP_i` over the remaining targets. Overflow or page-limit excess rejects all remaining targets before pixel access. Only then reserve the full page preflight and derive target data.
- P1 scans page pixels row-major. Ownership uses minimum Manhattan anchor distance with canonical-key ties. Build `Q` row-major. Build `B=(Dilate8(Q,1)\Q)∩Ω` in neighbor order `NW,N,NE,W,E,SW,S,SE`, deduplicate by membership bit, then compact row-major. Empty `Q/B` or nonopaque `Q/B` uses the existing semantic `SourceColorReason` and does not enter E. Build the fixed 32,768-bin histogram and scan keys numerically. If occupied count `H>64`, terminate `core_count` before per-channel histogram allocation or median work. Otherwise use the separately charged third `X_i` pass to populate only the at-most-64 admitted per-channel histograms and derive representatives through the reserved slot scans. P1 does not compute direct membership/P95/order, `bg(q)`, candidates, alpha witnesses, labels, or topology. Set `N=|Q|`, `L=|B|`, `M=area(Ω_bbox)`, `K=H^2`, and scalar-only `U=min(F_s-1,floor((min(width(Ω_bbox),height(Ω_bbox))-1)/2))`.
- For admitted target `i`, set `classifier_preflight_work_units_requested=P_i` and `reserved=P_i`. Set `classifier_preflight_work_units_consumed=C_i`, the checked sum of declared P1 operations that actually execute. Each real fixed operation or loop iteration increments its corresponding counter exactly once. Unused `B` capacity when `|B|<8X_i`, unvisited histogram slots, unused representative capacity when `H<64`, representative work skipped after `H>64`, semantic early-stop suffixes, padding/no-op work, and synthetic counter increments consume zero. Thus `0<=C_i<=P_i`; equality is not required. `consumed>reserved`, undeclared work, independent-counter disagreement, padding/no-op execution solely to consume slack, or synthetic inflation is `accounting_invariant_failure`. After admitted P0, page preflight requested/reserved is `Wmeta+ΣP_i`; actual page preflight consumption is exact admitted P0 consumption plus `ΣC_i`.
- **E evaluation admission.** Apply exact precedence: `H>64 -> core_count`, then `K>4096 -> candidate_count`, reservation overflow, `P_i+E_i>target limit`, then final page demand above page limit. Thus `H=65,K=4225` is `core_count`; `candidate_count` is subordinate and unreachable under the current paired limits. Before candidate/witness allocation compute:

  ```text
  Wbg     = N*L
  Wcore   = H*N + 256*H + H
  Worder  = H*(H-1)/2
  Wfill   = 1_028*N + 3
  Wstroke = 3_071*N + 12*M + 7
  E_i     = Wbg + Wcore + Worder + H*Wfill + H*(H-1)*Wstroke
  R_i     = P_i + E_i
  ```

  | Term | Exact reserved operation |
  |---|---|
  | `N*L` | build the one shared `bg(q)` table by comparing every `(q,b)∈Q×B` |
  | `H*N` | derive same-bin deviations, direct membership bits, and residual histograms with fixed unrolled RGB work |
  | `256*H` | scan every P95 residual-histogram slot |
  | `H` | checked nearest-rank P95/nonempty direct-core result per representative |
  | `H*(H-1)/2` | core-order comparisons |
  | `N` in `Wfill` | fill/background `L1` separation against shared `bg(q)` |
  | `1_016N` in `Wfill` | `254N` trials times three unrolled channels plus one residual/tie update |
  | `11N` in `Wfill` | one BFS initialization, removals, eight edge probes, and final validation |
  | `3` | fill record initialization, direct/P95 check, and completeness/result check |
  | `2N` in `Wstroke` | fill/background and stroke/background `L1` separation against shared `bg(q)` |
  | `3_048N` in `Wstroke` | three pair families times `254N` trials times four charged operations |
  | `21N` in `Wstroke` | two BFS passes plus compatible-label/tie validation |
  | `12M` | rectangular Chebyshev-distance initialization/BFS/edges plus partition/interval validation |
  | `7` | stroke record, two direct/P95 checks, fill/stroke separation, interval, width, and completeness checks |

  Exclude individually rejected targets from E page admission. Compute `Epage_requested=ΣE_i` and `Rpage_requested=Ppage_reserved+Epage_requested`; only a passing page reserves all evaluation allocations and traversals atomically. Otherwise every individually eligible target becomes page-budget unsupported with zero evaluation reservation/evaluation and no `Wbg/Wcore/Worder` work. Fixed unrolled channel operations have their listed charge; every other data-dependent loop increments its declared counter once per iteration and accumulates invalidity without an uncharged early-exit traversal. Target/candidate order is canonical, and page consumption is the checked deterministic counter sum, never completion order.
- **Exact label/topology algorithms.** Every candidate reads the one shared `bg(q)` table and the closed integer alpha witnesses above. For each compatible core label, one fixed-order multi-source 8-neighbor BFS inside `Q` initializes distance once, charges removals and eight edges, and maintains saturated shortest-path count `0|1|2`; equal-distance contributions do not requeue. A non-direct pixel is valid only when exactly one compatible label has minimum distance and its shortest-path count is `1`; same-core multipath, cross-label tie, unreachable core, incompatible witness, or incomplete classification invalidates the candidate.
- Stroke width performs no `1..=U` loop. One multi-source 8-neighbor BFS over the full rectangular `Ω_bbox` computes direct Chebyshev distance `d(p,F)`. Require finite distance for stroke pixels, set `lo=max(1,max d(s,F))`, and set `hi=min(U,min d(p,F)-1)` over `p∈Ω\(F∪S)` or `U` if that set is empty. Independently verify `p∈S iff d(p,F)<=lo` for every `p∈Ω\F`. Accept only `lo==hi` with `W_s=lo`; `lo>hi` or `lo<hi` is `UnprovableStrokeWidth`. `classifier_width_probe_count` counts stroke candidates reaching this interval proof.
- **Terminal diagnostics.** Retain the existing fields and add these exact fields:

  ```text
  classifier_page_node_count
  classifier_automatic_target_count
  classifier_p0_phase
  classifier_terminal_state
  classifier_preflight_work_units_requested
  classifier_preflight_work_units_reserved
  classifier_preflight_work_units_consumed
  classifier_evaluation_work_units_requested
  classifier_evaluation_work_units_reserved
  classifier_evaluation_work_units_consumed
  classifier_work_units_requested
  classifier_work_units_reserved
  classifier_work_units_consumed
  classifier_target_work_units_limit
  classifier_page_preflight_work_units_requested
  classifier_page_preflight_work_units_reserved
  classifier_page_preflight_work_units_consumed
  classifier_page_evaluation_work_units_requested
  classifier_page_evaluation_work_units_reserved
  classifier_page_evaluation_work_units_consumed
  classifier_page_work_units_requested
  classifier_page_work_units_reserved
  classifier_page_work_units_consumed
  classifier_page_work_units_limit
  ```

  `classifier_page_node_count=J` is always available without traversal. `classifier_automatic_target_count=T` is null until node enumeration completes. `classifier_p0_phase` is exactly `node_enumeration|target_terminalization|canonical_ranking`. `requested=null` only when an earlier stop/overflow prevents formula derivation; unadmitted `reserved=0`; unexecuted `consumed=0`; `H/K/U=null` before derivation; action counts are zero before execution; total requested is null if a required component is null. Before terminal slots are admitted, page-preflight reserved/consumed is exactly `0` or `J`; after terminalization but before canonical ranking admission it is exactly `Wterm`, never a sum of reservation snapshots. For admitted P1, requested/reserved equals the conservative calculator bound and consumed equals actual declared work, so an admitted early stop may have `consumed<reserved`; exact-bound and bound-plus-one tests refer to requested/reserved, never consumed equality. Padding/no-op/synthetic increments hard-fail. Buffer records until page termination and repeat identical final page totals in every automatic-target record. `p0_metadata_unbounded/node_enumeration`, `p0_metadata_unbounded/target_terminalization`, and `p0_arithmetic_overflow/target_terminalization` create no target record because node or terminal-slot work was not admitted; canonical-ranking failures emit the already-admitted target records.

  For a target, preflight requested is `P_i`, evaluation requested is `E_i`, and total requested is checked `P_i+E_i`; each reserved/consumed total is the checked sum of its preflight and evaluation components, with P1 consumed using `C_i` rather than `P_i`. For the page, evaluation consumed/requested/reserved is respectively the checked sum of emitted target evaluation consumption, `E_i` for individually eligible targets, and `E_i` for page-admitted targets. Page preflight consumed/requested/reserved is respectively checked `P0_consumed+ΣC_i`, checked `P0_requested+ΣP_i` for targets remaining after individual rejection, and checked `P0_reserved+ΣP_i` for page-admitted targets. Page total consumed/requested/reserved is the checked sum of its preflight and evaluation component. A component unavailable because of an earlier stop makes only dependent requested totals null. P-1/P0 metadata work is page-preflight work: admitted P0 is `requested=reserved=consumed=Wmeta`; node-enumeration unbounded is `J/0/0`; terminalization overflow is `null/J/J`; terminalization unbounded is `Wterm/J/J`; canonical-ranking overflow is `null/Wterm/Wterm`; and canonical-ranking page-limit is `Wmeta/Wterm/Wterm`. P0 never appears in any target `P_i` and is never counted twice.

  | Terminal state | Evaluation requested/reserved | Limit kind | Result |
  |---|---:|---|---|
  | `p0_metadata_unbounded` | `null/0` | `page_work_units` | hard page failure, no warning |
  | `p0_arithmetic_overflow` | `null/0` | `arithmetic_overflow` | all target ROIs preserved |
  | `p0_page_limit` | `null/0` | `page_work_units` | all target ROIs preserved |
  | `p1_target_arithmetic_overflow` | `null/0` | `arithmetic_overflow` | target unsupported |
  | `p1_target_limit` | `null/0` | `target_work_units` | target unsupported |
  | `p1_page_arithmetic_overflow` | `null/0` | `arithmetic_overflow` | remaining targets unsupported |
  | `p1_page_limit` | `null/0` | `page_work_units` | remaining targets unsupported |
  | `p1_semantic_unsupported` | `null/0` | `none` | existing semantic reason |
  | `core_limit` | `null/0` | `core_count` | target unsupported |
  | `candidate_limit` | `null/0` | `candidate_count` | target unsupported |
  | `evaluation_arithmetic_overflow` | `null/0` | `arithmetic_overflow` | target unsupported |
  | `target_total_limit` | exact/`0` | `target_work_units` | target unsupported |
  | `page_evaluation_limit` | exact/`0` | `page_work_units` | eligible targets unsupported |
  | `evaluated_semantic_unsupported` | exact/exact | `none` | existing semantic reason |
  | `automatic_strict` | exact/exact | `none` | render exact contract |
  | `accounting_invariant_failure` | observed | `none` | hard page failure, no warning |

  Every budget-unsupported target emits exact bytes `han_only.unsupported_source_color:classifier_work_budget_exceeded` once under dedupe key `(NodeId,ClassifierWorkBudgetExceeded)`; limit detail exists only in diagnostics. A post-admission overflow, `consumed>reserved`, counter/calculator disagreement, or uncharged traversal is `accounting_invariant_failure`, hard-fails RPC, and performs no erase/inference/blob/renderer/Scene mutation. Elapsed time is observability only.
- **Closed color-contract hash preimages.** Both digest inputs are closed JSON objects, not extracted prose. Reject missing, unknown, duplicate, or wrong-type fields. Recursively sort object keys by Unicode code point; every key and string in these two schemas is ASCII, so this is also ascending UTF-8 byte order. Preserve array order. Serialize nonnegative integers in base 10 with no leading zero except `0`; floats and negative values are forbidden. Escape `"`, `\`, and U+0000 through U+001F with the standard short escapes where available and lowercase `\u00xx` otherwise; emit `/` and all other scalars directly as UTF-8. Emit no insignificant whitespace, BOM, CRLF conversion, or trailing newline. SHA-256 covers exactly those UTF-8 bytes and is encoded as 64 lowercase hex. Native map order, pretty JSON, Unicode normalization, timestamps, image/manifest/runtime values, or alternate formula spelling are forbidden.

  `color_constant_set_sha256` hashes exactly this object:

  ```json
  {
    "color_aa_blend_residual_max_channel": 1,
    "color_min_cluster_distance_l1": 24,
    "color_uniformity_p95_max_channel": 4,
    "schema": "hanonly-color-constant-set-preimage-v1",
    "source_color_contract_version": "source-color-contract-v2",
    "source_color_max_candidates": 4096,
    "source_color_max_cores": 64,
    "source_color_page_work_units": 134217728,
    "source_color_target_work_units": 16777216,
    "source_color_work_budget_version": "source-color-work-budget-v1"
  }
  ```

  `source_color_contract_sha256` hashes exactly this object:

  ```json
  {
    "accounting": {
      "evaluation": {
        "candidate_count": "H*H",
        "core_membership_and_p95_insert": "H*N",
        "core_order": "H*(H-1)/2",
        "core_p95_checks": "H",
        "core_p95_histogram_scan": "256*H",
        "formula": "N*L+H*N+256*H+H+H*(H-1)/2+H*(1028*N+3)+H*(H-1)*(3071*N+12*M+7)",
        "shared_background_witness": "N*L",
        "wfill": {
          "constant": 3,
          "n": 1028
        },
        "wstroke": {
          "constant": 7,
          "m": 12,
          "n": 3071
        }
      },
      "p0": {
        "admitted": "J+T+choose2_checked(T)",
        "automatic_target_enumeration": "J",
        "canonical_rank_pair_comparisons": "choose2_checked(T)",
        "choose2_checked": {
          "t_even": "checked((T/2)*(T-1))",
          "t_lt_2": "0",
          "t_odd": "checked(T*((T-1)/2))"
        },
        "node_count": "J=page.nodes.len()",
        "target_terminalization": "T",
        "terminal": {
          "p0_arithmetic_overflow": {
            "canonical_ranking": {
              "consumed": "J+T",
              "requested": "null",
              "reserved": "J+T",
              "target_records": "T"
            },
            "target_terminalization": {
              "consumed": "J",
              "requested": "null",
              "reserved": "J",
              "target_records": "0"
            }
          },
          "p0_metadata_unbounded": {
            "node_enumeration": {
              "consumed": "0",
              "requested": "J",
              "reserved": "0",
              "target_records": "0"
            },
            "target_terminalization": {
              "consumed": "J",
              "requested": "J+T",
              "reserved": "J",
              "target_records": "0"
            }
          },
          "p0_page_limit": {
            "canonical_ranking": {
              "consumed": "J+T",
              "requested": "J+T+choose2_checked(T)",
              "reserved": "J+T",
              "target_records": "T"
            }
          }
        }
      },
      "p1": {
        "a": 4,
        "consumed_semantics": "actual-declared-loop-iterations-only",
        "fixed": 81920,
        "formula": "4*A+19*X_i+81920",
        "histogram_bins": 32768,
        "padding": "forbidden",
        "representative_histogram_population": "third-X_i-after-H-le-64",
        "representative_scan_per_core": 768,
        "requested_reserved_semantics": "conservative-upper-bound",
        "synthetic_counter_inflation": "forbidden",
        "unused_reserved_work": "not-consumed",
        "x": 19
      },
      "page": {
        "evaluation": {
          "consumed": "checked-sum(classifier_evaluation_work_units_consumed_i)",
          "requested": "checked-sum(E_i_for-individually-eligible-targets)",
          "reserved": "checked-sum(E_i_for-page-admitted-targets)"
        },
        "preflight": {
          "consumed": "checked(P0_consumed+sum(C_i))",
          "requested": "checked(P0_requested+sum(P_i_for-targets-remaining-after-individual-rejection))",
          "reserved": "checked(P0_reserved+sum(P_i_for-page-admitted-targets))"
        },
        "total": {
          "consumed": "checked(page_preflight_consumed+page_evaluation_consumed)",
          "requested": "checked(page_preflight_requested+page_evaluation_requested)",
          "reserved": "checked(page_preflight_reserved+page_evaluation_reserved)"
        }
      },
      "precedence": [
        "core_count",
        "candidate_count",
        "arithmetic_overflow",
        "target_work_units",
        "page_work_units"
      ],
      "work_unit_version": "source-color-work-unit-v3"
    },
    "algorithm": {
      "alpha": {
        "alpha_max": 254,
        "alpha_min": 1,
        "blend": "(a*foreground_c+(255-a)*background_c+127)/255",
        "cross_pair_tie": "fail-closed",
        "division": "checked-floor-integer",
        "residual": "max(abs(source_c-blend_c))",
        "same_pair_tie": "lowest-alpha"
      },
      "automatic_strict_preflight": {
        "failure": {
          "geometry_or_font": "hard-prewrite-no-mutation",
          "source_color_or_width": "unsupported-preserve-roi-zero-erase-block-sprite-completed-with-errors"
        },
        "order": [
          "source-color-partition",
          "layout-geometry-font-fit-planner-size-group",
          "target-font-rounding-F_t",
          "target-stroke-rounding-W_t",
          "final-geometry-font-color-width-validation",
          "publish-automatic-strict",
          "erase-inpaint-write"
        ],
        "publish": "only-after-all-preflight-checks",
        "renderer": "consume-frozen-result-without-fit-group-planner-size-width-recomputation"
      },
      "background_witness": {
        "candidate_shared": "single-read-only-table",
        "domain": "QxB",
        "order": [
          "chebyshev_distance",
          "y",
          "x"
        ]
      },
      "candidate_order": [
        "fill_core_order",
        "stroke_core_order"
      ],
      "candidate_result": {
        "more_than_one": "unsupported-ambiguous-semantic-partition",
        "one": "automatic-strict",
        "zero": "unsupported-source-color"
      },
      "core": {
        "bin": [
          "r>>3",
          "g>>3",
          "b>>3"
        ],
        "direct_membership": "same-bin-and-linf-rgb-le-4",
        "percentile_domain": "complete-same-bin-max-channel-residuals",
        "percentile_rank": "(95*n+99)/100",
        "percentile_rule": "nearest-rank-smallest-cumulative-slot",
        "representative": "per-channel-lower-median",
        "representative_order": [
          "negative-bin-count",
          "bin-r",
          "bin-g",
          "bin-b"
        ]
      },
      "domains": {
        "B": "(Dilate8(Q,1)-Q)-intersect-Omega",
        "Q": "Omega-intersect-source-bbox-intersect-source-ink",
        "source_alpha": 255,
        "source_bbox": "left-top-floor-right-bottom-ceil-minus-1-page-clipped"
      },
      "pair_order_fill": [
        "fill/background->{fill}"
      ],
      "pair_order_stroke": [
        "fill/background->{fill}",
        "stroke/background->{stroke}",
        "fill/stroke->{fill,stroke}"
      ],
      "separation": {
        "fill_background": "all-shared-bg-q-l1-ge-24",
        "fill_stroke": "representative-l1-ge-24",
        "stroke_background": "all-shared-bg-q-l1-ge-24"
      },
      "width": {
        "distance": "direct-chebyshev-over-Omega-bbox",
        "enumeration": "forbidden",
        "rule": "single-interval-lo-equals-hi",
        "target_font_size": {
          "definition": "final-backend-fit-planner-outcome-group-selected-font-size",
          "rounding": "checked-floor(value+0.5)",
          "unit": "integer-page-pixels-before-raster-scale"
        },
        "target_rounding": "checked-floor((2*W_s*F_t+F_s)/(2*F_s))"
      }
    },
    "constants": {
      "color_aa_blend_residual_max_channel": 1,
      "color_min_cluster_distance_l1": 24,
      "color_uniformity_p95_max_channel": 4,
      "source_color_max_candidates": 4096,
      "source_color_max_cores": 64,
      "source_color_page_work_units": 134217728,
      "source_color_target_work_units": 16777216
    },
    "contract_version": "source-color-contract-v2",
    "schema": "hanonly-source-color-contract-preimage-v1",
    "terminal": {
      "accounting_invariant_failure": "hard-page-no-warning-no-mutation",
      "complexity_warning": "han_only.unsupported_source_color:classifier_work_budget_exceeded",
      "p0_metadata_unbounded": "hard-page-no-target-record-no-mutation",
      "unsupported_budget": "preserve-roi-zero-erase-block-sprite-completed-with-errors"
    },
    "work_budget_version": "source-color-work-budget-v1"
  }
  ```

  Formula strings and tokens are exact ASCII schema values, not executable expressions. Revision 46 keeps the constant-set preimage byte-for-byte unchanged at 433 canonical bytes with SHA-256 `ea277ff2674aae711b62a39b6a0b930e7d9c863bd518521c59ff44be56c4c6e9`; the revised source-contract preimage is exactly 5506 canonical bytes with SHA-256 `13d2256fed7b8189e67db7222ce6ce7964f2745c977c42e7693679ffb2a341f8`. `plan_revision` is outside both preimages. The `accounting` object order is exactly `evaluation,p0,p1,page,precedence,work_unit_version` after canonical sorting. The external visual manifest remains an image/role/expectation input and is not either preimage; it is not claimed to contain these algorithms, tables, limits, translation digests, or canonicalization fields. B0, holdout, final acceptance, guarded runtime reports, `R`, `A`, and `C` independently recompute the two objects above and compare their digests; every Revision 45 source-contract digest is stale and rejected.

  The canonical public `geometry_or_font` token remains unchanged and outside both color preimages. A private typed discriminant distinguishes translation identity, final layout, cluster/control coverage, cluster/run alpha contribution, glyph validity, fill traversal, nonempty alpha, checked memory arithmetic, peak-live reservation, and fallible surface construction. Deterministic content/geometry variants are hard pre-write failures; checked arithmetic/reservation/construction variants are recoverable. The recoverable reservation includes every simultaneously live retained logical sprite, supersampled Pixmap, unavoidable copy, downsample output, mask, staged Scene/blob, and transaction buffer. Process allocator abort remains outside recovery, but every admitted input inside `image-input-contract-v1` must fit or fail before allocation.
- `SourceColorReason` includes `ClassifierWorkBudgetExceeded`. No partial/current-best candidate, prediction, Planner value, default, timeout, elapsed threshold, or retry may recover any budget rejection.
- `UnsupportedSourceColor` is removed from canonical erase ownership before backend expansion, retains byte-identical Source/Inpainted/Rendered pixels throughout its edit ROI, has zero erase-mask/block/sprite pixels, emits exactly one `han_only.unsupported_source_color:` warning, and yields RPC `CompletedWithErrors`.
- `ManualOverride` keeps the explicit user style and existing manual regression behavior. It must still pass complete source removal and successful-render checks but is excluded from source-color equality claims.

## Test-layer map

| Layer | Required coverage |
|---|---|
| Unit | T1 mask components/tri-state/forbidden subtraction/backend subsets; T2 frozen crop candidates, same-instance raw-device derivation, raw-selection metric recomputation, default-off feature metadata/negative evidence cases, backend-only `G/Csrc/S0/Smax/G0/G1`, geometry conversion/ownership/locality/maximal-rectangle oracle/raster fit/rotation reason, RenderControlsPanel Auto/empty state, retired UI estimators/fixed policies, unchanged manual/AllText, and PNG/JPEG/WebP allowlist parity; T3 unchanged public `StartPipelineRequest.region`/`PipelineRunOptions.region` type/serialization/`options_from_request` shape, truth-synced OpenAPI 400 behavior, exact unchanged public `EngineCtx`/`Engine::run` shape, HTTP pre-job and direct-run first-executable region rejection, repair role/descriptor pre-side-effect table, zero/one-producer DAG permutations, ambiguous-producer rejection, current-page task-local outside/wrong/prior/post-run failure, duplicate-page publication rejection, nested restore, spawn non-inheritance, reversible reflow/`S0`-`Smax` outcomes/style precedence/source-color extraction, expected/frozen/pre-inpaint-raster-entry translation reconstruction, cluster/control coverage, glyph-zero `.notdef`, nonzero out-of-range glyph error, legal no-ink controls, complete fill traversal, nonempty alpha, independently checked logical-sprite and actual-scale raster dimensions/bytes/`usize`, retained page cap, fallible surface rejection, peak-live-scratch `<=1`, per-target scratch release, one builder raster, zero Renderer raster, exact `J -> T -> Wterm -> Wmeta` work-reservation/boundary/overflow/P0-terminal/page-aggregation accounting, probe oracles, marker guard and inverse/mixed and nested Batch staging atomicity; policy-audit Rust token-tree/TypeScript AST exclusions, sanitized-export forbidden-field/value scans, synthetic line-count-coupling corpus, and exact-current-syntax rules |
| Integration | T0 layered pipeline/manifest/clean-reference capture; T2/B0 feature-enabled app-to-dependency evidence reachability, Source Gate post-load model-device/layer/buffer/MTMD evidence, per-inference context evidence, raw node/build evidence, process/instance binding, backend one-box fit/validator/warning flow, UI automatic non-authority, and format-allowlist parity; T3 HTTP region rejection before session/job/cancel/event/task effects, direct-run region rejection before registry/order/page/Scene/run-state/engine/warning/blob/history effects, task-local Arc/current-page object/sprite identity, sequential/concurrent/nested isolation, two-page `Scope::WholeProject` failure transition and run-state drop, per-run hint/color-mode lifecycle, accepted normal-pipeline zero/one-inpainter builder/publication matrix, guarded default-HanOnly repair execution only for Segment plus exact `[Inpainted]` with rejected engine classes stopped before side effects, complete P-1/P0 target/page work admission before erase, complete-text layout/glyph/fill/alpha/logical-sprite/actual-raster proof before inpainter run/apply, frozen-sprite Renderer persistence/composition, and payload/scratch release; T4 History/RPC `CompletedWithErrors`/import/unsupported complexity plus zero inpainter run/apply for every failure from either fully defined builder class, exact final equality to `B_prepare`, retained upstream commits, and zero erase/downstream persistence |
| End-to-end | T3 generated adversarial work-budget replay, reversed-order equality, task-local scope teardown and spawn non-inheritance, zero page entries without an inpainter, one immutable `PageId` object per qualifying page, cross-page isolation after engine failure, payload release/no cross-page pixel accumulation, at-most-one live raster scratch surface, per-target scratch release, and zero Renderer layout/raster rebuild; T5 one regression/four calibration/four holdout distinct full-page sources with every coverage axis, complete pre-inpaint translation/cluster/glyph/fill/nonempty-alpha proof, same frozen sprite at Renderer entry, pre-render residual oracle, exact source-color/width/work and persisted-input final-composite checks, and unsupported-color preservation; T6 pinned-AOT CPU/actual-Metal, repeats, fresh processes, final `IMPL_SHA` production desktop/UI build plus post-build-clean proof, one trusted-target closure, all-production-root policy audit, and human texture-only acceptance; T7 cross-platform common/generator-lock equality plus twice-reproduced PR target |
| Observability | Guarded T0/T5/T6 reports: Source/clean-reference/mask hashes, geometry, B0 same-instance Source Gate load/model/context/node evidence, crop selection artifact, residual metrics, Planner outcomes, color mode/reason/contract facts, `J/T/classifier_p0_phase`, classifier budget/core/candidate/width-probe/reserved/consumed/limit/limit-kind/elapsed facts, expected/frozen/pre-inpaint-raster-entry translation digests/counts, break/cluster/control transcript digests/counts, shaped/representable/missing/fill-visited glyph counts, nonempty-alpha, `B_prepare` equality/upstream-retention verdicts, logical sprite dimensions/bytes/reservation, actual raster scale/dimensions/surface bytes, peak-live-scratch and per-target scratch-release verdicts, frozen sprite identity, Renderer-zero-raster, payload-release and completion verdicts, unsupported warnings, protected overlap, cost, dirty baseline, and target-correlation mapping; sanitized `R/A`: correlation-ID-keyed non-length verdicts only; `C/G`/annotation/closure summary: no target text-proof field; T6 validator-derived Source Gate device plus actual AOT device/report containment/repeatability/unavailable prerequisite reporting |

## T4. Cross-policy and atomicity regressions

Run and preserve tests covering:

- unchanged `StartPipelineRequest.region`, `PipelineRunOptions.region`, serialization, and `options_from_request` field/conversion shape through the existing `http_options_inherits_source_text_policy` test, together with updated route comments, OpenAPI/Orval 400 behavior descriptions, and `docs/{en-US,ja-JP,zh-CN,pt-BR}/reference/http-api.md`;
- HTTP `start_pipeline` rejection of `req.region.is_some()` immediately after extraction and before session lookup, options/spec construction, job/cancel registration, event publication, or task spawn, with one HTTP 400 directing callers to the existing atomic page repair route;
- direct `pipeline::run` rejection of `spec.options.region.is_some()` as its first executable validation, before `infos_for_spec`, order construction, page/Scene access, run-Arc creation, engine load, warning emission, blob/history access, builder/publication, or mutation;
- HanOnly replaces only Han;
- complete Latin words remain protected;
- isolated `S` may remain part of `S型曲线` selection;
- pure English candidates are rejected;
- HanOnly zero-target cleanup and repair-region behavior;
- AllText behavior;
- invalid/missing target language behavior;
- explicit nonzero-rotation unsupported behavior with no rendered block/sprite, one warning per node, and RPC `CompletedWithErrors`;
- explicit unsupported-source-color behavior with pre-inpaint ownership removal, byte-identical Source/Inpainted/Rendered ROI, zero rendered block/sprite, one warning per node, and RPC `CompletedWithErrors`;
- explicit `ClassifierWorkBudgetExceeded` behavior at target and page limits with complete pre-erase admission, byte-identical Source/Inpainted/Rendered ROI, zero erase/block/sprite, one stable warning per node, RPC `CompletedWithErrors`, and reversed-order equality;
- page, protected-Latin, other-node, target-overlap, raster-geometry, and node-bijection validation;
- a translation-identity mismatch, final-layout failure after local fallback, cluster/control-coverage failure, glyph-validity failure, fill-traversal failure, or nonempty-alpha validation failure returns the existing deterministic hard pre-write `geometry_or_font` builder error; separately, only checked logical/actual-raster arithmetic or `usize` conversion failure, retained owner-rectangle/page-cap reservation failure, or fallible raster-surface-construction failure is a recoverable `geometry_or_font` failure. Both classes produce zero frozen-object publication, zero inpainter `engine.run`/`app.apply`, zero erase, and zero downstream Inpainted/Rendered/blob/sprite persistence. For each case the test first snapshots pre-upstream state, commits representative upstream producers, captures `B_prepare` immediately before the builder, and proves final Scene/History/reachable-blob/sprite state equals `B_prepare` while differing from pre-upstream state where a producer commit was expected; no upstream commit is rolled back. Process-level allocator OOM/abort is not injected or represented as recoverable.

Add one integration regression that injects a transient insertion-only Planner hint whose raster cannot fit. It must prove that deleting the inserted breaks reconstructs the exact original, local automatic fallback is attempted, and the final error occurs before inpainter run/apply with exact equality to `B_prepare`, retained upstream commits, and the full zero-downstream-effect tuple above. Add a second regression proving `protected_source_lines_for_page` does not reclassify this validated single-source-line transient reflow as protected Han. Extend the existing run-lifetime regression to prove the carrier accessor errors before and after scope and for wrong/prior/post-run pages; builder/inpainter/Renderer share one Arc, current-page frozen object, and frozen sprite identity; sequential and concurrent invocations use distinct Arcs; an inner same-task run shadows then restores the outer Arc; a spawned child cannot inherit/access the carrier; no state-derived mutable handle remains usable after the scoped future; and all page records drop with the run Arc. Its two-page `Scope::WholeProject` cases cover with and without Renderer plus page-one inpainter failure after publication followed by zero sprite/blob persistence, payload release, and independent page-two success; success and no-Renderer paths also release payloads with no cross-page accumulation.

Upgrade existing `repair_brush_engine_ctx_keeps_single_engine_path` without renaming it into a real route-level table regression using default `AppConfig` (`HanOnly`) and a nonempty region. Success requires the existing URL `MaskRole::Segment`, a known descriptor whose `produces` is exactly `[Artifact::Inpainted]`, the existing direct `EngineCtx` path, unchanged mask-plus-engine atomic behavior, zero builder/publication, and no task-local accessor invocation. In the same existing test, unknown ID, non-Segment role with an inpainter ID, `koharu-renderer`, Typography, and any multi/other-artifact descriptor each return the same stable HTTP 400 before body decode/blob persistence, Scene read/snapshot/apply, engine cache/load/run, or `app.apply`. Do not hardcode the three current inpainter IDs; descriptor truth is authoritative. This narrow validation intentionally rejects arbitrary dispatch but does not authorize a region-bearing `pipeline::run`, change `PutMaskParams.pipeline` type/serialization shape, or rewrite successful legal Segment repair behavior. OpenAPI snapshot/generated checks and all four localized HTTP API references must prove that the behavior/400 documentation changed consistently and points callers to the direct page-mask repair route.

Add focused lifecycle regressions for raw HTTP/MCP AddPage/AddNode/UpdateNode/nested Batch marker presence, allowed omitted-marker deserialization, direct core/history apply guards, Planner-owned persisted style followed by manual style, text-only edit, translation edit, undo, redo, untrusted project open, and archive import. Every implicit style clear must be present in the inverse patch. Add a run-lifetime test proving a HanOnly hint cannot survive a second pipeline run or application restart.

Add core and History atomicity regressions for a mixed Batch whose first child performs a valid node/style mutation and whose final child fails the explicit marker guard, plus a nested Batch whose late child fails a structural or marker invariant after earlier nested mutations. Snapshot the serialized/equality state of `Scene`, the caller's Batch op tree, History epoch, exact canonical-log bytes/hash/length, observable undo/redo behavior, caller response, and autosave notification before each attempt. Test both a strict-valid old generation and every currently accepted legacy-tail form: truncated length, truncated body, and undecodable trailing frame. Common faults cover coherent pre-permit History-then-Scene snapshot, authoritative `expected_epoch` capture, Scene-then-History staging-guard release, lock-free staging completion, permit acquisition, poison-only under-gate recheck, permit identity, Blob promotion and Blob-local release, exactly one permit-held post-Blob History acquisition and exactly one `current_epoch == expected_epoch` comparison, Scene acquisition and required-state revalidation, cloned apply/`prev`, staged undo/redo, frame serialization, canonical-prefix validation, task-temp `create_new`, partial/full write, flush, temp `sync_all`, strict replay-to-EOF, old-writer flush/sync/close, rename, inner-lock release order, synchronous success publication, and permit release. A deterministic stale-after-promotion case proves exact verified unreachable CAS only, poison false, and zero Scene/History/response/Dirty/autosave/job success publication. The adapter returns `Unchanged` only when exact raw-old bytes remain, a synced append writer reopens, and no task temp remains; every snapshot then stays unchanged and no history frame, response, Dirty, autosave, or job success is emitted. Unix-only faults cover parent-directory open/sync and exact-new verification after rename. Windows-only faults cover replacement with the prior writer relinquished, canonical reopen/sync, exact-new verification, append continuity, and temp absence. Any missing, partial, mixed, unexpected, unreadable, unsyncable, or ownership-uncertain result is `Indeterminate`; its ProjectSession caller stores `persistence_poisoned` under the still-held permit, returns exactly `project persistence is indeterminate; close and reopen the project`, and publishes no staged memory or success. The ordinary `session::tests::persistence_poison_lifecycle_contract` inventories apply/apply-if-epoch/undo/redo/snapshot/compact/autosave, bound public/direct Blob puts, RPC page import/mask repair, pipeline CAS/History, every response/Dirty/autosave/job publication, the exact 13-file direct-field caller closure, and a same-module structural check that `ProjectSession::open_untrusted` cannot write raw fields outside the approved internal gate/mutation path. Explicit barriers/channels prove two races: writer A passes the optional fast read then waits before the gate while writer B poisons under the gate, so A's under-gate read rejects before canonical I/O; or A holds the permit and pauses immediately before rename while B is proved blocked at the gate, so A completes canonical plus synchronous success before release and B poisons only afterward. Sleeps, elapsed-time assertions, a staging History/Scene guard crossing permit acquisition, any permit-held pre-Blob History acquisition or epoch read/validation, an absent or additional permit-held post-Blob History acquisition, an absent or additional permit-held `current_epoch == expected_epoch` comparison, gate acquisition under an inner lock, `.await` under permit, post-release success, permit/store identity mismatch, and any unenumerated former public-field/direct caller fail. Read-only Scene/export access still succeeds. Explicit close performs no write, joins autosave, releases the lock, and succeeds; reopen accepts only exact raw-old compatible replay or exact strict-new replay with the new frame applied once and creates the sole new unpoisoned session. A third sequence, task temp, `next_epoch + 1`, duplicate effect, probe leakage, fallback on the mutated probe, in-process poison reset, or post-poison success event fails. Successful Unix and Windows cases prove staged `prev`, platform-required durability evidence before publication, a usable append writer, and exact inverse/undo/redo round-trip. Fault injection never claims to prove physical power-loss survival beyond a successful host sync contract.

Add a pipeline integration fixture containing one finite nonzero-rotation Han node and one different non-rotation unsupported mixed-text node on the same page. Remove the page-start unsupported collection that precedes `EngineCtx`. After each `EngineCtx` construction and before `engine.run`, collect typed reasons and emit the rotation prefix through `ctx.warn`; after applied ops, use the same step's `EngineWarningSink` for newly created rotation reasons. Separate page-local `rotation_warned` from existing tracing `unsupported_seen`. Assert exactly one product warning for the rotated NodeId, zero product warnings for the other unsupported NodeId, unchanged rotated ROI, zero block/sprite, nonzero warning count, and RPC `CompletedWithErrors`.

Add target-limit, page-limit, and arithmetic-overflow atomicity regressions that snapshot Source/Inpainted/Rendered blobs, erase ownership, Scene, History, and queued ops before classifier admission. A valid complexity rejection preserves all target-local state and emits the stable unsupported result; an accounting invariant failure aborts the whole page with no write or partial color-mode map. No test uses sleep, timeout, or elapsed threshold as its oracle.

Add preflight-order atomicity regressions that snapshot pre-upstream state, run and commit representative upstream producers, capture `B_prepare`, then snapshot builder checkpoints before provisional source partition, before final fit/group/Planner-size resolution, before complete retained logical-sprite reservation, before each sole builder raster, and immediately before atomic page-object publication. Inject deterministic translation mismatch, final-layout failure after local fallback, cluster/control gap, glyph-zero, nonzero glyph overflow, fill error, and nonempty-alpha validation failure. Separately inject recoverable logical/actual-raster checked-arithmetic or `usize`-conversion failure, owner-rectangle/page-cap reservation failure, retained-reservation rejection, and fallible raster-surface-construction rejection. Each returns the same existing `geometry_or_font` builder error, proves exact final Scene/History/reachable-blob/sprite equality to `B_prepare`, proves upstream commits remain visible and therefore final state differs from the pre-upstream snapshot where expected, and has zero publication, inpainter run/apply, erase, or downstream persistence. No test treats allocator abort/OOM as recoverable. Independently prove a logical page reservation can pass while actual-scale raster arithmetic or fallible construction fails safely. Injected source-color or `F_s/W_s/F_t/W_t` representability failure yields the existing unsupported-preserve-ROI result with zero erase/block/sprite. A test-only publication/engine-order trace proves the complete frozen sprite object became visible before the first erase/inpaint operation, exactly one raster ran per successful target in the builder, retained logical bytes and actual-scale raster bytes are distinct fields, at most one target scratch surface is live, every scratch surface releases on success/failure before the next target, Renderer counters for translation/layout/shape/glyph/raster/fit/group/Planner-size/rounding/width are all zero, and every terminal path releases pixel payloads.

For every Renderer+Inpainter matrix row, add page-transaction fault injection at: pre-permit coherent History-then-Scene snapshot; `expected_epoch` and read-only staging-input capture; Scene-then-History staging-guard release; pre-gate async/staging completion; permit acquisition; poison-only under-gate recheck; every permit-aware sprite/final-CAS `durable_put_exact` stage, including common same-directory temp `create_new`, complete write/flush/temp `sync_all`, exact temp length/bytes/hash verification, and handle release; Unix rename, exact canonical verification, parent-directory open/`sync_all`; Windows overwrite rename, canonical reopen/file `sync_all`, exact canonical length/bytes/hash verification; existing-canonical reuse/repair, certain-owner temp cleanup, and unreachable-CAS audit; Blob-local lock release; exactly one permit-held post-Blob History acquisition and exactly one `current_epoch == expected_epoch` comparison; Scene acquisition and required-state revalidation; final Rendered persistence; cancellation immediately before gate acquisition; Batch cloned apply; complete History frame serialization; canonical-prefix validation; whole-log temp create/write/flush/`sync_all`; strict replay; old-writer relinquishment; canonical-log rename; platform-required post-rename sync; exact-byte verification; canonical-writer reopen; Scene then History release; synchronous job/Dirty publication; permit release; and temp cleanup. A deterministic stale-epoch-after-Blob-promotion case requires only exact verified unreachable CAS, poison false, and zero Scene/History/success publication. No Windows assertion claims parent-directory fsync. Blob/CAS faults occur before History replacement, preserve `B_prepare` reachable state, leave poison false, publish no success, and may add only exact verified objects to sorted `unreachable_cas`. Every History `Unchanged` case begins from `B_prepare`, runs inpainter and Renderer only against staging, and requires final Scene bytes, History epoch/canonical-log bytes/undo/redo, `reachable_blob_state`, sprite inventory, transforms, directions, and Rendered output to equal `B_prepare` exactly with no success. History `Indeterminate` alone stores `persistence_poisoned` under the permit before release; the session remains present for read-only access, every later writer rejects under the gate with the exact fatal reason, and explicit close/reopen follows the lifecycle above. Restart reads exactly one immutable canonical generation: strict-valid bytes replay to exact EOF on one fresh clone; only a malformed/truncated legacy trailing-frame failure may discard that probe and use compatible replay on a second fresh clone. Acceptance permits only raw-old/precommit or strict-new/staged state and rejects a staging guard crossing permit acquisition, any permit-held pre-Blob History acquisition or epoch read/validation, any count other than one permit-held post-Blob History acquisition plus one `current_epoch == expected_epoch` comparison, partial/mixed bytes, task-temp selection, probe reuse, double apply, `.await` under permit, inner-lock-to-gate acquisition, or post-release success. The success case proves the platform adapter returned `Committed` and all in-memory/synchronous success publication completed before the one permit released. `unreachable_cas` objects are enumerated separately with exact IDs/sizes/content and before/after delta, may never be reachable from Scene/replay/undo/redo, and do not enter equality; task-owned temps must be absent.

## T5. Real-image smoke and approved regression

### Approved `test.jpeg`

The image proves only:

- complete removal of the two top Han lines and three lower Han labels, including stroke/outline evidence;
- five complete English target sprites;
- unchanged protected Latin ROIs;
- no reproduction of the current incorrect output;
- no source/sprite/page overflow error.

It must not set font sizes, padding, region proportions, wrapping, grouping, or style constants.

### Multi-image smoke

The Revision 46 runtime-report contract requires `source-color-contract-v2`, `source-color-work-budget-v1`, both closed-preimage digests, all four work constants, accounting-table hash, closed complexity reason, `J/T/classifier_p0_phase`, all six P0 terminal tuples, conservative P1 requested/reserved plus independent actual consumption, exact page aggregation fields, selected producer order, builder/publication count matched to selected `Inpainted` (`0 -> 0`, `1 -> 1`), frozen `f_t_raw/F_t/W_t` on one-inpainter branches, expected/frozen/pre-inpaint-raster-entry translation digests/counts, reversible-break and cluster/control transcript digests/counts, shaped/representable/missing/fill-visited glyph counts, nonempty-alpha verdict, checked sprite dimensions/bytes/total/page cap, frozen sprite identity, payload-release verdict, exactly one builder raster per successful target, and zero Renderer layout/raster/recomputation. The external visual manifest remains an image/role/expectation input and does not contain either hash preimage. Missing, unknown, malformed, any pre-`J` traversal, incorrect P0 phase tuple, `preflight_consumed>preflight_reserved`, independent P1 counter disagreement, padding/synthetic inflation, incorrect page sums, incomplete pre-inpaint translation/cluster/glyph/fill/alpha/sprite-budget proof, `evaluation_consumed>evaluation_reserved`, `target_consumed>target_reserved`, `target_reserved>target_limit`, `page_consumed>page_reserved`, `page_reserved>page_limit`, wrong producer sequence, a builder/publication count that disagrees with the total matrix, Renderer without one inpainter, any Renderer translation/layout/shape/glyph/raster/fit/group/Planner-size/rounding/width work, payload leak/cross-page pixel accumulation, page/target disagreement, or elapsed-dependent verdict data fails before a cell result is accepted.

`HANONLY_VISUAL_MANIFEST` points to an external, user-approved absolute regular JSON file. Do not commit its images, masks, or clean references or encode their dimensions in production policy. The ignored existing pipeline harness validates this current Revision 50 schema before any crop candidate runs:

```json
{
  "version": 1,
  "entries": [
    {
      "id": "stable-local-id",
      "path": "/absolute/full-page-image",
      "sha256": "64-lowercase-hex",
      "decoded_rgba_blake3": "64-lowercase-hex",
      "clean_reference_path": "/absolute/page-sized-clean-reference",
      "clean_reference_sha256": "64-lowercase-hex",
      "clean_reference_decoded_rgba_blake3": "64-lowercase-hex",
      "role": "regression|calibration|holdout",
      "dimension_bin": "lt720|720_1439|1440_2159|gte2160",
      "aspect": "portrait|landscape|square_or_near",
      "background": "pure|gradient|texture|product|person",
      "targets": [
        {
          "id": "entry-local-target-id",
          "source_roi": [0, 0, 1, 1],
          "clean_reference_edit_roi": [0, 0, 1, 1],
          "erase_source_ink_mask_path": "/absolute/page-sized-binary-mask-a",
          "erase_source_ink_mask_sha256": "64-lowercase-hex",
          "residual_source_ink_mask_path": "/absolute/page-sized-binary-mask-b",
          "residual_source_ink_mask_sha256": "64-lowercase-hex",
          "position": "interior|page_edge",
          "writing": "horizontal|vertical",
          "effect": "plain|stroke|shadow|glow|decorative",
          "translation_length": "short|equal|2x|3x",
          "expected": "automatic_strict|manual_override|unsupported_source_color|unsupported_rotation"
        }
      ],
      "protected_rois": [[0, 0, 1, 1]],
      "multi_node": false
    }
  ]
}
```

Coordinates are evidence-only integer half-open page pixels. Every target has two page-sized binary source-ink masks, `M_erase` and `M_residual`, each with nonzero area and independently blind-annotated from immutable Source bytes by separate preparation lanes. Neither lane may read the other mask, production masks, runtime output, clean-reference pixels, OCR boxes beyond the declared `source_roi`, or shared derived thresholds. Each mask must cover every Han fill/stroke/shadow/glow/decorative pixel associated with that target, including ink outside the tight recognition ROI. Before any runtime output or clean-reference metric is opened, the validator requires exact dimensions, binary shape, nonempty area, and pixel equality `M_erase == M_residual`; disagreement fails closed and no cell runs. The agreed mask `M=M_erase|M_residual` is used for erase coverage and the first residual oracle. Each independently annotated `clean_reference_edit_roi` is page-clamped, contains `M`, and is disjoint from protected ROIs and every other target's edit ROI. Each clean reference is independently prepared and approved before runtime output is viewed; it has the exact Source dimensions, removes every `automatic_strict` and `manual_override` target, leaves every `unsupported_rotation` and `unsupported_source_color` edit ROI unchanged, and is pixel-identical to Source outside the union of successful-mode target edit ROIs. Before runtime output is opened, derive the second full-ROI oracle mask `M_delta={p in clean_reference_edit_roi | Source[p] != Clean[p]}` from the immutable Source/Clean buffers; require `|M_delta|>0` and `M subseteq M_delta`. `M_delta` is never derived from either blind mask or runtime output.

`effect: shadow` and `effect: glow` are valid only with `expected: unsupported_source_color` in Revision 46. A manifest that labels either effect successful fails before runtime. Coverage also requires at least one holdout `automatic_strict` stroke target whose independent topology oracle yields exactly one source width.

The ignored Rust harness and archive verifier have exactly three direct dependency edges. Add exact `[dev-dependencies]` in `crates/koharu-app/Cargo.toml`: `rustix = { version = "=1.1.4", features = ["fs"] }` and `sha2 = "=0.10.9"`. Add exact `[build-dependencies] sha2 = "=0.10.9"` in `crates/koharu-llm/Cargo.toml`; it is available only to `koharu-llm/build.rs` and verifies the pinned llama archive before extraction. All package records and checksums already exist in the current lock. `Cargo.lock` may change only the `koharu-app` dependency list by adding `rustix 1.1.4` and `sha2`, and the `koharu-llm` dependency list by adding `sha2`, with no new package record, version, source, or checksum. The two app edges must remain `dev` kind and the llm edge must remain `build` kind; none may become normal/root-workspace-wide. The integration/source-policy owner exclusively owns both Cargo-manifest hunks and `Cargo.lock`; the test-engineer owns harness use and read-only dependency-contract fixtures.

The harness uses `rustix::fs::{openat,mkdirat,renameat,statat,fsync}` with `OFlags::DIRECTORY|OFlags::NOFOLLOW`, `Mode`, and held `OwnedFd` values to walk/open/create one child at a time; it uses `sha2::{Digest,Sha256}` on immutable byte buffers. It opens every component from `/`, holds each parent descriptor until its child is opened, and opens the final visual manifest, Source, Clean, `M_erase`, and `M_residual` files with `NOFOLLOW`. For each file it reads the opened descriptor exactly once into an immutable byte buffer, computes raw SHA-256 from those bytes, and JSON-parses or image/mask-decodes from the same in-memory bytes; no path reopen is allowed between type/hash/dimension/decode checks or runtime use. Both blind masks are validated and compared before any runtime or clean-reference output is opened. The visual-manifest bytes must hash to `HANONLY_VISUAL_MANIFEST_SHA256`, and the fixed repository fixture manifest must hash to `HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256`, before their same byte buffers are parsed. Runtime/process/report/artifact children are created with the same `rustix` descriptor primitives only after all inputs validate. Replacement-race fixtures swap every parent/final component after initial lookup but before read/decode/output creation and require fail-closed behavior or continued use of the already-open object with zero report directory, model, inference, blob, or Scene side effects.

`scripts/check-hanonly-production-policy.ts --test-dependency-inventory` parses Cargo metadata, current `Cargo.lock`, and the D0 `pre-edit-Cargo.lock` snapshot after rechecking the snapshot's recorded SHA-256/owner/mode/type. It requires exactly the two app direct-dev edges and one llm build-only edge, exact versions/features/checksums, exactly the two allowed additions to the `koharu-app` dependency list plus one allowed addition to the `koharu-llm` dependency list relative to the parsed baseline, and byte-for-byte-equivalent package/version/source/checksum records elsewhere; no normal/root-workspace declaration, misplaced build/dev edge, or new lock package is allowed. Table fixtures reject a missing or mismatched baseline snapshot, missing edges, semver drift, missing/extra features, promotion to normal/root-workspace dependency, llm `sha2` outside build dependencies, any unrelated lock-list change, an added package record, source/checksum drift, and any fourth direct edge. The pre-B0 and final matrices run this mode; rollback removes all three manifest edges, their three lock-list additions, archive verification, and harness imports/tests together.

Validation order is fail-closed: schema/path/hash/dimensions, then both blind masks' binary shape and nonempty area, then exact `M_erase == M_residual`, then edit-ROI/protected/disjointness, then successful-mode Source/Clean discrimination and unsupported-mode equality, and only then runtime cells. The validator rejects unknown/missing fields or expectation values, relative/missing image/mask/clean-reference paths, hash/dimension mismatch, duplicate Source or clean-reference decoded hashes, a clean reference equal to Source for an entry with successful-mode targets, an empty/nonbinary/wrong-sized blind mask, equality disagreement between the two masks, a successful-mode agreed-mask pixel where Source and Clean are identical, a mask outside its edit ROI, overlapping target agreed masks/edit ROIs, clean-reference changes outside successful-mode edit ROIs, clean-reference changes inside protected ROIs or unsupported-mode edit ROIs, duplicate entry/target IDs, overlap between calibration and holdout hashes, any role count other than exactly one regression/four calibration/four holdout, or a regression entry whose `decoded_rgba_blake3` differs from the approved decoded-pixel identity. The selected regression input raw SHA-256 must match that entry's `sha256`; the JPEG and decoded-equivalent WebP are alternative container representations of one decoded regression identity and never count as two entries.

Runtime node-to-manifest matching is a deterministic one-to-one bipartite match. Rasterize each runtime recognition anchor and manifest `source_roi` to integer half-open boxes. An edge exists only when each box center lies inside the other box. The graph must have exactly one perfect matching; zero or multiple perfect matchings fail before candidate selection. Lexicographic target ID/NodeId ordering is used only for stable diagnostics, never to resolve ambiguity. This matching is evidence-only and never becomes production policy.

Machine oracles:

- For every axis-aligned target, Source Gate selects the matched node as a Han target on both CPU and actual Metal; it is not protected/rejected. The later source-color checkpoint, not Source Gate, decides whether an axis-aligned target is a successful or unsupported color mode.
- For every `automatic_strict` or `manual_override` target, `M_erase` and `M_residual` must have passed the blind exact-agreement gate before runtime output is opened. The final erase mask covers 100% of the agreed union `M` and overlaps no protected ROI or another target's agreed mask.
- Before target text is composited, the Inpainted layer passes the same globally frozen clean-reference residual function independently over agreed mask `M` and full-edit-ROI delta mask `M_delta` on every `automatic_strict` and `manual_override` target and device/repeat. For each `X in {M,M_delta}`, require `|X|>0`, define normalized per-pixel RGBA L1 distance `d(a,b)=(|ar-br|+|ag-bg|+|ab-bb|+|aa-ba|)/(4*255)`, `Dsrc_X=mean_X d(Source,Clean)`, `Dout_X=mean_X d(Inpainted,Clean)`, and `source_closer_fraction_X=count_X[d(Inpainted,Source)<=d(Inpainted,Clean)]/|X|`. Require `Dsrc_X>=0.08` before division, `Dout_X/Dsrc_X<=0.20`, `source_closer_fraction_X<=0.05`, and largest 8-connected component of those source-closer pixels at most `max(1,floor(0.01*|X|))`. As a conjunctive only-reject supplement, run the existing pinned Source Gate OCR model and its production-fixed detector/recognizer configuration on the pre-composite Inpainted `clean_reference_edit_roi`; map each candidate to integer half-open page pixels and reject any positive-area candidate whose box intersects `M_delta`. OCR cannot make either pixel oracle pass. All thresholds, connectivity, model identity, and OCR configuration are frozen before any calibration, holdout, regression, CPU, or Metal output is opened and cannot vary by entry. An empty/disagreeing blind mask, invalid `M_delta`, failed `Dsrc_X`, pixel-oracle failure, or OCR hit aborts the target before any cell pass is emitted.
- Every `automatic_strict` and `manual_override` target produces exactly one matching rendered block/sprite with a nonempty alpha bbox and a final Rendered image node. Before any inpainter run/apply, `expected_translation_utf8_sha256` must equal the digest reconstructed from the complete translated target text; `frozen_renderer_input_utf8_sha256` and legacy-named `renderer_entry_utf8_sha256` plus byte/scalar counts must prove that exact text, or only audited insertion-only newline breaks whose transcript reverses to that exact text, entered the builder's shared layout/raster path. `layout_cluster_coverage_sha256` must prove every non-control input byte is covered by canonical shaped-cluster intervals and every newline/legitimate no-ink whitespace byte appears exactly once in deterministic control records. Glyph ID `0` for any non-control cluster is missing `.notdef` and rejects before publication/inpaint; every remaining nonzero shaped glyph ID must fit `u16`. `fill_raster_visited_glyph_count` must equal `shaped_glyph_count`, with `missing_glyph_count=0`, `cluster_coverage_complete=true`, `fill_raster_visit_complete=true`, and nonempty alpha. Before retained sprite allocation/raster, each `logical_sprite_bytes_i=checked(logical_width_i*logical_height_i*4)` checked-converts to `usize`, stays within its disjoint owner rectangle, and the complete canonical retained set is at most `checked(page_width*page_height*4)`. For the actual existing 2-4x scale, independently checked-compute raster width/height/surface bytes and `usize` conversion, construct through the fallible surface path, keep at most one target scratch surface live, and release it before the next target. Every alpha pixel passes existing page/protected/other/target safety; the alpha-bbox center satisfies source-anchor locality; selected size satisfies `S0 <= selected <= Smax`. The page object publishes atomically only after all targets pass. After `Inpainted`, Renderer must receive the same frozen sprite identity, persist/composite it once, execute zero translation/layout/shape/glyph/raster/fit/group/Planner-size/rounding/width work, and release the pixel payload. `AutomaticStrict` additionally requires exact source-contract fill/stroke RGBA, exact independently recomputed `F_s/W_s/f_t_raw/F_t/W_t`, proof that the complete frozen preflight object was published before the first erase/inpaint operation, and passing fill-only/stroke-only/antialias probes; `ManualOverride` is excluded only from source-color/width equality.
- For every shaped cluster/run whose production classification is ink-bearing, the builder records the alpha-pixel set contributed by that cluster/run after clipping and downsample and requires it to be nonempty in the final logical sprite. Ligatures and combining sequences are evaluated as their shaped cluster/run group, not guessed per Unicode scalar or per glyph. Legal newline/control/whitespace records are the only zero-contribution class and must not carry an ink-bearing glyph. A whole-sprite nonempty alpha result cannot compensate for one omitted ink-bearing cluster/run.
- The test-only protected-support oracle computes `P` before reading any runtime protected mask, classification, report, block, sprite, or Rendered output. Its only inputs are the persisted Source Scene, page dimensions, page-node order, each node's visibility/detector/transform/TextData, and the frozen HanOnly policy. Detector `pp-ocr-v5-source-gate-protected` wins visibility and contributes the node's complete transformed/rotated AABB clipped to the page. Every other invisible node contributes nothing. Source lines are split and trimmed while retaining original indices; Han membership uses ICU Script=Han, and a protected Latin word is at least two Latin alphabetic code points with only internal hyphen/apostrophe. Any source line containing both Han and a protected Latin word makes the whole node ineligible and protected. A remaining node is geometrically eligible only when its bbox is finite/positive, its text rotation is zero, and its transformed bbox clips nonempty to the page. Nonempty source-line count must equal line-polygon count when polygons are used; each polygon has exactly four finite axis-aligned points, nonzero area, and a bbox intersecting both node bbox and page. Mixed Han/non-Han nodes require valid polygons and protect every non-Han line bbox; only Han line bboxes are eligible. Pure-Han nodes use valid per-line polygons when present and otherwise use the node bbox for every Han line. Missing/empty eligibility, invalid mixed-node polygons, nonzero rotation, or nonempty translated-line count unequal to eligible Han-line count protects the whole node. Rasterization uses page-clipped integer half-open boxes with `left/top=floor`, `right/bottom=ceil`, inclusive pixels `[left,right) x [top,bottom)`, and OR union. Unknown, duplicate, overlapping, or unclassifiable ownership fails before `P` is emitted. A runtime `P` or protected-node list is comparison evidence only.
- Decode the actual persisted Source, Inpainted base, optional Brush, ordered sprite, and final Rendered blobs to canonical RGBA8. Starting from the same Inpainted base for every replay, a test-only compositor executes exactly `replace independently derived P with Source RGBA -> optional Brush at (0,0) -> sprites in persisted page-node production order`; the persisted order filtered to successful target NodeIds must be a bijection with the report. It independently implements pinned `image 0.25.10` RGBA source-over semantics without production Renderer, `imageops::overlay`, or the production protected-support helper: top alpha `0` preserves, `255` replaces, otherwise use the same premultiplied `f32` source-over and `NumCast<u8>` truncation. Full-page decoded RGBA must equal final Rendered exactly. For each successful target, start again from the same base and omit only that target during the forward replay; never delete it from a completed image. The omission output must differ from final Rendered in at least one pixel inside that target's safe region. Missing/unreadable inputs, reading runtime outputs before hashing `P`, using runtime `P`, wrong restore/brush/sprite order, duplicate/reordered sprite, container-byte comparison instead of decoded RGBA, transform drift, composite mismatch, or omission equality fails.
- Every protected ROI is pixel-identical between Source and final Rendered.
- For `unsupported_rotation`, Source, Clean, Inpainted, and Rendered are pixel-identical throughout the target edit ROI, the final erase mask has zero pixels there, there is no rendered block/sprite, no residual division is attempted, the private reason is `Rotation`, the exact warning prefix occurs once, `warning_count > 0`, and RPC status is `CompletedWithErrors`.
- For `unsupported_source_color`, Source, Clean, Inpainted, and Rendered are pixel-identical throughout the target edit ROI, the final erase mask has zero pixels there, there is no rendered block/sprite, no residual division is attempted, one closed `SourceColorReason` is recorded, the exact warning prefix occurs once, `warning_count > 0`, and RPC status is `CompletedWithErrors`.
- Human review is authoritative only for successful-mode inpaint texture plausibility and can reject a machine pass for implausible texture. Residue, classification, geometry, clipping, size, style/color/width, unsupported preservation, and final composition remain machine-only verdicts and cannot be waived or replaced by human judgment.

Coverage is conjunctive, not “any six tags.” Across the nine distinct full-page hashes the manifest must include:

- all four dimension bins;
- portrait, landscape, and square-or-near aspect;
- at least one pure/gradient background and at least one texture/product/person background;
- interior and page-edge targets;
- horizontal and vertical writing;
- plain plus at least one stroke/shadow/glow/decorative target;
- short, equal, 2x, and 3x translation lengths;
- at least two entries with nonempty protected Latin ROIs;
- at least two multi-node entries;
- at least one `automatic_strict`, one `manual_override`, one `unsupported_source_color`, and one `unsupported_rotation` target across the nine entries;
- at least one `automatic_strict` and one `unsupported_source_color` target in holdout;
- the four holdout entries collectively include page-edge text, nonempty protected ROIs, at least two dimension bins, and a texture/product/person background.

Candidate IDs/formulas, manifest roles, masks, clean references, thresholds, raw-evidence schema, audit rules, and the default-off `hanonly-test-evidence` bridge are frozen in D2 before calibration output. D2 records the current CPU-context/offload observability gap, cross-crate `cfg(test)` reachability gap, cfg/build-script mismatch risk, missing dedicated B0 command, and release-feature-inventory gap as RED and retains no production fix. GREEN-B subgate B0 is the first retained Source Gate / PP-OCR recall-forward hunk: it adds the exact propagated feature, makes `PaddleOcrVl` carry the existing CPU request into context construction, explicitly sets `offload_kqv=false` and `op_offload=false` for CPU (`true` for GPU-requested execution), exposes same-instance cross-crate backend evidence only under feature-only `#[cfg(feature = "hanonly-test-evidence")]`, and may change only Source Gate detection recall, crop-local preprocessing/upscale, inverse mapping, source coverage acceptance, and source-removal preflight. B0 changes no target layout, font, renderer, UI, color-migration, or composition policy. First prove featureless dependency tests and default workspace/release inventories omit the feature, then prove the feature-enabled app harness reaches a real dependency snapshot. With B0 fixed and no B1 target layout/font/render/UI/color hunk present, run the required anti-fixture check, dedicated Source Gate-only `calibration-freeze` process, the same required check again before holdout model load, then a fresh `holdout` process using one feature set and one selection executable. Only a passing frozen holdout permits GREEN-B subgate B1 to consume the frozen recall contract and accepted source-coordinate boxes/anchors; B1 may not read or redefine selected candidate ID, candidate geometry terms, crop padding, OCR scale, recall thresholds, Source Gate acceptance, or crop-local preprocessing. Any feature leak, cfg/build mismatch, release inventory, reachability, command/env, calibration, device, required-check, or holdout failure rolls back B0 and ends the revision.

Add the exact default-off bridge before B0. `koharu-llm` and `koharu-ml` each define `hanonly-test-evidence = []`; `koharu-app` defines `hanonly-test-evidence = ["koharu-llm/hanonly-test-evidence", "koharu-ml/hanonly-test-evidence"]`; none includes it in `default`. Every dependency observational struct/accessor/log hook consumed across crates, including all llama-ext-dependent items, is enclosed by feature-only `#[cfg(feature = "hanonly-test-evidence")]`, is public only inside that gate so the app harness can call it, exposes no setter or behavior branch, and is absent from default/ordinary dependency-unit-test artifacts. Dependency-local non-FFI helpers may remain `#[cfg(test)]`. The app ignored harness is `#[cfg(all(test, feature = "hanonly-test-evidence"))]`. `koharu-llm/build.rs` includes/binds the existing staged `llama-ext.h` exports only when `CARGO_FEATURE_HANONLY_TEST_EVIDENCE` is set, exactly matching every FFI consumer. The feature-enabled real PaddleOCR-VL load/inference path exposes raw post-load evidence from the same instance, and Source Gate diagnostics expose every runtime node's NodeId, integer half-open recognition anchor, node/text rotation, and selected-as-Han bit. A process-global llama/ggml/mtmd log capture is allowed only in the feature-enabled dedicated ignored `--test-threads=1` harness, is serialized by one mutex, stores the raw log beneath the evidence root, and fails closed on missing/duplicate/unparsed backend records. It records actual offloaded-layer counts, model-buffer bytes by backend, the MTMD/CLIP backend, and separate context/compute-buffer bytes by backend. The feature-gated cross-crate accessor uses the existing llama staging APIs `llama_model_n_devices`/`llama_model_get_device` on the loaded model to record actual model devices. No component emits an authoritative `actual_device` or pass metric. The validator matches runtime anchors to manifest ROIs with the unique mutual-center rule, derives the actual device class only from the post-load/post-inference facts, and recomputes all target/protected/rotation sets and metrics. For crop selection, a candidate passes only when every axis-aligned target whose final expectation is `automatic_strict`, `manual_override`, or `unsupported_source_color` is selected as Han, no runtime node whose center lies in a protected ROI is selected as Han, every rotation target remains excluded, Source Gate coverage covers the complete source-text ROI needed for removal, PP/VL coverage is complete, neither `rejected_after_vl` nor `pp_vl_incomplete_coverage` is present, and B0 source-removal preflight passes, on all four calibration entries and both derived devices. Full final Rendered erase/render/source-ink/residual/alpha/locality/warning oracles run later under the frozen selected policy after GREEN-A/B1/C. Select the smallest recomputed all-pass candidate.

The dedicated B0 filter is `han_only_source_gate_crop_selection_matrix`; it never invokes inpaint, layout, renderer, Planner, or Scene mutation. It accepts only:

- `HANONLY_SOURCE_GATE_SELECTION_PHASE`: `calibration-freeze` or `holdout`;
- `HANONLY_B0_SHA`: exact lowercase 40-hex detached pre-B1 commit, equal to local `HEAD`;
- `HANONLY_SOURCE_GATE_SELECTION_ARTIFACT`: absolute path below the canonical evidence root; it must not exist for `calibration-freeze` and must be the fsynced regular file produced by that phase for `holdout`;
- `HANONLY_SOURCE_GATE_SELECTION_REPORT_DIR`: absolute directory below the canonical evidence root;
- `HANONLY_SOURCE_GATE_REQUIRED_CHECK_ATTESTATION`: absolute regular canonical anti-fixture attestation path below the evidence root for the current phase;
- `HANONLY_R50_CALIBRATION_MANIFEST` and `HANONLY_R50_CALIBRATION_LEDGER`: absolute descriptor-validated regular files below the evidence root, required in both phases;
- `HANONLY_R50_HOLDOUT_MANIFEST` and `HANONLY_R50_HOLDOUT_LEDGER`: absent in `calibration-freeze` and required as absolute descriptor-validated regular files below the evidence root in `holdout`.

`calibration-freeze` rehydrates and validates only the D0 calibration manifest/evidence ledger, same-descriptor revalidates the fixed repository Source Gate fixture-manifest hash, and descriptor-opens the pre-calibration required-check attestation before model load. It requires no existing artifact, runs all `4 entries x 2 devices x 4 candidates`, writes the calibration artifact with `holdout_manifest_sha256:null`, `holdout_entry_ids:[]`, and `terminal_diagnostic_index:null`, freezes the candidate/recall contract and calibration diagnostic generation, fsyncs it and its parent directory, and exits. Only then an independent no-model `seal-holdout` operation creates the immutable holdout-extension manifest and its separate ledger/hash; it cannot rewrite the calibration manifest or artifact. `holdout` descriptor-validates both manifests/ledgers and the pre-holdout attestation before model load, requires the calibration artifact and frozen projection unchanged, runs `4 entries x 2 devices` for the selected candidate, and appends only `holdout_manifest_sha256`, four `holdout_entry_ids`, the pre-holdout check, holdout process/results, terminal diagnostic binding, and completion timestamp. Unknown/missing phase or B0 SHA, local-head mismatch, relative/root/escaping/symlinked output path, existing nonregular artifact, report path outside the root, dirty fixture manifest, missing/unreadable/wrong-phase/wrong-B0/wrong-endpoint/wrong-result attestation, either manifest/ledger hash drift, a holdout seal before candidate freeze, or any calibration artifact mutation fails before model load. The exact two model processes plus intervening no-model seal exist only in the `b0` stage of the Required command matrix; standalone or duplicate invocation is forbidden.

For Revision 50, the calibration and holdout manifest/ledger pairs are explicit filter inputs, not inferred ambient state. Missing, swapped, substituted, symlinked, or hash-mismatched pairs reject; holdout inputs in calibration reject, and absent holdout inputs in holdout reject.

Revision 50 also supersedes the older unconditional “artifact absent” and duplicate-invocation wording with one closed resume state machine inside the same `b0` matrix. A fresh run requires artifact absence. If it exists, `--classify-b0-resume-state` descriptor-validates exact B0/candidate/calibration generation/frozen projection and returns only `calibration_frozen`, `seal_incomplete`, `seal_complete_holdout_unopened`, `holdout_complete_authorization_missing`, `holdout_complete_authorization_incomplete`, or `authorized`. It rejects partial/failed/unknown holdout, changed candidate/frozen fields, illegal files, and every unclassified state. Resume skips completed phases and never reruns a formal holdout cell. `authorized` idempotently revalidates the record and deterministically re-emits artifact and authorization-record digests.

Authorization publication has one additional closed state `holdout_complete_authorization_incomplete`, represented only by mode-0600 `.b0-authorization.<expected-record-sha256>.tmp` whose embedded hash, bytes, length, owner, mode, and type exactly match the recomputed canonical `hanonly-r50-b0-authorization-v1` record while final is absent. The only allowed authorization starts are both absent, that exact temp only, or the exact final only. Exact-temp resume removes only that task-owned temp and republishes authorization; exact-final resume revalidates and returns `authorized`. Temp+final, wrong hash/name/bytes/length, symlink/nonregular/wrong owner or mode, or any unknown entry rejects. Fault tests cover create, partial/complete write, file fsync, rename, final-file fsync, and parent-directory fsync and prove convergence without rerunning any formal holdout cell.

Fresh execution first invokes the frozen checker endpoint `--admit-b0-preflight-state`, which opens the evidence-root parent descriptor and uses descriptor-relative `lstat`-equivalent no-follow inspection for the contract's entire pre-B0-owned path set before the first gate. It returns only `fresh` when every final, artifact, cargo directory, complete `source-gate-selection` tree, red log, deterministic temp, and symlink is absent, or `preflight_complete` after descriptor-validating the existing attestation and every bound log plus classifying every existing owned-tree entry against the contract's closed legal phase states. Any other output rejects. Attestation absence with any owned path, or any symlink including a dangling symlink, is crash-partial and rejects without overwrite. Fresh execution atomically publishes mode-0600 `b0-preflight-attestation.json` only after every pre-B0 gate and staged-RED log passes. Its closed `hanonly-r50-b0-preflight-v1` bytes bind B0 SHA, current contract/plan/test hashes, all eight frozen interpreter hashes, every ordered gate result, and every red-list/run-log hash. Any resume, including preflight-complete/artifact-absent, must descriptor-validate this attestation, logs, clean B0 worktree, current HEAD, and every legal owned-tree entry before skipping the entire pre-B0 block. Missing, incomplete, changed, unknown, or collision state rejects; resume never recreates or overwrites pre-B0 logs. No shell path creates an owned directory or redirects output into it. Each checker or Rust harness writer holds the evidence-root descriptor, creates each absent child with `mkdirat`, reopens it with `O_DIRECTORY|O_NOFOLLOW`, verifies current-user ownership, mode 0700, type, and closed entries, writes through descriptor-relative create-new/fsync publication, and fsyncs child and parent. Tests include preexisting regular-file, nonempty-directory, symlink, and dangling-symlink collisions for every owned-path class, plus namespace replacement after admission and before each creator/writer; each rejects before the first gate or model load with zero change outside the evidence root, or continues only through already-held descriptors.

Both processes run from detached clean `B0_WORKTREE`, require `git rev-parse HEAD == B0_SHA`, hash their actual test executable and require equality; `enabled_cargo_features` is the sorted exact list `["hanonly-test-evidence","metal"]`. B1 authorization is the successfully fsynced post-holdout artifact bound to `B0_SHA`, both descriptor-verified anti-fixture attestations, its descriptor-derived whole-file SHA-256 recorded in durable execution state, and a validator exit code of zero, never the existence of a selected ID alone.

`B0_FROZEN_INTERPRETER_PATHS` is a closed ordered array containing exactly `.omx/plans/hanonly-r50-b0-evidence-contract.json`, `scripts/check-hanonly-production-policy.ts`, `scripts/check-hanonly-production-policy.test.ts`, `scripts/hanonly_evidence_ledger.py`, `scripts/hanonly_evidence_ledger_test.py`, `package.json`, `ui/package.json`, and `bun.lock`. The first path is the exact evidence contract; the next four files contain the B0 validator/ledger semantics and their tests; the final three bind Bun workspace resolution, the TypeScript parser declaration used by the checker, and its locked dependency graph. The artifact frozen projection binds the contract path, exact byte length, and SHA-256. Checker and ledger code may not import another repository-local helper. If such an import becomes necessary, execution stops and a new reviewed revision must extend the closed set before B0.

The checker mode `--validate-red-test-state b0` parses every tracked Rust file and requires the sixteen normative IDs exactly once. Under Revision 50, the two B0/G004-owned IDs `hanonly_pre_b1_red_t2_source_gate_ratio_contract` and `hanonly_pre_b1_red_t2_crop_local_ppocr_contract` must be unignored and executable at `B0_SHA`; the remaining five T2 IDs keep only their exact immediately adjacent `#[ignore = "hanonly-pre-b1-red"]`, and the nine T3 IDs keep only their exact immediately adjacent `#[ignore = "hanonly-pre-greenc-red"]`. It rejects duplicate/missing IDs, unknown IDs using either reason, a bare or alternate ignore on a normative ID, a B0-owned ID still ignored at `B0_SHA`, or a marker covering an existing test/module/crate. The mode `--validate-red-test-state final` requires the same sixteen IDs exactly once, rejects either marker in any tracked Rust token or comment, and rejects any normative ID with any `#[ignore]`. The checker/test source may contain marker strings only as TypeScript test data and is not a Rust marker hit. The final default workspace test command then proves the unignored tests execute normally.

The checker mode `--b0-source-gate-anti-fixture` requires `HANONLY_B0_SHA`, `HANONLY_B0_REQUIRED_CHECK_PHASE=pre-calibration|pre-holdout`, and absolute `HANONLY_B0_REQUIRED_CHECK_ATTESTATION_OUT` below the evidence root. It writes/fsyncs one canonical UTF-8 JSON attestation containing exactly `version:1`, `mode:"b0-source-gate-anti-fixture"`, `phase`, `b0_sha`, `calibration_manifest_sha256`, `holdout_manifest_sha256`, `source_gate_fixture_manifest_sha256`, `checker_endpoint_sha256`, `scanned_roots`, `allowed_descriptor_roots`, `policy_scan_sha256`, and `result:"pass"`. `holdout_manifest_sha256` is exactly null for pre-calibration and required lowercase SHA-256 for pre-holdout; the calibration hash is required and identical in both. Unknown fields, timestamps, dirty endpoint bytes, wrong phase, wrong B0 SHA, wrong manifest hash/nullability, wrong fixture-manifest hash, wrong root list, or non-pass result fail. `policy_scan_sha256` covers the same closed fields and ordered verdicts. `han_only_source_gate_crop_selection_matrix` descriptor-opens the phase attestation before model load, verifies its schema plus phase-appropriate manifest hashes against the two ledgers, computes its whole-file SHA-256, and retains that digest for the artifact. `calibration-freeze` writes the pre-calibration attestation digest into the calibration artifact after the phase succeeds. After `seal-holdout`, `holdout` first validates the existing pre-calibration entry, then descriptor-opens and hashes the two-manifest pre-holdout attestation before model load, and appends that digest only after holdout succeeds. `--validate-b0-authorization` descriptor-opens both attestations and both manifest ledgers, verifies hashes/schema, recomputes the frozen checker endpoint, independently reruns the static scan, and rejects if either attestation would not be reproduced.

The checker additionally requires explicit `HANONLY_CALIBRATION_LEDGER` with `HANONLY_CALIBRATION_MANIFEST` in both phases and explicit `HANONLY_HOLDOUT_LEDGER` with `HANONLY_HOLDOUT_MANIFEST` only in pre-holdout. Final authorization takes all four paths as required CLI arguments. Missing, swapped, substituted, path-escaped, symlinked, wrong-owner/mode, or manifest-ledger hash mismatch rejects before any attestation or artifact acceptance.

The final post-holdout `crop-policy-selection.json` under the evidence root has this exact logical shape. Pre-holdout, fsync the same schema with only the fields permitted below before any holdout output is opened:

```json
{
  "version": 2,
  "plan_revision": 50,
  "b0_sha": "40-lowercase-hex",
  "calibration_manifest_sha256": "64-lowercase-hex",
  "holdout_manifest_sha256": "64-lowercase-hex-or-null-before-seal",
  "holdout_custody_attestation_sha256": "64-lowercase-hex-or-null-before-seal",
  "source_gate_fixture_manifest_sha256": "64-lowercase-hex",
  "evidence_contract": {
    "path": ".omx/plans/hanonly-r50-b0-evidence-contract.json",
    "byte_length": "positive-integer",
    "sha256": "64-lowercase-hex"
  },
  "calibration_diagnostic_index": {
    "relpath": "source-gate-selection/diagnostic-index.generations/00000064.json",
    "generation": "positive-integer",
    "byte_length": "positive-integer",
    "sha256": "64-lowercase-hex"
  },
  "terminal_diagnostic_index": {
    "relpath": "source-gate-selection/diagnostic-index.generations/00000080.json-or-null-before-holdout",
    "generation": "larger-positive-integer-or-null-before-holdout",
    "byte_length": "positive-integer-or-null-before-holdout",
    "sha256": "64-lowercase-hex-or-null-before-holdout"
  },
  "image_input_contract_sha256": "64-lowercase-hex",
  "source_color_contract_sha256": "64-lowercase-hex",
  "color_constant_set_sha256": "64-lowercase-hex",
  "requested_devices": ["cpu", "metal"],
  "enabled_cargo_features": ["hanonly-test-evidence", "metal"],
  "backend_evidence_parser_version": 1,
  "required_checks": [
	    {
	      "phase": "pre-calibration",
	      "command": "bun scripts/check-hanonly-production-policy.ts --b0-source-gate-anti-fixture",
	      "checker_endpoint_sha256": "64-lowercase-hex",
		      "calibration_manifest_sha256": "64-lowercase-hex",
		      "holdout_manifest_sha256": null,
	      "source_gate_fixture_manifest_sha256": "64-lowercase-hex",
	      "attestation_relpath": "source-gate-selection/checks/pre-calibration.json",
	      "attestation_sha256": "64-lowercase-hex",
	      "b0_sha": "40-lowercase-hex",
	      "result": "pass"
	    },
	    {
	      "phase": "pre-holdout",
	      "command": "bun scripts/check-hanonly-production-policy.ts --b0-source-gate-anti-fixture",
	      "checker_endpoint_sha256": "64-lowercase-hex",
		      "calibration_manifest_sha256": "64-lowercase-hex",
		      "holdout_manifest_sha256": "64-lowercase-hex",
	      "source_gate_fixture_manifest_sha256": "64-lowercase-hex",
	      "attestation_relpath": "source-gate-selection/checks/pre-holdout.json",
	      "attestation_sha256": "64-lowercase-hex",
	      "b0_sha": "40-lowercase-hex",
	      "result": "pass"
	    }
  ],
  "frozen_recall_contract": {
    "candidate_set": ["S25L4", "S25L5", "S25L6", "S25L7"],
    "selected_candidate_id": "one-candidate-id",
    "ppocr_crop_local_preprocessing_sha256": "64-lowercase-hex",
    "inverse_mapping_rule_sha256": "64-lowercase-hex",
    "coverage_acceptance_rule_sha256": "64-lowercase-hex",
    "source_removal_preflight_rule_sha256": "64-lowercase-hex"
  },
  "candidates": [
    { "id": "S25L4", "short_side_numerator": 1, "short_side_denominator": 4, "long_side_numerator": 1, "long_side_denominator": 25 },
    { "id": "S25L5", "short_side_numerator": 1, "short_side_denominator": 4, "long_side_numerator": 1, "long_side_denominator": 20 },
    { "id": "S25L6", "short_side_numerator": 1, "short_side_denominator": 4, "long_side_numerator": 3, "long_side_denominator": 50 },
    { "id": "S25L7", "short_side_numerator": 1, "short_side_denominator": 4, "long_side_numerator": 7, "long_side_denominator": 100 }
  ],
  "calibration_entry_ids": ["four-distinct-ids"],
  "holdout_entry_ids": ["four-other-distinct-ids"],
  "process_evidence": [
    {
      "id": "phase-device-process-id",
      "phase": "calibration|holdout",
      "requested_device": "cpu|metal",
      "paddle_instance_id": "128-bit-hex-created-at-load",
      "executable_sha256": "64-lowercase-hex",
      "model_artifact_sha256": {
        "pp_detection": "64-lowercase-hex",
        "pp_recognition": "64-lowercase-hex",
        "pp_recognition_config": "64-lowercase-hex",
        "vl_model": "64-lowercase-hex",
        "vl_mmproj": "64-lowercase-hex"
      },
      "runtime_library_sha256": {
        "absolute-loaded-library-path": "64-lowercase-hex"
      },
      "load_evidence": {
        "cpu_forced": true,
        "gpu_offload_supported": false,
        "n_gpu_layers": 0,
        "mtmd_use_gpu": false,
        "word_boxes_backend": "rten_cpu",
        "raw_load_log_relpath": "source-gate/<phase>/<device>/<process>/load.log",
        "raw_load_log_sha256": "64-lowercase-hex",
        "enumerated_devices": [
          {
            "index": 0,
            "name": "raw-name",
            "description": "raw-description",
            "backend": "CPU|Metal|other",
            "device_type": "cpu|accelerator|gpu|integrated_gpu|unknown"
          }
        ],
        "loaded_model_devices": [
          {
            "model_device_ordinal": 0,
            "name": "raw-loaded-model-device-name",
            "backend": "CPU|Metal|other",
            "device_type": "cpu|accelerator|gpu|integrated_gpu|unknown"
          }
        ],
        "offloaded_layers": 0,
        "offloadable_layers": 39,
        "model_buffer_bytes_by_backend": { "CPU": 1 },
        "mtmd_backend": "CPU"
      }
    }
  ],
  "calibration_results": [
    {
      "entry_id": "id",
      "process_evidence_id": "calibration-cpu-process-id",
      "candidate_id": "S25L4|S25L5|S25L6|S25L7",
      "execution_evidence": {
        "paddle_instance_id": "same-128-bit-hex",
        "context_offload_kqv": false,
        "context_op_offload": false,
        "inference_completed": true,
        "raw_inference_log_relpath": "source-gate/calibration/<entry>/<device>/<candidate>.log",
        "raw_inference_log_sha256": "64-lowercase-hex",
        "context_buffer_bytes_by_backend": { "CPU": 1 },
        "compute_buffer_bytes_by_backend": { "CPU": 1 }
      },
      "runtime_nodes": [
        {
          "node_id": "uuid",
          "recognition_anchor": [0, 0, 1, 1],
          "node_rotation": 0.0,
          "text_rotation": 0.0,
          "selected_as_han": true
        }
      ],
	      "derived": {
	        "actual_device": "cpu",
	        "matched_target_ids": ["ids"],
	        "selected_target_ids": ["ids"],
	        "selected_protected_node_ids": [],
	        "selected_rotation_target_ids": [],
	        "unmatched_selected_node_ids": [],
	        "target_recall": 1.0,
	        "protected_false_positive_count": 0,
	        "rotation_targets_excluded": true,
	        "source_coverage_preflight": {
	          "pp_han_scalar_count": 8,
	          "vl_expected_han_scalar_count": 8,
	          "pp_vl_complete_coverage": true,
	          "rejected_after_vl": false,
	          "pp_vl_incomplete_coverage": false,
	          "covered_source_roi_ids": ["target-id"],
	          "source_text_roi_coverage": 1.0,
	          "source_removal_preflight_passed": true
	        },
	        "passed": true
	      }
    }
  ],
  "selected_candidate_id": "one-candidate-id",
  "frozen_at_utc": "RFC3339 timestamp",
  "frozen_payload_sha256": "canonical-frozen-fields-64-lowercase-hex",
  "holdout_results": [
    {
      "entry_id": "id",
      "process_evidence_id": "holdout-metal-process-id",
      "candidate_id": "selected-candidate-id",
      "execution_evidence": {
        "paddle_instance_id": "same-128-bit-hex",
        "context_offload_kqv": true,
        "context_op_offload": true,
        "inference_completed": true,
        "raw_inference_log_relpath": "source-gate/holdout/<entry>/<device>.log",
        "raw_inference_log_sha256": "64-lowercase-hex",
        "context_buffer_bytes_by_backend": { "CPU": 1, "Metal": 1 },
        "compute_buffer_bytes_by_backend": { "CPU": 1, "Metal": 1 }
      },
      "runtime_nodes": [
        {
          "node_id": "uuid",
          "recognition_anchor": [0, 0, 1, 1],
          "node_rotation": 0.0,
          "text_rotation": 0.0,
          "selected_as_han": true
        }
      ],
	      "derived": {
	        "actual_device": "metal",
	        "matched_target_ids": ["target-id"],
	        "selected_target_ids": ["target-id"],
	        "selected_protected_node_ids": [],
	        "selected_rotation_target_ids": [],
	        "unmatched_selected_node_ids": [],
	        "target_recall": 1.0,
	        "protected_false_positive_count": 0,
	        "rotation_targets_excluded": true,
	        "source_coverage_preflight": {
	          "pp_han_scalar_count": 8,
	          "vl_expected_han_scalar_count": 8,
	          "pp_vl_complete_coverage": true,
	          "rejected_after_vl": false,
	          "pp_vl_incomplete_coverage": false,
	          "covered_source_roi_ids": ["target-id"],
	          "source_text_roi_coverage": 1.0,
	          "source_removal_preflight_passed": true
	        },
	        "passed": true
	      }
    }
  ],
  "holdout_completed_at_utc": "RFC3339 timestamp",
  "retuned_after_freeze": false
}
```

The parser requires integer `plan_revision: 50`; focused negative fixtures substitute integers `29` through `49`, string `"50"`, and `null`, and must fail before model load or output.

The validator requires all `4 calibration x 2 requested devices x 4 candidates` Source Gate result cells and all `4 holdout x 2 requested devices` selected-candidate cells. Every cell references exactly one process-evidence record with the same phase/requested device, and its `paddle_instance_id` must equal the ID stored in that `PaddleOcrVl` instance at successful load and re-emitted by its inference method. Each `loaded_model_devices` element has exactly `model_device_ordinal`, `name`, `backend`, and `device_type`; ordinals are unique contiguous integers starting at zero, and unknown/empty names or enum values fail. Each fresh selection process hashes its current test executable, all five Source Gate model/config artifacts, every actually loaded llama/ggml/mtmd dynamic library, every raw load/inference log, and the same-descriptor fixed repository fixture manifest; calibration and holdout must use one identical selection-executable hash, five-artifact hash set, sorted loaded-library path/hash map, fixture-manifest hash, image-input-contract hash, and parser version. It derives CPU only when `cpu_forced=true`, `n_gpu_layers=0`, `mtmd_use_gpu=false`, both context offload flags are false, `loaded_model_devices` is nonempty and every element's backend/device type is CPU/cpu, `offloaded_layers=0`, `model_buffer_bytes_by_backend.CPU>0`, `context_buffer_bytes_by_backend.CPU>0`, `compute_buffer_bytes_by_backend.CPU>0`, all three maps contain no non-CPU bytes, `mtmd_backend` is CPU, and the same instance completed inference. It derives Metal only when `cpu_forced=false`, `n_gpu_layers=DEFAULT_GPU_LAYERS`, `mtmd_use_gpu=true`, both context offload flags are true, the loaded model has at least one Metal device and no other non-CPU backend, `offloaded_layers>0`, Metal model-buffer bytes are nonzero, `mtmd_backend` is Metal, both Metal context-buffer and Metal compute-buffer bytes are nonzero, and the same instance completed inference. Available-device enumeration is diagnostic only. Any missing required key, unknown key, empty loaded-device array, missing/empty buffer map, zero required backend bytes, duplicate, mixed, unknown, unparsed, or contradictory backend record is a failure, not a fallback. Unit tests independently reject each vacuous CPU variant: empty loaded devices; only unknown/non-CPU devices; absent, empty, or CPU-zero model map; absent, empty, or CPU-zero context map; absent, empty, or CPU-zero compute map; and an inference record whose instance ID differs from load. It recomputes the unique ROI matching from `runtime_nodes`, then recomputes selected target/protected/rotation/unmatched sets, recall, false positives, source coverage/preflight, and pass/fail; every stored `derived` field must equal recomputation. Each `source_coverage_preflight` object is required in every calibration and holdout cell; its PP and VL Han scalar counts must match recomputation, `pp_vl_complete_coverage` must be true, `rejected_after_vl` and `pp_vl_incomplete_coverage` must both be false, `covered_source_roi_ids` must include every matched source ROI that needs removal, `source_text_roi_coverage` must be exactly `1.0`, and `source_removal_preflight_passed` must be true. Mandatory `c04` must satisfy those fields on both CPU and Metal for the selected candidate, and no holdout cell may contain `rejected_after_vl` or `pp_vl_incomplete_coverage`. It then recomputes the smallest all-pass calibration candidate and rejects target recall other than 1.0, incomplete source coverage/preflight, any protected false positive, selected rotation/unmatched node, missing cell, changed candidate, selection-build/model/library/fixture/image-input-contract/device drift, or `retuned_after_freeze=true`.

Before holdout, the complete artifact has exact lowercase-hex `b0_sha`, `source_color_contract_sha256`, and `color_constant_set_sha256`; `required_checks` contains exactly the passing `pre-calibration` entry bound to the same `b0_sha`; `process_evidence` contains calibration records only; `calibration_results` is complete; `holdout_results: []`; `holdout_completed_at_utc: null`; `retuned_after_freeze: false`; and `frozen_payload_sha256` is set to the SHA-256 defined below. `enabled_cargo_features` must equal the sorted exact list `["hanonly-test-evidence","metal"]`; missing, extra, reordered, or default-enabled evidence features fail. The frozen projection contains exactly these keys: `version`, `plan_revision`, `b0_sha`, `manifest_sha256`, `source_gate_fixture_manifest_sha256`, `image_input_contract_sha256`, `source_color_contract_sha256`, `color_constant_set_sha256`, `requested_devices`, `enabled_cargo_features`, `backend_evidence_parser_version`, `required_checks` filtered to the single `pre-calibration` object, `frozen_recall_contract`, `candidates`, `calibration_entry_ids`, `holdout_entry_ids`, calibration-phase `process_evidence` sorted by `id`, `calibration_results` sorted by `(entry_id, process_evidence_id, candidate_id)`, `selected_candidate_id`, `frozen_at_utc`, and `retuned_after_freeze`. It excludes `frozen_payload_sha256` itself, every `required_checks` object whose phase is not `pre-calibration`, all holdout-phase process evidence, `holdout_results`, and `holdout_completed_at_utc`. Calibration-freeze recomputes both closed-preimage hashes before model load; holdout and final `IMPL_SHA` independently recompute and compare them. Missing/null/non-string, uppercase/nonhex, 63/65-character, one-nibble, swapped, old Revision 46/47/48/49 artifact values, alternate preimage bytes, missing required-check evidence, missing frozen recall-parameter evidence, checker endpoint drift, required-check phase-order drift, or cross-phase/final mismatch fails before model load, holdout output access, B1 authorization, or artifact mutation. Canonical JSON recursively sorts object keys by Unicode code point, preserves the specified array order, emits standard JSON escaping, finite JSON numbers, and no insignificant whitespace; SHA-256 is computed over its UTF-8 bytes. Write and fsync this pre-holdout artifact before opening holdout outputs. For holdout, rerun the anti-fixture check before model load; after holdout completes, append only the passing `pre-holdout` required-check entry, holdout-phase process evidence, sorted holdout results, and a nonnull completion timestamp, fsync again, rebuild the same filtered projection from only the `pre-calibration` required-check object, and require the exact frozen hash to remain unchanged. The validator then descriptor-reopens the artifact, validates exact `B0_SHA`, completed holdout, false retuning, unique selected Revision 50 candidate, both required-check executions, and frozen recall contract, computes the whole-file SHA-256 that binds both required-check entries plus holdout evidence, and records that digest only in durable execution state. The artifact is never rewritten after this point. Final `IMPL_SHA` acceptance first proves all eight frozen endpoint blobs identical, then revalidates those exact artifact bytes plus the frozen recall contract and accepted source-coordinate boxes/anchors without invoking calibration or selection. The four holdout hashes must pass without changing candidates, selection executable, feature set, model/config artifacts, loaded libraries, fixture manifest, code, masks, clean references, ROIs, expected outcomes, calibration records, required-check evidence, or frozen fields. Any B0-sensitive/frozen-interpreter change or holdout failure invalidates the artifact, ends Revision 50, and returns to D2/Planner; same-revision retuning or reselection is forbidden.

Revision 50 supersedes the older single-manifest projection wording: the frozen projection contains `calibration_manifest_sha256`, exact `evidence_contract`, and `calibration_diagnostic_index`, but excludes `holdout_manifest_sha256`, `holdout_entry_ids`, and `terminal_diagnostic_index`. Before seal those excluded fields are respectively null, empty, and null; after seal/holdout they are append-only and the frozen projection remains unchanged. Both diagnostic bindings point to immutable generation files, never the mutable current alias. `terminal_diagnostic_index` must name a later generation whose ancestry contains the frozen calibration generation. Each new cell requires one `absent -> captured_unclassified` generation that increments `expected_cell_count`, followed by one terminal generation; holdout additions are rejected before calibration and candidate freeze. The validator descriptor-recomputes both manifest ledgers, contract bytes/length/SHA, and the complete immutable diagnostic generation chain. Contract drift, calibration-manifest drift, changed calibration generation, terminal generation not descended from it, or any holdout path/length/hash mismatch fails closed.

Repository source-gate crop fixtures may supplement Source Gate coverage but do not satisfy full-page inpaint/render acceptance. The five current crops share one `source_raw_blake3` and therefore count as one source, not five.

Run `bun cargo test -p koharu-app --features metal,hanonly-test-evidence han_only_visual_manifest_matrix --lib -- --ignored --nocapture --test-threads=1` with the manifest/evidence environment. For every image, save the T0 report, raw Source Gate evidence, successful-mode residual and sprite metrics, source-color mode/reason/contract/width/probe metrics, expected/frozen/pre-inpaint-raster-entry translation digests and counts, insertion-only break transcript digest, cluster/control coverage digest and counts, shaped/representable/fill-visited glyph counts and completion verdicts, nonempty-alpha, `B_prepare` equality/upstream-retention facts for injected failures from either builder class, retained logical-sprite dimensions/bytes/reservation, actual raster scale/dimensions/surface bytes, peak-live-scratch and per-target scratch-release verdicts, frozen sprite identity, Renderer-zero-raster and payload-release verdicts, actual persisted Source/Inpainted-base/Brush/ordered-sprite identities/transforms and page-node order, independently recomputed protected-support hash, exact final-composite/omission metrics, rotation and unsupported-color equality/zero-erase metrics, and a side-by-side Source/Clean/Inpainted/Rendered contact sheet under the persistent guarded evidence root. Every `automatic_strict` and `manual_override` target must prove before inpaint complete expected translation reconstruction, complete cluster/control byte coverage, zero unrepresentable or missing glyphs, one successful fill visit per shaped glyph, checked complete-page retained logical-sprite reservation, checked actual-scale raster arithmetic/construction, at-most-one live scratch surface with per-target release, one nonempty frozen sprite, and then prove the Renderer received the same sprite with zero raster calls, persisted/composited it once, released the payload, produced final Rendered output, matched exact decoded-RGBA forward replay through protected restore/brush/ordered sprites, and satisfied per-target omission inequality; `automatic_strict` also proves exact source-contract RGBA and `F_s/W_s/F_t/W_t`. Rotated and unsupported-color targets require Source/Clean/Inpainted/Rendered equality throughout their edit ROI, zero final erase pixels, no rendered block/sprite, one exact-prefix warning, `warning_count > 0`, and RPC `CompletedWithErrors`. Human visual acceptance remains required only for successful-mode inpaint texture plausibility; the text, residual, and composite metrics use existing deterministic data and traversal/blending, with no learned scorer or new dependency. Evidence cleanup is explicit and occurs only after acceptance/checkpointing.

T5 reports also persist `classifier_page_node_count=J`, nullable `classifier_automatic_target_count=T`, `classifier_p0_phase`, and the complete classifier budget diagnostics, and assert `preflight_consumed<=preflight_reserved`, `evaluation_consumed<=evaluation_reserved`, `target_consumed<=target_reserved<=target_limit`, and `page_consumed<=page_reserved<=page_limit`. They independently recompute the six exact phase-specific P0 terminal tuples: `node_enumeration` limit; `target_terminalization` overflow and limit; `canonical_ranking` overflow and page limit; and admitted `Wmeta=checked(J+T+choose2_checked(T))`. They also recompute every page evaluation/preflight/total checked sum from target records. The node-enumeration failure proves only O(1) `page.nodes.len()` was read, with requested/reserved/consumed `J/0/0`, no node or target traversal, and no target record. Target-terminalization failures prove the admitted `J` traversal derived `T` but emitted no target records. Canonical-ranking failures prove `J+T` was consumed and exactly `T` terminal records exist before pairwise ranking was denied. `T=0/1/2`, the largest successful pair count, pair multiplication overflow, and final `Wterm+Wpair` addition overflow are independently checked. For P1 they independently recompute the conservative requested/reserved bound and a separate real-operation total; `|B|<8X_i`, `H=1`, `H=64`, `H=65`, and admitted semantic early stops prove unused capacity remains unconsumed. Any padding/no-op/synthetic increment or mismatch between the independent operation count and reported consumed hard-fails. Reports require the same final page totals in every emitted target record and across reversed input order. Every successful target's guarded local report also carries the digest/count/verdict transcript needed to prove expected/frozen/pre-inpaint-raster-entry text identity, reversible insertion-only breaks, complete cluster/control byte coverage, representable glyphs, complete fill traversal, nonempty alpha, retained logical-sprite reservation, actual-scale raster dimensions/bytes, peak-live-scratch `<=1`, per-target scratch release, same frozen sprite at Renderer entry, zero Renderer raster, and payload release; raw source or translated text is forbidden. Injected failures from either builder class separately record exact `B_prepare` equality and retained upstream commits; allocator OOM/abort is not a report verdict. Every normal pipeline cell first proves `spec.options.region.is_none()` and then records selected producer counts, current `PageId`, per-page builder/reservation/raster/publication counts, and boolean process-local same-object/sprite verdicts. A region-bearing HTTP request or direct run must be represented only by its pre-side-effect rejection verdict and may emit no normal cell, lifecycle record, warning, completion event, or mutation evidence. Accepted normal cells prove the total matrix: a no-inpainter page has zero builder/reservation/raster/publication and no frozen object; a one-inpainter page has one complete immediately preceding publication under its own key; Renderer without one inpainter is rejected; selected inpaint and Renderer consume the same current-page object/sprite; a failed page cannot expose its record to the next page and persists zero sprite/blob bytes; raster scratch releases per target, pixel payloads release on every terminal path, and Renderer performs zero layout/raster rebuild. Repair cells are reported separately and accepted only when the URL role is `MaskRole::Segment`, `Registry::find(params.pipeline)` succeeds, and the resolved descriptor produces exactly `[Artifact::Inpainted]`; unknown, non-Segment, Renderer, Typography, other, or multi-artifact pipelines return the stable HTTP 400 before body decode/blob/Scene/engine/apply and emit zero builder/publication/accessor evidence. Accepted repair cells are never interpreted through the normal matrix. Arc addresses, raw pointers, task-local keys, and object identity tokens are never serialized or compared across processes; `Arc::ptr_eq` plus current-page object/sprite identity are asserted inside the producing process by the focused lifetime test. `classifier_elapsed_us` is retained only for operations observability and excluded from every acceptance comparison and hash.

## T6. CPU/Metal runtime matrix

T6 is post-GREEN final acceptance only; RED-A uses T0 baseline diagnostics and never invokes this strict matrix. GREEN-B/B0 implements and passes the required Source Gate post-load/post-inference observability before B1 starts; GREEN-C adds the exact AOT-device accessor/label, private task-local scope, repair guard, P-1/P0 accounting, `B_prepare` baseline, independently checked retained logical-sprite and actual-scale raster facts, serial scratch lifetime/release, and complete pre-inpaint text/layout/glyph/fill/alpha/frozen-sprite evidence needed by the paired full-pipeline gate. All B0/T5/T6 evidence commands explicitly enable the same feature set recorded canonically as `["hanonly-test-evidence","metal"]`, while default workspace/release checks omit `hanonly-test-evidence`. Before the first final cell, the existing checker writes the canonical production closure consumed by every cell and the runtime validator. All nine manifest entries run through the full pipeline on both CPU and actual Metal, with ten repeats in each of two fresh processes. Every `automatic_strict` and `manual_override` target runs the residual oracle against its independently approved clean reference on the pre-render Inpainted layer and must prove before inpaint complete expected/frozen/pre-inpaint-raster-entry translation identity, reversible inserted newlines, complete cluster/control coverage, glyph-zero `.notdef` rejection, nonzero `u16` representability, zero missing glyphs, complete fill traversal, checked complete-page retained logical-sprite reservation, checked actual-scale raster dimensions/bytes/construction, at-most-one live scratch surface with per-target release, and exactly one nonempty frozen sprite; after inpaint it proves same frozen sprite identity at Renderer entry, zero Renderer layout/raster, one persistence/composition, payload release, final Rendered output, exact decoded-RGBA forward replay through independent protected restore/Brush/ordered sprites, and per-target omission inequality. Every injected failure from either builder class proves exact final equality to `B_prepare` and retention of upstream commits; process allocator OOM/abort is not encoded as a verdict. Every `automatic_strict` target also runs exact source-color/width contract and probe checks; every unsupported mode runs equality/zero-erase/no-sprite/warning/status checks. Detailed text evidence remains only in guarded reports; T6 writes `R` only after a closed sanitized projection and forbidden-field/value scan pass. Paired acceptance is pinned to the existing AOT inpaint engine. `koharu-ml` defines default-off `hanonly-test-evidence=[]`; only that exact feature may expose `#[cfg(feature = "hanonly-test-evidence")] #[doc(hidden)] pub fn device(&self) -> &candle_core::Device` on `AotInpainting`. No accessor, trait, enum, wrapper, or device-label method exists in a default build. Every app import, call, classifier helper, task-local run-state write, and test consuming it uses the same exact feature gate; the app feature propagates exactly to `koharu-ml/hanonly-test-evidence` and `koharu-llm/hanonly-test-evidence`. The feature-enabled AOT app engine records the loaded instance's private observational class (`cpu`, `metal`, or `cuda`) within the directly awaited task-local scope, without influencing engine selection, inference, retries, or output; default workspace/release builds contain no reference to `AotInpainting::device`. Source Gate actual device and metrics are independently derived from the B0/T5 same-instance raw load/model/context/node evidence, not copied from the AOT label or an engine-reported pass. Flux remains covered by deterministic dispatch/safety tests and an optional smoke, but is not a valid paired CPU baseline because its loader currently ignores the pipeline CPU flag.

Use these runtime-only environment variables:

- `HANONLY_VISUAL_INPUT`: approved absolute input path;
- `HANONLY_VISUAL_MANIFEST`: approved absolute Revision 50 manifest path;
- `HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256`: D0-frozen SHA-256 of `crates/koharu-app/tests/fixtures/source-gate-deterministic-recall/fixture-manifest.json`;
- `HANONLY_VISUAL_DEVICE`: `cpu` or `metal`; the inpaint engine is fixed to `aot`;
- `HANONLY_VISUAL_EVIDENCE_ROOT`: the absolute run-specific directory created by D0 and retained through final acceptance;
- `HANONLY_VISUAL_REPORT`: absolute report path;
- `HANONLY_VISUAL_ARTIFACT_DIR`: guarded persistent artifact directory below the evidence root;
- `HANONLY_VISUAL_RUNS`: positive repeat count.
- `HANONLY_VISUAL_PROCESS_INDEX`: `1` or `2`, identifying a fresh-process report.
- `HANONLY_PRODUCTION_CLOSURE`: absolute canonical closure path written once before the first final cell and passed to every runtime/aggregation verifier.

T6 has exactly one normative command surface: the continuous block in `Required command matrix` beginning with `# T6 canonical runtime acceptance` and ending after `--validate-runtime-matrix`. This section defines no duplicate or alternative shell command.

Every normal-pipeline T6 guarded cell proves `spec.options.region.is_none()` and records the current `PageId`, selected FontPredictions/TypographyStyles/Inpainted/Renderer counts, resolved conditional order, per-page builder/reservation/raster/publication sequence, process-local same-object/sprite verdicts, optional `f_t_raw/F_t/W_t`, `J/T/classifier_p0_phase`, first erase/inpaint sequence, expected/frozen/pre-inpaint-raster-entry translation digests and byte/scalar counts, insertion-only break transcript digest, cluster/control coverage digest and counts, shaped/representable/missing/fill-visited glyph counts, nonempty-alpha, `B_prepare` equality/upstream-retention verdicts for injected failures from either builder class, logical sprite dimensions/bytes/total/page-cap, actual raster scale/dimensions/surface bytes, peak-live-scratch and per-target scratch-release verdicts, payload-release verdicts, its preassigned target-correlation ID, and Renderer translation/layout/shape/glyph/raster/fit/group/Planner-size/rounding/width invocation counts. It never serializes raw source/translated text, an Arc address, raw pointer, task-local key, or cross-process identity token. The validator rejects any region-bearing normal cell and separately requires HTTP and direct-run region attempts to terminate at their declared pre-side-effect guard without a cell, run-state, warning, completion event, or mutation record. For accepted cells it applies the matrix per page: zero selected inpainters requires zero builder/reservation/raster/publication and no frozen object; one selected inpainter requires one complete publication under that `PageId` immediately before it and independently recomputed `F_t/W_t`, retained logical-sprite budget, and actual-scale raster facts; Renderer without one inpainter is invalid; when Renderer is selected, inpaint and Renderer must consume the same current-page object/sprite as proved inside that process. Every successful target must reconstruct the exact expected translation after removing only audited inserted newlines, cover all input bytes through canonical cluster intervals or deterministic legal-control records, reject glyph ID `0` for every non-control cluster, separately reject every nonzero glyph outside `u16`, require `missing_glyph_count=0`, prove the successful fill pass visited every shaped glyph exactly once before inpaint, require nonempty alpha, and prove at most one target scratch surface was live and released before the next target. Any deterministic hard pre-write `geometry_or_font` failure (translation-identity mismatch, final-layout failure after local fallback, cluster/control-coverage failure, glyph-validity failure, fill-traversal failure, or nonempty-alpha validation failure) or recoverable `geometry_or_font` failure (checked logical/actual-raster arithmetic or `usize` conversion failure, retained owner-rectangle/page-cap reservation failure, or fallible raster-surface-construction failure) must prove zero inpainter run/apply, exact final Scene/History/reachable-blob/sprite equality to `B_prepare`, retained upstream commits, and the complete zero-downstream-effect tuple; allocator OOM/abort is not a recoverable cell result. The accessor must be active only during the directly awaited related engine future and current page; wrong/prior/post-run access, spawned access, cross-page/run state, surviving pixel payload, scratch accumulation, or an escaping mutable handle fails the cell. Repair evidence is a separate direct-route verdict requiring nonempty region, default HanOnly, URL role `MaskRole::Segment`, successful registry resolution, descriptor `produces == &[Artifact::Inpainted]`, zero pre-guard body/blob/Scene/engine/apply effects, zero builder/publication/accessor, and unchanged successful atomic behavior; rejected unknown/non-Segment/Renderer/Typography/other/multi-artifact IDs return the stable HTTP 400 and never satisfy or violate a normal-pipeline matrix row. Conditional ordering omits absent terms, and every Renderer layout/raster/recomputation count is zero.

In addition, every Renderer-selected cell records the `B_prepare` page-transaction baseline, staged Inpainted/Rendered/sprite inventory, normalized frozen `sprite_transform`, `rendered_direction`, final cancel/epoch verdict, and one-commit outcome. Fault cells at every inpainter/Renderer/blob/Batch/History boundary must run the engines against staging yet end exactly at `B_prepare`; success commits the complete staged set once. Every ink-bearing shaped cluster/run records nonzero final logical-sprite alpha contribution. Peak-live fields enumerate all simultaneously live retained/Pixmap/copy/downsample/mask/staged/transaction buffers rather than only the largest scratch surface. A default/evidence public-output equivalence report is also required before T6 cells are accepted.

The harness must reject a relative/root/missing evidence root; an environment and ledger root that disagree with the D0 canonical shared external base/run root; an evidence base inside any linked worktree; a root that is not its direct mode-0700 owned run-id child; any symlink component in the repository/base/root/input/manifest/fixture-manifest chain; a missing, wrong-mode, wrong-owner, or mismatched D0 evidence ledger; report/artifact paths that are relative or escape the canonical evidence root through `..`/symlink components; an input/manifest that is not an absolute external regular file or whose same-`O_NOFOLLOW`-descriptor SHA-256 differs from D0; a dirty or same-descriptor-mismatched repository fixture manifest; a manifest Source/clean-reference/mask schema/role/hash/coverage/disjointness/oracle failure; a missing/nondiscriminative clean reference, residual-threshold failure, source-color/width contract/probe failure, unsupported-mode preservation failure, expected/frozen/pre-inpaint-raster-entry translation digest or count mismatch, non-reversible break transcript, cluster/control coverage gap/conflict, glyph-zero `.notdef`, nonzero out-of-range glyph, nonzero missing-glyph count, incomplete fill traversal, empty alpha, sprite area/byte/addition/`usize`/owner/page-cap failure, wrong frozen sprite identity, nonzero Renderer raster, payload leak/cross-page accumulation, successful-mode missing/empty/duplicate sprite, unreadable persisted composite input, protected-support recomputation/order failure, exact-composite mismatch, or omission equality; missing or invalid `crop-policy-selection.json` after calibration; calibration/holdout selection-executable drift, cross-stage model/config, loaded-library, fixture-manifest, image-input-contract, source-color-contract, color-constant-set, or production-closure drift, final-executable drift within the 360-cell matrix, actual-device drift, or a stored metric that disagrees with recomputation; missing baseline files; nonpositive runs; invalid process index; any inpaint engine other than pinned AOT; requested CPU when loaded AOT is not CPU; requested Metal when loaded AOT is not Metal; and conflicting page device labels. The shell passes nonexistent report/artifact paths and performs no per-cell `mkdir`. The Rust harness completes all ledger, visual-manifest, fixture-manifest, production-closure, Source, Clean, mask, and persisted composite-input single-read/hash/decode/parse and cross-validation before it descriptor-relatively creates the runtime/process/report/artifact children; a replacement-race failure leaves no output path for that cell and occurs before model load, inference, blob write, or Scene op. The approved input, manifest, masks, and clean references may remain outside the evidence root. Each guarded report must contain one row for every `(entry_id, repeat)` under its requested/actual device and process index, plus production-closure schema/common/generator-lock/trusted-target hashes, raw Source Gate load/node/build evidence, recomputed crop metrics, mandatory positive `source_line_count`/`ocr_line_count`/`translated_line_count` for every matched target, `J/T/classifier_p0_phase`, residual metrics, color mode/reason, source sample/uniformity/separation/AA facts, exact resolved fill/stroke RGBA and `F_s/W_s/F_t/W_t`, expected/frozen/pre-inpaint-raster-entry translation digests and counts, break/cluster/control transcript digests and counts, shaped/representable/missing/fill-visited glyph counts, nonempty-alpha and completion verdicts, checked sprite dimensions/bytes/total/page-cap, frozen sprite identity, Renderer-zero-raster and payload-release verdicts, target-correlation ID, probe verdict, successful-render sprite count/alpha bounds, actual persisted Source/Inpainted-base/Brush/ordered-sprite hashes/transforms and persisted page-node order, independently recomputed protected-support hash, exact-composite/omission verdicts, unsupported equality/zero-erase/warning/status verdict, the exact AOT device emitted by the inference instance, and existing layout backend identities. CPU/Metal acceptance compares structural invariants and safety outcomes, not necessarily inpainted pixel hashes. A missing model or unavailable actual Metal device is an explicit acceptance gap, never a pass.

For Revision 46, every generic `sprite` field in the preceding harness paragraph is interpreted as retained logical frozen RGBA only. The guarded report must separately carry actual `raster_scale_i`, checked raster width/height/surface bytes, peak-live-scratch count, and per-target scratch-release verdict; it rejects conflation of supersampled scratch with retained bytes, more than one live target scratch surface, or missing scratch release. Injected recoverable failures cover checked logical/raster arithmetic, retained reservation, and fallible surface construction, prove final equality to `B_prepare` plus upstream-commit retention, and never encode process allocator OOM/abort as a recoverable verdict.

Run each outer `process_index` iteration from a fresh application/test process to cover restart stability. Before the first final cell, the checker writes one canonical `production-closure-v1` artifact from the clean implementation checkpoint. The `--validate-runtime-matrix` mode reads only that closure artifact plus the four canonical guarded report paths, recomputes every stored metric, and requires exactly the Cartesian product of `9 entry_id x 2 requested/derived devices x 2 process_index x 10 repeat`, totaling 360 unique cells. It rejects a duplicate, missing, extra, out-of-range, or unknown tuple before writing `runtime-matrix-validation.json`. Global comparison is limited to one final-executable SHA-256, one production-closure schema/common/generator-lock/trusted-target tuple, one five-artifact model/config hash set, one sorted loaded-library path/hash map, frozen recall contract, visual-manifest hash, frozen Source Gate fixture-manifest hash, frozen image-input-contract hash, frozen source-color-contract hash, frozen color-constant-set hash, feature set, and parser version, equal to the accepted D2 artifact where applicable. Requested and derived device fields are validated against each cell's `device` axis rather than compared for equality between CPU and Metal. Entry-level structural values are grouped by guarded `entry_id` and must be identical only across that entry's `2 devices x 2 processes x 10 repeats`: target/protected/rotation/color-mode sets and counts, `J/T/P0` phase/accounting, B0 Source Gate crop choice, page ownership/locality decisions, checked sprite total/page cap and payload-release outcomes, warning/status outcome, and safety verdict. Target-level values are grouped by guarded `(entry_id,target_id)` and must be identical only across that target's 40 cells: resolved geometry, source-size/`G/Csrc/S0/Smax/G0/G1`, final size, Planner field outcomes, color mode/reason, source sample/reference/threshold facts, resolved fill/stroke RGBA, `F_s/W_s/F_t/W_t`, expected/frozen/pre-inpaint-raster-entry translation digests/counts, break/cluster/control transcript digests/counts, shaped/representable/missing/fill-visited glyph counts, nonempty-alpha/Renderer-zero-raster/frozen-sprite-identity/completion verdicts, checked sprite dimensions/bytes, probe verdict, successful-render sprite/exact-composite/omission outcome or unsupported preservation outcome, and residual/equality verdict; numeric CPU/Metal pixel residual values may differ while each must pass its frozen threshold. Differences between distinct entries or targets are expected and are never compared as equality. Before output, the validator projects each guarded target through its preassigned correlation ID into the closed sanitized `R` schema, carrying only the six non-length booleans and unrelated aggregate/closure fields, then runs the forbidden field/value scan. The final-executable and trusted-target closure tuple are captured after GREEN-C before the first final cell and remain identical across all 360 cells. Validation or sanitization failure emits no aggregate pass and blocks the production audit.

Runtime aggregation treats retained logical sprite totals/page caps and actual-scale raster-surface facts as distinct invariant families. Across each target's 40 cells it compares the actual scale, checked raster dimensions/bytes, peak-live-scratch `<=1`, and scratch-release verdict independently from logical frozen-sprite identity/bytes; `B_prepare` equality and upstream-retention verdicts are mandatory for every injected failure from either builder class and allocator OOM/abort has no schema value.

`runtime-matrix-validation.json` (`R`) is a closed sanitized Revision 50 artifact that inherits the Revision 46 visual contract. In addition to the recomputed aggregate matrix verdict and fingerprints above, it contains `plan_revision: 50`, `image_input_contract_sha256`, `source_color_contract_sha256`, `color_constant_set_sha256`, `production_closure_schema_sha256`, `production_common_sha256`, `generator_lock_inputs_sha256`, `trusted_target`, `trusted_generated_target_sha256`, `trusted_production_closure_sha256`, aggregate P-1/P0/stroke-width/preflight pass booleans, and `target_verdicts`. `target_verdicts` is sorted by unique 32-lowercase-hex `target_correlation_id`; each value has exactly six booleans: `translation_identity_pass`, `cluster_control_coverage_pass`, `glyph_coverage_pass`, `fill_traversal_pass`, `exact_composite_pass`, and `omission_pass`. `R` contains no raw source/translated text, stable NodeId, source/OCR/translated line count, text-derived digest, text/byte/scalar/cluster/glyph length or count, target mapping, guarded report path, or target geometry. The trusted aggregate is SHA-256 over canonical JSON containing the schema/common/generator-lock hashes plus the trusted target name and target-record hash. Missing, unknown, stale-revision, wrong-type, correlation-ID collision/format drift, forbidden field/value, or mismatched contract field fails before `R` is written.

T6 additionally validates every emitted automatic-target budget diagnostic, requires `preflight_consumed<=preflight_reserved`, `evaluation_consumed<=evaluation_reserved`, `target_consumed<=target_reserved<=target_limit`, and `page_consumed<=page_reserved<=page_limit`, and rejects missing/malformed fields, counter drift, input-order-dependent page admission/totals, or any elapsed-dependent verdict. It independently recomputes all six P-1/P0 phase tuples, proves `J` was admitted before the only node traversal, proves target-terminalization failures emit no target records, proves canonical-ranking failures emit exactly `T`, and proves admitted pair comparisons equal total `choose2_checked(T)` for zero, one, two, boundary, multiplication-overflow, and final-addition-overflow cases. It also independently recomputes every target `width*height*4`, checked `usize` conversion, owner-rectangle containment, complete-set sum, and page RGBA cap; proves reservation precedes allocation/raster; proves exactly one builder raster per successful target and zero Renderer raster; and proves that every injected deterministic hard pre-write `geometry_or_font` failure (translation-identity mismatch, final-layout failure after local fallback, cluster/control-coverage failure, glyph-validity failure, fill-traversal failure, or nonempty-alpha validation failure) and every injected recoverable `geometry_or_font` failure (checked logical/actual-raster arithmetic or `usize` conversion failure, retained owner-rectangle/page-cap reservation failure, or fallible raster-surface-construction failure) produces zero publication, inpainter run/apply, erase, or downstream persistence; final observable Scene/History/reachable-blob/sprite state equals `B_prepare` and retains upstream producer commits; process-level allocator OOM/abort is excluded. `source_color_contract_sha256` hashes `source-color-contract-v2` plus the complete `source-color-work-budget-v1` version/accounting text; `color_constant_set_sha256` includes the three color thresholds and all four work limits. Sprite reservation remains outside both preimages. The existing `han_only_visual_manifest_matrix` and `han_only_visual_runtime_matrix` filters enforce this complete pre-inpaint proof without adding staged IDs.

## T7. Required CI governance gate

All current `R/A/C/G`, corpus, annotation, and closure-summary schemas use Revision 50. They also enforce earlier inherited visual-contract fields wherever this specification keeps those fields normative. In addition to wrong-revision fixtures using integers `29..49`, string `"50"`, and `null`, closed-schema tests reject lowercase stale revision markers, mixed-case stale markers, stale `source-color-contract-v1`, missing `source-color-work-budget-v1`, altered work constants, altered accounting/preflight/preimage bytes or digest, missing complexity reason, and any contract-hash mismatch.

T7 runs only after T6, the final repository-wide audit, and human visual acceptance pass. It does not move model, image, Metal, or 360-cell work into Ubuntu PR CI. Its purpose is to make the bounded production-generalization policy a required normal-merge condition while omitting raw corpus values and all target text proof. Unsalted digests are not a confidentiality boundary for low-entropy counts, lengths, or text; `R/A/C/G`, the corpus, annotation, and closure summaries therefore contain no runtime/manifest text-derived digest or count and no secret or PII.

The checker exposes one shared `computeProductionClosureV1` used internally by `--write-production-closure`, the full audit that writes `A`, ordinary PR `--ci-static`, and T7. Every closure invocation requires a dedicated linked worktree whose tracked and untracked status is empty. The immediate local post-generation `--ci-static` is the sole exception: only `scripts/fixtures/hanonly-production-policy-ci-corpus.json` may be dirty because `C` is excluded from the closure. The original user worktree is never an input. Each invocation uses two new empty temporary `CARGO_TARGET_DIR` values, derives production resolution in each with exact argv `bun --silent run scripts/dev.ts cargo metadata --locked --format-version 1`, derives the host target from pinned `rustc -vV`, and runs exact argv `bun --silent run scripts/dev.ts cargo check -p koharu-llm --lib --locked --target <derived-host-target> --message-format=json-render-diagnostics`. Caller-supplied Cargo JSON, reused target directories, an unverified llama archive/cache, or any same-target generated-record difference fails. The separate feature-enabled evidence Cargo log remains required by generated-Rust/evidence auditing but never enters the closure.

`production-closure-v1` is canonical UTF-8 JSON with recursively sorted object keys and entries sorted by fixed tuples. Its `common` map hashes canonical repository-relative path plus whole normalized bytes for every tracked workspace `Cargo.toml`, `Cargo.lock`, root/UI `package.json`, `bun.lock`, root `.oxfmtrc.json`, `Dockerfile`, `cliff.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, every tracked regular file beneath `scripts/` except generated `scripts/fixtures/hanonly-production-policy-ci-corpus.json`, every tracked `.github/workflows/*.{yml,yaml}`, `.github/CODEOWNERS`, UI root build/config files `.oxlintrc.json`, `components.json`, `instrumentation-client.ts`, `next-env.d.ts`, `next.config.ts`, `openapi.json`, `orval.config.ts`, `postcss.config.mjs`, `tsconfig.json`, and `vitest.config.ts`, `crates/koharu/tauri.conf.json`, every tracked regular `.rs` file under each workspace production package's `src/`, each declared `build.rs`, every statically resolved tracked repository-local `mod`/`#[path]`/`include!` source outside those trees, and every tracked non-test regular production source under `ui/app`, `ui/components`, `ui/hooks`, and `ui/lib`, including generated API files. Text normalization is only CRLF-to-LF; all other bytes remain unchanged. Mixed Rust and checker files are hashed whole, so test/evidence-only edits inside them intentionally invalidate acceptance. Standalone Cargo test/example/bench targets, crate `tests/`, UI tests outside the named enforcement scripts, external evidence, untracked/ignored files, and files outside the named roots are excluded. No `cfg`/`cfg_attr` grammar, projection, item ownership, or semantic exclusion exists.

The closure's target-independent `generator_lock_inputs` contains exact Rust `1.97.0` and rustc commit `2d8144b7880597b6e6d3dfd63a9a9efae3f533d3`, Cargo/bindgen package identities from `Cargo.lock`, `LLAMA_CPP_TAG=b8935`, and `LLAMA_CPP_ARCHIVE_SHA256=a1732727571c2ad6f94bd9e650de2bef299ca170501d0deb429d00acc8854896`. Add `rust-toolchain.toml` pinned to `1.97.0`; add the archive digest beside the tag in `.cargo/config.toml`; and make `koharu-llm/build.rs` SHA-256-verify every downloaded or cached archive before extraction, reject a preexisting unverified source tree, and extract only beneath a digest-qualified source directory. Each `generated_targets[target]` record contains target/host triples, rustc/Cargo versions, bindgen version, libclang identity hash, and sorted logical generated filename/content hashes with no absolute `OUT_DIR`. Two fresh runs for the same target must produce byte-identical target records. Trusted macOS and Ubuntu PR target records are deliberately not compared across target names; only schema, `production_common_sha256`, and `generator_lock_inputs_sha256` must match across platforms.

The closure excludes absolute target/output paths, external images/masks/clean references/runtime reports, the external evidence base, `R/A/C`, GitHub envelopes, `G`, timestamps, inode/owner/worktree state, commit SHA, and every test-corpus path/hash/dimension/bbox/NodeId/count/color value. Fixtures require any named common-root byte, manifest/lock/config/profile/build script, module/include source, Rust pin, llama tag/archive digest, or same-target generated byte change to alter its owning hash. Standalone test/example/bench and external-evidence changes remain excluded; mixed-file test/evidence edits change `production_common_sha256` by design. Missing named roots, malformed/unresolved local source edges, target-record nondeterminism, archive mismatch, source tree without a verified archive, target-directory reuse, any `R/A/C/G`, commit, or corpus value in closure output fails closed.

The hash graph is strictly acyclic: `closure -> R -> A -> C -> PR notice annotation -> G`. Every tracked local governance/closure input is committed before clean `IMPL_SHA`. T6 at exactly that SHA first writes sanitized `runtime-matrix-validation.json` (`R`). The full audit then independently reruns `computeProductionClosureV1`, revalidates guarded reports and their correlation-ID mapping, recomputes the six per-target booleans, rejects disagreement with `R`, applies the same forbidden field/value scan, and writes sanitized `production-policy-audit.json` (`A`). `A`'s closed schema includes `plan_revision: 50`, `runtime_matrix_validation_sha256 = sha256(R)`, contract hashes, the same six trusted closure fields and exact `target_verdicts` carried by `R`, plus policy-category verdicts, but no corpus, commit SHA, PR target record, attestation hash, raw text, stable NodeId, runtime/manifest line count, text-derived digest/length/count, guarded path, or mapping. It may not copy closure fields or target verdicts from `R` without recomputation. Only after `R` and `A` pass may the exact trusted `--generate-ci-corpus` command run in the clean implementation worktree. It accepts the approved manifest/evidence root, exact `R` and `A` paths beneath that root, and only the fixed excluded repository output `scripts/fixtures/hanonly-production-policy-ci-corpus.json`; it revalidates `R+A`, independently recomputes the six identity categories from raw manifest/runtime/fixture evidence, recomputes the seventh `line_count` category only from checker-owned synthetic syntax-coupling sentinels, requires equality with sanitized `A`, and writes `C` through an exclusive same-directory temporary regular file, file fsync, atomic rename, and parent-directory fsync. If a prior output file exists, every failed write/fsync/rename attempt leaves its bytes unchanged; if no prior output exists, failure leaves the output absent. In both cases no temporary file remains. Focused fault fixtures cover both prior states and every boundary. Immediate local `--ci-static` permits only `C` dirty. Git-master then commits that exact generated file with no closure-input change; the remote PR job and final T7 run only on the resulting globally clean candidate head. Any tracked non-`C` repository change after T6 invalidates `R/A/C/G` and requires a new clean `IMPL_SHA` plus fresh T6/A/C.

`C` has exactly `version`, `plan_revision`, `rule_schema_sha256`, `visual_manifest_sha256`, `source_gate_fixture_manifest_sha256`, `image_input_contract_sha256`, `source_color_contract_sha256`, `color_constant_set_sha256`, the six trusted closure fields from `R/A`, `runtime_matrix_validation_sha256`, `full_audit_sha256`, and `categories`; `version` is `1`, `plan_revision` is `50`, and the two evidence hashes equal `sha256(R)` and `sha256(A)`. The contract and trusted closure fields must equal Revision 50 `R` and `A` while retaining inherited Revision 46 contract fields. `categories` has exactly `path`, `hash`, `crop_name`, `node_id`, `bbox`, `node_count`, and `line_count`. Each category has exactly `derived_count`, `value_digests`, and `value_digests_sha256`: `derived_count` is positive and equals the number of sorted unique lowercase 64-hex digests, each digest is SHA-256 of one category-normalized policy-test value, and `value_digests_sha256` hashes the canonical JSON digest array. The first six categories use the existing non-text identity-policy values; `line_count` uses only checker-owned synthetic syntax-coupling sentinels and never a runtime/manifest source/OCR/translated line count. `C` has no timestamp, commit SHA, raw runtime path, source hash, crop name, target correlation ID or mapping, stable ID/NodeId, bbox/ROI tuple, runtime node/line/text count, source text, translated text, text-derived digest/length/count, guarded report path, evidence-image RGBA value, target-verdict field, or Ubuntu target record. The exporter field-and-value scanner rejects every forbidden addition. Neither `R` nor `A` references `C` or the later attestation; fixtures that add any reverse reference fail. `C.plan_revision` negative fixtures use integers `29` through `49`, plus string `"50"` and `null`; all fail.

Ordinary PR `--ci-static` accepts only committed `C`. Because `R` and `A` are intentionally absent from PR CI, it validates only locally provable facts: the closed schema, hash syntax, internal policy-fixture count/digest consistency, frozen contract hashes, exact workflow/CODEOWNERS shape, a fresh globally clean `computeProductionClosureV1`, syntax-only crop/offset/dimension/color/width-fallback and line-count-coupling rules, release-feature inventory, candidate-to-corpus digest matches, and the forbidden text-proof field/value scan. It requires schema, common, and generator-lock hashes to match `C`; independently proves the Ubuntu target record twice in fresh target directories; and does not compare that record to the trusted macOS target record. A local run requires `HANONLY_CLOSURE_SUMMARY_OUT` outside the checkout and atomically writes canonical JSON with exactly `plan_revision:50`, `candidate_head_sha`, `production_closure_schema_sha256`, `production_common_sha256`, `generator_lock_inputs_sha256`, `ci_target`, and `ci_generated_target_sha256`; local `candidate_head_sha` is `git rev-parse HEAD`, and local output omits unavailable `check_sha`. Under `GITHUB_ACTIONS=true`, only `pull_request` and main `push` are accepted. The checker reads exact job env `HANONLY_CANDIDATE_HEAD_SHA` and `HANONLY_CHECK_SHA`, requires checked-out `HEAD == HANONLY_CANDIDATE_HEAD_SHA` and `GITHUB_SHA == HANONLY_CHECK_SHA`, requires PR candidate and check identities to differ, and requires main-push `HEAD == candidate == check == GITHUB_SHA`. It then builds annotation canonical JSON with exactly `plan_revision:50`, `candidate_head_sha`, `check_sha`, `production_closure_schema_sha256`, `production_common_sha256`, `generator_lock_inputs_sha256`, `ci_target`, and `ci_generated_target_sha256`. After all checks pass, it emits exactly one escaped workflow command `::notice file=scripts/fixtures/hanonly-production-policy-ci-corpus.json,line=1,title=HANONLY_PRODUCTION_CLOSURE_V1::<canonical-json>`, which GitHub Actions records as a notice annotation without any write token. It never claims to authenticate `C`'s `R`/`A` hashes; final T7 does that. Unknown/duplicate/forbidden fields, default/wrong checkout, swapped/collapsed PR identities, unsupported event, missing/duplicate annotation emission, bad candidate/check SHA, closure drift, target nondeterminism, missing root, unresolved include/module/generated source, release-feature leak, runtime-derived `line_count` corpus data, or any policy hit fails. Test fixtures prove exact escaping, one PR annotation where candidate and check differ, one push annotation where all identities collapse, one forbidden text-proof leak for each artifact class, plus one hit and one pass for every category and closure partition.

The existing `.github/workflows/test.yml` contains exactly one prerequisite job id `hanonly-windows-history-contract` with display name `HanOnly Windows History Contract` and exactly one required-context job id `hanonly-production-policy` with display name `HanOnly Production Policy`. The prerequisite uses `windows-2022`, read-only contents permission, exact candidate-head env, `actions/checkout@v7` with exact `with.ref`, the repository's pinned Rust toolchain, and only `cargo test -p koharu-app --lib history::tests::windows_durable_replace_generation_contract -- --exact --nocapture`; direct default-feature Cargo intentionally avoids the CUDA-requiring Windows development wrapper for this filesystem-only contract. It has no `if`, `continue-on-error`, strategy matrix, feature flag, model/backend input, token/secret, or write permission. The required job uses `ubuntu-latest`, read-only contents permissions, exact `needs: [hanonly-windows-history-contract]`, exact job-level `if: ${{ always() }}`, and exact first step `run: test '${{ needs.hanonly-windows-history-contract.result }}' = 'success'`. No later step runs before that assertion, and no step has `if`. It then uses exact job env `HANONLY_CANDIDATE_HEAD_SHA: ${{ github.event_name == 'pull_request' && github.event.pull_request.head.sha || github.sha }}` and `HANONLY_CHECK_SHA: ${{ github.sha }}`, `actions/checkout@v7` with exact `with.ref: ${{ env.HANONLY_CANDIDATE_HEAD_SHA }}`, `oven-sh/setup-bun@v2`, `bun install --frozen-lockfile`, `bun test scripts/check-hanonly-production-policy.test.ts`, the default-feature Rust metamorphic step, and `bun scripts/check-hanonly-production-policy.ts --ci-static --ci-corpus scripts/fixtures/hanonly-production-policy-ci-corpus.json`; the checker emits the native notice only after success. The metamorphic step derives exactly 128 random bits from runner `/dev/urandom`, exports and prints lowercase-hex `HANONLY_METAMORPHIC_SEED`, resolves existing ordinary `han_only_source_color_work_budget_adversarial_stress` to exactly one module-qualified full libtest name, executes only that name with `--exact`, and proves started=1, passed=1, failed=0. It generates dimensions, geometry, node order, text length, color, transforms, and PNG/JPEG/WebP-equivalent in-memory payloads after build, uses no external image/model/Metal input and no new dependency, and prints the seed on failure. The workflow retains unconditional `pull_request` and `push.branches: [main]`; across all workflows each display name is unique, but only `HanOnly Production Policy` is a required context. Fixtures remove/duplicate/rename/condition/matrix/skip the Windows job, alter runner/checkout/test name/default-feature command, remove or weaken `needs`, `always()`, or exact-success assertion, add another dependency/condition, or permit Windows non-success to skip/pass the required context; every mutation fails. Existing fixtures continue to reject altered checkout refs, collapsed/swapped SHA env, fixed/missing seed, weak test discovery/execution/assertions, forbidden escapes/write scopes, and missing/duplicate annotation.

The seeded workflow step body is exactly:

```bash
set -euo pipefail

test_id='han_only_source_color_work_budget_adversarial_stress'
seed="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
case "$seed" in
  ''|*[!0-9a-f]*) exit 1 ;;
esac
test "${#seed}" -eq 32
export HANONLY_METAMORPHIC_SEED="$seed"
printf 'HANONLY_METAMORPHIC_SEED=%s\n' "$seed"

list_file="$(mktemp)"
match_file="$(mktemp)"
run_file="$(mktemp)"
trap 'rm -f "$list_file" "$match_file" "$run_file"' EXIT

bun cargo test -p koharu-app --lib -- --list >"$list_file"
awk -v id="$test_id" '
  $0 ~ ("(^|::)" id ": test$") {
    sub(/: test$/, "")
    if (index($0, "::") == 0) exit 42
    print
  }
' "$list_file" >"$match_file"
test "$(wc -l <"$match_file" | tr -d ' ')" -eq 1
full_name="$(sed -n '1p' "$match_file")"
test "${full_name##*::}" = "$test_id"

bun cargo test -p koharu-app --lib "$full_name" -- \
  --exact --nocapture | tee "$run_file"
test "$(grep -Fxc 'running 1 test' "$run_file")" -eq 1
test "$(grep -Fxc "test $full_name ... ok" "$run_file")" -eq 1
grep -Eq \
  '^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in ' \
  "$run_file"
```

No fixed fallback seed is allowed in GitHub Actions; local reproduction may set the printed seed explicitly. The checker parses the step and requires the unchanged short ID, `/dev/urandom` 16-byte seed, `--list`, exactly-one module-qualified match, execution through `full_name`, `--exact`, and all three started/passed/failed output assertions. Missing, renamed, deleted, duplicate, root-unqualified, ignored, or zero-test behavior fails.

`.github/CODEOWNERS` retains its existing wildcard and adds these root-anchored exact paths: `/.github/CODEOWNERS`, `/.github/workflows/test.yml`, `/scripts/check-hanonly-production-policy.ts`, `/scripts/check-hanonly-production-policy.test.ts`, and `/scripts/fixtures/hanonly-production-policy-ci-corpus.json`. Every exact line has the current owner list `@mayocream @fffonion @Map1en @karrot0 @liksunrice` in that order. The checker rejects a missing path, broader glob substitution, wrong/reordered owner set, duplicate conflicting owner, or later override.

After the job succeeds for the implementation pull request's current test-merge commit while explicitly checking out its candidate head, an explicitly authorized repository administrator activates one Ruleset targeting `refs/heads/main`. It is `active`, has an empty bypass actor list, requires a pull request with `required_approving_review_count: 1`, code-owner review, stale-approval dismissal, and the branch to be up to date, requires exact status context `HanOnly Production Policy` from the GitHub Actions integration, and blocks force pushes and deletion.

Remote capture is frozen to GitHub REST API `2022-11-28` with `Accept: application/vnd.github+json`. Five mode-0600 closed envelopes record exact method/path/query, API version, HTTP status, and body but no token or authorization header: direct `GET /repos/{owner}/{repo}/rulesets/{ruleset_id}`; `GET /repos/{owner}/{repo}/commits/{check_sha}/check-runs?check_name=HanOnly%20Production%20Policy&filter=latest&per_page=100`; `GET /repos/{owner}/{repo}/check-runs/{validated_check_run_id}/annotations?per_page=100`; direct implementation-PR response; and a collaborators envelope containing exactly five unpaginated direct `/collaborators/{login}/permission` responses for the exact owners. The PR body must prove `mergeable == true`, `head.sha == candidate_head_sha`, and `merge_commit_sha == check_sha`. The direct Ruleset body must include `bypass_actors: []`; missing or privilege-redacted data fails. The check-run body must bind that PR, have `head_sha == check_sha`, and satisfy `total_count == check_runs.length == 1`; the annotation body must be one complete unpaginated array and contain exactly one matching closure notice carrying both SHAs. Unequal counts, more than one page, pagination/truncation ambiguity, check-run-id mismatch, stale test merge, or an unexpected request query fails.

The verifier then runs `--verify-required-check-ruleset` from a globally clean detached PR worktree. It first requires local `git rev-parse HEAD == candidate_head_sha == authenticated PR head.sha`, reruns `computeProductionClosureV1` twice with fresh target directories, and requires its schema/common/generator-lock fields to equal independently produced `R`, `A`, and committed `C`; its local PR target record must reproduce exactly. Separately it requires `check_sha == authenticated PR merge_commit_sha == queried check_run.head_sha`. It requires the validated check-run annotations envelope to contain exactly one notice with path `scripts/fixtures/hanonly-production-policy-ci-corpus.json`, start/end line `1`, title `HANONLY_PRODUCTION_CLOSURE_V1`, and canonical-JSON message. It parses without coercion, requires annotation `candidate_head_sha` to equal authenticated PR head and local `HEAD`, requires annotation `check_sha` to equal authenticated PR merge commit and check-run head, requires schema/common/generator-lock fields to match local recomputation and `C`, and requires `ci_target`/`ci_generated_target_sha256` to match the local PR target record. Trusted macOS and Ubuntu generated-target hashes may differ only because their target names differ. It then requires one matching active Ruleset; one unique successful check on `check_sha` from the configured integration and associated with the exact PR; the exact five CODEOWNERS lines; at least two listed owners with authenticated `write`, `maintain`, or `admin` permission; and at least one eligible owner whose login differs from the PR author. It rejects disabled/evaluate mode, wrong API version/request/ref/PR/SHA/check-run-id, missing/duplicate/malformed annotation, swapped/collapsed PR identities, stale test merge, closure drift, same-target nondeterminism, missing/redacted bypass actors, any bypass actor, incomplete pagination, zero approvals, missing rule, `any`/wrong integration, absent/duplicate/stale/skipped/neutral/failing check, self-only ownership, insufficient owner permission, and conflicting context.

On success the validator descriptor-hashes sanitized `R`, sanitized `A`, `C`, and all five remote JSON files and writes/fsyncs mode-0600 `$HANONLY_VISUAL_EVIDENCE_ROOT/t7-governance-attestation.json` (`G`). `G` has exactly `version`, `plan_revision`, `repository`, `target_ref`, `candidate_head_sha`, `check_sha`, `required_context`, `integration_id`, `eligible_owner_logins`, `non_author_owner_logins`, the three contract hashes, `production_closure_schema_sha256`, `production_common_sha256`, `generator_lock_inputs_sha256`, `trusted_target`, `trusted_generated_target_sha256`, `trusted_production_closure_sha256`, `ci_target`, `ci_generated_target_sha256`, `runtime_matrix_validation_sha256`, `full_audit_sha256`, `ci_corpus_sha256`, `ruleset_json_sha256`, `check_runs_json_sha256`, `check_annotations_json_sha256`, `pull_request_json_sha256`, `collaborators_json_sha256`, and `verified_at_utc`; `version` is `1`, `plan_revision` is `50`, both SHA fields are lowercase 40-hex and satisfy their independent chains, every artifact hash is over the exact validated sanitized bytes, trusted fields equal `R/A/C`, and CI fields equal both the authenticated annotation and local PR recomputation. `G` contains no target-correlation ID or mapping, raw source/translated text, stable NodeId, runtime/manifest line count, text-derived digest/length/count, guarded report path, or target proof field; the shared exporter scanner rejects any such addition. `G` is final evidence and never an input to `R`, `A`, or `C`. Negative fixtures cover every missing/unknown/forbidden key, hash mismatch, malformed/swapped/collapsed/stale SHA, contract/common/generator/target drift, reverse hash edge, insufficient owner set, author-only eligibility, and `plan_revision` equal to integer `29` through `49`, string `"50"`, or `null`. Tokens and headers are never persisted. Missing admin authority, API support, eligible independent reviewer, authenticated annotation, or authenticated evidence blocks completion. This prevents bypass through normal `main` merge paths; an ultimate settings administrator can still change repository governance.

Rollback is also tested: captured before/after Ruleset fixtures prove the required context is removed remotely and `main` is no longer waiting on it before the job is renamed/removed. A fixture that removes the job first fails.

### Linked-worktree execution contract

The execution lane never makes the original dirty worktree clean and never uses it for closure or acceptance. Before the first implementation edit, it records outside every worktree: `git status --porcelain=v1 -z --untracked-files=all`, tracked and staged binary diffs, the sorted path plus `git hash-object --no-filters` identity of every untracked file, current `HEAD`, and the exact binary `HEAD` patch and blob hash for `crates/koharu-app/src/typography.rs`. It creates a uniquely named implementation linked worktree from that `HEAD`, applies only the captured typography patch, proves the resulting blob hash equals the original dirty file, and commits that patch as a branch-local baseline before task-owned edits. It never copies `scripts/storage.ts`, `scripts/storage.test.ts`, `test-image/`, or any other original dirty path.

After GREEN-A and B0-only implementation, every B0-sensitive harness/parser/validator/feature/model/config/library fingerprint and candidate input, all sixteen normative test IDs, and all eight closed `B0_FROZEN_INTERPRETER_PATHS` are committed as globally clean `B0_SHA`; it contains no B1 target layout/font/render/UI/color behavior. A detached globally clean `B0_WORKTREE` at exactly `B0_SHA` creates or rehydrates the one canonical `HANONLY_SHARED_EVIDENCE_BASE` outside all worktrees, proves the two B0/G004-owned IDs are unignored and pass, proves the remaining five T2 plus nine T3 IDs compile and fail individually under their stage markers while the default workspace suite passes, runs the anti-fixture required check, calibration-freeze, the same required check again before holdout model load, and fresh holdout, then descriptor-validates and freezes the whole-file SHA-256 of the sole Revision 50 `crop-policy-selection.json`. That artifact binds `B0_SHA` and is never rewritten. B1 consumes only the frozen recall contract and accepted source-coordinate boxes/anchors, and none of the eight frozen path bytes may change.

After B1 removes only its remaining five markers and reaches GREEN, GREEN-C removes only its nine markers and reaches GREEN; all local governance/closure inputs are then committed, and `IMPL_SHA` names a globally clean implementation worktree. A detached globally clean acceptance worktree at exactly `IMPL_SHA` proves the eight frozen endpoint paths exist as mode-`100644` regular blobs with identical `B0_SHA`/`IMPL_SHA` OIDs and zero endpoint-tree diff, then runs the final RED-state validator and full default workspace suite before rehydrating the evidence root or opening the B0 artifact. It next verifies the frozen B0 artifact bytes/digest, frozen recall contract, and accepted source-coordinate boxes/anchors without invoking the selection filter, then runs T5, T6, the full audit, and human review. `C` is then generated in the still-clean implementation worktree at the same `IMPL_SHA`, immediate local `--ci-static` permits only that excluded file dirty, and a C-only commit creates `C_SHA`. A fresh detached globally clean PR worktree at `C_SHA` runs local PR-static/T7 checks and must equal authenticated PR `candidate_head_sha`; it never equals test-merge `check_sha`. Closure common/generator-lock inputs at `IMPL_SHA` and `C_SHA` are identical because `C` is excluded. Before completion, the original status, tracked/staged binary diffs, untracked path/blob identities, and `HEAD` must byte-compare equal to their captured values. Worktree removal uses `git worktree remove` without `--force` only after all evidence is checkpointed; no `reset`, `clean`, `checkout --`, forced removal, or mutation of the original worktree is permitted.

The sole command matrix below has three separately invoked ordered stages selected by `HANONLY_ACCEPTANCE_STAGE=b0|final|t7`. Stage `b0` must finish before B1 begins; stage `final` runs only after clean `IMPL_SHA`; stage `t7` runs only after the C-only PR check and authenticated envelopes exist. This is one normative command surface, not one long-lived shell process.

## Required command matrix

```sh
set -euo pipefail

# This is the sole normative command matrix. Invoke one ordered stage per shell:
# b0 before B1, final at clean IMPL_SHA, then t7 after authenticated PR evidence.
: "${HANONLY_ACCEPTANCE_STAGE:?b0, final, or t7 required}"
: "${HANONLY_ORIGINAL_WORKTREE:?original dirty worktree required}"
: "${HANONLY_IMPLEMENTATION_WORKTREE:?clean implementation worktree required}"
: "${HANONLY_SHARED_EVIDENCE_BASE:?canonical external evidence base required}"
: "${HANONLY_ORIGINAL_SNAPSHOT_DIR:?pre-implementation original snapshot required}"
: "${HANONLY_VISUAL_EVIDENCE_ROOT:?D0 evidence root required}"
case "$HANONLY_ACCEPTANCE_STAGE" in
  b0)
    : "${HANONLY_B0_WORKTREE:?clean detached B0 worktree required}"
    : "${HANONLY_B0_SHA:?immutable pre-B1 B0 SHA required}"
    worktree="$HANONLY_B0_WORKTREE"
    expected_sha="$HANONLY_B0_SHA"
    ;;
  final)
    : "${HANONLY_ACCEPTANCE_WORKTREE:?clean detached acceptance worktree required}"
    : "${HANONLY_PR_WORKTREE:?future clean detached PR worktree path required}"
    : "${HANONLY_IMPLEMENTATION_SHA:?immutable implementation SHA required}"
    : "${HANONLY_B0_SHA:?immutable pre-B1 B0 SHA required}"
    : "${HANONLY_B0_ARTIFACT_SHA256:?frozen B0 artifact digest required}"
    : "${HANONLY_B0_AUTHORIZATION_SHA256:?frozen B0 authorization-record digest required}"
    worktree="$HANONLY_ACCEPTANCE_WORKTREE"
    expected_sha="$HANONLY_IMPLEMENTATION_SHA"
    ;;
  t7)
    : "${HANONLY_PR_WORKTREE:?clean detached PR worktree required}"
    worktree="$HANONLY_PR_WORKTREE"
    expected_sha="$(git -C "$HANONLY_PR_WORKTREE" rev-parse HEAD)"
    ;;
  *)
    printf '%s\n' 'HANONLY_ACCEPTANCE_STAGE must be b0, final, or t7' >&2
    exit 2
    ;;
esac
test "$(git -C "$HANONLY_ORIGINAL_WORKTREE" rev-parse HEAD)" = "$(cat "$HANONLY_ORIGINAL_SNAPSHOT_DIR/head.txt")"
test -f "$HANONLY_ORIGINAL_SNAPSHOT_DIR/status.z"
test -f "$HANONLY_ORIGINAL_SNAPSHOT_DIR/worktree.diff"
test -f "$HANONLY_ORIGINAL_SNAPSHOT_DIR/index.diff"
test -f "$HANONLY_ORIGINAL_SNAPSHOT_DIR/untracked.zhash"
export HANONLY_ORIGINAL_SNAPSHOT_DIR
cd "$worktree"
test "$(pwd -P)" = "$worktree"
repo_root="$worktree"
test "$(git rev-parse HEAD)" = "$expected_sha"
test -z "$(git status --porcelain=v1 --untracked-files=all)"

HANONLY_B0_FROZEN_INTERPRETER_PATHS=(
  .omx/plans/hanonly-r50-b0-evidence-contract.json
  scripts/check-hanonly-production-policy.ts
  scripts/check-hanonly-production-policy.test.ts
  scripts/hanonly_evidence_ledger.py
  scripts/hanonly_evidence_ledger_test.py
  package.json
  ui/package.json
  bun.lock
)
if test "$HANONLY_ACCEPTANCE_STAGE" = final; then
  git cat-file -e "${HANONLY_B0_SHA}^{commit}"
  git cat-file -e "${HANONLY_IMPLEMENTATION_SHA}^{commit}"
  for frozen_path in "${HANONLY_B0_FROZEN_INTERPRETER_PATHS[@]}"; do
    for frozen_sha in "$HANONLY_B0_SHA" "$HANONLY_IMPLEMENTATION_SHA"; do
      git cat-file -e "${frozen_sha}:${frozen_path}"
      test "$(git cat-file -t "${frozen_sha}:${frozen_path}")" = blob
      test "$(git ls-tree "$frozen_sha" -- "$frozen_path" | cut -d' ' -f1)" = 100644
    done
    test "$(git rev-parse "${HANONLY_B0_SHA}:${frozen_path}")" = \
         "$(git rev-parse "${HANONLY_IMPLEMENTATION_SHA}:${frozen_path}")"
  done
  git diff --quiet --no-ext-diff --no-renames \
    "${HANONLY_B0_SHA}^{tree}" \
    "${HANONLY_IMPLEMENTATION_SHA}^{tree}" \
    -- "${HANONLY_B0_FROZEN_INTERPRETER_PATHS[@]}"
  bun scripts/check-hanonly-production-policy.ts \
    --validate-red-test-state final
  bun cargo test --workspace --tests
fi

if test "$HANONLY_ACCEPTANCE_STAGE" = t7; then
  export HANONLY_PRODUCTION_CLOSURE="$HANONLY_VISUAL_EVIDENCE_ROOT/production-closure.json"
else
D0_VISUAL_EVIDENCE_ROOT="$HANONLY_VISUAL_EVIDENCE_ROOT"
ledger_values=()
while IFS= read -r -d '' value; do
  ledger_values+=("$value")
done < <(
  python3 scripts/hanonly_evidence_ledger.py rehydrate \
    --repo-root "$repo_root" \
    --evidence-root "$D0_VISUAL_EVIDENCE_ROOT"
)
test "${#ledger_values[@]}" -eq 6
HANONLY_VISUAL_INPUT="${ledger_values[0]}"
HANONLY_VISUAL_INPUT_SHA256="${ledger_values[1]}"
HANONLY_VISUAL_MANIFEST="${ledger_values[2]}"
HANONLY_VISUAL_MANIFEST_SHA256="${ledger_values[3]}"
HANONLY_VISUAL_EVIDENCE_ROOT="${ledger_values[4]}"
HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256="${ledger_values[5]}"
test "$HANONLY_VISUAL_EVIDENCE_ROOT" = "$D0_VISUAL_EVIDENCE_ROOT"
test "$(git rev-parse --show-toplevel)" = "$repo_root"
test -z "$(git status --porcelain=v1 -- crates/koharu-app/tests/fixtures/source-gate-deterministic-recall/fixture-manifest.json)"
export HANONLY_VISUAL_INPUT HANONLY_VISUAL_INPUT_SHA256
export HANONLY_VISUAL_MANIFEST HANONLY_VISUAL_MANIFEST_SHA256
export HANONLY_VISUAL_EVIDENCE_ROOT HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256
readonly HANONLY_VISUAL_INPUT HANONLY_VISUAL_INPUT_SHA256
readonly HANONLY_VISUAL_MANIFEST HANONLY_VISUAL_MANIFEST_SHA256
readonly HANONLY_VISUAL_EVIDENCE_ROOT HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256

if test "$HANONLY_ACCEPTANCE_STAGE" = b0; then
# Pre-B0 gate: no selection artifact may exist before this block passes.
selection_artifact="$HANONLY_VISUAL_EVIDENCE_ROOT/crop-policy-selection.json"
pre_b0_attestation="$HANONLY_VISUAL_EVIDENCE_ROOT/b0-preflight-attestation.json"
run_pre_b0=true
run_calibration=true
run_seal=true
run_holdout=true
pre_b0_state="$(
  bun scripts/check-hanonly-production-policy.ts \
    --admit-b0-preflight-state \
    --attestation "$pre_b0_attestation" \
    --artifact "$selection_artifact" \
    --expected-b0-sha "$HANONLY_B0_SHA" \
    --evidence-root "$HANONLY_VISUAL_EVIDENCE_ROOT"
)"
case "$pre_b0_state" in
  fresh)
    ;;
  preflight_complete)
    run_pre_b0=false
    ;;
  *)
    printf '%s\n' 'unsafe B0 preflight admission state' >&2
    exit 1
    ;;
esac
if test -f "$selection_artifact"; then
  test "$run_pre_b0" = false
  b0_resume_state="$(
    bun scripts/check-hanonly-production-policy.ts \
      --classify-b0-resume-state \
      --artifact "$selection_artifact" \
      --expected-b0-sha "$HANONLY_B0_SHA" \
      --evidence-root "$HANONLY_VISUAL_EVIDENCE_ROOT"
  )"
  case "$b0_resume_state" in
    calibration_frozen)
      run_calibration=false
      ;;
    seal_incomplete)
      run_calibration=false
      ;;
    seal_complete_holdout_unopened)
      run_calibration=false
      run_seal=false
      ;;
    holdout_complete_authorization_missing|holdout_complete_authorization_incomplete|authorized)
      run_calibration=false
      run_seal=false
      run_holdout=false
      ;;
    *)
      printf '%s\n' 'unclassified or unsafe B0 resume state' >&2
      exit 1
      ;;
  esac
fi
if "$run_pre_b0"; then
bun cargo test --workspace --tests --no-run
expect_hanonly_red() {
  bun scripts/check-hanonly-production-policy.ts \
    --capture-staged-red \
    --crate "$1" \
    --test-id "$2" \
    --evidence-root "$HANONLY_VISUAL_EVIDENCE_ROOT"
}
expect_hanonly_red koharu-app hanonly_pre_b1_red_t2_dynamic_layout_contract
expect_hanonly_red koharu-app hanonly_pre_b1_red_t2_pipeline_layout_handoff_contract
expect_hanonly_red koharu-app hanonly_pre_b1_red_t2_blob_decode_budget_contract
expect_hanonly_red koharu-rpc hanonly_pre_b1_red_t2_replace_import_atomicity_contract
expect_hanonly_red koharu-app hanonly_pre_b1_red_t2_rotation_status_contract
expect_hanonly_red koharu-app hanonly_pre_greenc_red_t3_transient_planner_hint_contract
expect_hanonly_red koharu-app hanonly_pre_greenc_red_t3_run_state_lifetime_contract
expect_hanonly_red koharu-app hanonly_pre_greenc_red_t3_planner_font_outcome_contract
expect_hanonly_red koharu-app hanonly_pre_greenc_red_t3_source_color_contract
expect_hanonly_red koharu-core hanonly_pre_greenc_red_t3_marker_batch_atomicity_contract
expect_hanonly_red koharu-app hanonly_pre_greenc_red_t3_untrusted_marker_lifecycle_contract
expect_hanonly_red koharu-rpc hanonly_pre_greenc_red_t3_http_marker_rejection_contract
expect_hanonly_red koharu-rpc hanonly_pre_greenc_red_t3_mcp_marker_rejection_contract
expect_hanonly_red koharu-renderer hanonly_pre_greenc_red_t3_source_color_probe_contract
bun cargo test -p koharu-app --features hanonly-test-evidence \
  hanonly_pre_b1_red_t2_source_gate_ratio_contract --lib -- \
  --exact --nocapture
bun cargo test -p koharu-ml \
  hanonly_pre_b1_red_t2_crop_local_ppocr_contract --lib -- \
  --exact --nocapture
bun scripts/check-hanonly-production-policy.ts \
  --validate-red-test-state b0
python3 -m unittest scripts/hanonly_evidence_ledger_test.py
bun cargo test -p koharu-llm --no-default-features --lib
bun cargo test -p koharu-ml --no-default-features --lib
bun cargo test --workspace --tests
bun cargo check --workspace --all-targets
bun test scripts/check-hanonly-production-policy.test.ts
bun scripts/check-hanonly-production-policy.ts --test-dependency-inventory
bun scripts/check-hanonly-production-policy.ts --release-feature-inventory
bun scripts/check-hanonly-production-policy.ts \
  --capture-b0-cargo-inventory \
  --evidence-root "$HANONLY_VISUAL_EVIDENCE_ROOT"
default_cargo_messages="$HANONLY_VISUAL_EVIDENCE_ROOT/cargo/default.jsonl"
evidence_cargo_messages="$HANONLY_VISUAL_EVIDENCE_ROOT/cargo/evidence.jsonl"
bun scripts/check-hanonly-production-policy.ts --verify-generated-rust \
  --manifest "$HANONLY_VISUAL_MANIFEST" \
  --evidence-root "$HANONLY_VISUAL_EVIDENCE_ROOT" \
  --cargo-default-messages "$default_cargo_messages" \
  --cargo-evidence-messages "$evidence_cargo_messages"
bun cargo test -p koharu-app --features hanonly-test-evidence hanonly_test_evidence_bridge_reachable -- --nocapture
bun cargo test -p koharu-app --features hanonly-test-evidence source_gate_crop -- --nocapture
bun scripts/check-hanonly-production-policy.ts \
  --write-b0-preflight-attestation \
  --attestation-out "$pre_b0_attestation" \
  --expected-b0-sha "$HANONLY_B0_SHA" \
  --evidence-root "$HANONLY_VISUAL_EVIDENCE_ROOT"
fi

# B0: two fresh Source Gate-only processes with one exact feature set.
: "${HANONLY_VISUAL_EVIDENCE_ROOT:?D0 evidence root required}"
selection_reports="$HANONLY_VISUAL_EVIDENCE_ROOT/source-gate-selection"
check_reports="$selection_reports/checks"
calibration_ledger="$HANONLY_VISUAL_EVIDENCE_ROOT/evidence-ledger.json"
pre_calibration_check="$check_reports/pre-calibration.json"
pre_holdout_check="$check_reports/pre-holdout.json"
if "$run_calibration"; then
HANONLY_B0_REQUIRED_CHECK_PHASE="pre-calibration" \
HANONLY_B0_REQUIRED_CHECK_ATTESTATION_OUT="$pre_calibration_check" \
HANONLY_CALIBRATION_MANIFEST="$HANONLY_VISUAL_MANIFEST" \
HANONLY_CALIBRATION_LEDGER="$calibration_ledger" \
HANONLY_B0_SHA="$HANONLY_B0_SHA" \
bun scripts/check-hanonly-production-policy.ts --b0-source-gate-anti-fixture
HANONLY_B0_SHA="$HANONLY_B0_SHA" \
HANONLY_SOURCE_GATE_REQUIRED_CHECK_ATTESTATION="$pre_calibration_check" \
HANONLY_R50_CALIBRATION_MANIFEST="$HANONLY_VISUAL_MANIFEST" \
HANONLY_R50_CALIBRATION_LEDGER="$calibration_ledger" \
HANONLY_SOURCE_GATE_SELECTION_PHASE="calibration-freeze" \
HANONLY_SOURCE_GATE_SELECTION_ARTIFACT="$selection_artifact" \
HANONLY_SOURCE_GATE_SELECTION_REPORT_DIR="$selection_reports/calibration-freeze" \
bun cargo test -p koharu-app --features metal,hanonly-test-evidence \
  han_only_source_gate_crop_selection_matrix --lib -- \
  --ignored --nocapture --test-threads=1
fi
holdout_seal_dir="$selection_reports/holdout-seal"
holdout_manifest="$holdout_seal_dir/r50-holdout-manifest.json"
holdout_ledger="$holdout_seal_dir/r50-holdout-ledger.json"
holdout_decode_attestation="$holdout_seal_dir/r50-holdout-decode-attestation.json"
if "$run_seal"; then
: "${HANONLY_R50_HOLDOUT_SEAL_INPUT:?independent sealed holdout intake required after candidate freeze}"
: "${HANONLY_R50_HOLDOUT_CUSTODY_ATTESTATION:?independent holdout custody attestation required}"
: "${HANONLY_R50_HOLDOUT_OPERATOR_ATTESTATION:?independent holdout operator attestation required}"
: "${HANONLY_R50_ERASE_MASK_LANE_ATTESTATION:?independent erase-mask lane attestation required}"
: "${HANONLY_R50_RESIDUAL_MASK_LANE_ATTESTATION:?independent residual-mask lane attestation required}"
: "${HANONLY_R50_HISTORICAL_ROOT_REGISTRY:?pre-production-edit frozen historical root registry required}"
: "${CODEX_NATIVE_SUBAGENT_RECEIPT_ROOT:?read-only native subagent receipt root required}"
HANONLY_B0_SHA="$HANONLY_B0_SHA" \
HANONLY_SOURCE_GATE_SELECTION_ARTIFACT="$selection_artifact" \
HANONLY_R50_HOLDOUT_SEAL_INPUT="$HANONLY_R50_HOLDOUT_SEAL_INPUT" \
HANONLY_R50_HOLDOUT_DECODE_ATTESTATION_OUT="$holdout_decode_attestation" \
bun cargo test -p koharu-app --features hanonly-test-evidence \
  hanonly_r50_holdout_seal_decode_preflight --lib -- \
  --ignored --nocapture --test-threads=1
python3 scripts/hanonly_evidence_ledger.py seal-holdout \
  --repo-root "$repo_root" \
  --expected-base "$HANONLY_SHARED_EVIDENCE_BASE" \
  --b0-sha "$HANONLY_B0_SHA" \
  --calibration-manifest "$HANONLY_VISUAL_MANIFEST" \
  --calibration-artifact "$selection_artifact" \
  --sealed-input "$HANONLY_R50_HOLDOUT_SEAL_INPUT" \
  --custody-attestation "$HANONLY_R50_HOLDOUT_CUSTODY_ATTESTATION" \
  --operator-attestation "$HANONLY_R50_HOLDOUT_OPERATOR_ATTESTATION" \
  --erase-mask-lane-attestation "$HANONLY_R50_ERASE_MASK_LANE_ATTESTATION" \
  --residual-mask-lane-attestation "$HANONLY_R50_RESIDUAL_MASK_LANE_ATTESTATION" \
  --historical-root-registry "$HANONLY_R50_HISTORICAL_ROOT_REGISTRY" \
  --native-subagent-receipt-root "$CODEX_NATIVE_SUBAGENT_RECEIPT_ROOT" \
  --decoded-attestation "$holdout_decode_attestation" \
  --output-manifest "$holdout_manifest" \
  --output-ledger "$holdout_ledger"
fi
if "$run_holdout"; then
HANONLY_B0_REQUIRED_CHECK_PHASE="pre-holdout" \
HANONLY_B0_REQUIRED_CHECK_ATTESTATION_OUT="$pre_holdout_check" \
HANONLY_CALIBRATION_MANIFEST="$HANONLY_VISUAL_MANIFEST" \
HANONLY_CALIBRATION_LEDGER="$calibration_ledger" \
HANONLY_HOLDOUT_MANIFEST="$holdout_manifest" \
HANONLY_HOLDOUT_LEDGER="$holdout_ledger" \
HANONLY_B0_SHA="$HANONLY_B0_SHA" \
bun scripts/check-hanonly-production-policy.ts --b0-source-gate-anti-fixture
HANONLY_B0_SHA="$HANONLY_B0_SHA" \
HANONLY_SOURCE_GATE_REQUIRED_CHECK_ATTESTATION="$pre_holdout_check" \
HANONLY_R50_CALIBRATION_MANIFEST="$HANONLY_VISUAL_MANIFEST" \
HANONLY_R50_CALIBRATION_LEDGER="$calibration_ledger" \
HANONLY_R50_HOLDOUT_MANIFEST="$holdout_manifest" \
HANONLY_R50_HOLDOUT_LEDGER="$holdout_ledger" \
HANONLY_SOURCE_GATE_SELECTION_PHASE="holdout" \
HANONLY_SOURCE_GATE_SELECTION_ARTIFACT="$selection_artifact" \
HANONLY_SOURCE_GATE_SELECTION_REPORT_DIR="$selection_reports/holdout" \
bun cargo test -p koharu-app --features metal,hanonly-test-evidence \
  han_only_source_gate_crop_selection_matrix --lib -- \
  --ignored --nocapture --test-threads=1
fi
b0_artifact_sha256="$(shasum -a 256 "$selection_artifact" | awk '{print $1}')"
b0_authorization_record="$selection_reports/b0-authorization.json"
b0_authorization_sha256="$(
  bun scripts/check-hanonly-production-policy.ts \
    --validate-b0-authorization \
    --artifact "$selection_artifact" \
    --expected-artifact-sha256 "$b0_artifact_sha256" \
    --expected-b0-sha "$HANONLY_B0_SHA" \
    --b0-preflight-attestation "$pre_b0_attestation" \
    --calibration-manifest "$HANONLY_VISUAL_MANIFEST" \
    --calibration-ledger "$calibration_ledger" \
    --holdout-manifest "$holdout_manifest" \
    --holdout-ledger "$holdout_ledger" \
    --holdout-custody-attestation "$HANONLY_R50_HOLDOUT_CUSTODY_ATTESTATION" \
    --holdout-operator-attestation "$HANONLY_R50_HOLDOUT_OPERATOR_ATTESTATION" \
    --erase-mask-lane-attestation "$HANONLY_R50_ERASE_MASK_LANE_ATTESTATION" \
    --residual-mask-lane-attestation "$HANONLY_R50_RESIDUAL_MASK_LANE_ATTESTATION" \
    --historical-root-registry "$HANONLY_R50_HISTORICAL_ROOT_REGISTRY" \
    --native-subagent-receipt-root "$CODEX_NATIVE_SUBAGENT_RECEIPT_ROOT" \
    --holdout-decode-attestation "$holdout_decode_attestation" \
    --rerun-holdout-decode-attestation \
    --required-check-attestation "$pre_calibration_check" \
    --required-check-attestation "$pre_holdout_check" \
    --authorization-record-out "$b0_authorization_record" \
    --emit-authorization-record-sha256
)"
case "$b0_authorization_sha256" in
  ''|*[!0-9a-f]*)
    printf '%s\n' 'B0 authorization digest must be exactly 64 lowercase hex characters' >&2
    exit 1
    ;;
esac
test "${#b0_authorization_sha256}" -eq 64
test -z "$(git status --porcelain=v1 --untracked-files=all)"
printf 'HANONLY_B0_AUTHORIZATION_SHA256=%s\n' "$b0_authorization_sha256"
printf 'HANONLY_B0_ARTIFACT_SHA256=%s\n' "$b0_artifact_sha256"
exit 0
fi

# Final IMPL_SHA acceptance validates the one frozen artifact; it never selects.
selection_artifact="$HANONLY_VISUAL_EVIDENCE_ROOT/crop-policy-selection.json"
pre_calibration_check="$HANONLY_VISUAL_EVIDENCE_ROOT/source-gate-selection/checks/pre-calibration.json"
pre_holdout_check="$HANONLY_VISUAL_EVIDENCE_ROOT/source-gate-selection/checks/pre-holdout.json"
calibration_ledger="$HANONLY_VISUAL_EVIDENCE_ROOT/evidence-ledger.json"
holdout_manifest="$HANONLY_VISUAL_EVIDENCE_ROOT/source-gate-selection/holdout-seal/r50-holdout-manifest.json"
holdout_ledger="$HANONLY_VISUAL_EVIDENCE_ROOT/source-gate-selection/holdout-seal/r50-holdout-ledger.json"
holdout_decode_attestation="$HANONLY_VISUAL_EVIDENCE_ROOT/source-gate-selection/holdout-seal/r50-holdout-decode-attestation.json"
b0_authorization_record="$HANONLY_VISUAL_EVIDENCE_ROOT/source-gate-selection/b0-authorization.json"
bun scripts/check-hanonly-production-policy.ts \
  --validate-b0-authorization \
  --artifact "$selection_artifact" \
  --expected-b0-sha "$HANONLY_B0_SHA" \
  --expected-artifact-sha256 "$HANONLY_B0_ARTIFACT_SHA256" \
  --authorization-record "$b0_authorization_record" \
  --expected-authorization-record-sha256 "$HANONLY_B0_AUTHORIZATION_SHA256" \
  --b0-preflight-attestation "$HANONLY_VISUAL_EVIDENCE_ROOT/b0-preflight-attestation.json" \
  --calibration-manifest "$HANONLY_VISUAL_MANIFEST" \
  --calibration-ledger "$calibration_ledger" \
  --holdout-manifest "$holdout_manifest" \
  --holdout-ledger "$holdout_ledger" \
  --holdout-custody-attestation-from-ledger "$holdout_ledger" \
  --holdout-lane-attestations-from-ledger "$holdout_ledger" \
  --native-subagent-receipts-from-ledger "$holdout_ledger" \
  --historical-root-registry-from-calibration-ledger "$calibration_ledger" \
  --holdout-decode-attestation "$holdout_decode_attestation" \
  --verify-recorded-holdout-decode-attestation \
  --required-check-attestation "$pre_calibration_check" \
  --required-check-attestation "$pre_holdout_check" \
  --verify-frozen-recall-contract-in-production

# Remaining deterministic and post-GREEN acceptance.
bun cargo test -p koharu-app ctd_segment -- --nocapture
bun cargo test -p koharu-app final_inpaint_mask -- --nocapture
bun cargo test -p koharu-app inpaint_dispatch -- --nocapture
bun cargo test -p koharu-ml inpainting::mask -- --nocapture
bun cargo test -p koharu-app typography -- --nocapture
bun cargo test -p koharu-core typography_plan -- --nocapture
bun cargo test -p koharu-app untrusted_project_open -- --nocapture
bun cargo test -p koharu-app history -- --nocapture
bun cargo test -p koharu-rpc import -- --nocapture
bun cargo test -p koharu-rpc typography_plan_verified -- --nocapture
bun cargo test -p koharu-rpc koharu_apply -- --nocapture
bun cargo test -p koharu-rpc completed_with_errors -- --nocapture
bun cargo test -p koharu-app source_relative -- --nocapture
bun cargo test -p koharu-app han_only_renderer -- --nocapture
bun cargo test -p koharu-app han_only_source_color_work_budget -- --nocapture
bun cargo test -p koharu-app pipeline -- --nocapture
public_equivalence_root="$HANONLY_VISUAL_EVIDENCE_ROOT/public-feature-equivalence"
test ! -e "$public_equivalence_root"
HANONLY_PUBLIC_EQ_REPORT="$public_equivalence_root/default.json" \
  bun cargo test -p koharu-app \
    hanonly_pre_greenc_red_t3_run_state_lifetime_contract --lib -- \
    --nocapture --test-threads=1
HANONLY_PUBLIC_EQ_REPORT="$public_equivalence_root/evidence.json" \
  bun cargo test -p koharu-app --features hanonly-test-evidence \
    hanonly_pre_greenc_red_t3_run_state_lifetime_contract --lib -- \
    --nocapture --test-threads=1
bun scripts/check-hanonly-production-policy.ts \
  --compare-public-output-equivalence \
  "$public_equivalence_root/default.json" \
  "$public_equivalence_root/evidence.json"
bun cargo test -p koharu-app --features metal,hanonly-test-evidence han_only_visual_manifest_matrix --lib -- --ignored --nocapture --test-threads=1
# T6 canonical runtime acceptance -- this is the sole normative T6 command block.
test -z "$(git status --porcelain=v1 --untracked-files=all)"
export HANONLY_PRODUCTION_CLOSURE="$HANONLY_VISUAL_EVIDENCE_ROOT/production-closure.json"
test ! -e "$HANONLY_PRODUCTION_CLOSURE"
# Internally reruns computeProductionClosureV1 twice with fresh CARGO_TARGET_DIR
# values and the exact locked host-target Cargo argv; it consumes no prior log.
bun scripts/check-hanonly-production-policy.ts \
  --write-production-closure "$HANONLY_PRODUCTION_CLOSURE"
runtime_root="$HANONLY_VISUAL_EVIDENCE_ROOT/runtime"
test ! -e "$runtime_root"
for process_index in 1 2; do
  for device in cpu metal; do
    artifact_dir="$runtime_root/process-$process_index/$device"
    HANONLY_VISUAL_DEVICE="$device" \
    HANONLY_VISUAL_PROCESS_INDEX="$process_index" \
    HANONLY_VISUAL_REPORT="$runtime_root/process-$process_index/$device.json" \
    HANONLY_VISUAL_ARTIFACT_DIR="$artifact_dir" \
    HANONLY_VISUAL_RUNS=10 \
    bun cargo test -p koharu-app --features metal,hanonly-test-evidence \
      han_only_visual_runtime_matrix --lib -- \
      --ignored --nocapture --test-threads=1
  done
done
bun scripts/check-hanonly-production-policy.ts --validate-runtime-matrix \
  --manifest "$HANONLY_VISUAL_MANIFEST" \
  --evidence-root "$HANONLY_VISUAL_EVIDENCE_ROOT" \
  --production-closure "$HANONLY_PRODUCTION_CLOSURE" \
  --expected-cells 360
bun cargo clippy --workspace --all-targets -- -D warnings
bun cargo fmt --all -- --check
bun run test:ui
bun run lint:ui
bun run check:generated
bun run format:check
bun run build
test -z "$(git status --porcelain=v1 --untracked-files=all)"
bun scripts/check-hanonly-production-policy.ts --manifest "$HANONLY_VISUAL_MANIFEST" --evidence-root "$HANONLY_VISUAL_EVIDENCE_ROOT"

# Generate C only from the clean implementation branch at the same IMPL_SHA.
test -z "$(git status --porcelain=v1 --untracked-files=all)"
cd "$HANONLY_IMPLEMENTATION_WORKTREE"
test "$(git rev-parse HEAD)" = "$HANONLY_IMPLEMENTATION_SHA"
test -z "$(git status --porcelain=v1 --untracked-files=all)"
test ! -e scripts/fixtures/hanonly-production-policy-ci-corpus.json
bun scripts/check-hanonly-production-policy.ts \
  --generate-ci-corpus \
  --manifest "$HANONLY_VISUAL_MANIFEST" \
  --evidence-root "$HANONLY_VISUAL_EVIDENCE_ROOT" \
  --runtime-validation "$HANONLY_VISUAL_EVIDENCE_ROOT/runtime-matrix-validation.json" \
  --full-audit "$HANONLY_VISUAL_EVIDENCE_ROOT/production-policy-audit.json" \
  --output scripts/fixtures/hanonly-production-policy-ci-corpus.json
bun scripts/check-hanonly-production-policy.ts --release-feature-inventory
bun scripts/check-hanonly-production-policy.ts --test-dependency-inventory
local_precommit_summary="$HANONLY_VISUAL_EVIDENCE_ROOT/local-precommit-closure-summary.json"
test ! -e "$local_precommit_summary"
HANONLY_CLOSURE_SUMMARY_OUT="$local_precommit_summary" \
  bun scripts/check-hanonly-production-policy.ts \
    --ci-static \
    --ci-corpus scripts/fixtures/hanonly-production-policy-ci-corpus.json
test "$(grep -c '^{\"candidate_head_sha\"' "$local_precommit_summary")" -eq 1
test -z "$(git status --porcelain=v1 -- crates/koharu-app/tests/fixtures/source-gate-deterministic-recall/fixture-manifest.json)"
test "$(git status --porcelain=v1 --untracked-files=all | sed -n '1p')" = "?? scripts/fixtures/hanonly-production-policy-ci-corpus.json"
test "$(git status --porcelain=v1 --untracked-files=all | wc -l | tr -d ' ')" = 1
git add -- scripts/fixtures/hanonly-production-policy-ci-corpus.json
test "$(git diff --cached --name-only)" = "scripts/fixtures/hanonly-production-policy-ci-corpus.json"
git diff --cached --check
git commit -m "ci: refresh HanOnly production policy corpus"
HANONLY_C_SHA="$(git rev-parse HEAD)"
export HANONLY_C_SHA
test -z "$(git status --porcelain=v1 --untracked-files=all)"

# Create the globally clean PR worktree. Local CI writes its summary externally.
test ! -e "$HANONLY_PR_WORKTREE"
git worktree add --detach "$HANONLY_PR_WORKTREE" "$HANONLY_C_SHA"
cd "$HANONLY_PR_WORKTREE"
test "$(git rev-parse HEAD)" = "$HANONLY_C_SHA"
test -z "$(git status --porcelain=v1 --untracked-files=all)"
local_ci_summary="$HANONLY_VISUAL_EVIDENCE_ROOT/local-pr-closure-summary.json"
test ! -e "$local_ci_summary"
HANONLY_CLOSURE_SUMMARY_OUT="$local_ci_summary" \
  bun scripts/check-hanonly-production-policy.ts \
    --ci-static \
    --ci-corpus scripts/fixtures/hanonly-production-policy-ci-corpus.json
test "$(grep -c '^{\"candidate_head_sha\"' "$local_ci_summary")" -eq 1
test -z "$(git status --porcelain=v1 --untracked-files=all)"
printf 'HANONLY_C_SHA=%s\n' "$HANONLY_C_SHA"
exit 0
fi

# T7 resumes only after this exact C_SHA is the authenticated PR candidate
# head, the required job passes on the current test merge, and every envelope exists.
: "${HANONLY_GITHUB_RULESET_JSON:?authenticated Ruleset API evidence required}"
: "${HANONLY_GITHUB_CHECK_RUNS_JSON:?test-merge check-run API evidence required}"
: "${HANONLY_GITHUB_CHECK_ANNOTATIONS_JSON:?test-merge check-annotation API evidence required}"
: "${HANONLY_GITHUB_PULL_REQUEST_JSON:?implementation pull-request API evidence required}"
: "${HANONLY_GITHUB_COLLABORATORS_JSON:?collaborator-permission API evidence required}"
: "${HANONLY_GITHUB_CANDIDATE_HEAD_SHA:?implementation candidate-head SHA required}"
: "${HANONLY_GITHUB_CHECK_SHA:?pull-request test-merge check SHA required}"
test -z "$(git status --porcelain=v1)"
test "$(git rev-parse HEAD)" = "$HANONLY_GITHUB_CANDIDATE_HEAD_SHA"
test "$HANONLY_GITHUB_CANDIDATE_HEAD_SHA" != "$HANONLY_GITHUB_CHECK_SHA"
bun scripts/check-hanonly-production-policy.ts \
  --verify-required-check-ruleset \
  --ruleset-json "$HANONLY_GITHUB_RULESET_JSON" \
  --check-runs-json "$HANONLY_GITHUB_CHECK_RUNS_JSON" \
  --check-annotations-json "$HANONLY_GITHUB_CHECK_ANNOTATIONS_JSON" \
  --pull-request-json "$HANONLY_GITHUB_PULL_REQUEST_JSON" \
  --collaborators-json "$HANONLY_GITHUB_COLLABORATORS_JSON" \
  --ci-corpus scripts/fixtures/hanonly-production-policy-ci-corpus.json \
  --evidence-root "$HANONLY_VISUAL_EVIDENCE_ROOT" \
  --production-closure "$HANONLY_PRODUCTION_CLOSURE" \
  --attestation-out "$HANONLY_VISUAL_EVIDENCE_ROOT/t7-governance-attestation.json" \
  --repository nbjinkui1980-tech/EC-image-koharu \
  --target-ref refs/heads/main \
  --candidate-head-sha "$HANONLY_GITHUB_CANDIDATE_HEAD_SHA" \
  --check-sha "$HANONLY_GITHUB_CHECK_SHA" \
  --required-context "HanOnly Production Policy"
git diff --check
test -z "$(git status --porcelain=v1 --untracked-files=all)"

# Prove the original dirty worktree was never changed.
original_after="$HANONLY_VISUAL_EVIDENCE_ROOT/original-after"
test ! -e "$original_after"
mkdir "$original_after"
git -C "$HANONLY_ORIGINAL_WORKTREE" rev-parse HEAD >"$original_after/head.txt"
git -C "$HANONLY_ORIGINAL_WORKTREE" status --porcelain=v1 -z --untracked-files=all >"$original_after/status.z"
git -C "$HANONLY_ORIGINAL_WORKTREE" diff --binary --no-ext-diff >"$original_after/worktree.diff"
git -C "$HANONLY_ORIGINAL_WORKTREE" diff --cached --binary --no-ext-diff >"$original_after/index.diff"
while IFS= read -r -d '' untracked_path; do
  printf '%s\0' "$untracked_path"
  git -C "$HANONLY_ORIGINAL_WORKTREE" hash-object --no-filters -- "$untracked_path"
done < <(git -C "$HANONLY_ORIGINAL_WORKTREE" ls-files --others --exclude-standard -z) \
  >"$original_after/untracked.zhash"
cmp "$HANONLY_ORIGINAL_SNAPSHOT_DIR/head.txt" "$original_after/head.txt"
cmp "$HANONLY_ORIGINAL_SNAPSHOT_DIR/status.z" "$original_after/status.z"
cmp "$HANONLY_ORIGINAL_SNAPSHOT_DIR/worktree.diff" "$original_after/worktree.diff"
cmp "$HANONLY_ORIGINAL_SNAPSHOT_DIR/index.diff" "$original_after/index.diff"
cmp "$HANONLY_ORIGINAL_SNAPSHOT_DIR/untracked.zhash" "$original_after/untracked.zhash"
```

The policy audit derives Rust package/target roots from the repository wrapper command `bun --silent run scripts/dev.ts cargo metadata --no-deps --format-version 1`. The script must use this exact machine-JSON command rather than pipe the higher-level `bun cargo metadata` form, whose storage preflight emits non-JSON command output. For every workspace package it scans all `.rs` files beneath the package `src/` tree, every production target `src_path` whose Cargo target kind is `lib`, `bin`, `cdylib`, `staticlib`, or `proc-macro` even when it is outside `src/` (including current `crates/*/bin/*.rs` targets), every custom-build/build-script `src_path`, and every `.rs` module recursively reached from those target roots through balanced `mod name;` or `#[path = "..."] mod name;` declarations. It rejects unresolved module declarations, metadata/module paths outside the owning package root, and any discovered production target not classified into scanned or explicit generated exclusions. It also scans every `ui/**/*.{ts,tsx}` production source. It excludes `.next`, `out`, generated API/schema paths, Cargo targets whose kind is only `test`, `example`, or `bench`, TS/TSX test files, Rust items lexically enclosed by `#[cfg(test)]`, feature-only observational items enclosed by exact `#[cfg(feature = "hanonly-test-evidence")]`, and app harness items enclosed by exact `#[cfg(all(test, feature = "hanonly-test-evidence"))]`, only after Cargo metadata proves the feature exists in exactly `koharu-app`, `koharu-llm`, and `koharu-ml`, is absent from every `default`, and the app feature propagates only to the two dependency features. One explicitly delimited D0 provenance block must be inside one of those test/evidence-only items. A broader feature exclusion, default activation, or unclassified target/path/exclusion fails. The audit also proves that the sole cross-crate AOT accessor is the hidden borrowed `Device` method enclosed by exact `#[cfg(feature = "hanonly-test-evidence")]`; every consuming app import/call/helper/run-state write/test has the same exact gate; and no device label reaches a behavior branch. Table fixtures reject an ungated public accessor, `#[cfg(test)]`, `#[cfg(any(...))]`, any broader feature expression, ungated app use/helper, default activation, missing propagation, a wrapper/trait/enum/device-label API, or engine-selection/inference/retry/output branching on the observed label. Default Cargo JSON must contain no `AotInpainting::device` reference; the feature-enabled app build must reach the real loaded instance. From the visual manifest and mandatory runtime fields it derives normalized paths/basenames; source/mask/clean-reference/decoded hashes; entry/target IDs and runtime NodeIds; dimension and ROI/bbox tuples with multiplicity; per-entry node counts; and per-target source/OCR/translated line counts. `crop_name` comes only from unique nonempty `fixtures[].name` in `crates/koharu-app/tests/fixtures/source-gate-deterministic-recall/fixture-manifest.json`. The audit descriptor-walks and reads that fixture manifest once, computes SHA-256 from the same bytes it parses, and requires exact equality with the D0 ledger, `HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256`, B0 frozen projection, and every 360-cell global fingerprint before validating its exact top-level keys, source hashes, nonempty fixture array, bounds/sizes, and decoded hashes. Dirty/task-modified fixture status fails. Reports expose counts only, never text.

Production Rust closure follows `include!` as well as `mod`. Before stripping string literals, the token-tree scanner accepts only a package-contained literal path or `concat!(env!("OUT_DIR"), "/literal.rs")`. The pre-B0 commands capture exact default and evidence Cargo JSON logs. `--verify-generated-rust` parses every line, identifies `koharu-llm` by metadata package ID, requires exactly one canonical `build-script-executed.out_dir` per log, resolves every static include, and scans every regular `.rs` recursively beneath both exact out dirs with the same corpus/token rules. This includes current `types.rs`, `llama_loader.rs`, `ggml_loader.rs`, `ggml_base_loader.rs`, `mtmd_loader.rs`, and `wrappers.rs`. Recursive includes must remain static, regular, and contained. Dynamic/unresolved macros, missing/duplicate/outside-target out-dir messages, missing/nonregular/symlink/escaping files, or any generated-Rust exclusion fail closed. The report records log hashes, package IDs, out dirs, include edges, generated hashes, and scanned paths. Fixtures cover all six current names, a generated forbidden branch, recursive static include, dynamic/unresolved include, symlink escape, missing/duplicate out-dir evidence, and clean default/evidence trees.

The same script's `--release-feature-inventory` mode runs without manifest/model evidence and scans root/workspace/package `package.json` scripts, `bunfig.toml`, `.cargo/config*`, `crates/koharu/tauri*.conf.json`, `.github/workflows/**/*.{yml,yaml}`, `scripts/**/*.{ts,js,mjs,sh}`, `Dockerfile*`, `Makefile`, and `justfile` when present. It rejects the exact feature literal `hanonly-test-evidence`, the build-script environment form `CARGO_FEATURE_HANONLY_TEST_EVIDENCE`, and the standalone Cargo argument token `--all-features` or `--all-features=...`; it does not interpret arbitrary shell. Cargo feature declarations/propagation are validated separately through metadata and are the only allowed manifest occurrences; verifier script/test files are excluded from self-matching. The required job contains the checker/test commands, the default-feature runtime-seeded metamorphic Rust test, and the `--ci-static` corpus command; none enables the evidence feature, so no feature workflow allowlist is added. Table-driven temporary-tree fixtures inject both explicit and indirect activation into every listed surface: the current macOS Tauri config, build workflow, package script, shell/TS build command, `.cargo/config.toml` alias, `bunfig.toml` command string, Dockerfile `RUN`, Makefile recipe, and justfile recipe. Every injection must be discovered and rejected; unrelated feature names and prose outside configured surfaces pass. Missing configured surfaces, unreadable files, unexpected allowlist entries, explicit feature occurrences, build-script environment injection, or indirect all-features activation fails. The pre-B0 matrix runs this inventory before any selection artifact, the final full audit internally reruns it, and the matrix runs it once more afterward.

The static scanner is defense in depth. Its direct/one-hop/same-scope rules catch checked-in literals and obvious indirection but do not claim semantic completeness against arbitrary helper depth, encoding, generated computation, or scope splitting. Required merge protection depends conjunctively on this scanner and the default-feature runtime-seeded metamorphic behavior test; passing either one alone is insufficient.

The B0 mode `--b0-source-gate-anti-fixture` has a separate exact scan set. Production roots are exactly `crates/koharu-app/src/pipeline/engines/source_language_gate.rs`, `crates/koharu-ml/src/pp_ocr_v5.rs`, and `crates/koharu-llm/src/paddleocr_vl.rs`. The exact test/evidence Rust scan root is `crates/koharu-app/src/pipeline/mod.rs`, and only items lexically enclosed by exact `#[cfg(all(test, feature = "hanonly-test-evidence"))]` in that file may consume descriptor data for the existing ignored B0 harness. Source Gate / PP-OCR-affecting scripts are exactly `scripts/check-hanonly-production-policy.ts`, `scripts/check-hanonly-production-policy.test.ts`, `scripts/hanonly_evidence_ledger.py`, and `scripts/hanonly_evidence_ledger_test.py`, plus any future helper only after it is added to `B0_FROZEN_INTERPRETER_PATHS`. The attestation `scanned_roots` field is the ordered list of those three production roots, then `crates/koharu-app/src/pipeline/mod.rs`, then those four script roots. `allowed_descriptor_roots` is the ordered subset `crates/koharu-app/src/pipeline/mod.rs`, `scripts/check-hanonly-production-policy.ts`, `scripts/check-hanonly-production-policy.test.ts`, `scripts/hanonly_evidence_ledger.py`, and `scripts/hanonly_evidence_ledger_test.py`; those roots may use descriptor data only for the allowed test/evidence purposes below, not for production inference or acceptance. Colocated Rust test modules and TypeScript/Python checker tests may contain descriptor fixtures only inside `#[cfg(test)]`, exact `#[cfg(feature = "hanonly-test-evidence")]`, exact `#[cfg(all(test, feature = "hanonly-test-evidence"))]`, or checker-test fixture literals. Descriptor-guarded role/hash/dimension/corpus-role use is allowed only when the value is read from `HANONLY_VISUAL_MANIFEST`, the fixed Source Gate fixture manifest, or a test fixture and reaches only test harness selection, reporting, evidence organization, or negative fixtures. The same value may not reach production inference, candidate formula construction, crop-local preprocessing decisions, inverse mapping, `safe_crop_bounds`, `crop_policy_parameters`, `compute_safe_crop_bounds`, `word_box_inference_scale`, `word_boxes`, or Source Gate acceptance.

The B0 mode rejects direct or one-hop production branches on `c01`, `c02`, `c03`, `c04`, `h01`, `h02`, `h03`, `h04`, `test.jpeg`, `test.webp`, fixed image hashes, fixed decoded dimensions, fixed crop/title-box coordinates, fixed NodeIds, or corpus role. It also rejects any production Source Gate acceptance predicate whose accepted/rejected decision is not syntactically traceable to OCR/VL records, finite geometry, crop-local model-input scaling, inverse-mapped source coordinates, pixel support, and protected-source constraints. Fixtures must include at least one allowed descriptor-organizing role/hash use and one rejecting fixture for each forbidden category above, including an indirect one-hop helper from Source Gate acceptance into a role/hash/dimension value.

For Rust, a no-dependency token-tree scanner strips comments and string literals, balances delimiters, and applies exact syntax-aware rules. A `const` item named `PRIMARY_CROP_POLICY` fails when its initializer path's final segment is `C2`, catching the current `const PRIMARY_CROP_POLICY: SourceGateCropPolicy = SourceGateCropPolicy::C2;` plus whitespace and fully qualified variants. Inside a branch whose condition token tree contains `SourceTextPolicy::HanOnly`, find each `SourceRelativeFontSizePolicy` struct literal, isolate the complete balanced token tree for its `offset` field expression, and recursively reject any descendant unary-minus token immediately followed by a numeric literal; this catches the current `offset: match ... { ... _ => -5.0 }` while staying scoped to the HanOnly policy field. For TS/TSX, the installed TypeScript AST rejects any definition or reference of `eligibleSourceLayout`, `automaticSourceSize`, or `groupedAutomaticSourceSizes` in the HanOnly automatic UI path; any renamed direct or one-hop helper that derives a numeric automatic value from source/OCR boxes, prediction, detection, or fallback; and any reachable `-5`, `12..28`, `72`, or source-box cap/deduction. It permits existing manual-size and AllText numeric logic and the Auto/empty HanOnly path. Separate fixtures require the UI PNG/JPEG/WebP allowlist to match the backend contract.

The manifest also derives one unordered forbidden integer dimension pair per full-page decoded source. Rust normalizes decimal/hex/octal/binary literals, separators, and suffixes; within the smallest production function/method/closure/const/static initializer it rejects both members of a nonsquare pair, or two occurrences of a square member, including one-hop integer `const`/`static` references. TypeScript applies the same rule to the smallest function/arrow/method/property or module-variable initializer with one-hop `const` resolution. This covers split comparisons, tuples, arrays, struct/object literals or patterns, match guards/arms, and nested expressions while allowing a single common number. Rust/TS fixtures cover the approved `790x1023` split/reversed/tuple/array/struct-object/nested/numeric-spelling/one-hop forms plus passing isolated-member, unrelated-scope, nonpair, and common-scalar cases.

Corpus matching is explicit per category. Rust/TS string literals matching normalized path/basename, case-folded hash, crop name, entry/target ID, or NodeId fail in production. Bbox/ROI tuples fail when one executable scope contains all normalized components with derived multiplicity, including one-hop integer constants; partial or scope-separated tuples pass. Within HanOnly/source-gate branches, count expressions rooted in `node`, `target`, `source_lines`, `ocr_lines`, `translated_lines`, or `eligible_lines` fail when compared/matched to derived node/line counts by literal or one-hop constant. Direct equality/inequality or tuple matching between target/translated/eligible counts and source/OCR counts also fails; unrelated retry/pixel counts pass.

Each exact category `path`, `hash`, `crop_name`, `node_id`, `bbox`, `node_count`, and `line_count` has Rust/TS table fixtures for a production hit, a category-appropriate pass, and the same hit in every allowed test/evidence exclusion. Line fixtures cover `eligible_lines.len() == N`, match, one-hop const, TS `.length`, and direct target/OCR equality. Node fixtures use node/target roots; bbox fixtures preserve repeated-value multiplicity. The runtime validator writes canonical sorted deduplicated sets from raw manifest/runtime/fixture evidence; the audit independently recomputes all seven sets and requires element-by-element equality. Every set is nonempty. `production-policy-audit.json` must contain exactly seven category objects with `derived_count > 0`, `derived_values_sha256` over canonical JSON, `rules_executed: true`, sorted `production_hits`, sorted `excluded_hits`, and complete `fixture_case_ids`. Per-category negative fixtures remove its raw source, force an empty set, and inject a one-element set mismatch; missing/unknown/empty/mismatched categories, unexecuted rules, missing fixture IDs, or production hits fail.

Scanner tests also contain the exact current Rust C2, nested-match offset, and UI estimator snippets; whitespace/fully-qualified/nested-expression and renamed one-hop variants; Cargo bin discovery; generated include fixtures; passing zero-offset/C2-like/non-HanOnly/manual/AllText/Auto-empty variants; and format-allowlist parity fixtures. No scalar `1` or arbitrary negative-number scan is allowed. The script writes `production-policy-audit.json` with Cargo target/source/include/generated inventory, Cargo-message/out-dir evidence, scanned/excluded paths, canonical closure schema/common/generator-lock/trusted-target hashes, the exact seven-category corpus report, dimension pairs, image-input-contract hash, source-color-contract hash, color-constant-set hash, automatic-color/width contract hits, runtime-matrix aggregation hash, allowlist hits, and violations. Any production violation, unresolved path, unclassified target/exclusion, missing category/fixture proof, closure or contract drift, forbidden automatic fallback, UI automatic estimator, missing/failed 360-cell aggregation, or allowlist hit outside D0 fails. Generated API checks remain separate. The planned UI edit is limited to retiring HanOnly automatic pixel estimation while preserving Auto/empty state, manual sizing, AllText, and format allowlist parity; generated schemas change only if the optional marker documentation path is exercised.

The automatic-style scanner fixtures include the current predicted-stroke discard, predicted/default stroke-width authority, `contrasting_stroke_color`, default-black fallback, post-inpaint unsupported-color classification, rotation entering color-mode completeness, mutable color/width threshold, shadow/glow rendering, and default/release evidence-feature snippets, plus passing source-contract-only and unrelated-color cases. Every forbidden production form must fail in both direct and one-hop helper syntax while its exact test/evidence-only counterpart remains an allowed exclusion.

## Acceptance gate

Revision 46 passes only when every emitted automatic-target record proves `preflight_consumed<=preflight_reserved`, `evaluation_consumed<=evaluation_reserved`, `target_consumed<=target_reserved<=target_limit`, and `page_consumed<=page_reserved<=page_limit`; exact-bound and bound-plus-one cases match the checked P-1/P0/P1/E requested/reserved calculator; and all six phase-specific P0 terminal tuples plus every page evaluation/preflight/total checked sum match independent recomputation. `J=page.nodes.len()` is the only pre-admission read. If `J` exceeds the page limit, requested/reserved/consumed is `J/0/0`, phase is `node_enumeration`, and no node or target traversal/record/warning/mutation occurs. After reserving `J`, exactly one page-node traversal derives `T`; target-terminalization overflow or limit reports `null/J/J` or `J+T/J/J` with no target record. Define total `Wpair=choose2_checked(T)`, returning `0` for `T<2`, dividing the even operand before multiplication, succeeding through `T=6_074_001_000`, and overflowing at `6_074_001_001`. After reserving and consuming `Wterm=J+T`, canonical-ranking pair-overflow, final-addition overflow, or page-limit reports `null/Wterm/Wterm`, `null/Wterm/Wterm`, or `checked(Wterm+Wpair)/Wterm/Wterm` respectively, with exactly `T` target records and zero pair comparisons. Admission reports requested/reserved/consumed `Wmeta=checked(Wterm+Wpair)` and executes exactly `Wpair` unordered comparisons. P1 requested/reserved equals its conservative bound while consumed equals an independent real-operation count, may be lower, and contains no padding/no-op/synthetic inflation; P1 performs no E-owned membership/P95/order/witness/candidate work; every E loop matches its exact charge; and every record carries identical final page totals, including under reversed input order. Public `StartPipelineRequest.region`, `PipelineRunOptions.region`, serialization, and `options_from_request` field/conversion shapes must remain unchanged; route comments, generated OpenAPI/Orval response behavior, and `docs/{en-US,ja-JP,zh-CN,pt-BR}/reference/http-api.md` must truthfully document the stable 400 and direct repair route. HTTP `start_pipeline` must reject `req.region.is_some()` before session/options/spec/job/cancel/event/task effects, and direct `pipeline::run` must reject `spec.options.region.is_some()` as its first executable validation before registry/order/page/Scene/run-state/engine/warning/blob/history effects; each path returns one deterministic error and produces no duplicate lifecycle, completion, or mutation evidence. Every zero/one-producer permutation must preserve optional behavior and conditional order, while ambiguous multiple ordering-critical producers fail before engine load.

For accepted normal HanOnly `pipeline::run`, exactly `spec.options.region.is_none()`, one shared pure backend admission must pass before the per-page matrix. HTTP region validation is followed by that admission before request-step Registry preflight: unsupported RPC records zero Registry calls, emits no warning, still creates exactly one job, and lets direct `pipeline::run` produce the sole stable unsupported-backend warning, `warning_count=1`, and `CompletedWithErrors`; admitted unknown IDs retain synchronous HTTP 400 before job creation. Direct `pipeline::run` independently repeats admission before `infos_for_spec`, so non-RPC callers cannot bypass it. Explicit CPU accepts on every target; requested Metal accepts only on macOS and final T6 proves actual Metal; every other non-CPU HanOnly target has zero page/run-state/model/engine/Scene/blob/History/erase/render effects and no normal cell. AllText remains admitted for both CPU flags with unchanged scheduling/output. Static engine descriptors and AllText order remain unchanged; private conditional edges exist only for HanOnly. Zero selected inpainters means zero builder/reservation/raster/publication and no frozen object. One selected inpainter means `prepare_and_freeze_hanonly_render_inputs(...)` proves final translation/layout/cluster/control/glyph/fill/nonempty-alpha plus nonzero alpha contribution for every ink-bearing shaped cluster/run, performs complete peak-live memory admission, and publishes exactly one immutable object containing sprite bytes, normalized frozen `sprite_transform`, and `rendered_direction` under the current `PageId`. Immediately after upstream producer commits and before that builder call, capture `B_prepare` from Scene bytes/epoch, History epoch/canonical-log bytes/undo/redo, canonical `reachable_blob_state`, and sprite inventory. Renderer without one inpainter rejects before engine load. A one-inpainter row without Renderer commits through the same ProjectSession-gated atomic engine path. A Renderer+Inpainter row first acquires History then Scene for one coherent pre-permit staging snapshot, captures authoritative `expected_epoch` and all read-only inputs defining `B_prepare`, releases Scene then History, and finishes every `.await`, inpainter, Renderer, staging, and cancellation operation before persistence admission. It then acquires the one session permit, rechecks poison only, promotes every sprite and final Rendered CAS object through permit-aware Blob-local `durable_put_exact` without History/Scene locks, releases Blob-local locks, acquires History exactly once within the permit-held publication boundary, performs exactly one `current_epoch == expected_epoch` comparison, acquires Scene and revalidates required state, then commits one GREEN-B History Batch. It releases Scene then History, synchronously publishes response/Dirty/job success, and only then releases the permit. A stale epoch discovered after Blob promotion leaves only exact verified `unreachable_cas`, poison false, and zero Scene/History/success publication. Blob/CAS failure before History replacement leaves reachable state equal to `B_prepare`, may add only verified `unreachable_cas`, leaves poison false, and publishes no success. History `Committed` requires exact new canonical bytes plus every platform-required sync/reopen and no temp before in-memory/success publication; `Unchanged` requires exact raw-old bytes plus a synced writer and no temp and leaves observable state equal to `B_prepare` with no success; History `Indeterminate` alone stores `ProjectSession.persistence_poisoned` under the still-held permit and returns exactly `project persistence is indeterminate; close and reopen the project` before release. Poison does not remove the session: read-only diagnosis/export remains gate-free, every apply/apply-if-epoch/undo/redo/snapshot/compact/autosave/page-import/mask-repair/public-or-direct-Blob/pipeline path rejects under the gate with the same fatal reason, pipeline becomes `Failed`, autosave logs once and exits, and no response/Dirty/autosave/job success or canonical write follows poison. Explicit close removes the session, joins autosave, skips `FlushNow`/final compact, releases the lock, and succeeds; only a later strict-recovery open creates an unpoisoned session. Deterministic barrier tests prove poison-before-gate and permitted-publication-before-poison orders. No staging guard may cross permit acquisition. No permit-held pre-Blob History acquisition or epoch read/validation, missing/additional permit-held post-Blob History acquisition, missing/additional `current_epoch == expected_epoch` comparison, atomic epoch mirror, additional gate, additional permit-held History phase, check-then-act interleaving, sleep/time oracle, `.await` under permit, gate acquisition under an inner lock, post-release success, permit/store identity mismatch, public write-capable ProjectSession field, missing member of the exact 13-file caller closure, same-module `ProjectSession::open_untrusted` bypass, or ungated former direct caller is accepted. Restart reads one immutable canonical generation, first uses strict replay on a disposable fresh clone, and only for a malformed/truncated legacy trailing-frame failure discards that clone and applies compatible replay to a second fresh clone. It must produce exactly raw-old/precommit or strict-new/staged state with no partial/mixed bytes, probe-state reuse, or double apply. Renderer is the sole writer of sprite placement/direction and never changes `Node.transform` or source/OCR geometry. The peak-live reservation includes every simultaneously live retained sprite, supersampled Pixmap, unavoidable copy, downsample output, mask, staged Scene/blob, and transaction buffer; admitted supported inputs cannot rely on allocator OOM. Private typed causes map to unchanged public `geometry_or_font`. Every terminal path releases payload/scratch and no page state leaks to the next page or run. The explicit nonempty repair route in `pages.rs` under default HanOnly is the sole region-bearing engine ingress only when its pre-side-effect guard proves URL role `MaskRole::Segment`, successful `Registry::find(params.pipeline)`, and exact descriptor output `[Artifact::Inpainted]`; rejected classes return stable HTTP 400 before side effects. Existing public request, Scene data-model, Engine, `EngineCtx`, Renderer, and BlobStore put signatures remain compatible; ProjectSession field removal is the one explicit downstream-breaking Rust helper/API change, and `backend_admission` is the one explicit additive downstream-public helper.

Every successful target proves before inpaint that the complete expected translation reached the shared raster path: expected/frozen/pre-inpaint-raster-entry UTF-8 digests and byte/scalar counts match directly or reconstruct exactly after removing only audited inserted newlines; canonical cluster intervals plus legal control records cover every input byte without gap/conflict/out-of-range; glyph ID `0` for any non-control cluster rejects as missing `.notdef`; every remaining nonzero glyph fits `u16`; the successful fill pass visits every shaped glyph exactly once; `missing_glyph_count=0`; and alpha is nonempty. Exact final decoded-RGBA replay and per-target omission inequality remain separate post-composite mandatory proofs. `R/A` expose only unique 32-lowercase-hex correlation-ID-keyed non-length verdicts; `C/G`, corpus, annotation, and closure summaries expose no target proof fields; every exporter rejects raw source/translated text, stable NodeId, source/OCR/translated line counts, text-derived digest/length/count, guarded report path, or mapping, and `line_count` corpus evidence comes only from checker-owned synthetic syntax-coupling sentinels. Generated high-core/high-candidate/large-`U`/overflow/page-aggregate cases fail closed with `ClassifierWorkBudgetExceeded` before erase, no `U*M` width enumeration occurs, strict integer/P95/pair semantics and both closed-preimage digests match independent implementations, each staged RED uses one uniquely listed full name with `--exact`, repeat runs produce identical non-time diagnostics/reasons/pixels, and no wall-clock threshold affects correctness. All Revision 39 through 45 geometry, source-style fidelity, width, composite, UI Auto, dependency, B0, sixteen-ID, C-first-generation, closure, governance, and dual-SHA requirements remain mandatory.

The remediation passes only when the dependency inventory proves exactly two app dev edges plus one llm build-only `sha2` edge, exactly three lock-list additions, and no new package/version/source/checksum or normal/root-workspace edge; pinned Rust/tag/archive inputs and same-target generated output reproduce twice; featureless/default/release builds omit `hanonly-test-evidence`; generated Rust is fully bound and scanned; and all deterministic tests pass. D0 must descriptor-walk and same-object validate the owned mode-correct external evidence base, immutable input/manifest/fixed-fixture bytes, and every output identity, while fault-injection retries converge with zero leaked output. The original dirty worktree must match its before snapshot exactly, and implementation, acceptance, and PR linked worktrees must satisfy their clean checkpoints.

All nine independent full-page entries must pass the frozen Source Gate selection, CPU/actual-Metal `9 x 2 x 2 x 10 = 360` matrix, coverage, no-retuning, input-budget, source-removal, layout/locality, Planner, strict color/width, unsupported-preservation, and exact-composite oracles. Protected support must be hashed before runtime outputs from the independent Source Scene classification; protected pixels remain identical. Successful modes require complete erase, passing pre-composite residuals, expected/frozen/pre-inpaint-raster-entry translation identity or exact audited-newline reconstruction, complete cluster/control byte coverage, glyph-zero `.notdef` rejection, only nonzero `u16`-representable glyphs, zero missing glyphs, complete pre-inpaint fill-pass traversal, nonempty alpha, checked complete-page retained logical-sprite reservation, independently checked actual 2-4x raster dimensions/bytes/construction, at-most-one live target scratch surface, per-target scratch release, exactly one frozen sprite per target, same sprite identity at Renderer entry, zero Renderer layout/raster, one persistence/composition, payload release, exact source-contract values where applicable, exact frozen pre-erase `F_t/W_t`, exact decoded-RGBA replay, and omission inequality. Unsupported rotation/color require Source/Clean/Inpainted/Rendered ROI equality, zero erase/block/sprite, one exact warning, and `CompletedWithErrors`. Every deterministic hard pre-write `geometry_or_font` negative (translation-identity mismatch, final-layout failure after local fallback, cluster/control-coverage failure, glyph-validity failure, fill-traversal failure, or nonempty-alpha validation failure) and every recoverable `geometry_or_font` negative (checked logical/actual-raster arithmetic or `usize` conversion failure, retained owner-rectangle/page-cap reservation failure, or fallible raster-surface-construction failure) must prove zero publication, inpainter run/apply, erase, or downstream persistence, exact final Scene/History/reachable-blob/sprite equality to `B_prepare`, and retention of upstream producer commits; process-level allocator OOM/abort is outside this claim. The policy audit must report zero hits and seven exact nonempty categories; the first six use the established non-text identity values and `line_count` uses only checker-owned synthetic syntax-coupling sentinels. No production behavior may encode fixture identity, absolute C2/`-5px`/fixed automatic caps/page-size OCR branches, line-count coupling, guessed color/width, shadow/glow rendering, or rotation success. Clean pre-B1 `B0_SHA/B0_WORKTREE` must prove the two B0/G004-owned IDs are unignored and pass, the remaining five T2 plus nine T3 staged-RED tests fail individually by uniquely discovered full names and `--exact`, and the default workspace suite passes, then produce the one immutable Revision 50 artifact/digest before B1. Final `IMPL_SHA` must prove all eight frozen interpreter endpoint blobs identical, both stage markers absent from tracked Rust, all sixteen IDs unignored and present exactly once, and the complete default workspace suite green before validating the frozen recall contract and accepted source-coordinate boxes/anchors without reselection; it contains every local governance/closure input before T6. `R/A/C` must agree on the Revision 50 schema plus inherited Revision 46 contract and trusted schema/common/generator-lock/target fields; `R/A` may carry only correlation-ID-keyed non-length text-completeness verdicts, while `C/G`, corpus, annotation, and closure summaries carry no target proof fields. The shared forbidden-field/value scan must reject raw text, stable NodeId, runtime/manifest line count, text-derived digest/length/count, guarded path, or mapping from every export. PR CI must match common/generator locks and reproduce its own target twice; `G` must bind both target records, `candidate_head_sha`, and `check_sha`. Human review may reject only successful-mode inpaint texture and cannot waive any machine verdict.

Completion additionally requires the acyclic Revision 50 chain `B0 authorization; final closure -> R -> A -> C -> PR notice annotation -> G`; the B0 artifact binds clean pre-B1 `B0_SHA`, passes holdout, remains byte-identical, and matches the frozen recall contract plus accepted source-coordinate boxes/anchors; the eight-path byte gate has no missing/non-blob/wrong-mode/OID/diff error and final has no staged-RED marker or ignored normative ID; `C` matches the seven accepted audit sets without raw data, commit SHA, or PR target record; only excluded `C`, external evidence, remote Ruleset state, and external `G` change after T6; `local clean PR HEAD == authenticated PR head.sha == annotation.candidate_head_sha`; `authenticated PR merge_commit_sha == queried check_run.head_sha == annotation.check_sha`; the real `windows-2022` History prerequisite succeeds at that candidate; exact `needs + always() + success` propagation converts every prerequisite non-success into a failing required context; and the sole required `HanOnly Production Policy` context is enforced by one active empty-bypass `main` Ruleset with one approval, code-owner review, stale dismissal, strict status, and force-push/deletion protection. All five governance paths retain the exact owner line, at least two owners are authenticated write-capable, and at least one eligible owner differs from the PR author. A local-only pass, missing/failing/bypassable Windows prerequisite, staged-RED inventory mismatch, frozen-interpreter drift, default workspace failure, B0 reselection, closure/target drift, post-T6 tracked non-`C` mutation, missing/duplicate/malformed annotation, stale or collapsed PR identity, pending/neutral/skipped/stale check, author-only owner set, bypassable Ruleset, missing `G`, changed original worktree, or missing remote authority/evidence is not acceptance.
