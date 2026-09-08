"""Explicit live acceptance against one already prepared, isolated customer Runtime.

Requires an existing completed fixture task and saved decisions/relationships.
Stops and starts only that verified installation. Does not create or complete work.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

SPEC = importlib.util.spec_from_file_location("customer", Path(__file__).with_name("lattice-customer-runtime.py"))
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


def snapshot(root, project, completed):
    config, _ = M.load(root)
    calls = [("lattice_task_status", {"task_ref": completed}),
             ("lattice_control_snapshot", {"project_id": project}),
             ("lattice_control_snapshot", {"decisions": {"mode": "current", "scope": project, "limit": 32}})]
    requests = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "customer-restart-acceptance", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"}]
    requests += [{"jsonrpc": "2.0", "id": i + 2, "method": "tools/call", "params": {"name": name, "arguments": args}}
                 for i, (name, args) in enumerate(calls)]
    child = subprocess.Popen([config["python"], "-I", str(root / "bin" / M.SCRIPTS[0]), "serve", "--state", str(root)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8",
        env=M.closed_environment(), creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        stdout, stderr = child.communicate("".join(json.dumps(r) + "\n" for r in requests), timeout=60)
    except subprocess.TimeoutExpired:
        # Only this freshly spawned acceptance process and its own descendants.
        M.invoke([str(Path(M.os.environ["SystemRoot"]) / "System32" / "taskkill.exe"), "/PID", str(child.pid), "/T", "/F"], quiet=True)
        child.wait(timeout=5)
        raise RuntimeError("CUSTOMER_MCP_ACCEPTANCE_TIMEOUT")
    if child.returncode:
        raise RuntimeError("CUSTOMER_MCP_ACCEPTANCE_EXIT_REJECTED")
    replies = [json.loads(line) for line in stdout.splitlines()]
    if [reply.get("id") for reply in replies] != [1, 2, 3, 4]:
        raise RuntimeError("CUSTOMER_MCP_ACCEPTANCE_FRAMING_REJECTED")
    for reply in replies:
        if "error" in reply or reply.get("result", {}).get("isError"):
            raise RuntimeError("CUSTOMER_MCP_ACCEPTANCE_TOOL_REJECTED")
    return {"task": replies[1]["result"]["structuredContent"],
            "product": replies[2]["result"]["structuredContent"],
            "decisions": replies[3]["result"]["structuredContent"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", required=True, type=Path)
    parser.add_argument("--project-id", required=True)
    parser.add_argument("--completed-task", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    config, _ = M.load(args.state)
    if args.output.exists() or not args.output.is_absolute() or not args.output.parent.is_dir():
        raise RuntimeError("FRESH_EVIDENCE_PATH_REQUIRED")
    before = snapshot(args.state, args.project_id, args.completed_task)
    metadata = [row for row in before["product"]["product"]["metadata"] if row["task_ref"] == args.completed_task]
    tasks = before["product"]["tasks"]
    parent = metadata[0].get("parent_ref") if len(metadata) == 1 else None
    if (before["task"]["status"] != "COMPLETED" or not before["task"]["result_digest"]
            or before["task"]["project_id"] != args.project_id
            or before["product"]["project"]["id"] != args.project_id
            or before["decisions"]["scope"] != args.project_id
            or not before["decisions"]["decisions"]
            or not parent
            or not any(row["task_ref"] == args.completed_task and row["ledger"] == before["task"] for row in tasks)
            or not any(row["task_ref"] == parent and row["ledger"]["project_id"] == args.project_id for row in tasks)
            or not any(row["task_ref"] in (parent, args.completed_task) for row in before["product"]["product"]["decisions"])):
        raise RuntimeError("CUSTOMER_WORKFLOW_PREREQUISITES_NOT_VERIFIED")
    stopped = M.operate(args.state, "stop")
    if stopped["status"] != "STOPPED":
        raise RuntimeError("CUSTOMER_STOP_NOT_VERIFIED")
    started = M.operate(args.state, "start")
    after = snapshot(args.state, args.project_id, args.completed_task)
    report = {"schema": "lattice.customer-restart-acceptance.v1", "status": "PASS" if before == after else "FAIL",
              "run_id": config["run_id"], "system_id": config["system_id"],
              "runtime_sha256": config["files"][config["runtime"]], "before": before, "after": after,
              "postgres_stopped": stopped["status"], "postgres_started": started["status"],
              "scope": "Isolated fixture work, parent relation, decision and verified result; not complete download acceptance."}
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(report, stream, ensure_ascii=True, indent=2)
    if json.loads(args.output.read_bytes()) != report or before != after:
        raise RuntimeError("CUSTOMER_RESTART_READBACK_REJECTED")
    print(json.dumps({"status": report["status"], "run_id": config["run_id"], "evidence": str(args.output),
                      "completed_task": args.completed_task, "result_digest": after["task"]["result_digest"]}))


if __name__ == "__main__":
    main()
