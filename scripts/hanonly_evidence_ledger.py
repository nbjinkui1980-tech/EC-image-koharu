#!/usr/bin/env python3

import argparse
import contextlib
import datetime
import errno
import hashlib
import json
import os
import re
import stat
import struct
import subprocess
import sys
from dataclasses import dataclass


LEDGER_NAME = "evidence-ledger.json"
LEDGER_VERSION = 1
EXPECTED_DIMENSIONS = (790, 1023)
FIXTURE_RELATIVE_PATH = (
    "crates/koharu-app/tests/fixtures/"
    "source-gate-deterministic-recall/fixture-manifest.json"
)
LEDGER_KEYS = {
    "version",
    "visual_input",
    "visual_input_sha256",
    "visual_manifest",
    "visual_manifest_sha256",
    "source_gate_fixture_manifest_sha256",
    "evidence_root",
}
RUN_ID_RE = re.compile(r"\A\d{8}T\d{6}Z-[0-9a-f]{12}-[1-9]\d*\Z")
SHA256_RE = re.compile(r"\A[0-9a-f]{64}\Z")
B0_SHA_RE = re.compile(r"\A[0-9a-f]{40}\Z")
B0_UTC_SECONDS_RE = re.compile(r"\A\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z\Z")
B0_VERSION = 2
B0_PLAN_REVISION = 49
B0_DEFAULT_GPU_LAYERS = 1000
B0_REQUIRED_CHECK_COMMAND = (
    "bun scripts/check-hanonly-production-policy.ts --b0-source-gate-anti-fixture"
)
B0_CHECKER_ENDPOINT = "scripts/check-hanonly-production-policy.ts"
B0_ANTI_FIXTURE_SCANNED_ROOTS = [
    "crates/koharu-app/src/pipeline/engines/source_language_gate.rs",
    "crates/koharu-app/src/pipeline/engines/support.rs",
    "crates/koharu-ml/src/pp_ocr_v5.rs",
    "crates/koharu-llm/src/paddleocr_vl.rs",
    "crates/koharu-app/src/pipeline/mod.rs",
    "scripts/check-hanonly-production-policy.ts",
    "scripts/check-hanonly-production-policy.test.ts",
    "scripts/hanonly_evidence_ledger.py",
    "scripts/hanonly_evidence_ledger_test.py",
]
B0_ANTI_FIXTURE_ALLOWED_DESCRIPTOR_ROOTS = [
    "crates/koharu-app/src/pipeline/mod.rs",
    "scripts/check-hanonly-production-policy.ts",
    "scripts/check-hanonly-production-policy.test.ts",
    "scripts/hanonly_evidence_ledger.py",
    "scripts/hanonly_evidence_ledger_test.py",
]
B0_RECALL_PREIMAGES = {
    "ppocr_crop_local_preprocessing_sha256": '{"contract":"hanonly-b0-ppocr-crop-local-preprocessing-v1","operations":["decode-crop-rgba","isotropic-upscale-if-short-side-below-64","detect-and-recognize-in-upscaled-crop-space"]}',
    "inverse_mapping_rule_sha256": '{"contract":"hanonly-b0-inverse-mapping-v1","operations":["divide-upscaled-word-box-coordinates-by-inference-scale","preserve-half-open-crop-local-geometry","translate-by-source-crop-origin"]}',
    "coverage_acceptance_rule_sha256": '{"contract":"hanonly-b0-coverage-acceptance-v1","requirements":["pp-and-vl-han-scalar-counts-match","no-rejected-after-vl","no-pp-vl-incomplete-coverage","all-removal-target-rois-covered"]}',
    "source_removal_preflight_rule_sha256": '{"contract":"hanonly-b0-source-removal-preflight-v1","requirements":["target-recall-equals-one","protected-false-positive-count-equals-zero","rotation-targets-excluded","unmatched-selected-node-count-equals-zero","coverage-acceptance-passes"]}',
}
B0_CANDIDATES = [
    {
        "id": "S25L4",
        "short_side_numerator": 1,
        "short_side_denominator": 4,
        "long_side_numerator": 1,
        "long_side_denominator": 25,
    },
    {
        "id": "S25L5",
        "short_side_numerator": 1,
        "short_side_denominator": 4,
        "long_side_numerator": 1,
        "long_side_denominator": 20,
    },
    {
        "id": "S25L6",
        "short_side_numerator": 1,
        "short_side_denominator": 4,
        "long_side_numerator": 3,
        "long_side_denominator": 50,
    },
    {
        "id": "S25L7",
        "short_side_numerator": 1,
        "short_side_denominator": 4,
        "long_side_numerator": 7,
        "long_side_denominator": 100,
    },
]
B0_ROOT_KEYS = {
    "version",
    "plan_revision",
    "b0_sha",
    "manifest_sha256",
    "source_gate_fixture_manifest_sha256",
    "image_input_contract_sha256",
    "source_color_contract_sha256",
    "color_constant_set_sha256",
    "requested_devices",
    "enabled_cargo_features",
    "backend_evidence_parser_version",
    "required_checks",
    "frozen_recall_contract",
    "candidates",
    "calibration_entry_ids",
    "holdout_entry_ids",
    "process_evidence",
    "calibration_results",
    "selected_candidate_id",
    "frozen_at_utc",
    "frozen_payload_sha256",
    "holdout_results",
    "holdout_completed_at_utc",
    "retuned_after_freeze",
}
B0_PROCESS_KEYS = {
    "id",
    "phase",
    "requested_device",
    "paddle_instance_id",
    "executable_sha256",
    "model_artifact_sha256",
    "runtime_library_sha256",
    "load_evidence",
}
B0_MODEL_HASH_KEYS = {
    "pp_detection",
    "pp_recognition",
    "pp_recognition_config",
    "vl_model",
    "vl_mmproj",
}
B0_LOAD_KEYS = {
    "cpu_forced",
    "gpu_offload_supported",
    "n_gpu_layers",
    "mtmd_use_gpu",
    "word_boxes_backend",
    "raw_load_log_relpath",
    "raw_load_log_sha256",
    "enumerated_devices",
    "loaded_model_devices",
    "offloaded_layers",
    "offloadable_layers",
    "model_buffer_bytes_by_backend",
    "mtmd_backend",
}
B0_ENUMERATED_DEVICE_KEYS = {"index", "name", "description", "backend", "device_type"}
B0_LOADED_DEVICE_KEYS = {
    "model_device_ordinal",
    "name",
    "backend",
    "device_type",
}
B0_RESULT_KEYS = {
    "entry_id",
    "process_evidence_id",
    "candidate_id",
    "execution_evidence",
    "runtime_nodes",
    "derived",
}
B0_EXECUTION_KEYS = {
    "paddle_instance_id",
    "context_offload_kqv",
    "context_op_offload",
    "inference_completed",
    "raw_inference_log_relpath",
    "raw_inference_log_sha256",
    "source_gate_diagnostic_relpath",
    "source_gate_diagnostic_sha256",
    "context_buffer_bytes_by_backend",
    "compute_buffer_bytes_by_backend",
}
B0_RUNTIME_NODE_KEYS = {
    "node_id",
    "recognition_anchor",
    "node_rotation",
    "text_rotation",
    "selected_as_han",
}
B0_DERIVED_KEYS = {
    "actual_device",
    "matched_target_ids",
    "selected_target_ids",
    "selected_protected_node_ids",
    "selected_rotation_target_ids",
    "unmatched_selected_node_ids",
    "target_recall",
    "protected_false_positive_count",
    "rotation_targets_excluded",
    "source_coverage_preflight",
    "passed",
}
B0_SOURCE_COVERAGE_KEYS = {
    "pp_han_scalar_count",
    "vl_expected_han_scalar_count",
    "pp_vl_complete_coverage",
    "rejected_after_vl",
    "pp_vl_incomplete_coverage",
    "covered_source_roi_ids",
    "source_text_roi_coverage",
    "source_removal_preflight_passed",
}
B0_REQUIRED_CHECK_KEYS = {
    "phase",
    "command",
    "checker_endpoint_sha256",
    "manifest_sha256",
    "source_gate_fixture_manifest_sha256",
    "attestation_relpath",
    "attestation_sha256",
    "b0_sha",
    "result",
}
B0_ATTESTATION_KEYS = {
    "version",
    "mode",
    "phase",
    "b0_sha",
    "manifest_sha256",
    "source_gate_fixture_manifest_sha256",
    "checker_endpoint_sha256",
    "scanned_roots",
    "allowed_descriptor_roots",
    "policy_scan_sha256",
    "result",
}
B0_FROZEN_RECALL_KEYS = {
    "candidate_set",
    "selected_candidate_id",
    *B0_RECALL_PREIMAGES,
}
R51_PLAN_REVISION = 51
R51_FEATURES = ["hanonly-test-evidence"]
R51_CONTRACT_SHA256 = "1ffb19b169955e0d3bc7c8190777248fbec077acf75a7a28906216cd873a29f9"
R51_OPERATIVE_PLAN_SHA256 = (
    "ddc9709d5c0c762e2a4a6911f5db3889aea4dfce9927db7b1ff0ad1bca983fb6"
)
R51_TEST_SPEC_SHA256 = (
    "509960950db5cc8fb00a0de83ef0f22c74d55788557b8651e6418b919fd7bf35"
)
R51_BASE_CONTRACT_SHA256 = (
    "d29a18cd93d6d26516414009451117b131334f2a26439b0806d173e1e1c50afb"
)
R51_APPROVED_PUBLIC_FILES = {
    "r51_contract": ".omx/plans/hanonly-r51-b0-custody-contract.json",
    "operative_plan": (
        ".omx/plans/2026-07-23-hanonly-visual-rendering-remediation-plan.md"
    ),
    "r51_test_spec": ".omx/plans/test-spec-hanonly-r51-b0-custody.md",
    "base_production_contract": ".omx/plans/hanonly-r50-b0-evidence-contract.json",
}
R51_CUSTODY_FROZEN_NAMES = {
    "historical-inventory.json",
    "holdout.enc",
    "holdout-header.json",
    "holdout-freeze-receipt.json",
}
R51_CUSTODY_AUTHORIZED_NAMES = R51_CUSTODY_FROZEN_NAMES | {
    "holdout-open.json",
    "holdout-terminal.json",
}
R51_HEADER_KEYS = {
    "contract",
    "plan_revision",
    "cipher",
    "integrity",
    "iv_hex",
    "ciphertext_byte_length",
    "plaintext_archive_byte_length",
}
EXPECTED_B0_B1_MARKER_IDS = [
    "hanonly_pre_b1_red_t2_dynamic_layout_contract",
    "hanonly_pre_b1_red_t2_pipeline_layout_handoff_contract",
    "hanonly_pre_b1_red_t2_blob_decode_budget_contract",
    "hanonly_pre_b1_red_t2_replace_import_atomicity_contract",
    "hanonly_pre_b1_red_t2_rotation_status_contract",
]
EXPECTED_GREEN_C_RED_IDS = [
    "hanonly_pre_greenc_red_t3_transient_planner_hint_contract",
    "hanonly_pre_greenc_red_t3_run_state_lifetime_contract",
    "hanonly_pre_greenc_red_t3_planner_font_outcome_contract",
    "hanonly_pre_greenc_red_t3_source_color_contract",
    "hanonly_pre_greenc_red_t3_marker_batch_atomicity_contract",
    "hanonly_pre_greenc_red_t3_untrusted_marker_lifecycle_contract",
    "hanonly_pre_greenc_red_t3_http_marker_rejection_contract",
    "hanonly_pre_greenc_red_t3_mcp_marker_rejection_contract",
    "hanonly_pre_greenc_red_t3_source_color_probe_contract",
]
R51_CALIBRATION_IDS = [f"r51-c0{index}" for index in range(1, 5)]
R51_HOLDOUT_IDS = [f"r51-h0{index}" for index in range(1, 5)]
R51_PREFLIGHT_KEYS = {
    "contract",
    "plan_revision",
    "b0_sha",
    "implementation_thread_id",
    "r51_contract_sha256",
    "operative_plan_sha256",
    "r51_test_spec_sha256",
    "base_production_contract_sha256",
    "freeze_receipt_sha256",
    "historical_inventory_sha256",
    "ciphertext_sha256",
    "frozen_interpreter_sha256",
    "evidence_test_executable_path",
    "evidence_test_executable_sha256",
    "evidence_enabled_cargo_features",
    "gate_results",
    "staged_red_log_sha256",
    "result",
}
R51_GATE_KEYS = {
    "directed_source_gate_regressions",
    "directed_ppocr_regressions",
    "b0_owned_tests",
    "default_workspace_tests",
    "workspace_all_targets_check",
    "generated",
    "format",
    "policy",
    "anti_fixture",
    "r51_marker_inventory",
    "staged_red_t2",
    "staged_red_t3",
}
R51_FREEZE_KEYS = {
    "contract",
    "plan_revision",
    "base_b0_sha",
    "implementation_thread_id",
    "frozen_before_production_edit",
    "entry_ids",
    "cipher",
    "integrity",
    "iv_sha256",
    "ciphertext_byte_length",
    "ciphertext_sha256",
    "header_sha256",
    "hmac_sha256",
    "plaintext_archive_sha256_commitment",
    "manifest_sha256_commitment",
    "oracle_sha256_commitment",
    "hashes_sha256_commitment",
    "historical_inventory_sha256",
    "formal_source_identities",
    "disclosed_challenge_exclusion_pass",
    "result",
}
R51_OPEN_KEYS = {
    "contract",
    "plan_revision",
    "b0_sha",
    "selected_candidate_id",
    "freeze_receipt_sha256",
    "ciphertext_sha256",
    "pre_holdout_attestation_sha256",
    "nonce_hex",
    "result",
}
R51_BUNDLE_KEYS = {
    "contract",
    "plan_revision",
    "b0_sha",
    "test_executable_sha256",
    "enabled_cargo_features",
    "r51_contract_sha256",
    "freeze_receipt_sha256",
    "plaintext_archive_sha256",
    "manifest_sha256",
    "oracle_sha256",
    "hashes_sha256",
    "schema_validation_pass",
    "asset_binding_pass",
    "mask_source_clean_equality_pass",
    "oracle_semantics_pass",
    "result",
}
R51_TERMINAL_KEYS = {
    "contract",
    "plan_revision",
    "b0_sha",
    "selected_candidate_id",
    "freeze_receipt_sha256",
    "open_marker_sha256",
    "ciphertext_sha256",
    "pre_holdout_attestation_sha256",
    "bundle_validation_receipt_sha256",
    "terminal_diagnostic_index_sha256",
    "cell_results",
    "first_failed_cell",
    "unexecuted_cell_keys",
    "all_cells_terminated",
    "all_cells_passed",
    "plaintext_removed",
    "result",
}
R51_TERMINAL_CELL_KEYS = {
    "cell_key",
    "result",
    "selection_result",
    "target_recall",
    "pp_han_count",
    "vl_han_count",
    "rejection_reason",
    "device_evidence_sha256",
    "log_sha256",
    "diagnostic_sha256",
    "target_coverage_index_sha256",
}
R51_AUTHORIZATION_KEYS = {
    "contract",
    "plan_revision",
    "b0_sha",
    "r51_contract_sha256",
    "operative_plan_sha256",
    "r51_test_spec_sha256",
    "base_production_contract_sha256",
    "b0_preflight_attestation_sha256",
    "calibration_manifest_sha256",
    "calibration_ledger_sha256",
    "freeze_receipt_sha256",
    "historical_inventory_sha256",
    "ciphertext_sha256",
    "pre_calibration_attestation_sha256",
    "pre_holdout_attestation_sha256",
    "frozen_recall_contract_sha256",
    "selected_candidate_id",
    "open_marker_sha256",
    "bundle_validation_receipt_sha256",
    "terminal_receipt_sha256",
    "terminal_diagnostic_index_sha256",
    "failure_marker_absent",
    "artifact_payload_sha256",
    "result",
}
R51_ARTIFACT_PAYLOAD_KEYS = {
    "version",
    "plan_revision",
    "b0_sha",
    "selected_candidate_id",
    "frozen_recall_contract",
    "calibration_manifest_sha256",
    "freeze_receipt_sha256",
    "ciphertext_sha256",
    "required_checks",
    "calibration_results",
    "holdout_results",
    "bundle_validation_receipt_sha256",
    "terminal_diagnostic_index_sha256",
}
R51_DIAGNOSTIC_INDEX_KEYS = {
    "contract",
    "plan_revision",
    "b0_sha",
    "calibration_manifest_sha256",
    "holdout_manifest_sha256",
    "fixture_manifest_sha256",
    "generation",
    "previous_index_path",
    "previous_index_sha256",
    "previous_index_byte_length",
    "expected_cell_count",
    "records",
    "bundle_validation_receipt_path",
    "bundle_validation_receipt_sha256",
    "bundle_validation_receipt_byte_length",
}
R51_DIAGNOSTIC_RECORD_KEYS = {
    "cell_key",
    "phase",
    "candidate_id",
    "entry_id",
    "device",
    "state",
    "diagnostic_path",
    "diagnostic_sha256",
    "diagnostic_byte_length",
    "selection_result",
    "target_recall",
    "pp_han_count",
    "vl_han_count",
    "rejection_reason",
    "device_evidence_path",
    "device_evidence_sha256",
    "device_evidence_byte_length",
    "log_path",
    "log_sha256",
    "log_byte_length",
    "terminal_reason",
    "target_coverage_index_path",
    "target_coverage_index_sha256",
    "target_coverage_index_byte_length",
}
R51_TARGET_RECALL_KEYS = {"target_total", "selected", "covered", "uncovered"}
R51_COVERAGE_INDEX_KEYS = {
    "contract",
    "plan_revision",
    "b0_sha",
    "cell_key",
    "manifest_sha256",
    "oracle_sha256",
    "hashes_sha256",
    "records",
}
R51_COVERAGE_INDEX_RECORD_KEYS = {
    "entry_id",
    "target_id",
    "proof_path",
    "proof_sha256",
    "proof_byte_length",
}
R51_COVERAGE_PROOF_KEYS = {
    "contract",
    "plan_revision",
    "b0_sha",
    "cell_key",
    "entry_id",
    "target_id",
    "oracle_mask_raw_sha256",
    "oracle_mask_normalized_sha256",
    "page_width",
    "page_height",
    "support_stride_bytes",
    "selected_support_relpath",
    "selected_support_byte_length",
    "selected_support_sha256",
    "downstream_support_relpath",
    "downstream_support_byte_length",
    "downstream_support_sha256",
    "oracle_foreground_pixels",
    "selected_support_foreground_pixels",
    "downstream_support_foreground_pixels",
    "selected_covered_pixels",
    "downstream_covered_pixels",
    "missing_selected_pixels",
    "missing_downstream_pixels",
    "protected_overlap_pixels",
    "target_selected",
    "result",
}
R51_CELL_DIAGNOSTIC_KEYS = {
    "contract",
    "plan_revision",
    "b0_sha",
    "calibration_manifest_sha256",
    "holdout_manifest_sha256",
    "fixture_manifest_sha256",
    "phase",
    "entry_id",
    "device",
    "candidate_id",
    "state",
    "selection_result",
    "target_recall",
    "pp_han_count",
    "vl_han_count",
    "rejection_reason",
    "raw_detector_outputs",
    "canonical_lines",
    "raw_detector_count",
    "raw_detector_f32_bits_multiset_sha256",
    "detector_support_records",
    "device_evidence_sha256",
    "device_evidence_byte_length",
    "log_sha256",
    "log_byte_length",
    "terminal_reason",
    "bundle_validation_receipt_sha256",
    "target_coverage_index_sha256",
}
R51_DETECTOR_SUPPORT_RECORD_KEYS = {
    "preimage",
    "canonical_byte_length",
    "sha256",
}
R51_DETECTOR_SUPPORT_PREIMAGE_KEYS = {
    "contract",
    "plan_revision",
    "b0_sha",
    "phase",
    "entry_id",
    "device",
    "candidate_id",
    "target_id",
    "raw_detector",
    "canonical_assignment",
    "emitted_scene_quad",
    "eligible_text_line_quad",
    "detector_support_mask",
    "line_support_mask",
    "line_support_equals_detector",
    "agreed_mask",
    "agreed_mask_subset",
    "protected_support_pixels",
    "unsupported_rotation_selected",
    "unmatched_selected_nodes",
    "ownership_verdict",
    "selection_verdict",
    "rejection_reason",
}
R51_RAW_DETECTOR_KEYS = {
    "index",
    "source_scaled_quad_f32_bits",
    "rect",
    "recognition_present",
    "recognition_class",
}
R51_MASK_DESCRIPTOR_KEYS = {
    "width",
    "height",
    "stride",
    "pixel_encoding",
    "row_order",
    "bytes_sha256",
}
JPEG_SOF_MARKERS = {
    0xC0,
    0xC1,
    0xC2,
    0xC3,
    0xC5,
    0xC6,
    0xC7,
    0xC9,
    0xCA,
    0xCB,
    0xCD,
    0xCE,
    0xCF,
}


class LedgerError(Exception):
    pass


@dataclass
class HeldPath:
    path: str
    fd: int
    stat: os.stat_result


def _checkpoint(_point):
    """Test-only deterministic fault/race hook."""


def _platform_capabilities():
    flags = ("O_DIRECTORY", "O_NOFOLLOW", "O_NONBLOCK")
    functions = (os.open, os.mkdir, os.rename, os.unlink)
    return (
        all(hasattr(os, flag) for flag in flags)
        and all(function in os.supports_dir_fd for function in functions)
        and os.listdir in os.supports_fd
    )


def _require_platform_capabilities():
    if not _platform_capabilities():
        raise LedgerError(
            "required descriptor-relative filesystem operations are unavailable"
        )


def _identity(value):
    return (value.st_dev, value.st_ino, stat.S_IFMT(value.st_mode))


def _mode(value):
    return stat.S_IMODE(value.st_mode)


def _validate_text(value, label):
    if not isinstance(value, str) or not value:
        raise LedgerError(f"{label} must be nonempty text")
    if "\x00" in value or "\r" in value or "\n" in value:
        raise LedgerError(f"{label} contains forbidden control data")
    return value


def _validate_utc_seconds(value, label):
    value = _validate_text(value, label)
    if not B0_UTC_SECONDS_RE.fullmatch(value):
        raise LedgerError(f"{label} must be UTC RFC3339 seconds")
    try:
        return datetime.datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(
            tzinfo=datetime.timezone.utc
        )
    except ValueError as error:
        raise LedgerError(f"{label} must be a valid UTC timestamp") from error


def _validate_b0_raw_log(root, relpath, expected_sha256, label):
    relpath = _validate_text(relpath, f"{label} path")
    _validate_hash(expected_sha256, f"{label} sha256")
    if os.path.isabs(relpath) or os.path.normpath(relpath) != relpath:
        raise LedgerError(f"{label} path must be relative and normalized")
    if relpath == "." or relpath.startswith("../") or "/../" in relpath:
        raise LedgerError(f"{label} path must remain below the artifact directory")
    path = os.path.join(root, relpath)
    if os.path.realpath(path) != path:
        raise LedgerError(f"{label} path must be canonical")
    stat_result = os.stat(path)
    if stat.S_IFMT(stat_result.st_mode) != stat.S_IFREG:
        raise LedgerError(f"{label} path must be a regular file")
    if _mode(stat_result) != 0o600:
        raise LedgerError(f"{label} mode must be 0600")
    with open(path, "rb") as handle:
        if _sha256(handle.read()) != expected_sha256:
            raise LedgerError(f"{label} sha256 drift")


def _expected_frozen_recall(selected_candidate_id):
    return {
        "candidate_set": [candidate["id"] for candidate in B0_CANDIDATES],
        "selected_candidate_id": selected_candidate_id,
        **{
            field: _sha256(preimage.encode("utf-8"))
            for field, preimage in B0_RECALL_PREIMAGES.items()
        },
    }


def _validate_required_check(
    check,
    expected_phase,
    artifact_root,
    checker_endpoint_sha256,
    b0_sha,
    manifest_sha256,
    fixture_manifest_sha256,
):
    _require_keys(check, B0_REQUIRED_CHECK_KEYS, "B0 required check")
    relpath = f"source-gate-selection/checks/{expected_phase}.json"
    expected = {
        "phase": expected_phase,
        "command": B0_REQUIRED_CHECK_COMMAND,
        "checker_endpoint_sha256": checker_endpoint_sha256,
        "manifest_sha256": manifest_sha256,
        "source_gate_fixture_manifest_sha256": fixture_manifest_sha256,
        "attestation_relpath": relpath,
        "attestation_sha256": check["attestation_sha256"],
        "b0_sha": b0_sha,
        "result": "pass",
    }
    if check != expected:
        raise LedgerError("B0 required-check entry drift")
    _validate_hash(check["attestation_sha256"], "required-check attestation sha256")
    path = os.path.join(artifact_root, relpath)
    with contextlib.ExitStack() as stack:
        held = _open_absolute(path, directory=False, stack=stack)
        if _mode(held.stat) != 0o600:
            raise LedgerError("required-check attestation mode must be 0600")
        if _mode(os.stat(os.path.dirname(path))) != 0o700:
            raise LedgerError("required-check attestation parent mode must be 0700")
        data = _read_all(held.fd)
    if _sha256(data) != check["attestation_sha256"]:
        raise LedgerError("required-check attestation sha256 drift")
    attestation = _parse_json(data, "B0 required-check attestation")
    _require_keys(attestation, B0_ATTESTATION_KEYS, "B0 required-check attestation")
    if canonical_json(attestation) != data:
        raise LedgerError("B0 required-check attestation is not canonical JSON")
    if (
        attestation["version"] != 1
        or attestation["mode"] != "b0-source-gate-anti-fixture"
        or attestation["phase"] != expected_phase
        or attestation["b0_sha"] != b0_sha
        or attestation["manifest_sha256"] != manifest_sha256
        or attestation["source_gate_fixture_manifest_sha256"] != fixture_manifest_sha256
        or attestation["checker_endpoint_sha256"] != checker_endpoint_sha256
        or attestation["scanned_roots"] != B0_ANTI_FIXTURE_SCANNED_ROOTS
        or attestation["allowed_descriptor_roots"]
        != B0_ANTI_FIXTURE_ALLOWED_DESCRIPTOR_ROOTS
        or attestation["result"] != "pass"
    ):
        raise LedgerError("B0 required-check attestation drift")
    _validate_hash(attestation["policy_scan_sha256"], "policy scan sha256")


def _canonical_existing_path(value, label):
    value = _validate_text(value, label)
    if not os.path.isabs(value):
        raise LedgerError(f"{label} must be absolute")
    if os.path.normpath(value) != value or os.path.realpath(value) != value:
        raise LedgerError(f"{label} must be canonical")
    return value


def _canonical_future_path(value, label):
    value = _validate_text(value, label)
    if not os.path.isabs(value):
        raise LedgerError(f"{label} must be absolute")
    if os.path.normpath(value) != value or os.path.realpath(value) != value:
        raise LedgerError(f"{label} must be canonical")
    return value


def _parts(path):
    if path == "/":
        return ()
    return tuple(part for part in path.split("/") if part)


def _open_absolute(path, *, directory, stack):
    path = _canonical_existing_path(path, "path")
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    current = os.open("/", flags)
    stack.callback(os.close, current)
    parts = _parts(path)
    if not parts:
        if not directory:
            raise LedgerError("filesystem root is not a regular file")
        value = os.fstat(current)
        return HeldPath(path, current, value)
    for index, child in enumerate(parts):
        final = index == len(parts) - 1
        child_flags = os.O_RDONLY | os.O_NOFOLLOW
        if not final or directory:
            child_flags |= os.O_DIRECTORY
        else:
            child_flags |= os.O_NONBLOCK
        try:
            opened = os.open(child, child_flags, dir_fd=current)
        except OSError as error:
            raise LedgerError(f"cannot descriptor-open {path}: {error}") from error
        stack.callback(os.close, opened)
        current = opened
    value = os.fstat(current)
    if directory and not stat.S_ISDIR(value.st_mode):
        raise LedgerError(f"not a directory: {path}")
    if not directory and not stat.S_ISREG(value.st_mode):
        raise LedgerError(f"not a regular file: {path}")
    return HeldPath(path, current, value)


def _open_child(parent, name, *, directory, stack):
    _validate_text(name, "child name")
    if "/" in name or name in {".", ".."}:
        raise LedgerError("child name must be direct")
    flags = os.O_RDONLY | os.O_NOFOLLOW
    if directory:
        flags |= os.O_DIRECTORY
    else:
        flags |= os.O_NONBLOCK
    try:
        opened = os.open(name, flags, dir_fd=parent.fd)
    except OSError as error:
        raise LedgerError(f"cannot descriptor-open child {name}: {error}") from error
    stack.callback(os.close, opened)
    value = os.fstat(opened)
    expected = stat.S_ISDIR if directory else stat.S_ISREG
    if not expected(value.st_mode):
        raise LedgerError(f"unexpected child type: {name}")
    return HeldPath(os.path.join(parent.path, name), opened, value)


def _read_all(fd):
    chunks = []
    while True:
        chunk = os.read(fd, 1024 * 1024)
        if not chunk:
            return b"".join(chunks)
        chunks.append(chunk)


def _sha256(value):
    return hashlib.sha256(value).hexdigest()


def _strict_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise LedgerError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def _parse_json(value, label):
    try:
        return json.loads(value.decode("utf-8"), object_pairs_hook=_strict_object)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise LedgerError(f"invalid {label} JSON: {error}") from error


def canonical_json(value):
    return (
        json.dumps(
            value,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
        )
        + "\n"
    ).encode("utf-8")


def _r51_canonical_json(value):
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")


def _require_keys(value, keys, label):
    if not isinstance(value, dict) or set(value) != keys:
        raise LedgerError(f"{label} keys are not closed and complete")


def _validate_backend_map(value, label, required_backend):
    if (
        not isinstance(value, dict)
        or not value
        or any(
            not isinstance(key, str) or type(size) is not int or size < 0
            for key, size in value.items()
        )
        or value.get(required_backend, 0) <= 0
    ):
        raise LedgerError(f"{label} is invalid")


def _validate_result(
    result,
    processes,
    entry_ids,
    candidate_ids,
    phase,
    artifact_root,
    *,
    allow_detector_support_coverage=False,
):
    label = f"{phase} result"
    _require_keys(result, B0_RESULT_KEYS, label)
    if (
        result["entry_id"] not in entry_ids
        or result["candidate_id"] not in candidate_ids
    ):
        raise LedgerError(f"{label} identity is invalid")
    process = processes.get(result["process_evidence_id"])
    if process is None or process["phase"] != phase:
        raise LedgerError(f"{label} process reference is invalid")
    execution = result["execution_evidence"]
    _require_keys(execution, B0_EXECUTION_KEYS, f"{label} execution_evidence")
    if (
        execution["paddle_instance_id"] != process["paddle_instance_id"]
        or execution["inference_completed"] is not True
    ):
        raise LedgerError(f"{label} instance or completion evidence is invalid")
    _validate_b0_raw_log(
        artifact_root,
        execution["raw_inference_log_relpath"],
        execution["raw_inference_log_sha256"],
        "raw inference log",
    )
    _validate_b0_raw_log(
        artifact_root,
        execution["source_gate_diagnostic_relpath"],
        execution["source_gate_diagnostic_sha256"],
        "source gate diagnostic",
    )
    nodes = result["runtime_nodes"]
    if not isinstance(nodes, list):
        raise LedgerError(f"{label} runtime_nodes must be an array")
    for node in nodes:
        _require_keys(node, B0_RUNTIME_NODE_KEYS, f"{label} runtime node")
        _validate_text(node["node_id"], "runtime node id")
        if (
            not isinstance(node["recognition_anchor"], list)
            or len(node["recognition_anchor"]) != 4
            or any(
                type(value) not in (int, float) for value in node["recognition_anchor"]
            )
            or type(node["node_rotation"]) not in (int, float)
            or type(node["text_rotation"]) not in (int, float)
            or type(node["selected_as_han"]) is not bool
        ):
            raise LedgerError(f"{label} runtime node is invalid")
    derived = result["derived"]
    _require_keys(derived, B0_DERIVED_KEYS, f"{label} derived")
    if derived["actual_device"] != process["requested_device"]:
        raise LedgerError(f"{label} derived device mismatch")
    for field in (
        "matched_target_ids",
        "selected_target_ids",
        "selected_protected_node_ids",
        "selected_rotation_target_ids",
        "unmatched_selected_node_ids",
    ):
        if not isinstance(derived[field], list) or any(
            not isinstance(value, str) for value in derived[field]
        ):
            raise LedgerError(f"{label} {field} is invalid")
    if (
        type(derived["target_recall"]) not in (int, float)
        or type(derived["protected_false_positive_count"]) is not int
        or type(derived["rotation_targets_excluded"]) is not bool
        or type(derived["passed"]) is not bool
    ):
        raise LedgerError(f"{label} derived metrics are invalid")
    coverage = derived["source_coverage_preflight"]
    _require_keys(coverage, B0_SOURCE_COVERAGE_KEYS, f"{label} source coverage")
    if (
        type(coverage["pp_han_scalar_count"]) is not int
        or type(coverage["vl_expected_han_scalar_count"]) is not int
        or type(coverage["pp_vl_complete_coverage"]) is not bool
        or type(coverage["rejected_after_vl"]) is not bool
        or type(coverage["pp_vl_incomplete_coverage"]) is not bool
        or not isinstance(coverage["covered_source_roi_ids"], list)
        or any(
            not isinstance(value, str) or not value
            for value in coverage["covered_source_roi_ids"]
        )
        or type(coverage["source_text_roi_coverage"]) not in (int, float)
        or type(coverage["source_removal_preflight_passed"]) is not bool
    ):
        raise LedgerError(f"{label} source coverage evidence is invalid")
    count_coverage = (
        True
        if allow_detector_support_coverage
        else coverage["pp_han_scalar_count"] > 0
        and coverage["pp_han_scalar_count"] == coverage["vl_expected_han_scalar_count"]
    )
    expected_complete_coverage = (
        count_coverage
        and coverage["rejected_after_vl"] is False
        and coverage["pp_vl_incomplete_coverage"] is False
        and coverage["source_text_roi_coverage"] == 1.0
    )
    expected_preflight = derived["target_recall"] == 1.0 and expected_complete_coverage
    expected_pass = (
        expected_preflight
        and derived["protected_false_positive_count"] == 0
        and not derived["selected_protected_node_ids"]
        and not derived["selected_rotation_target_ids"]
        and not derived["unmatched_selected_node_ids"]
        and derived["rotation_targets_excluded"] is True
    )
    if (
        coverage["pp_vl_complete_coverage"] is not expected_complete_coverage
        or coverage["source_removal_preflight_passed"] is not expected_preflight
        or derived["passed"] is not expected_pass
    ):
        raise LedgerError(f"{label} source coverage or pass evidence is inconsistent")
    load = process["load_evidence"]
    requested_device = process["requested_device"]
    loaded = load["loaded_model_devices"]
    model_map = load["model_buffer_bytes_by_backend"]
    context_map = execution["context_buffer_bytes_by_backend"]
    compute_map = execution["compute_buffer_bytes_by_backend"]
    if requested_device == "cpu":
        if (
            load["cpu_forced"] is not True
            or load["n_gpu_layers"] != 0
            or load["mtmd_use_gpu"] is not False
            or execution["context_offload_kqv"] is not False
            or execution["context_op_offload"] is not False
            or not loaded
            or any(
                device["backend"] != "CPU" or device["device_type"] != "cpu"
                for device in loaded
            )
            or load["offloaded_layers"] != 0
            or load["mtmd_backend"] != "CPU"
        ):
            raise LedgerError(f"{label} CPU derivation is invalid")
        for name, backend_map in (
            ("model buffer map", model_map),
            ("context buffer map", context_map),
            ("compute buffer map", compute_map),
        ):
            _validate_backend_map(backend_map, name, "CPU")
            if any(
                size > 0 for backend, size in backend_map.items() if backend != "CPU"
            ):
                raise LedgerError(f"{label} CPU map contains non-CPU bytes")
    else:
        if (
            load["cpu_forced"] is not False
            or load["n_gpu_layers"] != B0_DEFAULT_GPU_LAYERS
            or load["mtmd_use_gpu"] is not True
            or execution["context_offload_kqv"] is not True
            or execution["context_op_offload"] is not True
            or not any(device["backend"] == "Metal" for device in loaded)
            or any(device["backend"] not in {"CPU", "Metal"} for device in loaded)
            or load["offloaded_layers"] <= 0
            or load["mtmd_backend"] != "Metal"
        ):
            raise LedgerError(f"{label} Metal derivation is invalid")
        _validate_backend_map(model_map, "model buffer map", "Metal")
        _validate_backend_map(context_map, "context buffer map", "Metal")
        _validate_backend_map(compute_map, "compute buffer map", "Metal")


def _b0_frozen_projection(value):
    return {
        "backend_evidence_parser_version": value["backend_evidence_parser_version"],
        "b0_sha": value["b0_sha"],
        "calibration_entry_ids": value["calibration_entry_ids"],
        "calibration_results": sorted(
            value["calibration_results"],
            key=lambda result: (
                result["entry_id"],
                result["process_evidence_id"],
                result["candidate_id"],
            ),
        ),
        "candidates": value["candidates"],
        "color_constant_set_sha256": value["color_constant_set_sha256"],
        "enabled_cargo_features": value["enabled_cargo_features"],
        "frozen_at_utc": value["frozen_at_utc"],
        "frozen_recall_contract": value["frozen_recall_contract"],
        "holdout_entry_ids": value["holdout_entry_ids"],
        "image_input_contract_sha256": value["image_input_contract_sha256"],
        "manifest_sha256": value["manifest_sha256"],
        "plan_revision": value["plan_revision"],
        "process_evidence": sorted(
            (
                process
                for process in value["process_evidence"]
                if process["phase"] == "calibration"
            ),
            key=lambda process: process["id"],
        ),
        "requested_devices": value["requested_devices"],
        "required_checks": [
            check
            for check in value["required_checks"]
            if check["phase"] == "pre-calibration"
        ],
        "retuned_after_freeze": value["retuned_after_freeze"],
        "selected_candidate_id": value["selected_candidate_id"],
        "source_color_contract_sha256": value["source_color_contract_sha256"],
        "source_gate_fixture_manifest_sha256": value[
            "source_gate_fixture_manifest_sha256"
        ],
    }


def _validate_b0_artifact(arguments):
    if not B0_SHA_RE.fullmatch(arguments.b0_sha):
        raise LedgerError("b0 sha must be 40 lowercase hexadecimal characters")
    _validate_hash(arguments.visual_manifest_sha256, "visual manifest sha256")
    _validate_hash(
        arguments.source_gate_fixture_manifest_sha256,
        "source gate fixture manifest sha256",
    )
    with contextlib.ExitStack() as stack:
        repo_root = _open_absolute(arguments.repo_root, directory=True, stack=stack)
        checker = _open_absolute(
            os.path.join(repo_root.path, B0_CHECKER_ENDPOINT),
            directory=False,
            stack=stack,
        )
        checker_endpoint_sha256 = _sha256(_read_all(checker.fd))
        artifact = _open_absolute(arguments.artifact, directory=False, stack=stack)
        artifact_bytes = _read_all(artifact.fd)
    artifact_root = os.path.dirname(artifact.path)
    value = _parse_json(artifact_bytes, "B0 frozen artifact")
    _require_keys(value, B0_ROOT_KEYS, "B0 frozen artifact")
    if canonical_json(value) != artifact_bytes:
        raise LedgerError("B0 frozen artifact is not canonical JSON")
    if (
        type(value["version"]) is not int
        or value["version"] != B0_VERSION
        or type(value["plan_revision"]) is not int
        or value["plan_revision"] != B0_PLAN_REVISION
    ):
        raise LedgerError("B0 frozen artifact version or plan revision mismatch")
    if value["b0_sha"] != arguments.b0_sha:
        raise LedgerError("B0 sha drift")
    if value["manifest_sha256"] != arguments.visual_manifest_sha256:
        raise LedgerError("visual manifest hash drift")
    if (
        value["source_gate_fixture_manifest_sha256"]
        != arguments.source_gate_fixture_manifest_sha256
    ):
        raise LedgerError("source gate fixture manifest hash drift")
    for field in (
        "manifest_sha256",
        "source_gate_fixture_manifest_sha256",
        "image_input_contract_sha256",
        "source_color_contract_sha256",
        "color_constant_set_sha256",
        "frozen_payload_sha256",
    ):
        _validate_hash(value[field], field)
    if value["requested_devices"] != ["cpu", "metal"]:
        raise LedgerError("requested devices drift")
    if value["enabled_cargo_features"] != ["hanonly-test-evidence", "metal"]:
        raise LedgerError("enabled cargo features drift")
    if value["backend_evidence_parser_version"] != 1:
        raise LedgerError("backend evidence parser version drift")
    if value["candidates"] != B0_CANDIDATES:
        raise LedgerError("candidate ratios drift")
    candidate_ids = {candidate["id"] for candidate in B0_CANDIDATES}
    if value["selected_candidate_id"] not in candidate_ids:
        raise LedgerError("invalid selected candidate")
    expected_recall = _expected_frozen_recall(value["selected_candidate_id"])
    _require_keys(
        value["frozen_recall_contract"],
        B0_FROZEN_RECALL_KEYS,
        "frozen recall contract",
    )
    if value["frozen_recall_contract"] != expected_recall:
        raise LedgerError("frozen recall contract drift")
    required_checks = value["required_checks"]
    if not isinstance(required_checks, list) or len(required_checks) != 2:
        raise LedgerError("B0 required checks must contain two records")
    for check, phase in zip(required_checks, ("pre-calibration", "pre-holdout")):
        _validate_required_check(
            check,
            phase,
            artifact_root,
            checker_endpoint_sha256,
            value["b0_sha"],
            value["manifest_sha256"],
            value["source_gate_fixture_manifest_sha256"],
        )
    calibration_ids = value["calibration_entry_ids"]
    holdout_ids = value["holdout_entry_ids"]
    if (
        not isinstance(calibration_ids, list)
        or not isinstance(holdout_ids, list)
        or len(calibration_ids) != 4
        or len(holdout_ids) != 4
        or len(set(calibration_ids)) != 4
        or len(set(holdout_ids)) != 4
        or set(calibration_ids) & set(holdout_ids)
        or any(
            not isinstance(value, str) or not value
            for value in calibration_ids + holdout_ids
        )
    ):
        raise LedgerError("calibration and holdout entry ids are invalid")
    if value["retuned_after_freeze"] is not False:
        raise LedgerError("artifact was retuned after freeze")
    frozen_at = _validate_utc_seconds(value["frozen_at_utc"], "frozen timestamp")
    holdout_completed_at = _validate_utc_seconds(
        value["holdout_completed_at_utc"], "holdout completion timestamp"
    )
    if holdout_completed_at <= frozen_at:
        raise LedgerError("holdout completion timestamp must be after freeze")
    process_evidence = value["process_evidence"]
    if not isinstance(process_evidence, list) or len(process_evidence) != 4:
        raise LedgerError("process evidence matrix must contain four records")
    processes = {}
    process_fingerprints = set()
    for process in process_evidence:
        _require_keys(process, B0_PROCESS_KEYS, "process evidence")
        process_id = _validate_text(process["id"], "process evidence id")
        if process_id in processes:
            raise LedgerError("duplicate process evidence id")
        if process["phase"] not in {"calibration", "holdout"}:
            raise LedgerError("invalid process phase")
        if process["requested_device"] not in {"cpu", "metal"}:
            raise LedgerError("invalid requested device")
        if not re.fullmatch(r"[0-9a-f]{32}", process["paddle_instance_id"] or ""):
            raise LedgerError("invalid paddle instance id")
        _validate_hash(process["executable_sha256"], "selection executable sha256")
        _require_keys(
            process["model_artifact_sha256"], B0_MODEL_HASH_KEYS, "model hashes"
        )
        for digest in process["model_artifact_sha256"].values():
            _validate_hash(digest, "model artifact sha256")
        libraries = process["runtime_library_sha256"]
        if not isinstance(libraries, dict) or not libraries:
            raise LedgerError("runtime library hashes must be nonempty")
        for path, digest in libraries.items():
            _validate_text(path, "runtime library path")
            _validate_hash(digest, "runtime library sha256")
        process_fingerprints.add(
            (
                process["executable_sha256"],
                canonical_json(process["model_artifact_sha256"]),
                canonical_json(libraries),
            )
        )
        load = process["load_evidence"]
        _require_keys(load, B0_LOAD_KEYS, "load evidence")
        _validate_text(load["word_boxes_backend"], "word boxes backend")
        _validate_b0_raw_log(
            artifact_root,
            load["raw_load_log_relpath"],
            load["raw_load_log_sha256"],
            "raw load log",
        )
        if (
            type(load["gpu_offload_supported"]) is not bool
            or type(load["offloaded_layers"]) is not int
            or type(load["offloadable_layers"]) is not int
            or type(load["model_buffer_bytes_by_backend"]) is not dict
        ):
            raise LedgerError("load evidence scalar types are invalid")
        if not isinstance(load["enumerated_devices"], list):
            raise LedgerError("enumerated devices must be an array")
        for device in load["enumerated_devices"]:
            _require_keys(device, B0_ENUMERATED_DEVICE_KEYS, "enumerated device")
        loaded = load["loaded_model_devices"]
        if not isinstance(loaded, list) or not loaded:
            raise LedgerError("loaded model devices must be nonempty")
        for ordinal, device in enumerate(loaded):
            _require_keys(device, B0_LOADED_DEVICE_KEYS, "loaded model device")
            if (
                device["model_device_ordinal"] != ordinal
                or not isinstance(device["name"], str)
                or not device["name"]
                or device["backend"] not in {"CPU", "Metal"}
                or device["device_type"]
                not in {
                    "cpu",
                    "accelerator",
                    "gpu",
                    "integrated_gpu",
                    "unknown",
                }
            ):
                raise LedgerError("loaded model device is invalid")
        processes[process_id] = process
    if len(process_fingerprints) != 1:
        raise LedgerError("process executable/model/runtime fingerprints drift")
    if {
        (process["phase"], process["requested_device"])
        for process in processes.values()
    } != {
        ("calibration", "cpu"),
        ("calibration", "metal"),
        ("holdout", "cpu"),
        ("holdout", "metal"),
    }:
        raise LedgerError("process evidence matrix is incomplete")
    calibration = value["calibration_results"]
    holdout = value["holdout_results"]
    if not isinstance(calibration, list) or len(calibration) != 32:
        raise LedgerError("calibration result matrix must contain 32 cells")
    if not isinstance(holdout, list) or len(holdout) != 8:
        raise LedgerError("holdout result matrix must contain 8 cells")
    for result in calibration:
        _validate_result(
            result,
            processes,
            calibration_ids,
            candidate_ids,
            "calibration",
            artifact_root,
        )
    for result in holdout:
        if result.get("candidate_id") != value["selected_candidate_id"]:
            raise LedgerError("holdout candidate drift")
        _validate_result(
            result, processes, holdout_ids, candidate_ids, "holdout", artifact_root
        )
    calibration_cells = {
        (
            result["entry_id"],
            processes[result["process_evidence_id"]]["requested_device"],
            result["candidate_id"],
        )
        for result in calibration
    }
    holdout_cells = {
        (
            result["entry_id"],
            processes[result["process_evidence_id"]]["requested_device"],
        )
        for result in holdout
    }
    if len(calibration_cells) != 32 or len(holdout_cells) != 8:
        raise LedgerError("selection result matrix contains duplicate or missing cells")
    selected = next(
        (
            candidate["id"]
            for candidate in B0_CANDIDATES
            if all(
                result["derived"]["passed"]
                for result in calibration
                if result["candidate_id"] == candidate["id"]
            )
        ),
        None,
    )
    if selected != value["selected_candidate_id"]:
        raise LedgerError("selected candidate is not the smallest all-pass candidate")
    if any(not result["derived"]["passed"] for result in holdout):
        raise LedgerError("holdout result failed")
    if (
        _sha256(canonical_json(_b0_frozen_projection(value)))
        != value["frozen_payload_sha256"]
    ):
        raise LedgerError("frozen payload sha256 mismatch")
    return b"PASS B0 frozen artifact\n"


def _jpeg_dimensions(data):
    if len(data) < 4 or data[:2] != b"\xff\xd8":
        raise LedgerError("invalid JPEG SOI")
    position = 2
    dimensions = []
    in_entropy = False
    saw_eoi = False
    while position < len(data):
        if in_entropy:
            marker = None
            while position < len(data):
                if data[position] != 0xFF:
                    position += 1
                    continue
                position += 1
                while position < len(data) and data[position] == 0xFF:
                    position += 1
                if position >= len(data):
                    raise LedgerError("truncated JPEG entropy marker")
                code = data[position]
                position += 1
                if code == 0x00 or 0xD0 <= code <= 0xD7:
                    continue
                marker = code
                in_entropy = False
                break
            if marker is None:
                raise LedgerError("truncated JPEG entropy data")
        else:
            if data[position] != 0xFF:
                raise LedgerError("malformed JPEG marker stream")
            position += 1
            while position < len(data) and data[position] == 0xFF:
                position += 1
            if position >= len(data):
                raise LedgerError("truncated JPEG marker")
            marker = data[position]
            position += 1
        if marker == 0xD9:
            saw_eoi = True
            if position != len(data):
                raise LedgerError("trailing JPEG data")
            break
        if marker in {0xD8, 0x01} or 0xD0 <= marker <= 0xD7:
            continue
        if position + 2 > len(data):
            raise LedgerError("truncated JPEG segment length")
        length = struct.unpack(">H", data[position : position + 2])[0]
        if length < 2:
            raise LedgerError("malformed JPEG segment length")
        end = position + length
        if end > len(data):
            raise LedgerError("truncated JPEG segment")
        payload = data[position + 2 : end]
        if marker in JPEG_SOF_MARKERS:
            if len(payload) < 6:
                raise LedgerError("truncated JPEG SOF")
            component_count = payload[5]
            if component_count == 0:
                raise LedgerError("JPEG SOF must declare at least one component")
            if len(payload) != 6 + 3 * component_count:
                raise LedgerError("JPEG SOF component table length mismatch")
            height, width = struct.unpack(">HH", payload[1:5])
            if width == 0 or height == 0:
                raise LedgerError("invalid JPEG dimensions")
            dimensions.append((width, height))
        position = end
        if marker == 0xDA:
            in_entropy = True
    if not saw_eoi:
        raise LedgerError("JPEG is missing EOI")
    if len(dimensions) != 1:
        raise LedgerError("JPEG must contain exactly one SOF dimension record")
    return dimensions[0]


def _webp_dimensions(data):
    if len(data) < 12 or data[:4] != b"RIFF" or data[8:12] != b"WEBP":
        raise LedgerError("invalid WebP RIFF header")
    declared_end = struct.unpack("<I", data[4:8])[0] + 8
    if declared_end != len(data):
        raise LedgerError("WebP RIFF length mismatch")
    position = 12
    records = {}
    while position < declared_end:
        if position + 8 > declared_end:
            raise LedgerError("truncated WebP chunk header")
        name = data[position : position + 4]
        size = struct.unpack("<I", data[position + 4 : position + 8])[0]
        payload_start = position + 8
        payload_end = payload_start + size
        padded_end = payload_end + (size & 1)
        if payload_end > declared_end or padded_end > declared_end:
            raise LedgerError("truncated WebP chunk")
        payload = data[payload_start:payload_end]
        if size & 1 and data[payload_end:padded_end] != b"\x00":
            raise LedgerError("invalid WebP padding")
        if name == b"VP8 ":
            if len(payload) < 10 or payload[3:6] != b"\x9d\x01\x2a":
                raise LedgerError("invalid VP8 frame header")
            width, height = struct.unpack("<HH", payload[6:10])
            width &= 0x3FFF
            height &= 0x3FFF
            if width == 0 or height == 0:
                raise LedgerError("invalid VP8 dimensions")
            current = (width, height)
        elif name == b"VP8L":
            if len(payload) < 5 or payload[0] != 0x2F:
                raise LedgerError("invalid VP8L header")
            bits = int.from_bytes(payload[1:5], "little")
            if bits >> 29:
                raise LedgerError("unsupported VP8L version")
            current = ((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1)
        elif name == b"VP8X":
            if len(payload) != 10 or payload[1:4] != b"\x00\x00\x00":
                raise LedgerError("invalid VP8X header")
            if payload[0] & 0x02:
                raise LedgerError("animated WebP is unsupported")
            current = (
                int.from_bytes(payload[4:7], "little") + 1,
                int.from_bytes(payload[7:10], "little") + 1,
            )
        elif name in {b"ANIM", b"ANMF"}:
            raise LedgerError("animated WebP chunk is unsupported")
        else:
            current = None
        if current is not None:
            if name in records:
                raise LedgerError("duplicate WebP dimension record")
            if records and any(value != current for value in records.values()):
                raise LedgerError("contradictory WebP dimension records")
            records[name] = current
        position = padded_end
    if not records:
        raise LedgerError("unsupported WebP image chunk")
    payload_records = [name for name in records if name in {b"VP8 ", b"VP8L"}]
    if len(payload_records) > 1:
        raise LedgerError("duplicate WebP image payload")
    return next(iter(records.values()))


def image_dimensions(data):
    if data.startswith(b"\xff\xd8"):
        return _jpeg_dimensions(data)
    if data.startswith(b"RIFF"):
        return _webp_dimensions(data)
    raise LedgerError("unsupported image container")


def _validate_hash(value, label):
    if not isinstance(value, str) or not SHA256_RE.fullmatch(value):
        raise LedgerError(f"{label} must be 64 lowercase hexadecimal characters")
    return value


def _parse_size(value):
    match = re.fullmatch(r"([1-9]\d*)x([1-9]\d*)", value or "")
    if not match:
        raise LedgerError("expected input size must be WIDTHxHEIGHT")
    result = (int(match.group(1)), int(match.group(2)))
    if result != EXPECTED_DIMENSIONS:
        raise LedgerError("expected input size is not the approved regression size")
    return result


def _validate_manifest_regression(value, input_path, input_sha256):
    manifest = _parse_json(value, "visual manifest")
    if not isinstance(manifest, dict) or not isinstance(manifest.get("entries"), list):
        raise LedgerError("visual manifest entries are missing")
    regressions = [
        entry
        for entry in manifest["entries"]
        if isinstance(entry, dict) and entry.get("role") == "regression"
    ]
    if len(regressions) != 1:
        raise LedgerError("visual manifest must contain exactly one regression entry")
    regression = regressions[0]
    if regression.get("path") != input_path:
        raise LedgerError("selected input does not match the regression manifest path")
    if regression.get("sha256") != input_sha256:
        raise LedgerError("selected input hash does not match the regression manifest")


def _require_owned_mode(path, value, expected_mode):
    if value.st_uid != os.geteuid():
        raise LedgerError(f"{path} is not owned by the current user")
    if _mode(value) != expected_mode:
        raise LedgerError(f"{path} must have mode {expected_mode:04o}")


def _run_git(repo_root, arguments):
    try:
        return subprocess.run(
            ["git", "-C", repo_root, *arguments],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            shell=False,
        )
    except OSError as error:
        raise LedgerError(f"cannot run git: {error}") from error


def _validate_repository(repo_root):
    physical_pwd = subprocess.run(
        ["pwd", "-P"],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        shell=False,
    ).stdout.rstrip("\n")
    if physical_pwd != repo_root:
        raise LedgerError("repo root does not equal canonical pwd -P")
    result = _run_git(repo_root, ["rev-parse", "--show-toplevel"])
    if result.returncode != 0:
        raise LedgerError("repo root is not a Git worktree")
    top = result.stdout.decode("utf-8").rstrip("\n")
    if os.path.realpath(top) != repo_root:
        raise LedgerError("repo root does not equal the Git top level")


def _single_nul_record(result, label):
    if result.returncode != 0 or not result.stdout.endswith(b"\0"):
        raise LedgerError(f"cannot read {label}")
    records = result.stdout[:-1].split(b"\0")
    if len(records) != 1 or not records[0]:
        raise LedgerError(f"{label} is not exactly one record")
    return records[0]


def _fixture_is_tracked_and_clean(repo_root, fixture_bytes, fixture_sha256):
    flags = _run_git(
        repo_root,
        ["ls-files", "-v", "-z", "--", FIXTURE_RELATIVE_PATH],
    )
    flag_record = _single_nul_record(flags, "fixed fixture index flags")
    expected_flag_record = b"H " + FIXTURE_RELATIVE_PATH.encode("utf-8")
    if flag_record != expected_flag_record:
        raise LedgerError("fixed fixture manifest has forbidden index flags")
    staged = _run_git(
        repo_root,
        ["ls-files", "--stage", "-z", "--", FIXTURE_RELATIVE_PATH],
    )
    stage_record = _single_nul_record(staged, "fixed fixture index entry")
    try:
        header, path = stage_record.split(b"\t", 1)
        mode, object_id, stage = header.split(b" ")
    except ValueError as error:
        raise LedgerError("fixed fixture index entry is malformed") from error
    if (
        mode != b"100644"
        or stage != b"0"
        or path != FIXTURE_RELATIVE_PATH.encode("utf-8")
        or not re.fullmatch(rb"[0-9a-f]{40}|[0-9a-f]{64}", object_id)
    ):
        raise LedgerError("fixed fixture index entry is not the expected stage-0 blob")
    index_blob = _run_git(repo_root, ["cat-file", "blob", object_id.decode("ascii")])
    if index_blob.returncode != 0:
        raise LedgerError("cannot read fixed fixture index blob")
    if (
        fixture_bytes != index_blob.stdout
        or fixture_sha256 != _sha256(fixture_bytes)
        or fixture_sha256 != _sha256(index_blob.stdout)
    ):
        raise LedgerError("fixed fixture bytes differ from the index blob")
    status_result = _run_git(
        repo_root,
        ["status", "--porcelain=v1", "--", FIXTURE_RELATIVE_PATH],
    )
    if status_result.returncode != 0 or status_result.stdout:
        raise LedgerError("fixed fixture manifest is dirty or untracked")


def _git_worktree_paths(repo_root):
    result = _run_git(repo_root, ["worktree", "list", "--porcelain", "-z"])
    if result.returncode != 0:
        raise LedgerError("cannot enumerate Git worktrees")
    paths = []
    for field in result.stdout.split(b"\0"):
        if field.startswith(b"worktree "):
            try:
                paths.append(os.path.realpath(field[9:].decode("utf-8")))
            except UnicodeDecodeError as error:
                raise LedgerError("Git worktree path is not UTF-8") from error
    for name in (
        "HANONLY_ORIGINAL_WORKTREE",
        "HANONLY_IMPLEMENTATION_WORKTREE",
        "HANONLY_B0_WORKTREE",
        "HANONLY_ACCEPTANCE_WORKTREE",
        "HANONLY_PR_WORKTREE",
    ):
        value = os.environ.get(name)
        if value:
            paths.append(_canonical_future_path(value, name))
    return set(paths)


def _is_beneath(path, parent):
    try:
        return os.path.commonpath((path, parent)) == parent
    except ValueError:
        return False


def _validate_external_base(repo_root, base):
    if base == "/":
        raise LedgerError("evidence base cannot be the filesystem root")
    for worktree in _git_worktree_paths(repo_root):
        if _is_beneath(base, worktree):
            raise LedgerError("evidence base must be outside every worktree")


def _expected_base(argument=None):
    environment = os.environ.get("HANONLY_SHARED_EVIDENCE_BASE")
    if not environment:
        raise LedgerError("HANONLY_SHARED_EVIDENCE_BASE is required")
    environment = _canonical_existing_path(environment, "HANONLY_SHARED_EVIDENCE_BASE")
    if argument is not None:
        argument = _canonical_existing_path(argument, "expected base")
        if argument != environment:
            raise LedgerError(
                "expected base disagrees with HANONLY_SHARED_EVIDENCE_BASE"
            )
    return environment


def _validate_ledger(value, expected_root):
    if not isinstance(value, dict) or set(value) != LEDGER_KEYS:
        raise LedgerError("ledger schema keys are not exact")
    if type(value["version"]) is not int or value["version"] != LEDGER_VERSION:
        raise LedgerError("ledger version must be integer 1")
    for key in (
        "visual_input_sha256",
        "visual_manifest_sha256",
        "source_gate_fixture_manifest_sha256",
    ):
        _validate_hash(value[key], key)
    for key in ("visual_input", "visual_manifest", "evidence_root"):
        _canonical_existing_path(value[key], key)
    if value["evidence_root"] != expected_root:
        raise LedgerError("ledger evidence root does not match the requested root")
    return value


def _nul_output(value):
    fields = (
        value["visual_input"],
        value["visual_input_sha256"],
        value["visual_manifest"],
        value["visual_manifest_sha256"],
        value["evidence_root"],
        value["source_gate_fixture_manifest_sha256"],
    )
    return b"".join(field.encode("utf-8") + b"\0" for field in fields)


def _fresh_identity_pass(paths, expected_files, *, checkpoints):
    with contextlib.ExitStack() as stack:
        for label, held, directory in paths:
            fresh = _open_absolute(held.path, directory=directory, stack=stack)
            if _identity(fresh.stat) != _identity(held.stat):
                raise LedgerError(f"namespace identity changed for {label}")
            if held.stat.st_uid != fresh.stat.st_uid or _mode(held.stat) != _mode(
                fresh.stat
            ):
                raise LedgerError(f"namespace metadata changed for {label}")
            if label in expected_files:
                expected_bytes, expected_hash = expected_files[label]
                fresh_bytes = _read_all(fresh.fd)
                if (
                    fresh_bytes != expected_bytes
                    or _sha256(fresh_bytes) != expected_hash
                ):
                    raise LedgerError(f"fresh content changed for {label}")
            if checkpoints:
                _checkpoint(f"identity_checked:{label}")


def _final_identity_proof(paths, expected_files):
    _fresh_identity_pass(paths, expected_files, checkpoints=True)
    _checkpoint("before_output_recheck")
    _fresh_identity_pass(paths, expected_files, checkpoints=False)
    _checkpoint("immediately_before_output")
    _fresh_identity_pass(paths, expected_files, checkpoints=False)


def _preflight_open(
    *,
    repo_root,
    base_path,
    input_path,
    manifest_path,
    fixture_path,
    stack,
):
    repo = _open_absolute(repo_root, directory=True, stack=stack)
    base = _open_absolute(base_path, directory=True, stack=stack)
    _require_owned_mode(base.path, base.stat, 0o700)
    input_file = _open_absolute(input_path, directory=False, stack=stack)
    manifest = _open_absolute(manifest_path, directory=False, stack=stack)
    fixture = _open_absolute(fixture_path, directory=False, stack=stack)
    return repo, base, input_file, manifest, fixture


def _validate_fixture_path(repo_root, fixture_path):
    expected = os.path.join(repo_root, FIXTURE_RELATIVE_PATH)
    if fixture_path != expected:
        raise LedgerError("source-gate fixture manifest path is not fixed")


def _open_or_create_run(base, run_id, stack):
    try:
        return _open_child(base, run_id, directory=True, stack=stack)
    except LedgerError as error:
        cause = error.__cause__
        if not isinstance(cause, OSError) or cause.errno != errno.ENOENT:
            raise
    try:
        os.mkdir(run_id, 0o700, dir_fd=base.fd)
    except OSError as error:
        raise LedgerError(f"cannot create run root: {error}") from error
    _checkpoint("run_creation")
    return _open_child(base, run_id, directory=True, stack=stack)


def _owned_regular_child(run, name, stack):
    child = _open_child(run, name, directory=False, stack=stack)
    _require_owned_mode(child.path, child.stat, 0o600)
    return child


def _write_complete(fd, value):
    midpoint = max(1, len(value) // 2)
    offset = 0
    while offset < midpoint:
        written = os.write(fd, value[offset:midpoint])
        if written <= 0:
            raise LedgerError("temporary ledger write made no progress")
        offset += written
    _checkpoint("partial_write")
    while offset < len(value):
        written = os.write(fd, value[offset:])
        if written <= 0:
            raise LedgerError("temporary ledger write made no progress")
        offset += written


def _persist_ledger(run, base, expected_bytes, temp_name, stack):
    names = os.listdir(run.fd)
    if names == [LEDGER_NAME] or set(names) == {LEDGER_NAME}:
        ledger_file = _owned_regular_child(run, LEDGER_NAME, stack)
        if _read_all(ledger_file.fd) != expected_bytes:
            raise LedgerError("existing final ledger bytes do not match")
    else:
        if names:
            if len(names) != 1 or names[0] != temp_name:
                raise LedgerError("existing run root has an unknown recovery state")
            _owned_regular_child(run, temp_name, stack)
            try:
                os.unlink(temp_name, dir_fd=run.fd)
            except OSError as error:
                raise LedgerError(
                    f"cannot remove deterministic temp: {error}"
                ) from error
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
        try:
            temp_fd = os.open(temp_name, flags, 0o600, dir_fd=run.fd)
        except OSError as error:
            raise LedgerError(f"cannot create deterministic temp: {error}") from error
        try:
            temp_stat = os.fstat(temp_fd)
            _require_owned_mode(temp_name, temp_stat, 0o600)
            _write_complete(temp_fd, expected_bytes)
            _checkpoint("temp_fsync")
            os.fsync(temp_fd)
        finally:
            os.close(temp_fd)
        _checkpoint("rename")
        try:
            os.rename(
                temp_name,
                LEDGER_NAME,
                src_dir_fd=run.fd,
                dst_dir_fd=run.fd,
            )
        except OSError as error:
            raise LedgerError(f"cannot promote final ledger: {error}") from error
        ledger_file = _owned_regular_child(run, LEDGER_NAME, stack)
        if _read_all(ledger_file.fd) != expected_bytes:
            raise LedgerError("promoted ledger bytes do not match")
    _checkpoint("final_file_fsync")
    os.fsync(ledger_file.fd)
    _checkpoint("run_directory_fsync")
    os.fsync(run.fd)
    _checkpoint("base_directory_fsync")
    os.fsync(base.fd)
    return ledger_file


def _create(arguments):
    _require_platform_capabilities()
    repo_root = _canonical_existing_path(arguments.repo_root, "repo root")
    _validate_repository(repo_root)
    base_path = _expected_base(arguments.expected_base)
    _validate_external_base(repo_root, base_path)
    run_id = _validate_text(arguments.run_id, "run id")
    if not RUN_ID_RE.fullmatch(run_id):
        raise LedgerError("run id does not match the Revision 46 grammar")
    evidence_root = _canonical_future_path(
        os.path.join(base_path, run_id),
        "evidence root",
    )
    if os.path.dirname(evidence_root) != base_path:
        raise LedgerError("evidence root is not a direct base child")
    input_path = _canonical_existing_path(arguments.input, "visual input")
    manifest_path = _canonical_existing_path(arguments.manifest, "visual manifest")
    fixture_path = _canonical_existing_path(
        arguments.source_gate_fixture_manifest,
        "source-gate fixture manifest",
    )
    _validate_fixture_path(repo_root, fixture_path)
    expected_hash = _validate_hash(
        arguments.expected_input_sha256, "expected input hash"
    )
    expected_dimensions = _parse_size(arguments.expected_input_size)

    with contextlib.ExitStack() as stack:
        repo, base, input_file, manifest, fixture = _preflight_open(
            repo_root=repo_root,
            base_path=base_path,
            input_path=input_path,
            manifest_path=manifest_path,
            fixture_path=fixture_path,
            stack=stack,
        )
        input_bytes = _read_all(input_file.fd)
        input_hash = _sha256(input_bytes)
        if input_hash != expected_hash:
            raise LedgerError("visual input SHA-256 mismatch")
        if image_dimensions(input_bytes) != expected_dimensions:
            raise LedgerError("visual input dimensions mismatch")
        manifest_bytes = _read_all(manifest.fd)
        _validate_manifest_regression(manifest_bytes, input_path, input_hash)
        fixture_bytes = _read_all(fixture.fd)
        fixture_hash = _sha256(fixture_bytes)
        _fixture_is_tracked_and_clean(repo_root, fixture_bytes, fixture_hash)
        value = {
            "version": LEDGER_VERSION,
            "visual_input": input_path,
            "visual_input_sha256": input_hash,
            "visual_manifest": manifest_path,
            "visual_manifest_sha256": _sha256(manifest_bytes),
            "source_gate_fixture_manifest_sha256": fixture_hash,
            "evidence_root": evidence_root,
        }
        expected_bytes = canonical_json(value)
        temp_name = f".evidence-ledger.{_sha256(expected_bytes)}.tmp"
        run = _open_or_create_run(base, run_id, stack)
        _require_owned_mode(run.path, run.stat, 0o700)
        ledger_file = _persist_ledger(run, base, expected_bytes, temp_name, stack)
        paths = (
            ("repo", repo, True),
            ("base", base, True),
            ("run", run, True),
            ("input", input_file, False),
            ("manifest", manifest, False),
            ("fixture", fixture, False),
            ("ledger", ledger_file, False),
        )
        expected_files = {
            "input": (input_bytes, input_hash),
            "manifest": (manifest_bytes, value["visual_manifest_sha256"]),
            "fixture": (fixture_bytes, value["source_gate_fixture_manifest_sha256"]),
            "ledger": (expected_bytes, _sha256(expected_bytes)),
        }
        _final_identity_proof(paths, expected_files)
        return _nul_output(value)


def _rehydrate(arguments):
    _require_platform_capabilities()
    repo_root = _canonical_existing_path(arguments.repo_root, "repo root")
    _validate_repository(repo_root)
    base_path = _expected_base()
    _validate_external_base(repo_root, base_path)
    evidence_root = _canonical_existing_path(arguments.evidence_root, "evidence root")
    if os.path.dirname(evidence_root) != base_path:
        raise LedgerError("evidence root is not a direct child of the canonical base")
    run_id = os.path.basename(evidence_root)
    if not RUN_ID_RE.fullmatch(run_id):
        raise LedgerError("evidence root run id does not match Revision 46")
    fixture_path = os.path.join(repo_root, FIXTURE_RELATIVE_PATH)

    with contextlib.ExitStack() as stack:
        repo = _open_absolute(repo_root, directory=True, stack=stack)
        base = _open_absolute(base_path, directory=True, stack=stack)
        _require_owned_mode(base.path, base.stat, 0o700)
        run = _open_child(base, run_id, directory=True, stack=stack)
        _require_owned_mode(run.path, run.stat, 0o700)
        names = os.listdir(run.fd)
        if set(names) != {LEDGER_NAME} or len(names) != 1:
            raise LedgerError("rehydration root must contain only the final ledger")
        ledger_file = _owned_regular_child(run, LEDGER_NAME, stack)
        ledger_bytes = _read_all(ledger_file.fd)
        value = _validate_ledger(_parse_json(ledger_bytes, "ledger"), evidence_root)
        if canonical_json(value) != ledger_bytes:
            raise LedgerError("ledger bytes are not canonical")
        input_file = _open_absolute(value["visual_input"], directory=False, stack=stack)
        manifest = _open_absolute(
            value["visual_manifest"], directory=False, stack=stack
        )
        fixture = _open_absolute(fixture_path, directory=False, stack=stack)
        input_bytes = _read_all(input_file.fd)
        input_hash = _sha256(input_bytes)
        if input_hash != value["visual_input_sha256"]:
            raise LedgerError("rehydrated visual input hash drift")
        if image_dimensions(input_bytes) != EXPECTED_DIMENSIONS:
            raise LedgerError("rehydrated visual input dimensions drift")
        manifest_bytes = _read_all(manifest.fd)
        if _sha256(manifest_bytes) != value["visual_manifest_sha256"]:
            raise LedgerError("rehydrated visual manifest hash drift")
        _validate_manifest_regression(manifest_bytes, value["visual_input"], input_hash)
        fixture_bytes = _read_all(fixture.fd)
        fixture_hash = _sha256(fixture_bytes)
        if fixture_hash != value["source_gate_fixture_manifest_sha256"]:
            raise LedgerError("rehydrated fixture manifest hash drift")
        _fixture_is_tracked_and_clean(repo_root, fixture_bytes, fixture_hash)
        paths = (
            ("repo", repo, True),
            ("base", base, True),
            ("run", run, True),
            ("input", input_file, False),
            ("manifest", manifest, False),
            ("fixture", fixture, False),
            ("ledger", ledger_file, False),
        )
        expected_files = {
            "input": (input_bytes, input_hash),
            "manifest": (manifest_bytes, value["visual_manifest_sha256"]),
            "fixture": (fixture_bytes, value["source_gate_fixture_manifest_sha256"]),
            "ledger": (ledger_bytes, _sha256(ledger_bytes)),
        }
        _final_identity_proof(paths, expected_files)
        return _nul_output(value)


def _r51_read(path, label, *, mode=None):
    path = _canonical_existing_path(path, label)
    with contextlib.ExitStack() as stack:
        held = _open_absolute(path, directory=False, stack=stack)
        if held.stat.st_uid != os.geteuid():
            raise LedgerError(f"{label} owner mismatch")
        if mode is not None and _mode(held.stat) != mode:
            raise LedgerError(f"{label} mode must be {mode:04o}")
        before = os.fstat(held.fd)
        value = _read_all(held.fd)
        after = os.fstat(held.fd)
        if (
            _identity(before) != _identity(after)
            or before.st_size != after.st_size
            or before.st_mtime_ns != after.st_mtime_ns
            or before.st_ctime_ns != after.st_ctime_ns
        ):
            raise LedgerError(f"{label} changed while being read")
    return path, value


def _r51_json(path, label, *, keys=None, canonical=True, mode=0o600):
    path, data = _r51_read(path, label, mode=mode)
    value = _parse_json(data, label)
    if keys is not None:
        _require_keys(value, keys, label)
    if canonical and _r51_canonical_json(value) != data:
        raise LedgerError(f"{label} is not canonical JSON")
    return path, data, value


def _r51_relative_path(root, relpath, label):
    relpath = _validate_text(relpath, label)
    if (
        os.path.isabs(relpath)
        or "\\" in relpath
        or os.path.normpath(relpath) != relpath
        or relpath == "."
        or any(part in {"", ".", ".."} for part in relpath.split("/"))
    ):
        raise LedgerError(f"{label} must be normalized beneath the diagnostic root")
    path = os.path.join(root, relpath)
    if not _is_beneath(path, root) or path == root:
        raise LedgerError(f"{label} escapes the diagnostic root")
    return path


def _r51_relative_file(root, relpath, label, *, keys=None, canonical=False):
    path = _r51_relative_path(root, relpath, label)
    if keys is None:
        path, data = _r51_read(path, label, mode=0o600)
        return path, data, None
    return _r51_json(
        path,
        label,
        keys=keys,
        canonical=canonical,
        mode=0o600,
    )


def _r51_scan_diagnostic_tree(root):
    root = _canonical_existing_path(root, "R51 diagnostic root")
    with contextlib.ExitStack() as stack:
        held_root = _open_absolute(root, directory=True, stack=stack)
        _require_owned_mode(held_root.path, held_root.stat, 0o700)

        def scan(directory):
            for name in sorted(os.listdir(directory.fd)):
                if name.endswith(".tmp"):
                    raise LedgerError("R51 diagnostic tree contains a temporary file")
                value = os.stat(name, dir_fd=directory.fd, follow_symlinks=False)
                if value.st_uid != os.geteuid():
                    raise LedgerError("R51 diagnostic tree owner mismatch")
                if stat.S_ISDIR(value.st_mode):
                    child = _open_child(directory, name, directory=True, stack=stack)
                    _require_owned_mode(child.path, child.stat, 0o700)
                    scan(child)
                elif stat.S_ISREG(value.st_mode):
                    child = _open_child(directory, name, directory=False, stack=stack)
                    _require_owned_mode(child.path, child.stat, 0o600)
                else:
                    raise LedgerError("R51 diagnostic tree contains an unsafe entry")

        scan(held_root)


def _r51_hash_file(path, label, *, mode=None):
    path, data = _r51_read(path, label, mode=mode)
    return path, data, _sha256(data)


def _r51_publish(path, value, label):
    _require_platform_capabilities()
    path = _canonical_future_path(path, label)
    parent_path = os.path.dirname(path)
    name = os.path.basename(path)
    expected_names = {
        "r51-b0-preflight.json",
        "r51-b0-authorization.json",
        "hanonly-r51-b0-artifact.json",
    }
    if name not in expected_names:
        raise LedgerError(f"{label} filename is not contract-fixed")
    data = _r51_canonical_json(value)
    digest = _sha256(data)
    temp_name = f".{name.removesuffix('.json')}.{digest}.tmp"
    with contextlib.ExitStack() as stack:
        parent = _open_absolute(parent_path, directory=True, stack=stack)
        _require_owned_mode(parent.path, parent.stat, 0o700)
        names = set(os.listdir(parent.fd))
        owned_prefix = f".{name.removesuffix('.json')}."
        owned_temps = sorted(
            item
            for item in names
            if item.startswith(owned_prefix) and item.endswith(".tmp")
        )
        if any(item != temp_name for item in owned_temps):
            raise LedgerError(f"{label} has an unknown deterministic temp")
        if name in names:
            if owned_temps:
                raise LedgerError(f"{label} final and temp cannot coexist")
            final = _open_child(parent, name, directory=False, stack=stack)
            _require_owned_mode(final.path, final.stat, 0o600)
            if _read_all(final.fd) != data:
                raise LedgerError(f"{label} existing final bytes drift")
            return final.path, digest
        if temp_name in names:
            temp = _open_child(parent, temp_name, directory=False, stack=stack)
            _require_owned_mode(temp.path, temp.stat, 0o600)
            os.unlink(temp_name, dir_fd=parent.fd)
            os.fsync(parent.fd)
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
        temp_fd = os.open(temp_name, flags, 0o600, dir_fd=parent.fd)
        try:
            _write_complete(temp_fd, data)
            os.fsync(temp_fd)
        finally:
            os.close(temp_fd)
        try:
            os.link(
                temp_name,
                name,
                src_dir_fd=parent.fd,
                dst_dir_fd=parent.fd,
                follow_symlinks=False,
            )
        except OSError as error:
            raise LedgerError(f"cannot publish {label}: {error}") from error
        final = _open_child(parent, name, directory=False, stack=stack)
        temp = _open_child(parent, temp_name, directory=False, stack=stack)
        _require_owned_mode(final.path, final.stat, 0o600)
        _require_owned_mode(temp.path, temp.stat, 0o600)
        final_bytes = _read_all(final.fd)
        if (
            final_bytes != data
            or _identity(final.stat) != _identity(temp.stat)
            or final.stat.st_ino != temp.stat.st_ino
        ):
            raise LedgerError(f"{label} publication identity drift")
        os.fsync(final.fd)
        os.fsync(parent.fd)
        os.unlink(temp_name, dir_fd=parent.fd)
        os.fsync(parent.fd)
        return final.path, digest


def _r51_validate_contract_files(arguments):
    for name, relative_path in R51_APPROVED_PUBLIC_FILES.items():
        expected_path = os.path.join(arguments.repo_root, relative_path)
        if getattr(arguments, name) != expected_path:
            raise LedgerError(f"approved R51 {name} path drift")
    contract_path, contract_bytes = _r51_read(
        arguments.r51_contract, "R51 contract", mode=None
    )
    contract = _parse_json(contract_bytes, "R51 contract")
    if contract.get("plan_revision") != R51_PLAN_REVISION:
        raise LedgerError("R51 contract plan revision drift")
    base_path, base_bytes = _r51_read(
        arguments.base_production_contract, "base production contract", mode=None
    )
    embedded_base = contract.get("base_production_contract")
    if not isinstance(embedded_base, dict) or embedded_base.get("sha256") != _sha256(
        base_bytes
    ):
        raise LedgerError("base production contract hash drift")
    plan_path, plan_bytes = _r51_read(
        arguments.operative_plan, "operative plan", mode=None
    )
    test_path, test_bytes = _r51_read(
        arguments.r51_test_spec, "R51 test spec", mode=None
    )
    hashes = {
        "r51_contract_sha256": _sha256(contract_bytes),
        "operative_plan_sha256": _sha256(plan_bytes),
        "r51_test_spec_sha256": _sha256(test_bytes),
        "base_production_contract_sha256": _sha256(base_bytes),
        "paths": (contract_path, plan_path, test_path, base_path),
    }
    expected = {
        "r51_contract_sha256": R51_CONTRACT_SHA256,
        "operative_plan_sha256": R51_OPERATIVE_PLAN_SHA256,
        "r51_test_spec_sha256": R51_TEST_SPEC_SHA256,
        "base_production_contract_sha256": R51_BASE_CONTRACT_SHA256,
    }
    if any(hashes[key] != value for key, value in expected.items()):
        raise LedgerError("approved R51 public contract hash drift")
    return hashes


def _r51_custody_namespace(arguments, *, authorized):
    paths = {
        "historical-inventory.json": arguments.historical_inventory,
        "holdout.enc": arguments.ciphertext,
        "holdout-freeze-receipt.json": arguments.freeze_receipt,
    }
    if authorized:
        paths["holdout-open.json"] = arguments.open_marker
        paths["holdout-terminal.json"] = arguments.terminal_receipt
    canonical = {}
    custody_root = None
    for basename, value in paths.items():
        value = _canonical_existing_path(value, f"R51 custody {basename}")
        if os.path.basename(value) != basename:
            raise LedgerError(f"R51 custody {basename} basename drift")
        parent = os.path.dirname(value)
        if custody_root is None:
            custody_root = parent
        elif parent != custody_root:
            raise LedgerError("R51 custody files must share one canonical namespace")
        canonical[basename] = value
    header_path = os.path.join(custody_root, "holdout-header.json")
    if os.path.realpath(header_path) != header_path:
        raise LedgerError("R51 custody header path is not canonical")
    canonical["holdout-header.json"] = header_path
    expected_names = (
        R51_CUSTODY_AUTHORIZED_NAMES if authorized else R51_CUSTODY_FROZEN_NAMES
    )
    with contextlib.ExitStack() as stack:
        custody = _open_absolute(custody_root, directory=True, stack=stack)
        _require_owned_mode(custody.path, custody.stat, 0o700)
        names = set(os.listdir(custody.fd))
        if names != expected_names:
            raise LedgerError(
                "R51 custody namespace has failure, temp, or unknown state"
            )
        for name in sorted(names):
            child = _open_child(custody, name, directory=False, stack=stack)
            _require_owned_mode(child.path, child.stat, 0o600)
    return custody_root, canonical


def _r51_preflight_custody_snapshot(arguments):
    contract_hashes = _r51_validate_contract_files(arguments)
    custody_root, paths = _r51_custody_namespace(arguments, authorized=False)
    with contextlib.ExitStack() as stack:
        custody = _open_absolute(custody_root, directory=True, stack=stack)
        _require_owned_mode(custody.path, custody.stat, 0o700)
        if set(os.listdir(custody.fd)) != R51_CUSTODY_FROZEN_NAMES:
            raise LedgerError(
                "R51 preflight custody has open, failure, terminal, temp, or unknown state"
            )
        files = {}
        contents = {}
        for name in sorted(R51_CUSTODY_FROZEN_NAMES):
            child = _open_child(custody, name, directory=False, stack=stack)
            _require_owned_mode(child.path, child.stat, 0o600)
            before = os.fstat(child.fd)
            data = _read_all(child.fd)
            after = os.fstat(child.fd)
            if (
                _identity(before) != _identity(after)
                or before.st_size != after.st_size
                or before.st_mtime_ns != after.st_mtime_ns
                or before.st_ctime_ns != after.st_ctime_ns
            ):
                raise LedgerError("R51 custody file changed during snapshot")
            contents[name] = data
            files[name] = {
                "st_dev": before.st_dev,
                "st_ino": before.st_ino,
                "uid": before.st_uid,
                "mode": _mode(before),
                "byte_length": len(data),
                "sha256": _sha256(data),
            }
        directory_after = os.fstat(custody.fd)
        if _identity(custody.stat) != _identity(directory_after):
            raise LedgerError("R51 custody directory identity drift")
    historical = _parse_json(
        contents["historical-inventory.json"], "historical inventory"
    )
    header = _parse_json(contents["holdout-header.json"], "holdout header")
    freeze = _parse_json(
        contents["holdout-freeze-receipt.json"], "holdout freeze receipt"
    )
    _require_keys(header, R51_HEADER_KEYS, "holdout header")
    _require_keys(freeze, R51_FREEZE_KEYS, "holdout freeze receipt")
    if (
        _r51_canonical_json(historical) != contents["historical-inventory.json"]
        or _r51_canonical_json(header) != contents["holdout-header.json"]
        or _r51_canonical_json(freeze) != contents["holdout-freeze-receipt.json"]
    ):
        raise LedgerError("R51 custody snapshot JSON is not canonical")
    ciphertext = contents["holdout.enc"]
    if (
        historical.get("contract") != "hanonly-r51-historical-inventory-v1"
        or historical.get("plan_revision") != R51_PLAN_REVISION
        or freeze["contract"] != "hanonly-r51-encrypted-holdout-freeze-v1"
        or freeze["plan_revision"] != R51_PLAN_REVISION
        or freeze["entry_ids"] != R51_HOLDOUT_IDS
        or freeze["frozen_before_production_edit"] is not True
        or freeze["historical_inventory_sha256"]
        != files["historical-inventory.json"]["sha256"]
        or freeze["ciphertext_sha256"] != files["holdout.enc"]["sha256"]
        or freeze["ciphertext_byte_length"] != len(ciphertext)
        or freeze["header_sha256"] != files["holdout-header.json"]["sha256"]
        or freeze["disclosed_challenge_exclusion_pass"] is not True
        or freeze["result"] != "pass"
        or header["contract"] != "hanonly-r51-encrypted-holdout-header-v1"
        or header["plan_revision"] != R51_PLAN_REVISION
        or header["cipher"] != "aes-256-ctr"
        or header["integrity"] != "hmac-sha256-etm-v1"
        or not re.fullmatch(r"[0-9a-f]{32}", header["iv_hex"] or "")
        or type(header["ciphertext_byte_length"]) is not int
        or header["ciphertext_byte_length"] != len(ciphertext)
        or type(header["plaintext_archive_byte_length"]) is not int
        or header["plaintext_archive_byte_length"] <= 0
        or freeze["cipher"] != header["cipher"]
        or freeze["integrity"] != header["integrity"]
        or freeze["iv_sha256"] != _sha256(bytes.fromhex(header["iv_hex"]))
    ):
        raise LedgerError("R51 custody snapshot binding drift")
    for key in (
        "iv_sha256",
        "ciphertext_sha256",
        "header_sha256",
        "hmac_sha256",
        "plaintext_archive_sha256_commitment",
        "manifest_sha256_commitment",
        "oracle_sha256_commitment",
        "hashes_sha256_commitment",
        "historical_inventory_sha256",
    ):
        _validate_hash(freeze[key], f"freeze {key}")
    return {
        "contract": "hanonly-r51-preflight-custody-snapshot-v1",
        **{key: value for key, value in contract_hashes.items() if key != "paths"},
        "custody_root": custody_root,
        "custody_root_st_dev": custody.stat.st_dev,
        "custody_root_st_ino": custody.stat.st_ino,
        "custody_root_uid": custody.stat.st_uid,
        "custody_root_mode": _mode(custody.stat),
        "fixed_paths": dict(sorted(paths.items())),
        "files": files,
    }


def _r51_validate_freeze(arguments, contract_hashes, *, authorized=False):
    _, custody_paths = _r51_custody_namespace(arguments, authorized=authorized)
    historical_path, historical_bytes, historical = _r51_json(
        custody_paths["historical-inventory.json"],
        "historical inventory",
        canonical=True,
        mode=0o600,
    )
    if (
        historical.get("contract") != "hanonly-r51-historical-inventory-v1"
        or historical.get("plan_revision") != R51_PLAN_REVISION
    ):
        raise LedgerError("historical inventory contract drift")
    inventory_hash = _sha256(historical_bytes)
    ciphertext_path, ciphertext_bytes, ciphertext_hash = _r51_hash_file(
        custody_paths["holdout.enc"], "holdout ciphertext", mode=0o600
    )
    header_path, header_bytes, header = _r51_json(
        custody_paths["holdout-header.json"],
        "holdout header",
        keys=R51_HEADER_KEYS,
        canonical=True,
        mode=0o600,
    )
    freeze_path, freeze_bytes, freeze = _r51_json(
        custody_paths["holdout-freeze-receipt.json"],
        "holdout freeze receipt",
        keys=R51_FREEZE_KEYS,
        canonical=True,
        mode=0o600,
    )
    if (
        freeze["contract"] != "hanonly-r51-encrypted-holdout-freeze-v1"
        or freeze["plan_revision"] != R51_PLAN_REVISION
        or freeze["entry_ids"] != R51_HOLDOUT_IDS
        or freeze["frozen_before_production_edit"] is not True
        or freeze["historical_inventory_sha256"] != inventory_hash
        or freeze["ciphertext_sha256"] != ciphertext_hash
        or freeze["ciphertext_byte_length"] != len(ciphertext_bytes)
        or freeze["header_sha256"] != _sha256(header_bytes)
        or freeze["disclosed_challenge_exclusion_pass"] is not True
        or freeze["result"] != "pass"
    ):
        raise LedgerError("holdout freeze receipt binding drift")
    if (
        header["contract"] != "hanonly-r51-encrypted-holdout-header-v1"
        or header["plan_revision"] != R51_PLAN_REVISION
        or header["cipher"] != "aes-256-ctr"
        or header["integrity"] != "hmac-sha256-etm-v1"
        or not re.fullmatch(r"[0-9a-f]{32}", header["iv_hex"] or "")
        or type(header["ciphertext_byte_length"]) is not int
        or header["ciphertext_byte_length"] != len(ciphertext_bytes)
        or type(header["plaintext_archive_byte_length"]) is not int
        or header["plaintext_archive_byte_length"] <= 0
        or freeze["cipher"] != header["cipher"]
        or freeze["integrity"] != header["integrity"]
        or freeze["iv_sha256"] != _sha256(bytes.fromhex(header["iv_hex"]))
    ):
        raise LedgerError("holdout header binding drift")
    for key in (
        "iv_sha256",
        "ciphertext_sha256",
        "header_sha256",
        "hmac_sha256",
        "plaintext_archive_sha256_commitment",
        "manifest_sha256_commitment",
        "oracle_sha256_commitment",
        "hashes_sha256_commitment",
        "historical_inventory_sha256",
    ):
        _validate_hash(freeze[key], f"freeze {key}")
    if (
        contract_hashes["r51_contract_sha256"]
        == contract_hashes["base_production_contract_sha256"]
    ):
        raise LedgerError(
            "R51 and base production contracts must be independently bound"
        )
    return {
        "freeze_path": freeze_path,
        "freeze_sha256": _sha256(freeze_bytes),
        "freeze": freeze,
        "header_path": header_path,
        "header_sha256": _sha256(header_bytes),
        "historical_path": historical_path,
        "historical_sha256": inventory_hash,
        "ciphertext_path": ciphertext_path,
        "ciphertext_sha256": ciphertext_hash,
    }


def _r51_clean_detached_head(repo_root, b0_sha):
    if not B0_SHA_RE.fullmatch(b0_sha):
        raise LedgerError("R51 B0 sha must be 40 lowercase hexadecimal characters")
    head = _run_git(repo_root, ["rev-parse", "HEAD"])
    if head.returncode != 0 or head.stdout.decode("ascii").strip() != b0_sha:
        raise LedgerError("R51 B0 sha does not equal HEAD")
    symbolic = _run_git(repo_root, ["symbolic-ref", "-q", "HEAD"])
    if symbolic.returncode == 0:
        raise LedgerError("R51 B0 preflight requires detached HEAD")
    status_result = _run_git(
        repo_root, ["status", "--porcelain=v1", "--untracked-files=all"]
    )
    if status_result.returncode != 0 or status_result.stdout:
        raise LedgerError("R51 B0 worktree must be clean")


def _r51_frozen_interpreter(repo_root, contract_path):
    paths = [
        contract_path,
        os.path.join(repo_root, B0_CHECKER_ENDPOINT),
        os.path.join(repo_root, "scripts/check-hanonly-production-policy.test.ts"),
        os.path.join(repo_root, "scripts/hanonly_evidence_ledger.py"),
        os.path.join(repo_root, "scripts/hanonly_evidence_ledger_test.py"),
        os.path.join(repo_root, "package.json"),
        os.path.join(repo_root, "ui/package.json"),
        os.path.join(repo_root, "bun.lock"),
    ]
    result = {}
    for item in paths:
        canonical, data = _r51_read(item, "frozen interpreter input", mode=None)
        key = (
            os.path.relpath(canonical, repo_root)
            if _is_beneath(canonical, repo_root)
            else "r51_contract"
        )
        result[key] = _sha256(data)
    return dict(sorted(result.items()))


def _r51_validate_staged_red_logs(root, hashes):
    directory = _canonical_existing_path(
        os.path.join(root, "r51-staged-red"),
        "R51 staged RED directory",
    )
    directory_stat = os.stat(directory)
    if (
        directory_stat.st_uid != os.geteuid()
        or not stat.S_ISDIR(directory_stat.st_mode)
        or _mode(directory_stat) != 0o700
    ):
        raise LedgerError("R51 staged RED directory is insecure")
    expected_names = {f"{test_id}.log" for test_id in hashes}
    if set(os.listdir(directory)) != expected_names:
        raise LedgerError("R51 staged RED log inventory drift")
    for test_id, digest in hashes.items():
        _, data = _r51_read(
            os.path.join(directory, f"{test_id}.log"),
            f"R51 staged RED {test_id} log",
            mode=0o600,
        )
        if _sha256(data) != digest:
            raise LedgerError(f"R51 staged RED {test_id} log hash drift")


def _r51_write_preflight(arguments):
    repo_root = _canonical_existing_path(arguments.repo_root, "repo root")
    _validate_repository(repo_root)
    _r51_clean_detached_head(repo_root, arguments.b0_sha)
    contract_hashes = _r51_validate_contract_files(arguments)
    frozen = _r51_validate_freeze(arguments, contract_hashes)
    if (
        frozen["freeze"]["implementation_thread_id"]
        != arguments.implementation_thread_id
    ):
        raise LedgerError("implementation thread binding drift")
    executable_path, executable_bytes = _r51_read(
        arguments.evidence_test_executable,
        "R51 evidence test executable",
        mode=None,
    )
    executable_stat = os.stat(executable_path)
    if executable_stat.st_uid != os.geteuid() or not executable_stat.st_mode & 0o111:
        raise LedgerError("R51 evidence test executable is not same-owner executable")
    target_dir = _canonical_existing_path(
        arguments.cargo_target_dir, "CARGO_TARGET_DIR"
    )
    if not _is_beneath(executable_path, target_dir) or executable_path == target_dir:
        raise LedgerError("R51 evidence test executable is outside CARGO_TARGET_DIR")
    _, gate_bytes, gate_results = _r51_json(
        arguments.gate_results,
        "R51 gate results",
        keys=R51_GATE_KEYS,
        canonical=True,
        mode=0o600,
    )
    if any(value != "pass" for value in gate_results.values()):
        raise LedgerError("R51 preflight gate did not pass")
    _, _, staged_red_hashes = _r51_json(
        arguments.staged_red_log,
        "staged RED hashes",
        canonical=True,
        mode=0o600,
    )
    expected_red_ids = set(EXPECTED_B0_B1_MARKER_IDS) | set(EXPECTED_GREEN_C_RED_IDS)
    if (
        not isinstance(staged_red_hashes, dict)
        or set(staged_red_hashes) != expected_red_ids
    ):
        raise LedgerError("R51 staged RED hash inventory drift")
    for test_id, digest in staged_red_hashes.items():
        _validate_hash(digest, f"R51 staged RED {test_id}")
    _r51_validate_staged_red_logs(
        os.path.dirname(arguments.staged_red_log),
        staged_red_hashes,
    )
    attestation = {
        "contract": "hanonly-r51-b0-preflight-v1",
        "plan_revision": R51_PLAN_REVISION,
        "b0_sha": arguments.b0_sha,
        "implementation_thread_id": _validate_text(
            arguments.implementation_thread_id, "implementation thread id"
        ),
        **{key: value for key, value in contract_hashes.items() if key != "paths"},
        "freeze_receipt_sha256": frozen["freeze_sha256"],
        "historical_inventory_sha256": frozen["historical_sha256"],
        "ciphertext_sha256": frozen["ciphertext_sha256"],
        "frozen_interpreter_sha256": _r51_frozen_interpreter(
            repo_root, contract_hashes["paths"][0]
        ),
        "evidence_test_executable_path": executable_path,
        "evidence_test_executable_sha256": _sha256(executable_bytes),
        "evidence_enabled_cargo_features": R51_FEATURES,
        "gate_results": gate_results,
        "staged_red_log_sha256": staged_red_hashes,
        "result": "pass",
    }
    _require_keys(attestation, R51_PREFLIGHT_KEYS, "R51 B0 preflight attestation")
    path, digest = _r51_publish(
        arguments.output, attestation, "R51 B0 preflight attestation"
    )
    return _r51_canonical_json({"path": path, "sha256": digest}) + b"\n"


def _r51_validate_attestation(path, phase, b0_sha, checker_sha256):
    path, data, value = _r51_json(
        path,
        f"R51 {phase} attestation",
        keys=B0_ATTESTATION_KEYS,
        canonical=False,
        mode=0o600,
    )
    if canonical_json(value) != data:
        raise LedgerError(f"R51 {phase} attestation is not canonical R49 JSON")
    if (
        value["version"] != 1
        or value["mode"] != "b0-source-gate-anti-fixture"
        or value["phase"] != phase
        or value["b0_sha"] != b0_sha
        or value["checker_endpoint_sha256"] != checker_sha256
        or value["scanned_roots"] != B0_ANTI_FIXTURE_SCANNED_ROOTS
        or value["allowed_descriptor_roots"] != B0_ANTI_FIXTURE_ALLOWED_DESCRIPTOR_ROOTS
        or value["result"] != "pass"
    ):
        raise LedgerError(f"R51 {phase} attestation drift")
    _validate_hash(value["policy_scan_sha256"], f"R51 {phase} policy scan")
    return path, _sha256(data), value


def _r51_result_passed(result):
    if not isinstance(result, dict):
        raise LedgerError("R51 result must be an object")
    if result.get("result") in {"pass", "passed"} or result.get("state") == "passed":
        return True
    if (
        result.get("result") in {"fail", "failed", "fail-closed"}
        or result.get("state") == "failed"
    ):
        return False
    derived = result.get("derived")
    if isinstance(derived, dict) and type(derived.get("passed")) is bool:
        return derived["passed"]
    if type(result.get("passed")) is bool:
        return result["passed"]
    raise LedgerError("R51 result has no terminal pass/fail state")


def _r51_validate_process_evidence(process, phase, device, evidence_root):
    _require_keys(process, B0_PROCESS_KEYS, "R51 process evidence")
    if process["phase"] != phase or process["requested_device"] != device:
        raise LedgerError("R51 process phase or requested device drift")
    if not re.fullmatch(r"[0-9a-f]{32}", process["paddle_instance_id"] or ""):
        raise LedgerError("R51 process paddle instance id drift")
    _validate_hash(process["executable_sha256"], "R51 process executable")
    _require_keys(
        process["model_artifact_sha256"], B0_MODEL_HASH_KEYS, "R51 model hashes"
    )
    for digest in process["model_artifact_sha256"].values():
        _validate_hash(digest, "R51 model artifact")
    libraries = process["runtime_library_sha256"]
    if not isinstance(libraries, dict) or not libraries:
        raise LedgerError("R51 runtime library hashes must be nonempty")
    for library, digest in libraries.items():
        _validate_text(library, "R51 runtime library path")
        _validate_hash(digest, "R51 runtime library")
    load = process["load_evidence"]
    _require_keys(load, B0_LOAD_KEYS, "R51 load evidence")
    _validate_b0_raw_log(
        evidence_root,
        load["raw_load_log_relpath"],
        load["raw_load_log_sha256"],
        "R51 raw load log",
    )
    loaded = load["loaded_model_devices"]
    if not isinstance(loaded, list) or not loaded:
        raise LedgerError("R51 loaded model devices must be nonempty")
    for ordinal, item in enumerate(loaded):
        _require_keys(item, B0_LOADED_DEVICE_KEYS, "R51 loaded model device")
        if item["model_device_ordinal"] != ordinal:
            raise LedgerError("R51 loaded model device ordinal drift")
    model_map = load["model_buffer_bytes_by_backend"]
    if device == "cpu":
        if (
            load["cpu_forced"] is not True
            or load["n_gpu_layers"] != 0
            or load["mtmd_use_gpu"] is not False
            or load["offloaded_layers"] != 0
            or load["mtmd_backend"] != "CPU"
            or any(item["backend"] != "CPU" for item in loaded)
        ):
            raise LedgerError("R51 CPU process evidence drift")
        _validate_backend_map(model_map, "R51 CPU model map", "CPU")
        if any(size > 0 for backend, size in model_map.items() if backend != "CPU"):
            raise LedgerError("R51 CPU process contains non-CPU model bytes")
    else:
        if (
            load["gpu_offload_supported"] is not True
            or type(load["n_gpu_layers"]) is not int
            or load["n_gpu_layers"] <= 0
            or load["mtmd_use_gpu"] is not True
            or type(load["offloaded_layers"]) is not int
            or load["offloaded_layers"] <= 0
            or load["mtmd_backend"] != "Metal"
            or not any(item["backend"] == "Metal" for item in loaded)
            or any(item["backend"] not in {"CPU", "Metal"} for item in loaded)
        ):
            raise LedgerError("R51 actual Metal process evidence drift")
        _validate_backend_map(model_map, "R51 Metal model map", "Metal")


def _r51_validate_calibration(payload, calibration_ledger, evidence_root=None):
    results = payload["calibration_results"]
    if not isinstance(results, list) or len(results) != 32:
        raise LedgerError("R51 calibration must contain exactly 32 terminal cells")
    if not isinstance(calibration_ledger, dict):
        raise LedgerError("R51 calibration ledger must be an object")
    if (
        calibration_ledger.get("calibration_entry_ids") != R51_CALIBRATION_IDS
        or calibration_ledger.get("candidates") != B0_CANDIDATES
        or calibration_ledger.get("calibration_results") != results
        or calibration_ledger.get("selected_candidate_id")
        != payload["selected_candidate_id"]
    ):
        raise LedgerError("R51 calibration ledger binding drift")
    process_list = calibration_ledger.get("process_evidence")
    if not isinstance(process_list, list) or len(process_list) != 2:
        raise LedgerError("R51 calibration requires exact CPU and Metal processes")
    processes = {}
    evidence_root = evidence_root or os.getcwd()
    for process in process_list:
        process_id = process.get("id") if isinstance(process, dict) else None
        if not isinstance(process_id, str) or process_id in processes:
            raise LedgerError("R51 calibration process id drift")
        device = process.get("requested_device")
        if device not in {"cpu", "metal"}:
            raise LedgerError("R51 calibration process device drift")
        _r51_validate_process_evidence(process, "calibration", device, evidence_root)
        processes[process_id] = process
    if {process["requested_device"] for process in processes.values()} != {
        "cpu",
        "metal",
    }:
        raise LedgerError("R51 calibration process matrix is incomplete")
    seen = set()
    pass_by_candidate = {candidate["id"]: True for candidate in B0_CANDIDATES}
    for result in results:
        entry_id = result.get("entry_id")
        candidate_id = result.get("candidate_id")
        process = processes.get(result.get("process_evidence_id"))
        device = process.get("requested_device") if process else None
        cell = (entry_id, device, candidate_id)
        if (
            entry_id not in R51_CALIBRATION_IDS
            or device not in {"cpu", "metal"}
            or candidate_id not in pass_by_candidate
            or cell in seen
        ):
            raise LedgerError("R51 calibration cell identity drift")
        _validate_result(
            result,
            processes,
            R51_CALIBRATION_IDS,
            {candidate["id"] for candidate in B0_CANDIDATES},
            "calibration",
            evidence_root,
            allow_detector_support_coverage=True,
        )
        seen.add(cell)
        pass_by_candidate[candidate_id] &= result["derived"]["passed"]
    expected = {
        (entry, device, candidate["id"])
        for entry in R51_CALIBRATION_IDS
        for device in ("cpu", "metal")
        for candidate in B0_CANDIDATES
    }
    if seen != expected:
        raise LedgerError("R51 calibration matrix is incomplete")
    selected = next(
        (
            candidate["id"]
            for candidate in B0_CANDIDATES
            if pass_by_candidate[candidate["id"]]
        ),
        None,
    )
    if selected != payload["selected_candidate_id"]:
        raise LedgerError("R51 selected candidate is not the first all-pass candidate")


def _r51_validate_terminal(value, bindings):
    _require_keys(value, R51_TERMINAL_KEYS, "R51 terminal receipt")
    expected_cells = [
        f"{entry}/{device}" for entry in R51_HOLDOUT_IDS for device in ("cpu", "metal")
    ]
    cells = value["cell_results"]
    if not isinstance(cells, list) or len(cells) != 8:
        raise LedgerError("R51 terminal receipt must contain eight cells")
    for cell, expected_key in zip(cells, expected_cells):
        _require_keys(cell, R51_TERMINAL_CELL_KEYS, "R51 terminal cell")
        if (
            cell["cell_key"] != expected_key
            or cell["result"] != "pass"
            or cell["selection_result"] != "selected"
        ):
            raise LedgerError("R51 terminal cell did not pass")
        for key in (
            "device_evidence_sha256",
            "log_sha256",
            "diagnostic_sha256",
            "target_coverage_index_sha256",
        ):
            _validate_hash(cell[key], f"terminal cell {key}")
    if (
        value["contract"] != "hanonly-r51-encrypted-holdout-terminal-v1"
        or value["plan_revision"] != R51_PLAN_REVISION
        or value["b0_sha"] != bindings["b0_sha"]
        or value["selected_candidate_id"] != bindings["selected_candidate_id"]
        or value["freeze_receipt_sha256"] != bindings["freeze_receipt_sha256"]
        or value["open_marker_sha256"] != bindings["open_marker_sha256"]
        or value["ciphertext_sha256"] != bindings["ciphertext_sha256"]
        or value["pre_holdout_attestation_sha256"]
        != bindings["pre_holdout_attestation_sha256"]
        or value["bundle_validation_receipt_sha256"]
        != bindings["bundle_validation_receipt_sha256"]
        or value["first_failed_cell"] is not None
        or value["unexecuted_cell_keys"] != []
        or value["all_cells_terminated"] is not True
        or value["all_cells_passed"] is not True
        or value["plaintext_removed"] is not True
        or value["result"] != "pass"
    ):
        raise LedgerError("R51 terminal receipt binding drift")
    return cells


def _r51_bound_relative(root, relpath, expected_sha256, expected_length, label):
    if (
        not isinstance(relpath, str)
        or not relpath
        or os.path.isabs(relpath)
        or "\\" in relpath
        or os.path.normpath(relpath) != relpath
        or relpath.startswith("../")
    ):
        raise LedgerError(f"{label} relative path is invalid")
    _validate_hash(expected_sha256, f"{label} sha256")
    if type(expected_length) is not int or expected_length < 0:
        raise LedgerError(f"{label} byte length is invalid")
    path, data = _r51_read(os.path.join(root, relpath), label, mode=0o600)
    if (
        not _is_beneath(path, root)
        or len(data) != expected_length
        or _sha256(data) != expected_sha256
    ):
        raise LedgerError(f"{label} binding drift")
    return path, data


def _r51_validate_target_recall(value, label):
    _require_keys(value, R51_TARGET_RECALL_KEYS, label)
    if any(type(value[key]) is not int or value[key] < 0 for key in value):
        raise LedgerError(f"{label} values are invalid")
    if (
        value["selected"] > value["target_total"]
        or value["covered"] > value["selected"]
        or value["uncovered"] != value["target_total"] - value["covered"]
    ):
        raise LedgerError(f"{label} arithmetic drift")


def _r51_validate_support_raster(
    root, relpath, expected_sha256, expected_length, width, height, label
):
    _, data = _r51_bound_relative(
        root, relpath, expected_sha256, expected_length, label
    )
    if (
        type(width) is not int
        or type(height) is not int
        or width <= 0
        or height <= 0
        or len(data) != width * height
        or any(value not in (0, 1) for value in data)
    ):
        raise LedgerError(f"{label} raster encoding drift")
    return sum(data)


def _r51_register_bound_path(seen_paths, path, label):
    if path in seen_paths:
        raise LedgerError(f"{label} reuses an already bound diagnostic path")
    seen_paths.add(path)


def _r51_rect(value, label):
    if (
        not isinstance(value, list)
        or len(value) != 4
        or any(type(coordinate) is not int for coordinate in value)
    ):
        raise LedgerError(f"{label} rectangle drift")
    return value


def _r51_rect_quad(rect):
    left, top, right, bottom = rect
    return [left, top, right, top, right, bottom, left, bottom]


def _r51_rect_mask_sha256(width, height, rect):
    left, top, right, bottom = rect
    empty = bytes(width)
    filled = bytes(left) + bytes([1]) * (right - left) + bytes(width - right)
    digest = hashlib.sha256()
    for row in range(height):
        digest.update(filled if top <= row < bottom else empty)
    return digest.hexdigest()


def _r51_validate_mask_descriptor(value, rect, label):
    _require_keys(value, R51_MASK_DESCRIPTOR_KEYS, label)
    width = value["width"]
    height = value["height"]
    if (
        type(width) is not int
        or type(height) is not int
        or width <= 0
        or height <= 0
        or value["stride"] != width
        or value["pixel_encoding"] != "u8-binary"
        or value["row_order"] != "top-to-bottom-left-to-right"
        or not (
            0 <= rect[0] <= rect[2] <= width
            and 0 <= rect[1] <= rect[3] <= height
        )
        or value["bytes_sha256"] != _r51_rect_mask_sha256(width, height, rect)
    ):
        raise LedgerError(f"{label} descriptor drift")
    return width, height


def _r51_validate_detector_support_preimage(preimage, expected_recognition):
    raw = preimage["raw_detector"]
    detector_rect = _r51_rect(raw["rect"], "R51 detector support")
    if (
        type(raw["recognition_present"]) is not bool
        or raw["recognition_class"]
        not in {"han", "neutral", "protected_latin", "ambiguous_latin", "missing"}
        or (raw["recognition_present"], raw["recognition_class"])
        != expected_recognition
        or preimage["emitted_scene_quad"] != _r51_rect_quad(detector_rect)
    ):
        raise LedgerError("R51 emitted detector geometry drift")
    eligible_quad = preimage["eligible_text_line_quad"]
    if eligible_quad is None:
        line_rect = [0, 0, 0, 0]
    else:
        if not isinstance(eligible_quad, list) or len(eligible_quad) != 8:
            raise LedgerError("R51 eligible line geometry drift")
        line_rect = _r51_rect(
            [eligible_quad[0], eligible_quad[1], eligible_quad[4], eligible_quad[5]],
            "R51 eligible line",
        )
        if eligible_quad != _r51_rect_quad(line_rect):
            raise LedgerError("R51 eligible line geometry drift")
    detector_dimensions = _r51_validate_mask_descriptor(
        preimage["detector_support_mask"],
        detector_rect,
        "R51 detector support mask",
    )
    line_dimensions = _r51_validate_mask_descriptor(
        preimage["line_support_mask"],
        line_rect,
        "R51 line support mask",
    )
    intersection = [
        max(detector_rect[0], line_rect[0]),
        max(detector_rect[1], line_rect[1]),
        min(detector_rect[2], line_rect[2]),
        min(detector_rect[3], line_rect[3]),
    ]
    if intersection[0] > intersection[2] or intersection[1] > intersection[3]:
        intersection = [0, 0, 0, 0]
    agreed_dimensions = _r51_validate_mask_descriptor(
        preimage["agreed_mask"], intersection, "R51 agreed mask"
    )
    if not detector_dimensions == line_dimensions == agreed_dimensions:
        raise LedgerError("R51 detector support mask dimensions drift")

    assignment = preimage["canonical_assignment"]
    common_safe = (
        preimage["line_support_equals_detector"] is (detector_rect == line_rect)
        and preimage["agreed_mask_subset"] is True
        and preimage["unsupported_rotation_selected"] is False
        and preimage["unmatched_selected_nodes"] == []
        and preimage["ownership_verdict"] == "unique"
    )
    if assignment == "selected_han":
        valid = (
            common_safe
            and isinstance(preimage["target_id"], str)
            and bool(preimage["target_id"])
            and type(preimage["protected_support_pixels"]) is int
            and preimage["protected_support_pixels"] == 0
            and preimage["selection_verdict"] == "selected"
            and preimage["rejection_reason"] is None
        )
    elif assignment == "preserved_source":
        valid = (
            common_safe
            and preimage["target_id"] is None
            and type(preimage["protected_support_pixels"]) is int
            and preimage["protected_support_pixels"] >= 0
            and preimage["selection_verdict"] == "preserved"
            and preimage["rejection_reason"] is None
        )
    elif assignment == "unassigned":
        valid = (
            preimage["target_id"] is None
            and eligible_quad is None
            and preimage["line_support_equals_detector"] is False
            and preimage["agreed_mask_subset"] is True
            and type(preimage["protected_support_pixels"]) is int
            and preimage["protected_support_pixels"] >= 0
            and preimage["unsupported_rotation_selected"] is False
            and preimage["unmatched_selected_nodes"] == []
            and preimage["ownership_verdict"] == "unassigned"
            and preimage["selection_verdict"] == "rejected"
            and isinstance(preimage["rejection_reason"], str)
            and bool(preimage["rejection_reason"])
        )
    else:
        valid = False
    if not valid:
        raise LedgerError("R51 detector support safety invariant failed")


def _r51_validate_coverage_index(root, record, bindings, target_total, seen_paths):
    _, index_bytes = _r51_bound_relative(
        root,
        record["target_coverage_index_path"],
        record["target_coverage_index_sha256"],
        record["target_coverage_index_byte_length"],
        "R51 target coverage index",
    )
    _r51_register_bound_path(
        seen_paths, record["target_coverage_index_path"], "R51 target coverage index"
    )
    index = _parse_json(index_bytes, "R51 target coverage index")
    _require_keys(index, R51_COVERAGE_INDEX_KEYS, "R51 target coverage index")
    if _r51_canonical_json(index) != index_bytes:
        raise LedgerError("R51 target coverage index is not canonical")
    records = index["records"]
    if (
        index["contract"] != "hanonly-r51-target-coverage-index-v1"
        or index["plan_revision"] != R51_PLAN_REVISION
        or index["b0_sha"] != bindings["b0_sha"]
        or index["cell_key"] != record["cell_key"]
        or index["manifest_sha256"] != bindings["manifest_sha256"]
        or index["oracle_sha256"] != bindings["oracle_sha256"]
        or index["hashes_sha256"] != bindings["hashes_sha256"]
        or not isinstance(records, list)
        or len(records) != target_total
    ):
        raise LedgerError("R51 target coverage index binding drift")
    identities = []
    for item in records:
        _require_keys(item, R51_COVERAGE_INDEX_RECORD_KEYS, "R51 coverage record")
        if (
            item["entry_id"] != record["entry_id"]
            or not isinstance(item["target_id"], str)
            or not item["target_id"]
        ):
            raise LedgerError("R51 coverage record target identity drift")
        identities.append((item["entry_id"], item["target_id"]))
        _, proof_bytes = _r51_bound_relative(
            root,
            item["proof_path"],
            item["proof_sha256"],
            item["proof_byte_length"],
            "R51 target coverage proof",
        )
        _r51_register_bound_path(
            seen_paths, item["proof_path"], "R51 target coverage proof"
        )
        proof = _parse_json(proof_bytes, "R51 target coverage proof")
        _require_keys(proof, R51_COVERAGE_PROOF_KEYS, "R51 target coverage proof")
        if _r51_canonical_json(proof) != proof_bytes:
            raise LedgerError("R51 target coverage proof is not canonical")
        width = proof["page_width"]
        height = proof["page_height"]
        if (
            proof["contract"] != "hanonly-r51-target-coverage-proof-v1"
            or proof["plan_revision"] != R51_PLAN_REVISION
            or proof["b0_sha"] != bindings["b0_sha"]
            or proof["cell_key"] != record["cell_key"]
            or proof["entry_id"] != item["entry_id"]
            or proof["target_id"] != item["target_id"]
            or proof["support_stride_bytes"] != width
        ):
            raise LedgerError("R51 target coverage proof binding drift")
        for key in ("oracle_mask_raw_sha256", "oracle_mask_normalized_sha256"):
            _validate_hash(proof[key], f"R51 target coverage proof {key}")
        selected_foreground = _r51_validate_support_raster(
            root,
            proof["selected_support_relpath"],
            proof["selected_support_sha256"],
            proof["selected_support_byte_length"],
            width,
            height,
            "R51 selected support",
        )
        _r51_register_bound_path(
            seen_paths,
            proof["selected_support_relpath"],
            "R51 selected support",
        )
        downstream_foreground = _r51_validate_support_raster(
            root,
            proof["downstream_support_relpath"],
            proof["downstream_support_sha256"],
            proof["downstream_support_byte_length"],
            width,
            height,
            "R51 downstream support",
        )
        _r51_register_bound_path(
            seen_paths,
            proof["downstream_support_relpath"],
            "R51 downstream support",
        )
        oracle_foreground = proof["oracle_foreground_pixels"]
        if (
            any(
                type(proof[key]) is not int or proof[key] < 0
                for key in (
                    "oracle_foreground_pixels",
                    "selected_support_foreground_pixels",
                    "downstream_support_foreground_pixels",
                    "selected_covered_pixels",
                    "downstream_covered_pixels",
                    "missing_selected_pixels",
                    "missing_downstream_pixels",
                    "protected_overlap_pixels",
                )
            )
            or oracle_foreground <= 0
            or oracle_foreground > width * height
            or proof["selected_support_foreground_pixels"] != selected_foreground
            or proof["downstream_support_foreground_pixels"] != downstream_foreground
            or selected_foreground < oracle_foreground
            or downstream_foreground < oracle_foreground
            or proof["selected_covered_pixels"] != oracle_foreground
            or proof["downstream_covered_pixels"] != oracle_foreground
            or proof["missing_selected_pixels"] != 0
            or proof["missing_downstream_pixels"] != 0
            or proof["protected_overlap_pixels"] != 0
            or proof["target_selected"] is not True
            or proof["result"] != "pass"
        ):
            raise LedgerError("R51 target coverage proof did not pass")
    if identities != sorted(identities) or len(set(identities)) != len(identities):
        raise LedgerError("R51 target coverage proof identity drift")


def _r51_validate_cell_diagnostic(root, record, bindings, seen_paths, *, holdout):
    _, diagnostic_bytes = _r51_bound_relative(
        root,
        record["diagnostic_path"],
        record["diagnostic_sha256"],
        record["diagnostic_byte_length"],
        "R51 cell diagnostic",
    )
    _r51_register_bound_path(
        seen_paths, record["diagnostic_path"], "R51 cell diagnostic"
    )
    diagnostic = _parse_json(diagnostic_bytes, "R51 cell diagnostic")
    _require_keys(diagnostic, R51_CELL_DIAGNOSTIC_KEYS, "R51 cell diagnostic")
    if _r51_canonical_json(diagnostic) != diagnostic_bytes:
        raise LedgerError("R51 cell diagnostic is not canonical")
    recall = diagnostic["target_recall"]
    _r51_validate_target_recall(recall, "R51 cell diagnostic target recall")
    if (
        diagnostic["contract"] != "hanonly-r50-cell-diagnostic-v1"
        or diagnostic["plan_revision"] != R51_PLAN_REVISION
        or diagnostic["b0_sha"] != bindings["b0_sha"]
        or diagnostic["calibration_manifest_sha256"]
        != bindings["calibration_manifest_sha256"]
        or diagnostic["holdout_manifest_sha256"]
        != (bindings["manifest_sha256"] if holdout else None)
        or diagnostic["fixture_manifest_sha256"] != bindings["fixture_manifest_sha256"]
        or diagnostic["phase"] != record["phase"]
        or diagnostic["entry_id"] != record["entry_id"]
        or diagnostic["device"] != record["device"]
        or diagnostic["candidate_id"] != record["candidate_id"]
        or diagnostic["state"] != record["state"]
        or diagnostic["selection_result"] != record["selection_result"]
        or diagnostic["rejection_reason"] != record["rejection_reason"]
        or diagnostic["target_recall"] != record["target_recall"]
        or diagnostic["pp_han_count"] != record["pp_han_count"]
        or diagnostic["vl_han_count"] != record["vl_han_count"]
        or diagnostic["bundle_validation_receipt_sha256"]
        != (bindings["bundle_sha256"] if holdout else None)
        or diagnostic["target_coverage_index_sha256"]
        != (record["target_coverage_index_sha256"] if holdout else None)
        or diagnostic["terminal_reason"] != record["terminal_reason"]
        or type(diagnostic["pp_han_count"]) is not int
        or diagnostic["pp_han_count"] < 0
        or type(diagnostic["vl_han_count"]) is not int
        or diagnostic["vl_han_count"] < 0
    ):
        raise LedgerError("R51 cell diagnostic binding drift")
    if holdout:
        if (
            record["state"] != "passed"
            or record["selection_result"] != "selected"
            or record["rejection_reason"] is not None
            or record["terminal_reason"] is not None
            or recall["selected"] != recall["target_total"]
            or recall["covered"] != recall["target_total"]
            or recall["uncovered"] != 0
            or recall["target_total"] <= 0
        ):
            raise LedgerError("R51 holdout cell diagnostic did not pass")
    elif any(
        record[key] is not None
        for key in (
            "target_coverage_index_path",
            "target_coverage_index_sha256",
            "target_coverage_index_byte_length",
        )
    ):
        raise LedgerError("R51 calibration diagnostic contains holdout coverage")
    raw = diagnostic["raw_detector_outputs"]
    canonical = diagnostic["canonical_lines"]
    raw_count = diagnostic["raw_detector_count"]
    if (
        not isinstance(raw, list)
        or type(raw_count) is not int
        or raw_count < 0
        or raw_count != len(raw)
    ):
        raise LedgerError("R51 raw detector count drift")
    raw_bits = []
    for index, occurrence in enumerate(raw):
        if (
            set(occurrence) != {"occurrence_index", "source_scaled_quad_f32_bits"}
            or occurrence["occurrence_index"] != index
            or not isinstance(occurrence["source_scaled_quad_f32_bits"], list)
            or len(occurrence["source_scaled_quad_f32_bits"]) != 8
            or any(
                type(value) is not int or not 0 <= value <= 0xFFFFFFFF
                for value in occurrence["source_scaled_quad_f32_bits"]
            )
        ):
            raise LedgerError("R51 raw detector occurrence drift")
        raw_bits.append(occurrence["source_scaled_quad_f32_bits"])
    if (
        _sha256(_r51_canonical_json(raw_bits))
        != diagnostic["raw_detector_f32_bits_multiset_sha256"]
    ):
        raise LedgerError("R51 raw detector multiset hash drift")
    flattened = []
    canonical_recognition = {}
    if not isinstance(canonical, list):
        raise LedgerError("R51 canonical lines must be an array")
    for line_index, line in enumerate(canonical):
        if (
            set(line) != {"line_index", "detector_occurrences", "recognition"}
            or line["line_index"] != line_index
            or not isinstance(line["detector_occurrences"], list)
        ):
            raise LedgerError("R51 canonical line drift")
        for occurrence in line["detector_occurrences"]:
            index = occurrence.get("occurrence_index")
            if (
                set(occurrence)
                != {
                    "occurrence_index",
                    "canonical_corners_f32_bits",
                }
                or type(index) is not int
                or not 0 <= index < raw_count
                or occurrence["canonical_corners_f32_bits"] != raw_bits[index]
            ):
                raise LedgerError("R51 canonical detector assignment drift")
            flattened.append(index)
        recognition = line["recognition"]
        if recognition is not None and (
            not isinstance(recognition, dict)
            or set(recognition) != {"present", "recognition_class", "segment_count"}
            or recognition["present"] is not True
            or recognition["recognition_class"]
            not in {"han", "neutral", "protected_latin", "ambiguous_latin"}
            or type(recognition["segment_count"]) is not int
            or recognition["segment_count"] < 0
        ):
            raise LedgerError("R51 canonical recognition drift")
        expected_recognition = (
            (False, "missing")
            if recognition is None
            else (True, recognition["recognition_class"])
        )
        for occurrence in line["detector_occurrences"]:
            canonical_recognition[occurrence["occurrence_index"]] = expected_recognition
    if sorted(flattened) != list(range(raw_count)):
        raise LedgerError("R51 canonical detector completeness drift")
    support_records = diagnostic["detector_support_records"]
    if not isinstance(support_records, list) or len(support_records) != raw_count:
        raise LedgerError("R51 detector support record completeness drift")
    support_indices = []
    for support_record in support_records:
        _require_keys(
            support_record,
            R51_DETECTOR_SUPPORT_RECORD_KEYS,
            "R51 detector support record",
        )
        preimage = support_record["preimage"]
        _require_keys(
            preimage,
            R51_DETECTOR_SUPPORT_PREIMAGE_KEYS,
            "R51 detector support preimage",
        )
        preimage_bytes = _r51_canonical_json(preimage)
        raw_detector = preimage["raw_detector"]
        _require_keys(raw_detector, R51_RAW_DETECTOR_KEYS, "R51 support raw detector")
        detector_index = raw_detector["index"]
        if (
            support_record["canonical_byte_length"] != len(preimage_bytes)
            or support_record["sha256"] != _sha256(preimage_bytes)
            or preimage["contract"] != "detector-support-raster-preimage-v1"
            or preimage["plan_revision"] != R51_PLAN_REVISION
            or preimage["b0_sha"] != bindings["b0_sha"]
            or preimage["phase"] != record["phase"]
            or preimage["entry_id"] != record["entry_id"]
            or preimage["device"] != record["device"]
            or preimage["candidate_id"] != record["candidate_id"]
            or type(detector_index) is not int
            or not 0 <= detector_index < raw_count
            or raw_detector["source_scaled_quad_f32_bits"] != raw_bits[detector_index]
        ):
            raise LedgerError("R51 detector support preimage binding drift")
        _r51_validate_detector_support_preimage(
            preimage, canonical_recognition[detector_index]
        )
        support_indices.append(detector_index)
    if support_indices != list(range(raw_count)):
        raise LedgerError("R51 detector support record order drift")
    _r51_bound_relative(
        root,
        record["device_evidence_path"],
        record["device_evidence_sha256"],
        record["device_evidence_byte_length"],
        "R51 device evidence",
    )
    _r51_register_bound_path(
        seen_paths, record["device_evidence_path"], "R51 device evidence"
    )
    _r51_bound_relative(
        root,
        record["log_path"],
        record["log_sha256"],
        record["log_byte_length"],
        "R51 cell log",
    )
    _r51_register_bound_path(seen_paths, record["log_path"], "R51 cell log")
    if (
        diagnostic["device_evidence_sha256"] != record["device_evidence_sha256"]
        or diagnostic["device_evidence_byte_length"]
        != record["device_evidence_byte_length"]
        or diagnostic["log_sha256"] != record["log_sha256"]
        or diagnostic["log_byte_length"] != record["log_byte_length"]
    ):
        raise LedgerError("R51 cell diagnostic file binding drift")
    if holdout:
        _r51_validate_coverage_index(
            root, record, bindings, recall["target_total"], seen_paths
        )


def _r51_validate_generation_continuity(generation, records):
    expected_count = (generation + 1) // 2
    calibration_count = sum(item["phase"] == "calibration-freeze" for item in records)
    holdout_count = sum(item["phase"] == "holdout" for item in records)
    captured_count = sum(item["state"] == "captured_unclassified" for item in records)
    contains_holdout = holdout_count > 0
    if (
        len(records) != expected_count
        or calibration_count != min(expected_count, 32)
        or holdout_count != max(0, expected_count - 32)
        or captured_count != generation % 2
        or contains_holdout is not (generation >= 65)
    ):
        raise LedgerError("R51 diagnostic phase continuity drift")


def _r51_validate_terminal_diagnostic_index(
    path,
    b0_sha,
    selected_candidate_id,
    calibration_manifest_sha256,
    fixture_manifest_sha256,
    calibration_ledger,
    bundle_path,
    bundle_sha256,
    bundle,
):
    path, data, index = _r51_json(
        path,
        "R51 terminal diagnostic index",
        keys=R51_DIAGNOSTIC_INDEX_KEYS,
        canonical=True,
        mode=0o600,
    )
    if os.path.basename(path) != "diagnostic-index.json":
        raise LedgerError("R51 terminal diagnostic index basename drift")
    root = os.path.dirname(path)
    _r51_scan_diagnostic_tree(root)
    records = index["records"]
    calibration_results = calibration_ledger.get("calibration_results")
    process_list = calibration_ledger.get("process_evidence")
    if (
        not isinstance(calibration_results, list)
        or len(calibration_results) != 32
        or not isinstance(process_list, list)
    ):
        raise LedgerError("R51 calibration diagnostic matrix drift")
    if any(
        not isinstance(process, dict)
        or not isinstance(process.get("id"), str)
        or process.get("requested_device") not in {"cpu", "metal"}
        for process in process_list
    ):
        raise LedgerError("R51 calibration diagnostic process drift")
    process_devices = {
        process["id"]: process["requested_device"] for process in process_list
    }
    calibration_by_cell = {}
    for result in calibration_results:
        if (
            not isinstance(result, dict)
            or not isinstance(result.get("entry_id"), str)
            or not isinstance(result.get("candidate_id"), str)
            or not isinstance(result.get("process_evidence_id"), str)
            or not isinstance(result.get("derived"), dict)
            or type(result["derived"].get("passed")) is not bool
        ):
            raise LedgerError("R51 calibration diagnostic result drift")
        device = process_devices.get(result["process_evidence_id"])
        key = (
            f"calibration-freeze/{result['candidate_id']}/{device}/{result['entry_id']}"
        )
        if device is None or key in calibration_by_cell:
            raise LedgerError("R51 calibration diagnostic identity drift")
        calibration_by_cell[key] = result
    expected = sorted(
        [
            (
                key,
                "calibration-freeze",
                result["entry_id"],
                process_devices[result["process_evidence_id"]],
                result["candidate_id"],
            )
            for key, result in calibration_by_cell.items()
        ]
        + [
            (
                f"holdout/{selected_candidate_id}/{device}/{entry}",
                "holdout",
                entry,
                device,
                selected_candidate_id,
            )
            for entry in R51_HOLDOUT_IDS
            for device in ("cpu", "metal")
        ]
    )
    expected_identity = {
        cell_key: (phase, entry_id, device, candidate_id)
        for cell_key, phase, entry_id, device, candidate_id in expected
    }
    if (
        index["contract"] != "hanonly-r50-diagnostic-index-v1"
        or index["plan_revision"] != R51_PLAN_REVISION
        or index["b0_sha"] != b0_sha
        or index["calibration_manifest_sha256"] != calibration_manifest_sha256
        or index["holdout_manifest_sha256"] != bundle["manifest_sha256"]
        or index["fixture_manifest_sha256"] != fixture_manifest_sha256
        or index["generation"] != 80
        or index["expected_cell_count"] != 40
        or not isinstance(records, list)
        or len(records) != 40
        or index["bundle_validation_receipt_sha256"] != bundle_sha256
    ):
        raise LedgerError("R51 terminal diagnostic index binding drift")
    bundle_relpath = os.path.relpath(bundle_path, root)
    seen_paths = {bundle_relpath}
    _, indexed_bundle_bytes = _r51_bound_relative(
        root,
        index["bundle_validation_receipt_path"],
        index["bundle_validation_receipt_sha256"],
        index["bundle_validation_receipt_byte_length"],
        "R51 indexed bundle validation receipt",
    )
    if index["bundle_validation_receipt_path"] != bundle_relpath:
        raise LedgerError("R51 bundle validation receipt path drift")

    def validate_generation(value, generation):
        generation_records = value["records"]
        if not isinstance(generation_records, list):
            raise LedgerError("R51 diagnostic generation records drift")
        contains_holdout = any(
            isinstance(item, dict) and item.get("phase") == "holdout"
            for item in generation_records
        )
        expected_holdout_manifest = (
            bundle["manifest_sha256"] if contains_holdout else None
        )
        expected_bundle_path = bundle_relpath if contains_holdout else None
        expected_bundle_sha256 = bundle_sha256 if contains_holdout else None
        expected_bundle_length = len(indexed_bundle_bytes) if contains_holdout else None
        if (
            value["contract"] != "hanonly-r50-diagnostic-index-v1"
            or value["plan_revision"] != R51_PLAN_REVISION
            or value["b0_sha"] != b0_sha
            or value["calibration_manifest_sha256"]
            != index["calibration_manifest_sha256"]
            or value["holdout_manifest_sha256"] != expected_holdout_manifest
            or value["fixture_manifest_sha256"] != index["fixture_manifest_sha256"]
            or value["generation"] != generation
            or value["expected_cell_count"] != len(generation_records)
            or value["bundle_validation_receipt_path"] != expected_bundle_path
            or value["bundle_validation_receipt_sha256"] != expected_bundle_sha256
            or value["bundle_validation_receipt_byte_length"] != expected_bundle_length
        ):
            raise LedgerError("R51 diagnostic generation binding drift")
        keys = []
        for item in generation_records:
            _require_keys(item, R51_DIAGNOSTIC_RECORD_KEYS, "R51 diagnostic record")
            key = item["cell_key"]
            identity = expected_identity.get(key)
            if (
                identity is None
                or (
                    item["phase"],
                    item["entry_id"],
                    item["device"],
                    item["candidate_id"],
                )
                != identity
                or item["state"]
                not in {
                    "captured_unclassified",
                    "passed",
                    "failed",
                }
            ):
                raise LedgerError("R51 diagnostic record cell key drift")
            keys.append(key)
        if keys != sorted(keys) or len(keys) != len(set(keys)):
            raise LedgerError("R51 diagnostic record order drift")
        _r51_validate_generation_continuity(generation, generation_records)

    current = index
    for generation in range(80, 0, -1):
        validate_generation(current, generation)
        if generation == 1:
            if (
                current["previous_index_path"] is not None
                or current["previous_index_sha256"] is not None
                or current["previous_index_byte_length"] is not None
                or len(current["records"]) != 1
                or current["records"][0].get("state") != "captured_unclassified"
            ):
                raise LedgerError("R51 first diagnostic generation drift")
            break
        expected_previous_path = (
            f"diagnostic-index.generations/{generation - 1:08d}.json"
        )
        if current["previous_index_path"] != expected_previous_path:
            raise LedgerError("R51 diagnostic generation path drift")
        _, previous_bytes = _r51_bound_relative(
            root,
            current["previous_index_path"],
            current["previous_index_sha256"],
            current["previous_index_byte_length"],
            "R51 previous diagnostic index",
        )
        _r51_register_bound_path(
            seen_paths,
            current["previous_index_path"],
            "R51 previous diagnostic index",
        )
        previous = _parse_json(previous_bytes, "R51 previous diagnostic index")
        _require_keys(previous, R51_DIAGNOSTIC_INDEX_KEYS, "R51 diagnostic index")
        if _r51_canonical_json(previous) != previous_bytes:
            raise LedgerError("R51 previous diagnostic index is not canonical")
        previous_map = {item["cell_key"]: item for item in previous["records"]}
        current_map = {item["cell_key"]: item for item in current["records"]}
        changed = [
            key
            for key in set(previous_map) | set(current_map)
            if previous_map.get(key) != current_map.get(key)
        ]
        if len(changed) != 1:
            raise LedgerError("R51 diagnostic generation changed more than one cell")
        key = changed[0]
        if key not in previous_map:
            if (
                current_map[key].get("state") != "captured_unclassified"
                or len(current_map) != len(previous_map) + 1
            ):
                raise LedgerError("R51 diagnostic cell addition drift")
            if current_map[key]["phase"] == "holdout" and (
                len(previous_map) != 32
                and not any(
                    item.get("phase") == "holdout" for item in previous_map.values()
                )
                or any(
                    item.get("state") == "captured_unclassified"
                    for item in previous_map.values()
                )
            ):
                raise LedgerError("R51 holdout started before calibration terminalized")
        elif (
            previous_map[key].get("state") != "captured_unclassified"
            or current_map[key].get("state") not in {"passed", "failed"}
            or len(current_map) != len(previous_map)
        ):
            raise LedgerError("R51 diagnostic cell terminal transition drift")
        else:
            immutable = {
                "cell_key",
                "phase",
                "candidate_id",
                "entry_id",
                "device",
                "diagnostic_path",
                "diagnostic_sha256",
                "diagnostic_byte_length",
                "device_evidence_path",
                "device_evidence_sha256",
                "device_evidence_byte_length",
                "log_path",
                "log_sha256",
                "log_byte_length",
                "target_coverage_index_path",
                "target_coverage_index_sha256",
                "target_coverage_index_byte_length",
            }
            if any(
                previous_map[key][field] != current_map[key][field]
                for field in immutable
            ):
                raise LedgerError("R51 diagnostic terminal identity rewrite")
        if (
            key not in previous_map
            and current_map[key]["phase"] == "calibration-freeze"
            and any(item.get("phase") == "holdout" for item in previous_map.values())
        ):
            raise LedgerError("R51 calibration cell added after holdout")
        current = previous
    terminal_cells = []
    for record, (
        cell_key,
        phase,
        entry_id,
        device,
        candidate_id,
    ) in zip(records, expected):
        _require_keys(record, R51_DIAGNOSTIC_RECORD_KEYS, "R51 diagnostic record")
        _r51_validate_target_recall(record["target_recall"], "R51 target recall")
        calibration_result = calibration_by_cell.get(cell_key)
        expected_state = (
            "passed"
            if calibration_result is None
            or calibration_result["derived"]["passed"] is True
            else "failed"
        )
        if (
            record["phase"] != phase
            or record["entry_id"] != entry_id
            or record["device"] != device
            or record["candidate_id"] != candidate_id
            or record["cell_key"] != cell_key
            or record["state"] != expected_state
            or record["selection_result"]
            not in {"selected", "preserved", "rejected", None}
        ):
            raise LedgerError("R51 diagnostic record terminal state drift")
        if record["state"] == "passed" and (
            record["rejection_reason"] is not None
            or record["terminal_reason"] is not None
        ):
            raise LedgerError("R51 passing diagnostic has a failure reason")
        if record["state"] == "failed" and not isinstance(
            record["terminal_reason"], str
        ):
            raise LedgerError("R51 failed diagnostic lacks a terminal reason")
        if phase == "holdout" and (
            record["selection_result"] != "selected"
            or record["rejection_reason"] is not None
            or record["terminal_reason"] is not None
        ):
            raise LedgerError("R51 holdout diagnostic record did not pass")
        bindings = {
            "b0_sha": b0_sha,
            "calibration_manifest_sha256": index["calibration_manifest_sha256"],
            "manifest_sha256": bundle["manifest_sha256"],
            "fixture_manifest_sha256": index["fixture_manifest_sha256"],
            "oracle_sha256": bundle["oracle_sha256"],
            "hashes_sha256": bundle["hashes_sha256"],
            "bundle_sha256": bundle_sha256,
        }
        _r51_validate_cell_diagnostic(
            root, record, bindings, seen_paths, holdout=phase == "holdout"
        )
        if phase == "holdout":
            terminal_cells.append(record)
    return path, data, index, terminal_cells


def _r51_validate_authorization(arguments):
    repo_root = _canonical_existing_path(arguments.repo_root, "repo root")
    _validate_repository(repo_root)
    _r51_clean_detached_head(repo_root, arguments.b0_sha)
    contract_hashes = _r51_validate_contract_files(arguments)
    frozen = _r51_validate_freeze(arguments, contract_hashes, authorized=True)
    preflight_path, preflight_bytes, preflight = _r51_json(
        arguments.b0_preflight_attestation,
        "R51 B0 preflight attestation",
        keys=R51_PREFLIGHT_KEYS,
        canonical=True,
        mode=0o600,
    )
    staged_red_hashes = preflight["staged_red_log_sha256"]
    expected_red_ids = set(EXPECTED_B0_B1_MARKER_IDS) | set(EXPECTED_GREEN_C_RED_IDS)
    if (
        not isinstance(staged_red_hashes, dict)
        or set(staged_red_hashes) != expected_red_ids
    ):
        raise LedgerError("R51 B0 preflight staged RED inventory drift")
    for test_id, digest in staged_red_hashes.items():
        _validate_hash(digest, f"R51 B0 preflight staged RED {test_id}")
    _r51_validate_staged_red_logs(
        os.path.dirname(preflight_path),
        staged_red_hashes,
    )
    if (
        preflight["contract"] != "hanonly-r51-b0-preflight-v1"
        or preflight["plan_revision"] != R51_PLAN_REVISION
        or preflight["b0_sha"] != arguments.b0_sha
        or preflight["evidence_enabled_cargo_features"] != R51_FEATURES
        or preflight["gate_results"].keys() != R51_GATE_KEYS
        or any(value != "pass" for value in preflight["gate_results"].values())
        or preflight["frozen_interpreter_sha256"]
        != _r51_frozen_interpreter(repo_root, contract_hashes["paths"][0])
        or any(
            preflight[key] != contract_hashes[key]
            for key in contract_hashes
            if key != "paths"
        )
        or preflight["freeze_receipt_sha256"] != frozen["freeze_sha256"]
        or preflight["historical_inventory_sha256"] != frozen["historical_sha256"]
        or preflight["ciphertext_sha256"] != frozen["ciphertext_sha256"]
        or preflight["result"] != "pass"
    ):
        raise LedgerError("R51 B0 preflight attestation drift")
    executable_path, executable_bytes = _r51_read(
        preflight["evidence_test_executable_path"],
        "frozen R51 evidence test executable",
        mode=None,
    )
    if _sha256(executable_bytes) != preflight["evidence_test_executable_sha256"]:
        raise LedgerError("frozen R51 evidence test executable hash drift")
    checker_path, checker_bytes = _r51_read(
        os.path.join(repo_root, B0_CHECKER_ENDPOINT), "R51 checker", mode=None
    )
    checker_sha256 = _sha256(checker_bytes)
    pre_calibration = _r51_validate_attestation(
        arguments.pre_calibration_attestation,
        "pre-calibration",
        arguments.b0_sha,
        checker_sha256,
    )
    pre_holdout = _r51_validate_attestation(
        arguments.pre_holdout_attestation,
        "pre-holdout",
        arguments.b0_sha,
        checker_sha256,
    )
    if pre_calibration[0] == pre_holdout[0] or pre_calibration[1] == pre_holdout[1]:
        raise LedgerError("R51 requires two independent attestations")
    calibration_manifest_path, calibration_manifest_bytes = _r51_read(
        arguments.calibration_manifest, "R51 calibration manifest", mode=0o600
    )
    calibration_ledger_path, calibration_ledger_bytes, calibration_ledger = _r51_json(
        arguments.calibration_ledger,
        "R51 calibration ledger",
        canonical=True,
        mode=0o600,
    )
    recall_path, recall_bytes, recall = _r51_json(
        arguments.frozen_recall_contract,
        "R51 frozen recall contract",
        canonical=True,
        mode=0o600,
    )
    if recall.get("selected_candidate_id") not in {
        candidate["id"] for candidate in B0_CANDIDATES
    }:
        raise LedgerError("R51 frozen recall candidate drift")
    open_path, open_bytes, open_marker = _r51_json(
        arguments.open_marker,
        "R51 holdout open marker",
        keys=R51_OPEN_KEYS,
        canonical=True,
        mode=0o600,
    )
    open_sha256 = _sha256(open_bytes)
    bundle_path, bundle_bytes, bundle = _r51_json(
        arguments.bundle_validation_receipt,
        "R51 bundle validation receipt",
        keys=R51_BUNDLE_KEYS,
        canonical=True,
        mode=0o600,
    )
    bundle_sha256 = _sha256(bundle_bytes)
    selected_candidate_id = recall["selected_candidate_id"]
    if (
        open_marker["contract"] != "hanonly-r51-encrypted-holdout-open-v1"
        or open_marker["plan_revision"] != R51_PLAN_REVISION
        or open_marker["b0_sha"] != arguments.b0_sha
        or open_marker["selected_candidate_id"] != selected_candidate_id
        or open_marker["freeze_receipt_sha256"] != frozen["freeze_sha256"]
        or open_marker["ciphertext_sha256"] != frozen["ciphertext_sha256"]
        or open_marker["pre_holdout_attestation_sha256"] != pre_holdout[1]
        or open_marker["result"] != "opened"
    ):
        raise LedgerError("R51 holdout open marker binding drift")
    _validate_hash(open_marker["nonce_hex"], "R51 open nonce")
    if (
        bundle["contract"] != "hanonly-r51-bundle-validation-v1"
        or bundle["plan_revision"] != R51_PLAN_REVISION
        or bundle["b0_sha"] != arguments.b0_sha
        or bundle["test_executable_sha256"]
        != preflight["evidence_test_executable_sha256"]
        or bundle["enabled_cargo_features"] != R51_FEATURES
        or bundle["r51_contract_sha256"] != contract_hashes["r51_contract_sha256"]
        or bundle["freeze_receipt_sha256"] != frozen["freeze_sha256"]
        or bundle["plaintext_archive_sha256"]
        != frozen["freeze"]["plaintext_archive_sha256_commitment"]
        or bundle["manifest_sha256"] != frozen["freeze"]["manifest_sha256_commitment"]
        or bundle["oracle_sha256"] != frozen["freeze"]["oracle_sha256_commitment"]
        or bundle["hashes_sha256"] != frozen["freeze"]["hashes_sha256_commitment"]
        or any(
            bundle[key] is not True
            for key in (
                "schema_validation_pass",
                "asset_binding_pass",
                "mask_source_clean_equality_pass",
                "oracle_semantics_pass",
            )
        )
        or bundle["result"] != "pass"
    ):
        raise LedgerError("R51 bundle validation receipt binding drift")
    if (
        pre_calibration[2]["manifest_sha256"] != _sha256(calibration_manifest_bytes)
        or pre_holdout[2]["manifest_sha256"] != bundle["manifest_sha256"]
        or pre_calibration[2]["source_gate_fixture_manifest_sha256"]
        != pre_holdout[2]["source_gate_fixture_manifest_sha256"]
    ):
        raise LedgerError("R51 required-check manifest binding drift")
    diagnostic_path, diagnostic_bytes, diagnostic, diagnostic_cells = (
        _r51_validate_terminal_diagnostic_index(
            arguments.terminal_diagnostic_index,
            arguments.b0_sha,
            selected_candidate_id,
            _sha256(calibration_manifest_bytes),
            pre_holdout[2]["source_gate_fixture_manifest_sha256"],
            calibration_ledger,
            bundle_path,
            bundle_sha256,
            bundle,
        )
    )
    diagnostic_sha256 = _sha256(diagnostic_bytes)
    terminal_path, terminal_bytes, terminal = _r51_json(
        arguments.terminal_receipt,
        "R51 terminal receipt",
        keys=R51_TERMINAL_KEYS,
        canonical=True,
        mode=0o600,
    )
    terminal_cells = _r51_validate_terminal(
        terminal,
        {
            "b0_sha": arguments.b0_sha,
            "selected_candidate_id": selected_candidate_id,
            "freeze_receipt_sha256": frozen["freeze_sha256"],
            "open_marker_sha256": open_sha256,
            "ciphertext_sha256": frozen["ciphertext_sha256"],
            "pre_holdout_attestation_sha256": pre_holdout[1],
            "bundle_validation_receipt_sha256": bundle_sha256,
        },
    )
    if terminal["terminal_diagnostic_index_sha256"] != diagnostic_sha256:
        raise LedgerError("R51 terminal diagnostic index hash drift")
    diagnostic_by_terminal_key = {
        f"{record['entry_id']}/{record['device']}": record
        for record in diagnostic_cells
    }
    for terminal_cell in terminal_cells:
        record = diagnostic_by_terminal_key.get(terminal_cell["cell_key"])
        if record is None:
            raise LedgerError("R51 terminal receipt diagnostic cell is missing")
        if (
            terminal_cell["selection_result"] != record["selection_result"]
            or terminal_cell["target_recall"] != record["target_recall"]
            or terminal_cell["pp_han_count"] != record["pp_han_count"]
            or terminal_cell["vl_han_count"] != record["vl_han_count"]
            or terminal_cell["rejection_reason"] != record["rejection_reason"]
            or terminal_cell["device_evidence_sha256"]
            != record["device_evidence_sha256"]
            or terminal_cell["log_sha256"] != record["log_sha256"]
            or terminal_cell["diagnostic_sha256"] != record["diagnostic_sha256"]
            or terminal_cell["target_coverage_index_sha256"]
            != record["target_coverage_index_sha256"]
        ):
            raise LedgerError("R51 terminal receipt diagnostic projection drift")
    custody_dir = os.path.dirname(open_path)
    failure_temp_prefix = ".holdout-failure."
    with contextlib.ExitStack() as stack:
        custody = _open_absolute(custody_dir, directory=True, stack=stack)
        custody_names = set(os.listdir(custody.fd))
    if "holdout-failure.json" in custody_names or any(
        name.startswith(failure_temp_prefix) and name.endswith(".tmp")
        for name in custody_names
    ):
        raise LedgerError("R51 irreversible failure marker is present")
    payload_path, payload_bytes, payload = _r51_json(
        arguments.artifact_payload,
        "R51 artifact payload",
        keys=R51_ARTIFACT_PAYLOAD_KEYS,
        canonical=True,
        mode=0o600,
    )
    if (
        payload["version"] != B0_VERSION
        or payload["plan_revision"] != R51_PLAN_REVISION
        or payload["b0_sha"] != arguments.b0_sha
        or payload["selected_candidate_id"] != selected_candidate_id
        or payload["frozen_recall_contract"] != recall
        or payload["calibration_manifest_sha256"] != _sha256(calibration_manifest_bytes)
        or payload["freeze_receipt_sha256"] != frozen["freeze_sha256"]
        or payload["ciphertext_sha256"] != frozen["ciphertext_sha256"]
        or payload["bundle_validation_receipt_sha256"] != bundle_sha256
        or payload["terminal_diagnostic_index_sha256"] != diagnostic_sha256
        or payload["holdout_results"] != terminal_cells
    ):
        raise LedgerError("R51 artifact payload binding drift")
    checks = payload["required_checks"]
    if (
        not isinstance(checks, list)
        or len(checks) != 2
        or [check.get("phase") for check in checks]
        != ["pre-calibration", "pre-holdout"]
        or [check.get("attestation_sha256") for check in checks]
        != [pre_calibration[1], pre_holdout[1]]
        or any(check.get("result") != "pass" for check in checks)
    ):
        raise LedgerError("R51 artifact required checks drift")
    _r51_validate_calibration(
        payload,
        calibration_ledger,
        os.path.dirname(calibration_ledger_path),
    )
    record = {
        "contract": "hanonly-r51-b0-authorization-v1",
        "plan_revision": R51_PLAN_REVISION,
        "b0_sha": arguments.b0_sha,
        **{key: value for key, value in contract_hashes.items() if key != "paths"},
        "b0_preflight_attestation_sha256": _sha256(preflight_bytes),
        "calibration_manifest_sha256": _sha256(calibration_manifest_bytes),
        "calibration_ledger_sha256": _sha256(calibration_ledger_bytes),
        "freeze_receipt_sha256": frozen["freeze_sha256"],
        "historical_inventory_sha256": frozen["historical_sha256"],
        "ciphertext_sha256": frozen["ciphertext_sha256"],
        "pre_calibration_attestation_sha256": pre_calibration[1],
        "pre_holdout_attestation_sha256": pre_holdout[1],
        "frozen_recall_contract_sha256": _sha256(recall_bytes),
        "selected_candidate_id": selected_candidate_id,
        "open_marker_sha256": open_sha256,
        "bundle_validation_receipt_sha256": bundle_sha256,
        "terminal_receipt_sha256": _sha256(terminal_bytes),
        "terminal_diagnostic_index_sha256": diagnostic_sha256,
        "failure_marker_absent": True,
        "artifact_payload_sha256": _sha256(payload_bytes),
        "result": "pass",
    }
    _require_keys(record, R51_AUTHORIZATION_KEYS, "R51 B0 authorization record")
    authorization_out = _canonical_future_path(
        arguments.authorization_record_out, "R51 B0 authorization record"
    )
    artifact_out = _canonical_future_path(arguments.artifact_out, "R51 B0 artifact")
    if os.path.dirname(authorization_out) != os.path.dirname(artifact_out):
        raise LedgerError("R51 authorization and artifact must share one evidence root")
    with contextlib.ExitStack() as stack:
        output_root = _open_absolute(
            os.path.dirname(authorization_out), directory=True, stack=stack
        )
        _require_owned_mode(output_root.path, output_root.stat, 0o700)
        output_names = set(os.listdir(output_root.fd))
    authorization_name = os.path.basename(authorization_out)
    artifact_name = os.path.basename(artifact_out)
    artifact_temps = {
        name
        for name in output_names
        if name.startswith(".hanonly-r51-b0-artifact.") and name.endswith(".tmp")
    }
    if authorization_name not in output_names and (
        artifact_name in output_names or artifact_temps
    ):
        raise LedgerError("R51 artifact exists before its authorization record")
    record_path, record_sha256 = _r51_publish(
        authorization_out,
        record,
        "R51 B0 authorization record",
    )
    artifact = {**payload, "authorization_record_sha256": record_sha256}
    artifact_path, artifact_sha256 = _r51_publish(
        artifact_out,
        artifact,
        "R51 B0 artifact",
    )
    return (
        _r51_canonical_json(
            {
                "authorization_record_sha256": record_sha256,
                "artifact_path": artifact_path,
                "artifact_sha256": artifact_sha256,
            }
        )
        + b"\n"
    )


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        raise LedgerError(message)


def _parse_arguments(argv):
    parser = _Parser(prog="hanonly_evidence_ledger.py")
    subparsers = parser.add_subparsers(dest="command", required=True)
    create = subparsers.add_parser("create")
    create.add_argument("--repo-root", required=True)
    create.add_argument("--expected-base", required=True)
    create.add_argument("--run-id", required=True)
    create.add_argument("--input", required=True)
    create.add_argument("--expected-input-sha256", required=True)
    create.add_argument("--expected-input-size", required=True)
    create.add_argument("--manifest", required=True)
    create.add_argument("--source-gate-fixture-manifest", required=True)
    rehydrate = subparsers.add_parser("rehydrate")
    rehydrate.add_argument("--repo-root", required=True)
    rehydrate.add_argument("--evidence-root", required=True)
    validate_b0 = subparsers.add_parser("validate-b0-artifact")
    validate_b0.add_argument("--repo-root", required=True)
    validate_b0.add_argument("--artifact", required=True)
    validate_b0.add_argument("--b0-sha", required=True)
    validate_b0.add_argument("--visual-manifest-sha256", required=True)
    validate_b0.add_argument("--source-gate-fixture-manifest-sha256", required=True)
    r51_preflight = subparsers.add_parser("write-r51-b0-preflight-attestation")
    r51_preflight.add_argument("--repo-root", required=True)
    r51_preflight.add_argument("--output", required=True)
    r51_preflight.add_argument("--b0-sha", required=True)
    r51_preflight.add_argument("--implementation-thread-id", required=True)
    r51_preflight.add_argument("--r51-contract", required=True)
    r51_preflight.add_argument("--operative-plan", required=True)
    r51_preflight.add_argument("--r51-test-spec", required=True)
    r51_preflight.add_argument("--base-production-contract", required=True)
    r51_preflight.add_argument("--freeze-receipt", required=True)
    r51_preflight.add_argument("--historical-inventory", required=True)
    r51_preflight.add_argument("--ciphertext", required=True)
    r51_preflight.add_argument("--gate-results", required=True)
    r51_preflight.add_argument("--staged-red-log", required=True)
    r51_preflight.add_argument("--evidence-test-executable", required=True)
    r51_preflight.add_argument("--cargo-target-dir", required=True)
    r51_snapshot = subparsers.add_parser("snapshot-r51-preflight-custody")
    r51_snapshot.add_argument("--repo-root", required=True)
    r51_snapshot.add_argument("--r51-contract", required=True)
    r51_snapshot.add_argument("--operative-plan", required=True)
    r51_snapshot.add_argument("--r51-test-spec", required=True)
    r51_snapshot.add_argument("--base-production-contract", required=True)
    r51_snapshot.add_argument("--freeze-receipt", required=True)
    r51_snapshot.add_argument("--historical-inventory", required=True)
    r51_snapshot.add_argument("--ciphertext", required=True)
    r51_authorization = subparsers.add_parser("validate-r51-b0-authorization")
    r51_authorization.add_argument("--repo-root", required=True)
    r51_authorization.add_argument("--b0-sha", required=True)
    r51_authorization.add_argument("--r51-contract", required=True)
    r51_authorization.add_argument("--operative-plan", required=True)
    r51_authorization.add_argument("--r51-test-spec", required=True)
    r51_authorization.add_argument("--base-production-contract", required=True)
    r51_authorization.add_argument("--b0-preflight-attestation", required=True)
    r51_authorization.add_argument("--calibration-manifest", required=True)
    r51_authorization.add_argument("--calibration-ledger", required=True)
    r51_authorization.add_argument("--freeze-receipt", required=True)
    r51_authorization.add_argument("--historical-inventory", required=True)
    r51_authorization.add_argument("--ciphertext", required=True)
    r51_authorization.add_argument("--pre-calibration-attestation", required=True)
    r51_authorization.add_argument("--pre-holdout-attestation", required=True)
    r51_authorization.add_argument("--frozen-recall-contract", required=True)
    r51_authorization.add_argument("--open-marker", required=True)
    r51_authorization.add_argument("--bundle-validation-receipt", required=True)
    r51_authorization.add_argument("--terminal-receipt", required=True)
    r51_authorization.add_argument("--terminal-diagnostic-index", required=True)
    r51_authorization.add_argument("--artifact-payload", required=True)
    r51_authorization.add_argument("--authorization-record-out", required=True)
    r51_authorization.add_argument("--artifact-out", required=True)
    return parser.parse_args(argv)


def execute(argv):
    arguments = _parse_arguments(argv)
    if arguments.command == "create":
        return _create(arguments)
    if arguments.command == "rehydrate":
        return _rehydrate(arguments)
    if arguments.command == "validate-b0-artifact":
        return _validate_b0_artifact(arguments)
    if arguments.command == "write-r51-b0-preflight-attestation":
        return _r51_write_preflight(arguments)
    if arguments.command == "snapshot-r51-preflight-custody":
        return _r51_canonical_json(_r51_preflight_custody_snapshot(arguments)) + b"\n"
    return _r51_validate_authorization(arguments)


def main(argv=None, *, stdout=None, stderr=None):
    stdout = stdout if stdout is not None else sys.stdout.buffer
    stderr = stderr if stderr is not None else sys.stderr
    try:
        output = execute(sys.argv[1:] if argv is None else argv)
    except (LedgerError, OSError, subprocess.SubprocessError) as error:
        stderr.write(f"hanonly evidence ledger: {error}\n")
        return 2
    stdout.write(output)
    if hasattr(stdout, "flush"):
        stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
