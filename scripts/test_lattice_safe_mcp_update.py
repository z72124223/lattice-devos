import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


MODULE_PATH = Path(__file__).with_name("lattice-safe-mcp-update.py")
SPEC = importlib.util.spec_from_file_location("lattice_safe_mcp_update", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class SafeMcpUpdateTests(unittest.TestCase):
    def write_config(self, root: Path, command: str) -> Path:
        config = root / "config.toml"
        config.write_text(
            f'[mcp_servers.lattice]\ncommand = "{command}"\n\n[mcp_servers.lattice.env]\n',
            encoding="utf-8",
        )
        return config

    def test_active_task_lock_is_detected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            locks = root / "thread-writer-locks"
            locks.mkdir()
            (locks / ".coordination.lock").touch()
            (locks / "active.lock").touch()
            self.assertEqual(MODULE.active_locks(root), ["active.lock"])

    def test_failed_candidate_does_not_change_config(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = self.write_config(root, r"C:\\old\\latticed.exe")
            before = config.read_bytes()
            with self.assertRaises(RuntimeError):
                MODULE.verify_candidate(root / "missing.exe")
            self.assertEqual(config.read_bytes(), before)

    def test_verified_candidate_replaces_only_lattice_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = self.write_config(root, r"C:\\old\\latticed.exe")
            candidate = root / "candidate.exe"
            candidate.write_bytes(b"candidate")
            MODULE.replace_command(config, candidate)
            text = config.read_text(encoding="utf-8")
            self.assertIn(str(candidate).replace("\\", "\\\\"), text)
            self.assertIn("[mcp_servers.lattice.env]", text)

    def test_saved_config_can_be_atomically_restored_after_activation_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = self.write_config(root, r"C:\\old\\latticed.exe")
            original = config.read_text(encoding="utf-8")
            candidate = root / "candidate.exe"
            candidate.write_bytes(b"candidate")
            saved = MODULE.replace_command(config, candidate)
            self.assertNotEqual(config.read_text(encoding="utf-8"), original)
            MODULE.atomic_write(config, saved)
            self.assertEqual(config.read_text(encoding="utf-8"), original)

    def test_verifier_requires_the_fail_closed_marker(self):
        candidate = Path("candidate.exe")
        with patch.object(MODULE.subprocess, "run") as run:
            run.return_value.returncode = 1
            run.return_value.stdout = ""
            run.return_value.stderr = "LATTICED_ARGUMENTS_REJECTED\n"
            MODULE.verify_candidate(candidate)
        with patch.object(MODULE.subprocess, "run") as run:
            run.return_value.returncode = 0
            run.return_value.stdout = "LATTICED_ARGUMENTS_REJECTED\n"
            run.return_value.stderr = ""
            with self.assertRaises(RuntimeError):
                MODULE.verify_candidate(candidate)


if __name__ == "__main__":
    unittest.main()
