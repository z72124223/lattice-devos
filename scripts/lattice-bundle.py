"""Build and verify a local Windows dependency bundle from explicit software roots.

Never copies a database, user configuration, credentials or an installed WSL distribution.
An optional pinned official image can provision a new private WSL2 distribution.
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
import io
import urllib.request
import zipfile

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("customer", Path(__file__).with_name("lattice-customer-runtime.py"))
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)
SCHEMA = "lattice.windows-dependency-bundle.v1"
MAX_FILES = 50_000
MAX_BYTES = 3_000_000_000
NODE_VERSION = "24.16.0"
NODE_ZIP_SHA256 = "edaca9bd58ec8e92037dac4e877d52f6b8f430b81c18b57e264b4e2fb111cd56"
NODE_EXE_SHA256 = "b3094d0b49f9ad602262a9921551737bb97637c05dd357a06ae98188d7290aa3"
NODE_LICENSE_SHA256 = "8efdacdc1cfa3460aeb7fe98e3c54337b971d5da70e6eee292b73b981acb220c"
WSL_IMAGE_SHA256 = "48d56724b5c8e60f24893e83e73bbb58c60b3ca22fba3da977075420acd54104"
NODE_URL = "https://nodejs.org/download/release/v24.16.0/node-v24.16.0-win-x64.zip"
VC_REDIST_SOURCE = "VC/Redist/MSVC/14.44.35112/x64/Microsoft.VC143.CRT"
VC_LICENSE_SHA256 = "2f66b86a00e8d9833789897ce23d05a4a2dbea370cf39c8c1098dbc17d0e7bdc"
VC_REDIST_LIST_SHA256 = "da53b097e02b08e0fc69706102a60bc384fe756426ae4dc4a855e96f95cb2b9c"
VC_REDIST_DOCUMENTS = {
    "application_local": "https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files",
    "redistribution_list": "https://aka.ms/vs/17/redist.txt",
    "redistribution_terms": "https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution",
    "license": "https://visualstudio.microsoft.com/license-terms/vs2022-ga-diagnosticbuildtools/",
    "supplemental_license_document": "https://visualstudio.microsoft.com/wp-content/uploads/2024/03/Visual-Studio-2022-Diagnostic-Build-Tools-Agent-License_Update-March-2024_EN.docx",
}


def vc_redist_provenance():
    return {"schema": "lattice.vc-runtime.v1", "product": "Microsoft Visual C++ 2022 x64 release CRT",
            "version": M.VC_RUNTIME_VERSION, "source_subdirectory": VC_REDIST_SOURCE,
            "license_classification": "Microsoft proprietary redistributable, subject to licensed Visual Studio distribution terms",
            "documents": VC_REDIST_DOCUMENTS, "files": M.VC_RUNTIME_FILES,
            "license_sha256": VC_LICENSE_SHA256, "redistribution_list_sha256": VC_REDIST_LIST_SHA256}


def verify_vc_documents(license_path, redist_list):
    if license_path is None or redist_list is None:
        raise M.Rejected("VC_RUNTIME_LICENSE_DOCUMENTS_REQUIRED")
    if sha(license_path) != VC_LICENSE_SHA256 or sha(redist_list) != VC_REDIST_LIST_SHA256:
        raise M.Rejected("VC_RUNTIME_LICENSE_DOCUMENTS_REJECTED")


def bundle_vc_redist(root, source, license_path, redist_list):
    """Copy exact official release bytes app-locally; never read System32."""
    verify_vc_documents(license_path, redist_list)
    M.copy_vc_runtime(source, root / "bin", required=True)
    M.copy_vc_runtime(source, root / "postgres/bin", required=True)
    notices = root / "licenses/visual-cpp-runtime"
    notices.mkdir(parents=True)
    shutil.copyfile(license_path, notices / "Visual-Studio-2022-Build-Tools-License.docx")
    shutil.copyfile(redist_list, notices / "Redist.txt")
    verify_vc_documents(notices / "Visual-Studio-2022-Build-Tools-License.docx", notices / "Redist.txt")
    (notices / "provenance.json").write_bytes(M.CONFIG.json_bytes(vc_redist_provenance()))
    (notices / "NOTICE.txt").write_text(
        "Microsoft Visual C++ 2022 x64 release CRT\n"
        "Copyright Microsoft Corporation. All rights reserved.\n"
        "Unmodified app-local binaries distributed under the applicable Microsoft Visual Studio terms.\n"
        "These Microsoft binaries are not covered by the LATTICE project license.\n"
        + "\n".join(name + ": " + url for name, url in VC_REDIST_DOCUMENTS.items()) + "\n",
        encoding="utf-8")


def supply_node(root):
    """Exact upstream archive; extract only runtime/license, never npm or installers."""
    M.CONFIG.regular_path(root)
    if root.exists() or not root.is_absolute() or not root.parent.is_dir():
        raise M.Rejected("FRESH_NODE_SUPPLY_REQUIRED")
    with urllib.request.urlopen(NODE_URL, timeout=60) as response:
        data = response.read(100_000_001)
    if len(data) > 100_000_000 or hashlib.sha256(data).hexdigest() != NODE_ZIP_SHA256:
        raise M.Rejected("NODE_ARCHIVE_DIGEST_REJECTED")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        contents = {}
        for name in ("node.exe", "LICENSE"):
            entry = archive.getinfo("node-v24.16.0-win-x64/" + name)
            if entry.file_size > 150_000_000:
                raise M.Rejected("NODE_ARCHIVE_CAPACITY_REJECTED")
            contents[name] = archive.read(entry)
    if (hashlib.sha256(contents["node.exe"]).hexdigest() != NODE_EXE_SHA256
            or hashlib.sha256(contents["LICENSE"]).hexdigest() != NODE_LICENSE_SHA256):
        raise M.Rejected("NODE_EXECUTABLE_DIGEST_REJECTED")
    M.private_new_root(root)
    for name, body in contents.items():
        (root / name).write_bytes(body)
    provenance = {"schema": "lattice.node-supply.v1", "version": NODE_VERSION, "source": NODE_URL,
                  "archive_sha256": NODE_ZIP_SHA256, "files": {name: sha(root / name) for name in contents}}
    (root / "provenance.json").write_bytes(M.CONFIG.json_bytes(provenance))
    verify_node(root)
    return {"status": "NODE_SUPPLY_VERIFIED", "path": str(root), "version": NODE_VERSION, "sha256": NODE_EXE_SHA256}


def verify_node(root):
    provenance = json.loads(M.regular(root / "provenance.json").read_bytes())
    if (provenance.get("schema") != "lattice.node-supply.v1" or provenance.get("version") != NODE_VERSION
            or provenance.get("source") != NODE_URL or provenance.get("archive_sha256") != NODE_ZIP_SHA256
            or set(provenance.get("files", {})) != {"node.exe", "LICENSE"}
            or provenance["files"]["node.exe"] != NODE_EXE_SHA256
            or provenance["files"]["LICENSE"] != NODE_LICENSE_SHA256
            or any(sha(root / name) != expected for name, expected in provenance["files"].items())):
        raise M.Rejected("NODE_SUPPLY_REJECTED")
    return provenance


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
    crt_directories = (root / "bin", root / "postgres/bin")
    if "vc_runtime" not in data:
        for directory in crt_directories:
            if directory.exists() and M.vc_runtime_files(directory):
                raise M.Rejected("VC_RUNTIME_PROVENANCE_REQUIRED")
    if "vc_runtime" in data:
        if data["vc_runtime"] != vc_redist_provenance():
            raise M.Rejected("VC_RUNTIME_PROVENANCE_REJECTED")
        for directory in crt_directories:
            M.vc_runtime_files(directory, required=True)
        verify_vc_documents(root / "licenses/visual-cpp-runtime/Visual-Studio-2022-Build-Tools-License.docx",
                            root / "licenses/visual-cpp-runtime/Redist.txt")
        if json.loads((root / "licenses/visual-cpp-runtime/provenance.json").read_bytes()) != data["vc_runtime"]:
            raise M.Rejected("VC_RUNTIME_PROVENANCE_REJECTED")
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


def build(root, runtime, runtime_sha, postgres, python, git, graphify, node=None, archive=None,
          vc_redist=None, vc_license=None, vc_redist_list=None):
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
    if node is None:
        raise M.Rejected("NODE_SUPPLY_REQUIRED")
    M.regular(node, directory=True)
    node = node.resolve(strict=True)
    if root == node or root.is_relative_to(node) or node.is_relative_to(root):
        raise M.Rejected("BUNDLE_SOURCE_OUTPUT_OVERLAP")
    verify_node(node)
    if archive is not None:
        if M.file_digest(archive) != WSL_IMAGE_SHA256:
            raise M.Rejected("WSL_IMAGE_DIGEST_REJECTED")
    if vc_redist is None:
        raise M.Rejected("VC_RUNTIME_SUPPLY_REQUIRED")
    M.regular(vc_redist, directory=True)
    vc_redist = vc_redist.resolve(strict=True)
    if root == vc_redist or root.is_relative_to(vc_redist) or vc_redist.is_relative_to(root):
        raise M.Rejected("BUNDLE_SOURCE_OUTPUT_OVERLAP")
    M.vc_runtime_files(vc_redist, required=True)
    verify_vc_documents(vc_license, vc_redist_list)
    M.private_new_root(root)
    (root / "bin").mkdir()
    shutil.copyfile(runtime, root / "bin/latticed.exe")
    for name in M.SCRIPTS:
        shutil.copyfile(Path(__file__).with_name(name), root / "bin" / name)
    budget = {"files": len(M.SCRIPTS) + 1, "bytes": sum(path.stat().st_size for path in (root / "bin").iterdir())}
    # Explicit software subtrees; PostgreSQL data, Git global config, Python
    # site-packages, user homes, installer state and credentials are never inputs.
    for name in ("bin", "lib", "share"):
        copy_tree(postgres / name, root / "postgres" / name, budget=budget)
    for name in ("server_license.txt", "commandlinetools_3rd_party_licenses.txt"):
        shutil.copyfile(M.regular(postgres / name), root / "postgres" / name)
    bundle_vc_redist(root, vc_redist, vc_license, vc_redist_list)
    for name in ("DLLs", "Lib", "tcl"):
        copy_tree(python / name, root / "python" / name, ("site-packages", "__pycache__"), budget)
    for name in ("python.exe", "python3.dll", "python312.dll", "vcruntime140.dll", "vcruntime140_1.dll", "LICENSE.txt"):
        shutil.copyfile(M.regular(python / name), root / "python" / name)
    for name in ("cmd", "mingw64", "usr"):
        copy_tree(git / name, root / "git" / name, ("etc", "__pycache__"), budget)
    shutil.copyfile(M.regular(git / "LICENSE.txt"), root / "git/LICENSE.txt")
    copy_tree(graphify, root / "graphify", ("__pycache__",), budget)
    (root / "node").mkdir()
    for name in ("node.exe", "LICENSE", "provenance.json"):
        shutil.copyfile(node / name, root / "node" / name)
    verify_node(root / "node")
    if archive is not None:
        (root / "platform").mkdir()
        image = root / "platform/ubuntu-26.04.1-wsl-amd64.wsl"
        shutil.copyfile(archive, image)
        if M.file_digest(image) != WSL_IMAGE_SHA256:
            raise M.Rejected("WSL_IMAGE_COPY_CHANGED")
    files, size = inventory(root)
    if files["bin/latticed.exe"]["sha256"] != runtime_sha:
        raise M.Rejected("BUNDLE_RUNTIME_CHANGED")
    data = {"schema": SCHEMA, "platform": "windows-x86_64", "distribution_status": "LOCAL_CANDIDATE",
            "files": files, "total_bytes": size, "runtime_sha256": runtime_sha, "vc_runtime": vc_redist_provenance(),
            "bundled": ["LATTICE Runtime and launchers", "PostgreSQL software and licenses", "CPython 3.12 standard library and native DLLs", "Git software and license", "Graphify reviewed payload", "Node.js 24.16.0 runtime and license", "Microsoft Visual C++ 2022 x64 app-local CRT 14.44.35211.0"],
            "external_requirements": (["Enabled Windows WSL2 with virtualization and an authenticated Microsoft WSL launcher"] if archive is not None else ["Reviewed WSL launcher and Ubuntu system"])
                + ["Codex client and its existing account authorization"],
            "full_dependency_portability": "NOT_VERIFIED", "cores": ["control", "postgresql", "graphify"]}
    if archive is not None:
        data["bundled"].append("Pinned official Ubuntu 26.04.1 WSL image with Python 3.14 and bubblewrap")
    (root / "bundle.json").write_bytes(M.CONFIG.json_bytes(data))
    digest = sha(root / "bundle.json")
    verify(root, digest)
    return {"status": "LOCAL_BUNDLE_VERIFIED", "path": str(root), "sha256": digest,
            "file_count": len(files), "bytes": size, "external_requirements": data["external_requirements"]}


def install(root, expected, state, source, wsl, graphify_platform=None):
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
                      str(root / "bundle.json"): expected}, root,
                     node=root / "node/node.exe" if "node/node.exe" in bundle["files"] else None,
                     graphify_platform=graphify_platform)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "verify", "install", "supply-node"))
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--node", type=Path)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--vc-redist", type=Path, help="reviewed Microsoft.VC143.CRT x64 release directory (never System32)")
    parser.add_argument("--vc-license", type=Path, help="pinned Microsoft Build Tools supplemental license document")
    parser.add_argument("--vc-redist-list", type=Path, help="pinned unmodified Visual Studio Redist.txt")
    parser.add_argument("--sha256")
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--runtime-sha256")
    parser.add_argument("--postgres", type=Path)
    parser.add_argument("--python", type=Path)
    parser.add_argument("--git", type=Path)
    parser.add_argument("--graphify", type=Path)
    parser.add_argument("--graphify-platform", type=Path)
    parser.add_argument("--state", type=Path)
    parser.add_argument("--graph-source", type=Path)
    parser.add_argument("--wsl", type=Path)
    args = parser.parse_args()
    if args.action == "supply-node":
        if args.node is None:
            raise M.Rejected("NODE_SUPPLY_PATH_REQUIRED")
        result = supply_node(args.node)
    elif args.bundle is None:
        raise M.Rejected("BUNDLE_PATH_REQUIRED")
    elif args.action == "build":
        if not all((args.runtime, args.runtime_sha256, args.postgres, args.python, args.git, args.graphify, args.node)):
            raise M.Rejected("BUNDLE_BUILD_ARGUMENTS_REQUIRED")
        result = build(args.bundle, args.runtime, args.runtime_sha256, args.postgres, args.python, args.git, args.graphify,
                       args.node, args.archive, args.vc_redist, args.vc_license, args.vc_redist_list)
    elif args.action == "install":
        if not all((args.sha256, args.state, args.graph_source, args.wsl)):
            raise M.Rejected("BUNDLE_INSTALL_ARGUMENTS_REQUIRED")
        result = install(args.bundle, args.sha256, args.state, args.graph_source, args.wsl, args.graphify_platform)
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
