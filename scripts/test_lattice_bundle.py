import importlib.util
from pathlib import Path
import tempfile
import unittest
import io
import shutil
import subprocess
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("bundle", Path(__file__).with_name("lattice-bundle.py"))
B = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(B)


class BundleTests(unittest.TestCase):
    def purge_fixture(self):
        source_temp = tempfile.TemporaryDirectory(prefix="lattice-purge-software-test-")
        self.addCleanup(source_temp.cleanup)
        source = Path(source_temp.name)
        binary = source / "lattice-project-purge.exe"
        binary.write_bytes(b"synthetic-maintenance-binary-not-executed")
        repository = Path(__file__).resolve().parent.parent
        payload = B.project_purge_payload(binary, B.sha(binary), repository)
        runtime = self.root / "bin/latticed.exe"
        runtime.write_bytes(b"synthetic-runtime-not-executed")
        (self.root / "node").mkdir()
        (self.root / "node/node.exe").write_bytes(b"synthetic-node-not-executed")
        self.manifest["runtime_sha256"] = B.sha(runtime)
        self.manifest["project_purge"] = B.copy_project_purge(
            self.root, payload, B.sha(runtime), {"files": 0, "bytes": 0})
        self.manifest["files"], self.manifest["total_bytes"] = B.inventory(self.root)
        return self.save(), source, binary

    def test_purge_capability_requires_complete_supply_before_creating_bundle(self):
        runtime = self.root / "bin/runtime.exe"
        output = self.root / "new-bundle"
        with self.assertRaisesRegex(B.M.Rejected, "PROJECT_PURGE_SUPPLY_REQUIRED"):
            B.build(output, runtime, B.sha(runtime), self.root, self.root, self.root, self.root,
                    project_purge_binary=runtime)
        self.assertFalse(output.exists())
        with self.assertRaisesRegex(B.M.Rejected, "PROJECT_PURGE_BINARY_REQUIRED"):
            B.project_purge_payload(self.root / "absent.exe", "a" * 64, self.root)
        self.assertIsNone(B.project_purge_payload(None, None, None))

    def test_purge_supply_rejects_wrong_binary_digest_and_missing_cli(self):
        runtime = self.root / "bin/runtime.exe"
        with self.assertRaisesRegex(B.M.Rejected, "PROJECT_PURGE_BINARY_DIGEST_REJECTED"):
            B.project_purge_payload(runtime, "a" * 64, self.root)
        with self.assertRaisesRegex(B.M.Rejected, "PROJECT_PURGE_SOURCE_REQUIRED"):
            B.project_purge_payload(runtime, B.sha(runtime), self.root)

    def test_purge_capability_verifies_exact_packaged_software_and_runtime_binding(self):
        digest, _, _ = self.purge_fixture()
        with patch.object(B, "verify_node") as node_verifier:
            result = B.verify(self.root, digest)
        node_verifier.assert_called_once_with(self.root / "node")
        self.assertEqual(set(result["project_purge"]["files"]), {B.PROJECT_PURGE_BINARY, *B.PROJECT_PURGE_FILES})
        self.assertEqual((self.root / B.PROJECT_PURGE_BINARY).read_bytes(), b"synthetic-maintenance-binary-not-executed")
        self.manifest["project_purge"]["runtime_sha256"] = "a" * 64
        with self.assertRaisesRegex(B.M.Rejected, "PROJECT_PURGE_BUNDLE_REJECTED"):
            B.verify(self.root, self.save())

    def test_build_includes_purge_capability_in_verified_local_artifact(self):
        source = self.root / "software"
        for directory in ("bin", "lib", "share", "DLLs", "Lib", "tcl", "cmd", "mingw64", "usr"):
            (source / directory).mkdir(parents=True, exist_ok=True)
            (source / directory / "fixture.txt").write_bytes(b"synthetic-software-directory")
        for name in ("server_license.txt", "commandlinetools_3rd_party_licenses.txt", "python.exe",
                     "python3.dll", "python312.dll", "vcruntime140.dll", "vcruntime140_1.dll", "LICENSE.txt",
                     "node.exe", "LICENSE", "provenance.json", "lattice-project-purge.exe"):
            (source / name).write_bytes(b"synthetic-software-fixture-not-executed")
        output = self.root / "candidate"
        runtime = self.root / "bin/runtime.exe"
        binary = source / "lattice-project-purge.exe"
        repository = Path(__file__).resolve().parent.parent
        with patch.object(B, "verify_node"), patch.object(B.M, "vc_runtime_files"), \
                patch.object(B, "verify_vc_documents"), patch.object(B, "bundle_vc_redist") as copy_crt, \
                patch.object(B, "vc_redist_provenance", return_value={}):
            # Vendor redistributables are independently covered above. This fixture
            # verifies actual bundle creation and purge software inclusion only.
            def fake_crt(root, *_):
                notices = root / "licenses/visual-cpp-runtime"
                notices.mkdir(parents=True)
                (notices / "provenance.json").write_text("{}")
            copy_crt.side_effect = fake_crt
            result = B.build(output, runtime, B.sha(runtime), source, source, source, source,
                             node=source, vc_redist=source,
                             project_purge_binary=binary, project_purge_sha256=B.sha(binary),
                             project_purge_source=repository)
            manifest = B.verify(output, result["sha256"])
        self.assertEqual(result["status"], "LOCAL_BUNDLE_VERIFIED")
        self.assertEqual(manifest["project_purge"]["binary"], B.PROJECT_PURGE_BINARY)
        self.assertEqual((output / B.PROJECT_PURGE_BINARY).read_bytes(), binary.read_bytes())
        self.assertTrue((output / B.PROJECT_PURGE_ENTRYPOINT).is_file())
        self.assertEqual(manifest["project_purge"]["runtime_sha256"], B.sha(runtime))

    def test_purge_manifest_cannot_omit_required_file_even_with_recomputed_inventory(self):
        self.purge_fixture()
        required = "apps/lattice-control/data-scope-contract.json"
        del self.manifest["project_purge"]["files"][required]
        with self.assertRaisesRegex(B.M.Rejected, "PROJECT_PURGE_BUNDLE_REJECTED"):
            B.verify(self.root, self.save())

    def test_purge_payload_cannot_silently_claim_legacy_bundle(self):
        self.purge_fixture()
        del self.manifest["project_purge"]
        with self.assertRaisesRegex(B.M.Rejected, "PROJECT_PURGE_CAPABILITY_REQUIRED"):
            B.verify(self.root, self.save())

    def test_packaged_purge_cli_dependency_closure_loads_help_without_a_database(self):
        node = shutil.which("node")
        if node is None:
            self.skipTest("Node executable unavailable")
        self.purge_fixture()
        result = subprocess.run([node, str(self.root / B.PROJECT_PURGE_ENTRYPOINT), "--help"],
                                cwd=self.root, capture_output=True, text=True, encoding="utf-8", timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("preview", result.stdout)

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


class MaintenanceBundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lattice-maintenance-test-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        supply = self.base / "supply"; supply.mkdir()
        node = supply / "node"; node.mkdir()
        for name in ("node.exe", "LICENSE"):
            (node / name).write_bytes(("fixture-" + name).encode())
        node_hashes = {name: B.sha(node / name) for name in ("node.exe", "LICENSE")}
        (node / "provenance.json").write_bytes(B.M.CONFIG.json_bytes({
            "schema": "lattice.node-supply.v1", "version": B.NODE_VERSION, "source": B.NODE_URL,
            "archive_sha256": B.NODE_ZIP_SHA256, "files": node_hashes}))
        redist = supply / "redist"; redist.mkdir()
        for name in B.M.VC_RUNTIME_FILES:
            (redist / name).write_bytes(("fixture-" + name).encode())
        vc_hashes = {name: B.sha(redist / name) for name in B.M.VC_RUNTIME_FILES}
        license_path = supply / "license.docx"; license_path.write_bytes(b"fixture-license")
        redist_list = supply / "Redist.txt"; redist_list.write_bytes(b"fixture-redist-list")
        runtime = supply / "latticed.exe"; runtime.write_bytes(b"fixture-runtime-not-executed")
        binary = supply / "purge.exe"; binary.write_bytes(b"fixture-purge-not-executed")
        self.enterContext(patch.multiple(B, NODE_EXE_SHA256=node_hashes["node.exe"],
            NODE_LICENSE_SHA256=node_hashes["LICENSE"], VC_LICENSE_SHA256=B.sha(license_path),
            VC_REDIST_LIST_SHA256=B.sha(redist_list)))
        self.enterContext(patch.dict(B.M.VC_RUNTIME_FILES, vc_hashes, clear=True))
        self.enterContext(patch.object(B.M, "private_new_root", side_effect=lambda root: root.mkdir()))
        self.arguments = dict(runtime=runtime, runtime_sha=B.sha(runtime), node=node,
            vc_redist=redist, vc_license=license_path, vc_redist_list=redist_list,
            project_purge_binary=binary, project_purge_sha256=B.sha(binary),
            project_purge_source=Path(__file__).resolve().parent.parent)
        self.number = 0

    def build(self):
        self.number += 1
        self.root = self.base / ("output-" + str(self.number))
        result = B.build_maintenance(self.root, **self.arguments)
        self.manifest = B.json.loads((self.root / "bundle.json").read_bytes())
        return result["sha256"]

    def save(self, *, inventory=False):
        if inventory:
            self.manifest["files"], self.manifest["total_bytes"] = B.inventory(self.root)
        (self.root / "bundle.json").write_bytes(B.M.CONFIG.json_bytes(self.manifest))
        return B.sha(self.root / "bundle.json")

    def test_minimal_bundle_has_distinct_profile_and_exact_software(self):
        digest = self.build()
        data = B.verify_maintenance(self.root, digest)
        self.assertEqual(data["schema"], "lattice.windows-project-purge-maintenance.v1")
        self.assertEqual(data["profile"], "offline-project-purge")
        self.assertEqual(len(data["files"]), len(B.PROJECT_PURGE_FILES) + 19)
        self.assertFalse(any((self.root / name).exists() for name in ("postgres", "python", "git", "graphify", "platform")))
        self.assertEqual(data["project_purge"]["runtime_sha256"], B.sha(self.root / "bin/latticed.exe"))

    def test_cli_build_and_verify_maintenance_use_the_distinct_profile(self):
        output = self.base / "cli-output"
        arguments = ["lattice-bundle.py", "build-maintenance", "--bundle", str(output)]
        for name, value in self.arguments.items():
            option = "runtime-sha256" if name == "runtime_sha" else name.replace("_", "-")
            arguments.extend(["--" + option, str(value)])
        with patch.object(B.sys, "argv", arguments), patch.object(B.sys, "stdout", io.StringIO()) as stream:
            B.main()
            built = B.json.loads(stream.getvalue())
        self.assertEqual(built["status"], "LOCAL_MAINTENANCE_BUNDLE_VERIFIED")
        with patch.object(B.sys, "argv", ["lattice-bundle.py", "verify-maintenance", "--bundle",
                str(output), "--sha256", built["sha256"]]), patch.object(B.sys, "stdout", io.StringIO()) as stream:
            B.main()
            self.assertEqual(B.json.loads(stream.getvalue())["status"], "LOCAL_MAINTENANCE_BUNDLE_VERIFIED")
        with patch.object(B.sys, "argv", arguments + ["--postgres", str(self.base)]), \
                self.assertRaisesRegex(B.M.Rejected, "MAINTENANCE_OPTION_REJECTED"):
            B.main()

    def test_missing_or_mismatched_supply_rejected_before_creating_output(self):
        for overrides, error in (({"node": None}, "MAINTENANCE_SUPPLY_REQUIRED"),
                                 ({"runtime_sha": "a" * 64}, "RUNTIME_DIGEST_MISMATCH"),
                                 ({"project_purge_sha256": "a" * 64}, "PROJECT_PURGE_BINARY_DIGEST_REJECTED")):
            output = self.base / "invalid-supply"
            with self.subTest(overrides=overrides), self.assertRaisesRegex(B.M.Rejected, error):
                B.build_maintenance(output, **{**self.arguments, **overrides})
            self.assertFalse(output.exists())

    def test_packaged_maintenance_cli_loads_help_without_database(self):
        node = shutil.which("node")
        if node is None:
            self.skipTest("Node executable unavailable")
        self.build()
        result = subprocess.run([node, str(self.root / B.PROJECT_PURGE_ENTRYPOINT), "--help"],
            cwd=self.root, capture_output=True, text=True, encoding="utf-8", timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("preview", result.stdout)

    def test_manifest_and_payload_changes_reject_original_digest(self):
        digest = self.build()
        self.manifest["profile"] = "full-install"
        self.save()
        with self.assertRaisesRegex(B.M.Rejected, "MANIFEST_DIGEST_REJECTED"):
            B.verify_maintenance(self.root, digest)
        with self.assertRaisesRegex(B.M.Rejected, "MAINTENANCE_MANIFEST_REJECTED"):
            B.verify_maintenance(self.root, self.save())
        digest = self.build()
        (self.root / B.PROJECT_PURGE_ENTRYPOINT).write_bytes(b"changed")
        with self.assertRaisesRegex(B.M.Rejected, "BUNDLE_CONTENT_CHANGED"):
            B.verify_maintenance(self.root, digest)

    def test_recomputed_inventory_cannot_add_or_omit_files(self):
        for missing in (False, True):
            with self.subTest(missing=missing):
                self.build()
                if missing:
                    (self.root / B.PROJECT_PURGE_FILES[-1]).unlink()
                else:
                    (self.root / "unexpected.dll").write_bytes(b"extra")
                with self.assertRaisesRegex(B.M.Rejected, "MAINTENANCE_FILE_SET_REJECTED"):
                    B.verify_maintenance(self.root, self.save(inventory=True))

    def test_runtime_binding_cannot_be_replaced_by_recomputed_inventory(self):
        self.build()
        (self.root / "bin/latticed.exe").write_bytes(b"different-runtime")
        self.manifest["runtime_sha256"] = B.sha(self.root / "bin/latticed.exe")
        with self.assertRaisesRegex(B.M.Rejected, "PROJECT_PURGE_BUNDLE_REJECTED"):
            B.verify_maintenance(self.root, self.save(inventory=True))

    def test_vendor_bytes_remain_pinned_after_manifest_recomputation(self):
        for name, error in (("node/node.exe", "NODE_SUPPLY_REJECTED"),
                            ("bin/vcruntime140.dll", "VC_RUNTIME_SOURCE_REJECTED")):
            with self.subTest(name=name):
                self.build()
                (self.root / name).write_bytes(b"replacement")
                with self.assertRaisesRegex(B.M.Rejected, error):
                    B.verify_maintenance(self.root, self.save(inventory=True))

    def test_full_verify_and_install_reject_maintenance_before_preparing_state(self):
        digest = self.build()
        with self.assertRaisesRegex(B.M.Rejected, "BUNDLE_MANIFEST_REJECTED"):
            B.verify(self.root, digest)
        with patch.object(B.M, "prepare") as prepare:
            with self.assertRaisesRegex(B.M.Rejected, "BUNDLE_MANIFEST_REJECTED"):
                B.install(self.root, digest, self.base / "state", self.base / "source", self.base / "wsl.exe")
            prepare.assert_not_called()
        self.assertFalse((self.base / "state").exists())

    def test_fresh_absolute_nonoverlapping_output_required_before_copy(self):
        for output, error in ((Path("relative"), "ABSOLUTE_PATH_REQUIRED"),
                              (self.base, "FRESH_CUSTOMER_DIRECTORY_REQUIRED"),
                              (self.arguments["node"] / "output", "BUNDLE_SOURCE_OUTPUT_OVERLAP")):
            with self.subTest(output=str(output)), self.assertRaisesRegex(B.M.Rejected, error):
                B.build_maintenance(output, **self.arguments)
        output = self.base / "too-large"
        with patch.object(B, "MAX_BYTES", 1), self.assertRaisesRegex(B.M.Rejected, "CAPACITY_REJECTED"):
            B.build_maintenance(output, **self.arguments)
        self.assertFalse(output.exists())

    def test_alias_supply_or_output_parent_rejected(self):
        link = self.base / "alias"
        try:
            link.symlink_to(self.arguments["node"], target_is_directory=True)
        except OSError:
            self.skipTest("Windows symbolic-link privilege unavailable")
        with self.assertRaisesRegex(B.M.Rejected, "PATH_REDIRECTION_REJECTED"):
            B.build_maintenance(self.base / "output", **{**self.arguments, "node": link})
        with self.assertRaisesRegex(B.M.Rejected, "PATH_REDIRECTION_REJECTED"):
            B.build_maintenance(link / "output", **self.arguments)
        self.assertFalse((self.arguments["node"] / "output").exists())


if __name__ == "__main__":
    unittest.main()
