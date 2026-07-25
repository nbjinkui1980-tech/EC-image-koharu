#!/usr/bin/env python3

import argparse
import contextlib
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
    return parser.parse_args(argv)


def execute(argv):
    arguments = _parse_arguments(argv)
    if arguments.command == "create":
        return _create(arguments)
    return _rehydrate(arguments)


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
