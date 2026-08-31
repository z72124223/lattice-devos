#!/usr/bin/env python3
"""Prepare a verified LATTICE MCP binary without interrupting Codex tasks.

This hook deliberately never terminates a process.  It only changes Codex's
MCP command for future sessions; active writer locks are recorded as an
observation because existing processes retain their already-started command.
"""

from __future__ import annotations

import hashlib
import io
import json
import msvcrt
import os
from pathlib import Path
from pathlib import PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
from contextlib import contextmanager


LOCK_SUFFIX = ".lock"
ARTIFACT_PROVENANCE = "lattice-safe-mcp-artifact.v1.json"
COMMIT = re.compile(r"^[0-9a-f]{40}$")
COMMAND = re.compile(
    r'(?m)^(\[mcp_servers\.lattice\]\r?\ncommand = ")[^"]+("\r?$)'
)


def write_receipt(root: Path, status: str, **fields: object) -> None:
    receipt_root = root / "automation" / "safe-mcp-update"
    receipt_root.mkdir(parents=True, exist_ok=True)
    receipt = {"schema_version": "lattice.safe-mcp-update.v1", "status": status, **fields}
    path = receipt_root / f"{int(time.time() * 1000)}.json"
    path.write_text(json.dumps(receipt, sort_keys=True) + "\n", encoding="utf-8")


def lock_is_active(path: Path) -> bool:
    """Return whether another process currently holds the Windows byte lock.

    Codex deliberately keeps the zero-byte marker after a task or process
    restart.  File existence therefore is not evidence of an active writer.
    Acquiring the same first-byte lock is a read-only ownership probe: an
    unlocked marker remains untouched, while a live writer fails closed.
    """
    try:
        with path.open("r+b", buffering=0) as lock_file:
            try:
                msvcrt.locking(lock_file.fileno(), msvcrt.LK_NBLCK, 1)
            except PermissionError:
                return True
            else:
                msvcrt.locking(lock_file.fileno(), msvcrt.LK_UNLCK, 1)
                return False
    except FileNotFoundError:
        return False
    except PermissionError:
        return True


def active_locks(codex_home: Path) -> list[str]:
    locks = codex_home / "thread-writer-locks"
    if not locks.is_dir():
        return []
    return sorted(
        entry.name
        for entry in locks.iterdir()
        if entry.name.endswith(LOCK_SUFFIX)
        and entry.name != ".coordination.lock"
        and lock_is_active(entry)
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
    revision = subprocess.check_output(
        ["git", "-C", str(source_root), "rev-parse", "--verify", "HEAD"], text=True
    ).strip()
    if not COMMIT.fullmatch(revision):
        raise RuntimeError("LATTICE_SAFE_UPDATE_REVISION_REJECTED")
    return revision


def artifact_provenance_path(candidate_root: Path) -> Path:
    return candidate_root / ARTIFACT_PROVENANCE


def has_exact_artifact_provenance(executable: Path, revision: str) -> bool:
    try:
        provenance = json.loads(artifact_provenance_path(executable.parent.parent).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return False
    return (
        provenance == {
            "schema_version": "lattice.safe-mcp-artifact.v1",
            "revision": revision,
            "executable_sha256": sha256(executable),
        }
    )


def write_artifact_provenance(executable: Path, revision: str) -> None:
    provenance = {
        "schema_version": "lattice.safe-mcp-artifact.v1",
        "revision": revision,
        "executable_sha256": sha256(executable),
    }
    atomic_write(
        artifact_provenance_path(executable.parent.parent),
        json.dumps(provenance, sort_keys=True) + "\n",
    )


def archive_member_is_safe(member: tarfile.TarInfo) -> bool:
    path = PurePosixPath(member.name)
    return (
        not path.is_absolute()
        and ".." not in path.parts
        and (member.isfile() or member.isdir())
    )


@contextmanager
def materialize_commit_source(source_root: Path, revision: str):
    """Yield a deleted-on-exit source tree made only from the exact Git commit."""
    if not COMMIT.fullmatch(revision):
        raise RuntimeError("LATTICE_SAFE_UPDATE_REVISION_REJECTED")
    archive = subprocess.run(
        ["git", "-C", str(source_root), "archive", "--format=tar", revision],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
        timeout=120,
    )
    with tempfile.TemporaryDirectory(prefix="lattice-safe-mcp-source-") as temporary:
        snapshot_root = Path(temporary) / "source"
        snapshot_root.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive.stdout), mode="r:") as source_archive:
            members = source_archive.getmembers()
            if any(not archive_member_is_safe(member) for member in members):
                raise RuntimeError("LATTICE_SAFE_UPDATE_ARCHIVE_REJECTED")
            source_archive.extractall(snapshot_root, members=members, filter="data")
        yield snapshot_root


def build_candidate(source_root: Path, cache_root: Path, revision: str) -> Path:
    candidate_root = cache_root / f"latticed-runtime-{revision[:12]}"
    executable = candidate_root / "release" / "latticed.exe"
    if executable.is_file() and executable.stat().st_size > 0 and has_exact_artifact_provenance(executable, revision):
        return executable
    if candidate_root.exists():
        shutil.rmtree(candidate_root)
    with materialize_commit_source(source_root, revision) as snapshot_root:
        subprocess.run(
            [
                "cargo", "build", "--release", "-p", "lattice-runtime", "--bin", "latticed",
                "--locked", "--target-dir", str(candidate_root),
            ],
            cwd=snapshot_root,
            check=True,
            timeout=600,
        )
    if not executable.is_file() or executable.stat().st_size == 0:
        raise RuntimeError("LATTICE_SAFE_UPDATE_CANDIDATE_MISSING")
    write_artifact_provenance(executable, revision)
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
    original_config = None
    try:
        revision = git_head(source_root)
        executable = build_candidate(source_root, lattice_root / "build-cache", revision)
        verify_candidate(executable)
        activation_locks = active_locks(codex_home)
        activation_lock_observed_at_unix_ms = int(time.time() * 1000)
        original_config = replace_command(config, executable)
        write_receipt(
            lattice_root,
            "ACTIVATED",
            revision=revision,
            executable_sha256=sha256(executable),
            active_lock_count=len(activation_locks),
            active_lock_observation_stage="PRE_ACTIVATION",
            active_lock_observed_at_unix_ms=activation_lock_observed_at_unix_ms,
        )
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
