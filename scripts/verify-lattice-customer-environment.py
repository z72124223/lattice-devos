"""Live clean/custom/NTFS-denied configuration acceptance for one owned fixture.

Run with the trusted bundle's Python and provide its independently obtained digest.
Requires an already restored completed fixture; never creates business work or users.
Only a new evidence directory's test config permissions are changed.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import winreg


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_bundle(root, expected):
    manifest = root / "bundle.json"
    if manifest.stat().st_size > 32 * 1024 * 1024 or digest(manifest) != expected:
        raise RuntimeError("BUNDLE_MANIFEST_DIGEST_REJECTED")
    data = json.loads(manifest.read_bytes())
    names = ("lattice-customer-runtime.py", "lattice-mcp-config.py", "lattice-runtime-update.py",
             "lattice-wsl-platform.py", "lattice-bundle.py", "lattice-customer-backup.py", "lattice-backup-crypto.mjs")
    for name in names:
        if digest(root / "bin" / name) != data["files"]["bin/" + name]["sha256"]:
            raise RuntimeError("BUNDLE_SCRIPT_DIGEST_REJECTED")
    spec = importlib.util.spec_from_file_location("acceptance_bundle", root / "bin/lattice-bundle.py")
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    module.verify(root, expected)
    if Path(sys.executable).resolve() != (root / "python/python.exe").resolve():
        raise RuntimeError("USE_BUNDLED_PYTHON")
    return module.M


def snapshot(m, entry, args, evidence, phase):
    calls = [("lattice_task_status", {"task_ref": args.completed_task}),
             ("lattice_control_snapshot", {"project_id": args.project_id}),
             ("lattice_control_snapshot", {"decisions": {"mode": "current", "scope": args.project_id, "limit": 32}}),
             ("lattice_code_relations", {"project_id": args.project_id, "commit": args.commit, "query": args.query, "limit": 32})]
    requests = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "customer-environment-acceptance", "version": "1"}}},
                {"jsonrpc": "2.0", "method": "notifications/initialized"}]
    requests += [{"jsonrpc": "2.0", "id": i + 2, "method": "tools/call", "params": {"name": name, "arguments": value}} for i, (name, value) in enumerate(calls)]
    evidence["phase"] = phase
    child = subprocess.Popen([entry["command"], *entry["args"]], env=m.closed_environment(),
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             text=True, encoding="utf-8", errors="replace", creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        stdout, stderr = child.communicate("".join(json.dumps(row) + "\n" for row in requests), timeout=240)
        result = subprocess.CompletedProcess(child.args, child.returncode, stdout, stderr)
    except subprocess.TimeoutExpired as error:
        for name in ("stdout", "stderr"):
            value = getattr(error, name, None) or b""
            evidence[phase + "_" + name] = (value.decode("utf-8", errors="replace") if isinstance(value, bytes) else value)[-65536:]
        # Kill only this newly created acceptance process and its descendants.
        m.invoke([str(Path(os.environ["SystemRoot"]) / "System32/taskkill.exe"), "/PID", str(child.pid), "/T", "/F"], quiet=True)
        child.wait(timeout=10)
        raise RuntimeError("CONFIGURED_MCP_TIMEOUT") from None
    evidence[phase + "_process"] = {"returncode": result.returncode, "stdout": result.stdout[-65536:], "stderr": result.stderr[-65536:]}
    if result.returncode: raise RuntimeError("CONFIGURED_MCP_PROCESS_REJECTED")
    replies = [json.loads(line) for line in result.stdout.splitlines()]
    if [row.get("id") for row in replies] != [1, 2, 3, 4, 5] or any("error" in row or row["result"].get("isError") for row in replies):
        raise RuntimeError("CONFIGURED_MCP_FRAMING_OR_TOOL_REJECTED")
    value = dict(zip(("task", "product", "decisions", "graph"), [row["result"]["structuredContent"] for row in replies[1:]]))
    evidence[phase] = value
    verify_bindings(value, args)
    return value


def verify_bindings(value, args):
    task, product, decisions, graph = (value[key] for key in ("task", "product", "decisions", "graph"))
    metadata = [row for row in product["product"]["metadata"] if row["task_ref"] == args.completed_task]
    parent = metadata[0].get("parent_ref") if len(metadata) == 1 else None
    if (task["status"] != "COMPLETED" or not task["result_digest"] or task["project_id"] != args.project_id
            or task["task_ref"] != args.completed_task or product["project"]["id"] != args.project_id
            or decisions["scope"] != args.project_id or not decisions["decisions"] or not parent
            or not any(row["task_ref"] == args.completed_task and row["ledger"] == task for row in product["tasks"])
            or not any(row["task_ref"] == parent and row["ledger"]["project_id"] == args.project_id for row in product["tasks"])
            or not any(row["task_ref"] in (parent, args.completed_task) for row in product["product"]["decisions"])
            or graph["registered_project_id"] != args.project_id or graph["commit"] != args.commit
            or graph["query"] != args.query or not graph["records"] or not graph["source_receipt_digest"]):
        raise RuntimeError("WORKFLOW_AND_RELATION_BINDINGS_REJECTED")


def equivalent(actual, reference):
    # A successful physical restore legitimately changes only the project locator header.
    return (actual["task"] == reference["task"] and actual["decisions"] == reference["decisions"]
            and actual["graph"] == reference["graph"] and actual["product"]["tasks"] == reference["product"]["tasks"]
            and actual["product"]["product"] == reference["product"]["product"])


def evidence_destination(m, output, sources):
    m.CONFIG.regular_path(output)
    if output.exists() or not output.is_absolute() or not output.parent.is_dir():
        raise RuntimeError("FRESH_EVIDENCE_DIRECTORY_REQUIRED")
    output = output.parent.resolve(strict=True) / output.name
    for source in sources:
        source = source.resolve(strict=True)
        if output.is_relative_to(source) or source.is_relative_to(output):
            raise RuntimeError("EVIDENCE_SOURCE_OVERLAP")
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("state", "bundle", "output", "reference"):
        parser.add_argument("--" + name, required=True, type=Path)
    for name in ("bundle-sha256", "project-id", "completed-task", "commit", "query"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    if os.name != "nt" or not sys.flags.isolated or not sys.flags.no_site or not sys.dont_write_bytecode:
        raise RuntimeError("WINDOWS_AND_I_B_S_REQUIRED")
    m = load_bundle(args.bundle, args.bundle_sha256)
    config, password = m.load(args.state)
    catalog = json.loads(m.regular(args.state / "projects.json").read_bytes())
    sources = [args.state, args.bundle, *(Path(row["canonical_path"]) for row in catalog["projects"])]
    if config.get("graph_source"): sources.append(Path(config["graph_source"]))
    args.output = evidence_destination(m, args.output, sources)
    reference = json.loads(args.reference.read_bytes())["after"]
    m.private_new_root(args.output)
    report = {"schema": "lattice.customer-environment-acceptance.v1", "status": "RUNNING",
              "scope": "Actual configured MCP command; Codex App UI and enterprise organization policy are not exercised.",
              "windows_version": str(sys.getwindowsversion()), "runtime_sha256": digest(Path(config["runtime"])),
              "bundle_sha256": args.bundle_sha256, "reference_sha256": digest(args.reference), "run_id": config["run_id"], "cases": []}
    with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Cryptography") as key:
        report["windows_installation_id_sha256"] = hashlib.sha256(winreg.QueryValueEx(key, "MachineGuid")[0].encode()).hexdigest()
    try:
        report["phase"] = "reference_bindings"
        verify_bindings(reference, args)
        report["phase"] = "start"
        m.start(config, password)
        for label, original in (("clean", b""), ("custom", b'model = "customer-model"\r\napproval_policy = "never"\r\n[mcp_servers.other]\r\ncommand = "customer-other"')):
            case = {"case": label, "phase": "connect"}; report["cases"].append(case); report["phase"] = label
            path = args.output / (label + ".toml"); path.write_bytes(original)
            m.operate(args.state, "connect", path)
            entry = m.CONFIG.parse(path.read_bytes())["mcp_servers"]["lattice"]
            before = snapshot(m, entry, args, case, "before")
            if not equivalent(before, reference): raise RuntimeError("ORIGINAL_WORKFLOW_CHANGED")
            case["phase"] = "postgres_stop"
            if m.operate(args.state, "stop")["status"] != "STOPPED": raise RuntimeError("STOP_NOT_VERIFIED")
            case["phase"] = "postgres_start"
            m.operate(args.state, "start")
            after = snapshot(m, entry, args, case, "after")
            if before != after: raise RuntimeError("RESTART_WORKFLOW_CHANGED")
            case["phase"] = "reconnect_and_remove"
            m.operate(args.state, "reconnect", path)
            m.CONFIG.change(path, "remove")
            if path.read_bytes() != original: raise RuntimeError("CUSTOMER_CONFIG_NOT_RESTORED")
            case.update({"phase": "VERIFIED", "configured_command_used": True, "restart_equal": True, "config_restored_exactly": True})
        report["phase"] = "ntfs_write_denied"
        strict = args.output / "ntfs-denied.toml"; strict.write_bytes(b'model = "strict-customer"\n')
        access_copy = args.output / "original-access.txt"; access_copy.write_bytes(b"Owned acceptance ACL source only.\n")
        m.CONFIG.copy_access(strict, access_copy)
        system = Path(os.environ["SystemRoot"]) / "System32"
        identity = m.invoke([str(system / "whoami.exe"), "/user", "/fo", "csv", "/nh"], env=m.closed_environment())
        sid = re.search(r"S-1-5-[0-9-]+", identity.stdout).group()
        original = strict.read_bytes()
        try:
            # Deny write-specific rights; generic W also denies shared standard
            # rights needed by readers and would invalidate a read-preservation check.
            denied = m.invoke([str(system / "icacls.exe"), str(strict), "/deny", "*" + sid + ":(WD,AD,WEA,WA)"], env=m.closed_environment())
            if denied.returncode: raise RuntimeError("OWNED_FIXTURE_ACL_SETUP_FAILED")
            try:
                with strict.open("r+b"): pass
            except PermissionError: pass
            else: raise RuntimeError("NTFS_DENIAL_NOT_EFFECTIVE")
            try: m.operate(args.state, "connect", strict)
            except PermissionError: pass
            else: raise RuntimeError("DENIED_CONFIG_WAS_NOT_REJECTED")
            if strict.read_bytes() != original or m.CONFIG.locations(strict)[0].exists():
                raise RuntimeError("DENIED_CONFIG_MUTATED")
            report["cases"].append({"case": "ntfs_write_denied", "os_denial_observed": True, "connect_denied_before_state_creation": True, "original_bytes_preserved": True, "enterprise_policy": "NOT_VERIFIED"})
        finally:
            m.CONFIG.copy_access(access_copy, strict)
        report["status"] = "CUSTOMER_ENVIRONMENT_CASES_VERIFIED"
        report["phase"] = "VERIFIED"
    except Exception as error:
        report["status"] = "FAILED"; report["failure_type"] = type(error).__name__
        if type(error) is RuntimeError: report["failure_code"] = str(error)
        raise
    finally:
        with (args.output / "acceptance.json").open("x", encoding="utf-8") as stream: json.dump(report, stream, indent=2)
    print(json.dumps({"status": report["status"], "cases": len(report["cases"]), "output": str(args.output / "acceptance.json")}))


if __name__ == "__main__":
    main()
