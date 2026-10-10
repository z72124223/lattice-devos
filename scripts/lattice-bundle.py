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
MAINTENANCE_SCHEMA = "lattice.windows-project-purge-maintenance.v1"
MAINTENANCE_PROFILE = "offline-project-purge"
PROJECT_PURGE_SCHEMA = "lattice.project-purge.bundle.v1"
PROJECT_PURGE_BINARY = "bin/lattice-project-purge.exe"
PROJECT_PURGE_ENTRYPOINT = "apps/lattice-control/src/project-purge-client.mjs"
PROJECT_PURGE_FILES = (
    PROJECT_PURGE_ENTRYPOINT,
    "apps/lattice-control/src/project-purge.mjs",
    "apps/lattice-control/src/project-purge-files.mjs",
    "apps/lattice-control/src/project-purge-sqlite.mjs",
    "apps/lattice-control/src/project-purge-report.mjs",
    "apps/lattice-control/src/project-purge-code-graph.mjs",
    "apps/lattice-control/src/code-graph.mjs",
    "apps/lattice-control/src/code-graph-model.mjs",
    "apps/lattice-control/src/lattice-runtime-health.mjs",
    "apps/lattice-control/src/store.mjs",
    "apps/lattice-control/src/database-path.mjs",
    "apps/lattice-control/data-scope-contract.json",
)
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


def vc_metadata():
    notice = (
        "Microsoft Visual C++ 2022 x64 release CRT\n"
        "Copyright Microsoft Corporation. All rights reserved.\n"
        "Unmodified app-local binaries distributed under the applicable Microsoft Visual Studio terms.\n"
        "These Microsoft binaries are not covered by the LATTICE project license.\n"
        + "\n".join(name + ": " + url for name, url in VC_REDIST_DOCUMENTS.items()) + "\n")
    return {"provenance.json": M.CONFIG.json_bytes(vc_redist_provenance()), "NOTICE.txt": notice.encode("utf-8")}


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
    for name, body in vc_metadata().items():
        (notices / name).write_bytes(body)


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


def maintenance_files():
    return {PROJECT_PURGE_BINARY, *PROJECT_PURGE_FILES, "bin/latticed.exe",
            *("node/" + name for name in ("node.exe", "LICENSE", "provenance.json")),
            *("bin/" + name for name in M.VC_RUNTIME_FILES),
            *("licenses/visual-cpp-runtime/" + name for name in
              ("Visual-Studio-2022-Build-Tools-License.docx", "Redist.txt", "provenance.json", "NOTICE.txt"))}


def verify(root, expected, *, maintenance=False):
    M.regular(root, directory=True)
    manifest = M.regular(root / "bundle.json")
    if sha(manifest) != expected:
        raise M.Rejected("BUNDLE_MANIFEST_DIGEST_REJECTED")
    data = json.loads(manifest.read_bytes())
    if data["schema"] != (MAINTENANCE_SCHEMA if maintenance else SCHEMA) or data["platform"] != "windows-x86_64":
        raise M.Rejected("BUNDLE_MANIFEST_REJECTED")
    if maintenance:
        if (set(data) != {"schema", "profile", "platform", "distribution_status", "files", "total_bytes",
                         "runtime_sha256", "vc_runtime", "project_purge"}
                or data["profile"] != MAINTENANCE_PROFILE or data["distribution_status"] != "LOCAL_CANDIDATE"):
            raise M.Rejected("MAINTENANCE_MANIFEST_REJECTED")
        if set(data["files"]) != maintenance_files():
            raise M.Rejected("MAINTENANCE_FILE_SET_REJECTED")
    for name in data["files"]:
        path = PurePosixPath(name)
        if path.is_absolute() or ".." in path.parts or "\\" in name or ":" in name or path.as_posix() != name:
            raise M.Rejected("BUNDLE_PATH_REJECTED")
    actual, size = inventory(root)
    if actual != data["files"] or size != data["total_bytes"]:
        raise M.Rejected("BUNDLE_CONTENT_CHANGED")
    if "project_purge" in data:
        feature = data["project_purge"]
        required = {PROJECT_PURGE_BINARY, *PROJECT_PURGE_FILES}
        if (not isinstance(feature, dict)
                or set(feature) != {"schema", "entrypoint", "binary", "runtime_sha256", "files"}
                or feature["schema"] != PROJECT_PURGE_SCHEMA
                or feature["entrypoint"] != PROJECT_PURGE_ENTRYPOINT
                or feature["binary"] != PROJECT_PURGE_BINARY
                or not isinstance(feature["files"], dict)
                or set(feature["files"]) != required
                or not required.issubset(actual)
                or any(feature["files"][name] != actual[name]["sha256"] for name in required)
                or feature["runtime_sha256"] != data.get("runtime_sha256")
                or actual.get("bin/latticed.exe", {}).get("sha256") != feature["runtime_sha256"]
                or "node/node.exe" not in actual):
            raise M.Rejected("PROJECT_PURGE_BUNDLE_REJECTED")
        verify_node(root / "node")
    elif any(name in actual for name in (PROJECT_PURGE_BINARY, PROJECT_PURGE_ENTRYPOINT)):
        raise M.Rejected("PROJECT_PURGE_CAPABILITY_REQUIRED")
    crt_directories = (root / "bin",) if maintenance else (root / "bin", root / "postgres/bin")
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


def verify_maintenance(root, expected):
    return verify(root, expected, maintenance=True)


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


def project_purge_payload(binary, expected, source):
    """Select only the reviewed maintenance executable and its software closure."""
    if binary is None and expected is None and source is None:
        return None
    if binary is None or expected is None or source is None:
        raise M.Rejected("PROJECT_PURGE_SUPPLY_REQUIRED")
    if not binary.is_absolute() or not source.is_absolute():
        raise M.Rejected("ABSOLUTE_PATH_REQUIRED")
    M.regular(source, directory=True)
    if not binary.exists():
        raise M.Rejected("PROJECT_PURGE_BINARY_REQUIRED")
    if sha(binary) != expected:
        raise M.Rejected("PROJECT_PURGE_BINARY_DIGEST_REJECTED")
    result = {PROJECT_PURGE_BINARY: (binary, expected)}
    for name in PROJECT_PURGE_FILES:
        original = source / name
        if not original.exists():
            raise M.Rejected("PROJECT_PURGE_SOURCE_REQUIRED")
        result[name] = (original, sha(original))
    return result


def copy_payload(root, payload, budget):
    for name, (original, expected) in payload.items():
        if sha(original) != expected:
            raise M.Rejected("BUNDLE_SOURCE_CHANGED")
        budget["files"] += 1
        budget["bytes"] += original.stat().st_size
        if budget["files"] > MAX_FILES or budget["bytes"] > MAX_BYTES:
            raise M.Rejected("BUNDLE_CAPACITY_REJECTED")
        copied = root / name
        copied.parent.mkdir(parents=True, exist_ok=True)
        if copied.exists():
            raise M.Rejected("PROJECT_PURGE_OUTPUT_EXISTS")
        shutil.copyfile(original, copied)
        if sha(copied) != expected or sha(original) != expected:
            raise M.Rejected("BUNDLE_SOURCE_CHANGED")


def copy_project_purge(root, payload, runtime_sha, budget):
    copy_payload(root, payload, budget)
    return {"schema": PROJECT_PURGE_SCHEMA, "entrypoint": PROJECT_PURGE_ENTRYPOINT,
            "binary": PROJECT_PURGE_BINARY, "runtime_sha256": runtime_sha,
            "files": {name: value[1] for name, value in sorted(payload.items())}}


def build_maintenance(root, runtime, runtime_sha, node, vc_redist, vc_license, vc_redist_list,
                      project_purge_binary, project_purge_sha256, project_purge_source):
    """An offline maintenance payload, not an installable dependency bundle."""
    if not all((runtime, runtime_sha, node, vc_redist, vc_license, vc_redist_list,
                project_purge_binary, project_purge_sha256, project_purge_source)):
        raise M.Rejected("MAINTENANCE_SUPPLY_REQUIRED")
    if not root.is_absolute():
        raise M.Rejected("ABSOLUTE_PATH_REQUIRED")
    M.CONFIG.regular_path(root)
    if root.exists() or not root.parent.is_dir():
        raise M.Rejected("FRESH_CUSTOMER_DIRECTORY_REQUIRED")
    root = root.parent.resolve(strict=True) / root.name
    for source in (runtime, node, vc_redist, vc_license, vc_redist_list, project_purge_binary, project_purge_source):
        M.regular(source, directory=source in (node, vc_redist, project_purge_source))
        canonical = source.resolve(strict=True)
        if root == canonical or root.is_relative_to(canonical) or canonical.is_relative_to(root):
            raise M.Rejected("BUNDLE_SOURCE_OUTPUT_OVERLAP")
    if sha(runtime) != runtime_sha:
        raise M.Rejected("RUNTIME_DIGEST_MISMATCH")
    purge = project_purge_payload(project_purge_binary, project_purge_sha256, project_purge_source)
    verify_node(node)
    crt = M.vc_runtime_files(vc_redist, required=True)
    verify_vc_documents(vc_license, vc_redist_list)
    supplies = {"bin/latticed.exe": (runtime, runtime_sha),
                **{"node/" + name: (node / name, sha(node / name)) for name in ("node.exe", "LICENSE", "provenance.json")},
                **{"bin/" + path.name: (path, digest) for path, digest in crt.items()},
                "licenses/visual-cpp-runtime/Visual-Studio-2022-Build-Tools-License.docx": (vc_license, VC_LICENSE_SHA256),
                "licenses/visual-cpp-runtime/Redist.txt": (vc_redist_list, VC_REDIST_LIST_SHA256)}
    metadata = {"licenses/visual-cpp-runtime/" + name: body for name, body in vc_metadata().items()}
    size = sum(path.stat().st_size for path, _ in {**purge, **supplies}.values()) + sum(map(len, metadata.values()))
    if len(purge) + len(supplies) + len(metadata) > MAX_FILES or size > MAX_BYTES:
        raise M.Rejected("BUNDLE_CAPACITY_REJECTED")
    M.private_new_root(root)
    budget = {"files": len(metadata), "bytes": sum(map(len, metadata.values()))}
    feature = copy_project_purge(root, purge, runtime_sha, budget)
    copy_payload(root, supplies, budget)
    for name, body in metadata.items():
        (root / name).write_bytes(body)
    files, size = inventory(root)
    data = {"schema": MAINTENANCE_SCHEMA, "profile": MAINTENANCE_PROFILE, "platform": "windows-x86_64",
            "distribution_status": "LOCAL_CANDIDATE", "files": files, "total_bytes": size,
            "runtime_sha256": runtime_sha, "vc_runtime": vc_redist_provenance(), "project_purge": feature}
    (root / "bundle.json").write_bytes(M.CONFIG.json_bytes(data))
    digest = sha(root / "bundle.json")
    verify_maintenance(root, digest)
    return {"status": "LOCAL_MAINTENANCE_BUNDLE_VERIFIED", "path": str(root), "sha256": digest,
            "file_count": len(files), "bytes": size}


def build(root, runtime, runtime_sha, postgres, python, git, graphify, node=None, archive=None,
          vc_redist=None, vc_license=None, vc_redist_list=None,
          project_purge_binary=None, project_purge_sha256=None, project_purge_source=None):
    if sha(runtime) != runtime_sha:
        raise M.Rejected("RUNTIME_DIGEST_MISMATCH")
    purge_payload = project_purge_payload(project_purge_binary, project_purge_sha256, project_purge_source)
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
    purge_feature = copy_project_purge(root, purge_payload, runtime_sha, budget) if purge_payload is not None else None
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
    if purge_feature is not None:
        data["project_purge"] = purge_feature
        data["bundled"].append("LATTICE offline project purge executable and verified CLI software closure")
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
    parser.add_argument("action", choices=("build", "verify", "build-maintenance", "verify-maintenance", "install", "supply-node"))
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--node", type=Path)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--vc-redist", type=Path, help="reviewed Microsoft.VC143.CRT x64 release directory (never System32)")
    parser.add_argument("--vc-license", type=Path, help="pinned Microsoft Build Tools supplemental license document")
    parser.add_argument("--vc-redist-list", type=Path, help="pinned unmodified Visual Studio Redist.txt")
    parser.add_argument("--sha256")
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--runtime-sha256")
    parser.add_argument("--project-purge-binary", type=Path, help="opt in to the offline purge capability with its reviewed executable")
    parser.add_argument("--project-purge-sha256", help="expected SHA-256 of the purge executable; required with purge capability")
    parser.add_argument("--project-purge-source", type=Path, help="repository root supplying the exact purge CLI dependency closure")
    parser.add_argument("--postgres", type=Path)
    parser.add_argument("--python", type=Path)
    parser.add_argument("--git", type=Path)
    parser.add_argument("--graphify", type=Path)
    parser.add_argument("--graphify-platform", type=Path)
    parser.add_argument("--state", type=Path)
    parser.add_argument("--graph-source", type=Path)
    parser.add_argument("--wsl", type=Path)
    args = parser.parse_args()
    if args.action not in ("build", "build-maintenance") and any((args.project_purge_binary, args.project_purge_sha256, args.project_purge_source)):
        raise M.Rejected("PROJECT_PURGE_BUILD_OPTION_ONLY")
    if args.action == "supply-node":
        if args.node is None:
            raise M.Rejected("NODE_SUPPLY_PATH_REQUIRED")
        result = supply_node(args.node)
    elif args.bundle is None:
        raise M.Rejected("BUNDLE_PATH_REQUIRED")
    elif args.action == "build-maintenance":
        if any((args.postgres, args.python, args.git, args.graphify, args.archive, args.graphify_platform,
                args.state, args.graph_source, args.wsl)):
            raise M.Rejected("MAINTENANCE_OPTION_REJECTED")
        result = build_maintenance(args.bundle, args.runtime, args.runtime_sha256, args.node,
                                   args.vc_redist, args.vc_license, args.vc_redist_list,
                                   args.project_purge_binary, args.project_purge_sha256, args.project_purge_source)
    elif args.action == "build":
        if not all((args.runtime, args.runtime_sha256, args.postgres, args.python, args.git, args.graphify, args.node)):
            raise M.Rejected("BUNDLE_BUILD_ARGUMENTS_REQUIRED")
        result = build(args.bundle, args.runtime, args.runtime_sha256, args.postgres, args.python, args.git, args.graphify,
                       args.node, args.archive, args.vc_redist, args.vc_license, args.vc_redist_list,
                       args.project_purge_binary, args.project_purge_sha256, args.project_purge_source)
    elif args.action == "install":
        if not all((args.sha256, args.state, args.graph_source, args.wsl)):
            raise M.Rejected("BUNDLE_INSTALL_ARGUMENTS_REQUIRED")
        result = install(args.bundle, args.sha256, args.state, args.graph_source, args.wsl, args.graphify_platform)
    else:
        maintenance = args.action == "verify-maintenance"
        data = verify(args.bundle, args.sha256, maintenance=maintenance)
        result = {"status": "LOCAL_MAINTENANCE_BUNDLE_VERIFIED" if maintenance else "LOCAL_BUNDLE_VERIFIED",
                  "files": len(data["files"]), "bytes": data["total_bytes"]}
    print(json.dumps(result))


if __name__ == "__main__":
    try:
        main()
    except (M.Rejected, OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({"status": "BLOCKED", "code": str(error) if isinstance(error, M.Rejected) else "BUNDLE_IO_OR_MANIFEST_REJECTED"}))
        sys.exit(2)
