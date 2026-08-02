import contextlib
import hashlib
import io
import json
import os
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

try:
    from scripts import hanonly_evidence_ledger as ledger
except ModuleNotFoundError:
    import hanonly_evidence_ledger as ledger  # type: ignore[import-not-found,no-redef]


WIDTH = 790
HEIGHT = 1023
HEX64 = "a" * 64
B0_RAW_LOG_BYTES = b"hanonly b0 raw log\n"
B0_RAW_LOG_SHA256 = hashlib.sha256(B0_RAW_LOG_BYTES).hexdigest()


def jpeg(width=WIDTH, height=HEIGHT, sof=0xC0, extra=b""):
    app = b"\xff\xe1" + struct.pack(">H", len(extra) + 2) + extra
    frame = (
        b"\xff"
        + bytes([sof])
        + struct.pack(">H", 17)
        + bytes([8])
        + struct.pack(">HH", height, width)
        + b"\x03\x01\x11\x00\x02\x11\x00\x03\x11\x00"
    )
    scan = b"\xff\xda\x00\x08\x01\x01\x00\x00\x3f\x00\x11\xff\x00\x22"
    return b"\xff\xd8" + app + frame + scan + b"\xff\xd9"


def riff(*chunks):
    body = b"WEBP"
    for name, payload in chunks:
        body += name + struct.pack("<I", len(payload)) + payload
        if len(payload) & 1:
            body += b"\x00"
    return b"RIFF" + struct.pack("<I", len(body)) + body


def vp8(width=WIDTH, height=HEIGHT):
    payload = b"\x00\x00\x00\x9d\x01\x2a" + struct.pack("<HH", width, height)
    return riff((b"VP8 ", payload))


def vp8l(width=WIDTH, height=HEIGHT, *, alpha_is_used=False, version=0):
    bits = (width - 1) | ((height - 1) << 14)
    bits |= int(alpha_is_used) << 28
    bits |= version << 29
    return riff((b"VP8L", b"\x2f" + struct.pack("<I", bits)))


def vp8x(width=WIDTH, height=HEIGHT):
    payload = b"\x00\x00\x00\x00" + (width - 1).to_bytes(3, "little")
    payload += (height - 1).to_bytes(3, "little")
    return riff((b"VP8X", payload))


def jpeg_with_sof_payload(payload):
    return (
        b"\xff\xd8"
        + b"\xff\xc0"
        + struct.pack(">H", len(payload) + 2)
        + payload
        + b"\xff\xd9"
    )


def b0_attestation(phase, b0_sha, manifest_sha256, fixture_sha256, checker_sha256):
    return {
        "version": 1,
        "mode": "b0-source-gate-anti-fixture",
        "phase": phase,
        "b0_sha": b0_sha,
        "manifest_sha256": manifest_sha256,
        "source_gate_fixture_manifest_sha256": fixture_sha256,
        "checker_endpoint_sha256": checker_sha256,
        "scanned_roots": ledger.B0_ANTI_FIXTURE_SCANNED_ROOTS,
        "allowed_descriptor_roots": ledger.B0_ANTI_FIXTURE_ALLOWED_DESCRIPTOR_ROOTS,
        "policy_scan_sha256": "9" * 64,
        "result": "pass",
    }


def b0_artifact():
    b0_sha = "b" * 40
    manifest_sha256 = "c" * 64
    fixture_sha256 = "d" * 64
    checker_sha256 = hashlib.sha256(
        Path(__file__).with_name("check-hanonly-production-policy.ts").read_bytes()
    ).hexdigest()

    def process(phase, device):
        metal = device == "metal"
        return {
            "id": f"{phase}-{device}",
            "phase": phase,
            "requested_device": device,
            "paddle_instance_id": ("1" if device == "cpu" else "2") * 32,
            "executable_sha256": HEX64,
            "model_artifact_sha256": {
                name: HEX64
                for name in (
                    "pp_detection",
                    "pp_recognition",
                    "pp_recognition_config",
                    "vl_model",
                    "vl_mmproj",
                )
            },
            "runtime_library_sha256": {"/usr/lib/libsynthetic.dylib": HEX64},
            "load_evidence": {
                "cpu_forced": not metal,
                "gpu_offload_supported": metal,
                "n_gpu_layers": 1000 if metal else 0,
                "mtmd_use_gpu": metal,
                "word_boxes_backend": "rten_cpu",
                "raw_load_log_relpath": f"source-gate/{phase}/{device}/load.log",
                "raw_load_log_sha256": B0_RAW_LOG_SHA256,
                "enumerated_devices": [],
                "loaded_model_devices": [
                    {
                        "model_device_ordinal": 0,
                        "name": "Apple GPU" if metal else "CPU",
                        "backend": "Metal" if metal else "CPU",
                        "device_type": "integrated_gpu" if metal else "cpu",
                    }
                ],
                "offloaded_layers": 32 if metal else 0,
                "offloadable_layers": 39,
                "model_buffer_bytes_by_backend": {
                    "CPU": 1,
                    **({"Metal": 1} if metal else {}),
                },
                "mtmd_backend": "Metal" if metal else "CPU",
            },
        }

    processes = [
        process(phase, device)
        for phase in ("calibration", "holdout")
        for device in ("cpu", "metal")
    ]

    def result(phase, entry_id, device, candidate):
        metal = device == "metal"
        return {
            "entry_id": entry_id,
            "process_evidence_id": f"{phase}-{device}",
            "candidate_id": candidate,
            "execution_evidence": {
                "paddle_instance_id": ("2" if metal else "1") * 32,
                "context_offload_kqv": metal,
                "context_op_offload": metal,
                "inference_completed": True,
                "raw_inference_log_relpath": (
                    f"source-gate/{phase}/{entry_id}/{device}/{candidate}.log"
                ),
                "raw_inference_log_sha256": B0_RAW_LOG_SHA256,
                "source_gate_diagnostic_relpath": (
                    f"source-gate/{phase}/{entry_id}/{device}/{candidate}.source-gate.json"
                ),
                "source_gate_diagnostic_sha256": B0_RAW_LOG_SHA256,
                "context_buffer_bytes_by_backend": {
                    "CPU": 1,
                    **({"Metal": 1} if metal else {}),
                },
                "compute_buffer_bytes_by_backend": {
                    "CPU": 1,
                    **({"Metal": 1} if metal else {}),
                },
            },
            "runtime_nodes": [
                {
                    "node_id": f"{entry_id}-node",
                    "recognition_anchor": [0, 0, 1, 1],
                    "node_rotation": 0.0,
                    "text_rotation": 0.0,
                    "selected_as_han": True,
                }
            ],
            "derived": {
                "actual_device": device,
                "matched_target_ids": ["target"],
                "selected_target_ids": ["target"],
                "selected_protected_node_ids": [],
                "selected_rotation_target_ids": [],
                "unmatched_selected_node_ids": [],
                "target_recall": 1.0,
                "protected_false_positive_count": 0,
                "rotation_targets_excluded": True,
                "source_coverage_preflight": {
                    "pp_han_scalar_count": 1,
                    "vl_expected_han_scalar_count": 1,
                    "pp_vl_complete_coverage": True,
                    "rejected_after_vl": False,
                    "pp_vl_incomplete_coverage": False,
                    "covered_source_roi_ids": ["target"],
                    "source_text_roi_coverage": 1.0,
                    "source_removal_preflight_passed": True,
                },
                "passed": True,
            },
        }

    calibration_ids = [f"c{index:02}" for index in range(1, 5)]
    holdout_ids = [f"h{index:02}" for index in range(1, 5)]
    required_checks = []
    for phase in ("pre-calibration", "pre-holdout"):
        attestation = b0_attestation(
            phase,
            b0_sha,
            manifest_sha256,
            fixture_sha256,
            checker_sha256,
        )
        required_checks.append(
            {
                "phase": phase,
                "command": ledger.B0_REQUIRED_CHECK_COMMAND,
                "checker_endpoint_sha256": checker_sha256,
                "manifest_sha256": manifest_sha256,
                "source_gate_fixture_manifest_sha256": fixture_sha256,
                "attestation_relpath": (f"source-gate-selection/checks/{phase}.json"),
                "attestation_sha256": ledger._sha256(
                    ledger.canonical_json(attestation)
                ),
                "b0_sha": b0_sha,
                "result": "pass",
            }
        )
    value = {
        "version": ledger.B0_VERSION,
        "plan_revision": ledger.B0_PLAN_REVISION,
        "b0_sha": b0_sha,
        "manifest_sha256": manifest_sha256,
        "source_gate_fixture_manifest_sha256": fixture_sha256,
        "image_input_contract_sha256": "e" * 64,
        "source_color_contract_sha256": "f" * 64,
        "color_constant_set_sha256": "1" * 64,
        "requested_devices": ["cpu", "metal"],
        "enabled_cargo_features": ["hanonly-test-evidence", "metal"],
        "backend_evidence_parser_version": 1,
        "required_checks": required_checks,
        "frozen_recall_contract": ledger._expected_frozen_recall("S25L4"),
        "candidates": json.loads(json.dumps(ledger.B0_CANDIDATES)),
        "calibration_entry_ids": calibration_ids,
        "holdout_entry_ids": holdout_ids,
        "process_evidence": processes,
        "calibration_results": [
            result("calibration", entry_id, device, candidate["id"])
            for entry_id in calibration_ids
            for device in ("cpu", "metal")
            for candidate in ledger.B0_CANDIDATES
        ],
        "selected_candidate_id": "S25L4",
        "frozen_at_utc": "2026-07-26T00:00:00Z",
        "frozen_payload_sha256": "2" * 64,
        "holdout_results": [
            result("holdout", entry_id, device, "S25L4")
            for entry_id in holdout_ids
            for device in ("cpu", "metal")
        ],
        "holdout_completed_at_utc": "2026-07-26T00:01:00Z",
        "retuned_after_freeze": False,
    }
    value["frozen_payload_sha256"] = ledger._sha256(
        ledger.canonical_json(ledger._b0_frozen_projection(value))
    )
    return value


class Case:
    def __init__(self, name="case"):
        self.temp = tempfile.TemporaryDirectory(prefix=f"hanonly ledger {name} ")
        self.root = Path(self.temp.name).resolve()
        self.repo = self.root / "repo"
        self.repo.mkdir(mode=0o700)
        self.base = self.root / "evidence"
        self.base.mkdir(mode=0o700)
        self.input = self.root / "input.jpeg"
        self.input.write_bytes(jpeg())
        self.manifest = self.root / "visual manifest.json"
        self.fixture = (
            self.repo
            / "crates/koharu-app/tests/fixtures/source-gate-deterministic-recall"
            / "fixture-manifest.json"
        )
        self.fixture.parent.mkdir(parents=True)
        self.fixture.write_text('{"fixtures":[{"name":"fixed"}]}\n', encoding="utf-8")
        self._write_manifest()
        subprocess.run(["git", "init", "-q"], cwd=self.repo, check=True)
        subprocess.run(
            ["git", "config", "user.name", "Ledger Test"], cwd=self.repo, check=True
        )
        subprocess.run(
            ["git", "config", "user.email", "ledger@example.invalid"],
            cwd=self.repo,
            check=True,
        )
        subprocess.run(["git", "add", "."], cwd=self.repo, check=True)
        subprocess.run(["git", "commit", "-qm", "fixture"], cwd=self.repo, check=True)
        self.run_id = "20260725T123456Z-88718dd92986-1234"

    def close(self):
        self.temp.cleanup()

    def _write_manifest(self, *, sha=None, path=None, role="regression"):
        sha = sha or hashlib.sha256(self.input.read_bytes()).hexdigest()
        path = str(path or self.input)
        value = {
            "version": 1,
            "entries": [
                {
                    "id": "regression",
                    "path": path,
                    "sha256": sha,
                    "decoded_rgba_blake3": HEX64,
                    "role": role,
                }
            ],
        }
        self.manifest.write_text(json.dumps(value), encoding="utf-8")

    @property
    def evidence_root(self):
        return self.base / self.run_id

    @property
    def ledger_path(self):
        return self.evidence_root / "evidence-ledger.json"

    def create_args(self):
        return [
            "create",
            "--repo-root",
            str(self.repo),
            "--expected-base",
            str(self.base),
            "--run-id",
            self.run_id,
            "--input",
            str(self.input),
            "--expected-input-sha256",
            hashlib.sha256(self.input.read_bytes()).hexdigest(),
            "--expected-input-size",
            f"{WIDTH}x{HEIGHT}",
            "--manifest",
            str(self.manifest),
            "--source-gate-fixture-manifest",
            str(self.fixture),
        ]

    def rehydrate_args(self):
        return [
            "rehydrate",
            "--repo-root",
            str(self.repo),
            "--evidence-root",
            str(self.evidence_root),
        ]

    @contextlib.contextmanager
    def environment(self):
        old = os.getcwd()
        os.chdir(self.repo)
        with mock.patch.dict(
            os.environ,
            {"HANONLY_SHARED_EVIDENCE_BASE": str(self.base)},
            clear=False,
        ):
            try:
                yield
            finally:
                os.chdir(old)

    def cli(self, args, checkpoint=None):
        stdout = io.BytesIO()
        stderr = io.StringIO()
        patch = (
            mock.patch.object(ledger, "_checkpoint", side_effect=checkpoint)
            if checkpoint
            else contextlib.nullcontext()
        )
        with self.environment(), patch:
            status = ledger.main(args, stdout=stdout, stderr=stderr)
        return status, stdout.getvalue(), stderr.getvalue()

    def subprocess_cli(self, args, *, timeout=2):
        return subprocess.run(
            ["python3", str(Path(ledger.__file__).resolve()), *args],
            cwd=self.repo,
            env={
                **os.environ,
                "HANONLY_SHARED_EVIDENCE_BASE": str(self.base),
                "PYTHONDONTWRITEBYTECODE": "1",
            },
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
        )

    def hide_fixture_change(self, flag):
        subprocess.run(
            [
                "git",
                "update-index",
                flag,
                "--",
                ledger.FIXTURE_RELATIVE_PATH,
            ],
            cwd=self.repo,
            check=True,
        )
        self.fixture.write_bytes(self.fixture.read_bytes() + b"hidden dirty\n")
        hidden = subprocess.run(
            [
                "git",
                "status",
                "--porcelain=v1",
                "--",
                ledger.FIXTURE_RELATIVE_PATH,
            ],
            cwd=self.repo,
            check=True,
            stdout=subprocess.PIPE,
        )
        if hidden.stdout:
            raise AssertionError(f"{flag} did not hide fixture dirtiness")


class ImageDimensionTests(unittest.TestCase):
    def test_reads_baseline_and_progressive_jpeg_dimensions(self):
        for marker in (0xC0, 0xC2):
            with self.subTest(marker=marker):
                self.assertEqual(
                    ledger.image_dimensions(jpeg(sof=marker)), (WIDTH, HEIGHT)
                )

    def test_accepts_legal_jpeg_metadata(self):
        self.assertEqual(
            ledger.image_dimensions(jpeg(extra=b"metadata")), (WIDTH, HEIGHT)
        )

    def test_rejects_malformed_jpeg_structures(self):
        valid = jpeg()
        duplicate = valid[:-2] + jpeg()[2:-2] + b"\xff\xd9"
        contradictory = valid[:-2] + jpeg(WIDTH + 1)[2:-2] + b"\xff\xd9"
        cases = {
            "truncated": valid[:-1],
            "bad-length": b"\xff\xd8\xff\xe1\x00\x01\xff\xd9",
            "missing-sof": b"\xff\xd8\xff\xd9",
            "duplicate-sof": duplicate,
            "contradictory-sof": contradictory,
            "not-jpeg": b"not an image",
        }
        for name, value in cases.items():
            with self.subTest(name=name):
                with self.assertRaises(ledger.LedgerError):
                    ledger.image_dimensions(value)

    def test_rejects_invalid_jpeg_component_tables(self):
        prefix = bytes([8]) + struct.pack(">HH", HEIGHT, WIDTH)
        cases = {
            "zero-components": prefix + b"\x00",
            "truncated-components": prefix + b"\x01\x01\x11",
            "oversized-components": prefix + b"\x01\x01\x11\x00\xff",
        }
        for name, payload in cases.items():
            with self.subTest(name=name):
                with self.assertRaises(ledger.LedgerError):
                    ledger.image_dimensions(jpeg_with_sof_payload(payload))

    def test_reads_each_supported_webp_dimension_header(self):
        for name, value in (("VP8", vp8()), ("VP8L", vp8l()), ("VP8X", vp8x())):
            with self.subTest(name=name):
                self.assertEqual(ledger.image_dimensions(value), (WIDTH, HEIGHT))

    def test_vp8l_accepts_alpha_is_used_and_rejects_nonzero_version(self):
        self.assertEqual(
            ledger.image_dimensions(vp8l(alpha_is_used=True)),
            (WIDTH, HEIGHT),
        )
        with self.assertRaises(ledger.LedgerError):
            ledger.image_dimensions(vp8l(version=1))

    def test_accepts_consistent_vp8x_and_vp8_dimensions(self):
        extended = riff(
            (b"VP8X", vp8x()[20:30]),
            (b"EXIF", b"x"),
            (b"VP8 ", vp8()[20:30]),
        )
        self.assertEqual(ledger.image_dimensions(extended), (WIDTH, HEIGHT))

    def test_rejects_malformed_webp_structures(self):
        contradictory = riff(
            (b"VP8X", vp8x()[20:30]),
            (b"VP8 ", vp8(WIDTH + 1)[20:30]),
        )
        duplicate = riff((b"VP8L", vp8l()[20:25]), (b"VP8L", vp8l()[20:25]))
        cases = {
            "truncated-riff": vp8()[:-1],
            "bad-riff-size": vp8() + b"x",
            "truncated-chunk": b"RIFF\x10\x00\x00\x00WEBPVP8 \x10\x00\x00\x00x",
            "unsupported-primary": riff((b"ANIM", b"\x00" * 6)),
            "duplicate-record": duplicate,
            "contradictory-record": contradictory,
            "bad-vp8-start-code": riff((b"VP8 ", b"\x00" * 10)),
            "bad-vp8l-signature": riff((b"VP8L", b"\x00" * 5)),
            "animated-chunk": riff((b"ANIM", b"\x00" * 6)),
            "animated-vp8x": riff((b"VP8X", b"\x02\x00\x00\x00" + b"\x00" * 6)),
        }
        for name, value in cases.items():
            with self.subTest(name=name):
                with self.assertRaises(ledger.LedgerError):
                    ledger.image_dimensions(value)


class LedgerContractTests(unittest.TestCase):
    def setUp(self):
        self.case = Case(self._testMethodName)

    def tearDown(self):
        self.case.close()

    def assertSixValues(self, output):
        self.assertTrue(output.endswith(b"\0"))
        values = output[:-1].split(b"\0")
        self.assertEqual(len(values), 6)
        return [value.decode("utf-8") for value in values]

    def test_create_writes_canonical_closed_ledger_and_six_nul_values(self):
        status, output, error = self.case.cli(self.case.create_args())
        self.assertEqual((status, error), (0, ""))
        values = self.assertSixValues(output)
        expected = {
            "version": 1,
            "visual_input": str(self.case.input),
            "visual_input_sha256": hashlib.sha256(
                self.case.input.read_bytes()
            ).hexdigest(),
            "visual_manifest": str(self.case.manifest),
            "visual_manifest_sha256": hashlib.sha256(
                self.case.manifest.read_bytes()
            ).hexdigest(),
            "source_gate_fixture_manifest_sha256": hashlib.sha256(
                self.case.fixture.read_bytes()
            ).hexdigest(),
            "evidence_root": str(self.case.evidence_root),
        }
        expected_bytes = (
            json.dumps(
                expected,
                sort_keys=True,
                separators=(",", ":"),
                ensure_ascii=False,
            )
            + "\n"
        ).encode("utf-8")
        self.assertEqual(self.case.ledger_path.read_bytes(), expected_bytes)
        self.assertEqual(stat.S_IMODE(self.case.evidence_root.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(self.case.ledger_path.stat().st_mode), 0o600)
        self.assertEqual(
            values,
            [
                str(self.case.input),
                expected["visual_input_sha256"],
                str(self.case.manifest),
                expected["visual_manifest_sha256"],
                str(self.case.evidence_root),
                expected["source_gate_fixture_manifest_sha256"],
            ],
        )

    def test_rehydrate_recomputes_hashes_and_emits_create_values(self):
        create_status, create_output, _ = self.case.cli(self.case.create_args())
        status, output, error = self.case.cli(self.case.rehydrate_args())
        self.assertEqual((create_status, status, error), (0, 0, ""))
        self.assertEqual(output, create_output)

    def test_shell_metacharacters_are_data_and_do_not_execute(self):
        odd = self.case.root / "odd ' \" $() `touch PWNED` ; [*?] path"
        odd.mkdir()
        self.case.input = odd / "input $().jpeg"
        self.case.input.write_bytes(jpeg())
        self.case.manifest = odd / "manifest `x`.json"
        self.case._write_manifest()
        sentinel = self.case.repo / "PWNED"
        status, output, _ = self.case.cli(self.case.create_args())
        self.assertEqual(status, 0)
        self.assertSixValues(output)
        self.assertFalse(sentinel.exists())

    def test_fresh_bash_rehydrate_preserves_exact_nul_bytes(self):
        hostile = self.case.root / "hostile ' \" $() `touch PWNED` ; [*?]"
        hostile.mkdir()
        moved_base = hostile / "evidence ' \" $() `touch PWNED` ; [*?]"
        self.case.base.rename(moved_base)
        self.case.base = moved_base
        moved_input = hostile / "input ' \" $() `touch PWNED` ; [*?].jpeg"
        self.case.input.rename(moved_input)
        self.case.input = moved_input
        self.case.manifest = hostile / "manifest ' \" $() `touch PWNED` ; [*?].json"
        self.case._write_manifest()
        script = str(Path(ledger.__file__).resolve())
        environment = {
            **os.environ,
            "HANONLY_SHARED_EVIDENCE_BASE": str(self.case.base),
        }
        created = subprocess.run(
            ["python3", script, *self.case.create_args()],
            cwd=self.case.repo,
            env=environment,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        rehydrated = subprocess.run(
            [
                "bash",
                "-c",
                'exec python3 "$1" rehydrate --repo-root "$2" --evidence-root "$3"',
                "hanonly-ledger-test",
                script,
                str(self.case.repo),
                str(self.case.evidence_root),
            ],
            cwd=self.case.repo,
            env=environment,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self.assertEqual(rehydrated.stdout, created.stdout)
        self.assertSixValues(rehydrated.stdout)
        self.assertFalse((self.case.repo / "PWNED").exists())
        self.assertFalse((self.case.root / "PWNED").exists())

    def test_create_rejects_pre_mutation_base_and_run_id_errors(self):
        cases = []
        relative = self.case.create_args()
        relative[relative.index("--expected-base") + 1] = "relative"
        cases.append(("relative-base", relative))
        root = self.case.create_args()
        root[root.index("--expected-base") + 1] = "/"
        cases.append(("root-base", root))
        bad_run = self.case.create_args()
        bad_run[bad_run.index("--run-id") + 1] = "../bad"
        cases.append(("bad-run", bad_run))
        for name, args in cases:
            with self.subTest(name=name):
                status, output, _ = self.case.cli(args)
                self.assertNotEqual(status, 0)
                self.assertEqual(output, b"")
        self.assertFalse(self.case.evidence_root.exists())

    def test_create_rejects_wrong_base_mode_or_owner_before_mutation(self):
        os.chmod(self.case.base, 0o755)
        status, output, _ = self.case.cli(self.case.create_args())
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")
        os.chmod(self.case.base, 0o700)
        with mock.patch.object(ledger.os, "geteuid", return_value=os.geteuid() + 1):
            status, output, _ = self.case.cli(self.case.create_args())
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")
        self.assertFalse(self.case.evidence_root.exists())

    def test_create_rejects_symlinked_expected_base_without_mutation(self):
        linked_base = self.case.root / "linked-evidence"
        linked_base.symlink_to(self.case.base, target_is_directory=True)
        args = self.case.create_args()
        args[args.index("--expected-base") + 1] = str(linked_base)
        old = os.getcwd()
        os.chdir(self.case.repo)
        try:
            with mock.patch.dict(
                os.environ,
                {"HANONLY_SHARED_EVIDENCE_BASE": str(linked_base)},
                clear=False,
            ):
                stdout = io.BytesIO()
                status = ledger.main(args, stdout=stdout, stderr=io.StringIO())
        finally:
            os.chdir(old)
        self.assertNotEqual(status, 0)
        self.assertEqual(stdout.getvalue(), b"")
        self.assertFalse(self.case.evidence_root.exists())
        self.assertTrue(linked_base.is_symlink())

    def test_create_rejects_worktree_contained_base(self):
        contained = self.case.repo / "external"
        contained.mkdir(mode=0o700)
        args = self.case.create_args()
        args[args.index("--expected-base") + 1] = str(contained)
        with mock.patch.dict(
            os.environ,
            {"HANONLY_SHARED_EVIDENCE_BASE": str(contained)},
            clear=False,
        ):
            old = os.getcwd()
            os.chdir(self.case.repo)
            try:
                status = ledger.main(args, stdout=io.BytesIO(), stderr=io.StringIO())
            finally:
                os.chdir(old)
        self.assertNotEqual(status, 0)
        self.assertFalse((contained / self.case.run_id).exists())

    def test_create_rejects_symlinked_input_manifest_or_fixture(self):
        for target in ("input", "manifest", "fixture"):
            with self.subTest(target=target):
                case = Case(f"symlink-{target}")
                path = {
                    "input": case.input,
                    "manifest": case.manifest,
                    "fixture": case.fixture,
                }[target]
                held = path.with_name(path.name + ".held")
                path.rename(held)
                path.symlink_to(held)
                try:
                    status, output, _ = case.cli(case.create_args())
                finally:
                    case.close()
                self.assertNotEqual(status, 0)
                self.assertEqual(output, b"")

    def test_create_rejects_an_intermediate_symlink(self):
        real = self.case.root / "real"
        real.mkdir()
        linked = self.case.root / "linked"
        linked.symlink_to(real, target_is_directory=True)
        candidate = real / "input.jpeg"
        candidate.write_bytes(jpeg())
        args = self.case.create_args()
        args[args.index("--input") + 1] = str(linked / "input.jpeg")
        args[args.index("--expected-input-sha256") + 1] = hashlib.sha256(
            candidate.read_bytes()
        ).hexdigest()
        status, output, _ = self.case.cli(args)
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")
        self.assertFalse(self.case.evidence_root.exists())

    def test_create_rejects_dirty_or_untracked_fixed_fixture(self):
        self.case.fixture.write_text('{"fixtures":[]}\n', encoding="utf-8")
        status, output, _ = self.case.cli(self.case.create_args())
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")
        self.assertFalse(self.case.evidence_root.exists())

    def test_create_rejects_actually_untracked_fixed_fixture_without_mutation(self):
        subprocess.run(
            [
                "git",
                "rm",
                "--cached",
                "-q",
                "--",
                ledger.FIXTURE_RELATIVE_PATH,
            ],
            cwd=self.case.repo,
            check=True,
        )
        fixture_bytes = self.case.fixture.read_bytes()
        status, output, _ = self.case.cli(self.case.create_args())
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")
        self.assertFalse(self.case.evidence_root.exists())
        self.assertEqual(self.case.fixture.read_bytes(), fixture_bytes)
        untracked = subprocess.run(
            [
                "git",
                "status",
                "--porcelain=v1",
                "--",
                ledger.FIXTURE_RELATIVE_PATH,
            ],
            cwd=self.case.repo,
            check=True,
            stdout=subprocess.PIPE,
        ).stdout
        self.assertIn(b"?? ", untracked)

    def test_create_rejects_index_flags_hiding_dirty_fixture(self):
        for flag in ("--assume-unchanged", "--skip-worktree"):
            with self.subTest(flag=flag):
                case = Case(f"create-hidden-{flag}")
                try:
                    case.hide_fixture_change(flag)
                    result = case.subprocess_cli(case.create_args())
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(result.stdout, b"")
                    self.assertFalse(case.evidence_root.exists())
                finally:
                    case.close()

    def test_create_rejects_fixture_index_flags_without_content_drift(self):
        for flag in ("--assume-unchanged", "--skip-worktree"):
            with self.subTest(flag=flag):
                case = Case(f"create-flag-{flag}")
                try:
                    subprocess.run(
                        [
                            "git",
                            "update-index",
                            flag,
                            "--",
                            ledger.FIXTURE_RELATIVE_PATH,
                        ],
                        cwd=case.repo,
                        check=True,
                    )
                    result = case.subprocess_cli(case.create_args())
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(result.stdout, b"")
                    self.assertFalse(case.evidence_root.exists())
                finally:
                    case.close()

    def test_fixture_cleanliness_compares_held_bytes_to_index_blob(self):
        held_bytes = self.case.fixture.read_bytes() + b"not the index blob"
        with self.assertRaises(ledger.LedgerError):
            ledger._fixture_is_tracked_and_clean(
                str(self.case.repo),
                held_bytes,
                hashlib.sha256(held_bytes).hexdigest(),
            )

    def test_create_rejects_hash_dimension_and_regression_manifest_mismatch(self):
        cases = []
        bad_hash = self.case.create_args()
        bad_hash[bad_hash.index("--expected-input-sha256") + 1] = "0" * 64
        cases.append(("hash", bad_hash))
        bad_size = self.case.create_args()
        bad_size[bad_size.index("--expected-input-size") + 1] = "791x1023"
        cases.append(("size", bad_size))
        self.case._write_manifest(sha="0" * 64)
        cases.append(("manifest", self.case.create_args()))
        for name, args in cases:
            with self.subTest(name=name):
                status, output, _ = self.case.cli(args)
                self.assertNotEqual(status, 0)
                self.assertEqual(output, b"")
        self.assertFalse(self.case.evidence_root.exists())

    def test_create_rejects_nul_cr_and_lf_path_data_without_mutation(self):
        cases = (
            ("nul-input", "--input", "\x00"),
            ("cr-manifest", "--manifest", "\r"),
            ("lf-fixture", "--source-gate-fixture-manifest", "\n"),
        )
        for name, flag, suffix in cases:
            with self.subTest(name=name):
                args = self.case.create_args()
                args[args.index(flag) + 1] += suffix
                status, output, _ = self.case.cli(args)
                self.assertNotEqual(status, 0)
                self.assertEqual(output, b"")
        self.assertFalse(self.case.evidence_root.exists())

    def test_create_recovers_from_empty_temp_and_final_states(self):
        with self.case.environment():
            self.case.evidence_root.mkdir(mode=0o700)
        status, output, _ = self.case.cli(self.case.create_args())
        self.assertEqual(status, 0)
        expected = self.case.ledger_path.read_bytes()
        temp_name = f".evidence-ledger.{hashlib.sha256(expected).hexdigest()}.tmp"
        self.case.ledger_path.unlink()
        (self.case.evidence_root / temp_name).write_bytes(expected[:7])
        os.chmod(self.case.evidence_root / temp_name, 0o600)
        status, output, _ = self.case.cli(self.case.create_args())
        self.assertEqual(status, 0)
        self.assertSixValues(output)
        self.assertEqual(self.case.ledger_path.read_bytes(), expected)
        self.assertFalse((self.case.evidence_root / temp_name).exists())
        status, repeat, _ = self.case.cli(self.case.create_args())
        self.assertEqual(status, 0)
        self.assertEqual(repeat, output)

    def test_create_rejects_unknown_or_mismatched_existing_root(self):
        self.case.evidence_root.mkdir(mode=0o700)
        (self.case.evidence_root / "foreign").write_text("x", encoding="utf-8")
        status, output, _ = self.case.cli(self.case.create_args())
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")
        self.assertTrue((self.case.evidence_root / "foreign").exists())

    def test_create_preserves_a_mismatched_existing_final_ledger(self):
        self.case.evidence_root.mkdir(mode=0o700)
        mismatched = b'{"version":1,"mismatch":true}\n'
        self.case.ledger_path.write_bytes(mismatched)
        os.chmod(self.case.ledger_path, 0o600)
        inode = self.case.ledger_path.stat().st_ino
        status, output, _ = self.case.cli(self.case.create_args())
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")
        self.assertEqual(self.case.ledger_path.read_bytes(), mismatched)
        self.assertEqual(self.case.ledger_path.stat().st_ino, inode)
        self.assertEqual(
            [entry.name for entry in self.case.evidence_root.iterdir()],
            ["evidence-ledger.json"],
        )

    def test_create_preserves_exact_final_ledger_and_extra_entry(self):
        status, _, _ = self.case.cli(self.case.create_args())
        self.assertEqual(status, 0)
        ledger_bytes = self.case.ledger_path.read_bytes()
        ledger_inode = self.case.ledger_path.stat().st_ino
        foreign = self.case.evidence_root / "foreign-entry"
        foreign.write_bytes(b"foreign")
        foreign_inode = foreign.stat().st_ino
        status, output, _ = self.case.cli(self.case.create_args())
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")
        self.assertEqual(self.case.ledger_path.read_bytes(), ledger_bytes)
        self.assertEqual(self.case.ledger_path.stat().st_ino, ledger_inode)
        self.assertEqual(foreign.read_bytes(), b"foreign")
        self.assertEqual(foreign.stat().st_ino, foreign_inode)
        self.assertEqual(
            {entry.name for entry in self.case.evidence_root.iterdir()},
            {"evidence-ledger.json", "foreign-entry"},
        )

    def test_create_does_not_create_a_missing_shared_base(self):
        missing = self.case.root / "missing-base"
        args = self.case.create_args()
        args[args.index("--expected-base") + 1] = str(missing)
        old = os.getcwd()
        os.chdir(self.case.repo)
        try:
            with mock.patch.dict(
                os.environ,
                {"HANONLY_SHARED_EVIDENCE_BASE": str(missing)},
                clear=False,
            ):
                status = ledger.main(args, stdout=io.BytesIO(), stderr=io.StringIO())
        finally:
            os.chdir(old)
        self.assertNotEqual(status, 0)
        self.assertFalse(missing.exists())

    def test_fifo_preflight_inputs_fail_within_timeout_without_mutation(self):
        for target in ("input", "manifest", "fixture"):
            with self.subTest(target=target):
                case = Case(f"fifo-{target}")
                try:
                    args = case.create_args()
                    path = {
                        "input": case.input,
                        "manifest": case.manifest,
                        "fixture": case.fixture,
                    }[target]
                    path.unlink()
                    os.mkfifo(path, 0o600)
                    result = case.subprocess_cli(args, timeout=1)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(result.stdout, b"")
                    self.assertTrue(stat.S_ISFIFO(path.lstat().st_mode))
                    self.assertFalse(case.evidence_root.exists())
                finally:
                    case.close()

    def test_fifo_recovery_children_fail_within_timeout_without_mutation(self):
        for target in ("final", "temp"):
            with self.subTest(target=target):
                case = Case(f"fifo-recovery-{target}")
                try:
                    if target == "final":
                        case.evidence_root.mkdir(mode=0o700)
                        fifo = case.ledger_path
                    else:
                        status, _, _ = case.cli(case.create_args())
                        self.assertEqual(status, 0)
                        expected = case.ledger_path.read_bytes()
                        case.ledger_path.unlink()
                        name = f".evidence-ledger.{hashlib.sha256(expected).hexdigest()}.tmp"
                        fifo = case.evidence_root / name
                    os.mkfifo(fifo, 0o600)
                    before = [
                        (entry.name, entry.lstat().st_mode)
                        for entry in case.evidence_root.iterdir()
                    ]
                    result = case.subprocess_cli(case.create_args(), timeout=1)
                    after = [
                        (entry.name, entry.lstat().st_mode)
                        for entry in case.evidence_root.iterdir()
                    ]
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(result.stdout, b"")
                    self.assertEqual(after, before)
                    self.assertTrue(stat.S_ISFIFO(fifo.lstat().st_mode))
                finally:
                    case.close()

    def test_faults_emit_zero_output_and_identical_retry_converges(self):
        points = (
            "run_creation",
            "partial_write",
            "temp_fsync",
            "rename",
            "final_file_fsync",
            "run_directory_fsync",
            "base_directory_fsync",
        )
        for point in points:
            with self.subTest(point=point):
                case = Case(point)
                fired = False

                def fail(current):
                    nonlocal fired
                    if current == point and not fired:
                        fired = True
                        raise OSError(f"injected {point}")

                try:
                    status, output, _ = case.cli(case.create_args(), fail)
                    self.assertNotEqual(status, 0)
                    self.assertEqual(output, b"")
                    status, output, error = case.cli(case.create_args())
                    self.assertEqual((status, error), (0, ""))
                    self.assertSixValues(output)
                    self.assertEqual(
                        [path.name for path in case.evidence_root.iterdir()],
                        ["evidence-ledger.json"],
                    )
                finally:
                    case.close()

    def test_fsync_checkpoints_follow_required_order(self):
        seen = []
        status, _, _ = self.case.cli(self.case.create_args(), seen.append)
        self.assertEqual(status, 0)
        ordered = [
            "partial_write",
            "temp_fsync",
            "rename",
            "final_file_fsync",
            "run_directory_fsync",
            "base_directory_fsync",
        ]
        self.assertEqual([point for point in seen if point in ordered], ordered)

    def test_namespace_replacement_after_each_identity_check_emits_zero_values(self):
        targets = ("repo", "input", "manifest", "fixture", "base", "run", "ledger")
        for target in targets:
            with self.subTest(target=target):
                case = Case(f"race-{target}")
                replaced = False

                def race(point):
                    nonlocal replaced
                    if point != f"identity_checked:{target}" or replaced:
                        return
                    replaced = True
                    path = {
                        "input": case.input,
                        "manifest": case.manifest,
                        "fixture": case.fixture,
                        "repo": case.repo,
                        "base": case.base,
                        "run": case.evidence_root,
                        "ledger": case.ledger_path,
                    }[target]
                    old = path.with_name(path.name + ".held")
                    path.rename(old)
                    if target in {"repo", "base", "run"}:
                        path.mkdir(mode=0o700)
                    else:
                        path.write_bytes(b"replacement")
                        if target == "ledger":
                            os.chmod(path, 0o600)

                try:
                    status, output, _ = case.cli(case.create_args(), race)
                    self.assertNotEqual(status, 0)
                    self.assertEqual(output, b"")
                finally:
                    case.close()

    def test_namespace_replacement_immediately_before_output_emits_zero_values(self):
        replaced = False

        def race(point):
            nonlocal replaced
            if point != "immediately_before_output" or replaced:
                return
            replaced = True
            held = self.case.input.with_name("input.held")
            self.case.input.rename(held)
            self.case.input.write_bytes(b"replacement")

        status, output, _ = self.case.cli(self.case.create_args(), race)
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")

    def test_create_same_inode_content_mutation_emits_zero_values(self):
        targets = ("input", "manifest", "fixture", "ledger")
        for target in targets:
            with self.subTest(target=target):
                case = Case(f"create-same-inode-{target}")
                mutated = False
                try:

                    def race(point):
                        nonlocal mutated
                        if point != "immediately_before_output" or mutated:
                            return
                        mutated = True
                        path = {
                            "input": case.input,
                            "manifest": case.manifest,
                            "fixture": case.fixture,
                            "ledger": case.ledger_path,
                        }[target]
                        inode = path.stat().st_ino
                        path.write_bytes(path.read_bytes() + b"x")
                        self.assertEqual(path.stat().st_ino, inode)

                    status, output, _ = case.cli(case.create_args(), race)
                    self.assertNotEqual(status, 0)
                    self.assertEqual(output, b"")
                finally:
                    case.close()

    def test_rehydrate_rejects_closed_schema_hash_and_root_drift(self):
        status, _, _ = self.case.cli(self.case.create_args())
        self.assertEqual(status, 0)
        original = json.loads(self.case.ledger_path.read_text(encoding="utf-8"))
        mutations = {
            "extra-key": {**original, "extra": True},
            "missing-key": {k: v for k, v in original.items() if k != "version"},
            "bad-version": {**original, "version": 2},
            "bad-hash": {**original, "visual_input_sha256": "bad"},
            "wrong-root": {**original, "evidence_root": str(self.case.base / "other")},
        }
        for name, value in mutations.items():
            with self.subTest(name=name):
                self.case.ledger_path.write_bytes(ledger.canonical_json(value))
                status, output, _ = self.case.cli(self.case.rehydrate_args())
                self.assertNotEqual(status, 0)
                self.assertEqual(output, b"")
        self.case.ledger_path.write_bytes(ledger.canonical_json(original))

    def test_rehydrate_rejects_input_manifest_fixture_and_base_drift(self):
        for target in ("input", "manifest", "fixture", "base-env"):
            with self.subTest(target=target):
                case = Case(f"drift-{target}")
                try:
                    status, _, _ = case.cli(case.create_args())
                    self.assertEqual(status, 0)
                    if target == "base-env":
                        other = case.root / "other"
                        other.mkdir(mode=0o700)
                        old = os.getcwd()
                        os.chdir(case.repo)
                        try:
                            with mock.patch.dict(
                                os.environ,
                                {"HANONLY_SHARED_EVIDENCE_BASE": str(other)},
                                clear=False,
                            ):
                                stdout = io.BytesIO()
                                status = ledger.main(
                                    case.rehydrate_args(),
                                    stdout=stdout,
                                    stderr=io.StringIO(),
                                )
                        finally:
                            os.chdir(old)
                        output = stdout.getvalue()
                    else:
                        path = {
                            "input": case.input,
                            "manifest": case.manifest,
                            "fixture": case.fixture,
                        }[target]
                        path.write_bytes(path.read_bytes() + b"x")
                        status, output, _ = case.cli(case.rehydrate_args())
                    self.assertNotEqual(status, 0)
                    self.assertEqual(output, b"")
                finally:
                    case.close()

    def test_rehydrate_rejects_index_flags_hiding_dirty_fixture(self):
        for flag in ("--assume-unchanged", "--skip-worktree"):
            with self.subTest(flag=flag):
                case = Case(f"rehydrate-hidden-{flag}")
                try:
                    status, _, _ = case.cli(case.create_args())
                    self.assertEqual(status, 0)
                    case.hide_fixture_change(flag)
                    result = case.subprocess_cli(case.rehydrate_args())
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(result.stdout, b"")
                finally:
                    case.close()

    def test_rehydrate_rejects_root_or_ledger_symlinks_and_wrong_metadata(self):
        cases = ("root-symlink", "ledger-symlink", "root-mode", "ledger-mode", "owner")
        for target in cases:
            with self.subTest(target=target):
                case = Case(target)
                try:
                    status, _, _ = case.cli(case.create_args())
                    self.assertEqual(status, 0)
                    patch = contextlib.nullcontext()
                    if target == "root-symlink":
                        held = case.evidence_root.with_name(
                            case.evidence_root.name + ".held"
                        )
                        case.evidence_root.rename(held)
                        case.evidence_root.symlink_to(held, target_is_directory=True)
                    elif target == "ledger-symlink":
                        held = case.ledger_path.with_name("ledger.held")
                        case.ledger_path.rename(held)
                        case.ledger_path.symlink_to(held)
                    elif target == "root-mode":
                        os.chmod(case.evidence_root, 0o755)
                    elif target == "ledger-mode":
                        os.chmod(case.ledger_path, 0o644)
                    else:
                        patch = mock.patch.object(
                            ledger.os,
                            "geteuid",
                            return_value=os.geteuid() + 1,
                        )
                    with patch:
                        status, output, _ = case.cli(case.rehydrate_args())
                    self.assertNotEqual(status, 0)
                    self.assertEqual(output, b"")
                finally:
                    case.close()

    def test_rehydrate_namespace_replacement_emits_zero_values(self):
        targets = ("repo", "input", "manifest", "fixture", "base", "run", "ledger")
        for target in targets:
            with self.subTest(target=target):
                case = Case(f"rehydrate-race-{target}")
                replaced = False
                try:
                    status, _, _ = case.cli(case.create_args())
                    self.assertEqual(status, 0)

                    def race(point):
                        nonlocal replaced
                        if point != f"identity_checked:{target}" or replaced:
                            return
                        replaced = True
                        path = {
                            "repo": case.repo,
                            "input": case.input,
                            "manifest": case.manifest,
                            "fixture": case.fixture,
                            "base": case.base,
                            "run": case.evidence_root,
                            "ledger": case.ledger_path,
                        }[target]
                        held = path.with_name(path.name + ".held")
                        path.rename(held)
                        if target in {"repo", "base", "run"}:
                            path.mkdir(mode=0o700)
                        else:
                            path.write_bytes(b"replacement")
                            if target == "ledger":
                                os.chmod(path, 0o600)

                    status, output, _ = case.cli(case.rehydrate_args(), race)
                    self.assertNotEqual(status, 0)
                    self.assertEqual(output, b"")
                finally:
                    case.close()

    def test_rehydrate_same_inode_content_mutation_emits_zero_values(self):
        targets = ("input", "manifest", "fixture", "ledger")
        for target in targets:
            with self.subTest(target=target):
                case = Case(f"rehydrate-same-inode-{target}")
                mutated = False
                try:
                    status, _, _ = case.cli(case.create_args())
                    self.assertEqual(status, 0)

                    def race(point):
                        nonlocal mutated
                        if point != "immediately_before_output" or mutated:
                            return
                        mutated = True
                        path = {
                            "input": case.input,
                            "manifest": case.manifest,
                            "fixture": case.fixture,
                            "ledger": case.ledger_path,
                        }[target]
                        inode = path.stat().st_ino
                        path.write_bytes(path.read_bytes() + b"x")
                        self.assertEqual(path.stat().st_ino, inode)

                    status, output, _ = case.cli(case.rehydrate_args(), race)
                    self.assertNotEqual(status, 0)
                    self.assertEqual(output, b"")
                finally:
                    case.close()

    def test_startup_rejects_missing_descriptor_capability_before_mutation(self):
        with mock.patch.object(ledger, "_platform_capabilities", return_value=False):
            status, output, _ = self.case.cli(self.case.create_args())
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")
        self.assertFalse(self.case.evidence_root.exists())


class B0ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="hanonly b0 artifact ")
        self.root = Path(self.temp.name).resolve()
        self.repo = self.root / "repo"
        self.repo.mkdir()
        checker = self.repo / ledger.B0_CHECKER_ENDPOINT
        checker.parent.mkdir(parents=True)
        checker.write_bytes(
            Path(__file__).with_name("check-hanonly-production-policy.ts").read_bytes()
        )
        self.path = self.root / "artifact.json"
        self.value = b0_artifact()
        self.write_raw_logs()
        self.write_required_checks()

    def tearDown(self):
        self.temp.cleanup()

    def args(self, **overrides):
        values = {
            "b0_sha": self.value["b0_sha"],
            "visual_manifest_sha256": self.value["manifest_sha256"],
            "source_gate_fixture_manifest_sha256": self.value[
                "source_gate_fixture_manifest_sha256"
            ],
        }
        values.update(overrides)
        return [
            "validate-b0-artifact",
            "--repo-root",
            str(self.repo),
            "--artifact",
            str(self.path),
            "--b0-sha",
            values["b0_sha"],
            "--visual-manifest-sha256",
            values["visual_manifest_sha256"],
            "--source-gate-fixture-manifest-sha256",
            values["source_gate_fixture_manifest_sha256"],
        ]

    def run_artifact(self, raw=None, **overrides):
        self.path.write_bytes(ledger.canonical_json(self.value) if raw is None else raw)
        stdout = io.BytesIO()
        stderr = io.StringIO()
        status = ledger.main(self.args(**overrides), stdout=stdout, stderr=stderr)
        return status, stdout.getvalue(), stderr.getvalue()

    def write_raw_logs(self):
        relpaths = {
            process["load_evidence"]["raw_load_log_relpath"]
            for process in self.value["process_evidence"]
        }
        relpaths.update(
            result["execution_evidence"]["raw_inference_log_relpath"]
            for result in self.value["calibration_results"]
            + self.value["holdout_results"]
        )
        relpaths.update(
            result["execution_evidence"]["source_gate_diagnostic_relpath"]
            for result in self.value["calibration_results"]
            + self.value["holdout_results"]
        )
        for relpath in relpaths:
            path = self.path.parent / relpath
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(B0_RAW_LOG_BYTES)
            os.chmod(path, 0o600)

    def write_required_checks(self):
        directory = self.path.parent / "source-gate-selection/checks"
        directory.mkdir(parents=True, exist_ok=True)
        os.chmod(directory.parent, 0o700)
        os.chmod(directory, 0o700)
        for check in self.value["required_checks"]:
            attestation = b0_attestation(
                check["phase"],
                self.value["b0_sha"],
                self.value["manifest_sha256"],
                self.value["source_gate_fixture_manifest_sha256"],
                check["checker_endpoint_sha256"],
            )
            path = self.path.parent / check["attestation_relpath"]
            path.write_bytes(ledger.canonical_json(attestation))
            os.chmod(path, 0o600)

    def assert_rejected(self, raw=None, **overrides):
        status, output, _ = self.run_artifact(raw, **overrides)
        self.assertNotEqual(status, 0)
        self.assertEqual(output, b"")

    def test_accepts_valid_frozen_artifact(self):
        status, output, error = self.run_artifact()
        self.assertEqual((status, output, error), (0, b"PASS B0 frozen artifact\n", ""))

    def test_rejects_noncanonical_artifact(self):
        self.assert_rejected(json.dumps(self.value).encode())

    def test_rejects_wrong_plan_revisions(self):
        for revision in (*range(29, 49), "49", None):
            with self.subTest(revision=revision):
                self.value = b0_artifact()
                self.value["plan_revision"] = revision
                self.assert_rejected()

    def test_rejects_version_one_and_legacy_candidate_schema(self):
        self.value["version"] = 1
        self.assert_rejected()
        self.value = b0_artifact()
        self.value["candidates"][0] = {
            "id": "R0",
            "numerator": 0,
            "denominator": 1,
        }
        self.assert_rejected()

    def test_rejects_missing_cell_and_wrong_feature_order(self):
        self.value["calibration_results"].pop()
        self.assert_rejected()
        self.value = b0_artifact()
        self.value["enabled_cargo_features"].reverse()
        self.assert_rejected()

    def test_rejects_retuning_after_freeze(self):
        self.value["retuned_after_freeze"] = True
        self.assert_rejected()

    def test_rejects_invalid_b0_timestamps(self):
        self.value["holdout_completed_at_utc"] = "2026-07-26T00:01:00+00:00"
        self.assert_rejected()
        self.value = b0_artifact()
        self.value["holdout_completed_at_utc"] = self.value["frozen_at_utc"]
        self.assert_rejected()

    def test_rejects_frozen_projection_hash_drift(self):
        self.value["selected_candidate_id"] = "S25L6"
        self.assert_rejected()

    def test_rejects_required_check_and_recall_contract_drift(self):
        self.value["required_checks"].reverse()
        self.assert_rejected()
        self.value = b0_artifact()
        self.value["frozen_recall_contract"]["coverage_acceptance_rule_sha256"] = (
            "3" * 64
        )
        self.assert_rejected()

    def test_rejects_metal_default_gpu_layer_drift(self):
        self.value["process_evidence"][3]["load_evidence"]["n_gpu_layers"] = 32
        self.assert_rejected()

    def test_rejects_holdout_process_fingerprint_drift(self):
        self.value["process_evidence"][3]["executable_sha256"] = "3" * 64
        self.assert_rejected()

    def test_rejects_missing_or_drifting_raw_logs(self):
        relpath = self.value["process_evidence"][0]["load_evidence"][
            "raw_load_log_relpath"
        ]
        (self.path.parent / relpath).unlink()
        self.assert_rejected()
        self.write_raw_logs()
        self.value["process_evidence"][0]["load_evidence"]["raw_load_log_sha256"] = (
            "3" * 64
        )
        self.assert_rejected()

    def test_rejects_insecure_or_escaping_raw_logs(self):
        relpath = self.value["holdout_results"][0]["execution_evidence"][
            "raw_inference_log_relpath"
        ]
        os.chmod(self.path.parent / relpath, 0o644)
        self.assert_rejected()
        self.write_raw_logs()
        self.value["holdout_results"][0]["execution_evidence"][
            "raw_inference_log_relpath"
        ] = "../escape.log"
        self.assert_rejected()

    def test_rejects_vacuous_cpu_and_instance_mismatch(self):
        for mutation in ("empty-devices", "zero-buffer", "instance"):
            with self.subTest(mutation=mutation):
                self.value = b0_artifact()
                if mutation == "empty-devices":
                    self.value["process_evidence"][0]["load_evidence"][
                        "loaded_model_devices"
                    ] = []
                elif mutation == "zero-buffer":
                    self.value["process_evidence"][0]["load_evidence"][
                        "model_buffer_bytes_by_backend"
                    ]["CPU"] = 0
                else:
                    self.value["calibration_results"][0]["execution_evidence"][
                        "paddle_instance_id"
                    ] = "9" * 32
                self.assert_rejected()

    def test_rejects_unknown_or_missing_nested_keys(self):
        self.value["process_evidence"][0]["load_evidence"]["unknown"] = True
        self.assert_rejected()
        self.value = b0_artifact()
        del self.value["holdout_results"][0]["derived"]["passed"]
        self.assert_rejected()

    def test_rejects_b0_and_manifest_hash_drift(self):
        for field in (
            "b0_sha",
            "manifest_sha256",
            "source_gate_fixture_manifest_sha256",
        ):
            with self.subTest(field=field):
                self.value = b0_artifact()
                argument = (
                    "visual_manifest_sha256" if field == "manifest_sha256" else field
                )
                self.assert_rejected(**{argument: "e" * len(self.value[field])})

    def test_rejects_candidate_ratio_drift(self):
        self.value["candidates"][1]["long_side_denominator"] = 4
        self.assert_rejected()


class R51EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="hanonly r51 evidence ")
        self.root = Path(self.temp.name).resolve()
        os.chmod(self.root, 0o700)

    def tearDown(self):
        self.temp.cleanup()

    def write_private(self, relpath, data):
        path = self.root / relpath
        path.parent.mkdir(parents=True, exist_ok=True)
        for parent in path.parents:
            if parent == self.root.parent:
                break
            os.chmod(parent, 0o700)
        path.write_bytes(data)
        os.chmod(path, 0o600)
        return path

    def calibration_fixture(self):
        value = b0_artifact()
        processes = [
            process
            for process in value["process_evidence"]
            if process["phase"] == "calibration"
        ]
        id_map = dict(zip(value["calibration_entry_ids"], ledger.R51_CALIBRATION_IDS))
        results = value["calibration_results"]
        for result in results:
            result["entry_id"] = id_map[result["entry_id"]]
        for process in processes:
            load = process["load_evidence"]
            self.write_private(load["raw_load_log_relpath"], B0_RAW_LOG_BYTES)
        for result in results:
            execution = result["execution_evidence"]
            self.write_private(execution["raw_inference_log_relpath"], B0_RAW_LOG_BYTES)
            self.write_private(
                execution["source_gate_diagnostic_relpath"], B0_RAW_LOG_BYTES
            )
        payload = {
            "calibration_results": results,
            "selected_candidate_id": ledger.B0_CANDIDATES[0]["id"],
        }
        calibration_ledger = {
            "calibration_entry_ids": ledger.R51_CALIBRATION_IDS,
            "candidates": ledger.B0_CANDIDATES,
            "calibration_results": results,
            "selected_candidate_id": ledger.B0_CANDIDATES[0]["id"],
            "process_evidence": processes,
        }
        return payload, calibration_ledger

    def test_calibration_selects_only_the_first_complete_all_pass_candidate(self):
        payload, calibration_ledger = self.calibration_fixture()
        ledger._r51_validate_calibration(payload, calibration_ledger, str(self.root))
        payload["selected_candidate_id"] = ledger.B0_CANDIDATES[1]["id"]
        calibration_ledger["selected_candidate_id"] = payload["selected_candidate_id"]
        with self.assertRaises(ledger.LedgerError):
            ledger._r51_validate_calibration(
                payload, calibration_ledger, str(self.root)
            )

    def test_calibration_rejects_metal_log_and_terminal_evidence_drift(self):
        mutations = ("metal-device", "load-log", "bare-bool")
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                payload, calibration_ledger = self.calibration_fixture()
                if mutation == "metal-device":
                    metal = calibration_ledger["process_evidence"][1]["load_evidence"]
                    metal["mtmd_backend"] = "CPU"
                elif mutation == "load-log":
                    calibration_ledger["process_evidence"][0]["load_evidence"][
                        "raw_load_log_sha256"
                    ] = "f" * 64
                else:
                    payload["calibration_results"][0]["derived"]["passed"] = False
                with self.assertRaises(ledger.LedgerError):
                    ledger._r51_validate_calibration(
                        payload, calibration_ledger, str(self.root)
                    )

    def test_approved_contract_paths_are_fixed_before_read(self):
        arguments = mock.Mock(repo_root=str(self.root))
        for name, path in ledger.R51_APPROVED_PUBLIC_FILES.items():
            setattr(arguments, name, str(self.root / path))
        arguments.r51_contract = str(self.root / "copied-contract.json")
        with self.assertRaisesRegex(ledger.LedgerError, "path drift"):
            ledger._r51_validate_contract_files(arguments)

    def test_custody_namespace_rejects_mixed_roots_and_any_temp(self):
        names = ledger.R51_CUSTODY_FROZEN_NAMES
        for name in names:
            self.write_private(name, b"{}")
        arguments = mock.Mock(
            historical_inventory=str(self.root / "historical-inventory.json"),
            ciphertext=str(self.root / "holdout.enc"),
            freeze_receipt=str(self.root / "holdout-freeze-receipt.json"),
        )
        ledger._r51_custody_namespace(arguments, authorized=False)
        other = self.root / "other"
        other.mkdir(mode=0o700)
        moved = other / "holdout.enc"
        moved.write_bytes(b"ciphertext")
        os.chmod(moved, 0o600)
        arguments.ciphertext = str(moved)
        with self.assertRaises(ledger.LedgerError):
            ledger._r51_custody_namespace(arguments, authorized=False)
        arguments.ciphertext = str(self.root / "holdout.enc")
        self.write_private(".holdout-failure.deadbeef.tmp", b"failure")
        with self.assertRaises(ledger.LedgerError):
            ledger._r51_custody_namespace(arguments, authorized=False)

    def test_preflight_custody_snapshot_is_stable_and_formal_rerun_fails_closed(
        self,
    ):
        repo_root = Path(ledger.__file__).resolve().parent.parent
        historical = {
            "contract": "hanonly-r51-historical-inventory-v1",
            "plan_revision": 51,
        }
        historical_bytes = ledger._r51_canonical_json(historical)
        ciphertext = b"public encrypted holdout"
        iv = bytes.fromhex("01" * 16)
        header = {
            "contract": "hanonly-r51-encrypted-holdout-header-v1",
            "plan_revision": 51,
            "cipher": "aes-256-ctr",
            "integrity": "hmac-sha256-etm-v1",
            "iv_hex": iv.hex(),
            "ciphertext_byte_length": len(ciphertext),
            "plaintext_archive_byte_length": 1,
        }
        header_bytes = ledger._r51_canonical_json(header)
        freeze = {
            "contract": "hanonly-r51-encrypted-holdout-freeze-v1",
            "plan_revision": 51,
            "base_b0_sha": "b" * 40,
            "implementation_thread_id": "implementation-thread",
            "frozen_before_production_edit": True,
            "entry_ids": ledger.R51_HOLDOUT_IDS,
            "cipher": header["cipher"],
            "integrity": header["integrity"],
            "iv_sha256": hashlib.sha256(iv).hexdigest(),
            "ciphertext_byte_length": len(ciphertext),
            "ciphertext_sha256": hashlib.sha256(ciphertext).hexdigest(),
            "header_sha256": hashlib.sha256(header_bytes).hexdigest(),
            "hmac_sha256": "1" * 64,
            "plaintext_archive_sha256_commitment": "2" * 64,
            "manifest_sha256_commitment": "3" * 64,
            "oracle_sha256_commitment": "4" * 64,
            "hashes_sha256_commitment": "5" * 64,
            "historical_inventory_sha256": hashlib.sha256(historical_bytes).hexdigest(),
            "formal_source_identities": [],
            "disclosed_challenge_exclusion_pass": True,
            "result": "pass",
        }
        files = {
            "historical-inventory.json": historical_bytes,
            "holdout.enc": ciphertext,
            "holdout-header.json": header_bytes,
            "holdout-freeze-receipt.json": ledger._r51_canonical_json(freeze),
        }
        for name, data in files.items():
            self.write_private(name, data)
        arguments = mock.Mock(
            repo_root=str(repo_root),
            historical_inventory=str(self.root / "historical-inventory.json"),
            ciphertext=str(self.root / "holdout.enc"),
            freeze_receipt=str(self.root / "holdout-freeze-receipt.json"),
        )
        for name, relative_path in ledger.R51_APPROVED_PUBLIC_FILES.items():
            setattr(arguments, name, str(repo_root / relative_path))

        before = ledger._r51_preflight_custody_snapshot(arguments)
        after = ledger._r51_preflight_custody_snapshot(arguments)
        self.assertEqual(before, after)
        self.assertEqual(before["custody_root_mode"], 0o700)
        self.assertEqual(set(before["files"]), ledger.R51_CUSTODY_FROZEN_NAMES)

        os.chmod(self.root / "holdout.enc", 0o644)
        with self.assertRaises(ledger.LedgerError):
            ledger._r51_preflight_custody_snapshot(arguments)
        os.chmod(self.root / "holdout.enc", 0o600)
        self.write_private("holdout-open.json", b"{}")
        with self.assertRaises(ledger.LedgerError):
            ledger._r51_preflight_custody_snapshot(arguments)

    def test_generation_continuity_is_calibration_1_64_then_holdout_65_80(self):
        calibration = [
            {"phase": "calibration-freeze", "state": "passed"} for _ in range(32)
        ]
        first_holdout = {"phase": "holdout", "state": "captured_unclassified"}
        ledger._r51_validate_generation_continuity(64, calibration)
        ledger._r51_validate_generation_continuity(65, [*calibration, first_holdout])
        ledger._r51_validate_generation_continuity(
            66, [*calibration, {**first_holdout, "state": "passed"}]
        )
        with self.assertRaises(ledger.LedgerError):
            ledger._r51_validate_generation_continuity(
                64,
                [
                    *calibration[:-1],
                    {"phase": "holdout", "state": "passed"},
                ],
            )
        with self.assertRaises(ledger.LedgerError):
            ledger._r51_validate_generation_continuity(
                65, [*calibration, {**first_holdout, "state": "passed"}]
            )

    def test_coverage_index_recomputes_binary_rasters_and_rejects_path_reuse(self):
        selected = b"\x01\x00\x01\x00"
        downstream = b"\x01\x01\x01\x00"
        self.write_private("selected.bin", selected)
        self.write_private("downstream.bin", downstream)
        bindings = {
            "b0_sha": "b" * 40,
            "manifest_sha256": "1" * 64,
            "oracle_sha256": "2" * 64,
            "hashes_sha256": "3" * 64,
        }
        proof = {
            "contract": "hanonly-r51-target-coverage-proof-v1",
            "plan_revision": 51,
            "b0_sha": bindings["b0_sha"],
            "cell_key": "holdout/S25L4/cpu/r51-h01",
            "entry_id": "r51-h01",
            "target_id": "opaque-target-1",
            "oracle_mask_raw_sha256": "4" * 64,
            "oracle_mask_normalized_sha256": "5" * 64,
            "page_width": 2,
            "page_height": 2,
            "support_stride_bytes": 2,
            "selected_support_relpath": "selected.bin",
            "selected_support_byte_length": len(selected),
            "selected_support_sha256": hashlib.sha256(selected).hexdigest(),
            "downstream_support_relpath": "downstream.bin",
            "downstream_support_byte_length": len(downstream),
            "downstream_support_sha256": hashlib.sha256(downstream).hexdigest(),
            "oracle_foreground_pixels": 2,
            "selected_support_foreground_pixels": 2,
            "downstream_support_foreground_pixels": 3,
            "selected_covered_pixels": 2,
            "downstream_covered_pixels": 2,
            "missing_selected_pixels": 0,
            "missing_downstream_pixels": 0,
            "protected_overlap_pixels": 0,
            "target_selected": True,
            "result": "pass",
        }
        proof_bytes = ledger._r51_canonical_json(proof)
        self.write_private("proof.json", proof_bytes)
        index = {
            "contract": "hanonly-r51-target-coverage-index-v1",
            "plan_revision": 51,
            "b0_sha": bindings["b0_sha"],
            "cell_key": proof["cell_key"],
            "manifest_sha256": bindings["manifest_sha256"],
            "oracle_sha256": bindings["oracle_sha256"],
            "hashes_sha256": bindings["hashes_sha256"],
            "records": [
                {
                    "entry_id": proof["entry_id"],
                    "target_id": proof["target_id"],
                    "proof_path": "proof.json",
                    "proof_sha256": hashlib.sha256(proof_bytes).hexdigest(),
                    "proof_byte_length": len(proof_bytes),
                }
            ],
        }
        index_bytes = ledger._r51_canonical_json(index)
        self.write_private("coverage.json", index_bytes)
        record = {
            "cell_key": proof["cell_key"],
            "entry_id": proof["entry_id"],
            "target_coverage_index_path": "coverage.json",
            "target_coverage_index_sha256": hashlib.sha256(index_bytes).hexdigest(),
            "target_coverage_index_byte_length": len(index_bytes),
        }
        ledger._r51_validate_coverage_index(str(self.root), record, bindings, 1, set())
        proof["downstream_support_relpath"] = proof["selected_support_relpath"]
        proof["downstream_support_sha256"] = proof["selected_support_sha256"]
        proof["downstream_support_foreground_pixels"] = 2
        proof_bytes = ledger._r51_canonical_json(proof)
        self.write_private("proof-reused.json", proof_bytes)
        index["records"][0]["proof_path"] = "proof-reused.json"
        index["records"][0]["proof_sha256"] = hashlib.sha256(proof_bytes).hexdigest()
        index["records"][0]["proof_byte_length"] = len(proof_bytes)
        index_bytes = ledger._r51_canonical_json(index)
        self.write_private("coverage-reused.json", index_bytes)
        record["target_coverage_index_path"] = "coverage-reused.json"
        record["target_coverage_index_sha256"] = hashlib.sha256(index_bytes).hexdigest()
        record["target_coverage_index_byte_length"] = len(index_bytes)
        with self.assertRaisesRegex(ledger.LedgerError, "reuses"):
            ledger._r51_validate_coverage_index(
                str(self.root), record, bindings, 1, set()
            )

    def test_terminal_receipt_rejects_any_failed_or_unexecuted_cell(self):
        bindings = {
            "b0_sha": "b" * 40,
            "selected_candidate_id": ledger.B0_CANDIDATES[0]["id"],
            "freeze_receipt_sha256": "1" * 64,
            "open_marker_sha256": "2" * 64,
            "ciphertext_sha256": "3" * 64,
            "pre_holdout_attestation_sha256": "4" * 64,
            "bundle_validation_receipt_sha256": "5" * 64,
        }
        cells = [
            {
                "cell_key": f"{entry}/{device}",
                "result": "pass",
                "selection_result": "selected",
                "target_recall": {
                    "target_total": 1,
                    "selected": 1,
                    "covered": 1,
                    "uncovered": 0,
                },
                "pp_han_count": 1,
                "vl_han_count": 1,
                "rejection_reason": None,
                "device_evidence_sha256": "6" * 64,
                "log_sha256": "7" * 64,
                "diagnostic_sha256": "8" * 64,
                "target_coverage_index_sha256": "9" * 64,
            }
            for entry in ledger.R51_HOLDOUT_IDS
            for device in ("cpu", "metal")
        ]
        receipt = {
            "contract": "hanonly-r51-encrypted-holdout-terminal-v1",
            "plan_revision": 51,
            **bindings,
            "terminal_diagnostic_index_sha256": "a" * 64,
            "cell_results": cells,
            "first_failed_cell": None,
            "unexecuted_cell_keys": [],
            "all_cells_terminated": True,
            "all_cells_passed": True,
            "plaintext_removed": True,
            "result": "pass",
        }

        ledger._r51_validate_terminal(receipt, bindings)
        receipt["cell_results"][0]["result"] = "fail"
        with self.assertRaises(ledger.LedgerError):
            ledger._r51_validate_terminal(receipt, bindings)

    def test_publication_is_idempotent_and_rejects_byte_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            os.chmod(directory, 0o700)
            output = os.path.join(os.path.realpath(directory), "r51-b0-preflight.json")
            value = {"contract": "test", "result": "pass"}
            first = ledger._r51_publish(output, value, "R51 test publication")
            second = ledger._r51_publish(output, value, "R51 test publication")
            self.assertEqual(first, second)
            with self.assertRaises(ledger.LedgerError):
                ledger._r51_publish(
                    output,
                    {"contract": "test", "result": "fail"},
                    "R51 test publication",
                )

    def test_staged_red_logs_reject_hash_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            root = os.path.realpath(directory)
            os.chmod(root, 0o700)
            logs = os.path.join(root, "r51-staged-red")
            os.mkdir(logs, 0o700)
            test_id = ledger.EXPECTED_B0_B1_MARKER_IDS[0]
            log = os.path.join(logs, f"{test_id}.log")
            data = b"running 1 test\nFAILED\ntest result: FAILED\n"
            Path(log).write_bytes(data)
            os.chmod(log, 0o600)
            hashes = {test_id: hashlib.sha256(data).hexdigest()}

            ledger._r51_validate_staged_red_logs(root, hashes)
            hashes[test_id] = "0" * 64
            with self.assertRaises(ledger.LedgerError):
                ledger._r51_validate_staged_red_logs(root, hashes)

    def test_support_raster_rejects_non_binary_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = os.path.realpath(directory)
            os.chmod(root, 0o700)
            raster = os.path.join(root, "support.bin")
            data = b"\x00\x02"
            Path(raster).write_bytes(data)
            os.chmod(raster, 0o600)

            with self.assertRaises(ledger.LedgerError):
                ledger._r51_validate_support_raster(
                    root,
                    "support.bin",
                    hashlib.sha256(data).hexdigest(),
                    len(data),
                    2,
                    1,
                    "test support",
                )


class R52EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="hanonly r52 evidence ")
        self.root = Path(self.temp.name).resolve()
        os.chmod(self.root, 0o700)

    def tearDown(self):
        self.temp.cleanup()

    def write_private(self, path, value):
        path.parent.mkdir(parents=True, exist_ok=True)
        for parent in path.parents:
            if parent == self.root.parent:
                break
            os.chmod(parent, 0o700)
        data = value if isinstance(value, bytes) else ledger._r51_canonical_json(value)
        path.write_bytes(data)
        os.chmod(path, 0o600)
        return data

    def projection_fixture(self):
        entries = [
            {"id": entry_id, "role": "calibration", "value": ordinal}
            for ordinal, entry_id in enumerate(ledger.R52_CALIBRATION_IDS)
        ]
        inventory = {
            "assets": {
                f"asset-{ordinal}": {"byte_length": 1, "sha256": "a" * 64}
                for ordinal in range(17)
            },
            "contract": "hanonly-r52-calibration-hashes-v1",
            "manifest_sha256": ledger.R52_CALIBRATION_MANIFEST_SHA256,
            "plan_revision": 52,
        }
        return {"entries": entries, "version": 1}, inventory

    def challenge_cell(self, ordinal):
        entry_id, device = ledger.R52_CHALLENGE_CELL_IDS[ordinal].split("/")
        return {
            "ordinal": ordinal,
            "entry_id": entry_id,
            "device": device,
            "kind": "regression" if ordinal < 8 else "supplemental",
            "candidate_id": "S25L4",
            "selection_result_path": f"selection-{ordinal}.json",
            "selection_result_sha256": "1" * 64,
            "target_recall": (
                {
                    "target_total": 1,
                    "selected": 1,
                    "covered": 1,
                    "uncovered": 0,
                }
                if ordinal < 8
                else None
            ),
            "pp_count": 1,
            "vl_count": 1,
            "rejection_reason": None,
            "diagnostic_path": f"diagnostic-{ordinal}.json",
            "diagnostic_sha256": "2" * 64,
            "process_evidence_path": f"process-{ordinal}.json",
            "process_evidence_sha256": "3" * 64,
            "log_path": f"log-{ordinal}.txt",
            "log_sha256": "4" * 64,
            "result": "pass",
        }

    def write_challenge_cell_evidence(
        self, evidence, ordinal, *, result="pass", b0_sha="b" * 40
    ):
        entry_id, device = ledger.R52_CHALLENGE_CELL_IDS[ordinal].split("/")
        metal = device == "metal"
        load_relpath = f"challenge/{ordinal}/load.log"
        self.write_private(evidence / load_relpath, B0_RAW_LOG_BYTES)
        process = {
            "id": f"challenge/S25L4/{entry_id}/{device}",
            "phase": "challenge",
            "requested_device": device,
            "paddle_instance_id": ("2" if metal else "1") * 32,
            "executable_sha256": HEX64,
            "model_artifact_sha256": {
                name: HEX64
                for name in (
                    "pp_detection",
                    "pp_recognition",
                    "pp_recognition_config",
                    "vl_model",
                    "vl_mmproj",
                )
            },
            "runtime_library_sha256": {"/usr/lib/libsynthetic.dylib": HEX64},
            "load_evidence": {
                "cpu_forced": not metal,
                "gpu_offload_supported": metal,
                "n_gpu_layers": 1000 if metal else 0,
                "mtmd_use_gpu": metal,
                "word_boxes_backend": "rten_cpu",
                "raw_load_log_relpath": load_relpath,
                "raw_load_log_sha256": B0_RAW_LOG_SHA256,
                "enumerated_devices": [],
                "loaded_model_devices": [
                    {
                        "model_device_ordinal": 0,
                        "name": "Apple GPU" if metal else "CPU",
                        "backend": "Metal" if metal else "CPU",
                        "device_type": "integrated_gpu" if metal else "cpu",
                    }
                ],
                "offloaded_layers": 32 if metal else 0,
                "offloadable_layers": 39,
                "model_buffer_bytes_by_backend": {
                    "CPU": 1,
                    **({"Metal": 1} if metal else {}),
                },
                "mtmd_backend": "Metal" if metal else "CPU",
            },
        }
        process_path = f"challenge/{ordinal}/process.json"
        process_bytes = self.write_private(evidence / process_path, process)
        rejection = "pp_no_han_protected_latin" if entry_id == "r49-h04" else None
        target_recall = (
            {
                "target_total": 1,
                "selected": 1,
                "covered": 1,
                "uncovered": 0,
            }
            if ordinal < 8
            else None
        )
        selection = {
            "entry_id": entry_id,
            "process_evidence_id": process["id"],
            "candidate_id": "S25L4",
            "execution_evidence": {
                key: value
                for key, value in {
                    "paddle_instance_id": process["paddle_instance_id"],
                    "context_offload_kqv": metal,
                    "context_op_offload": metal,
                    "inference_completed": True,
                    "raw_inference_log_relpath": f"challenge/{ordinal}/inference.log",
                    "raw_inference_log_sha256": HEX64,
                    "source_gate_diagnostic_relpath": f"challenge/{ordinal}/source.json",
                    "source_gate_diagnostic_sha256": HEX64,
                    "context_buffer_bytes_by_backend": {"CPU": 1},
                    "compute_buffer_bytes_by_backend": {"CPU": 1},
                }.items()
            },
            "runtime_nodes": [],
            "derived": {
                "actual_device": device,
                "matched_target_ids": [],
                "selected_target_ids": [],
                "selected_protected_node_ids": [],
                "selected_rotation_target_ids": [],
                "unmatched_selected_node_ids": [],
                "target_recall": 1.0,
                "protected_false_positive_count": 0,
                "rotation_targets_excluded": True,
                "source_coverage_preflight": {
                    "pp_han_scalar_count": 1,
                    "vl_expected_han_scalar_count": 1,
                    "pp_vl_complete_coverage": rejection is None,
                    "rejected_after_vl": rejection is not None,
                    "pp_vl_incomplete_coverage": False,
                    "covered_source_roi_ids": [],
                    "source_text_roi_coverage": 1.0,
                    "source_removal_preflight_passed": result == "pass",
                },
                "passed": result == "pass",
            },
        }
        selection_path = f"challenge/{ordinal}/selection.json"
        log_path = f"challenge/{ordinal}/run.log"
        log_bytes = self.write_private(evidence / log_path, b"challenge log\n")
        diagnostic = {key: None for key in ledger.R51_CELL_DIAGNOSTIC_KEYS}
        diagnostic.update(
            {
                "contract": "hanonly-r52-challenge-cell-diagnostic-v1",
                "plan_revision": 52,
                "b0_sha": b0_sha,
                "phase": "challenge",
                "entry_id": entry_id,
                "device": device,
                "candidate_id": "S25L4",
                "state": "passed" if result == "pass" else "failed",
                "selection_result": "selected" if rejection is None else "rejected",
                "target_recall": target_recall,
                "pp_han_count": 1,
                "vl_han_count": 1,
                "rejection_reason": rejection,
                "raw_detector_outputs": [],
                "canonical_lines": [],
                "raw_detector_count": 0,
                "raw_detector_f32_bits_multiset_sha256": hashlib.sha256(
                    ledger._r51_canonical_json([])
                ).hexdigest(),
                "detector_support_records": [],
                "device_evidence_sha256": hashlib.sha256(process_bytes).hexdigest(),
                "device_evidence_byte_length": len(process_bytes),
                "log_sha256": hashlib.sha256(log_bytes).hexdigest(),
                "log_byte_length": len(log_bytes),
            }
        )
        diagnostic_path = f"challenge/{ordinal}/diagnostic.json"
        diagnostic_bytes = self.write_private(evidence / diagnostic_path, diagnostic)
        selection["execution_evidence"].update(
            {
                "source_gate_diagnostic_relpath": diagnostic_path,
                "source_gate_diagnostic_sha256": hashlib.sha256(
                    diagnostic_bytes
                ).hexdigest(),
                "raw_inference_log_relpath": log_path,
                "raw_inference_log_sha256": hashlib.sha256(log_bytes).hexdigest(),
            }
        )
        selection_bytes = self.write_private(evidence / selection_path, selection)
        return {
            "ordinal": ordinal,
            "entry_id": entry_id,
            "device": device,
            "kind": "regression" if ordinal < 8 else "supplemental",
            "candidate_id": "S25L4",
            "selection_result_path": selection_path,
            "selection_result_sha256": hashlib.sha256(selection_bytes).hexdigest(),
            "target_recall": target_recall,
            "pp_count": 1,
            "vl_count": 1,
            "rejection_reason": rejection,
            "diagnostic_path": diagnostic_path,
            "diagnostic_sha256": hashlib.sha256(diagnostic_bytes).hexdigest(),
            "process_evidence_path": process_path,
            "process_evidence_sha256": hashlib.sha256(process_bytes).hexdigest(),
            "log_path": log_path,
            "log_sha256": hashlib.sha256(log_bytes).hexdigest(),
            "result": result,
        }

    def test_projection_changes_only_the_four_entry_ids(self):
        outer, inventory = self.projection_fixture()
        inner, unchanged = ledger._r52_projection_values(outer, inventory)
        self.assertEqual(
            [entry["id"] for entry in inner["entries"]],
            ledger.R51_CALIBRATION_IDS,
        )
        for outer_entry, inner_entry in zip(outer["entries"], inner["entries"]):
            self.assertEqual(
                {key: value for key, value in outer_entry.items() if key != "id"},
                {key: value for key, value in inner_entry.items() if key != "id"},
            )
        self.assertRegex(unchanged, r"\A[0-9a-f]{64}\Z")
        self.assertNotIn(b"\n", ledger._r51_canonical_json(inner))

    def test_projection_rejects_missing_duplicate_reordered_and_non_calibration(self):
        mutations = (
            lambda entries: entries.pop(),
            lambda entries: entries.__setitem__(1, dict(entries[0])),
            lambda entries: entries.reverse(),
            lambda entries: entries[0].__setitem__("role", "holdout"),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                outer, inventory = self.projection_fixture()
                mutate(outer["entries"])
                with self.assertRaises(ledger.LedgerError):
                    ledger._r52_projection_values(outer, inventory)

    def test_preflight_schema_is_closed_and_rejects_identity_claims(self):
        value = {
            "contract": "hanonly-r52-b0-preflight-v1",
            "plan_revision": 52,
            "b0_sha": "b" * 40,
            "parent_b0_sha": ledger.R52_PARENT_B0_SHA,
            "r52_contract_sha256": ledger.R52_CONTRACT_SHA256,
            "r52_test_spec_sha256": ledger.R52_TEST_SPEC_SHA256,
            "r51_contract_sha256": ledger.R51_CONTRACT_SHA256,
            "r51_test_spec_sha256": ledger.R51_TEST_SPEC_SHA256,
            "r51_failure_summary_sha256": ledger.R52_R51_FAILURE_SHA256,
            "calibration_manifest_sha256": ledger.R52_CALIBRATION_MANIFEST_SHA256,
            "calibration_hash_inventory_sha256": ledger.R52_CALIBRATION_HASHES_SHA256,
            "checker_endpoint_sha256": "1" * 64,
            "evidence_ledger_endpoint_sha256": "2" * 64,
            "evidence_test_executable_path": "/closed/evidence-test",
            "evidence_test_executable_sha256": "3" * 64,
            "evidence_enabled_cargo_features": ledger.R51_FEATURES,
            "gate_results": {key: "pass" for key in ledger.R51_GATE_KEYS},
            "result": "pass",
        }
        ledger._r52_validate_preflight_value(value, "b" * 40)
        for claim in ("identity", "operator", "agent", "thread", "author"):
            mutated = json.loads(json.dumps(value))
            mutated[f"{claim}_claim"] = "forbidden"
            with self.subTest(claim=claim), self.assertRaises(ledger.LedgerError):
                ledger._r52_validate_preflight_value(mutated, "b" * 40)
        mutated = json.loads(json.dumps(value))
        mutated["gate_results"]["unknown"] = "pass"
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_preflight_value(mutated, "b" * 40)

    def test_create_new_lock_capability_is_process_local_and_collision_closes(self):
        state = self.root / "state"
        state.mkdir(mode=0o700)
        value = {
            "contract": "test-r52-lock",
            "created_at_utc": "2026-07-28T10:00:00Z",
        }
        observed = {}

        def action(root, lock, digest, revalidate, _stack):
            revalidate()
            observed.update(
                digest=digest,
                root_identity=ledger._identity(os.fstat(root.fd)),
                lock_identity=ledger._identity(os.fstat(lock.fd)),
            )
            return b"pass\n"

        self.assertEqual(
            ledger._r52_with_one_shot_lock(
                str(state),
                ledger.R52_CHALLENGE_LOCK_NAME,
                value,
                "challenge",
                action,
            ),
            b"pass\n",
        )
        self.assertRegex(observed["digest"], r"\A[0-9a-f]{64}\Z")
        replay = subprocess.run(
            [
                sys.executable,
                "-c",
                (
                    "from scripts import hanonly_evidence_ledger as l;"
                    f"l._r52_with_one_shot_lock({str(state)!r},"
                    f"{ledger.R52_CHALLENGE_LOCK_NAME!r},{value!r},'challenge',"
                    "lambda *_: b'replayed')"
                ),
            ],
            cwd=Path(__file__).parents[1],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(replay.returncode, 0)
        self.assertNotIn("replayed", replay.stdout)

    def test_create_new_lock_race_has_exactly_one_gated_winner(self):
        state = self.root / "race-state"
        state.mkdir(mode=0o700)
        marker = self.root / "gated-winners.log"
        script = f"""
import os
from scripts import hanonly_evidence_ledger as l
state = {str(state)!r}
marker = {str(marker)!r}
value = {{"contract": "race"}}
def action(*_):
    fd = os.open(marker, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    os.write(fd, b"winner\\n")
    os.close(fd)
    return b""
l._r52_with_one_shot_lock(
    state, l.R52_CHALLENGE_LOCK_NAME, value, "challenge", action
)
"""
        processes = [
            subprocess.Popen(
                [sys.executable, "-c", script],
                cwd=Path(__file__).parents[1],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            for _ in range(2)
        ]
        results = [process.communicate(timeout=10) for process in processes]
        self.assertEqual(sorted(process.returncode for process in processes), [0, 1])
        self.assertEqual(marker.read_text(), "winner\n")
        self.assertEqual(sum("LedgerError" in stderr for _, stderr in results), 1)

    def test_r52_publications_revalidate_inputs_at_the_actual_link(self):
        final_names = (
            "r52-b0-artifact-payload.json",
            "hanonly-r52-b0-authorization.json",
            "hanonly-r52-b0-artifact.json",
            "challenge-failure.json",
            "challenge-terminal.json",
        )
        for final_name in final_names:
            with self.subTest(final_name=final_name):
                root = self.root / final_name.removesuffix(".json")
                root.mkdir(mode=0o700)
                held_path = root / "held.json"
                self.write_private(held_path, b'{"value":1}')
                with contextlib.ExitStack() as stack:
                    parent = ledger._open_absolute(
                        str(root), directory=True, stack=stack
                    )
                    held = ledger._open_absolute(
                        str(held_path), directory=False, stack=stack
                    )
                    expected = hashlib.sha256(ledger._read_all(held.fd)).hexdigest()

                    def revalidate():
                        ledger._revalidate_held_path(held, "R52 test held input")
                        os.lseek(held.fd, 0, os.SEEK_SET)
                        self.assertEqual(
                            hashlib.sha256(ledger._read_all(held.fd)).hexdigest(),
                            expected,
                        )

                    def checkpoint(name):
                        if name == f"before_link:{final_name}":
                            with held_path.open("wb") as output:
                                output.write(b'{"value":2}')
                                output.flush()
                                os.fsync(output.fileno())

                    with (
                        mock.patch.object(
                            ledger, "_checkpoint", side_effect=checkpoint
                        ),
                        self.assertRaises((ledger.LedgerError, AssertionError)),
                    ):
                        ledger._publish_canonical_held(
                            parent,
                            str(root / final_name),
                            {"result": "pass"},
                            "R52 race test",
                            allowed_names={final_name},
                            temp_name=f".{final_name}.tmp",
                            existing_ok=False,
                            pre_link=revalidate,
                            stack=stack,
                        )
                self.assertFalse((root / final_name).exists())

    def test_challenge_final_failure_has_full_prefix_and_empty_suffix(self):
        evidence = self.root / "challenge-failure-evidence"
        evidence.mkdir(mode=0o700)
        cells = [
            self.write_challenge_cell_evidence(
                evidence, ordinal, result="fail" if ordinal == 17 else "pass"
            )
            for ordinal in range(18)
        ]
        failure = {
            "contract": "hanonly-r52-challenge-failure-v1",
            "plan_revision": 52,
            "b0_sha": "b" * 40,
            "challenge_lock_sha256": "1" * 64,
            "challenge_start_sha256": "2" * 64,
            "executed_prefix": cells,
            "first_failed_cell": cells[-1],
            "unexecuted_suffix": [],
            "failure_reason": "terminal cell failed",
            "failed_at_utc": "2026-07-28T10:00:02Z",
            "result": "fail",
        }
        self.assertEqual(
            len(
                ledger._r52_validate_challenge_failure(
                    failure,
                    "b" * 40,
                    "1" * 64,
                    "2" * 64,
                    "S25L4",
                    str(evidence),
                )
            ),
            18,
        )
        failure["unexecuted_suffix"] = ["extra"]
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_challenge_failure(
                failure,
                "b" * 40,
                "1" * 64,
                "2" * 64,
                "S25L4",
                str(evidence),
            )

    def test_challenge_cell_schema_rejects_order_alias_omission_and_extra(self):
        valid = self.challenge_cell(0)
        ledger._r52_validate_challenge_cell_identity(valid, 0, "S25L4")
        mutations = (
            lambda cell: cell.__setitem__("entry_id", "r49-h02"),
            lambda cell: cell.__setitem__("device", "gpu"),
            lambda cell: cell.__setitem__("kind", "supplemental"),
            lambda cell: cell.__setitem__("ordinal", 1),
            lambda cell: cell.__setitem__("selection_result_path", "./result.json"),
            lambda cell: cell.pop("pp_count"),
            lambda cell: cell.__setitem__("extra", True),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                cell = self.challenge_cell(0)
                mutate(cell)
                with self.assertRaises(ledger.LedgerError):
                    ledger._r52_validate_challenge_cell_identity(cell, 0, "S25L4")
        supplemental = self.challenge_cell(8)
        supplemental["target_recall"] = {
            "target_total": 1,
            "selected": 1,
            "covered": 1,
            "uncovered": 0,
        }
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_challenge_cell_identity(supplemental, 8, "S25L4")
        invalid_reason = self.challenge_cell(0)
        invalid_reason["rejection_reason"] = "caller_defined"
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_challenge_cell_identity(invalid_reason, 0, "S25L4")

    def test_challenge_cell_binds_selection_diagnostic_process_and_counts(self):
        evidence = self.root / "cell-binding"
        evidence.mkdir(mode=0o700)
        cell = self.write_challenge_cell_evidence(evidence, 0)
        ledger._r52_validate_challenge_cell(
            str(evidence), cell, 0, "S25L4", set(), "b" * 40
        )
        diagnostic_path = evidence / cell["diagnostic_path"]
        diagnostic = json.loads(diagnostic_path.read_text())
        for key, value in (
            ("entry_id", "r49-h02"),
            ("pp_han_count", 2),
            ("vl_han_count", 2),
            ("rejection_reason", "pp_no_words"),
        ):
            with self.subTest(key=key):
                mutated = dict(diagnostic)
                mutated[key] = value
                data = self.write_private(diagnostic_path, mutated)
                changed = dict(cell)
                changed["diagnostic_sha256"] = hashlib.sha256(data).hexdigest()
                with self.assertRaises(ledger.LedgerError):
                    ledger._r52_validate_challenge_cell(
                        str(evidence), changed, 0, "S25L4", set(), "b" * 40
                    )
        self.write_private(diagnostic_path, diagnostic)

    def test_r52_endpoint_surface_is_one_shot_and_never_names_r51_publisher(self):
        endpoint_names = {
            "write-r52-b0-preflight-attestation",
            "run-r52-challenge",
            "run-r52-holdout",
            "validate-r52-b0-authorization",
        }
        for name in endpoint_names:
            with (
                self.subTest(name=name),
                self.assertRaises(ledger.LedgerError) as error,
            ):
                ledger._parse_arguments([name])
            self.assertIn("required", str(error.exception))
        for forbidden in (
            "create-r52-challenge-lock",
            "create-r52-holdout-lock",
            "publish-r52-challenge-receipt",
        ):
            with self.assertRaises(ledger.LedgerError):
                ledger._parse_arguments([forbidden])
        holdout_names = set(ledger._r52_run_holdout.__code__.co_names)
        challenge_names = set(ledger._r52_run_challenge.__code__.co_names)
        self.assertNotIn("_r51_validate_authorization", holdout_names)
        self.assertNotIn("_r51_validate_authorization", challenge_names)

    def test_run_r52_challenge_endpoint_executes_full_18_cell_binding_once(self):
        repo = self.root / "repo"
        scripts = repo / "scripts"
        scripts.mkdir(parents=True)
        shutil.copy2(Path(ledger.__file__), scripts / "hanonly_evidence_ledger.py")
        shutil.copy2(
            Path(__file__).with_name("check-hanonly-production-policy.ts"),
            scripts / "check-hanonly-production-policy.ts",
        )

        def git(*args):
            return subprocess.run(
                ["git", *args],
                cwd=repo,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()

        git("init", "-q")
        git("config", "user.name", "R52 Test")
        git("config", "user.email", "r52@example.invalid")
        git("add", "scripts")
        git("commit", "-qm", "parent")
        parent_sha = git("rev-parse", "HEAD")
        (repo / "marker").write_text("r52\n")
        git("add", "marker")
        git("commit", "-qm", "b0")
        b0_sha = git("rev-parse", "HEAD")
        git("checkout", "--detach", "-q", b0_sha)

        evidence = self.root / "endpoint-evidence"
        state = self.root / "endpoint-state"
        evidence.mkdir(mode=0o700)
        state.mkdir(mode=0o700)
        cells = [
            self.write_challenge_cell_evidence(evidence, ordinal, b0_sha=b0_sha)
            for ordinal in range(18)
        ]
        evaluator_result = evidence / "pinned-result.json"
        evaluator_result_bytes = ledger._r51_canonical_json(
            {
                "contract": "hanonly-r52-pinned-evaluator-result-v1",
                "plan_revision": 52,
                "b0_sha": b0_sha,
                "selected_candidate_id": "S25L4",
                "ordered_cell_results": cells,
                "result": "pass",
            }
        )
        self.write_private(evaluator_result, evaluator_result_bytes)
        executable = self.root / "pinned-evaluator"
        executable.write_text("#!/bin/sh\nexit 1\n")
        executable.chmod(0o700)
        preflight = evidence / "r52-b0-preflight.json"
        self.write_private(
            preflight,
            {
                "contract": "hanonly-r52-b0-preflight-v1",
                "plan_revision": 52,
                "b0_sha": b0_sha,
                "parent_b0_sha": parent_sha,
                "r52_contract_sha256": ledger.R52_CONTRACT_SHA256,
                "r52_test_spec_sha256": ledger.R52_TEST_SPEC_SHA256,
                "r51_contract_sha256": ledger.R51_CONTRACT_SHA256,
                "r51_test_spec_sha256": ledger.R51_TEST_SPEC_SHA256,
                "r51_failure_summary_sha256": ledger.R52_R51_FAILURE_SHA256,
                "calibration_manifest_sha256": ledger.R52_CALIBRATION_MANIFEST_SHA256,
                "calibration_hash_inventory_sha256": ledger.R52_CALIBRATION_HASHES_SHA256,
                "checker_endpoint_sha256": hashlib.sha256(
                    (scripts / "check-hanonly-production-policy.ts").read_bytes()
                ).hexdigest(),
                "evidence_ledger_endpoint_sha256": hashlib.sha256(
                    (scripts / "hanonly_evidence_ledger.py").read_bytes()
                ).hexdigest(),
                "evidence_test_executable_path": str(executable),
                "evidence_test_executable_sha256": hashlib.sha256(
                    executable.read_bytes()
                ).hexdigest(),
                "evidence_enabled_cargo_features": ledger.R51_FEATURES,
                "gate_results": {key: "pass" for key in ledger.R51_GATE_KEYS},
                "result": "pass",
            },
        )
        recall = evidence / "frozen-recall.json"
        self.write_private(recall, {"selected_candidate_id": "S25L4"})
        calibration = evidence / "calibration-selection.json"
        self.write_private(
            calibration,
            {
                "b0_sha": b0_sha,
                "selected_candidate_id": "S25L4",
            },
        )
        invocations = []

        def run_pinned(observed_executable, mode, environment):
            self.assertEqual(observed_executable, str(executable))
            self.assertEqual(mode, "challenge")
            request = json.loads(
                Path(environment["HANONLY_R52_BRIDGE_REQUEST"]).read_text()
            )
            self.assertEqual(
                request["contract"], "hanonly-r52-evidence-bridge-request-v1"
            )
            target = Path(request["result_path"])
            target.write_bytes(evaluator_result_bytes)
            target.chmod(0o600)
            invocations.append(mode)

        argv = [
            "run-r52-challenge",
            "--repo-root",
            str(repo),
            "--b0-sha",
            b0_sha,
            "--evidence-root",
            str(evidence),
            "--challenge-state-root",
            str(state),
            "--b0-preflight-attestation",
            str(preflight),
            "--challenge-manifest",
            ledger.R52_CHALLENGE_MANIFEST_PATH,
            "--challenge-hash-record",
            ledger.R52_CHALLENGE_HASHES_PATH,
            "--calibration-ledger",
            str(calibration),
            "--frozen-recall-contract",
            str(recall),
            "--source-gate-fixture-manifest-sha256",
            "f" * 64,
            "--created-at-utc",
            "2026-07-28T10:00:00Z",
            "--started-at-utc",
            "2026-07-28T10:00:01Z",
            "--completed-at-utc",
            "2026-07-28T10:00:02Z",
        ]
        original_cwd = os.getcwd()
        try:
            os.chdir(repo)
            with (
                mock.patch.object(ledger, "R52_PARENT_B0_SHA", parent_sha),
                mock.patch.object(ledger, "R52_STATE_ROOT", str(state)),
                mock.patch.object(
                    ledger, "_r52_run_pinned_evaluator", side_effect=run_pinned
                ),
            ):
                output = ledger.execute(argv)
                self.assertIn(b'"challenge_terminal_sha256"', output)
                with self.assertRaises(ledger.LedgerError):
                    ledger.execute(argv)
        finally:
            os.chdir(original_cwd)
        self.assertEqual(invocations, ["challenge"])
        self.assertTrue((state / "challenge-terminal.json").is_file())
        self.assertFalse((state / "challenge-failure.json").exists())
        self.assertFalse((evidence / "r51-b0-authorization.json").exists())
        self.assertFalse((evidence / "hanonly-r51-b0-artifact.json").exists())

        failed_state = self.root / "endpoint-failed-state"
        failed_state.mkdir(mode=0o700)
        failed_argv = list(argv)
        state_index = failed_argv.index("--challenge-state-root") + 1
        failed_argv[state_index] = str(failed_state)
        try:
            os.chdir(repo)
            with (
                mock.patch.object(ledger, "R52_PARENT_B0_SHA", parent_sha),
                mock.patch.object(ledger, "R52_STATE_ROOT", str(failed_state)),
                mock.patch.object(
                    ledger,
                    "_r52_run_pinned_evaluator",
                    side_effect=subprocess.TimeoutExpired("bridge", 1),
                ),
                self.assertRaises(ledger.LedgerError) as failure_error,
            ):
                ledger.execute(failed_argv)
        finally:
            os.chdir(original_cwd)
        failure_path = failed_state / "challenge-failure.json"
        self.assertTrue(failure_path.exists(), str(failure_error.exception))
        failure = json.loads(failure_path.read_text())
        self.assertEqual(failure["executed_prefix"], [])
        self.assertIsNone(failure["first_failed_cell"])
        self.assertEqual(failure["unexecuted_suffix"], ledger.R52_CHALLENGE_CELL_IDS)
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_challenge_receipts(
                str(failed_state), str(evidence), b0_sha, "S25L4", "0" * 64
            )

    def test_challenge_receipts_require_closed_contracts_and_terminal_state(self):
        state = self.root / "state"
        evidence = self.root / "challenge-evidence"
        state.mkdir(mode=0o700)
        evidence.mkdir(mode=0o700)
        lock = {
            "contract": "hanonly-r52-challenge-use-lock-v1",
            "plan_revision": 52,
            "b0_sha": "b" * 40,
            "challenge_manifest_sha256": ledger.R52_CHALLENGE_MANIFEST_SHA256,
            "challenge_hash_record_sha256": ledger.R52_CHALLENGE_HASHES_SHA256,
            "selected_candidate_id": "S25L4",
            "frozen_recall_contract_sha256": "1" * 64,
            "created_at_utc": "2026-07-28T10:00:00Z",
        }
        lock_bytes = self.write_private(state / ledger.R52_CHALLENGE_LOCK_NAME, lock)
        start = {
            "contract": "hanonly-r52-challenge-start-v1",
            "plan_revision": 52,
            "b0_sha": "b" * 40,
            "challenge_lock_sha256": hashlib.sha256(lock_bytes).hexdigest(),
            "selected_candidate_id": "S25L4",
            "ordered_cell_ids": ledger.R52_CHALLENGE_CELL_IDS,
            "started_at_utc": "2026-07-28T10:00:01Z",
        }
        start_bytes = self.write_private(state / "challenge-start.json", start)
        terminal = {
            "contract": "hanonly-r52-challenge-terminal-v1",
            "plan_revision": 52,
            "b0_sha": "b" * 40,
            "challenge_lock_sha256": hashlib.sha256(lock_bytes).hexdigest(),
            "challenge_start_sha256": hashlib.sha256(start_bytes).hexdigest(),
            "selected_candidate_id": "S25L4",
            "ordered_cell_results": [
                self.write_challenge_cell_evidence(evidence, ordinal)
                for ordinal in range(18)
            ],
            "completed_at_utc": "2026-07-28T10:00:02Z",
            "result": "pass",
        }
        terminal_path = state / "challenge-terminal.json"
        self.write_private(terminal_path, terminal)
        ledger._r52_validate_challenge_receipts(
            str(state), str(evidence), "b" * 40, "S25L4", "1" * 64
        )
        terminal["contract"] = "hanonly-r51-encrypted-holdout-terminal-v1"
        self.write_private(terminal_path, terminal)
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_challenge_receipts(
                str(state), str(evidence), "b" * 40, "S25L4", "1" * 64
            )
        terminal_path.unlink()
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_challenge_receipts(
                str(state), str(evidence), "b" * 40, "S25L4", "1" * 64
            )

    def test_run_r52_holdout_keeps_lock_while_custody_opens_runs_and_cleans(self):
        repo = self.root / "holdout-repo"
        checker = repo / ledger.B0_CHECKER_ENDPOINT
        checker.parent.mkdir(parents=True)
        checker.write_text("checker\n")
        evidence = self.root / "holdout-evidence"
        evidence.mkdir(mode=0o700)
        state = self.root / "holdout-state"
        state.mkdir(mode=0o700)
        custody = self.root / "holdout-custody"
        custody.mkdir(mode=0o700)
        for name in (
            ledger.R52_CHALLENGE_LOCK_NAME,
            "challenge-start.json",
            "challenge-terminal.json",
        ):
            self.write_private(state / name, b"{}")

        def private(name, value=b"{}"):
            path = evidence / name
            self.write_private(path, value)
            return str(path)

        executable = evidence / "evidence-test"
        executable.write_text("#!/bin/sh\nexit 1\n")
        executable.chmod(0o700)
        calibration = private(
            "calibration.json",
            ledger._r51_canonical_json(
                {
                    "b0_sha": "b" * 40,
                    "manifest_sha256": "c" * 64,
                    "selected_candidate_id": "S25L4",
                }
            ),
        )
        recall = private(
            "recall.json",
            ledger._r51_canonical_json({"selected_candidate_id": "S25L4"}),
        )
        ciphertext = custody / "holdout.enc"
        freeze = custody / "holdout-freeze-receipt.json"
        historical = custody / "historical-inventory.json"
        for path in (ciphertext, freeze, historical):
            self.write_private(path, b"frozen")
        open_marker = custody / "holdout-open.json"
        plaintext = evidence / "r51-plaintext"
        archive = plaintext / "holdout.tar"
        ciphertext_sha256 = hashlib.sha256(b"frozen").hexdigest()
        holdout_lock_name = f"holdout-use-{ciphertext_sha256}.lock"
        arguments = SimpleNamespace(
            repo_root=str(repo),
            b0_sha="b" * 40,
            evidence_root=str(evidence),
            b0_preflight_attestation=private("preflight.json"),
            pre_holdout_attestation=private("pre-holdout.json"),
            calibration_ledger=calibration,
            frozen_recall_contract=recall,
            holdout_adoption=private("adoption.json"),
            freeze_receipt=str(freeze),
            historical_inventory=str(historical),
            ciphertext=str(ciphertext),
            challenge_state_root=str(state),
            holdout_use_lock=str(state / holdout_lock_name),
            open_marker=str(open_marker),
            plaintext_directory=str(plaintext),
            plaintext_archive=str(archive),
            source_gate_fixture_manifest_sha256="d" * 64,
            created_at_utc="2026-07-28T10:00:03Z",
        )
        observed = {}

        def publish_runtime(_seconds):
            if plaintext.exists():
                return
            plaintext.mkdir(mode=0o700)
            self.write_private(archive, b"archive")
            self.write_private(open_marker, b'{"result":"opened"}')

        def run_pinned(observed_executable, mode, environment):
            self.assertEqual(observed_executable, str(executable))
            self.assertEqual(mode, "holdout")
            self.assertEqual(
                environment["HANONLY_SOURCE_GATE_SELECTION_PHASE"], "holdout"
            )
            self.assertEqual(environment["HANONLY_R51_PLAINTEXT_ARCHIVE"], str(archive))
            observed.update(environment)
            archive.unlink()
            plaintext.rmdir()

        with (
            mock.patch.object(ledger, "R52_STATE_ROOT", str(state)),
            mock.patch.object(ledger, "R52_CUSTODY_ROOT", str(custody)),
            mock.patch.object(ledger, "R52_CIPHERTEXT_SHA256", ciphertext_sha256),
            mock.patch.object(ledger, "R52_HOLDOUT_LOCK_NAME", holdout_lock_name),
            mock.patch.object(ledger, "_validate_repository"),
            mock.patch.object(ledger, "_r52_validate_b0_lineage"),
            mock.patch.object(
                ledger,
                "_r52_runner_from_preflight",
                return_value=str(executable),
            ),
            mock.patch.object(
                ledger,
                "_r52_validate_challenge_receipts",
                return_value=(b"challenge", {}),
            ),
            mock.patch.object(
                ledger,
                "_r52_validate_adoption",
                return_value=(b"adoption", {"key_capability": "retained"}),
            ),
            mock.patch.object(
                ledger,
                "_r51_validate_attestation",
                return_value=(str(evidence / "pre-holdout.json"), "e" * 64, {}),
            ),
            mock.patch.object(ledger, "_r51_preflight_custody_snapshot"),
            mock.patch.object(
                ledger, "_r52_run_pinned_evaluator", side_effect=run_pinned
            ),
            mock.patch.object(
                ledger,
                "_r52_validate_authorization_inputs",
                return_value={"imported_r51_terminal_sha256": "f" * 64},
            ),
            mock.patch.object(ledger.time, "sleep", side_effect=publish_runtime),
        ):
            output = ledger._r52_run_holdout(arguments)
        self.assertIn(b'"holdout_use_lock_sha256"', output)
        self.assertEqual(
            observed["HANONLY_R51_OPEN_MARKER_SHA256"],
            hashlib.sha256(b'{"result":"opened"}').hexdigest(),
        )
        self.assertFalse(plaintext.exists())

    def test_protected_latin_correction_is_closed(self):
        valid = {
            "contract": "hanonly-r51-disclosed-challenge-manifest-v1",
            "entries": [
                {
                    "id": entry_id,
                    "prior_role": "r49_disclosed_holdout",
                    "source_path": f"/closed/{entry_id}.jpg",
                    "source_sha256": f"{ordinal + 1:064x}",
                    **(
                        {
                            "notes_path": f"/closed/{entry_id}.md",
                            "notes_sha256": f"{ordinal + 10:064x}",
                        }
                        if ordinal >= len(ledger.R52_CHALLENGE_IDS)
                        else {}
                    ),
                }
                for ordinal, entry_id in enumerate(
                    ledger.R52_CHALLENGE_IDS + ledger.R52_SUPPLEMENTAL_IDS
                )
            ],
            "oracle_corrections": [
                {
                    "entry_id": "r49-h04",
                    "target_id": "product-id",
                    "source_script_class": "protected_latin",
                    "expected_decision": "reject",
                    "expected_rejection_reason": "pp_no_han_protected_latin",
                    "r49_corpus_immutable": True,
                }
            ],
            "plan_revision": 51,
            "role": "challenge",
        }
        ledger._r52_validate_challenge_manifest(valid)
        recursive_alias = json.loads(json.dumps(valid))
        recursive_alias["oracle_corrections"] = []
        recursive_alias["entries"][0]["nested"] = valid["oracle_corrections"][0]
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_challenge_manifest(recursive_alias)

    def test_imported_inner_index_is_exactly_five_ordered_records(self):
        evidence = self.root / "evidence"
        custody = self.root / "custody"
        evidence.mkdir(mode=0o700)
        custody.mkdir(mode=0o700)
        expected_paths = {}
        records = []
        for ordinal, kind in enumerate(ledger.R52_INNER_KINDS):
            root = custody if kind == "r51_terminal_receipt" else evidence
            path = root / f"{kind}.json"
            value = {
                "contract": f"inner-{ordinal}",
                "plan_revision": 51,
                "result": "pass",
            }
            data = self.write_private(path, value)
            expected_paths[kind] = str(path)
            records.append(
                {
                    "kind": kind,
                    "relative_path": path.name,
                    "byte_length": len(data),
                    "sha256": hashlib.sha256(data).hexdigest(),
                    "inner_contract": value["contract"],
                    "inner_plan_revision": 51,
                }
            )
        index = {
            "contract": "hanonly-r52-imported-r51-inner-evidence-index-v1",
            "plan_revision": 52,
            "b0_sha": "b" * 40,
            "records": records,
            "result": "pass",
        }
        index_path = evidence / "r52-imported-r51-inner-evidence-index.json"
        self.write_private(index_path, index)
        ledger._r52_validate_inner_index(
            str(index_path),
            str(evidence),
            str(custody),
            "b" * 40,
            expected_paths,
        )
        index["records"].reverse()
        self.write_private(index_path, index)
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_inner_index(
                str(index_path),
                str(evidence),
                str(custody),
                "b" * 40,
                expected_paths,
            )

    def test_authorization_held_descriptors_reject_input_mutation_and_output_swap(self):
        evidence = self.root / "evidence"
        authorization = evidence / "authorization"
        evidence.mkdir(mode=0o700)
        authorization.mkdir(mode=0o700)
        state = self.root / "state"
        custody = self.root / "custody"
        state.mkdir(mode=0o700)
        custody.mkdir(mode=0o700)
        source = evidence / "input.json"
        self.write_private(source, {"contract": "input-v1"})
        arguments = mock.Mock(
            evidence_root=str(evidence),
            challenge_state_root=str(state),
            ciphertext=str(custody / "holdout.enc"),
            artifact_payload_out=str(authorization / "r52-b0-artifact-payload.json"),
            authorization_record_out=str(
                authorization / "hanonly-r52-b0-authorization.json"
            ),
            artifact_out=str(authorization / "hanonly-r52-b0-artifact.json"),
            input_path=str(source),
        )
        self.write_private(custody / "holdout.enc", b"ciphertext")
        with contextlib.ExitStack() as stack:
            held_dir = ledger._open_absolute(
                str(authorization), directory=True, stack=stack
            )
            held, digests = ledger._r52_hold_authorization_inputs(
                arguments, stack, str(authorization)
            )
            ledger._r52_revalidate_authorization_inputs(held, digests)
            source.write_bytes(b"mutated")
            with self.assertRaises(ledger.LedgerError):
                ledger._r52_revalidate_authorization_inputs(held, digests)
            source.unlink()
            self.write_private(source, {"contract": "input-v1"})
            swapped = evidence / "authorization-swapped"
            authorization.rename(swapped)
            authorization.mkdir(mode=0o700)
            with self.assertRaises(ledger.LedgerError):
                ledger._revalidate_held_path(
                    held_dir, "R52 held authorization output directory"
                )

    def test_authorization_rejects_temp_and_artifact_without_authorization(self):
        evidence = self.root / "evidence"
        authorization = evidence / "authorization"
        evidence.mkdir(mode=0o700)
        authorization.mkdir(mode=0o700)
        arguments = mock.Mock(
            evidence_root=str(evidence),
            artifact_payload_out=str(authorization / "r52-b0-artifact-payload.json"),
            authorization_record_out=str(
                authorization / "hanonly-r52-b0-authorization.json"
            ),
            artifact_out=str(authorization / "hanonly-r52-b0-artifact.json"),
        )
        self.write_private(
            authorization / ".r52-b0-artifact-payload.json.tmp", b"partial"
        )
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_authorization(arguments)
        (authorization / ".r52-b0-artifact-payload.json.tmp").unlink()
        self.write_private(authorization / "hanonly-r52-b0-artifact.json", b"{}")
        with self.assertRaises(ledger.LedgerError):
            ledger._r52_validate_authorization(arguments)


if __name__ == "__main__":
    unittest.main()
