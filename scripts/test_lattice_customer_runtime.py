import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("lattice-customer-runtime.py")
SPEC = importlib.util.spec_from_file_location("customer_runtime", SCRIPT)
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


@unittest.skipUnless(os.name == "nt", "Windows customer Runtime")
class CustomerRuntimeTests(unittest.TestCase):
    def test_import_defaults_to_installed_hash_pinned_node(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); request = root / "request.json"; request.write_bytes(b"{}")
            node = root / "node.exe"; node.write_bytes(b"fixture-node")
            config = {"node": str(node), "files": {str(node): M.file_digest(node)}, "runtime": "fixture-runtime.exe"}
            calls = []
            def invoke(command, **kwargs):
                calls.append((command, kwargs))
                return type("Result", (), {"returncode": 0, "stdout": '{"status":"COMPLETED"}'})()
            with patch.object(M, "load", return_value=(config, "fixture-only")), patch.object(M, "verify_running"), patch.object(M, "environment", return_value={}), patch.object(M, "invoke", side_effect=invoke):
                self.assertEqual(M.import_result(root, request)["status"], "COMPLETED")
            self.assertEqual(calls[0][1]["env"]["LATTICE_LOCAL_RESULT_NODE_EXE"], str(node))

    def test_import_without_configured_node_does_not_search_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(M, "load", return_value=({}, "fixture-only")), patch.object(M, "invoke", side_effect=AssertionError("No process")):
                with self.assertRaisesRegex(M.Rejected, "NODE_NOT_CONFIGURED"):
                    M.import_result(root, root / "request.json")

    def test_dpapi_keeps_plaintext_out_and_rejects_tampering(self):
        secret = b"a-customer-only-secret-never-written-as-plaintext"
        sealed = M.dpapi(secret)
        self.assertNotIn(secret, sealed)
        self.assertEqual(M.dpapi(sealed, decrypt=True), secret)
        damaged = bytearray(sealed)
        damaged[-1] ^= 1
        with self.assertRaisesRegex(M.Rejected, "PROTECTION_REJECTED"):
            M.dpapi(bytes(damaged), decrypt=True)

    def test_customer_state_rejects_configuration_edits_before_starting_any_process(self):
        with tempfile.TemporaryDirectory(prefix="lattice-customer-identity-") as directory:
            root = Path(directory)
            original = M.CONFIG.json_bytes({"schema": M.SCHEMA, "root": str(root)})
            (root / "installation.json").write_bytes(original)
            (root / "credentials.dpapi").write_bytes(M.dpapi(M.CONFIG.json_bytes({
                "password": "test-only", "installation_sha256": M.CONFIG.digest(original)})))
            altered = json.loads(original)
            altered["postgres_bin"] = "C:/unrelated/installation"
            (root / "installation.json").write_bytes(M.CONFIG.json_bytes(altered))
            with patch.object(M, "invoke", side_effect=AssertionError("No process may start")):
                with self.assertRaisesRegex(M.Rejected, "CUSTOMER_INSTALLATION_CHANGED"):
                    M.load(root)

    def test_existing_directory_is_never_adopted_or_repermissioned(self):
        with tempfile.TemporaryDirectory(prefix="lattice-customer-existing-") as directory:
            root = Path(directory)
            original = root / "keep.txt"
            original.write_bytes(b"customer-owned-data")
            with patch.object(M, "invoke", side_effect=AssertionError("No ACL/process mutation")):
                with self.assertRaisesRegex(M.Rejected, "FRESH_CUSTOMER_DIRECTORY_REQUIRED"):
                    M.private_new_root(root)
            self.assertEqual(original.read_bytes(), b"customer-owned-data")

    def test_effective_external_listener_is_rejected_before_start(self):
        config = {"root": "C:/customer", "postgres_bin": "C:/PostgreSQL/bin", "port": 49152}
        calls = []
        def observe(command, *args, **kwargs):
            calls.append(command)
            return "*"
        with patch.object(M, "running", return_value=False), patch.object(M, "checked", side_effect=observe):
            with self.assertRaisesRegex(M.Rejected, "EFFECTIVE_POSTGRES_CONFIG_REJECTED"):
                M.start(config, "test-only")
        self.assertEqual(len(calls), 1)
        self.assertNotIn("pg_ctl.exe", calls[0][0])

    def test_postgres_override_file_tampering_rejected_before_process(self):
        with tempfile.TemporaryDirectory(prefix="lattice-customer-override-") as directory:
            root = Path(directory)
            (root / "cluster").mkdir()
            override = root / "cluster" / "postgresql.auto.conf"
            override.write_bytes(b"# empty override\n")
            config = {"schema": M.SCHEMA, "root": str(root), "port": 49152, "run_id": "a" * 32,
                      "files": {str(override): M.file_digest(override)}}
            public = M.CONFIG.json_bytes(config)
            (root / "installation.json").write_bytes(public)
            (root / "credentials.dpapi").write_bytes(M.dpapi(M.CONFIG.json_bytes({
                "password": "test-only", "installation_sha256": M.CONFIG.digest(public)})))
            override.write_bytes(b"listen_addresses='*'\n")
            with patch.object(M, "invoke", side_effect=AssertionError("No process may start")):
                with self.assertRaisesRegex(M.Rejected, "CUSTOMER_COMPONENT_CHANGED"):
                    M.load(root)

    def test_closed_environment_excludes_author_secrets_and_provider_configuration(self):
        poisoned = {"LATTICE_TASK019_PASSWORD": "author", "PGPASSWORD": "author-db",
                    "PGSERVICE": "author", "CODEX_HOME": "C:/author", "OPENAI_API_KEY": "secret",
                    "PYTHONPATH": "C:/injection", "LATTICE_MANAGED_FOREMAN_MODE": "ACTIVE"}
        with patch.dict(os.environ, poisoned):
            child = M.closed_environment()
        self.assertTrue(set(poisoned).isdisjoint(child))

    def test_invalid_project_name_preserves_existing_catalog_without_starting_process(self):
        with tempfile.TemporaryDirectory(prefix="lattice-customer-project-name-") as directory:
            root = Path(directory)
            catalog = root / "projects.json"
            original = b'{"schema":"lattice.customer-project-catalog.v1","projects":[]}'
            catalog.write_bytes(original)
            with patch.object(M, "invoke", side_effect=AssertionError("No process may start")):
                for name in (" Demo", "Demo ", "Demo\x7f", "Demo\x85"):
                    with self.subTest(name=name), self.assertRaisesRegex(M.Rejected, "PROJECT_NAME_REJECTED"):
                        M.register_project(root, root, name)
            self.assertEqual(catalog.read_bytes(), original)

    def test_project_registration_never_creates_a_lock_in_an_unowned_directory(self):
        with tempfile.TemporaryDirectory(prefix="lattice-customer-unowned-") as directory:
            root = Path(directory)
            with self.assertRaises(M.Rejected):
                M.register_project(root, root, "Customer")
            self.assertFalse((root / ".operations").exists())

    def test_customer_runtime_has_distinct_persistent_authority_and_no_managed_execution(self):
        config = {"root": "C:/customer", "run_id": "a" * 32, "system_id": "12345",
                  "port": 49152, "git": "C:/Git/git.exe"}
        first = M.environment(config, "customer-only-password")
        second = M.environment({**config, "run_id": "b" * 32}, "other-password")
        self.assertNotEqual(first["LATTICE_STORE_AUTHORITY_HEAD_DIGEST"], second["LATTICE_STORE_AUTHORITY_HEAD_DIGEST"])
        self.assertEqual(first, M.environment(config, "customer-only-password"))
        self.assertEqual(first["LATTICE_TASK_INGRESS_KIND"], "CODEX_LOCAL_MCP")
        self.assertEqual(first["LATTICE_MANAGED_FOREMAN_MODE"], "DISABLED")
        self.assertNotIn("CODEX_HOME", first)

    def test_result_import_rejects_changed_node_before_any_process(self):
        with tempfile.TemporaryDirectory(prefix="lattice-customer-result-") as directory:
            root = Path(directory)
            request, node = root / "request.json", root / "node.exe"
            request.write_bytes(b"{}")
            node.write_bytes(b"changed test dependency")
            with patch.object(M, "load", return_value=({}, "test-only")), patch.object(M, "invoke", side_effect=AssertionError("No process may start")):
                with self.assertRaisesRegex(M.Rejected, "NODE_DIGEST_MISMATCH"):
                    M.import_result(root, request, node, "0" * 64)


if __name__ == "__main__":
    unittest.main()
