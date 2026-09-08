"""Explicit Graphify read/restart acceptance against one owned customer installation.

An optional hash-pinned candidate binary allows upgrade testing before packaging.
Its evidence distinguishes candidate execution from the actually installed binary.
No graph refresh, task submission, result import or schema mutation occurs here.
"""
import argparse
import importlib.util
import json
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("customer", Path(__file__).with_name("lattice-customer-runtime.py"))
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


def read(config, password, runtime, project, commit, query, path, task):
    base = {"project_id": project, "commit": commit, "query": query, "limit": 32}
    calls = [("lattice_code_relations", base),
             ("lattice_code_relations", {**base, "query": path, "limit": 1}),
             ("lattice_code_relations", {**base, "query": "lattice_delivery_fixture"}),
             ("lattice_code_relations", {**base, "query": "%"}),
             ("lattice_code_relations", {**base, "query": "' OR 1=1 --"}),
             ("lattice_code_relations", {**base, "commit": "f" * 40}),
             ("lattice_code_relations", {**base, "project_id": "unregistered-relations-acceptance"}),
             ("lattice_code_relations", {**base, "limit": 33}),
             ("lattice_task_status", {"task_ref": task}),
             ("lattice_control_snapshot", {"project_id": project})]
    requests = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "code-relations-acceptance", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"}]
    requests += [{"jsonrpc": "2.0", "id": i + 2, "method": "tools/call", "params": {"name": name, "arguments": args}}
                 for i, (name, args) in enumerate(calls)]
    result = M.invoke([str(runtime)], env=M.environment(config, password),
                      input_text="".join(json.dumps(r) + "\n" for r in requests), timeout=180)
    if result.returncode:
        raise RuntimeError("RELATIONS_RUNTIME_EXIT_REJECTED")
    replies = [json.loads(line) for line in result.stdout.splitlines()]
    if [r.get("id") for r in replies] != list(range(1, len(calls) + 2)):
        raise RuntimeError("RELATIONS_FRAMING_REJECTED")
    values = [r.get("result", {}).get("structuredContent") for r in replies[1:]]
    for index in [0, 1, 2, 3, 4, 8, 9]:
        reply = replies[index + 1]
        if "error" in reply or reply.get("result", {}).get("isError") or values[index] is None:
            raise RuntimeError("RELATIONS_POSITIVE_CALL_REJECTED")
    first = values[0]
    if not first["records"] or first["truncated"] or first["authority"] != "DERIVED":
        raise RuntimeError("RELATIONS_HIT_REQUIRED")
    if not any(r["graph_kind"] == "EDGE" and r["relation"] == "calls" for r in first["records"]):
        raise RuntimeError("RELATIONS_CALL_EDGE_REQUIRED")
    if len(values[1]["records"]) != 1 or not values[1]["truncated"]:
        raise RuntimeError("RELATIONS_TRUNCATION_REJECTED")
    for v in values[2:5]:
        if v["records"] or v["truncated"]:
            raise RuntimeError("RELATIONS_LITERAL_EMPTY_QUERY_REJECTED")
    for v in values[:5]:
        if (v["registered_project_id"] != project or v["commit"] != commit
                or v["source_receipt_digest"] != first["source_receipt_digest"]
                or "record_set_proof" in v or any(r["trusted_context"] for r in v["records"])):
            raise RuntimeError("RELATIONS_SOURCE_BINDING_REJECTED")
    if values[5].get("code") != "CODE_RELATIONS_SOURCE_RECEIPT_UNAVAILABLE":
        raise RuntimeError("RELATIONS_MISSING_COMMIT_NOT_REJECTED")
    if values[6].get("code") != "PROJECT_IS_NOT_REGISTERED":
        raise RuntimeError("RELATIONS_OTHER_PROJECT_NOT_REJECTED")
    if replies[8].get("error", {}).get("code") != -32602:
        raise RuntimeError("RELATIONS_LIMIT_NOT_REJECTED")
    if (values[8]["status"] != "COMPLETED" or values[9]["project"]["id"] != project
            or not any(t["task_ref"] == task and t["ledger"] == values[8] for t in values[9]["tasks"])):
        raise RuntimeError("RELATIONS_EXISTING_WORK_BINDING_REJECTED")
    return replies


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--project-id", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--query", required=True)
    parser.add_argument("--source-path", required=True)
    parser.add_argument("--task-ref", required=True)
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--runtime-sha256")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists() or not args.output.is_absolute() or not args.output.parent.is_dir():
        raise RuntimeError("FRESH_EVIDENCE_PATH_REQUIRED")
    config, password = M.load(args.state)
    M.verify_running(config, password)
    runtime = args.runtime or Path(config["runtime"])
    actual = M.file_digest(runtime)
    if args.runtime and actual != args.runtime_sha256:
        raise RuntimeError("CANDIDATE_RUNTIME_DIGEST_REJECTED")
    params = (config, password, runtime, args.project_id, args.commit, args.query, args.source_path, args.task_ref)
    before = read(*params)
    M.operate(args.state, "stop")
    M.operate(args.state, "start")
    after = read(*params)
    if before != after or M.file_digest(runtime) != actual:
        raise RuntimeError("RELATIONS_RESTART_OR_VERSION_MISMATCH")
    evidence = {"schema": "lattice.code-relations-acceptance.v1", "status": "VERIFIED",
                "run_id": config["run_id"], "system_id": config["system_id"], "runtime_sha256": actual,
                "installed_runtime_sha256": M.file_digest(Path(config["runtime"])),
                "execution": "INSTALLED" if runtime == Path(config["runtime"]) else "PINNED_CANDIDATE",
                "pg_and_mcp_restarted": True, "before": before, "after": after}
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(evidence, stream, ensure_ascii=False, indent=2)
    print(json.dumps({"status": "VERIFIED", "hits": len(before[1]["result"]["structuredContent"]["records"]),
                      "execution": evidence["execution"], "evidence": str(args.output)}))


if __name__ == "__main__":
    main()
