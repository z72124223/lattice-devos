#!/usr/bin/env python3
"""Check or request official WSL host setup; never manage customer distributions.

Microsoft requirements and flags:
https://learn.microsoft.com/windows/wsl/install
https://learn.microsoft.com/windows/wsl/basic-commands
READY covers host prerequisites only. The platform importer must still verify its
own WSL2 distribution and execute the Graphify acceptance afterwards.
"""
from __future__ import annotations

import argparse
import base64
import ctypes
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

SCHEMA = "lattice.wsl-host.v1"
MINIMUM_BUILD = 19041


def report(status: str, code: str, message: str, **extra) -> dict:
    return {"schema": SCHEMA, "status": status, "code": code, "message": message, **extra}


def system_directory() -> Path:
    # Use the Windows API, not a caller-controlled SystemRoot or PATH.
    buffer = ctypes.create_unicode_buffer(32768)
    length = ctypes.windll.kernel32.GetSystemDirectoryW(buffer, len(buffer))
    if not 0 < length < len(buffer):
        raise OSError("Windows system directory unavailable")
    return Path(buffer.value)


def invoke(arguments: list[str], *, timeout: int = 60) -> subprocess.CompletedProcess:
    return subprocess.run(arguments, capture_output=True, stdin=subprocess.DEVNULL, timeout=timeout,
                          creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)


def powershell(code: str, *, timeout: int = 60) -> dict:
    encoded = base64.b64encode(code.encode("utf-16-le")).decode("ascii")
    result = invoke([str(system_directory() / "WindowsPowerShell/v1.0/powershell.exe"),
                     "-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded], timeout=timeout)
    if result.returncode:
        raise OSError("Windows host query failed")
    value = json.loads(result.stdout.decode("utf-8-sig"))
    if not isinstance(value, dict):
        raise ValueError("Windows host query returned no object")
    return value


def virtualization() -> dict:
    return powershell("""
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$machine = Get-CimInstance -ClassName Win32_ComputerSystem
$cpu = @(Get-CimInstance -ClassName Win32_Processor)
$vm = Get-CimInstance -ClassName Win32_OptionalFeature -Filter "Name='VirtualMachinePlatform'"
@{hypervisor_present = [bool]$machine.HypervisorPresent;
  firmware_enabled = [bool](@($cpu | Where-Object {$_.VirtualizationFirmwareEnabled}).Count);
  virtual_machine_platform_enabled = [bool]($vm.InstallState -eq 1)} | ConvertTo-Json -Compress
""")


def pending_reboot() -> bool:
    import winreg
    paths = (
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
    )
    for path in paths:
        try:
            with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, path):
                return True
        except FileNotFoundError:
            continue
    return False


def check(wsl: Path | None = None) -> dict:
    if os.name != "nt":
        return report("BLOCKED", "WINDOWS_REQUIRED", "這個安裝包需要 Windows。")
    if platform.machine().lower() not in ("amd64", "x86_64"):
        return report("BLOCKED", "WINDOWS_X64_REQUIRED", "這個安裝包需要 x64 電腦，尚不支援 ARM。")
    if sys.getwindowsversion().build < MINIMUM_BUILD:
        return report("BLOCKED", "WINDOWS_UPDATE_REQUIRED", "請先將 Windows 更新至 Windows 10 2004（19041）以上或 Windows 11。")
    launcher = system_directory() / "wsl.exe"
    if wsl is not None and wsl.resolve() != launcher.resolve():
        return report("BLOCKED", "SYSTEM_WSL_REQUIRED", "只允許使用 Windows 系統內的 WSL。")
    if not launcher.is_file():
        return report("BLOCKED", "WSL_LAUNCHER_MISSING", "Windows 缺少系統 WSL 程式，請先完成 Windows 更新，再重新執行安裝。")
    details = {"wsl": str(launcher), "scope": "HOST_PREREQUISITES_ONLY"}
    if pending_reboot():
        return report("REBOOT_REQUIRED", "WINDOWS_REBOOT_PENDING", "Windows 有尚未完成的更新，請儲存工作並重新開機，再雙擊安裝。", **details)
    features = virtualization()
    if features.get("hypervisor_present") is not True and features.get("firmware_enabled") is not True:
        return report("BLOCKED", "FIRMWARE_VIRTUALIZATION_REQUIRED", "電腦尚未提供虛擬化功能。請由電腦管理者確認 BIOS／虛擬機的虛擬化設定。", **details)
    result = invoke([str(launcher), "--status"])
    if (result.returncode == 0 and features.get("hypervisor_present") is True
            and features.get("virtual_machine_platform_enabled") is True):
        return report("READY", "WSL_HOST_READY", "WSL 主機檢查通過；接著安裝並驗證 LATTICE 專用環境。", **details)
    return report("BLOCKED", "WSL_SETUP_REQUIRED", "需要安裝或啟用 Microsoft 官方 WSL 功能。", **details)


def confirm_install() -> bool:
    message = ("LATTICE 需要 Microsoft 官方 WSL，才能執行 Graphify。\n\n"
               "按「是」將下載並啟用 WSL，Windows 接著可能要求管理員同意。"
               "此步驟不會移除既有 Linux 環境，也不會自動重新開機。\n\n"
               "是否繼續？")
    # YES/NO, information icon, NO selected by default; explicit even if elevated.
    return ctypes.windll.user32.MessageBoxW(None, message, "LATTICE：安裝必要的 Windows 功能", 0x00000144) == 6


def install_official_wsl() -> dict:
    # All elevated arguments are fixed; no user-controlled strings enter this code.
    # Do not pass /restart, install a distro, or change any default distro/version.
    return powershell("""
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$launcher = Join-Path ([Environment]::SystemDirectory) 'wsl.exe'
try {
  $child = Start-Process -FilePath $launcher -ArgumentList @('--install','--no-distribution','--web-download') -Verb RunAs -WindowStyle Hidden -Wait -PassThru
  @{exit_code = $child.ExitCode} | ConvertTo-Json -Compress
} catch {
  $native = $_.Exception.NativeErrorCode
  if (-not $native -and $_.Exception.InnerException) { $native = $_.Exception.InnerException.NativeErrorCode }
  @{error = $(if ($native -eq 1223) {'UAC_CANCELLED'} else {'WSL_INSTALL_LAUNCH_FAILED'})} | ConvertTo-Json -Compress
}
""", timeout=1800)


def ensure(wsl: Path | None = None) -> dict:
    before = check(wsl)
    if before["code"] != "WSL_SETUP_REQUIRED":
        return before
    if not confirm_install():
        return report("BLOCKED", "WSL_CONSENT_DECLINED", "您取消了 WSL 安裝。可再次雙擊 LATTICE 安裝包繼續。")
    installed = install_official_wsl()
    if installed.get("error"):
        return report("BLOCKED", installed["error"], "Windows 未允許啟動 WSL 安裝。請再次執行並確認管理員提示。")
    if installed.get("exit_code") == 3010:
        return report("REBOOT_REQUIRED", "WSL_REBOOT_REQUIRED", "必要功能已安裝。請儲存工作並重新開機，再雙擊 LATTICE 安裝包。")
    if installed.get("exit_code") != 0:
        return report("BLOCKED", "WSL_INSTALL_FAILED", "Microsoft WSL 安裝未成功。請確認網路與 Windows 更新後重試。", install_exit_code=installed.get("exit_code"))
    after = check(wsl)
    if after["code"] == "WSL_SETUP_REQUIRED":
        return report("BLOCKED", "WSL_SETUP_NOT_READY", "Microsoft 安裝程式已結束，但 WSL 尚未通過檢查。請確認 Windows 更新與虛擬化功能。")
    return after


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "ensure"))
    parser.add_argument("--wsl", type=Path)
    args = parser.parse_args(argv)
    try:
        result = ensure(args.wsl) if args.action == "ensure" else check(args.wsl)
    except subprocess.TimeoutExpired:
        result = report("BLOCKED", "WSL_OPERATION_TIMED_OUT", "WSL 檢查或安裝逾時。Windows 安裝可能仍在執行，請稍後再試。")
    except (OSError, ValueError, AttributeError):
        result = report("BLOCKED", "WSL_HOST_CHECK_FAILED", "無法完成 Windows 功能檢查。請確認系統可用後重新執行。")
    print(json.dumps(result, ensure_ascii=True))
    # Keep reboot separate from a failed operation for the double-click launcher.
    return {"READY": 0, "REBOOT_REQUIRED": 3, "BLOCKED": 2}[result["status"]]


if __name__ == "__main__":
    sys.exit(main())
