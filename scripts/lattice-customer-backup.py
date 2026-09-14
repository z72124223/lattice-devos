"""Authenticated customer data backup and fresh-directory physical restore.

Only an owned stopped cluster and explicitly registered standalone Git projects
are captured. Dependencies, Windows credentials, Codex settings and WSL images
are supplied afresh at the destination. Source data is always retained.
"""
from __future__ import annotations
import argparse
import importlib.util
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import secrets
import shutil
import sys

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("bundle", Path(__file__).with_name("lattice-bundle.py"))
B = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(B)
M = B.M
SCHEMA = "lattice.customer-backup.v1"
MAX_FILES = 100000
MAX_BYTES = 10 * 1024**3
MAX_METADATA = 32 * 1024 * 1024


def bounded_json(value):
    data = M.CONFIG.json_bytes(value)
    if len(data) > MAX_METADATA: raise M.Rejected("BACKUP_METADATA_BOUND_EXCEEDED")
    return data


def fresh(path):
    M.CONFIG.regular_path(path)
    if not path.is_absolute() or path.exists() or not path.parent.is_dir():
        raise M.Rejected("BACKUP_FRESH_DIRECTORY_REQUIRED")
    return path.parent.resolve() / path.name


def relative(name):
    path = PurePosixPath(name)
    if (not isinstance(name, str) or not name or path.is_absolute() or str(path) != name
            or any(p in (".", "..") or ':' in p or '\\' in p or p.endswith((' ', '.'))
                   or re.match(r"(?i)^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)", p) for p in path.parts)):
        raise M.Rejected("BACKUP_PATH_REJECTED")
    return path


def inventory(root):
    M.regular(root, directory=True)
    result = {}; size = 0
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in dirs: M.regular(Path(directory) / name, directory=True)
        for name in files:
            path = M.regular(Path(directory) / name)
            key = path.relative_to(root).as_posix(); relative(key)
            length = path.stat().st_size; size += length
            if len(result) >= MAX_FILES or size > MAX_BYTES:
                raise M.Rejected("BACKUP_BOUND_EXCEEDED")
            result[key] = {"sha256": M.file_digest(path), "bytes": length}
    return result


def crypto(node, action, key, files):
    script = Path(__file__).with_name("lattice-backup-crypto.mjs")
    M.regular(script); M.regular(key)
    if action == "encrypt":
        files = [item if "expected_sha256" in item and "expected_bytes" in item else
                 {"expected_sha256": M.file_digest(Path(item["input"])), "expected_bytes": Path(item["input"]).stat().st_size, **item} for item in files]
    response = M.invoke([str(node), str(script)], env=M.closed_environment(),
                        input_text=json.dumps({"action": action, "key": str(key), "files": files}), timeout=1800)
    if response.returncode or json.loads(response.stdout).get("status") != "AUTHENTICATED_FILES_COMPLETE":
        raise M.Rejected("BACKUP_AUTHENTICATION_REJECTED")


def registry(config, password, project, action="inspect", extra=()):
    command = [config["runtime"], "project-registry-" + action, "--postgres-host", "127.0.0.1",
               "--postgres-port", str(config["port"]), "--postgres-run-id", config["run_id"], "--project-id", project, *extra]
    env = M.environment(config, password)
    if action in ("restore", "restore-observe"): env["LATTICE_CUSTOMER_RESTORE_ROOT"] = config["root"]
    result = M.invoke(command, env=env, timeout=120)
    if result.returncode: raise M.Rejected("BACKUP_REGISTRY_" + action.upper() + "_REJECTED")
    return json.loads(result.stdout)


def verify_snapshot_heads(roots, snapshots, observations):
    for identifier, observation in observations.items():
        prefix = "projects/" + identifier
        data = M.regular(roots[prefix] / ".git/HEAD").read_bytes()
        saved = snapshots[prefix].get(".git/HEAD")
        if (saved != {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
                or data.decode("utf-8").strip() != "ref: " + observation["accepted_ref"]):
            raise M.Rejected("BACKUP_PROJECT_HEAD_CHANGED")


def backup(state, destination, key_root, evidence_roots=()):
    config, _ = M.load(state)
    node = Path(config.get("node") or "missing-node")
    if M.file_digest(node) != config["files"].get(str(node)):
        raise M.Rejected("BACKUP_PINNED_NODE_REQUIRED")
    destination, key_root = fresh(destination), fresh(key_root)
    roots = {name: state / name for name in ("cluster", "graph-work", "dependencies")}
    catalog = json.loads(M.regular(state / "projects.json").read_bytes())
    if catalog.get("schema") != "lattice.customer-project-catalog.v1":
        raise M.Rejected("BACKUP_CATALOG_REJECTED")
    for row in catalog["projects"]:
        identifier = row["id"]
        if not re.fullmatch(r"[0-9a-f-]{36}", identifier): raise M.Rejected("BACKUP_PROJECT_ID_REJECTED")
        project = M.regular(Path(row["canonical_path"]), directory=True)
        M.regular(project / ".git", directory=True)
        if (project / ".git/objects/info/alternates").exists(): raise M.Rejected("BACKUP_EXTERNAL_GIT_OBJECTS_REJECTED")
        roots["projects/" + identifier] = project
    for index, path in enumerate(evidence_roots):
        roots["evidence/" + str(index)] = M.regular(path, directory=True)
    for output in (destination, key_root):
        for source in (state, *roots.values()):
            if output == source or output.is_relative_to(source) or source.is_relative_to(output):
                raise M.Rejected("BACKUP_SOURCE_OUTPUT_OVERLAP")
    if destination.is_relative_to(key_root) or key_root.is_relative_to(destination):
        raise M.Rejected("BACKUP_KEY_MUST_BE_SEPARATE")
    with M.runtime_lease(state, exclusive=True), M.CONFIG.manager_lock(state / ".operations"):
        config, password = M.load(state)
        if json.loads(M.regular(state / "projects.json").read_bytes()) != catalog:
            raise M.Rejected("BACKUP_CATALOG_CHANGED")
        was_running = M.running(config)
        if not was_running: M.start(config, password)
        try:
            observations = {row["id"]: registry(config, password, row["id"])["current"] for row in catalog["projects"]}
            graph_configuration = M.runtime_action(config, password, "--graphify-configuration") if config.get("graph_source") else None
            if any(not row["active"] or not row["fresh_matches_accepted"] for row in observations.values()):
                raise M.Rejected("BACKUP_ACTIVE_PROJECT_IDENTITY_REQUIRED")
        finally:
            if not was_running:
                M.verify_running(config, password)
                M.checked([M.pg(config, "pg_ctl"), "-D", str(state / "cluster"), "-m", "fast", "-w", "-t", "30", "stop"], "BACKUP_STOP_REJECTED", quiet=True)
        if M.running(config):
            M.verify_running(config, password)
            M.checked([M.pg(config, "pg_ctl"), "-D", str(state / "cluster"), "-m", "fast", "-w", "-t", "30", "stop"], "BACKUP_STOP_REJECTED", quiet=True)
        temporary = None
        try:
            if M.running(config): raise M.Rejected("BACKUP_CLUSTER_STILL_RUNNING")
            if any((state / "cluster/pg_tblspc").iterdir()): raise M.Rejected("BACKUP_EXTERNAL_TABLESPACES_REJECTED")
            M.private_new_root(destination); M.private_new_root(key_root)
            (destination / "objects").mkdir()
            key = key_root / "recovery.key"; key.write_bytes(secrets.token_bytes(32))
            snapshots = {prefix: inventory(root) for prefix, root in roots.items()}
            verify_snapshot_heads(roots, snapshots, observations)
            directories = []
            for prefix, root in roots.items():
                directories.append(prefix)
                for directory, dirs, _ in os.walk(root, followlinks=False):
                    for name in dirs:
                        path = M.regular(Path(directory) / name, directory=True)
                        directories.append(prefix + "/" + path.relative_to(root).as_posix())
                        if len(directories) > MAX_FILES: raise M.Rejected("BACKUP_BOUND_EXCEEDED")
            entries = {}; jobs = []
            for prefix, files in snapshots.items():
                for name, info in files.items():
                    target = "objects/" + str(len(entries)) + ".gcm"
                    entries[prefix + "/" + name] = {**info, "object": target}
                    jobs.append({"input": str(roots[prefix] / name), "output": str(destination / target), "expected_sha256": info["sha256"], "expected_bytes": info["bytes"]})
            if len(entries) > MAX_FILES or sum(x["bytes"] for x in entries.values()) > MAX_BYTES:
                raise M.Rejected("BACKUP_BOUND_EXCEEDED")
            crypto(node, "encrypt", key, jobs)
            if any(inventory(root) != snapshots[prefix] for prefix, root in roots.items()):
                raise M.Rejected("BACKUP_SOURCE_CHANGED")
            objects = {name: {"sha256": M.file_digest(destination / name), "bytes": (destination / name).stat().st_size} for name in (item["object"] for item in entries.values())}
            metadata = {"schema": SCHEMA, "source": config, "password": password, "catalog": catalog,
                        "entries": entries, "directories": directories, "objects": objects, "registry": observations, "graph_configuration": graph_configuration,
                        "evidence_roots": [str(p) for p in evidence_roots]}
            temporary = destination / "metadata.private.tmp"
            temporary.write_bytes(bounded_json(metadata))
            crypto(node, "encrypt", key, [{"input": str(temporary), "output": str(destination / "metadata.gcm")}])
            temporary.unlink(); temporary = None
            public = {"schema": SCHEMA, "encryption": "AES-256-GCM", "metadata_sha256": M.file_digest(destination / "metadata.gcm"),
                      "file_count": len(entries), "plain_bytes": sum(x["bytes"] for x in entries.values())}
            (destination / "backup.json").write_bytes(M.CONFIG.json_bytes(public))
            return {"status": "ENCRYPTED_BACKUP_VERIFIED", "backup": str(destination), "sha256": M.file_digest(destination / "backup.json"),
                    "key_file": str(key), "files": len(entries), "source": "PRESERVED", "restore": "NOT_VERIFIED"}
        finally:
            if temporary is not None: temporary.unlink(missing_ok=True)
            if was_running: M.start(config, password)


def decrypt_backup(backup_root, expected, key, destination, node):
    """Authenticate all bytes into a new private staging tree before any DB process."""
    public_file = M.regular(backup_root / "backup.json")
    if public_file.stat().st_size > 65536: raise M.Rejected("BACKUP_METADATA_BOUND_EXCEEDED")
    if M.file_digest(public_file) != expected: raise M.Rejected("BACKUP_MANIFEST_DIGEST_REJECTED")
    public = json.loads(public_file.read_bytes())
    if public.get("schema") != SCHEMA or public.get("encryption") != "AES-256-GCM": raise M.Rejected("BACKUP_SCHEMA_REJECTED")
    if M.file_digest(backup_root / "metadata.gcm") != public["metadata_sha256"]: raise M.Rejected("BACKUP_METADATA_CHANGED")
    if (backup_root / "metadata.gcm").stat().st_size > MAX_METADATA + 28: raise M.Rejected("BACKUP_METADATA_BOUND_EXCEEDED")
    destination = fresh(destination); M.private_new_root(destination)
    (destination / "restore.in-progress").write_bytes(b"lattice.customer-restore.v1\n")
    metadata_file = destination / "metadata.private.tmp"
    try:
        crypto(node, "decrypt", key, [{"input": str(backup_root / "metadata.gcm"), "output": str(metadata_file)}])
        if metadata_file.stat().st_size > MAX_METADATA: raise M.Rejected("BACKUP_METADATA_BOUND_EXCEEDED")
        metadata = json.loads(metadata_file.read_bytes())
    finally:
        metadata_file.unlink(missing_ok=True)
    entries, objects = metadata["entries"], metadata["objects"]
    if metadata.get("schema") != SCHEMA or len(entries) != public["file_count"] or len(entries) > MAX_FILES or sum(x["bytes"] for x in entries.values()) != public["plain_bytes"] or public["plain_bytes"] > MAX_BYTES:
        raise M.Rejected("BACKUP_BOUND_EXCEEDED")
    seen = set(); jobs = []
    if len(metadata["directories"]) > MAX_FILES: raise M.Rejected("BACKUP_BOUND_EXCEEDED")
    for name in metadata["directories"]:
        path = relative(name)
        if path.parts[0] not in ("cluster", "projects", "evidence", "graph-work", "dependencies"): raise M.Rejected("BACKUP_PATH_REJECTED")
        (destination / name).mkdir(parents=True, exist_ok=True)
    for name, entry in entries.items():
        path = relative(name)
        if path.parts[0] not in ("cluster", "projects", "evidence", "graph-work", "dependencies") or name.casefold() in seen:
            raise M.Rejected("BACKUP_PATH_REJECTED")
        seen.add(name.casefold())
        object_name = entry["object"]
        if not re.fullmatch(r"objects/[0-9]+\.gcm", object_name): raise M.Rejected("BACKUP_OBJECT_REJECTED")
        source = M.regular(backup_root / object_name)
        if source.stat().st_size != objects[object_name]["bytes"] or source.stat().st_size != entry["bytes"] + 28 or M.file_digest(source) != objects[object_name]["sha256"]:
            raise M.Rejected("BACKUP_OBJECT_CHANGED")
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        jobs.append({"input": str(source), "output": str(target)})
    crypto(node, "decrypt", key, jobs)
    for name, entry in entries.items():
        path = destination / name
        if path.stat().st_size != entry["bytes"] or M.file_digest(path) != entry["sha256"]:
            raise M.Rejected("BACKUP_PLAINTEXT_CHANGED")
    return metadata


def restore(backup_root, expected, key, state, bundle, bundle_sha, platform, runtime=None, runtime_sha=None):
    manifest = B.verify(bundle, bundle_sha)
    if Path(sys.executable).resolve() != (bundle / "python/python.exe").resolve():
        raise M.Rejected("USE_BUNDLED_PYTHON_FOR_RESTORE")
    if not sys.flags.isolated or not sys.flags.no_site or not sys.dont_write_bytecode:
        raise M.Rejected("BUNDLED_PYTHON_REQUIRES_I_B_S")
    state = fresh(state)
    for source in (backup_root, bundle, key.parent, platform):
        if state.is_relative_to(source) or source.is_relative_to(state): raise M.Rejected("RESTORE_SOURCE_OUTPUT_OVERLAP")
    wsl = Path(os.environ["SystemRoot"]) / "System32/wsl.exe"
    M.platform_for_config({"graphify_platform": str(platform), "wsl": str(wsl)}, preparing=True)
    runtime = runtime or bundle / "bin/latticed.exe"
    runtime_sha = runtime_sha or manifest["runtime_sha256"]
    if M.file_digest(runtime) != runtime_sha: raise M.Rejected("RUNTIME_DIGEST_MISMATCH")
    metadata = decrypt_backup(backup_root, expected, key, state, bundle / "node/node.exe")
    old = metadata["source"]
    if old["files"].get(str(Path(old["postgres_bin"]) / "postgres.exe")) != manifest["files"]["postgres/bin/postgres.exe"]["sha256"]:
        raise M.Rejected("RESTORE_POSTGRES_BINARY_INCOMPATIBLE")
    if M.control_identifier(M.checked([str(bundle / "postgres/bin/pg_controldata.exe"), str(state / "cluster")], "RESTORE_CLUSTER_CONTROL_REJECTED")) != old["system_id"]:
        raise M.Rejected("RESTORE_CLUSTER_IDENTITY_REJECTED")
    for name in ("postgresql.conf", "postgresql.auto.conf", "pg_hba.conf", "pg_ident.conf"):
        if M.file_digest(state / "cluster" / name) != old["files"].get(str(Path(old["root"]) / "cluster" / name)):
            raise M.Rejected("RESTORE_POSTGRES_CONFIG_CHANGED")
    port = M.free_port()
    with (state / "cluster/postgresql.conf").open("a", encoding="utf-8") as stream:
        stream.write("\n# Verified customer restore to a new owned location\nlisten_addresses='127.0.0.1'\nport=" + str(port) + "\n")
    for name in ("bin", "graph-work", "dependencies", "restore-proofs"): (state / name).mkdir(exist_ok=True)
    target = state / "bin/latticed.exe"; shutil.copyfile(runtime, target)
    for name in M.SCRIPTS:
        shutil.copyfile(Path(__file__).with_name(name), state / "bin" / name)
    catalog = metadata["catalog"]; mapping = {}
    for row in catalog["projects"]:
        location = state / "projects" / row["id"]
        M.regular(location / ".git", directory=True)
        mapping[Path(row["canonical_path"])] = str(location)
        row["canonical_path"] = str(location)
        proof = {"schema": "lattice.customer-project-restore-proof.v1", "project_id": row["id"],
                 "accepted_observation_digest": metadata["registry"][row["id"]]["accepted_observation_digest"],
                 "files": {name.removeprefix("projects/" + row["id"] + "/"): {"sha256": entry["sha256"], "bytes": entry["bytes"]}
                           for name, entry in metadata["entries"].items() if name.startswith("projects/" + row["id"] + "/")}}
        (state / "restore-proofs" / (row["id"] + ".json")).write_bytes(M.CONFIG.json_bytes(proof))
    (state / "projects.json").write_bytes(M.CONFIG.json_bytes(catalog))
    graph_source = mapping.get(Path(old["graph_source"])) if old.get("graph_source") else None
    if old.get("graph_source") and not graph_source: raise M.Rejected("RESTORE_GRAPH_SOURCE_NOT_REGISTERED")
    files = {str(bundle / name): entry["sha256"] for name, entry in manifest["files"].items()}
    paths = [target, *(state / "bin" / name for name in M.SCRIPTS), state / "cluster/postgresql.conf", state / "cluster/postgresql.auto.conf",
             state / "cluster/pg_hba.conf", state / "cluster/pg_ident.conf", platform / "platform.json", wsl]
    files.update({str(path): M.file_digest(path) for path in paths}); files[str(bundle / "bundle.json")] = bundle_sha
    config = {"schema": M.SCHEMA, "root": str(state), "run_id": old["run_id"], "system_id": old["system_id"], "port": port,
              "runtime": str(target), "python": str(bundle / "python/python.exe"), "node": str(bundle / "node/node.exe"),
              "postgres_bin": str(bundle / "postgres/bin"), "git": str(bundle / "git/cmd/git.exe"), "dependency_root": str(bundle),
              "graphify_platform": str(platform), "graphify_runtime": str(bundle / "graphify"), "graph_source": graph_source,
              "wsl": str(wsl), "files": files,
              "retained_graph_configurations": list(dict.fromkeys([
                  *((metadata.get("graph_configuration") or {}).get("readable_configurations") or []),
                  *(([(metadata["graph_configuration"]["configuration_sha256"])] if metadata.get("graph_configuration") else [])),
                  *(old.get("retained_graph_configurations") or []),
                  *([old["retained_graph_configuration"]] if old.get("retained_graph_configuration") else [])]))}
    if files[str(target)] != runtime_sha: raise M.Rejected("RESTORE_RUNTIME_COPY_CHANGED")
    public = M.CONFIG.json_bytes(config)
    (state / "installation.json").write_bytes(public)
    (state / "credentials.dpapi").write_bytes(M.dpapi(M.CONFIG.json_bytes({"password": metadata["password"], "installation_sha256": M.CONFIG.digest(public)})))
    journal = {"schema": "lattice.customer-restore.v1", "root": str(state), "installation_sha256": M.CONFIG.digest(public),
               "backup_sha256": expected, "catalog_sha256": M.file_digest(state / "projects.json"),
               "proofs": {row["id"]: M.file_digest(state / "restore-proofs" / (row["id"] + ".json")) for row in catalog["projects"]},
               "original_registry": metadata["registry"], "requests": {}, "results": {}}
    (state / "restore.pending.dpapi").write_bytes(M.dpapi(M.CONFIG.json_bytes(journal)))
    return finalize_restore(state)


def finalize_restore(state):
    config, password = M.load(state, allow_restore=True)
    with M.runtime_lease(state, exclusive=True), M.CONFIG.manager_lock(state / ".operations"):
        path = state / "restore.pending.dpapi"
        journal = json.loads(M.dpapi(M.regular(path).read_bytes(), decrypt=True))
        if journal.get("schema") != "lattice.customer-restore.v1" or Path(journal["root"]) != state or journal["installation_sha256"] != M.file_digest(state / "installation.json") or journal["catalog_sha256"] != M.file_digest(state / "projects.json"):
            raise M.Rejected("RESTORE_JOURNAL_IDENTITY_REJECTED")
        if not (state / "restore.in-progress").exists():
            M.CONFIG.atomic_write(state / "restore.in-progress", b"lattice.customer-restore.v1\n")
        M.start(config, password)
        spec = importlib.util.spec_from_file_location("update_probe", Path(__file__).with_name("lattice-runtime-update.py"))
        update = importlib.util.module_from_spec(spec); spec.loader.exec_module(update)
        update.probe(config, password)
        for project, expected in journal["proofs"].items():
            proof = state / "restore-proofs" / (project + ".json")
            if M.file_digest(proof) != expected: raise M.Rejected("RESTORE_PROOF_CHANGED")
            if project not in journal["requests"]:
                # Native project admission records the physical observation; a
                # reconciliation-required response is expected, never suppressed as success.
                current = registry(config, password, project, "restore-observe", ["--restore-proof", str(proof), "--restore-proof-sha256", expected])["current"]
                if current["active"] or not current["fresh_matches_pending"]: raise M.Rejected("RESTORE_PENDING_IDENTITY_REQUIRED")
                journal["requests"][project] = ["--expected-revision", str(current["registry_revision"]), "--expected-receipt-digest", current["receipt_digest"],
                                                  "--pending-observation-digest", current["pending_observation_digest"], "--restore-proof", str(proof), "--restore-proof-sha256", expected]
                M.CONFIG.atomic_write(path, M.dpapi(M.CONFIG.json_bytes(journal)))
            outcome = registry(config, password, project, "restore", journal["requests"][project])
            if outcome["status"] not in ("APPLIED", "REPLAYED") or not outcome["current"]["active"] or not outcome["current"]["fresh_matches_accepted"]:
                raise M.Rejected("RESTORE_REGISTRY_NOT_VERIFIED")
            journal["results"][project] = outcome
            M.CONFIG.atomic_write(path, M.dpapi(M.CONFIG.json_bytes(journal)))
        report = {"schema": "lattice.customer-restore.v1", "status": "CUSTOMER_RESTORE_IDENTITY_VERIFIED", "run_id": config["run_id"],
                  "system_id": config["system_id"], "backup_sha256": journal["backup_sha256"], "projects": journal["results"],
                  "workflow": "READBACK_REQUIRED", "cross_user_os_acceptance": "NOT_VERIFIED"}
        M.CONFIG.atomic_write(state / "restore-receipt.json", M.CONFIG.json_bytes(report))
        M.CONFIG.atomic_write(state / "ready.json", M.CONFIG.json_bytes({"schema": M.SCHEMA, "run_id": config["run_id"], "system_id": config["system_id"]}))
        (state / "restore.in-progress").unlink(missing_ok=True)
        path.unlink()
        return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("backup", "restore", "finalize-restore"))
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--key-root", type=Path)
    parser.add_argument("--backup", type=Path); parser.add_argument("--sha256")
    parser.add_argument("--key", type=Path); parser.add_argument("--bundle", type=Path); parser.add_argument("--bundle-sha256")
    parser.add_argument("--graphify-platform", type=Path)
    parser.add_argument("--runtime", type=Path); parser.add_argument("--runtime-sha256")
    parser.add_argument("--evidence-root", type=Path, action="append", default=[])
    args = parser.parse_args()
    if args.action == "backup":
        if not args.output or not args.key_root: raise M.Rejected("BACKUP_ARGUMENTS_REQUIRED")
        result = backup(args.state, args.output, args.key_root, args.evidence_root)
    elif args.action == "restore":
        if not all((args.backup, args.sha256, args.key, args.bundle, args.bundle_sha256, args.graphify_platform)) or bool(args.runtime) != bool(args.runtime_sha256): raise M.Rejected("RESTORE_ARGUMENTS_REQUIRED")
        result = restore(args.backup, args.sha256, args.key, args.state, args.bundle, args.bundle_sha256, args.graphify_platform, args.runtime, args.runtime_sha256)
    else: result = finalize_restore(args.state)
    print(json.dumps(result))


if __name__ == "__main__":
    try: main()
    except (M.Rejected, OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({"status": "BLOCKED", "code": str(error) if isinstance(error, M.Rejected) else "CUSTOMER_BACKUP_REJECTED", "assets": "RETAINED"})); sys.exit(2)
