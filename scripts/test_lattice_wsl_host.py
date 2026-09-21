import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("wsl_host", Path(__file__).with_name("lattice-wsl-host.py"))
H = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(H)


class HostTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lattice-wsl-host-test-")
        self.addCleanup(self.temp.cleanup)
        self.system = Path(self.temp.name)
        self.wsl = self.system / "wsl.exe"
        self.wsl.write_bytes(b"mock system launcher")
        for name, value in (
            ("system_directory", self.system),
            ("pending_reboot", False),
            ("virtualization", {"hypervisor_present": True, "firmware_enabled": False,
                                "virtual_machine_platform_enabled": True}),
            ("invoke", subprocess.CompletedProcess([], 0, b"mock status", b"")),
            ("confirm_install", False),
            ("install_official_wsl", {"exit_code": 0}),
        ):
            self.addCleanup(patch.stopall)
            setattr(self, name, patch.object(H, name, return_value=value).start())
        patch.object(H.os, "name", "nt").start()
        patch.object(H.platform, "machine", return_value="AMD64").start()
        patch.object(H.sys, "getwindowsversion", return_value=SimpleNamespace(build=22631), create=True).start()

    def needs_setup(self):
        self.invoke.return_value.returncode = 1

    def test_ready_does_not_prompt_or_install(self):
        self.assertEqual(H.ensure()["status"], "READY")
        self.confirm_install.assert_not_called()
        self.install_official_wsl.assert_not_called()
        self.invoke.assert_called_once_with([str(self.wsl), "--status"])

    def test_check_never_installs(self):
        self.needs_setup()
        self.assertEqual(H.check()["code"], "WSL_SETUP_REQUIRED")
        self.confirm_install.assert_not_called()
        self.install_official_wsl.assert_not_called()

    def test_rejects_old_windows_and_arm_before_running_commands(self):
        with patch.object(H.sys, "getwindowsversion", return_value=SimpleNamespace(build=18363)):
            self.assertEqual(H.ensure()["code"], "WINDOWS_UPDATE_REQUIRED")
        with patch.object(H.platform, "machine", return_value="ARM64"):
            self.assertEqual(H.ensure()["code"], "WINDOWS_X64_REQUIRED")
        self.invoke.assert_not_called()
        self.confirm_install.assert_not_called()

    def test_missing_or_custom_launcher_is_preserved_and_blocked(self):
        self.assertEqual(H.ensure(self.system / "external.exe")["code"], "SYSTEM_WSL_REQUIRED")
        self.wsl.unlink()
        self.assertEqual(H.ensure()["code"], "WSL_LAUNCHER_MISSING")
        self.install_official_wsl.assert_not_called()

    def test_pending_reboot_does_not_run_installer_again(self):
        self.pending_reboot.return_value = True
        self.assertEqual(H.ensure()["status"], "REBOOT_REQUIRED")
        self.invoke.assert_not_called()
        self.install_official_wsl.assert_not_called()

    def test_no_firmware_virtualization_requires_owner_action(self):
        self.virtualization.return_value = {"hypervisor_present": False, "firmware_enabled": False}
        self.assertEqual(H.ensure()["code"], "FIRMWARE_VIRTUALIZATION_REQUIRED")
        self.install_official_wsl.assert_not_called()

    def test_wsl_status_alone_does_not_prove_hypervisor_ready(self):
        self.virtualization.return_value = {"hypervisor_present": False, "firmware_enabled": True}
        self.assertEqual(H.check()["code"], "WSL_SETUP_REQUIRED")

    def test_disabled_wsl2_platform_is_not_ready_even_when_other_hypervisor_runs(self):
        self.virtualization.return_value["virtual_machine_platform_enabled"] = False
        self.assertEqual(H.check()["code"], "WSL_SETUP_REQUIRED")

    def test_declined_consent_never_installs(self):
        self.needs_setup()
        self.assertEqual(H.ensure()["code"], "WSL_CONSENT_DECLINED")
        self.install_official_wsl.assert_not_called()

    def test_uac_cancellation_is_visible(self):
        self.needs_setup()
        self.confirm_install.return_value = True
        self.install_official_wsl.return_value = {"error": "UAC_CANCELLED"}
        self.assertEqual(H.ensure()["code"], "UAC_CANCELLED")

    def test_installer_reboot_code_requires_next_double_click(self):
        self.needs_setup()
        self.confirm_install.return_value = True
        self.install_official_wsl.return_value = {"exit_code": 3010}
        self.assertEqual(H.ensure()["status"], "REBOOT_REQUIRED")

    def test_success_requires_fresh_readback(self):
        self.confirm_install.return_value = True
        self.invoke.side_effect = [subprocess.CompletedProcess([], 1, b"", b""),
                                   subprocess.CompletedProcess([], 0, b"", b"")]
        self.assertEqual(H.ensure()["status"], "READY")
        self.assertEqual(self.invoke.call_count, 2)

    def test_zero_exit_with_unready_host_is_not_success(self):
        self.needs_setup()
        self.confirm_install.return_value = True
        self.assertEqual(H.ensure()["code"], "WSL_SETUP_NOT_READY")

    def test_install_failure_is_not_reported_as_reboot_or_ready(self):
        self.needs_setup()
        self.confirm_install.return_value = True
        self.install_official_wsl.return_value = {"exit_code": 1}
        self.assertEqual(H.ensure()["code"], "WSL_INSTALL_FAILED")

    def test_main_reboot_exit_code_and_timeout_json(self):
        self.pending_reboot.return_value = True
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(H.main(["ensure"]), 3)
        self.assertEqual(json.loads(output.getvalue())["status"], "REBOOT_REQUIRED")
        self.pending_reboot.return_value = False
        self.invoke.side_effect = subprocess.TimeoutExpired("wsl", 60)
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(H.main(["check"]), 2)
        self.assertEqual(json.loads(output.getvalue())["code"], "WSL_OPERATION_TIMED_OUT")


class ElevationContractTests(unittest.TestCase):
    def test_checks_cannot_accept_a_console_install_prompt(self):
        with patch.object(H.subprocess, "run") as execute:
            H.invoke(["wsl.exe", "--status"])
        self.assertEqual(execute.call_args.kwargs["stdin"], subprocess.DEVNULL)

    def test_fixed_official_command_never_manages_distros_or_restarts(self):
        # Restore the real function: the result is mocked at the PowerShell edge.
        with patch.object(H, "powershell", return_value={"exit_code": 0}) as execute:
            H.install_official_wsl()
        command = execute.call_args.args[0]
        self.assertIn("'--install','--no-distribution','--web-download'", command)
        self.assertIn("-Verb RunAs", command)
        self.assertIn("-WindowStyle Hidden", command)
        for forbidden in ("--unregister", "--terminate", "--shutdown", "Restart-Computer", "Set-ExecutionPolicy"):
            self.assertNotIn(forbidden, command)


if __name__ == "__main__":
    unittest.main()
