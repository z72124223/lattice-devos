#!/usr/bin/env python3
"""Build an unsigned self-extracting candidate with the official LZMA SDK module.

The official SFX waits for setup but discards its exit code. Read the setup report
and sfx-last-result.json; a zero SFX process exit never proves LATTICE installed.
"""
from __future__ import annotations
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys

SDK_URL = "https://github.com/ip7z/7zip/releases/download/26.03/lzma2603.7z"
SDK_SHA256 = "86c213f752520ab5325c310f50bef63ec344b56dd1c80b0246d06dc6cec953b2"
MODULE_SHA256 = "92194891840ce85cc528f44fd066a4acd708c75723e0721c10cb9850679c9ab9"
AS_INVOKER_MANIFEST = b'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<assemblyIdentity type="win32" name="LATTICE.Installer" version="2.0.0.0" processorArchitecture="*"/>
<trustInfo xmlns="urn:schemas-microsoft-com:asm.v3"><security><requestedPrivileges>
<requestedExecutionLevel level="asInvoker" uiAccess="false"/>
</requestedPrivileges></security></trustInfo></assembly>'''
LICENSE = """LZMA SDK 26.03
LZMA SDK is written and placed in the public domain by Igor Pavlov.
Anyone is free to copy, modify, publish, use, compile, sell, or distribute the
original LZMA SDK code, either in source code form or as a compiled binary, for
any purpose, commercial or non-commercial, and by any means.
Source: https://www.7-zip.org/sdk.html and LZMA SDK DOC/lzma-sdk.txt
"""
README = """LATTICE 自解壓安裝程式
雙擊後會解壓縮到暫存目錄，再開啟 LATTICE 安裝畫面；不需要另外安裝 7-Zip。
請等安裝畫面完成，依畫面顯示判斷結果。解壓縮成功不代表三核心安裝成功。
官方 SFX 不會傳回安裝子程式的退出碼；外層退出 0 不能當成成功證據。
實際退出碼保存在 %LOCALAPPDATA%\\LATTICE\\sfx-last-result.json；
各項驗收結果保存在 %LOCALAPPDATA%\\LATTICE\\one-click-report.json。
如畫面要求重新開機，請先儲存工作，自行重開 Windows，再雙擊同一個安裝程式。
這個候選安裝程式尚未附加數位簽章。
"""


class Rejected(Exception):
    pass


def regular(path: Path) -> Path:
    if not path.is_absolute() or ".." in path.parts:
        raise Rejected("ABSOLUTE_PATH_REQUIRED")
    for part in (path, *path.parents):
        try:
            info = part.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
            raise Rejected("PATH_REDIRECTION_REJECTED")
    return path


def sha(path: Path) -> str:
    regular(path)
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def inventory(root: Path) -> dict:
    files = {}
    for directory, folders, names in os.walk(root, followlinks=False):
        for name in [*folders, *names]:
            path = regular(Path(directory) / name)
            if path.is_file():
                files[path.relative_to(root).as_posix()] = sha(path)
    return dict(sorted(files.items()))


def run(arguments: list[str], *, cwd: Path) -> None:
    result = subprocess.run(arguments, cwd=cwd, capture_output=True, stdin=subprocess.DEVNULL,
                            timeout=3600, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    if result.returncode:
        raise Rejected("SEVENZIP_OPERATION_FAILED_" + str(result.returncode))


def sfx_config() -> bytes:
    command = '"%%T\\bundle\\python\\python.exe" -I -B -S "%%T\\lattice-sfx-launch.py"'
    escaped = command.replace("\\", "\\\\").replace('"', '\\"')
    return (';!@Install@!UTF-8!\nTitle="LATTICE 安裝程式"\nProgress="yes"\nDirectory=""\n'
            'RunProgram="' + escaped + '"\n;!@InstallEnd@!\n').encode("utf-8")


def prepare_stub(module: Path, destination: Path) -> None:
    """Declare this per-user wrapper asInvoker; WSL retains its explicit UAC flow.

SDK 7zSD has no manifest, so Windows otherwise heuristically elevates setup.exe.
Only modify our copy, as permitted by SDK DOC/installer.txt. Never alter UAC.
"""
    shutil.copyfile(module, destination)
    api = ctypes.WinDLL("kernel32", use_last_error=True)
    api.BeginUpdateResourceW.argtypes = [ctypes.c_wchar_p, ctypes.c_int]
    api.BeginUpdateResourceW.restype = ctypes.c_void_p
    api.UpdateResourceW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_ushort, ctypes.c_void_p, ctypes.c_uint]
    api.UpdateResourceW.restype = ctypes.c_int
    api.EndUpdateResourceW.argtypes = [ctypes.c_void_p, ctypes.c_int]
    api.EndUpdateResourceW.restype = ctypes.c_int
    handle = api.BeginUpdateResourceW(str(destination), False)
    if not handle:
        raise Rejected("SFX_MANIFEST_UPDATE_FAILED")
    body = ctypes.create_string_buffer(AS_INVOKER_MANIFEST)
    if not api.UpdateResourceW(handle, 24, 1, 0, body, len(AS_INVOKER_MANIFEST)):
        api.EndUpdateResourceW(handle, True)
        raise Rejected("SFX_MANIFEST_UPDATE_FAILED")
    if not api.EndUpdateResourceW(handle, False):
        raise Rejected("SFX_MANIFEST_UPDATE_FAILED")


def build(package: Path, output: Path, sevenzip: Path, module: Path) -> dict:
    for path in (package, output, sevenzip, module):
        regular(path)
    if not package.is_dir() or output.suffix.lower() != ".exe" or not output.parent.is_dir():
        raise Rejected("INSTALLER_BUILD_PATH_REJECTED")
    if output.is_relative_to(package):
        raise Rejected("PACKAGE_OUTPUT_OVERLAP")
    if sha(module) != MODULE_SHA256:
        raise Rejected("OFFICIAL_SFX_MODULE_DIGEST_REJECTED")
    for name in ("Install-LATTICE.cmd", "bundle/python/python.exe"):
        if not (package / name).is_file():
            raise Rejected("INSTALLER_ENTRY_MISSING")
    # UTF-8 batch files with bare LF can be misparsed by cmd.exe. Check bytes,
    # without universal-newline translation, before creating any build output.
    if b"\n" in (package / "Install-LATTICE.cmd").read_bytes().replace(b"\r\n", b""):
        raise Rejected("BATCH_CRLF_REQUIRED")
    archive, config = output.with_suffix(".7z"), output.with_suffix(".config.txt")
    receipt, extra = output.with_suffix(".build.json"), output.with_suffix(".sfx-files")
    stub = output.with_suffix(".stub.sfx")
    if any(path.exists() for path in (output, archive, config, receipt, extra, stub)):
        raise Rejected("FRESH_BUILD_OUTPUT_REQUIRED")
    before = inventory(package)
    if any(name in before for name in ("lattice-sfx-launch.py", "LZMA-SDK-LICENSE.txt", "README-SFX.txt")):
        raise Rejected("SFX_BOOTSTRAP_NAME_CONFLICT")
    extra.mkdir()
    prepare_stub(module, stub)
    shutil.copyfile(Path(__file__).with_name("lattice-sfx-launch.py"), extra / "lattice-sfx-launch.py")
    (extra / "LZMA-SDK-LICENSE.txt").write_text(LICENSE, encoding="utf-8")
    (extra / "README-SFX.txt").write_text(README, encoding="utf-8")
    with config.open("xb") as stream:
        stream.write(sfx_config())
    options = [str(sevenzip), "a", "-t7z", "-m0=LZMA2", "-mx=1", "-md=16m", "-ms=32m", "-mmt=2", "-bb0", "-bd", str(archive), ".\\*"]
    run(options, cwd=package)
    run(options, cwd=extra)
    run([str(sevenzip), "t", "-bb0", "-bd", str(archive)], cwd=output.parent)
    if inventory(package) != before:
        raise Rejected("PACKAGE_CHANGED_DURING_BUILD")
    with output.open("xb") as target:
        for part in (stub, config, archive):
            with part.open("rb") as source:
                shutil.copyfileobj(source, target, 1024 * 1024)
    run([str(sevenzip), "t", "-bb0", "-bd", str(output)], cwd=output.parent)
    result = {"schema": "lattice.sfx-build.v1", "status": "BUILT_ARCHIVE_TESTED", "installer": str(output),
              "sha256": sha(output), "bytes": output.stat().st_size, "archive_sha256": sha(archive),
              "source_tree_sha256": hashlib.sha256(json.dumps(before, sort_keys=True).encode()).hexdigest(),
              "source_files": len(before), "sfx_module_sha256": sha(module), "sdk_source": SDK_URL, "sdk_sha256": SDK_SHA256,
              "packaged_sfx_module_sha256": sha(stub), "requested_execution_level": "asInvoker",
              "sevenzip_sha256": sha(sevenzip), "bootstrap_sha256": sha(extra / "lattice-sfx-launch.py"),
              "signed": False, "installed": False, "sfx_exit_code": "DOES_NOT_PROPAGATE_SETUP_EXIT_CODE",
              "setup_result": "%LOCALAPPDATA%/LATTICE/sfx-last-result.json",
              "reboot_resume": "Save work, restart Windows, then run this same installer again."}
    with receipt.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, ensure_ascii=False, indent=2)
    return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("package", "output", "sevenzip", "sfx-module"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        result = build(args.package, args.output, args.sevenzip, args.sfx_module)
    except (Rejected, OSError, subprocess.TimeoutExpired) as error:
        print(json.dumps({"status": "BLOCKED", "code": str(error) if isinstance(error, Rejected) else "SFX_BUILD_FAILED_PARTIAL_PRESERVED"}))
        return 2
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
