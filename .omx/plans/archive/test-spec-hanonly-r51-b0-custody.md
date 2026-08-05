# Revision 51 G004 B0 executable custody and test matrix

## Scope

Revision 51 is the sole operative G004 revision. The authoritative files are the main plan, `.omx/plans/hanonly-r51-b0-custody-contract.json`, this test specification, the current UltraGoal brief, and the G004/G005 rows in `goals.json`. Every Revision 50 custody, historical-registry, native-agent identity/receipt, corpus ID, artifact revision, authorization, resume, and completion directive is historical and non-normative. Only the five production detector-support/diagnostic schema keys explicitly inherited by the R51 machine contract remain normative from R50.

Revision 51 does not claim that a local JSON file proves who prepared a sample. It proves operational blindness through encryption, pre-edit freezing, withheld keys, immutable ciphertext commitments, one durable formal open, a supervised one-shot execution, and a terminal receipt. Production scope remains the shared PP-OCR observation and Source Gate detector-support boundary.

## Gate order

1. Validate the Revision 49 B0 base SHA `31f58b85cfa723c6e9bb0910e5402059d9db7184`, R49 immutable evidence, G004 `in_progress`, and G005 `pending`.
2. Architect reviews the exact R51 contract, operative plan, and this test specification.
3. Critic reviews only after Architect approval. Implementation remains disabled until both approve.
4. Before holdout preparation, the frozen R51 planning-only inventory endpoint generates the closed public historical inventory from the contract's exact root registry and pinned Python/sips executables.
5. A persistent custody operator, with no implementation or calibration context, prepares four never-used entries and freezes the encrypted holdout before any production edit or R51 worktree exists.
6. The implementation thread descriptor-validates only the historical inventory, ciphertext, closed public header, and freeze receipt. It never receives plaintext paths, archive listing, oracle, mask, source, clean reference, or keys.
7. Create an independent R51 branch/worktree from the R49 B0 SHA and implement the production-generic root fix plus durable diagnostics.
8. Run directed regressions, both B0-owned tests, default workspace tests, `bun cargo check --workspace --all-targets`, generated/format/policy/anti-fixture/marker inventory, and prove five T2 plus nine T3 tests remain staged RED.
9. Commit a clean scoped B0 with `Co-Authored-By: Codex <codex@openai.com>` and `Codex-Thread-ID: 019fa26c-3b11-78d1-addd-b631590180e4`.
10. Publish the closed R51 B0 preflight attestation, including the exact descriptor-validated `koharu-app` evidence test executable path/SHA and sole `hanonly-test-evidence` feature set. From a detached clean B0 worktree, run D0 manifest preflight, pre-calibration anti-fixture, all `4 x 2 x 4` calibration cells, choose the first all-pass candidate, and freeze the recall contract.
11. Run pre-holdout anti-fixture. The implementation thread then waits only on the native completion handle while the custody operator durably publishes the open marker before key use, performs the one formal open, and runs the selected candidate over `4 x 2` holdout cells.
12. Stop on the first failed cell. Any cell, cleanup, handle, deadline, or invalid-terminal failure durably publishes the irreversible failure marker. For handle, deadline, or invalid-terminal failure, the supervisor publishes that marker before it closes or attempts to close the custody operator.
13. Validate the terminal receipt, failure-marker absence for a passing run, diagnostic generation chain, canonical JSON, permissions, device/model evidence, logs, frozen payload, and independent SHA-256 values.
14. Invoke the exact `--validate-r51-b0-authorization` endpoint. It first publishes an authorization record binding the artifact payload hash, then adds only that record's SHA to derive and publish the final artifact, avoiding a hash cycle. Freeze the artifact and complete G004 only when the endpoint returns the valid final artifact SHA. Never start G005 automatically.

## Custody checks

- Before freeze, publish and independently recompute `historical-inventory.json`. The endpoint first fail-closes on helper/Python/sips path, version, and SHA drift; its exact root set, discovery full-match rule, payload-hash projection, and every supported image's raw SHA-256 plus pinned-sips, stdlib-decompressed normalized scanline identity are closed by the machine contract.
- Reject any candidate source whose raw SHA-256 or normalized scanline SHA-256 appears in that inventory. The freeze receipt exposes only four opaque IDs with raw/normalized identities, allowing the implementation validator to recompute exclusion without opening holdout source bytes.
- Before encryption, validate the exact closed manifest/oracle/hashes schemas, relative asset mapping, canonical JSON, mutual hashes, asset completeness, decoded identities, binary/equal/nonempty masks, geometry, exact mask-to-Source/Clean delta equality, protected equality, hidden source-text script/count/hash, and decision invariants. Any mismatch rejects; the three exact plaintext file hashes and canonical archive hash are committed in the public freeze receipt.
- Freeze occurs before production edits. The freeze receipt binds the base B0 SHA, implementation thread ID, historical inventory, ciphertext/header commitments, the exact HMAC-SHA256 tag, four opaque entry IDs, and exclusion result.
- `holdout-header.json` has the exact seven-field schema in the machine contract and cannot contain paths, names, IDs, dimensions, image/oracle/mask facts, individual plaintext hashes, or free-form data.
- The archive is canonical POSIX ustar containing only normalized regular files/directories. Reject links, devices, duplicate/prefix-colliding paths, pax/sparse extensions, path escape, or unknown entries before descriptor-relative extraction.
- The implementation thread may read only `historical-inventory.json`, `holdout-header.json`, `holdout.enc`, and `holdout-freeze-receipt.json`; it may hash but not decrypt or list the plaintext archive.
- Encryption and MAC keys stay only in the persistent custody context. Key disclosure or loss fails Revision 51.
- Freeze files publish in the exact historical-inventory, ciphertext, header, receipt order. Inventory publication retains a no-follow custody-directory descriptor and uses a descriptor-relative create-new hard link, so it cannot replace an existing final. Only the closed partial/final states and one next-step deterministic temp are legal; every collision or out-of-order state rejects.
- `holdout-open.json` is create-new and irreversible. At formal-open entry, any existing final or temp rejects. Before any key access the same uninterrupted invocation must complete temp write/fsync, descriptor-relative create-new hard-link publication without replacement, exact reopen verification, final fsync, temp removal, directory fsync, and fresh identity validation; only its internal non-serializable post-publication flag permits that invocation to continue. No later or resumed invocation can restore the flag, so the formal holdout can never be rerun.
- The main thread waits on two sequential one-hour completion-handle windows and does not inspect custody storage while the handle runs. Completed returns only terminal receipt path/SHA. Failed, closed, two timeouts, or invalid completed output first trigger a descriptor-relative create-new irreversible failure marker with `operator_close_result: close_pending`, `open_marker_observation: not_inspected`, and null open SHA. Only after that marker is durable may one close-agent attempt run; its result is ledger-only and cannot mutate the marker. Public open/terminal finals are inspected only afterward and never authorize a rerun.
- The custody operator creates all per-cell diagnostics before terminal classification. Each record includes `SelectionResult`, target recall, PP/VL counts, rejection reason, device evidence, log path/hash, and detector-support evidence.
- Before model load, the B0-preflight-frozen evidence executable emits a privacy-safe bundle-validation receipt binding its executable SHA/features and every freeze commitment. For each executed cell it then emits exactly one proof per hidden oracle target that hashes the oracle mask and actual selected/downstream page-raster supports and proves zero missing and protected-overlap pixels. Each proof registers support path, byte length, dimensions, stride, raw SHA, binary encoding, and foreground count; the terminal diagnostic index binds every receipt, proof, coverage index, and support-raster payload.
- For each sorted target, selected then downstream support rasters publish before the proof JSON. Every raster uses a deterministic hash-named temp and descriptor-relative no-follow hard-link create-new final, with complete-write/file-fsync, exact bytes/mode/owner/same-inode reopen, final and parent fsync, temp unlink, and parent fsync. Any collision, drift, partial write, or out-of-order state fails the cell; no raster final is replaceable.
- The first holdout-cell generation atomically binds inherited `holdout_manifest_sha256` to the exact `manifest_sha256` in the public bundle-validation receipt, binds that receipt's path/SHA/length, and adds the first captured cell. No manifest-only or receipt-only generation is legal, and authorization recomputes the equality from the receipt.
- A crash after the open marker, missing or invalid terminal receipt, any failed cell, plaintext cleanup failure, handle failure/closure/deadline, or receipt/hash drift is a formal gate failure. The irreversible failure marker dominates every late receipt and makes authorization reject.
- A passing terminal receipt contains exactly eight sorted passing cell records and binds the bundle-validation and per-cell coverage-index hashes. A cell failure stops immediately, persists only the executed prefix through the first failed diagnostic, lists the exact unexecuted suffix, and requires the prior failure marker.
- Historical inventory discovery, recursive snapshots, and every item read use descriptor-relative per-component `O_NOFOLLOW`; opened root/directory/file device, inode, kind, size, and mtime must match the first snapshot, and file metadata must remain identical before and after reading.
- B0 preflight, authorization record, and artifact finals use descriptor-relative no-follow hard-link create-new publication with exact same-inode verification; no final can be replaced.

## Production regressions

The minimum deterministic tests must cover:

1. PP recognizes only neutral digits while VL confirms Han and detector support fully owns the target.
2. PP/VL line or character counts differ, but every promoted Han target has complete detector-to-Scene-to-downstream raster support.
3. PP recognition order changes, but geometric ownership and full support remain unchanged.
4. Pure Latin product identifiers remain protected and rejected.
5. Missing detector support, ambiguous ownership, protected overlap, unsupported rotation, or incomplete downstream raster coverage remains rejected.
6. PP detector observations retain duplicate occurrences and `None` recognition results without changing the existing `word_boxes` projection.

No test or production path may branch on image name, entry ID, hash, dimensions, coordinates, NodeId, corpus role, specific text, or R49/R51 markers.

## Formal failure

Any failed Architect/Critic review, pre-B0 validation, calibration cell, Metal proof, pre-holdout attestation, formal holdout cell, terminal receipt, final authorization, or artifact hash check stops Revision 51. Do not freeze the artifact, do not modify the selected candidate, do not reopen the holdout, and do not enter G005. Save the complete diagnostic chain and append the failure to the G004 ledger.
