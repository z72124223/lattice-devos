import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("install_assets", Path(__file__).with_name("lattice-install-assets.py"))
A = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(A)


class AssetTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lattice-assets-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.source = self.root / "Downloads/package"
        self.source.mkdir(parents=True)
        self.parent = self.root / "LocalAppData/LATTICE/bundles"
        self.contents = {"python/python.exe": b"mock python", "bin/lattice-bundle.py": b"mock verifier", "graphify/a.py": b"print('hello')\n"}
        for name, body in self.contents.items():
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(body)
        manifest = {"files": {name: {"sha256": hashlib.sha256(body).hexdigest()} for name, body in self.contents.items()}}
        self.manifest = json.dumps(manifest).encode()
        (self.source / "bundle.json").write_bytes(self.manifest)
        self.expected = hashlib.sha256(self.manifest).hexdigest()
        self.final = self.parent / self.expected
        self.calls = []
        self.verifier = patch.object(A, "verify", side_effect=self.fake_verify).start()
        self.addCleanup(patch.stopall)

    def fake_verify(self, original, target, expected):
        self.calls.append(target)
        self.assertEqual(original, self.source)
        self.assertEqual(expected, self.expected)
        if (target / "bundle.json").read_bytes() != self.manifest:
            raise A.Rejected("MOCK_MANIFEST_CHANGED")
        if any((target / name).read_bytes() != body for name, body in self.contents.items()):
            raise A.Rejected("MOCK_CONTENT_CHANGED")

    def test_stages_and_reads_back_only_after_source_and_temporary_validation(self):
        result = A.stage(self.source, self.parent)
        self.assertEqual(result, {"status": "STAGED", "bundle": str(self.final), "manifest_sha256": self.expected})
        self.assertEqual(self.calls[0], self.source)
        self.assertIn(".partial-", self.calls[1].name)
        self.assertEqual(self.calls[2], self.final)
        self.assertFalse(self.calls[1].exists())
        self.assertTrue((self.source / "graphify/a.py").is_file())

    def test_same_manifest_reuses_verified_final_without_copying(self):
        A.stage(self.source, self.parent)
        self.calls.clear()
        with patch.object(A.shutil, "copyfile", side_effect=AssertionError("must not copy")):
            self.assertEqual(A.stage(self.source, self.parent)["status"], "REUSED")
        self.assertEqual(self.calls, [self.source, self.final])

    def test_changed_existing_final_is_preserved_and_rejected(self):
        A.stage(self.source, self.parent)
        changed = self.final / "graphify/a.py"
        changed.write_bytes(b"owner modified")
        with self.assertRaisesRegex(A.Rejected, "MOCK_CONTENT_CHANGED"):
            A.stage(self.source, self.parent)
        self.assertEqual(changed.read_bytes(), b"owner modified")

    def test_failed_source_verification_does_not_create_destination(self):
        self.verifier.side_effect = A.Rejected("BAD_SOURCE")
        with self.assertRaisesRegex(A.Rejected, "BAD_SOURCE"):
            A.stage(self.source, self.parent)
        self.assertFalse(self.parent.exists())

    def test_failed_target_verification_keeps_partial_and_never_publishes(self):
        def reject_temporary(original, target, expected):
            if target != original:
                raise A.Rejected("BAD_COPY")
        self.verifier.side_effect = reject_temporary
        with self.assertRaisesRegex(A.Rejected, "BAD_COPY"):
            A.stage(self.source, self.parent)
        partials = list(self.parent.glob("*.partial-*"))
        self.assertEqual(len(partials), 1)
        self.assertTrue((partials[0] / "graphify/a.py").is_file())
        self.assertFalse(self.final.exists())

    def test_copy_failure_preserves_prior_partial_and_current_partial(self):
        old = self.parent / ".old.partial-owned-by-another-run"
        old.mkdir(parents=True)
        with patch.object(A.shutil, "copyfile", side_effect=OSError("disk full")):
            with self.assertRaises(OSError):
                A.stage(self.source, self.parent)
        self.assertTrue(old.exists())
        self.assertEqual(len(list(self.parent.iterdir())), 2)
        self.assertFalse(self.final.exists())

    def test_destination_created_during_copy_is_not_overwritten(self):
        def race(original, target, expected):
            self.fake_verify(original, target, expected)
            if ".partial-" in target.name:
                self.final.mkdir()
                (self.final / "owner.txt").write_text("preserve", encoding="utf-8")
        self.verifier.side_effect = race
        with self.assertRaisesRegex(A.Rejected, "ASSET_DESTINATION_APPEARED"):
            A.stage(self.source, self.parent)
        self.assertEqual((self.final / "owner.txt").read_text(), "preserve")

    def test_overlap_rejected_in_both_directions(self):
        for destination in (self.source, self.source / "nested", self.source.parent):
            with self.assertRaisesRegex(A.Rejected, "ASSET_PATHS_OVERLAP"):
                A.stage(self.source, destination)
        self.verifier.assert_not_called()

    def test_relative_traversal_and_trailing_space_paths_are_rejected(self):
        for destination in (Path("relative"), self.parent / ".." / "alias", self.parent / "alias "):
            with self.assertRaises(A.Rejected):
                A.stage(self.source, destination)
        self.verifier.assert_not_called()

    def test_junction_rejected_without_following_it(self):
        with patch.object(Path, "lstat", return_value=SimpleNamespace(st_mode=0o40755, st_file_attributes=0x400)):
            with self.assertRaisesRegex(A.Rejected, "PATH_REDIRECTION_REJECTED"):
                A.regular_path(self.parent)

    def test_cli_failure_is_json_and_does_not_leak_os_error(self):
        self.verifier.side_effect = OSError("internal details")
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(A.main(["--bundle", str(self.source), "--destination-root", str(self.parent)]), 2)
        payload = json.loads(output.getvalue())
        self.assertEqual(payload["status"], "BLOCKED")
        self.assertNotIn("internal details", output.getvalue())


class VerifierTests(unittest.TestCase):
    def test_verification_uses_original_executable_and_script_for_existing_target(self):
        source, target = Path("C:/source"), Path("C:/installed")
        with patch.object(A, "regular_path", side_effect=lambda path: path), patch.object(A.subprocess, "run") as run:
            run.return_value = subprocess.CompletedProcess([], 0, b'{"status":"LOCAL_BUNDLE_VERIFIED"}', b"")
            A.verify(source, target, "abc")
        self.assertEqual(run.call_args.args[0], [str(source / "python/python.exe"), "-I", "-B", "-S", str(source / "bin/lattice-bundle.py"), "verify", "--bundle", str(target), "--sha256", "abc"])

    def test_exit_failure_wins_over_positive_json(self):
        with patch.object(A, "regular_path", side_effect=lambda path: path), patch.object(A.subprocess, "run") as run:
            run.return_value = subprocess.CompletedProcess([], 1, b'{"status":"LOCAL_BUNDLE_VERIFIED"}', b"")
            with self.assertRaisesRegex(A.Rejected, "ASSET_BUNDLE_VERIFICATION_FAILED"):
                A.verify(Path("C:/source"), Path("C:/target"), "abc")


if __name__ == "__main__":
    unittest.main()
