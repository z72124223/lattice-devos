#!/usr/bin/env python3
"""Retain a verified bundle outside Downloads; preserve all failed copies."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess
import sys
import uuid


class Rejected(Exception):
    pass


def regular_path(path: Path) -> Path:
    if not path.is_absolute() or ".." in path.parts:
        raise Rejected("ABSOLUTE_CANONICAL_PATH_REQUIRED")
    for part in path.parts[1:]:
        if part.endswith((".", " ")) or ":" in part:
            raise Rejected("PATH_ALIAS_REJECTED")
    for entry in (path, *path.parents):
        try:
            info = entry.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
            raise Rejected("PATH_REDIRECTION_REJECTED")
    if path != path.resolve():
        raise Rejected("PATH_ALIAS_REJECTED")
    return path


def verify(source: Path, target: Path, expected: str) -> None:
    # Execute only the original verifier, also when checking an existing target.
    python = regular_path(source / "python/python.exe")
    script = regular_path(source / "bin/lattice-bundle.py")
    result = subprocess.run([str(python), "-I", "-B", "-S", str(script), "verify",
                             "--bundle", str(target), "--sha256", expected],
                            capture_output=True, stdin=subprocess.DEVNULL, timeout=900,
                            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    try:
        payload = json.loads(result.stdout)
    except (ValueError, UnicodeError):
        raise Rejected("ASSET_VERIFICATION_UNREADABLE") from None
    if result.returncode or not isinstance(payload, dict) or payload.get("status") != "LOCAL_BUNDLE_VERIFIED":
        raise Rejected("ASSET_BUNDLE_VERIFICATION_FAILED")


def stage(source_bundle: Path, target_parent: Path) -> dict:
    # Windows rename refuses an existing destination, including an empty folder.
    if os.name != "nt":
        raise Rejected("WINDOWS_REQUIRED")
    source = regular_path(Path(source_bundle))
    parent = regular_path(Path(target_parent))
    if source == parent or source.is_relative_to(parent) or parent.is_relative_to(source):
        raise Rejected("ASSET_PATHS_OVERLAP")
    body = regular_path(source / "bundle.json").read_bytes()
    expected = hashlib.sha256(body).hexdigest()
    data = json.loads(body)
    verify(source, source, expected)
    final = regular_path(parent / expected)
    if final.exists():
        verify(source, final, expected)
        return {"status": "REUSED", "bundle": str(final), "manifest_sha256": expected}
    parent.mkdir(parents=True, exist_ok=True)
    regular_path(parent)
    temporary = regular_path(parent / ("." + expected + ".partial-" + uuid.uuid4().hex))
    temporary.mkdir()  # Never adopt, overwrite, or delete a partial directory.
    for name in ["bundle.json", *data["files"]]:
        relative = PurePosixPath(name)
        if relative.is_absolute() or ".." in relative.parts or "\\" in name or ":" in name or relative.as_posix() != name:
            raise Rejected("ASSET_MANIFEST_PATH_REJECTED")
        original = regular_path(source / name)
        target = regular_path(temporary / name)
        target.parent.mkdir(parents=True, exist_ok=True)
        regular_path(target.parent)
        shutil.copyfile(original, target, follow_symlinks=False)
    verify(source, temporary, expected)
    regular_path(temporary)
    regular_path(final)
    if final.exists():
        raise Rejected("ASSET_DESTINATION_APPEARED_PARTIAL_PRESERVED")
    temporary.rename(final)
    verify(source, regular_path(final), expected)
    return {"status": "STAGED", "bundle": str(final), "manifest_sha256": expected}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--destination-root", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        result = stage(args.bundle, args.destination_root)
    except (Rejected, OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
        result = {"status": "BLOCKED", "code": str(error) if isinstance(error, Rejected) else "ASSET_STAGE_FAILED_PARTIAL_PRESERVED"}
    print(json.dumps(result))
    return 2 if result["status"] == "BLOCKED" else 0


if __name__ == "__main__":
    sys.exit(main())
