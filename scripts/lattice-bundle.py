"""Build and verify a local Windows dependency bundle from explicit software roots.

Never copies a database, user configuration, credentials or a WSL distribution.
The current reviewed WSL system remains an explicit external platform requirement.
"""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import sys

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("customer", Path(__file__).with_name("lattice-customer-runtime.py"))
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)
SCHEMA = "lattice.windows-dependency-bundle.v1"
MAX_FILES = 50_000
MAX_BYTES = 3_000_000_000


def sha(path, *, checked=False):
    if not checked:
        M.regular(path)
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def files_in(root, excluded=()):
    M.regular(root, directory=True)
    excluded = {name.casefold() for name in excluded}
    pending = [root]
    while pending:
        directory = pending.pop()
        with os.scandir(directory) as entries:
            for entry in entries:
                if entry.name.casefold() in excluded:
                    continue
                info = entry.stat(follow_symlinks=False)
                if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
                    raise M.Rejected("PATH_REDIRECTION_REJECTED")
                path = Path(entry.path)
                if stat.S_ISDIR(info.st_mode):
                    pending.append(path)
                elif stat.S_ISREG(info.st_mode):
                    yield path, info.st_size
                else:
                    raise M.Rejected("BUNDLE_FILE_TYPE_REJECTED")


def inventory(root):
    M.regular(root, directory=True)
    result = {}
    size = 0
    for path, length in files_in(root):
        relative = path.relative_to(root).as_posix()
        if relative == "bundle.json":
            continue
        size += length
        if len(result) >= MAX_FILES or size > MAX_BYTES:
            raise M.Rejected("BUNDLE_CAPACITY_REJECTED")
        result[relative] = {"sha256": sha(path, checked=True), "bytes": length}
    return dict(sorted(result.items())), size


def verify(root, expected):
    M.regular(root, directory=True)
    manifest = M.regular(root / "bundle.json")
    if sha(manifest) != expected:
        raise M.Rejected("BUNDLE_MANIFEST_DIGEST_REJECTED")
    data = json.loads(manifest.read_bytes())
    if data["schema"] != SCHEMA or data["platform"] != "windows-x86_64":
        raise M.Rejected("BUNDLE_MANIFEST_REJECTED")
    for name in data["files"]:
        path = PurePosixPath(name)
        if path.is_absolute() or ".." in path.parts or "\\" in name or ":" in name or path.as_posix() != name:
            raise M.Rejected("BUNDLE_PATH_REJECTED")
    actual, size = inventory(root)
    if actual != data["files"] or size != data["total_bytes"]:
        raise M.Rejected("BUNDLE_CONTENT_CHANGED")
    return data


def copy_tree(source, target, excluded=(), budget=None):
    M.regular(source, directory=True)
    budget = budget if budget is not None else {"files": 0, "bytes": 0}
    for original, length in files_in(source, excluded):
        budget["files"] += 1
        budget["bytes"] += length
        if budget["files"] > MAX_FILES or budget["bytes"] > MAX_BYTES:
            raise M.Rejected("BUNDLE_CAPACITY_REJECTED")
        copied = target / original.relative_to(source)
        copied.parent.mkdir(parents=True, exist_ok=True)
        expected = sha(original, checked=True)
        shutil.copyfile(original, copied)
        if sha(copied, checked=True) != expected or sha(original, checked=True) != expected:
            raise M.Rejected("BUNDLE_SOURCE_CHANGED")


def build(root, runtime, runtime_sha, postgres, python, git, graphify):
    if sha(runtime) != runtime_sha:
        raise M.Rejected("RUNTIME_DIGEST_MISMATCH")
    if not root.is_absolute():
        raise M.Rejected("ABSOLUTE_PATH_REQUIRED")
    M.CONFIG.regular_path(root)
    root = root.parent.resolve(strict=True) / root.name
    sources = []
    for source in (postgres, python, git, graphify):
        M.regular(source, directory=True)
        source = source.resolve(strict=True)
        sources.append(source)
        if root == source or root.is_relative_to(source) or source.is_relative_to(root):
            raise M.Rejected("BUNDLE_SOURCE_OUTPUT_OVERLAP")
    postgres, python, git, graphify = sources
    M.private_new_root(root)
    (root / "bin").mkdir()
    shutil.copyfile(runtime, root / "bin/latticed.exe")
    for name in (*M.SCRIPTS, "lattice-bundle.py"):
        shutil.copyfile(Path(__file__).with_name(name), root / "bin" / name)
    budget = {"files": len(M.SCRIPTS) + 2, "bytes": sum(path.stat().st_size for path in (root / "bin").iterdir())}
    # Explicit software subtrees; PostgreSQL data, Git global config, Python
    # site-packages, user homes, installer state and credentials are never inputs.
    for name in ("bin", "lib", "share"):
        copy_tree(postgres / name, root / "postgres" / name, budget=budget)
    for name in ("server_license.txt", "commandlinetools_3rd_party_licenses.txt"):
        shutil.copyfile(M.regular(postgres / name), root / "postgres" / name)
    for name in ("DLLs", "Lib", "tcl"):
        copy_tree(python / name, root / "python" / name, ("site-packages", "__pycache__"), budget)
    for name in ("python.exe", "python3.dll", "python312.dll", "vcruntime140.dll", "vcruntime140_1.dll", "LICENSE.txt"):
        shutil.copyfile(M.regular(python / name), root / "python" / name)
    for name in ("cmd", "mingw64", "usr"):
        copy_tree(git / name, root / "git" / name, ("etc", "__pycache__"), budget)
    shutil.copyfile(M.regular(git / "LICENSE.txt"), root / "git/LICENSE.txt")
    copy_tree(graphify, root / "graphify", budget=budget)
    files, size = inventory(root)
    if files["bin/latticed.exe"]["sha256"] != runtime_sha:
        raise M.Rejected("BUNDLE_RUNTIME_CHANGED")
    data = {"schema": SCHEMA, "platform": "windows-x86_64", "distribution_status": "LOCAL_CANDIDATE",
            "files": files, "total_bytes": size, "runtime_sha256": runtime_sha,
            "bundled": ["LATTICE Runtime and launchers", "PostgreSQL software and licenses", "CPython 3.12 standard library and native DLLs", "Git software and license", "Graphify reviewed payload"],
            "external_requirements": ["Reviewed WSL launcher and Ubuntu 26.04 system", "Pinned Python 3.14.4 and bubblewrap 0.11.1 within Ubuntu", "Explicit Node executable for local result import", "Codex client and its existing account authorization"],
            "full_dependency_portability": "NOT_VERIFIED", "hermes": "TASK_ONLY_DEFERRED"}
    (root / "bundle.json").write_bytes(M.CONFIG.json_bytes(data))
    digest = sha(root / "bundle.json")
    verify(root, digest)
    return {"status": "LOCAL_BUNDLE_VERIFIED", "path": str(root), "sha256": digest,
            "file_count": len(files), "bytes": size, "external_requirements": data["external_requirements"]}


def install(root, expected, state, source, wsl):
    M.CONFIG.regular_path(root)
    root = root.resolve(strict=True)
    for path in (state, source):
        if not path.is_absolute():
            raise M.Rejected("ABSOLUTE_PATH_REQUIRED")
        M.CONFIG.regular_path(path)
        canonical = path.parent.resolve(strict=True) / path.name
        if canonical == root or canonical.is_relative_to(root) or root.is_relative_to(canonical):
            raise M.Rejected("BUNDLE_CUSTOMER_DATA_OVERLAP")
    bundle = verify(root, expected)
    if Path(sys.executable).resolve() != (root / "python/python.exe").resolve():
        raise M.Rejected("USE_BUNDLED_PYTHON_FOR_INSTALL")
    if not sys.flags.isolated or not sys.flags.no_site or not sys.dont_write_bytecode:
        raise M.Rejected("BUNDLED_PYTHON_REQUIRES_I_B_S")
    return M.prepare(state, root / "bin/latticed.exe", bundle["runtime_sha256"], root / "postgres/bin",
                     root / "git/cmd/git.exe", root / "graphify", source, wsl,
                     {**{str(root / name): entry["sha256"] for name, entry in bundle["files"].items()},
                      str(root / "bundle.json"): expected}, root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "verify", "install"))
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--sha256")
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--runtime-sha256")
    parser.add_argument("--postgres", type=Path)
    parser.add_argument("--python", type=Path)
    parser.add_argument("--git", type=Path)
    parser.add_argument("--graphify", type=Path)
    parser.add_argument("--state", type=Path)
    parser.add_argument("--graph-source", type=Path)
    parser.add_argument("--wsl", type=Path)
    args = parser.parse_args()
    if args.action == "build":
        if not all((args.runtime, args.runtime_sha256, args.postgres, args.python, args.git, args.graphify)):
            raise M.Rejected("BUNDLE_BUILD_ARGUMENTS_REQUIRED")
        result = build(args.bundle, args.runtime, args.runtime_sha256, args.postgres, args.python, args.git, args.graphify)
    elif args.action == "install":
        if not all((args.sha256, args.state, args.graph_source, args.wsl)):
            raise M.Rejected("BUNDLE_INSTALL_ARGUMENTS_REQUIRED")
        result = install(args.bundle, args.sha256, args.state, args.graph_source, args.wsl)
    else:
        data = verify(args.bundle, args.sha256)
        result = {"status": "LOCAL_BUNDLE_VERIFIED", "files": len(data["files"]), "bytes": data["total_bytes"]}
    print(json.dumps(result))


if __name__ == "__main__":
    try:
        main()
    except (M.Rejected, OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({"status": "BLOCKED", "code": str(error) if isinstance(error, M.Rejected) else "BUNDLE_IO_OR_MANIFEST_REJECTED"}))
        sys.exit(2)
