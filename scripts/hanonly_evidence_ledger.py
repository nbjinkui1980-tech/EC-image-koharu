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
        raise LedgerError("required descriptor-relative filesystem operations are unavailable")


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
        or attestation["source_gate_fixture_manifest_sha256"]
        != fixture_manifest_sha256
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


def _require_keys(value, keys, label):
    if not isinstance(value, dict) or set(value) != keys:
        raise LedgerError(f"{label} keys are not closed and complete")


def _validate_backend_map(value, label, required_backend):
    if (
        not isinstance(value, dict)
        or not value
        or any(not isinstance(key, str) or type(size) is not int or size < 0 for key, size in value.items())
        or value.get(required_backend, 0) <= 0
    ):
        raise LedgerError(f"{label} is invalid")


def _validate_result(result, processes, entry_ids, candidate_ids, phase, artifact_root):
    label = f"{phase} result"
    _require_keys(result, B0_RESULT_KEYS, label)
    if result["entry_id"] not in entry_ids or result["candidate_id"] not in candidate_ids:
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
            or any(type(value) not in (int, float) for value in node["recognition_anchor"])
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
    expected_complete_coverage = (
        coverage["pp_han_scalar_count"] > 0
        and coverage["pp_han_scalar_count"]
        == coverage["vl_expected_han_scalar_count"]
        and coverage["rejected_after_vl"] is False
        and coverage["pp_vl_incomplete_coverage"] is False
        and coverage["source_text_roi_coverage"] == 1.0
    )
    expected_preflight = (
        derived["target_recall"] == 1.0 and expected_complete_coverage
    )
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
            if any(size > 0 for backend, size in backend_map.items() if backend != "CPU"):
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
        or any(not isinstance(value, str) or not value for value in calibration_ids + holdout_ids)
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
        _require_keys(process["model_artifact_sha256"], B0_MODEL_HASH_KEYS, "model hashes")
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
                or device["device_type"] not in {
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
        (process["phase"], process["requested_device"]) for process in processes.values()
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
            result, processes, calibration_ids, candidate_ids, "calibration", artifact_root
        )
    for result in holdout:
        if result.get("candidate_id") != value["selected_candidate_id"]:
            raise LedgerError("holdout candidate drift")
        _validate_result(
            result, processes, holdout_ids, candidate_ids, "holdout", artifact_root
        )
    calibration_cells = {
        (result["entry_id"], processes[result["process_evidence_id"]]["requested_device"], result["candidate_id"])
        for result in calibration
    }
    holdout_cells = {
        (result["entry_id"], processes[result["process_evidence_id"]]["requested_device"])
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
    if _sha256(canonical_json(_b0_frozen_projection(value))) != value["frozen_payload_sha256"]:
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
            raise LedgerError("expected base disagrees with HANONLY_SHARED_EVIDENCE_BASE")
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
            if held.stat.st_uid != fresh.stat.st_uid or _mode(held.stat) != _mode(fresh.stat):
                raise LedgerError(f"namespace metadata changed for {label}")
            if label in expected_files:
                expected_bytes, expected_hash = expected_files[label]
                fresh_bytes = _read_all(fresh.fd)
                if fresh_bytes != expected_bytes or _sha256(fresh_bytes) != expected_hash:
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
                raise LedgerError(f"cannot remove deterministic temp: {error}") from error
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
    expected_hash = _validate_hash(arguments.expected_input_sha256, "expected input hash")
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
        manifest = _open_absolute(value["visual_manifest"], directory=False, stack=stack)
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
    return parser.parse_args(argv)


def execute(argv):
    arguments = _parse_arguments(argv)
    if arguments.command == "create":
        return _create(arguments)
    if arguments.command == "rehydrate":
        return _rehydrate(arguments)
    return _validate_b0_artifact(arguments)


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
