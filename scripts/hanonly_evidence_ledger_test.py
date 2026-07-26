import contextlib
import hashlib
import io
import json
import os
import stat
import struct
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

try:
    from scripts import hanonly_evidence_ledger as ledger
except ModuleNotFoundError:
    import hanonly_evidence_ledger as ledger


WIDTH = 790
HEIGHT = 1023
HEX64 = "a" * 64


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


def b0_artifact():
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
                "n_gpu_layers": 32 if metal else 0,
                "mtmd_use_gpu": metal,
                "word_boxes_backend": "rten_cpu",
                "raw_load_log_relpath": f"source-gate/{phase}/{device}/load.log",
                "raw_load_log_sha256": HEX64,
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
                "raw_inference_log_sha256": HEX64,
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
                "passed": True,
            },
        }

    calibration_ids = [f"calibration-{index}" for index in range(4)]
    holdout_ids = [f"holdout-{index}" for index in range(4)]
    value = {
        "version": ledger.B0_VERSION,
        "plan_revision": 46,
        "b0_sha": "b" * 40,
        "manifest_sha256": "c" * 64,
        "source_gate_fixture_manifest_sha256": "d" * 64,
        "image_input_contract_sha256": "e" * 64,
        "source_color_contract_sha256": "f" * 64,
        "color_constant_set_sha256": "1" * 64,
        "requested_devices": ["cpu", "metal"],
        "enabled_cargo_features": ["hanonly-test-evidence", "metal"],
        "backend_evidence_parser_version": 1,
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
        "selected_candidate_id": "R025",
        "frozen_at_utc": "2026-07-26T00:00:00Z",
        "frozen_payload_sha256": "2" * 64,
        "holdout_results": [
            result("holdout", entry_id, device, "R025")
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
        subprocess.run(["git", "config", "user.name", "Ledger Test"], cwd=self.repo, check=True)
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
                self.assertEqual(ledger.image_dimensions(jpeg(sof=marker)), (WIDTH, HEIGHT))

    def test_accepts_legal_jpeg_metadata(self):
        self.assertEqual(ledger.image_dimensions(jpeg(extra=b"metadata")), (WIDTH, HEIGHT))

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
            "animated-vp8x": riff(
                (b"VP8X", b"\x02\x00\x00\x00" + b"\x00" * 6)
            ),
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
            "visual_input_sha256": hashlib.sha256(self.case.input.read_bytes()).hexdigest(),
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
                        name = (
                            f".evidence-ledger.{hashlib.sha256(expected).hexdigest()}.tmp"
                        )
                        fifo = case.evidence_root / name
                    os.mkfifo(fifo, 0o600)
                    before = [(entry.name, entry.lstat().st_mode) for entry in case.evidence_root.iterdir()]
                    result = case.subprocess_cli(case.create_args(), timeout=1)
                    after = [(entry.name, entry.lstat().st_mode) for entry in case.evidence_root.iterdir()]
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
                        held = case.evidence_root.with_name(case.evidence_root.name + ".held")
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
        self.path = self.root / "artifact.json"
        self.value = b0_artifact()

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
        for revision in (29, 45, "46", None):
            with self.subTest(revision=revision):
                self.value = b0_artifact()
                self.value["plan_revision"] = revision
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

    def test_rejects_frozen_projection_hash_drift(self):
        self.value["selected_candidate_id"] = "R05"
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
                argument = "visual_manifest_sha256" if field == "manifest_sha256" else field
                self.assert_rejected(**{argument: "e" * len(self.value[field])})

    def test_rejects_candidate_ratio_drift(self):
        self.value["candidates"][1]["denominator"] = 4
        self.assert_rejected()


if __name__ == "__main__":
    unittest.main()
