#!/usr/bin/env python3
"""Exact canonical USTAR layout writer and validator for Han-only R60."""

from __future__ import annotations

import hashlib
import json
import os
import stat
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, BinaryIO


BLOCK = 512
EXPECTED_ENTRY_IDS = ("r60-h01", "r60-h02", "r60-h03", "r60-h04")
ROOT_FIELDS = frozenset(("contract", "entries", "plan_revision", "role"))
ENTRY_FIELDS = frozenset(
    (
        "aspect",
        "background",
        "clean_reference_relpath",
        "dimension_bin",
        "id",
        "multi_node",
        "protected_rois",
        "role",
        "source_relpath",
        "targets",
    )
)
TARGET_FIELDS = frozenset(
    (
        "clean_reference_edit_roi",
        "effect",
        "erase_source_ink_mask_relpath",
        "expected",
        "id",
        "position",
        "residual_source_ink_mask_relpath",
        "source_roi",
        "translation_length",
        "writing",
    )
)
REQUIRED_ROOT_NAMES = frozenset(("assets/", "hashes.json", "manifest.json", "oracle.json"))


@dataclass(frozen=True)
class DescriptorMetadata:
    device: int
    inode: int
    uid: int
    gid: int
    mode: int
    size: int
    mtime_ns: int


@dataclass(frozen=True)
class ValidationResult:
    archive_sha256: str
    archive_size: int
    manifest_sha256: str
    private_manifest_commitment_sha256: str
    member_name_digest_sha256: str
    entry_ids: tuple[str, ...]
    member_names: tuple[str, ...]
    descriptor_metadata: DescriptorMetadata


@dataclass(frozen=True)
class EncryptionResult:
    validation: ValidationResult
    streamed_archive_sha256: str
    streamed_archive_size: int
    post_stream_archive_sha256: str
    descriptor_metadata_before: DescriptorMetadata
    descriptor_metadata_after: DescriptorMetadata
    ciphertext_sha256: str
    ciphertext_size: int


def _metadata(fd: int) -> DescriptorMetadata:
    value = os.fstat(fd)
    if not stat.S_ISREG(value.st_mode):
        raise ValueError("archive descriptor is not a regular file")
    return DescriptorMetadata(
        value.st_dev,
        value.st_ino,
        value.st_uid,
        value.st_gid,
        stat.S_IMODE(value.st_mode),
        value.st_size,
        value.st_mtime_ns,
    )


def _read_exact(fd: int, size: int, offset: int) -> bytes:
    chunks: list[bytes] = []
    remaining = size
    while remaining:
        chunk = os.pread(fd, remaining, offset)
        if not chunk:
            raise ValueError("archive is truncated")
        chunks.append(chunk)
        offset += len(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def _hash_fd(fd: int, size: int) -> str:
    digest = hashlib.sha256()
    offset = 0
    while offset < size:
        chunk = os.pread(fd, min(1024 * 1024, size - offset), offset)
        if not chunk:
            raise ValueError("archive is truncated while hashing")
        digest.update(chunk)
        offset += len(chunk)
    if os.pread(fd, 1, size):
        raise ValueError("archive grew while hashing")
    return digest.hexdigest()


def _safe_name(name: str, *, directory: bool) -> bytes:
    if not isinstance(name, str) or not name:
        raise ValueError("member name must be a non-empty string")
    if name.startswith("/") or "\\" in name or "\0" in name:
        raise ValueError("unsafe member name")
    if directory != name.endswith("/"):
        raise ValueError("member type/name suffix mismatch")
    body = name[:-1] if directory else name
    if not body or any(part in ("", ".", "..") for part in body.split("/")):
        raise ValueError("non-canonical member name")
    try:
        encoded = name.encode("utf-8", "strict")
    except UnicodeError as error:
        raise ValueError("member name is not strict UTF-8") from error
    if len(encoded) > 100:
        raise ValueError("member name exceeds USTAR name field")
    return encoded


def _octal(field: bytes, digits: int, label: str) -> int:
    if len(field) != digits + 1 or field[-1:] != b"\0" or any(c not in b"01234567" for c in field[:-1]):
        raise ValueError(f"non-canonical {label}")
    return int(field[:-1], 8)


def _canonical_json(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def _json_no_duplicates(data: bytes) -> Any:
    def pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in items:
            if key in result:
                raise ValueError(f"duplicate JSON field: {key}")
            result[key] = value
        return result

    def constant(value: str) -> Any:
        raise ValueError(f"invalid JSON constant: {value}")

    try:
        return json.loads(data.decode("utf-8", "strict"), object_pairs_hook=pairs, parse_constant=constant)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("manifest.json is not strict UTF-8 JSON") from error


def _exact_fields(value: Any, fields: frozenset[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or frozenset(value) != fields:
        raise ValueError(f"manifest {label} fields are not exact")
    return value


def _manifest_bindings(data: bytes) -> tuple[tuple[str, ...], frozenset[str]]:
    manifest = _json_no_duplicates(data)
    if _canonical_json(manifest) != data:
        raise ValueError("manifest.json is not compact canonical sorted JSON")
    root = _exact_fields(manifest, ROOT_FIELDS, "root")
    if (
        root["contract"] != "hanonly-r60-holdout-manifest-v1"
        or root["role"] != "holdout"
        or type(root["plan_revision"]) is not int
        or root["plan_revision"] != 60
        or not isinstance(root["entries"], list)
    ):
        raise ValueError("manifest root binding drift")

    ids: list[str] = []
    assets: list[str] = []
    for raw_entry in root["entries"]:
        entry = _exact_fields(raw_entry, ENTRY_FIELDS, "entry")
        if not isinstance(entry["id"], str) or not isinstance(entry["targets"], list):
            raise ValueError("manifest entry binding drift")
        ids.append(entry["id"])
        for field in ("source_relpath", "clean_reference_relpath"):
            assets.append(entry[field])
        for raw_target in entry["targets"]:
            target = _exact_fields(raw_target, TARGET_FIELDS, "target")
            assets.extend(
                (target["erase_source_ink_mask_relpath"], target["residual_source_ink_mask_relpath"])
            )
    if tuple(ids) != EXPECTED_ENTRY_IDS:
        raise ValueError("manifest entry IDs are not the exact R60 IDs in order")

    expected = set(REQUIRED_ROOT_NAMES)
    for name in assets:
        if not isinstance(name, str) or name.endswith("/"):
            raise ValueError("manifest asset reference is not a file name")
        _safe_name(name, directory=False)
        expected.add(name)
        parts = name.split("/")[:-1]
        for end in range(1, len(parts) + 1):
            expected.add("/".join(parts[:end]) + "/")
    return tuple(ids), frozenset(expected)


def validate_archive_fd(fd: int) -> ValidationResult:
    """Validate the exact raw USTAR bytes held by *fd*. The fd remains open."""
    before = _metadata(fd)
    size = before.size
    if size < 2 * BLOCK or size % BLOCK:
        raise ValueError("archive size is not an exact USTAR block sequence")

    offset = 0
    previous: bytes | None = None
    names: list[str] = []
    types: dict[str, bool] = {}
    manifest_bytes: bytes | None = None
    while True:
        header = _read_exact(fd, BLOCK, offset)
        if header == bytes(BLOCK):
            if _read_exact(fd, BLOCK, offset + BLOCK) != bytes(BLOCK) or offset + 2 * BLOCK != size:
                raise ValueError("archive must end with exactly two zero blocks and EOF")
            break

        typeflag = header[156]
        if typeflag not in (ord("0"), ord("5")):
            raise ValueError("archive member type is not regular file or directory")
        directory = typeflag == ord("5")
        name_field = header[:100]
        nul = name_field.find(b"\0")
        if nul == 0 or (nul > 0 and any(name_field[nul:])):
            raise ValueError("member name field lacks exact NUL padding")
        raw_name = name_field if nul == -1 else name_field[:nul]
        try:
            name = raw_name.decode("utf-8", "strict")
        except UnicodeError as error:
            raise ValueError("member name is not strict UTF-8") from error
        if _safe_name(name, directory=directory) != raw_name:
            raise ValueError("member name encoding drift")
        if previous is not None and previous >= raw_name:
            raise ValueError("member names are not unique strict UTF-8 bytewise order")
        previous = raw_name

        expected_mode = b"0000700\0" if directory else b"0000600\0"
        if header[100:108] != expected_mode:
            raise ValueError("member mode drift")
        if header[108:116] != b"0000000\0" or header[116:124] != b"0000000\0":
            raise ValueError("member uid/gid drift")
        member_size = _octal(header[124:136], 11, "size")
        if directory and member_size:
            raise ValueError("directory has a payload")
        if header[136:148] != b"00000000000\0":
            raise ValueError("member mtime drift")
        expected_checksum = _octal(header[148:155], 6, "checksum")
        if header[155:156] != b" " or expected_checksum != sum(header[:148]) + 8 * ord(" ") + sum(header[156:]):
            raise ValueError("member checksum drift")
        if header[157:257] != bytes(100) or header[257:263] != b"ustar\0" or header[263:265] != b"00":
            raise ValueError("member USTAR metadata drift")
        if any(header[265:512]):
            raise ValueError("member optional USTAR fields or tail are not NUL")

        data_offset = offset + BLOCK
        padded_size = (member_size + BLOCK - 1) // BLOCK * BLOCK
        if data_offset + padded_size > size - 2 * BLOCK:
            raise ValueError("member payload exceeds archive")
        if member_size < padded_size and any(_read_exact(fd, padded_size - member_size, data_offset + member_size)):
            raise ValueError("member padding is not NUL")
        if name == "manifest.json":
            manifest_bytes = _read_exact(fd, member_size, data_offset)
        names.append(name)
        types[name] = directory
        offset = data_offset + padded_size

    if manifest_bytes is None:
        raise ValueError("manifest.json is absent")
    for name, directory in types.items():
        body = name[:-1] if directory else name
        for other in types:
            if not directory and other.startswith(body + "/"):
                raise ValueError("file/directory prefix collision")

    entry_ids, expected_names = _manifest_bindings(manifest_bytes)
    if frozenset(names) != expected_names:
        raise ValueError("archive member set does not equal manifest-derived set")
    manifest_sha = hashlib.sha256(manifest_bytes).hexdigest()
    names_digest = hashlib.sha256(_canonical_json(sorted(names))).hexdigest()
    archive_sha = _hash_fd(fd, size)
    after = _metadata(fd)
    if after != before:
        raise ValueError("archive descriptor metadata changed during validation")
    return ValidationResult(
        archive_sha,
        size,
        manifest_sha,
        manifest_sha,
        names_digest,
        entry_ids,
        tuple(names),
        before,
    )


def _header(name: str, directory: bool, size: int) -> bytes:
    encoded = _safe_name(name, directory=directory)
    if size < 0 or size > 0o77777777777:
        raise ValueError("file is too large for canonical USTAR size field")
    header = bytearray(BLOCK)
    header[: len(encoded)] = encoded
    header[100:108] = b"0000700\0" if directory else b"0000600\0"
    header[108:116] = b"0000000\0"
    header[116:124] = b"0000000\0"
    header[124:136] = f"{size:011o}\0".encode("ascii")
    header[136:148] = b"00000000000\0"
    header[148:156] = b"        "
    header[156] = ord("5") if directory else ord("0")
    header[257:263] = b"ustar\0"
    header[263:265] = b"00"
    header[148:156] = f"{sum(header):06o}\0 ".encode("ascii")
    return bytes(header)


def _bundle_entries(root: Path) -> list[tuple[str, bool, Path]]:
    if root.is_symlink() or not root.is_dir():
        raise ValueError("bundle_root must be a non-symlink directory")
    found: list[tuple[str, bool, Path]] = []

    def visit(directory: Path, prefix: str) -> None:
        with os.scandir(directory) as iterator:
            entries = list(iterator)
        for entry in entries:
            relative = f"{prefix}{entry.name}"
            if entry.is_symlink():
                raise ValueError("bundle contains a symlink")
            if entry.is_dir(follow_symlinks=False):
                name = relative + "/"
                _safe_name(name, directory=True)
                found.append((name, True, Path(entry.path)))
                visit(Path(entry.path), name)
            elif entry.is_file(follow_symlinks=False):
                _safe_name(relative, directory=False)
                found.append((relative, False, Path(entry.path)))
            else:
                raise ValueError("bundle contains a non-regular entry")

    visit(root, "")
    found.sort(key=lambda item: item[0].encode("utf-8"))
    return found


def write_canonical_archive(bundle_root: os.PathLike[str] | str, archive_path: os.PathLike[str] | str) -> ValidationResult:
    """Archive the contents of bundle_root in exact deterministic R60 USTAR form."""
    entries = _bundle_entries(Path(bundle_root))
    target = os.fspath(archive_path)
    flags = os.O_RDWR | os.O_CREAT | os.O_EXCL | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(target, flags, 0o600)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(os.dup(fd), "wb", closefd=True) as output:
            for name, directory, path in entries:
                if directory:
                    output.write(_header(name, True, 0))
                    continue
                open_flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
                source_fd = os.open(path, open_flags)
                try:
                    source_stat = os.fstat(source_fd)
                    if not stat.S_ISREG(source_stat.st_mode):
                        raise ValueError("bundle file changed type while archiving")
                    output.write(_header(name, False, source_stat.st_size))
                    remaining = source_stat.st_size
                    while remaining:
                        chunk = os.read(source_fd, min(1024 * 1024, remaining))
                        if not chunk:
                            raise ValueError("bundle file shrank while archiving")
                        output.write(chunk)
                        remaining -= len(chunk)
                    if os.read(source_fd, 1):
                        raise ValueError("bundle file grew while archiving")
                    output.write(bytes((-source_stat.st_size) % BLOCK))
                finally:
                    os.close(source_fd)
            output.write(bytes(2 * BLOCK))
            output.flush()
            os.fsync(output.fileno())
        return validate_archive_fd(fd)
    except Exception:
        os.close(fd)
        try:
            os.unlink(target)
        except FileNotFoundError:
            pass
        raise
    finally:
        try:
            os.close(fd)
        except OSError:
            pass


def encrypt_archive_with_age(
    archive_path: os.PathLike[str] | str,
    ciphertext_path: os.PathLike[str] | str,
    recipient: str,
    age_binary: os.PathLike[str] | str = "/opt/local/bin/age",
) -> EncryptionResult:
    """Validate and stream the exact same archive descriptor to age stdin."""
    archive_flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    archive_fd = os.open(archive_path, archive_flags)
    ciphertext_fd: int | None = None
    ciphertext_created = False
    process: subprocess.Popen[bytes] | None = None
    target = os.fspath(ciphertext_path)
    try:
        before = _metadata(archive_fd)
        validation = validate_archive_fd(archive_fd)
        ciphertext_flags = os.O_RDWR | os.O_CREAT | os.O_EXCL | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
        ciphertext_fd = os.open(target, ciphertext_flags, 0o600)
        ciphertext_created = True
        os.fchmod(ciphertext_fd, 0o600)
        process = subprocess.Popen(
            [os.fspath(age_binary), "--recipient", recipient],
            stdin=subprocess.PIPE,
            stdout=ciphertext_fd,
            stderr=subprocess.PIPE,
        )
        assert process.stdin is not None
        os.lseek(archive_fd, 0, os.SEEK_SET)
        digest = hashlib.sha256()
        count = 0
        while count < validation.archive_size:
            chunk = os.read(archive_fd, min(1024 * 1024, validation.archive_size - count))
            if not chunk:
                raise ValueError("archive ended before validated size during age stream")
            process.stdin.write(chunk)
            digest.update(chunk)
            count += len(chunk)
        if os.read(archive_fd, 1):
            raise ValueError("archive has bytes beyond validated size during age stream")
        process.stdin.close()
        stderr = process.stderr.read() if process.stderr is not None else b""
        returncode = process.wait()
        if process.stderr is not None:
            process.stderr.close()
        if returncode:
            raise RuntimeError(f"age failed with exit {returncode}: {stderr.decode('utf-8', 'replace').strip()}")
        streamed_sha = digest.hexdigest()
        if count != validation.archive_size or streamed_sha != validation.archive_sha256:
            raise ValueError("bytes streamed to age differ from validated archive")

        os.lseek(archive_fd, 0, os.SEEK_SET)
        post_sha = _hash_fd(archive_fd, validation.archive_size)
        after = _metadata(archive_fd)
        if after != before or post_sha != validation.archive_sha256:
            raise ValueError("archive descriptor changed after age stream")
        os.fsync(ciphertext_fd)
        ciphertext_size = os.fstat(ciphertext_fd).st_size
        ciphertext_sha = _hash_fd(ciphertext_fd, ciphertext_size)
        return EncryptionResult(
            validation,
            streamed_sha,
            count,
            post_sha,
            before,
            after,
            ciphertext_sha,
            ciphertext_size,
        )
    except Exception:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
        if ciphertext_fd is not None:
            os.close(ciphertext_fd)
            ciphertext_fd = None
        if ciphertext_created:
            try:
                os.unlink(target)
            except FileNotFoundError:
                pass
        raise
    finally:
        if process is not None:
            if process.stdin is not None and not process.stdin.closed:
                try:
                    process.stdin.close()
                except OSError:
                    pass
            if process.stderr is not None and not process.stderr.closed:
                process.stderr.close()
        os.close(archive_fd)
        if ciphertext_fd is not None:
            os.close(ciphertext_fd)


def public_layout_values(result: EncryptionResult, validator_sha256: str) -> dict[str, Any]:
    """Return the exact closed, privacy-safe R60 layout-receipt values."""
    if not isinstance(result, EncryptionResult):
        raise TypeError("layout receipt publication requires an EncryptionResult")
    validation = result.validation
    if (
        result.streamed_archive_size != validation.archive_size
        or result.streamed_archive_sha256 != validation.archive_sha256
        or result.post_stream_archive_sha256 != validation.archive_sha256
        or result.descriptor_metadata_before
        != validation.descriptor_metadata
        or validation.descriptor_metadata != result.descriptor_metadata_after
    ):
        raise ValueError("same archive object proof drift")
    for label, value in (("ciphertext", result.ciphertext_sha256), ("validator", validator_sha256)):
        if len(value) != 64 or any(character not in "0123456789abcdef" for character in value):
            raise ValueError(f"{label} SHA-256 is not lowercase hexadecimal")
    return {
        "canonical_ustar_pass": True,
        "ciphertext_sha256": result.ciphertext_sha256,
        "entry_ids": list(validation.entry_ids),
        "layout_pass": True,
        "layout_validator_sha256": validator_sha256,
        "manifest_binding_pass": True,
        "manifest_sha256": validation.manifest_sha256,
        "member_name_digest_sha256": validation.member_name_digest_sha256,
        "plan_revision": 60,
        "private_manifest_commitment_sha256": validation.private_manifest_commitment_sha256,
        "required_root_present": True,
        "restricted_values_disclosed": False,
        "same_archive_object_pass": True,
        "schema": "hanonly.r60.layout-receipt.v1",
        "wrapper_absent": True,
    }
