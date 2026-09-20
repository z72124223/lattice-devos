"""Observe app-local CRT modules in a new owned MCP and its prepared PostgreSQL.

Uses Windows Process.Modules (after Refresh), not DLL presence as live evidence.
Never prints configuration, credentials, arbitrary stderr, or tool response bodies.
"""
import argparse
import base64
import ctypes
import hashlib
import json
import os
from pathlib import Path
import queue
import stat
import subprocess
import sys
import threading

PINS = {
    "vcruntime140.dll": "d5e4d9a3e835fa679450145d6a7d94e36573a509317111904d9b3712c30d9066",
    "msvcp140.dll": "0f885b509a685d2bbfa652fed26b5fb31d88fbdab0a978c641d1c7b8aa460aa9",
}


class Rejected(Exception):
    pass


def require(condition, code):
    if not condition:
        raise Rejected(code)


def regular(path):
    path = Path(path)
    require(path.is_absolute(), "CRT_ABSOLUTE_PATH_REQUIRED")
    for item in (path, *path.parents):
        info = item.lstat()
        require(not stat.S_ISLNK(info.st_mode) and not getattr(info, "st_file_attributes", 0) & 0x400,
                "CRT_PATH_REDIRECTION_REJECTED")
    return path


def digest(path):
    return hashlib.sha256(regular(path).read_bytes()).hexdigest()


def powershell(code, env):
    buffer = ctypes.create_unicode_buffer(32768)
    require(0 < ctypes.windll.kernel32.GetSystemDirectoryW(buffer, len(buffer)) < len(buffer),
            "CRT_SYSTEM_DIRECTORY_UNAVAILABLE")
    executable = str(Path(buffer.value) / "WindowsPowerShell/v1.0/powershell.exe")
    script = "$ErrorActionPreference='Stop';[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);" + code
    result = subprocess.run([executable, "-NoProfile", "-NonInteractive", "-EncodedCommand",
                             base64.b64encode(script.encode("utf-16-le")).decode("ascii")],
                            env=env, capture_output=True, timeout=45, creationflags=subprocess.CREATE_NO_WINDOW)
    require(result.returncode == 0 and len(result.stdout) <= 1_000_000, "CRT_PROCESS_QUERY_FAILED")
    return json.loads(result.stdout.decode("utf-8-sig"))


def observe(parent_pid, pg_pid, env):
    # Only numeric process IDs enter PowerShell source; never interpolate paths.
    return powershell("""
function Modules($processId) {
  $p = Get-Process -Id $processId; $p.Refresh()
  @{pid=$p.Id; executable=$p.MainModule.FileName; modules=@($p.Modules |
    Where-Object {$_.ModuleName -in @('vcruntime140.dll','msvcp140.dll')} |
    ForEach-Object {@{name=$_.ModuleName; path=$_.FileName}})}
}
$children = @(Get-CimInstance Win32_Process -Filter 'ParentProcessId = %d' |
  Where-Object {$_.Name -eq 'latticed.exe'})
if ($children.Count -ne 1) { exit 2 }
@{native=(Modules $children[0].ProcessId); postgres=(Modules %d)} | ConvertTo-Json -Depth 6 -Compress
""" % (int(parent_pid), int(pg_pid)), env)


def validate_modules(observed, executable, names):
    executable = regular(executable)
    require(Path(observed.get("executable", "")) == executable, "CRT_PROCESS_IMAGE_MISMATCH")
    require(isinstance(observed.get("pid"), int) and observed["pid"] > 0, "CRT_PROCESS_ID_INVALID")
    verified = []
    for name in names:
        matches = [row for row in observed.get("modules", []) if row.get("name", "").casefold() == name]
        require(len(matches) == 1, "CRT_MODULE_NOT_LOADED")
        loaded = Path(matches[0]["path"])
        require(loaded == executable.parent / name, "CRT_MODULE_NOT_APP_LOCAL")
        require(digest(loaded) == PINS[name], "CRT_LOADED_FILE_HASH_MISMATCH")
        verified.append({"name": name, "source": "EXECUTABLE_DIRECTORY", "sha256": PINS[name]})
    return {"pid": observed["pid"], "modules": verified}


def reply(process, message, expected_id, timeout=300):
    process.stdin.write(json.dumps(message) + "\n")
    process.stdin.flush()
    replies = queue.Queue(maxsize=1)

    def read():
        try:
            for _ in range(16):
                line = process.stdout.readline(1_000_001)
                require(line and len(line) <= 1_000_000, "CRT_MCP_RESPONSE_INVALID")
                value = json.loads(line)
                if value.get("id") == expected_id:
                    replies.put(value)
                    return
            raise Rejected("CRT_MCP_RESPONSE_MISSING")
        except Exception:
            replies.put(None)

    threading.Thread(target=read, daemon=True).start()
    try:
        value = replies.get(timeout=timeout)
    except queue.Empty:
        raise Rejected("CRT_MCP_RESPONSE_TIMEOUT") from None
    require(isinstance(value, dict) and "error" not in value, "CRT_MCP_RESPONSE_REJECTED")
    return value.get("result", {})


def close_owned_mcp(process, env):
    try:
        process.stdin.close()
        process.wait(timeout=10)
        return process.returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        # If EOF failed, stop only an identified child still owned by our live wrapper.
        try:
            if process.poll() is None:
                powershell("""
  $owned = @(Get-CimInstance Win32_Process -Filter 'ParentProcessId = %d' |
    Where-Object {$_.Name -eq 'latticed.exe' -and $_.ExecutablePath -eq $env:LATTICE_CRT_NATIVE_EXE})
  if ($owned.Count -gt 1) { exit 2 }
  foreach ($child in $owned) { Stop-Process -Id $child.ProcessId -ErrorAction Stop }
  @{cleanup='OWNED_CHILD_ONLY'} | ConvertTo-Json -Compress
""" % int(process.pid), env)
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
        return False


def verify(state):
    state = regular(state)
    require(os.name == "nt", "CRT_WINDOWS_REQUIRED")
    config = json.loads(regular(state / "installation.json").read_text(encoding="utf-8"))
    require(Path(config["root"]) == state, "CRT_STATE_MISMATCH")
    python = regular(config["python"])
    native = regular(config["runtime"])
    postgres = regular(Path(config["postgres_bin"]) / "postgres.exe")
    launcher = regular(state / "bin/lattice-customer-runtime.py")
    require(Path(sys.executable) == python, "CRT_BUNDLED_PYTHON_REQUIRED")
    require(native == state / "bin/latticed.exe", "CRT_NATIVE_PATH_MISMATCH")
    for executable, names in ((native, ("vcruntime140.dll",)), (postgres, tuple(PINS))):
        for name in names:
            require(digest(executable.parent / name) == PINS[name], "CRT_PINNED_FILE_REQUIRED")
    pg_lines = regular(state / "cluster/postmaster.pid").read_text(encoding="utf-8").splitlines()
    pg_pid = int(pg_lines[0])
    require(pg_pid > 0 and Path(pg_lines[1]) == state / "cluster", "CRT_POSTGRES_IDENTITY_MISMATCH")
    env = {key: os.environ[key] for key in ("SystemRoot", "WINDIR", "COMSPEC", "PATH", "PATHEXT", "TEMP",
           "TMP", "USERPROFILE", "LOCALAPPDATA", "APPDATA", "PROCESSOR_ARCHITECTURE") if key in os.environ}
    env["LATTICE_CRT_NATIVE_EXE"] = str(native)
    process = subprocess.Popen([str(python), "-I", "-B", "-S", str(launcher), "serve", "--state", str(state)],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                               text=True, encoding="utf-8", env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    outcome = None
    try:
        initialized = reply(process, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "lattice-crt-verifier", "version": "1"}}}, 1)
        require(initialized.get("protocolVersion") == "2025-11-25", "CRT_MCP_INITIALIZE_REJECTED")
        process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        status = reply(process, {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "lattice_runtime_status", "arguments": {}}}, 2, timeout=60)
        require(status.get("isError") is not True and status.get("structuredContent", {}).get("runtime_integration") == "GRAPHIFY",
                "CRT_MCP_RUNTIME_NOT_READY")
        observed = observe(process.pid, pg_pid, env)
        require(int(regular(state / "cluster/postmaster.pid").read_text().splitlines()[0]) == pg_pid,
                "CRT_POSTGRES_PROCESS_CHANGED")
        outcome = {"native": validate_modules(observed["native"], native, ("vcruntime140.dll",)),
                   "postgres": validate_modules(observed["postgres"], postgres, tuple(PINS))}
    finally:
        closed = close_owned_mcp(process, env)
    require(closed, "CRT_MCP_CLEANUP_NOT_VERIFIED")
    retained = powershell("$p=Get-Process -Id %d;$p.Refresh();@{image=$p.MainModule.FileName}|ConvertTo-Json -Compress" % pg_pid, env)
    require(Path(retained["image"]) == postgres and
            int(regular(state / "cluster/postmaster.pid").read_text().splitlines()[0]) == pg_pid,
            "CRT_POSTGRES_NOT_PRESERVED")
    return {"schema": "lattice.loaded-crt.v1", "status": "VERIFIED", "observed": outcome,
            "mcp_closed": True, "postgres_preserved": True, "ai_turn_started": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", required=True, type=Path)
    args = parser.parse_args()
    try:
        result = verify(args.state)
    except Rejected as error:
        result = {"schema": "lattice.loaded-crt.v1", "status": "BLOCKED", "code": str(error)}
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired):
        result = {"schema": "lattice.loaded-crt.v1", "status": "BLOCKED", "code": "CRT_OBSERVATION_FAILED"}
    print(json.dumps(result))
    return 0 if result["status"] == "VERIFIED" else 2


if __name__ == "__main__":
    sys.exit(main())
