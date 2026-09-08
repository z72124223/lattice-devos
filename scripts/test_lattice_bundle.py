import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("bundle", Path(__file__).with_name("lattice-bundle.py"))
B = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(B)


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lattice-bundle-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "bin").mkdir()
        (self.root / "bin/runtime.exe").write_bytes(b"test-runtime")
        files, size = B.inventory(self.root)
        self.manifest = {"schema": B.SCHEMA, "platform": "windows-x86_64", "files": files, "total_bytes": size}
        self.digest = self.save()

    def save(self):
        path = self.root / "bundle.json"
        path.write_bytes(B.M.CONFIG.json_bytes(self.manifest))
        return B.sha(path)

    def test_verified_bundle_rejects_changed_executable(self):
        B.verify(self.root, self.digest)
        (self.root / "bin/runtime.exe").write_bytes(b"substituted-runtime")
        with self.assertRaisesRegex(B.M.Rejected, "BUNDLE_CONTENT_CHANGED"):
            B.verify(self.root, self.digest)

    def test_unlisted_dll_cannot_enter_bundle(self):
        (self.root / "bin/unlisted.dll").write_bytes(b"new")
        with self.assertRaisesRegex(B.M.Rejected, "BUNDLE_CONTENT_CHANGED"):
            B.verify(self.root, self.digest)

    def test_manifest_change_cannot_reuse_old_trusted_digest(self):
        self.manifest["total_bytes"] += 1
        self.save()
        with self.assertRaisesRegex(B.M.Rejected, "MANIFEST_DIGEST_REJECTED"):
            B.verify(self.root, self.digest)

    def test_manifest_rejects_parent_absolute_and_windows_stream_paths(self):
        for path in ("../outside", "/outside", "C:/outside", "bin/file:stream", "bin\\runtime.exe"):
            self.manifest["files"] = {path: {"sha256": "a" * 64, "bytes": 1}}
            digest = self.save()
            with self.assertRaisesRegex(B.M.Rejected, "BUNDLE_PATH_REJECTED"):
                B.verify(self.root, digest)

    def test_install_requires_bundled_python_before_creating_state(self):
        with tempfile.TemporaryDirectory(prefix="bundle-customer-test-") as directory:
            customer = Path(directory)
            state = customer / "fresh-state"
            with self.assertRaisesRegex(B.M.Rejected, "USE_BUNDLED_PYTHON"):
                B.install(self.root, self.digest, state, customer, self.root / "wsl.exe")
            self.assertFalse(state.exists())

    def test_parent_alias_output_inside_source_is_rejected_before_creation(self):
        output = self.root / "bin" / ".." / "out"
        runtime = self.root / "bin/runtime.exe"
        with self.assertRaisesRegex(B.M.Rejected, "SOURCE_OUTPUT_OVERLAP"):
            B.build(output, runtime, B.sha(runtime), self.root, self.root, self.root, self.root)
        self.assertFalse((self.root / "out").exists())

    def test_customer_state_cannot_be_created_inside_bundle(self):
        state = self.root / "state"
        with self.assertRaisesRegex(B.M.Rejected, "CUSTOMER_DATA_OVERLAP"):
            B.install(self.root, self.digest, state, self.root, self.root / "wsl.exe")
        self.assertFalse(state.exists())

    def test_copy_exclusions_are_case_insensitive(self):
        source = self.root / "source"
        (source / "Site-Packages").mkdir(parents=True)
        (source / "Site-Packages/private.txt").write_text("must not copy")
        (source / "vendor.txt").write_text("copy")
        B.copy_tree(source, self.root / "copy", ("site-packages",))
        self.assertFalse((self.root / "copy/Site-Packages").exists())
        self.assertTrue((self.root / "copy/vendor.txt").is_file())

    def test_capacity_is_enforced_before_copying_large_file(self):
        with patch.object(B, "MAX_BYTES", 1):
            with self.assertRaisesRegex(B.M.Rejected, "CAPACITY_REJECTED"):
                B.copy_tree(self.root / "bin", self.root / "copy")
        self.assertFalse((self.root / "copy/runtime.exe").exists())

    def test_installed_dependency_file_set_rejects_unlisted_module(self):
        config = {"dependency_root": str(self.root), "files": {
            str(self.root / "bin/runtime.exe"): B.sha(self.root / "bin/runtime.exe"), str(self.root / "bundle.json"): B.sha(self.root / "bundle.json")}}
        B.M.verify_dependency_file_set(config)
        (self.root / "sitecustomize.py").write_text("unlisted")
        with self.assertRaisesRegex(B.M.Rejected, "DEPENDENCY_FILE_SET_CHANGED"):
            B.M.verify_dependency_file_set(config)

    def test_static_symbolic_link_is_rejected(self):
        link = self.root / "alias"
        try:
            link.symlink_to(self.root / "bin", target_is_directory=True)
        except OSError:
            self.skipTest("Windows symbolic-link privilege unavailable")
        with self.assertRaisesRegex(B.M.Rejected, "PATH_REDIRECTION_REJECTED"):
            B.inventory(self.root)


if __name__ == "__main__":
    unittest.main()
