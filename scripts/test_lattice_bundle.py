import importlib.util
from pathlib import Path
import tempfile
import unittest
import io
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("bundle", Path(__file__).with_name("lattice-bundle.py"))
B = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(B)


class BundleTests(unittest.TestCase):
    def test_vc_runtime_is_pinned_in_both_executable_directories(self):
        source = self.root / "redist"; source.mkdir()
        payloads = {"vcruntime140.dll": b"official-fixture-crt", "msvcp140.dll": b"official-fixture-cpp"}
        for name, payload in payloads.items(): (source / name).write_bytes(payload)
        pinned = {name: B.sha(source / name) for name in payloads}
        license_path = source / "license.docx"; license_path.write_bytes(b"official-license-fixture")
        redist_list = source / "Redist.txt"; redist_list.write_bytes(b"official-redist-fixture")
        (self.root / "postgres/bin").mkdir(parents=True)
        with patch.dict(B.M.VC_RUNTIME_FILES, pinned, clear=True), \
                patch.multiple(B, VC_LICENSE_SHA256=B.sha(license_path), VC_REDIST_LIST_SHA256=B.sha(redist_list)):
            B.bundle_vc_redist(self.root, source, license_path, redist_list)
            for location in ("bin", "postgres/bin"):
                for name, payload in payloads.items():
                    self.assertEqual((self.root / location / name).read_bytes(), payload)
            self.manifest["vc_runtime"] = B.vc_redist_provenance()
            self.manifest["files"], self.manifest["total_bytes"] = B.inventory(self.root)
            self.digest = self.save()
            B.verify(self.root, self.digest)
            metadata = self.manifest.pop("vc_runtime")
            with self.assertRaisesRegex(B.M.Rejected, "VC_RUNTIME_PROVENANCE_REQUIRED"):
                B.verify(self.root, self.save())
            self.manifest["vc_runtime"] = metadata
            public = (self.root / "licenses/visual-cpp-runtime/provenance.json").read_text()
            self.assertNotIn(str(source), public)
            self.assertIn("Microsoft.VC143.CRT", public)
            # Even a recomputed manifest cannot authorize a substituted vendor DLL.
            (self.root / "postgres/bin/msvcp140.dll").write_bytes(b"replacement")
            self.manifest["files"], self.manifest["total_bytes"] = B.inventory(self.root)
            with self.assertRaisesRegex(B.M.Rejected, "VC_RUNTIME_SOURCE_REJECTED"):
                B.verify(self.root, self.save())

    def test_vc_runtime_missing_source_blocks_new_bundle_before_creation(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory); source = base / "source"; source.mkdir()
            runtime = source / "runtime.exe"; runtime.write_bytes(b"fixture")
            with patch.object(B, "verify_node"), self.assertRaisesRegex(B.M.Rejected, "VC_RUNTIME_SUPPLY_REQUIRED"):
                B.build(base / "bundle", runtime, B.sha(runtime), source, source, source, source, source)
            self.assertFalse((base / "bundle").exists())

    def test_bundle_rejects_wrong_wsl_image_before_creating_output(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            source = base / "source"; source.mkdir()
            runtime = source / "runtime.exe"; runtime.write_bytes(b"fixture")
            image = source / "image.wsl"; image.write_bytes(b"wrong-image")
            output = base / "bundle"
            with patch.object(B, "verify_node"), self.assertRaisesRegex(B.M.Rejected, "WSL_IMAGE_DIGEST_REJECTED"):
                B.build(output, runtime, B.sha(runtime), source, source, source, source, source, image)
            self.assertFalse(output.exists())

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

    def test_node_supply_rejects_untrusted_archive_before_creating_output(self):
        output = self.root / "node"
        with patch.object(B.urllib.request, "urlopen", return_value=io.BytesIO(b"not an official archive")):
            with self.assertRaisesRegex(B.M.Rejected, "NODE_ARCHIVE_DIGEST_REJECTED"):
                B.supply_node(output)
        self.assertFalse(output.exists())

    def test_node_supply_never_adopts_existing_directory_or_downloads(self):
        with patch.object(B.urllib.request, "urlopen", side_effect=AssertionError("No download")):
            with self.assertRaisesRegex(B.M.Rejected, "FRESH_NODE_SUPPLY_REQUIRED"):
                B.supply_node(self.root)

    def test_node_license_and_provenance_cannot_self_authorize_replacement(self):
        node = self.root / "node"; node.mkdir()
        (node / "node.exe").write_bytes(b"fixture")
        (node / "LICENSE").write_bytes(b"altered license")
        (node / "provenance.json").write_bytes(B.M.CONFIG.json_bytes({
            "schema": "lattice.node-supply.v1", "version": B.NODE_VERSION, "source": B.NODE_URL,
            "archive_sha256": B.NODE_ZIP_SHA256, "files": {"node.exe": B.NODE_EXE_SHA256, "LICENSE": B.sha(node / "LICENSE")}}))
        with self.assertRaisesRegex(B.M.Rejected, "NODE_SUPPLY_REJECTED"):
            B.verify_node(node)


if __name__ == "__main__":
    unittest.main()
