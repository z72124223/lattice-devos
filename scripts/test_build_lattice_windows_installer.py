import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

SPEC = importlib.util.spec_from_file_location("build_sfx", Path(__file__).with_name("build-lattice-windows-installer.py"))
B = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(B)


class BuilderTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lattice-sfx-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.package = self.root / "package"
        (self.package / "bundle/python").mkdir(parents=True)
        (self.package / "bundle/python/python.exe").write_bytes(b"python fixture")
        (self.package / "Install-LATTICE.cmd").write_bytes(b"exit /b 7\r\n")
        self.sevenzip = self.root / "7z.exe"
        self.sevenzip.write_bytes(b"7z fixture")
        self.module = self.root / "official.sfx"
        self.module.write_bytes(b"SFX fixture")
        self.output = self.root / "setup.exe"
        self.real_sha = B.sha
        self.calls = []
        patch.object(B, "sha", side_effect=lambda path: B.MODULE_SHA256 if path == self.module else self.real_sha(path)).start()
        patch.object(B, "run", side_effect=self.fake_run).start()
        patch.object(B, "prepare_stub", side_effect=lambda source, destination: destination.write_bytes(source.read_bytes())).start()
        self.addCleanup(patch.stopall)

    def fake_run(self, args, *, cwd):
        self.calls.append((args, cwd))
        if args[1] == "a":
            archive = Path(args[-2])
            with archive.open("ab") as stream:
                stream.write(b"mock compressed package")

    def build(self):
        return B.build(self.package, self.output, self.sevenzip, self.module)

    def test_build_hashes_full_composition_and_tests_archive_and_executable(self):
        result = self.build()
        expected = self.module.read_bytes() + B.sfx_config() + self.output.with_suffix(".7z").read_bytes()
        self.assertEqual(self.output.read_bytes(), expected)
        self.assertEqual(result["sha256"], self.real_sha(self.output))
        self.assertEqual([call[0][1] for call in self.calls], ["a", "a", "t", "t"])
        self.assertEqual(self.calls[-1][0][-1], str(self.output))
        self.assertFalse(result["installed"])
        self.assertFalse(result["signed"])
        self.assertEqual(result["requested_execution_level"], "asInvoker")
        self.assertEqual(result["sfx_exit_code"], "DOES_NOT_PROPAGATE_SETUP_EXIT_CODE")
        self.assertEqual(json.loads(self.output.with_suffix(".build.json").read_text())["sha256"], result["sha256"])

    def test_runprogram_uses_exact_bundled_python_and_utf8(self):
        config = B.sfx_config().decode("utf-8")
        self.assertIn("LATTICE 安裝程式", config)
        self.assertIn("%%T\\\\bundle\\\\python\\\\python.exe", config)
        self.assertIn("-I -B -S", config)
        self.assertNotIn("cmd.exe", config)
        self.assertNotIn("ExecuteFile", config)

    def test_previous_outputs_are_not_overwritten(self):
        self.output.write_bytes(b"preserve existing installer")
        with self.assertRaisesRegex(B.Rejected, "FRESH_BUILD_OUTPUT_REQUIRED"):
            self.build()
        self.assertEqual(self.output.read_bytes(), b"preserve existing installer")
        self.assertEqual(self.calls, [])

    def test_wrong_module_is_rejected_before_compressing(self):
        with patch.object(B, "sha", side_effect=self.real_sha):
            with self.assertRaisesRegex(B.Rejected, "OFFICIAL_SFX_MODULE_DIGEST_REJECTED"):
                self.build()
        self.assertEqual(self.calls, [])

    def test_source_changes_reject_executable_publication(self):
        def mutate(args, *, cwd):
            self.fake_run(args, cwd=cwd)
            if args[1] == "t":
                (self.package / "Install-LATTICE.cmd").write_bytes(b"changed")
        with patch.object(B, "run", side_effect=mutate):
            with self.assertRaisesRegex(B.Rejected, "PACKAGE_CHANGED_DURING_BUILD"):
                self.build()
        self.assertFalse(self.output.exists())

    def test_archive_failure_preserves_evidence_without_executable(self):
        with patch.object(B, "run", side_effect=B.Rejected("SEVENZIP_OPERATION_FAILED_2")):
            with self.assertRaises(B.Rejected):
                self.build()
        self.assertTrue(self.output.with_suffix(".config.txt").exists())
        self.assertFalse(self.output.exists())

    def test_output_inside_source_is_rejected(self):
        self.output = self.package / "setup.exe"
        with self.assertRaisesRegex(B.Rejected, "PACKAGE_OUTPUT_OVERLAP"):
            self.build()


@unittest.skipUnless(os.name == "nt", "Windows bootstrap")
class BootstrapTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("sfx_launch", Path(__file__).with_name("lattice-sfx-launch.py"))
        self.launcher = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.launcher)
        self.temp = tempfile.TemporaryDirectory(prefix="lattice-sfx-start-")
        self.addCleanup(self.temp.cleanup)
        self.result_path = Path(self.temp.name) / "LATTICE/sfx-last-result.json"
        self.result_path.parent.mkdir()
        self.result_path.write_text(json.dumps({"status": "SETUP_EXITED", "setup_exit_code": 0,
                                               "started_at": 1, "finished_at": 2, "run_id": "old"}))
        env = patch.dict(os.environ, {"LOCALAPPDATA": self.temp.name})
        env.start()
        self.addCleanup(env.stop)

    def test_full_cmd_failure_and_reboot_codes_are_retained(self):
        spec = importlib.util.spec_from_file_location("sfx_launch", Path(__file__).with_name("lattice-sfx-launch.py"))
        launcher = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(launcher)
        with tempfile.TemporaryDirectory(prefix="lattice-sfx-result-") as directory:
            with patch.dict(os.environ, {"LOCALAPPDATA": directory}):
                for code in (7, 3, 0):
                    with patch.object(launcher.subprocess, "run", return_value=SimpleNamespace(returncode=code)) as run:
                        self.assertEqual(launcher.main(), code)
                    result = json.loads((Path(directory) / "LATTICE/sfx-last-result.json").read_text())
                    self.assertEqual(result["setup_exit_code"], code)
                    self.assertEqual(result["reboot_requested"], code == 3)
                    self.assertGreaterEqual(result["finished_at"], result["started_at"])
                    self.assertEqual(result["status"], "SETUP_EXITED")
                    self.assertIn('cmd.exe" /d /s /c ""', run.call_args.args[0])
                    self.assertNotIn("installed", result)

    def test_starting_receipt_replaces_previous_success_before_cmd_runs(self):
        def start(*args, **kwargs):
            result = json.loads(self.result_path.read_text())
            self.assertEqual(result["status"], "SETUP_STARTING")
            self.assertIsNone(result["setup_exit_code"])
            self.assertIsNone(result["finished_at"])
            self.assertNotEqual(result["run_id"], "old")
            return SimpleNamespace(returncode=0)
        with patch.object(self.launcher.subprocess, "run", side_effect=start):
            self.assertEqual(self.launcher.main(), 0)

    def test_cmd_start_error_writes_fresh_failure_without_exception_details(self):
        with patch.object(self.launcher.subprocess, "run", side_effect=OSError("private path and stderr")):
            self.assertEqual(self.launcher.main(), 2)
        result = json.loads(self.result_path.read_text())
        self.assertEqual(result["status"], "SETUP_FAILED")
        self.assertEqual(result["code"], "SETUP_COULD_NOT_START")
        self.assertEqual(result["setup_exit_code"], 2)
        self.assertGreater(result["started_at"], 2)
        self.assertGreaterEqual(result["finished_at"], result["started_at"])
        self.assertNotIn("private path", self.result_path.read_text())

    def test_system_directory_error_writes_failure_without_starting_cmd(self):
        with patch.object(self.launcher.ctypes.windll.kernel32, "GetSystemDirectoryW", return_value=0):
            with patch.object(self.launcher.subprocess, "run") as run:
                self.assertEqual(self.launcher.main(), 2)
        run.assert_not_called()
        result = json.loads(self.result_path.read_text())
        self.assertEqual(result["code"], "WINDOWS_SYSTEM_DIRECTORY_UNAVAILABLE")
        self.assertEqual(result["status"], "SETUP_FAILED")
        self.assertEqual(result["setup_exit_code"], 2)
        self.assertGreater(result["finished_at"], 2)


if __name__ == "__main__":
    unittest.main()
