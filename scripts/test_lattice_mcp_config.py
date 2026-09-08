import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("lattice-mcp-config.py")
SPEC = importlib.util.spec_from_file_location("lattice_mcp_config", SCRIPT)
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


class ConfigTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="lattice-customer-config-")
        self.root = Path(self.temporary.name)
        self.config = self.root / "config.toml"
        self.runtime = self.root / "runtime one.exe"
        self.runtime.write_bytes(b"synthetic artifact, never launched")
        self.runtime2 = self.root / "runtime two.exe"
        self.runtime2.write_bytes(b"synthetic revision two, never launched")
        self.sha = M.digest(self.runtime.read_bytes())

    def tearDown(self):
        if self.config.exists():
            os.chmod(self.config, 0o600)
        self.temporary.cleanup()

    def install(self):
        return M.change(self.config, "install", self.runtime, self.sha)

    def test_clean_customer_lifecycle_across_process_restart(self):
        def run(operation, runtime=None):
            args = [sys.executable, str(SCRIPT), operation, "--config", str(self.config)]
            if runtime:
                args += ["--runtime", str(runtime), "--sha256", M.digest(runtime.read_bytes())]
            result = subprocess.run(args, capture_output=True, text=True, check=True)
            return json.loads(result.stdout)
        self.assertEqual(run("install", self.runtime)["runtime_workflow"], "NOT_VERIFIED")
        self.assertTrue(run("diagnose")["managed"])
        run("update", self.runtime2)
        self.assertEqual(M.parse(self.config.read_bytes())["mcp_servers"]["lattice"]["command"], str(self.runtime2))
        run("rollback")
        self.assertEqual(M.parse(self.config.read_bytes())["mcp_servers"]["lattice"]["command"], str(self.runtime))
        run("remove")
        self.assertEqual(self.config.read_bytes(), b"")
        self.assertFalse(run("diagnose")["managed"])

    def test_custom_settings_and_data_preserved_with_later_edit(self):
        before = b'\xef\xbb\xbf# keep exact\r\nmodel = "custom"\r\napproval_policy = "untrusted"\r\nsandbox_mode = "read-only"\r\n[mcp_servers.other]\r\ncommand = "other-runtime"'
        self.config.write_bytes(before)
        for name in ("skills", "memories", "customer-data"):
            (self.root / name).mkdir()
            (self.root / name / "keep").write_bytes(b"untouched")
        self.install()
        installed = self.config.read_bytes()
        self.assertTrue(installed.startswith(before))
        edited = installed.replace(b'model = "custom"', b'model = "new-custom"') + b'\n[profiles.customer]\nsandbox_mode = "read-only"\n'
        self.config.write_bytes(edited)
        M.change(self.config, "update", self.runtime2, M.digest(self.runtime2.read_bytes()))
        M.change(self.config, "rollback")
        M.change(self.config, "remove")
        result = self.config.read_bytes()
        self.assertTrue(result.startswith(before.replace(b'"custom"', b'"new-custom"')))
        parsed = M.parse(result)
        self.assertEqual(parsed["sandbox_mode"], "read-only")
        self.assertEqual(parsed["approval_policy"], "untrusted")
        self.assertEqual(parsed["mcp_servers"]["other"]["command"], "other-runtime")
        self.assertEqual(parsed["profiles"]["customer"]["sandbox_mode"], "read-only")
        for name in ("skills", "memories", "customer-data"):
            self.assertEqual((self.root / name / "keep").read_bytes(), b"untouched")

    def test_remove_restores_exact_original_without_final_newline(self):
        for original in (b'key = "value"', b'key = "value"\r\n'):
            with self.subTest(original=original):
                # Each filename owns its own sidecar.
                self.config = self.root / ("config" + str(len(original)) + ".toml")
                self.config.write_bytes(original)
                self.install()
                M.change(self.config, "remove")
                self.assertEqual(self.config.read_bytes(), original)

    def test_existing_quoted_or_inline_lattice_is_never_adopted(self):
        for original in (b'[mcp_servers."lattice"]\ncommand="existing"\n', b'mcp_servers = { lattice = { command = "existing" } }\n'):
            self.config.write_bytes(original)
            with self.assertRaisesRegex(M.Rejected, "LATTICE_CONFIG_CONFLICT"):
                self.install()
            self.assertEqual(self.config.read_bytes(), original)

    def test_read_only_config_is_diagnosed_without_chmod(self):
        self.config.write_bytes(b'model="customer"\n')
        os.chmod(self.config, 0o444)
        before = self.config.stat().st_mode
        with self.assertRaisesRegex(M.Rejected, "CONFIG_READ_ONLY"):
            self.install()
        self.assertEqual(self.config.stat().st_mode, before)
        self.assertEqual(self.config.read_bytes(), b'model="customer"\n')

    def test_changed_managed_block_blocks_update_and_remove(self):
        self.install()
        before = self.config.read_bytes().replace(b'command = ', b'command=')
        self.config.write_bytes(before)
        for op in ("remove", "rollback", "update"):
            with self.assertRaises(M.Rejected):
                M.change(self.config, op, self.runtime2, M.digest(self.runtime2.read_bytes()))
            self.assertEqual(self.config.read_bytes(), before)

    def test_failed_candidate_or_invalid_toml_never_changes_config(self):
        self.config.write_bytes(b'model="keep"\n')
        before = self.config.read_bytes()
        with self.assertRaisesRegex(M.Rejected, "RUNTIME_DIGEST_MISMATCH"):
            M.change(self.config, "install", self.runtime, "0" * 64)
        self.assertEqual(self.config.read_bytes(), before)
        self.config.write_bytes(b'model="unterminated')
        with self.assertRaisesRegex(M.Rejected, "CONFIG_TOML_INVALID"):
            self.install()
        self.assertEqual(self.config.read_bytes(), b'model="unterminated')

    def test_install_is_idempotent_and_preserves_backups(self):
        self.config.write_bytes(b'model="keep"\n')
        self.install()
        folder, _, _ = M.locations(self.config)
        backups = list(folder.glob("*.bak"))
        self.assertEqual(backups[0].read_bytes(), b'model="keep"\n')
        self.assertEqual(self.install()["status"], "UNCHANGED")
        self.assertEqual(list(folder.glob("*.bak")), backups)

    def test_crash_after_config_write_can_recover_without_rewriting_config(self):
        original_write = M.atomic_write
        _, state, pending = M.locations(self.config)
        def fail_state(path, data):
            if path == state:
                raise OSError("synthetic interruption")
            original_write(path, data)
        with patch.object(M, "atomic_write", side_effect=fail_state):
            with self.assertRaises(OSError):
                self.install()
        before_recovery = self.config.read_bytes()
        self.assertTrue(pending.exists())
        self.assertEqual(M.change(self.config, "recover")["status"], "RECOVERED_APPLIED")
        self.assertEqual(self.config.read_bytes(), before_recovery)
        self.assertTrue(M.diagnose(self.config)["managed"])

    def test_pending_operation_rejects_new_work_and_concurrent_edit(self):
        self.config.write_bytes(b'model="first"\n')
        original_write = M.atomic_write
        _, _, pending = M.locations(self.config)
        def concurrent_edit(path, data):
            original_write(path, data)
            if path == pending:
                self.config.write_bytes(b'model="new-customer-value"\n')
        with patch.object(M, "atomic_write", side_effect=concurrent_edit):
            with self.assertRaisesRegex(M.Rejected, "CONFIG_CHANGED_DURING_OPERATION"):
                self.install()
        with self.assertRaisesRegex(M.Rejected, "RECOVERY_REQUIRED"):
            self.install()
        with self.assertRaisesRegex(M.Rejected, "RECOVERY_CONFIG_CHANGED"):
            M.change(self.config, "recover")
        self.assertEqual(self.config.read_bytes(), b'model="new-customer-value"\n')

    def test_failed_config_replace_recovers_not_applied(self):
        self.config.write_bytes(b'model="keep"\n')
        with patch.object(M, "write_config", side_effect=PermissionError("synthetic denied write")):
            with self.assertRaises(PermissionError):
                self.install()
        self.assertEqual(M.change(self.config, "recover")["status"], "RECOVERED_NOT_APPLIED")
        self.assertEqual(self.config.read_bytes(), b'model="keep"\n')

    def test_windows_exclusive_write_blocks_external_writer(self):
        self.config.write_bytes(b'model="keep"\n')
        with M.exclusive_config(self.config, True):
            with self.assertRaises(PermissionError):
                self.config.write_bytes(b'model="racing"\n')
            with self.assertRaises(PermissionError):
                os.replace(self.runtime2, self.config)
        self.assertEqual(self.config.read_bytes(), b'model="keep"\n')

    def test_remove_rejects_user_keys_appended_to_lattice_table(self):
        self.install()
        before = self.config.read_bytes() + b'startup_timeout_sec=60\n'
        self.config.write_bytes(before)
        with self.assertRaisesRegex(M.Rejected, "MANAGED_TABLE_CHANGED"):
            M.change(self.config, "remove")
        self.assertEqual(self.config.read_bytes(), before)

    def test_update_interruption_diagnoses_pending_before_old_state(self):
        self.install()
        write = M.atomic_write
        _, state, _ = M.locations(self.config)
        def interrupted(path, data):
            if path == state:
                raise OSError("synthetic interruption")
            write(path, data)
        with patch.object(M, "atomic_write", side_effect=interrupted):
            with self.assertRaises(OSError):
                M.change(self.config, "update", self.runtime2, M.digest(self.runtime2.read_bytes()))
        self.assertEqual(M.diagnose(self.config)["status"], "RECOVERY_REQUIRED")
        self.assertEqual(M.diagnose(self.config)["config_match"], "after")
        self.assertEqual(M.change(self.config, "recover")["status"], "RECOVERED_APPLIED")

    def test_rollback_rejects_missing_or_changed_previous_runtime(self):
        self.install()
        M.change(self.config, "update", self.runtime2, M.digest(self.runtime2.read_bytes()))
        before = self.config.read_bytes()
        self.runtime.write_bytes(b"changed")
        with self.assertRaisesRegex(M.Rejected, "RUNTIME_DIGEST_MISMATCH"):
            M.change(self.config, "rollback")
        self.runtime.unlink()
        with self.assertRaisesRegex(M.Rejected, "RUNTIME_MISSING"):
            M.change(self.config, "rollback")
        self.assertEqual(self.config.read_bytes(), before)

    def test_symlink_config_rejected_without_following(self):
        target = self.root / "private.toml"
        target.write_bytes(b'model="private"\n')
        try:
            self.config.symlink_to(target)
        except OSError:
            self.skipTest("Symlink creation unavailable to this process")
        with self.assertRaisesRegex(M.Rejected, "PATH_REDIRECTION_REJECTED"):
            self.install()
        self.assertEqual(target.read_bytes(), b'model="private"\n')

    def test_diagnose_is_read_only_and_does_not_print_config(self):
        self.config.write_bytes(b'customer_secret="synthetic-sensitive-value"\n')
        result = subprocess.run([sys.executable, str(SCRIPT), "diagnose", "--config", str(self.config)], capture_output=True, text=True, check=True)
        self.assertNotIn("synthetic-sensitive-value", result.stdout + result.stderr)
        self.assertEqual(set(p.name for p in self.root.iterdir()), {"config.toml", "runtime one.exe", "runtime two.exe"})

    @unittest.skipUnless(os.name == "nt", "Windows DACL integration")
    def test_windows_protected_access_preserved_on_config_and_backup(self):
        self.config.write_bytes(b'model="customer"\n')
        command = '''
$p = $env:LATTICE_SYNTHETIC_CONFIG
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = [System.Security.AccessControl.FileSecurity]::new()
$acl.SetAccessRuleProtection($true, $false)
$acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'Allow'))
[System.IO.File]::SetAccessControl($p, $acl)
[System.IO.File]::GetAccessControl($p).GetSecurityDescriptorSddlForm([System.Security.AccessControl.AccessControlSections]::Access)
'''
        environment = dict(os.environ, LATTICE_SYNTHETIC_CONFIG=str(self.config))
        before = subprocess.check_output(["powershell.exe", "-NoProfile", "-Command", command], env=environment, text=True).strip()
        self.install()
        folder, _, _ = M.locations(self.config)
        for file in (self.config, next(folder.glob("*.bak"))):
            environment["LATTICE_SYNTHETIC_CONFIG"] = str(file)
            after = subprocess.check_output(["powershell.exe", "-NoProfile", "-Command", '[System.IO.File]::GetAccessControl($env:LATTICE_SYNTHETIC_CONFIG).GetSecurityDescriptorSddlForm([System.Security.AccessControl.AccessControlSections]::Access)'], env=environment, text=True).strip()
            self.assertEqual(after, before)

    @unittest.skipUnless(os.name == "nt", "Windows DACL integration")
    def test_windows_denied_write_preserves_customer_file_and_acl(self):
        self.config.write_bytes(b'model="restricted"\n')
        command = '''
$p = $env:LATTICE_SYNTHETIC_CONFIG
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = [System.IO.File]::GetAccessControl($p)
$acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($sid, 'WriteData', 'Deny'))
[System.IO.File]::SetAccessControl($p, $acl)
[System.IO.File]::GetAccessControl($p).GetSecurityDescriptorSddlForm([System.Security.AccessControl.AccessControlSections]::Access)
'''
        environment = dict(os.environ, LATTICE_SYNTHETIC_CONFIG=str(self.config))
        before_acl = subprocess.check_output(["powershell.exe", "-NoProfile", "-Command", command], env=environment, text=True).strip()
        with self.assertRaises(PermissionError):
            self.install()
        self.assertEqual(self.config.read_bytes(), b'model="restricted"\n')
        after_acl = subprocess.check_output(["powershell.exe", "-NoProfile", "-Command", '[System.IO.File]::GetAccessControl($env:LATTICE_SYNTHETIC_CONFIG).GetSecurityDescriptorSddlForm([System.Security.AccessControl.AccessControlSections]::Access)'], env=environment, text=True).strip()
        self.assertEqual(after_acl, before_acl)


if __name__ == "__main__":
    unittest.main()
