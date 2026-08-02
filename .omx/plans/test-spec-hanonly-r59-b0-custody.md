# Revision 59 B0 custody test specification

## Scope

Revision 59 replaces only the unavailable Revision 51 formal holdout custody path. It does not change Source Gate, PP-OCR, crop selection, model parameters, calibration results, rendering, layout, or G005 behavior. The existing R59 ciphertext and private-manifest commitment remain immutable.

## Required bindings

1. Validate the original public commitment at its fixed path and SHA-256. All public receipts use the fixed `/Users/Shared/hanonly-r59-public` directory and the revision-specific filenames frozen by the contract; caller-selected custody paths are forbidden.
2. Require a custody-produced successor commitment at the fixed path after the reviewed B0 commit exists. It must bind the original commitment SHA, unchanged ciphertext/private-manifest commitments, exact contract/spec SHA, successor B0 SHA, selected candidate `S25L4`, calibration artifact SHA, runtime bundle commitments, and `start_marker_absent=true`.
3. Before restricted access, require `successor_b0_sha == requested_b0_sha == detached clean HEAD`.
4. Reject R51 IDs, R51 contracts, arbitrary revision aliases, commitment drift, or a successor that changes the corpus.

## Runtime bundle

The custody runner may transform its private package only after the start marker into the fixed ephemeral `hanonly-r59-runtime-bundle-v1` shape. The bundle reuses the existing validated manifest/oracle/hashes and asset structure with exact R59 contract names, revision 59, and IDs `r59-h01` through `r59-h04`. The B0 harness independently verifies canonical JSON, mutual hashes, source/clean/mask bindings, protected Latin semantics, geometry, and every runtime bundle commitment from the successor receipt.

No oracle, mask, source image, private manifest, key, archive listing, or plaintext path is exposed to the implementation lane. Oracle and masks never enter production decisions.

## One-shot order

1. Revalidate public commitment, successor commitment, contract/spec, B0, calibration artifact, selected candidate, permissions, and absence of the start marker.
2. Atomically create the fixed start marker using create-new semantics.
3. Only after marker publication may the custody runner access its age identity, decrypt, create the fixed mode-0700 plaintext root, transform/extract the runtime bundle, or execute a formal cell.
4. Run exactly four opaque entries on CPU and actual Metal with frozen candidate `S25L4`.
5. Stop at the first failed cell. Save the executed prefix and complete diagnostics.
6. Exit related processes, close descriptors, remove the fixed plaintext root, and prove the same root is absent. The fixed terminal receipt binds the actual bundle-validation receipt SHA-256 and artifact-payload SHA-256; authorization never accepts those hashes as caller assertions.
7. Any failure, crash, drift, missing evidence, unknown state, or unproved cleanup is permanently non-authorizing. The holdout cannot be retried after marker creation.

## Deterministic tests

- Exact original commitment and successor schema acceptance.
- Successor B0 mismatch rejects before start.
- Successor ciphertext or private-manifest drift rejects before start.
- Exact R59 IDs pass; R51 or arbitrary IDs reject.
- Runtime manifest/oracle/hashes mutual commitment mismatch rejects.
- Start marker already present rejects without restricted access.
- Any restricted action before marker produces permanent non-authorization.
- CPU or Metal failure produces `completed_fail`.
- Crash, missing terminal evidence, or unknown state produces `incomplete_non_authorizing`.
- Plaintext outside the fixed root, open descriptors, live runner process, or root residue refuses authorization.
- Exactly eight passing cells plus cleanup are required for `completed_pass`.

## Stop conditions

Do not run formal holdout until Architect and Critic approve these exact bytes, the implementation and tests pass, a new clean B0 commit exists, and custody publishes the exact successor commitment. Never relabel R59 as R51. Never enter G005 automatically.
