#!/usr/bin/env python3
"""Prepare a verified LATTICE MCP binary without interrupting Codex tasks.

This hook deliberately never terminates a process.  It only changes Codex's
MCP command for future sessions after the writer-lock directory is empty.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time


LOCK_SUFFIX = ".lock"
COMMAND = re.compile(
    r'(?m)^(\[mcp_servers\.lattice\]\r?\ncommand = ")[^"]+("\r?$)'
)


def write_receipt(root: Path, status: str, **fields: object) -> None:
    receipt_root = root / "automation" / "safe-mcp-update"
    receipt_root.mkdir(parents=True, exist_ok=True)
    receipt = {"schema_version": "lattice.safe-mcp-update.v1", "status": status, **fields}
    path = receipt_root / f"{int(time.time() * 1000)}.json"
    path.write_text(json.dumps(receipt, sort_keys=True) + "\n", encoding="utf-8")


def active_locks(codex_home: Path) -> list[str]:
    locks = codex_home / "thread-writer-locks"
    if not locks.is_dir():
        return []
    return sorted(
        entry.name
        for entry in locks.iterdir()
        if entry.name.endswith(LOCK_SUFFIX) and entry.name != ".coordination.lock"
    )


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def git_head(source_root: Path) -> str:
    if subprocess.check_output(["git", "-C", str(source_root), "status", "--porcelain"], text=True):
        raise RuntimeError("LATTICE_SAFE_UPDATE_SOURCE_DIRTY")
    return subprocess.check_output(
        ["git", "-C", str(source_root), "rev-parse", "--verify", "HEAD"], text=True
    ).strip()


def build_candidate(source_root: Path, cache_root: Path, revision: str) -> Path:
    candidate_root = cache_root / f"latticed-runtime-{revision[:12]}"
    executable = candidate_root / "release" / "latticed.exe"
    if executable.is_file() and executable.stat().st_size > 0:
        return executable
    subprocess.run(
        [
            "cargo", "build", "--release", "-p", "lattice-runtime", "--bin", "latticed",
            "--locked", "--target-dir", str(candidate_root),
        ],
        cwd=source_root,
        check=True,
        timeout=600,
    )
    if not executable.is_file() or executable.stat().st_size == 0:
        raise RuntimeError("LATTICE_SAFE_UPDATE_CANDIDATE_MISSING")
    return executable


def verify_candidate(executable: Path) -> None:
    # An invalid argument is rejected before LATTICE can perform any delivery action.
    try:
        result = subprocess.run(
            [str(executable), "--help"], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, timeout=15, check=False,
        )
    except OSError as error:
        raise RuntimeError("LATTICE_SAFE_UPDATE_CANDIDATE_VERIFICATION_FAILED") from error
    if result.returncode == 0 or "LATTICED_ARGUMENTS_REJECTED" not in (result.stdout + result.stderr):
        raise RuntimeError("LATTICE_SAFE_UPDATE_CANDIDATE_VERIFICATION_FAILED")


def atomic_write(config: Path, text: str) -> None:
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", newline="", delete=False, dir=config.parent) as target:
        target.write(text)
        temporary = Path(target.name)
    os.replace(temporary, config)


def replace_command(config: Path, executable: Path) -> str:
    original = config.read_text(encoding="utf-8")
    toml_path = str(executable).replace("\\", "\\\\")
    replacement, changed = COMMAND.subn(
        lambda match: f"{match.group(1)}{toml_path}{match.group(2)}",
        original,
        count=1,
    )
    if changed != 1:
        raise RuntimeError("LATTICE_SAFE_UPDATE_CONFIG_REJECTED")
    atomic_write(config, replacement)
    return original


def main() -> int:
    codex_home = Path(os.environ.get("CODEX_HOME", Path.home() / ".codex"))
    lattice_root = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local")) / "LATTICE"
    source_root = Path(
        os.environ.get("LATTICE_SAFE_UPDATE_SOURCE_ROOT", Path(__file__).resolve().parents[1])
    )
    config = codex_home / "config.toml"
    locks = active_locks(codex_home)
    if locks:
        write_receipt(lattice_root, "DEFERRED_ACTIVE_CODEX_TASKS", active_lock_count=len(locks))
        return 0
    original_config = None
    try:
        revision = git_head(source_root)
        executable = build_candidate(source_root, lattice_root / "build-cache", revision)
        verify_candidate(executable)
        original_config = replace_command(config, executable)
        write_receipt(lattice_root, "ACTIVATED", revision=revision, executable_sha256=sha256(executable))
        return 0
    except Exception as error:  # hook errors must preserve the previous command
        if original_config is not None:
            try:
                atomic_write(config, original_config)
            except Exception:
                write_receipt(lattice_root, "ROLLBACK_FAILED", code="LATTICE_SAFE_UPDATE_ROLLBACK_FAILED")
                return 0
        write_receipt(lattice_root, "FAILED_PRESERVED_PREVIOUS", code=str(error))
        return 0


if __name__ == "__main__":
    sys.exit(main())
