"""Recoverable local Runtime/launcher updates for a DPAPI-owned installation.

Old files and database data are retained. Rollback is permitted only after the
previous binary verifies the current database; this never restores stale data.
"""
from __future__ import annotations
import base64
import importlib.util
import json
import os
from pathlib import Path
import shutil
import uuid

SPEC = importlib.util.spec_from_file_location("customer", Path(__file__).with_name("lattice-customer-runtime.py"))
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)
SCHEMA = "lattice.runtime-update.v1"
PENDING = "update.pending.dpapi"


def encoded(data):
    return base64.b64encode(data).decode("ascii")


def decoded(data):
    return base64.b64decode(data, validate=True)


def pair(root):
    return {name: encoded(M.regular(root / name).read_bytes())
            for name in ("installation.json", "credentials.dpapi")}


def validate_pair(root, value):
    public = decoded(value["installation.json"])
    secret = json.loads(M.dpapi(decoded(value["credentials.dpapi"]), decrypt=True))
    config = json.loads(public)
    if (secret["installation_sha256"] != M.CONFIG.digest(public)
            or config["schema"] != M.SCHEMA or Path(config["root"]) != root):
        raise M.Rejected("UPDATE_IDENTITY_REJECTED")
    dependencies = M.verify_dependency_file_set(config)
    for path, digest in config["files"].items():
        if Path(path) in dependencies:
            continue
        if M.file_digest(Path(path)) != digest:
            raise M.Rejected("UPDATE_COMPONENT_CHANGED")
    M.platform_for_config(config)
    offline = M.checked([M.pg(config, "pg_controldata"), str(root / "cluster")], "UPDATE_CLUSTER_UNAVAILABLE")
    if config["system_id"] != M.control_identifier(offline):
        raise M.Rejected("UPDATE_CLUSTER_IDENTITY_REJECTED")
    return config, secret["password"]


def probe(config, password):
    requests = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "runtime-update-check", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "lattice_runtime_status", "arguments": {}}}]
    result = M.invoke([config["runtime"]], env=M.environment(config, password),
                      input_text="".join(json.dumps(r) + "\n" for r in requests), timeout=120)
    if result.returncode:
        raise M.Rejected("UPDATE_RUNTIME_COMPATIBILITY_REJECTED")
    replies = [json.loads(line) for line in result.stdout.splitlines()]
    if ([r.get("id") for r in replies] != [1, 2] or any("error" in r or r.get("result", {}).get("isError") for r in replies)
            or not isinstance(replies[1].get("result", {}).get("structuredContent"), dict)):
        raise M.Rejected("UPDATE_RUNTIME_COMPATIBILITY_REJECTED")
    return replies[1]["result"]["structuredContent"]


def journal(root):
    pending = M.regular(root / PENDING).read_bytes()
    value = json.loads(M.dpapi(pending, decrypt=True))
    if value["schema"] != SCHEMA or Path(value["root"]) != root or value["action"] not in ("update", "rollback"):
        raise M.Rejected("UPDATE_JOURNAL_REJECTED")
    if len(value["id"]) != 32 or any(c not in "0123456789abcdef" for c in value["id"]):
        raise M.Rejected("UPDATE_JOURNAL_REJECTED")
    before, before_password = validate_pair(root, value["before"])
    after, after_password = validate_pair(root, value["after"])
    for key in ("root", "run_id", "system_id", "port", "postgres_bin", "python", "git", "node", "graph_source", "graphify_runtime", "graphify_platform", "wsl", "dependency_root"):
        if before.get(key) != after.get(key):
            raise M.Rejected("UPDATE_SCOPE_REJECTED")
    if before_password != after_password:
        raise M.Rejected("UPDATE_SCOPE_REJECTED")
    current = pair(root)
    if any(current[name] not in (value["before"][name], value["after"][name]) for name in current):
        raise M.Rejected("UPDATE_CONCURRENT_CHANGE_PRESERVED")
    return value, after, after_password


def finish(root):
    value, after, password = journal(root)
    M.start(after, password)  # A pending update must also recover after host restart.
    if value["action"] == "update":
        M.runtime_action(after, password, "--postgres-bootstrap")
    status = probe(after, password)
    # Recheck the sealed journal and files after native schema verification.
    journal(root)
    for name, data in value["after"].items():
        M.CONFIG.atomic_write(root / name, decoded(data))
    if pair(root) != value["after"]:
        raise M.Rejected("UPDATE_READBACK_REJECTED")
    history = root / "updates" / (value["id"] + ".dpapi")
    pending = (root / PENDING).read_bytes()
    if history.exists():
        if M.regular(history).read_bytes() != pending:
            raise M.Rejected("UPDATE_HISTORY_CONFLICT")
    else:
        temporary = history.with_name(history.name + ".tmp-" + uuid.uuid4().hex)
        with temporary.open("xb") as stream:
            stream.write(pending)
            stream.flush()
            os.fsync(stream.fileno())
        # Windows rename refuses an existing target; never overwrite unknown history.
        os.rename(temporary, history)
    if M.regular(history).read_bytes() != pending:
        raise M.Rejected("UPDATE_HISTORY_READBACK_REJECTED")
    (root / PENDING).unlink()
    config, _ = M.load(root)
    return {"schema": SCHEMA, "status": "RUNTIME_" + value["action"].upper() + "_VERIFIED",
            "update_id": value["id"], "runtime_sha256": config["files"][config["runtime"]],
            "launcher": config.get("launcher"), "database": "PRESERVED", "runtime_status": status}


def apply(root, runtime=None, expected=None, *, recover=False, rollback=None):
    M.regular(root, directory=True)
    if recover:
        # The DPAPI journal validates ownership even when the two active files
        # were interrupted between atomic replacements.
        journal(root)
    else:
        M.load(root)
    with M.runtime_lease(root, exclusive=True), M.CONFIG.manager_lock(root / ".operations"):
        if recover:
            return finish(root)
        config, password = M.load(root)
        M.verify_running(config, password)
        before = pair(root)
        update_id = uuid.uuid4().hex
        if rollback:
            if len(rollback) != 32 or any(c not in "0123456789abcdef" for c in rollback):
                raise M.Rejected("UPDATE_ID_REJECTED")
            history = json.loads(M.dpapi(M.regular(root / "updates" / (rollback + ".dpapi")).read_bytes(), decrypt=True))
            if history["root"] != str(root) or history["after"] != before:
                raise M.Rejected("UPDATE_ROLLBACK_HEAD_CHANGED")
            after = history["before"]
            previous, previous_password = validate_pair(root, after)
            probe(previous, previous_password)  # Never roll a binary over incompatible data.
            action = "rollback"
        else:
            if runtime is None or M.file_digest(runtime) != expected:
                raise M.Rejected("RUNTIME_DIGEST_MISMATCH")
            updates = root / "updates"
            if updates.exists():
                M.regular(updates, directory=True)
            else:
                updates.mkdir()
            version = updates / update_id
            version.mkdir()  # New immutable version only; old binaries stay intact.
            paths = [(runtime, version / "latticed.exe")]
            paths += [(Path(__file__).with_name(name), version / name) for name in M.SCRIPTS]
            files = dict(config["files"])
            for source, target in paths:
                before_hash = M.file_digest(source)
                shutil.copyfile(source, target)
                if M.file_digest(target) != before_hash or M.file_digest(source) != before_hash:
                    raise M.Rejected("UPDATE_COPY_CHANGED")
                files[str(target)] = before_hash
            if files[str(version / "latticed.exe")] != expected:
                raise M.Rejected("RUNTIME_DIGEST_MISMATCH")
            new = {**config, "files": files, "runtime": str(version / "latticed.exe"),
                   "launcher": str(version / M.SCRIPTS[0]), "update_id": update_id}
            public = M.CONFIG.json_bytes(new)
            after = {"installation.json": encoded(public), "credentials.dpapi": encoded(M.dpapi(M.CONFIG.json_bytes({
                "password": password, "installation_sha256": M.CONFIG.digest(public)})))}
            validate_pair(root, after)
            probe(new, password)
            action = "update"
        if pair(root) != before:
            raise M.Rejected("UPDATE_CONCURRENT_CHANGE_PRESERVED")
        value = {"schema": SCHEMA, "root": str(root), "id": update_id, "action": action, "before": before, "after": after}
        M.CONFIG.atomic_write(root / PENDING, M.dpapi(M.CONFIG.json_bytes(value)))
        return finish(root)
