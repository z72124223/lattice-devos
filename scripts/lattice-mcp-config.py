#!/usr/bin/env python3
"""Incremental Codex connection management for a separately prepared Runtime.

Python 3.11+. No service activation, credentials, downloads or policy changes.
Backups stay next to the customer's config; never include them in a package.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import tempfile
import tomllib
import uuid

BEGIN = b"# BEGIN LATTICE MCP MANAGED v1\n"
END = b"# END LATTICE MCP MANAGED v1\n"
SCHEMA = "lattice.mcp-config.v1"


class Rejected(Exception):
    pass


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def regular_path(path: Path) -> None:
    # A junction can redirect a write just as a symbolic link can on Windows.
    for part in [path, *path.parents]:
        try:
            info = part.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
            raise Rejected("PATH_REDIRECTION_REJECTED")


def read_config(path: Path) -> bytes:
    regular_path(path)
    if not path.exists():
        return b""
    if not path.is_file() or path.stat().st_size > 4 * 1024 * 1024:
        raise Rejected("CONFIG_FILE_REJECTED")
    data = path.read_bytes()
    parse(data)
    return data


def parse(data: bytes) -> dict:
    try:
        return tomllib.loads(data.decode("utf-8-sig"))
    except (ValueError, UnicodeError):
        raise Rejected("CONFIG_TOML_INVALID") from None


def copy_access(source: Path, target: Path) -> None:
    """Preserve access to existing config bytes before writing a new copy."""
    if os.name != "nt":
        os.chmod(target, stat.S_IMODE(source.stat().st_mode))
        return
    import ctypes as c
    from ctypes import wintypes as w
    api = c.WinDLL("advapi32", use_last_error=True)
    kernel = c.WinDLL("kernel32", use_last_error=True)
    pointer = c.c_void_p
    api.GetNamedSecurityInfoW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD] + [c.POINTER(pointer)] * 5
    api.GetNamedSecurityInfoW.restype = w.DWORD
    api.SetNamedSecurityInfoW.argtypes = [w.LPWSTR, w.DWORD, w.DWORD] + [pointer] * 4
    api.SetNamedSecurityInfoW.restype = w.DWORD
    api.GetSecurityDescriptorControl.argtypes = [pointer, c.POINTER(w.WORD), c.POINTER(w.DWORD)]
    api.GetSecurityDescriptorControl.restype = w.BOOL
    kernel.LocalFree.argtypes = [pointer]
    kernel.LocalFree.restype = pointer
    dacl, descriptor = pointer(), pointer()
    code = api.GetNamedSecurityInfoW(str(source), 1, 4, None, None, c.byref(dacl), None, c.byref(descriptor))
    if code:
        raise Rejected("CONFIG_ACCESS_READ_DENIED")
    try:
        control, revision = w.WORD(), w.DWORD()
        if not api.GetSecurityDescriptorControl(descriptor, c.byref(control), c.byref(revision)):
            raise Rejected("CONFIG_ACCESS_READ_DENIED")
        protection = 0x80000000 if control.value & 0x1000 else 0x20000000
        if api.SetNamedSecurityInfoW(str(target), 1, 4 | protection, None, None, dacl, None):
            raise Rejected("CONFIG_ACCESS_COPY_DENIED")
    finally:
        kernel.LocalFree(descriptor)


def atomic_write(path: Path, data: bytes) -> None:
    regular_path(path)
    fd, name = tempfile.mkstemp(prefix=".lattice-", dir=path.parent)
    temporary = Path(name)
    try:
        with os.fdopen(fd, "wb") as target:
            if path.exists():
                copy_access(path, temporary)
            target.write(data)
            target.flush()
            os.fsync(target.fileno())
        if path.exists():
            os.chmod(temporary, stat.S_IMODE(path.stat().st_mode))
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def json_bytes(value: dict) -> bytes:
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()


@contextmanager
def manager_lock(folder: Path):
    regular_path(folder)
    folder.mkdir(mode=0o700, exist_ok=True)
    lock_path = folder / "manager.lock"
    regular_path(lock_path)
    with lock_path.open("a+b") as handle:
        if handle.tell() == 0:
            handle.write(b"0")
            handle.flush()
        handle.seek(0)
        try:
            if os.name == "nt":
                import msvcrt
                msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            raise Rejected("CONFIG_MANAGER_BUSY") from None
        try:
            yield
        finally:
            if os.name == "nt":
                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(handle, fcntl.LOCK_UN)


def read_json(path: Path) -> dict | None:
    regular_path(path)
    if not path.exists():
        return None
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (ValueError, UnicodeError):
        raise Rejected("MANAGEMENT_RECORD_INVALID") from None
    if not isinstance(value, dict) or value.get("schema") != SCHEMA:
        raise Rejected("MANAGEMENT_RECORD_INVALID")
    return value


def make_block(runtime: Path, expected_digest: str, arguments: list[str] | None = None) -> str:
    if not runtime.is_absolute():
        raise Rejected("RUNTIME_ABSOLUTE_PATH_REQUIRED")
    regular_path(runtime)
    if not runtime.is_file():
        raise Rejected("RUNTIME_MISSING")
    if not re.fullmatch(r"[0-9a-f]{64}", expected_digest) or digest(runtime.read_bytes()) != expected_digest:
        raise Rejected("RUNTIME_DIGEST_MISMATCH")
    # JSON basic strings are valid TOML strings for these path characters.
    command = json.dumps(str(runtime), ensure_ascii=False)
    if arguments is not None and (not isinstance(arguments, list) or len(arguments) > 16
            or any(not isinstance(arg, str) or len(arg) > 4096 or any(ord(c) < 32 for c in arg) for arg in arguments)):
        raise Rejected("RUNTIME_ARGUMENTS_REJECTED")
    args = "args = " + json.dumps(arguments, ensure_ascii=False) + "\n" if arguments else ""
    # Customer startup hashes the sealed dependency bundle before its handshake,
    # and may start the owned PostgreSQL cluster after a Windows restart.
    timeouts = "startup_timeout_sec = 600\ntool_timeout_sec = 120\n" if arguments and "serve" in arguments and "--state" in arguments else ""
    block = BEGIN.decode() + "[mcp_servers.lattice]\ncommand = " + command + "\n" + args + timeouts + END.decode()
    parse(block.encode())
    return block


def replace_owned(data: bytes, state: dict, replacement: str) -> bytes:
    block = state.get("block")
    if not isinstance(block, str) or not block.startswith(BEGIN.decode()) or not block.endswith(END.decode()):
        raise Rejected("MANAGEMENT_RECORD_INVALID")
    owned = block.encode()
    if parse(data).get("mcp_servers", {}).get("lattice") != parse(owned)["mcp_servers"]["lattice"]:
        raise Rejected("MANAGED_TABLE_CHANGED")
    # Markers inside user TOML strings must never be mistaken for an owned table.
    if data.count(owned) != 1 or data.count(BEGIN) != 1 or data.count(END) != 1:
        raise Rejected("MANAGED_BLOCK_CHANGED")
    without = data.replace(owned, b"", 1)
    if "lattice" in parse(without).get("mcp_servers", {}):
        raise Rejected("LATTICE_CONFIG_CONFLICT")
    result = data.replace(owned, replacement.encode(), 1)
    parsed = parse(result)
    if replacement and parsed.get("mcp_servers", {}).get("lattice") != parse(replacement.encode())["mcp_servers"]["lattice"]:
        raise Rejected("LATTICE_CONFIG_CONFLICT")
    return result


def locations(config: Path) -> tuple[Path, Path, Path]:
    if not config.is_absolute():
        raise Rejected("CONFIG_ABSOLUTE_PATH_REQUIRED")
    regular_path(config)
    # A separate sidecar per configuration filename also supports isolated tests.
    folder = config.parent / ("." + config.name + ".lattice")
    return folder, folder / "state.json", folder / "pending.json"


def diagnose(config: Path) -> dict:
    folder, state_path, pending_path = locations(config)
    pending = read_json(pending_path)
    if pending:
        # A partially written file may not even be valid TOML. Keep the backup
        # and journal rather than guessing whether newer customer edits exist.
        current = config.read_bytes() if config.exists() else b""
        match = "before" if digest(current) == pending["before_digest"] else "after" if digest(current) == pending["after_digest"] else "changed_or_partial"
        return {"schema": SCHEMA, "status": "RECOVERY_REQUIRED", "config_match": match,
                "runtime_workflow": "NOT_VERIFIED"}
    data = read_config(config)
    state = read_json(state_path)
    managed = bool(state and state.get("block"))
    if managed:
        replace_owned(data, state, state["block"])
    return {"schema": SCHEMA, "status": "RECOVERY_REQUIRED" if pending else "INSPECTED",
            "config_exists": config.exists(), "lattice_configured": "lattice" in parse(data).get("mcp_servers", {}),
            "managed": managed, "runtime_workflow": "NOT_VERIFIED",
            "customer_policy": "PRESERVED_NOT_EVALUATED"}


def recover(config: Path, state_path: Path, pending_path: Path) -> dict:
    pending = read_json(pending_path)
    if not pending:
        return {"schema": SCHEMA, "status": "NO_RECOVERY_NEEDED"}
    current = read_config(config)
    current_digest = digest(current)
    if current_digest == pending["after_digest"]:
        atomic_write(state_path, json_bytes(pending["after_state"]))
        status = "RECOVERED_APPLIED"
    elif current_digest == pending["before_digest"]:
        status = "RECOVERED_NOT_APPLIED"
    else:
        raise Rejected("RECOVERY_CONFIG_CHANGED")
    pending_path.unlink()
    return {"schema": SCHEMA, "status": status, "runtime_workflow": "NOT_VERIFIED"}


@contextmanager
def exclusive_config(config: Path, existed: bool):
    """Windows denies other reads/writes/renames until this handle closes.

    In-place writes preserve file identity and its complete security metadata.
    A power loss can leave a partial file: the prior backup and pending journal
    are durable first, and unknown bytes are never automatically overwritten.
    """
    import ctypes as c
    from ctypes import wintypes as w
    import msvcrt
    kernel = c.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, c.c_void_p, w.DWORD, w.DWORD, w.HANDLE]
    kernel.CreateFileW.restype = w.HANDLE
    regular_path(config)
    handle = kernel.CreateFileW(str(config), 0xC0000000, 0, None, 3 if existed else 1, 0x80, None)
    if handle == c.c_void_p(-1).value:
        raise Rejected("CONFIG_EXCLUSIVE_ACCESS_DENIED")
    descriptor = msvcrt.open_osfhandle(handle, os.O_RDWR | os.O_BINARY)
    with os.fdopen(descriptor, "r+b") as stream:
        yield stream


def write_config(config: Path, before: bytes, after: bytes, existed: bool) -> None:
    with exclusive_config(config, existed) as stream:
        if stream.read() != before:
            raise Rejected("CONFIG_CHANGED_DURING_OPERATION")
        stream.seek(0)
        stream.write(after)
        stream.truncate()
        stream.flush()
        os.fsync(stream.fileno())
        stream.seek(0)
        if stream.read() != after:
            raise Rejected("CONFIG_READBACK_MISMATCH")


def change(config: Path, operation: str, runtime: Path | None = None, runtime_digest: str = "", arguments: list[str] | None = None) -> dict:
    if os.name != "nt":
        raise Rejected("CONFIG_WRITE_PLATFORM_NOT_VERIFIED")
    folder, state_path, pending_path = locations(config)
    # Do not create a customer home, change its permissions, or request elevation.
    if not config.parent.is_dir():
        raise Rejected("CONFIG_PARENT_MISSING")
    if config.exists() and not config.stat().st_mode & 0o222:
        raise Rejected("CONFIG_READ_ONLY")
    if config.exists():
        # An atomic rename may be allowed by the parent even when writing the
        # existing file is forbidden. Respect that file's write restriction.
        with config.open("r+b"):
            pass
    with manager_lock(folder):
        if operation == "recover":
            return recover(config, state_path, pending_path)
        if read_json(pending_path):
            raise Rejected("RECOVERY_REQUIRED")
        before = read_config(config)
        existed = config.exists()
        state = read_json(state_path)
        block = state.get("block", "") if state else ""
        if operation in ("install", "update"):
            if runtime is None:
                raise Rejected("RUNTIME_REQUIRED")
            new_block = make_block(runtime, runtime_digest, arguments)
        else:
            new_block = ""
        if operation == "install":
            if block:
                replace_owned(before, state, block)
                if block == new_block:
                    return {"schema": SCHEMA, "status": "UNCHANGED", "runtime_workflow": "NOT_VERIFIED"}
                raise Rejected("USE_UPDATE_FOR_MANAGED_INSTALL")
            if "lattice" in parse(before).get("mcp_servers", {}) or BEGIN in before or END in before:
                raise Rejected("LATTICE_CONFIG_CONFLICT")
            separator = "" if not before or before.endswith(b"\n") else "\n"
            after = before + separator.encode() + new_block.encode()
        elif operation in ("update", "remove", "rollback"):
            if not block:
                raise Rejected("MANAGED_INSTALL_REQUIRED")
            separator = state.get("separator", "")
            if operation == "rollback":
                new_block = state.get("previous_block", "")
                if not new_block:
                    raise Rejected("NO_PREVIOUS_RUNTIME")
                previous = parse(new_block.encode())["mcp_servers"]["lattice"]
                previous_path = previous["command"]
                if make_block(Path(previous_path), state.get("previous_runtime_digest", ""), previous.get("args")) != new_block:
                    raise Rejected("PREVIOUS_RUNTIME_CHANGED")
            after = replace_owned(before, state, new_block)
            if operation == "remove" and state.get("original_digest") == digest(after.removesuffix(separator.encode()) if separator else after):
                after = after.removesuffix(separator.encode()) if separator else after
        else:
            raise Rejected("OPERATION_REJECTED")
        parse(after)
        if after == before:
            return {"schema": SCHEMA, "status": "UNCHANGED", "runtime_workflow": "NOT_VERIFIED"}
        new_state = {"schema": SCHEMA, "block": new_block,
                     "previous_block": block if operation in ("update", "rollback") else "",
                     "runtime_digest": state.get("previous_runtime_digest", "") if operation == "rollback" else runtime_digest if new_block else "",
                     "previous_runtime_digest": state.get("runtime_digest", "") if state and operation in ("update", "rollback") else "",
                     "separator": separator, "original_digest": state.get("original_digest", digest(before)) if state else digest(before)}
        backup = folder / (uuid.uuid4().hex + ".config.bak")
        # The private backup contains original bytes, including any customer secrets.
        descriptor = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as target:
            if existed:
                copy_access(config, backup)
            target.write(before)
            target.flush()
            os.fsync(target.fileno())
        os.chmod(backup, stat.S_IMODE(config.stat().st_mode) if existed else 0o600)
        pending = {"schema": SCHEMA, "before_digest": digest(before), "after_digest": digest(after),
                   "after_state": new_state, "backup": backup.name}
        atomic_write(pending_path, json_bytes(pending))
        # Reject observed concurrent edits, including absent -> empty creation.
        if config.exists() != existed or read_config(config) != before:
            raise Rejected("CONFIG_CHANGED_DURING_OPERATION")
        write_config(config, before, after, existed)
        if read_config(config) != after:
            raise Rejected("CONFIG_READBACK_MISMATCH")
        atomic_write(state_path, json_bytes(new_state))
        pending_path.unlink()
        return {"schema": SCHEMA, "status": "CONFIG_" + operation.upper() + "_APPLIED",
                "backup_saved": True, "runtime_workflow": "NOT_VERIFIED",
                "customer_policy": "PRESERVED_NOT_EVALUATED"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["diagnose", "install", "update", "rollback", "remove", "recover"])
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--sha256", default="")
    args = parser.parse_args()
    try:
        result = diagnose(args.config) if args.operation == "diagnose" else change(args.config, args.operation, args.runtime, args.sha256)
        print(json.dumps(result))
        return 0
    except Rejected as error:
        code = str(error)
    except PermissionError:
        code = "PERMISSION_DENIED_PRESERVED"
    except OSError:
        code = "FILESYSTEM_OPERATION_FAILED_RECOVERY_MAY_BE_REQUIRED"
    except (KeyError, TypeError, ValueError):
        code = "MANAGEMENT_RECORD_INVALID"
    print(json.dumps({"schema": SCHEMA, "status": "BLOCKED", "code": code}))
    return 2


if __name__ == "__main__":
    sys.exit(main())
