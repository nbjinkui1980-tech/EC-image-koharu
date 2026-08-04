#!/usr/bin/env python3

import contextlib
import hashlib
import json
import os
import stat
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest import mock

import hanonly_tar_layout as layout


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


GOLDEN_DIRECTORY_HEADER = (
    b"assets/"
    + bytes(93)
    + b"0000700\0"
    + b"0000000\0"
    + b"0000000\0"
    + b"00000000000\0"
    + b"00000000000\0"
    + b"007235\0 "
    + b"5"
    + bytes(100)
    + b"ustar\0"
    + b"00"
    + bytes(247)
)
GOLDEN_FILE_HEADER = (
    b"hashes.json"
    + bytes(89)
    + b"0000600\0"
    + b"0000000\0"
    + b"0000000\0"
    + b"00000000002\0"
    + b"00000000000\0"
    + b"010073\0 "
    + b"0"
    + bytes(100)
    + b"ustar\0"
    + b"00"
    + bytes(247)
)


def manifest(asset="assets/source.png"):
    entries = []
    for index, entry_id in enumerate(layout.EXPECTED_ENTRY_IDS, 1):
        target = {
            "clean_reference_edit_roi": [0, 0, 1, 1],
            "effect": "none",
            "erase_source_ink_mask_relpath": f"assets/e{index}.png",
            "expected": "text",
            "id": f"t{index}",
            "position": "center",
            "residual_source_ink_mask_relpath": f"assets/r{index}.png",
            "source_roi": [0, 0, 1, 1],
            "translation_length": 2,
            "writing": "horizontal",
        }
        entries.append(
            {
                "aspect": "square",
                "background": "plain",
                "clean_reference_relpath": f"assets/c{index}.png",
                "dimension_bin": "small",
                "id": entry_id,
                "multi_node": False,
                "protected_rois": [],
                "role": "holdout",
                "source_relpath": asset if index == 1 else f"assets/s{index}.png",
                "targets": [target],
            }
        )
    return {
        "contract": "hanonly-r60-holdout-manifest-v1",
        "entries": entries,
        "plan_revision": 60,
        "role": "holdout",
    }


class TarLayoutTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name) / "bundle"
        self.root.mkdir()
        (self.root / "assets").mkdir()
        value = manifest()
        refs = set()
        for entry in value["entries"]:
            refs.update((entry["source_relpath"], entry["clean_reference_relpath"]))
            for target in entry["targets"]:
                refs.update((target["erase_source_ink_mask_relpath"], target["residual_source_ink_mask_relpath"]))
        for ref in refs:
            path = self.root / ref
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(ref.encode())
        (self.root / "manifest.json").write_bytes(canonical(value))
        (self.root / "oracle.json").write_bytes(b"{}")
        (self.root / "hashes.json").write_bytes(b"{}")
        self.archive = Path(self.temp.name) / "bundle.tar"

    def tearDown(self):
        self.temp.cleanup()

    def write(self):
        return layout.write_canonical_archive(self.root, self.archive)

    def validate_bytes(self, data):
        path = Path(self.temp.name) / "mutated.tar"
        path.write_bytes(data)
        fd = os.open(path, os.O_RDONLY)
        try:
            return layout.validate_archive_fd(fd)
        finally:
            os.close(fd)

    def test_golden_headers_and_flat_roundtrip(self):
        result = self.write()
        data = self.archive.read_bytes()
        self.assertEqual(len(GOLDEN_DIRECTORY_HEADER), 512)
        self.assertEqual(len(GOLDEN_FILE_HEADER), 512)
        self.assertEqual(result.member_names, tuple(sorted(result.member_names, key=lambda name: name.encode())))
        self.assertEqual(result.archive_sha256, hashlib.sha256(data).hexdigest())
        self.assertEqual(data[:512], GOLDEN_DIRECTORY_HEADER)
        file_index = result.member_names.index("hashes.json")
        offset = 0
        for name in result.member_names[:file_index]:
            size = int(data[offset + 124 : offset + 135], 8)
            offset += 512 + (size + 511) // 512 * 512
        self.assertEqual(data[offset : offset + 512], GOLDEN_FILE_HEADER)
        self.assertEqual(data[-1024:], bytes(1024))

    def test_wrapped_root_rejected(self):
        wrapped = Path(self.temp.name) / "wrapped"
        wrapped.mkdir()
        self.root.rename(wrapped / "bundle")
        with self.assertRaisesRegex(ValueError, "manifest.json is absent"):
            layout.write_canonical_archive(wrapped, self.archive)
        self.assertFalse(self.archive.exists())

    def test_manifest_bindings_and_canonical_json(self):
        value = manifest("assets/nested/source.png")
        nested = self.root / "assets/nested"
        nested.mkdir()
        (nested / "source.png").write_bytes(b"nested")
        (self.root / "assets/source.png").unlink()
        (self.root / "manifest.json").write_bytes(canonical(value))
        result = self.write()
        self.assertIn("assets/nested/", result.member_names)
        self.assertEqual(result.entry_ids, layout.EXPECTED_ENTRY_IDS)

        self.archive.unlink()
        (self.root / "manifest.json").write_text(json.dumps(value, indent=2))
        with self.assertRaisesRegex(ValueError, "canonical"):
            self.write()

    def test_duplicate_unknown_and_wrong_ids_rejected(self):
        unknown_entry = manifest()
        unknown_entry["entries"][0]["unknown"] = 1
        unknown_target = manifest()
        unknown_target["entries"][0]["targets"][0]["unknown"] = 1
        tests = [
            b'{"contract":"x","contract":"hanonly-r60-holdout-manifest-v1","entries":[],"plan_revision":60,"role":"holdout"}',
            canonical({**manifest(), "unknown": 1}),
            canonical({**manifest(), "contract": "wrong"}),
            canonical({**manifest(), "plan_revision": 59}),
            canonical({**manifest(), "role": "calibration"}),
            canonical(unknown_entry),
            canonical(unknown_target),
            canonical({**manifest(), "entries": [{**manifest()["entries"][0], "id": "wrong"}]}),
        ]
        for index, payload in enumerate(tests):
            with self.subTest(index=index):
                (self.root / "manifest.json").write_bytes(payload)
                with self.assertRaises(ValueError):
                    self.write()
                self.assertFalse(self.archive.exists())

    def test_raw_header_and_member_mutations_rejected(self):
        self.write()
        original = self.archive.read_bytes()
        cases = {}
        for label, index, value in (
            ("name-padding", 99, 1),
            ("mode", 100, ord("1")),
            ("uid", 108, ord("1")),
            ("gid", 116, ord("1")),
            ("size", 124, ord("8")),
            ("mtime", 136, ord("1")),
            ("checksum-format", 155, 0),
            ("type", 156, ord("2")),
            ("linkname", 157, 1),
            ("magic", 257, 1),
            ("version", 263, 1),
            ("uname", 265, 1),
            ("gname", 297, 1),
            ("devmajor", 329, 1),
            ("devminor", 337, 1),
            ("prefix", 345, 1),
            ("tail", 500, 1),
        ):
            changed = bytearray(original)
            changed[index] = value
            if label != "checksum-format":
                self._fix_checksum(changed, 0)
            cases[label] = bytes(changed)
        changed = bytearray(original)
        changed[148] = ord("7") if changed[148] != ord("7") else ord("6")
        cases["checksum-value"] = bytes(changed)
        first_file = self._entry_end(original, 0)
        file_size = int(original[first_file + 124 : first_file + 135], 8)
        changed = bytearray(original)
        changed[first_file + 512 + file_size] = 1
        cases["payload-padding"] = bytes(changed)
        cases["one-zero-block"] = original[:-512]
        cases["first-terminal-zero-block"] = original[:-1024] + b"x" + original[-1023:]
        cases["second-terminal-zero-block"] = original[:-512] + b"x" + original[-511:]
        cases["trailing"] = original + bytes(512)
        cases["order"] = self._swap_first_two(original)
        cases["duplicate"] = self._duplicate_first(original)
        for label, name in (
            ("absolute-name", b"/bad/"),
            ("dot-name", b"./bad/"),
            ("dot-dot-name", b"a/../bad/"),
            ("backslash-name", b"a\\bad/"),
        ):
            cases[label] = self._rename_first(original, name)
        for label, data in cases.items():
            with self.subTest(label=label), self.assertRaises(ValueError):
                self.validate_bytes(data)

    def test_name_numeric_and_framing_mutations_rejected_independently(self):
        self.write()
        original = self.archive.read_bytes()
        file_offset = self._member_offset(original, "assets/c1.png")
        cases = {"non-512-framing": original[:-1]}
        for label, offset, index, value, fix_checksum in (
            ("zero-name", 0, 0, 0, True),
            ("invalid-utf8-name", 0, 0, 0xFF, True),
            ("name-nul-padding", 0, 99, 1, True),
            ("name-prefix", 0, 345, 1, True),
            ("directory-mode-alphabet", 0, 100, ord("8"), True),
            ("directory-mode-terminator", 0, 107, ord(" "), True),
            ("file-mode", file_offset, 100, ord("7"), True),
            ("uid-alphabet", 0, 108, ord("8"), True),
            ("uid-terminator", 0, 115, ord(" "), True),
            ("gid-alphabet", 0, 116, ord("8"), True),
            ("gid-terminator", 0, 123, ord(" "), True),
            ("size-alphabet", 0, 124, ord("8"), True),
            ("size-terminator", 0, 135, ord(" "), True),
            ("mtime-alphabet", 0, 136, ord("8"), True),
            ("mtime-terminator", 0, 147, ord(" "), True),
            ("checksum-alphabet", 0, 148, ord("8"), False),
            ("checksum-nul", 0, 154, ord(" "), False),
            ("checksum-final-space", 0, 155, 0, False),
        ):
            changed = bytearray(original)
            changed[offset + index] = value
            if fix_checksum:
                self._fix_checksum(changed, offset)
            cases[label] = bytes(changed)
        changed = bytearray(original)
        changed[:100] = b"a" * 99 + b"/"
        changed[345] = ord("b")
        self._fix_checksum(changed, 0)
        cases["name-over-100"] = bytes(changed)
        changed = bytearray(original)
        raw_sum = sum(changed[:512])
        changed[148:156] = f"{raw_sum:06o}\0 ".encode()
        cases["checksum-without-space-substitution"] = bytes(changed)
        for label, data in cases.items():
            with self.subTest(label=label), self.assertRaises(ValueError):
                self.validate_bytes(data)

    def test_directory_type_and_optional_header_mutations_rejected(self):
        self.write()
        original = self.archive.read_bytes()
        file_offset = self._member_offset(original, "assets/c1.png")
        cases = {}
        changed = bytearray(original)
        changed[124:136] = b"00000000001\0"
        self._fix_checksum(changed, 0)
        cases["directory-nonzero-size"] = bytes(changed)
        cases["directory-payload-byte"] = original[:512] + b"x" + bytes(511) + original[512:]
        changed = bytearray(original)
        file_size = int(changed[file_offset + 124 : file_offset + 135], 8)
        changed[file_offset + 124 : file_offset + 136] = f"{file_size + 512:011o}\0".encode()
        self._fix_checksum(changed, file_offset)
        cases["regular-declared-size"] = bytes(changed)
        changed = bytearray(original)
        changed[:100] = bytes(100)
        changed[:6] = b"assets"
        changed[100:108] = b"0000600\0"
        changed[156] = ord("0")
        self._fix_checksum(changed, 0)
        cases["file-directory-prefix-collision"] = bytes(changed)
        for label, offset, typeflag in (
            ("directory-type-not-5", 0, b"0"),
            ("regular-type-not-0", file_offset, b"5"),
            ("hardlink", file_offset, b"1"),
            ("symlink", file_offset, b"2"),
            ("character-device", file_offset, b"3"),
            ("block-device", file_offset, b"4"),
            ("fifo", file_offset, b"6"),
            ("pax", file_offset, b"x"),
            ("gnu-long-name", file_offset, b"L"),
            ("gnu-sparse", file_offset, b"S"),
            ("unknown-type", file_offset, b"7"),
        ):
            changed = bytearray(original)
            changed[offset + 156 : offset + 157] = typeflag
            self._fix_checksum(changed, offset)
            cases[label] = bytes(changed)
        for label, index in (
            ("linkname", 157),
            ("devmajor", 329),
            ("devminor", 337),
            ("final-padding", 500),
        ):
            changed = bytearray(original)
            changed[index] = 1
            self._fix_checksum(changed, 0)
            cases[label] = bytes(changed)
        for label, data in cases.items():
            with self.subTest(label=label), self.assertRaises(ValueError):
                self.validate_bytes(data)

    def test_manifest_derived_member_set_mutations_rejected(self):
        source = self.root / "assets/source.png"
        source_bytes = source.read_bytes()
        source.unlink()
        with self.assertRaisesRegex(ValueError, "member set"):
            self.write()
        source.write_bytes(source_bytes)

        extra = self.root / "assets/extra.png"
        extra.write_bytes(b"extra")
        with self.assertRaisesRegex(ValueError, "member set"):
            self.write()
        extra.unlink()

        extra_directory = self.root / "assets/extra"
        extra_directory.mkdir()
        with self.assertRaisesRegex(ValueError, "member set"):
            self.write()
        extra_directory.rmdir()

        extra_root = self.root / "extra.json"
        extra_root.write_bytes(b"{}")
        with self.assertRaisesRegex(ValueError, "member set"):
            self.write()
        extra_root.unlink()

        value = manifest("assets/nested/source.png")
        nested = self.root / "assets/nested"
        nested.mkdir()
        (nested / "source.png").write_bytes(b"nested")
        source.unlink()
        (self.root / "manifest.json").write_bytes(canonical(value))
        self.write()
        without_ancestor = self._remove_member(self.archive.read_bytes(), "assets/nested/")
        with self.assertRaisesRegex(ValueError, "member set"):
            self.validate_bytes(without_ancestor)

    @staticmethod
    def _entry_end(data, offset):
        size = int(data[offset + 124 : offset + 135], 8)
        return offset + 512 + (size + 511) // 512 * 512

    def _member_offset(self, data, expected_name):
        offset = 0
        while any(data[offset : offset + 512]):
            name = data[offset : offset + 100].split(b"\0", 1)[0].decode()
            if name == expected_name:
                return offset
            offset = self._entry_end(data, offset)
        self.fail(f"missing fixture member: {expected_name}")

    def _remove_member(self, data, name):
        offset = self._member_offset(data, name)
        return data[:offset] + data[self._entry_end(data, offset) :]

    @staticmethod
    def _fix_checksum(data, offset):
        data[offset + 148 : offset + 156] = b"        "
        data[offset + 148 : offset + 156] = f"{sum(data[offset:offset + 512]):06o}\0 ".encode()

    def _swap_first_two(self, data):
        first = self._entry_end(data, 0)
        second = self._entry_end(data, first)
        return data[first:second] + data[:first] + data[second:]

    def _duplicate_first(self, data):
        first = self._entry_end(data, 0)
        return data[:first] + data[:first] + data[first:-1024] + bytes(1024)

    def _rename_first(self, data, name):
        changed = bytearray(data)
        changed[:100] = bytes(100)
        changed[: len(name)] = name
        changed[148:156] = b"        "
        changed[148:156] = f"{sum(changed[:512]):06o}\0 ".encode()
        return bytes(changed)

    def encrypt(self):
        validation = self.write()
        mock = Path(self.temp.name) / "mock-age.py"
        mock.write_text(
            "#!/usr/bin/env python3\n"
            "import sys\n"
            "data=sys.stdin.buffer.read()\n"
            "sys.stdout.buffer.write(b'AGE'+data)\n"
        )
        mock.chmod(mock.stat().st_mode | stat.S_IXUSR)
        ciphertext = Path(self.temp.name) / "bundle.age"
        result = layout.encrypt_archive_with_age(
            self.archive, ciphertext, "age1test", mock
        )
        return validation, result, ciphertext

    def test_public_values_are_closed_and_omit_private_names(self):
        validation, result, _ = self.encrypt()
        values = layout.public_layout_values(result, "b" * 64)
        self.assertNotIn("member_names", values)
        self.assertNotIn("archive_size", values)
        self.assertEqual(values["ciphertext_sha256"], result.ciphertext_sha256)
        self.assertEqual(set(values), {
            "canonical_ustar_pass", "ciphertext_sha256", "entry_ids", "layout_pass",
            "layout_validator_sha256", "manifest_binding_pass", "manifest_sha256",
            "member_name_digest_sha256", "plan_revision",
            "private_manifest_commitment_sha256", "required_root_present",
            "restricted_values_disclosed", "same_archive_object_pass", "schema", "wrapper_absent",
        })
        self.assertNotIn("plaintext_archive_sha256", values)
        with self.assertRaises(TypeError):
            layout.public_layout_values(validation, "b" * 64)
        with self.assertRaises(TypeError):
            layout.public_layout_values(result, "a" * 64, "b" * 64)

    def test_public_values_reject_tampered_stream_proof(self):
        _, result, _ = self.encrypt()
        changed_metadata = replace(
            result.descriptor_metadata_before,
            mtime_ns=result.descriptor_metadata_before.mtime_ns + 1,
        )
        cases = {
            "streamed-size": replace(
                result, streamed_archive_size=result.streamed_archive_size - 1
            ),
            "streamed-sha": replace(result, streamed_archive_sha256="0" * 64),
            "post-stream-sha": replace(result, post_stream_archive_sha256="0" * 64),
            "descriptor-before": replace(
                result, descriptor_metadata_before=changed_metadata
            ),
            "descriptor-after": replace(
                result, descriptor_metadata_after=changed_metadata
            ),
        }
        for label, changed in cases.items():
            with self.subTest(label=label), self.assertRaisesRegex(
                ValueError, "same archive object proof"
            ):
                layout.public_layout_values(changed, "b" * 64)
        with self.assertRaisesRegex(ValueError, "ciphertext SHA-256"):
            layout.public_layout_values(
                replace(result, ciphertext_sha256="A" * 64), "b" * 64
            )
        with self.assertRaisesRegex(ValueError, "validator SHA-256"):
            layout.public_layout_values(result, "invalid")

    def test_age_streams_exact_validated_bytes_and_create_new(self):
        validation, result, ciphertext = self.encrypt()
        self.assertEqual(result.validation.archive_sha256, validation.archive_sha256)
        self.assertEqual(result.streamed_archive_size, validation.archive_size)
        self.assertEqual(result.streamed_archive_sha256, validation.archive_sha256)
        self.assertEqual(result.post_stream_archive_sha256, validation.archive_sha256)
        self.assertEqual(ciphertext.read_bytes(), b"AGE" + self.archive.read_bytes())
        self.assertEqual(result.ciphertext_sha256, hashlib.sha256(ciphertext.read_bytes()).hexdigest())
        mock = Path(self.temp.name) / "mock-age.py"
        with self.assertRaises(FileExistsError):
            layout.encrypt_archive_with_age(self.archive, ciphertext, "age1test", mock)
        self.assertEqual(ciphertext.read_bytes(), b"AGE" + self.archive.read_bytes())

    def test_age_failure_removes_partial_ciphertext(self):
        self.write()
        mock = Path(self.temp.name) / "bad-age.py"
        mock.write_text("#!/usr/bin/env python3\nimport sys\nsys.stdout.buffer.write(b'partial')\nsys.exit(3)\n")
        mock.chmod(mock.stat().st_mode | stat.S_IXUSR)
        ciphertext = Path(self.temp.name) / "failed.age"
        with self.assertRaises(RuntimeError):
            layout.encrypt_archive_with_age(self.archive, ciphertext, "age1test", mock)
        self.assertFalse(ciphertext.exists())

    def test_age_runtime_faults_fail_closed_and_remove_ciphertext(self):
        age = Path(self.temp.name) / "fault-age.py"
        age.write_text(
            "#!/usr/bin/env python3\n"
            "import sys\n"
            "sys.stdout.buffer.write(sys.stdin.buffer.read())\n"
        )
        age.chmod(age.stat().st_mode | stat.S_IXUSR)
        faults = (
            "descriptor-replacement",
            "archive-mutation",
            "omitted-pre-seek",
            "omitted-post-seek",
            "short-stream",
            "long-stream-missing-eof",
            "post-age-rehash",
            "inode-change",
            "size-change",
            "mtime-change",
        )
        for fault in faults:
            with self.subTest(fault=fault):
                if self.archive.exists():
                    self.archive.unlink()
                self.write()
                ciphertext = Path(self.temp.name) / f"{fault}.age"
                real_validate = layout.validate_archive_fd
                real_lseek = os.lseek
                real_read = os.read
                real_hash_fd = layout._hash_fd
                real_metadata = layout._metadata
                with contextlib.ExitStack() as stack:
                    if fault == "descriptor-replacement":
                        alternate = Path(self.temp.name) / "alternate.tar"
                        alternate.write_bytes(self.archive.read_bytes())

                        def replace_descriptor(fd):
                            result = real_validate(fd)
                            alternate_fd = os.open(alternate, os.O_RDONLY)
                            try:
                                os.dup2(alternate_fd, fd)
                            finally:
                                os.close(alternate_fd)
                            return result

                        stack.enter_context(
                            mock.patch.object(
                                layout,
                                "validate_archive_fd",
                                side_effect=replace_descriptor,
                            )
                        )
                    elif fault == "archive-mutation":

                        def mutate_archive(fd):
                            result = real_validate(fd)
                            writer = os.open(self.archive, os.O_WRONLY)
                            try:
                                os.pwrite(writer, b"X", layout.BLOCK)
                            finally:
                                os.close(writer)
                            return result

                        stack.enter_context(
                            mock.patch.object(
                                layout,
                                "validate_archive_fd",
                                side_effect=mutate_archive,
                            )
                        )
                    elif fault == "omitted-pre-seek":
                        seek_calls = 0

                        def offset_after_validation(fd):
                            result = real_validate(fd)
                            real_lseek(fd, 1, os.SEEK_SET)
                            return result

                        def omit_first_seek(fd, offset, whence):
                            nonlocal seek_calls
                            seek_calls += 1
                            return offset if seek_calls == 1 else real_lseek(fd, offset, whence)

                        stack.enter_context(
                            mock.patch.object(
                                layout,
                                "validate_archive_fd",
                                side_effect=offset_after_validation,
                            )
                        )
                        stack.enter_context(
                            mock.patch.object(layout.os, "lseek", side_effect=omit_first_seek)
                        )
                    elif fault == "omitted-post-seek":
                        seek_calls = 0
                        hash_calls = 0

                        def omit_second_seek(fd, offset, whence):
                            nonlocal seek_calls
                            seek_calls += 1
                            if seek_calls == 2:
                                return real_lseek(fd, 0, os.SEEK_CUR)
                            return real_lseek(fd, offset, whence)

                        def require_post_seek(fd, size):
                            nonlocal hash_calls
                            hash_calls += 1
                            if hash_calls == 2 and real_lseek(fd, 0, os.SEEK_CUR) != 0:
                                raise ValueError("post-age seek omitted")
                            return real_hash_fd(fd, size)

                        stack.enter_context(
                            mock.patch.object(layout.os, "lseek", side_effect=omit_second_seek)
                        )
                        stack.enter_context(
                            mock.patch.object(layout, "_hash_fd", side_effect=require_post_seek)
                        )
                    elif fault == "short-stream":
                        stack.enter_context(
                            mock.patch.object(layout.os, "read", return_value=b"")
                        )
                    elif fault == "long-stream-missing-eof":
                        stack.enter_context(
                            mock.patch.object(
                                layout.os,
                                "read",
                                side_effect=lambda fd, size: b"x"
                                if size == 1
                                else real_read(fd, size),
                            )
                        )
                    elif fault == "post-age-rehash":
                        hash_calls = 0

                        def mismatch_post_hash(fd, size):
                            nonlocal hash_calls
                            hash_calls += 1
                            return "0" * 64 if hash_calls == 2 else real_hash_fd(fd, size)

                        stack.enter_context(
                            mock.patch.object(layout, "_hash_fd", side_effect=mismatch_post_hash)
                        )
                    else:
                        metadata_calls = 0
                        field = {
                            "inode-change": "inode",
                            "size-change": "size",
                            "mtime-change": "mtime_ns",
                        }[fault]

                        def changed_metadata(fd):
                            nonlocal metadata_calls
                            metadata_calls += 1
                            value = real_metadata(fd)
                            if metadata_calls == 4:
                                return replace(value, **{field: getattr(value, field) + 1})
                            return value

                        stack.enter_context(
                            mock.patch.object(layout, "_metadata", side_effect=changed_metadata)
                        )
                    with self.assertRaises(ValueError):
                        layout.encrypt_archive_with_age(
                            self.archive, ciphertext, "age1test", age
                        )
                self.assertFalse(ciphertext.exists())


if __name__ == "__main__":
    unittest.main()
