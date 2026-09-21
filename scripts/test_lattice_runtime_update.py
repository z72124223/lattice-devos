import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("update", Path(__file__).with_name("lattice-runtime-update.py"))
U = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(U)
M = U.M


class RuntimeUpdateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lattice-update-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "owned"
        self.root.mkdir()
        (self.root / "cluster").mkdir()
        old = self.root / "old.exe"
        old.write_bytes(b"old-runtime-test-only")
        self.new = Path(self.temp.name) / "new.exe"
        self.new.write_bytes(b"new-runtime-test-only")
        config = {"schema": M.SCHEMA, "root": str(self.root), "runtime": str(old),
                  "run_id": "a" * 32, "port": 49151, "system_id": "12345678",
                  "postgres_bin": str(self.root), "files": {str(old): M.file_digest(old)}}
        public = M.CONFIG.json_bytes(config)
        (self.root / "installation.json").write_bytes(public)
        (self.root / "credentials.dpapi").write_bytes(M.dpapi(M.CONFIG.json_bytes({
            "password": "fixture-only", "installation_sha256": M.CONFIG.digest(public)})))
        self.before = U.pair(self.root)
        for obj, name, kwargs in [(M, "checked", {"return_value": "Database system identifier: 12345678"}),
                                  (M, "verify_running", {"return_value": None}),
                                  (M, "start", {"return_value": None}),
                                  (M, "runtime_action", {"return_value": None}),
                                  (U, "probe", {"return_value": {"status": "NOT_STARTED"}})]:
            patcher = patch.object(obj, name, **kwargs)
            patcher.start()
            self.addCleanup(patcher.stop)

    def update(self):
        return U.apply(self.root, self.new, M.file_digest(self.new))

    def test_standalone_binary_update_retains_sealed_app_local_crt(self):
        config, password = M.load(self.root)
        payloads = {"vcruntime140.dll": b"crt-fixture", "msvcp140.dll": b"cpp-fixture"}
        for name, payload in payloads.items(): (self.root / name).write_bytes(payload)
        pinned = {name: M.file_digest(self.root / name) for name in payloads}
        config["files"].update({str(self.root / name): digest for name, digest in pinned.items()})
        public = M.CONFIG.json_bytes(config)
        (self.root / "installation.json").write_bytes(public)
        (self.root / "credentials.dpapi").write_bytes(M.dpapi(M.CONFIG.json_bytes({
            "password": password, "installation_sha256": M.CONFIG.digest(public)})))
        with patch.dict(M.VC_RUNTIME_FILES, pinned, clear=True):
            self.update()
        updated, _ = M.load(self.root)
        destination = Path(updated["runtime"]).parent
        for name, digest in pinned.items():
            self.assertEqual(updated["files"][str(destination / name)], digest)
            self.assertEqual((destination / name).read_bytes(), payloads[name])
        (destination / "vcruntime140.dll").write_bytes(b"tampered")
        with self.assertRaisesRegex(M.Rejected, "COMPONENT_CHANGED"):
            M.load(self.root)

    def interrupt_update(self):
        atomic = M.CONFIG.atomic_write
        def interrupted(path, data):
            if path == self.root / "credentials.dpapi":
                raise OSError("simulated power interruption")
            return atomic(path, data)
        with patch.object(M.CONFIG, "atomic_write", side_effect=interrupted):
            with self.assertRaisesRegex(OSError, "simulated"):
                self.update()

    def test_update_preserves_old_files_and_compatible_rollback_restores_exact_pair(self):
        result = self.update()
        self.assertEqual(result["status"], "RUNTIME_UPDATE_VERIFIED")
        config, password = M.load(self.root)
        self.assertEqual(password, "fixture-only")
        self.assertEqual(Path(config["runtime"]).read_bytes(), self.new.read_bytes())
        self.assertEqual((self.root / "old.exe").read_bytes(), b"old-runtime-test-only")
        self.assertTrue(Path(config["launcher"]).is_file())
        restored = U.apply(self.root, rollback=result["update_id"])
        self.assertEqual(restored["status"], "RUNTIME_ROLLBACK_VERIFIED")
        self.assertEqual(U.pair(self.root), self.before)
        self.assertTrue(Path(config["runtime"]).is_file())

    def test_incompatible_rollback_preserves_current_version_and_data(self):
        result = self.update()
        after = U.pair(self.root)
        with patch.object(U, "probe", side_effect=M.Rejected("UPDATE_RUNTIME_COMPATIBILITY_REJECTED")):
            with self.assertRaisesRegex(M.Rejected, "COMPATIBILITY"):
                U.apply(self.root, rollback=result["update_id"])
        self.assertEqual(U.pair(self.root), after)
        self.assertFalse((self.root / U.PENDING).exists())

    def test_interrupted_pair_fails_closed_and_explicit_recovery_finishes(self):
        self.interrupt_update()
        with self.assertRaisesRegex(M.Rejected, "UPDATE_RECOVERY_REQUIRED"):
            M.load(self.root)
        result = U.apply(self.root, recover=True)
        self.assertEqual(result["status"], "RUNTIME_UPDATE_VERIFIED")
        self.assertFalse((self.root / U.PENDING).exists())
        self.assertEqual(M.load(self.root)[1], "fixture-only")

    def test_recovery_never_overwrites_unrecognized_active_edits(self):
        self.interrupt_update()
        path = self.root / "installation.json"
        path.write_bytes(b"unrelated edit after interruption")
        with self.assertRaisesRegex(M.Rejected, "CONCURRENT_CHANGE_PRESERVED"):
            U.apply(self.root, recover=True)
        self.assertEqual(path.read_bytes(), b"unrelated edit after interruption")

    def test_recovery_rejects_changed_staged_executable(self):
        self.interrupt_update()
        journal, after, _ = U.journal(self.root)
        Path(after["runtime"]).write_bytes(b"corrupt")
        before = U.pair(self.root)
        with self.assertRaisesRegex(M.Rejected, "UPDATE_COMPONENT_CHANGED"):
            U.apply(self.root, recover=True)
        self.assertEqual(U.pair(self.root), before)

    def test_bad_digest_rejected_before_creating_version_directory(self):
        with self.assertRaisesRegex(M.Rejected, "RUNTIME_DIGEST_MISMATCH"):
            U.apply(self.root, self.new, "f" * 64)
        self.assertEqual(U.pair(self.root), self.before)
        self.assertFalse((self.root / "updates").exists())

    def test_foreign_journal_rejected_before_recovery_lock_or_writes(self):
        self.interrupt_update()
        other = Path(self.temp.name) / "foreign"
        other.mkdir()
        (other / U.PENDING).write_bytes((self.root / U.PENDING).read_bytes())
        with self.assertRaisesRegex(M.Rejected, "JOURNAL_REJECTED"):
            U.apply(other, recover=True)
        self.assertFalse((other / ".operations").exists())

    def test_changed_rollback_head_is_rejected(self):
        first = self.update()
        self.update()
        after = U.pair(self.root)
        with self.assertRaisesRegex(M.Rejected, "ROLLBACK_HEAD_CHANGED"):
            U.apply(self.root, rollback=first["update_id"])
        self.assertEqual(U.pair(self.root), after)

    def test_shared_connections_allow_normal_operations_but_block_update(self):
        with M.runtime_lease(self.root), M.runtime_lease(self.root):
            with M.CONFIG.manager_lock(self.root / ".operations"):
                M.load(self.root)
            with self.assertRaisesRegex(M.Rejected, "RUNTIME_IN_USE"):
                self.update()
        self.update()

    def test_recovery_starts_owned_cluster_after_interruption(self):
        self.interrupt_update()
        with patch.object(M, "start", return_value=None) as start:
            U.apply(self.root, recover=True)
        self.assertEqual(start.call_count, 1)
        self.assertEqual(start.call_args.args[0]["system_id"], "12345678")

    def test_interrupted_history_creation_can_recover_without_overwrite(self):
        with patch.object(U.os, "rename", side_effect=OSError("simulated rename interruption")):
            with self.assertRaisesRegex(OSError, "simulated"):
                self.update()
        self.assertTrue((self.root / U.PENDING).is_file())
        self.assertTrue(list((self.root / "updates").glob("*.tmp-*")))
        result = U.apply(self.root, recover=True)
        self.assertEqual(result["status"], "RUNTIME_UPDATE_VERIFIED")

    def test_interrupted_pending_removal_replays_complete_history(self):
        unlink = Path.unlink
        def interrupted(path, *args, **kwargs):
            if path == self.root / U.PENDING:
                raise OSError("simulated removal interruption")
            return unlink(path, *args, **kwargs)
        with patch.object(Path, "unlink", interrupted):
            with self.assertRaisesRegex(OSError, "simulated"):
                self.update()
        U.apply(self.root, recover=True)
        self.assertFalse((self.root / U.PENDING).exists())

    def test_added_dependency_module_blocks_load_and_recovery_before_process(self):
        bundle = Path(self.temp.name) / "bundle"
        bundle.mkdir()
        software = bundle / "python.exe"
        software.write_bytes(b"fixture-python")
        config, password = M.load(self.root)
        config["dependency_root"] = str(bundle)
        config["files"][str(software)] = M.file_digest(software)
        public = M.CONFIG.json_bytes(config)
        (self.root / "installation.json").write_bytes(public)
        (self.root / "credentials.dpapi").write_bytes(M.dpapi(M.CONFIG.json_bytes({
            "password": password, "installation_sha256": M.CONFIG.digest(public)})))
        added = bundle / "sitecustomize.py"
        added.write_text("unlisted")
        with patch.object(M, "checked", side_effect=AssertionError("No process may start")):
            with self.assertRaisesRegex(M.Rejected, "DEPENDENCY_FILE_SET_CHANGED"):
                M.load(self.root)
        added.unlink()
        self.interrupt_update()
        added.write_text("unlisted after interruption")
        with patch.object(M, "checked", side_effect=AssertionError("No process may start")):
            with self.assertRaisesRegex(M.Rejected, "DEPENDENCY_FILE_SET_CHANGED"):
                U.apply(self.root, recover=True)


if __name__ == "__main__":
    unittest.main()
