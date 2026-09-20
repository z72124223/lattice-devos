import importlib.util
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager, redirect_stdout
import io
import json
import os
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("lattice-customer-runtime.py")
SPEC = importlib.util.spec_from_file_location("customer_runtime", SCRIPT)
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


@unittest.skipUnless(os.name == "nt", "Windows customer Runtime")
class CustomerRuntimeTests(unittest.TestCase):
    def test_app_local_crt_copy_is_complete_pinned_and_sealable(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); source = root / "source"; source.mkdir()
            target = root / "target"; target.mkdir()
            # Old installations with no CRT group retain their previous behavior.
            self.assertEqual(M.copy_vc_runtime(source, target), {})
            payloads = {"vcruntime140.dll": b"crt", "msvcp140.dll": b"cpp"}
            for name, payload in payloads.items(): (source / name).write_bytes(payload)
            pinned = {name: M.file_digest(source / name) for name in payloads}
            trusted = {str(source / name): digest for name, digest in pinned.items()}
            with patch.dict(M.VC_RUNTIME_FILES, pinned, clear=True):
                with self.assertRaisesRegex(M.Rejected, "VC_RUNTIME_SOURCE_REJECTED"):
                    M.copy_vc_runtime(source, target, trusted_files={})
                self.assertEqual(list(target.iterdir()), [])
                copied = M.copy_vc_runtime(source, target, trusted_files=trusted)
                self.assertEqual(copied, {str(target / name): digest for name, digest in pinned.items()})
                self.assertTrue(all(M.file_digest(Path(path)) == digest for path, digest in copied.items()))
                with self.assertRaisesRegex(M.Rejected, "TARGET_EXISTS"):
                    M.copy_vc_runtime(source, target, trusted_files=trusted)

    def test_partial_crt_group_is_rejected_before_any_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); source = root / "source"; source.mkdir()
            target = root / "target"; target.mkdir()
            (source / "vcruntime140.dll").write_bytes(b"crt")
            pinned = {"vcruntime140.dll": M.file_digest(source / "vcruntime140.dll"), "msvcp140.dll": "0" * 64}
            with patch.dict(M.VC_RUNTIME_FILES, pinned, clear=True), self.assertRaises(M.Rejected):
                M.copy_vc_runtime(source, target)
            self.assertEqual(list(target.iterdir()), [])

    def test_debug_or_unknown_crt_cannot_be_treated_as_an_old_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            for name in ("vcruntime140d.dll", "msvcp999.dll"):
                with self.subTest(name=name):
                    path = source / name; path.write_bytes(b"unapproved")
                    with self.assertRaisesRegex(M.Rejected, "VC_RUNTIME_SET_REJECTED"):
                        M.vc_runtime_files(source)
                    path.unlink()

    def graph_project_fixture(self, root):
        source = root / "another project"; source.mkdir()
        # Match register_project(): Windows temp roots can use a short-path alias.
        source = source.resolve()
        project_id = "12345678-1234-1234-1234-123456789abc"
        schema = "lattice.customer-project-catalog.v1"
        project = {"schema_version": schema, "record_kind": "CUSTOMER_LOCAL_LOCATOR", "id": project_id,
                   "control_project_id": project_id, "name": "Other", "canonical_path": str(source),
                   "registry_authority": "NONE", "registry_project_id": None}
        catalog = {"schema": schema, "projects": [project]}
        (root / "projects.json").write_text(json.dumps(catalog), encoding="utf-8")
        config = {**self.serve_fixture(root), "graph_source": str(root / "sample"), "graphify_runtime": "pinned",
                  "retained_graph_configuration": "a" * 64, "retained_graph_configurations": ["b" * 64]}
        return config, catalog, project_id, source

    def test_registration_canonicalizes_path_alias_before_project_refresh(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); config, catalog, _, source = self.graph_project_fixture(root)
            alias = source / ".." / source.name
            self.assertNotEqual(alias, alias.resolve())
            catalog["projects"] = []
            (root / "projects.json").write_text(json.dumps(catalog), encoding="utf-8")
            config["git"] = "fixture-git.exe"
            with patch.object(M, "load", return_value=(config, "fixture-only")), \
                    patch.object(M, "checked", side_effect=[str(source), "refs/heads/main", "a" * 40]):
                registered = M.register_project(root, alias, "Customer")
            saved = json.loads((root / "projects.json").read_bytes())["projects"][0]
            self.assertEqual(saved["canonical_path"], str(source))
            selected = M.project_graph_config(config, registered["project_id"])
            self.assertEqual(selected["graph_source"], str(source))

    def test_project_refresh_derives_source_without_mutating_sealed_default_or_catalog(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); config, _, project_id, source = self.graph_project_fixture(root)
            original = json.dumps(config, sort_keys=True)
            catalog = (root / "projects.json").read_bytes()
            with patch.object(M, "load", return_value=(config, "fixture-only")), patch.object(M, "verify_running"), \
                    patch.object(M, "running", return_value=True), patch.object(M, "runtime_action", return_value={"status": "PERSISTED"}) as action:
                result = M.operate(root, "graphify-refresh", project_id=project_id)
            self.assertEqual(result["operation_evidence"]["status"], "PERSISTED")
            selected, _, flag, selected_id = action.call_args.args
            self.assertEqual((selected["graph_source"], flag, selected_id), (str(source), "--graphify-refresh-project", project_id))
            self.assertNotIn("retained_graph_configuration", selected)
            self.assertNotIn("retained_graph_configurations", selected)
            self.assertEqual(json.dumps(config, sort_keys=True), original)
            self.assertEqual((root / "projects.json").read_bytes(), catalog)

    def test_project_selector_rejects_unknown_duplicate_tampered_and_relative_locators(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); config, catalog, project_id, _ = self.graph_project_fixture(root)
            with self.assertRaisesRegex(M.Rejected, "ID_REJECTED"):
                M.project_graph_config(config, "../foreign")
            with self.assertRaisesRegex(M.Rejected, "LOCATOR_NOT_UNIQUE"):
                M.project_graph_config(config, "00000000-1234-1234-1234-123456789abc")
            catalog["projects"].append(dict(catalog["projects"][0]))
            (root / "projects.json").write_text(json.dumps(catalog), encoding="utf-8")
            with self.assertRaisesRegex(M.Rejected, "LOCATOR_NOT_UNIQUE"):
                M.project_graph_config(config, project_id)
            catalog["projects"].pop()
            catalog["projects"][0]["registry_authority"] = "FORGED"
            (root / "projects.json").write_text(json.dumps(catalog), encoding="utf-8")
            with self.assertRaisesRegex(M.Rejected, "CATALOG_REJECTED"):
                M.project_graph_config(config, project_id)
            catalog["projects"][0]["registry_authority"] = "NONE"
            catalog["projects"][0]["canonical_path"] = "relative-path"
            (root / "projects.json").write_text(json.dumps(catalog), encoding="utf-8")
            with self.assertRaisesRegex(M.Rejected, "SOURCE_REJECTED"):
                M.project_graph_config(config, project_id)

    def test_default_project_retains_its_own_backup_selectors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); config, _, project_id, source = self.graph_project_fixture(root)
            config["graph_source"] = str(source)
            self.assertEqual(M.project_graph_config(config, project_id), config)

    def test_native_missing_registration_is_reported_without_fallback_refresh(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); config, _, project_id, _ = self.graph_project_fixture(root)
            rejected = type("Result", (), {"returncode": 2, "stderr": "PROJECT_IS_NOT_REGISTERED\n", "stdout": ""})()
            with patch.object(M, "load", return_value=(config, "fixture-only")), patch.object(M, "verify_running"), \
                    patch.object(M, "environment", return_value={}), patch.object(M, "invoke", return_value=rejected) as invoke:
                with self.assertRaisesRegex(M.Rejected, "PROJECT_IS_NOT_REGISTERED"):
                    M.operate(root, "graphify-refresh", project_id=project_id)
            self.assertEqual(invoke.call_count, 1)
            self.assertEqual(invoke.call_args.args[0][-2:], ["--graphify-refresh-project", project_id])

    def test_project_selector_cannot_silently_apply_to_another_action(self):
        with patch.object(M, "load", side_effect=AssertionError("No configuration access")):
            with self.assertRaisesRegex(M.Rejected, "SELECTOR_ACTION_REJECTED"):
                M.operate(Path("C:/unused"), "serve", project_id="unused")

    def serve_fixture(self, root):
        config = {"root": str(root), "runtime": str(root / "latticed.exe"), "run_id": "a" * 32,
                  "system_id": "123456", "postgres_bin": str(root / "postgres/bin"), "port": 49152}
        (root / "ready.json").write_text(json.dumps({"schema": M.SCHEMA, "run_id": config["run_id"], "system_id": config["system_id"]}))
        return config

    def test_mcp_serve_starts_only_prepared_cluster_and_keeps_stdout_for_native_server(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); config = self.serve_fixture(root)
            with patch.object(M, "load", return_value=(config, "test-only")), patch.object(M, "running", return_value=False), \
                    patch.object(M, "start") as start, patch.object(M, "environment", return_value={}), \
                    patch.object(M, "runtime_action", side_effect=AssertionError("No initialization or bootstrap")), \
                    patch.object(M.subprocess, "call", return_value=0) as native, redirect_stdout(io.StringIO()) as output:
                self.assertEqual(M.operate(root, "serve"), 0)
            start.assert_called_once_with(config, "test-only")
            self.assertEqual(native.call_args.args[0], [config["runtime"]])
            self.assertEqual(output.getvalue(), "")

    def test_mcp_serve_never_initializes_missing_or_foreign_ready_state(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); config = self.serve_fixture(root)
            for contents, code in ((None, "INITIALIZATION_INCOMPLETE"), ("{}", "INITIALIZATION_IDENTITY_REJECTED")):
                if contents is None:
                    (root / "ready.json").unlink()
                else:
                    (root / "ready.json").write_text(contents)
                with self.subTest(contents=contents), patch.object(M, "load", return_value=(config, "test-only")), \
                        patch.object(M, "start_for_mcp", side_effect=AssertionError("No start")), \
                        patch.object(M.subprocess, "call", side_effect=AssertionError("No native server")), self.assertRaisesRegex(M.Rejected, code):
                    M.operate(root, "serve")

    def test_mcp_serve_does_not_start_when_sealed_installation_fails_validation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(M, "load", side_effect=M.Rejected("CUSTOMER_CLUSTER_IDENTITY_REJECTED")), \
                    patch.object(M, "start_for_mcp", side_effect=AssertionError("No start")), \
                    patch.object(M.subprocess, "call", side_effect=AssertionError("No native server")), \
                    self.assertRaisesRegex(M.Rejected, "CLUSTER_IDENTITY_REJECTED"):
                M.operate(root, "serve")

    def test_mcp_autostart_rejects_non_loopback_effective_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); config = self.serve_fixture(root)
            with patch.object(M, "load", return_value=(config, "test-only")), patch.object(M, "running", return_value=False), \
                    patch.object(M, "checked", return_value="*") as observed, \
                    patch.object(M.subprocess, "call", side_effect=AssertionError("No native server")), \
                    self.assertRaisesRegex(M.Rejected, "EFFECTIVE_POSTGRES_CONFIG_REJECTED"):
                M.operate(root, "serve")
            self.assertEqual(observed.call_count, 1)
            self.assertEqual(observed.call_args.args[0][-2:], ["-C", "listen_addresses"])

    def test_mcp_serve_rejects_other_running_cluster_without_starting_or_adopting(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); config = self.serve_fixture(root)
            observation = {"id": "different", "data": str(root / "cluster"), "port": config["port"], "listen": "127.0.0.1"}
            with patch.object(M, "load", return_value=(config, "test-only")), patch.object(M, "running", return_value=True), \
                    patch.object(M, "checked", return_value=json.dumps(observation)), \
                    patch.object(M, "start_for_mcp", side_effect=AssertionError("No start")), \
                    patch.object(M.subprocess, "call", side_effect=AssertionError("No native server")), \
                    self.assertRaisesRegex(M.Rejected, "CLUSTER_IDENTITY_REJECTED"):
                M.operate(root, "serve")

    def test_concurrent_mcp_connections_start_owned_cluster_once(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); config = self.serve_fixture(root)
            started, release, contention = threading.Event(), threading.Event(), threading.Event()
            live = {"running": False, "starts": 0}
            lock = M.CONFIG.manager_lock

            @contextmanager
            def observed_lock(folder):
                try:
                    with lock(folder):
                        yield
                except M.Rejected as error:
                    if str(error) == "CONFIG_MANAGER_BUSY":
                        contention.set()
                    raise

            def checked(command, *args, **kwargs):
                if command[-1] == "start":
                    live["starts"] += 1
                    started.set()
                    if not release.wait(5):
                        raise AssertionError("test start was not released")
                    live["running"] = True
                    return ""
                return {"listen_addresses": "127.0.0.1", "port": str(config["port"]), "data_directory": str(root / "cluster")}[command[-1]]

            with patch.object(M, "load", return_value=(config, "test-only")), \
                    patch.object(M, "running", side_effect=lambda _: live["running"]), \
                    patch.object(M, "checked", side_effect=checked), patch.object(M, "verify_running"), \
                    patch.object(M, "environment", return_value={}), patch.object(M.CONFIG, "manager_lock", side_effect=observed_lock), \
                    patch.object(M, "runtime_action", side_effect=AssertionError("No initialization or bootstrap")), \
                    patch.object(M.subprocess, "call", return_value=0), ThreadPoolExecutor(max_workers=2) as pool:
                first = pool.submit(M.operate, root, "serve")
                try:
                    self.assertTrue(started.wait(5))
                    second = pool.submit(M.operate, root, "serve")
                    self.assertTrue(contention.wait(5))
                finally:
                    release.set()
                self.assertEqual(first.result(timeout=5), 0)
                self.assertEqual(second.result(timeout=5), 0)
            self.assertEqual(live["starts"], 1)

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
