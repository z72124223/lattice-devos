import hashlib
import importlib.util
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

SPEC = importlib.util.spec_from_file_location("crt_check", Path(__file__).with_name("verify-lattice-loaded-crt.py"))
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


class LoadedCrtTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.exe = self.root / "latticed.exe"
        self.exe.write_bytes(b"test executable")
        self.dll = self.root / "vcruntime140.dll"
        self.dll.write_bytes(b"test dll")
        self.pin = hashlib.sha256(self.dll.read_bytes()).hexdigest()
        self.observed = {"pid": 123, "executable": str(self.exe), "modules": [
            {"name": "VCRUNTIME140.dll", "path": str(self.dll)}]}

    def validate(self):
        with patch.dict(M.PINS, {"vcruntime140.dll": self.pin}):
            return M.validate_modules(self.observed, self.exe, ("vcruntime140.dll",))

    def test_live_observation_path_and_pin_required(self):
        result = self.validate()
        self.assertEqual(result["modules"][0]["source"], "EXECUTABLE_DIRECTORY")
        self.assertEqual(result["modules"][0]["sha256"], self.pin)

    def test_disk_presence_does_not_replace_loaded_observation(self):
        self.observed["modules"] = []
        with self.assertRaisesRegex(M.Rejected, "CRT_MODULE_NOT_LOADED"):
            self.validate()

    def test_system32_module_rejected_even_with_matching_basename(self):
        self.observed["modules"][0]["path"] = str(self.root / "System32/vcruntime140.dll")
        with self.assertRaisesRegex(M.Rejected, "CRT_MODULE_NOT_APP_LOCAL"):
            self.validate()

    def test_loaded_file_hash_mismatch_rejected(self):
        self.dll.write_bytes(b"changed")
        with self.assertRaisesRegex(M.Rejected, "CRT_LOADED_FILE_HASH_MISMATCH"):
            self.validate()

    def test_other_process_image_rejected(self):
        self.observed["executable"] = str(self.root / "other.exe")
        with self.assertRaisesRegex(M.Rejected, "CRT_PROCESS_IMAGE_MISMATCH"):
            self.validate()

    def test_duplicate_modules_rejected(self):
        self.observed["modules"] *= 2
        with self.assertRaisesRegex(M.Rejected, "CRT_MODULE_NOT_LOADED"):
            self.validate()

    def test_normal_eof_stops_no_other_process(self):
        process = Mock(returncode=0)
        with patch.object(M, "powershell") as query:
            self.assertTrue(M.close_owned_mcp(process, {}))
        process.stdin.close.assert_called_once()
        process.terminate.assert_not_called()
        query.assert_not_called()

    def test_mcp_reply_parses_only_matching_json_response(self):
        process = Mock(stdin=io.StringIO(), stdout=io.StringIO('{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25"}}\n'))
        self.assertEqual(M.reply(process, {"method": "initialize"}, 1)["protocolVersion"], "2025-11-25")

    def test_bad_mcp_json_has_only_fixed_error_code(self):
        process = Mock(stdin=io.StringIO(), stdout=io.StringIO('sensitive arbitrary stderr text\n'))
        with self.assertRaisesRegex(M.Rejected, '^CRT_MCP_RESPONSE_REJECTED$'):
            M.reply(process, {}, 1)

    def test_mcp_wait_is_bounded(self):
        process = Mock(stdin=io.StringIO())
        with patch.object(M.threading, "Thread"):
            with self.assertRaisesRegex(M.Rejected, '^CRT_MCP_RESPONSE_TIMEOUT$'):
                M.reply(process, {}, 1, timeout=0.01)

    def test_timeout_cleanup_targets_owned_native_not_postgres(self):
        process = Mock(pid=456)
        process.wait.side_effect = [subprocess.TimeoutExpired("owned", 10), 0]
        process.poll.return_value = None
        with patch.object(M, "powershell", return_value={}) as query:
            self.assertFalse(M.close_owned_mcp(process, {"LATTICE_CRT_NATIVE_EXE": str(self.exe)}))
        script = query.call_args.args[0]
        self.assertIn("ParentProcessId = 456", script)
        self.assertIn("latticed.exe", script)
        self.assertIn("ExecutablePath -eq $env:LATTICE_CRT_NATIVE_EXE", script)
        self.assertNotIn("postgres", script)
        process.terminate.assert_called_once()


if __name__ == "__main__":
    unittest.main()
