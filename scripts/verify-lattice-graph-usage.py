#!/usr/bin/env python3
"""Bounded live SQL acceptance in an explicitly owned disposable customer cluster.

Prepare with --bundle and a NEW --state; then run --stage sql. No Graphify/WSL
execution, external login, existing cluster adoption, or credential output.
The retained fixture also supports subsequent native MCP verification.
Alternatively --stage bootstrap --bundle OLD_BUNDLE --runtime NEW_EXE creates
a fresh fixture and verifies explicit upgrade/replay; --stage mcp --runtime
NEW_EXE verifies failed query observations and readback across process restart.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import uuid

REPO = Path(__file__).resolve().parents[1]
SCHEMA = "lattice.graph-usage-acceptance.v1"


def module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


CUSTOMER = module("usage_customer", REPO / "scripts/lattice-customer-runtime.py")
INSTALL = module("usage_installer", REPO / "scripts/lattice-one-click-install.py")


def require(condition, code):
    if not condition:
        raise RuntimeError(code)


def save(path: Path, value: dict):
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False)
        stream.write("\n")


def prepare(state: Path, bundle: Path) -> dict:
    require(not state.exists(), "FRESH_STATE_REQUIRED")
    runtime = bundle / "bin/latticed.exe"
    CUSTOMER.prepare(state, runtime, INSTALL.digest(runtime), bundle / "postgres/bin", bundle / "git/cmd/git.exe")
    config, _ = CUSTOMER.load(state)
    owned = {"schema": SCHEMA, "run_id": config["run_id"], "system_id": config["system_id"]}
    save(state / "usage-acceptance-owned.json", owned)
    return prepare_fixture(state, bundle, owned)


def prepare_fixture(state: Path, bundle: Path, owned: dict) -> dict:
    source = state / "fixture-project"
    INSTALL.prepare_source(bundle, source, managed_sample=True)
    locator = CUSTOMER.register_project(state, source, "Graph usage acceptance")
    common = [sys.executable, "-I", "-B", "-S", str(state / "bin/lattice-customer-runtime.py"), "--state", str(state)]
    task = INSTALL.mcp_batch(common, [("lattice_task_submit", {
        "client_request_id": "graph-usage-fixture-" + owned["run_id"], "project_id": locator["project_id"],
        "objective": "Verify Graphify usage observations in this disposable local fixture."})])[0]
    require(task.get("status") == "SUBMITTED" and task.get("project_id") == locator["project_id"], "FIXTURE_TASK_REJECTED")
    fixture = {"project_id": task["project_id"], "task_ref": task["task_ref"], "source": str(source),
               "commit": INSTALL.git_command(bundle, source, "rev-parse", "HEAD")}
    save(state / "usage-fixture.json", fixture)
    return {"status": "PREPARED", **owned, **fixture}


class Database:
    def __init__(self, state: Path):
        self.state = state
        self.config, self.password = CUSTOMER.load(state)
        owned = json.loads((state / "usage-acceptance-owned.json").read_bytes())
        require(owned == {"schema": SCHEMA, "run_id": self.config["run_id"], "system_id": self.config["system_id"]},
                "DISPOSABLE_CLUSTER_OWNERSHIP_REJECTED")
        CUSTOMER.verify_running(self.config, self.password)
        self.name = "lattice_task019_" + self.config["run_id"][:8] + "_base"

    def sql(self, query: str, role="runtime", expected_error=None) -> str:
        env = {key: value for key, value in os.environ.items() if not key.upper().startswith("PG")}
        env.update(PGPASSWORD=self.password, PGCLIENTENCODING="UTF8", PGCONNECT_TIMEOUT="10")
        result = subprocess.run([str(Path(self.config["postgres_bin"]) / "psql.exe"), "-X", "-qAt", "-w",
            "-v", "ON_ERROR_STOP=1", "-h", "127.0.0.1", "-p", str(self.config["port"]),
            "-U", "lattice_" + role + "_login", "-d", self.name],
            input="SET ROLE lattice_" + role + ";\n" + query, capture_output=True, text=True,
            encoding="utf-8", env=env, timeout=60, creationflags=subprocess.CREATE_NO_WINDOW)
        if expected_error:
            require(result.returncode != 0 and expected_error in result.stderr, "EXPECTED_SQL_REJECTION_MISSING:" + expected_error)
            return expected_error
        require(result.returncode == 0, "SQL_REJECTED:" + next(iter(re.findall(r"GRAPH_USAGE_[A-Z_]+", result.stderr)), "DATABASE_ERROR"))
        return result.stdout.strip()

    def call(self, function: str, payload: dict, expected_error=None):
        encoded = json.dumps(payload, separators=(",", ":")).replace("'", "''")
        return self.sql("SELECT control_product.graph_usage_" + function + "_v1('" + encoded + "'::jsonb);", expected_error=expected_error)

    def summary(self, project: str, task=None):
        require(re.fullmatch(r"[a-z0-9][a-z0-9._-]{1,63}", project), "PROJECT_SHAPE_REJECTED")
        require(task is None or re.fullmatch(r"[a-f0-9]{64}", task), "TASK_SHAPE_REJECTED")
        value = "NULL" if task is None else "'" + task + "'"
        return json.loads(self.sql("BEGIN READ ONLY; SELECT control_product.graph_usage_summary_v1('" + project + "'," + value + "); COMMIT;"))

    def catalog(self):
        source = (REPO / "crates/lattice-postgres-store/src/postgres_setup.rs").read_text(encoding="utf-8")
        result = {}
        for kind in ("FUNCTION", "TABLE"):
            query = re.search(r'const MANAGED_FOREMAN_' + kind + r'_CATALOG_SQL: &str = r"(.*?)";', source, re.S).group(1)
            rows = self.sql(query.replace("foreman_execution", "control_product") + ";", role="migrator").splitlines()
            digest = hashlib.sha256(("LATTICE_CONTROL_PRODUCT_" + kind + "_CATALOG_V1\0").encode())
            for row in rows:
                value = row.encode("utf-8")
                digest.update(len(value).to_bytes(8, "big"))
                digest.update(value)
            result[kind.lower() + "_sha256"] = digest.hexdigest()
        result["shape"] = json.loads(self.sql("SELECT json_build_object('relations',(SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='control_product'),'functions',(SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='control_product'),'types',(SELECT count(*) FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='control_product'));", role="migrator"))
        return result


def verify(state: Path) -> dict:
    db = Database(state)
    fixture = json.loads((state / "usage-fixture.json").read_bytes())
    sql_path = REPO / "db/extensions/control-product/graph-usage-v1.sql"
    require(db.sql("SELECT to_regprocedure('control_product.graph_usage_begin_v1(jsonb)') IS NULL;", role="migrator") == "t",
            "FRESH_USAGE_EXTENSION_REQUIRED")
    baseline = db.catalog()
    db.sql("BEGIN;\n" + sql_path.read_text(encoding="utf-8") + "\nCOMMIT;", role="migrator")
    catalog = db.catalog()
    save(state / "usage-catalog.json", {"sql_sha256": INSTALL.digest(sql_path), "before": baseline, "after": catalog})
    checks = []

    def check(condition, name):
        require(condition, name)
        checks.append(name)

    def begin(operation="QUERY", task=fixture["task_ref"], project=fixture["project_id"]):
        return {"usage_id": hashlib.sha256(uuid.uuid4().bytes).hexdigest(), "project_id": project,
                "task_ref": task, "commit": fixture["commit"], "operation": operation,
                "integration_mode": "GRAPHIFY", "query_digest": "a" * 64 if operation == "QUERY" else None}

    def finish(start, outcome="QUERIED"):
        return {"usage_id": start["usage_id"], "outcome": outcome, "source_receipt_digest": "b" * 64,
                "record_count": 3, "result_bytes": 432, "duration_ms": 12, "error_code": None,
                "analysis_calls": 2 if outcome == "ANALYZED" else 0, "query_calls": 1 if start["operation"] == "QUERY" else 0}

    project, task = fixture["project_id"], fixture["task_ref"]
    check(db.summary(project, task)["coverage"] == "UNKNOWN", "ZERO_RECORDS_UNKNOWN")
    query = begin()
    check(db.call("begin", query) == "RECORDED", "BEGIN_RECORDED")
    check(db.call("begin", query) == "REPLAYED", "BEGIN_EXACT_REPLAY")
    check(db.summary(project, task)["counts"]["pending"] == 1, "PENDING_VISIBLE")
    db.call("begin", {**query, "commit": "c" * 40}, "GRAPH_USAGE_IDEMPOTENCY_CONFLICT")
    checks.append("BEGIN_CONFLICT_REJECTED")
    db.call("begin", begin(project="forged-project"), "GRAPH_USAGE_TASK_BINDING_REJECTED")
    checks.append("FORGED_TASK_PROJECT_REJECTED")
    db.call("begin", begin(task="f" * 64), "GRAPH_USAGE_TASK_BINDING_REJECTED")
    checks.append("UNKNOWN_TASK_REJECTED")
    for bad in ({**begin(), "unknown": 1}, {**begin(), "operation": None}, {**begin(), "usage_id": None}):
        db.call("begin", bad, "GRAPH_USAGE_ARGUMENTS_REJECTED")
    checks.append("BEGIN_CLOSED_SHAPE_REJECTED")
    done = finish(query)
    check(db.call("finish", done) == "RECORDED", "FINISH_RECORDED")
    check(db.call("finish", done) == "REPLAYED", "FINISH_EXACT_REPLAY")
    db.call("finish", {**done, "result_bytes": 433}, "GRAPH_USAGE_IDEMPOTENCY_CONFLICT")
    checks.append("FINISH_CONFLICT_REJECTED")
    db.call("finish", {**done, "usage_id": "e" * 64}, "GRAPH_USAGE_START_REQUIRED")
    checks.append("ORPHAN_FINISH_REJECTED")
    failed = begin()
    db.call("begin", failed)
    failure = {**finish(failed, "FAILED"), "source_receipt_digest": None, "record_count": None,
               "result_bytes": None, "error_code": "GRAPHIFY_FIXTURE_FAILURE"}
    db.call("finish", {**failure, "error_code": None}, "GRAPH_USAGE_ARGUMENTS_REJECTED")
    db.call("finish", failure)
    checks.append("FAILURE_RECORDED_AND_ERROR_REQUIRED")
    for outcome in ("ANALYZED", "REUSED"):
        refresh = begin("REFRESH", task=None)
        db.call("begin", refresh)
        db.call("finish", finish(refresh, outcome))
    checks.append("UNBOUND_ANALYSIS_AND_REUSE_RECORDED")
    pending = begin()
    db.call("begin", pending)
    db.call("finish", {**finish(pending), "analysis_calls": 1}, "GRAPH_USAGE_ARGUMENTS_REJECTED")
    db.call("finish", {**finish(pending), "result_bytes": 16777217}, "GRAPH_USAGE_ARGUMENTS_REJECTED")
    for changes in ({"query_calls": 0}, {"duration_ms": None}, {"record_count": "3"}, {"unknown": 1},
                    {"outcome": "REUSED", "analysis_calls": 1}, {"analysis_calls": None}):
        db.call("finish", {**finish(pending), **changes}, "GRAPH_USAGE_ARGUMENTS_REJECTED")
    db.call("finish", finish(pending, "ANALYZED"), "GRAPH_USAGE_OUTCOME_REJECTED")
    checks.append("COUNTERS_AND_SIZE_BOUNDS_REJECTED")
    db.sql("SELECT * FROM control_product.graph_usage_starts;", expected_error="permission denied")
    db.sql("UPDATE control_product.graph_usage_starts SET project_id='forged-project';", expected_error="permission denied")
    db.sql("DELETE FROM control_product.graph_usage_finishes;", expected_error="permission denied")
    checks.append("RUNTIME_DIRECT_TABLE_ACCESS_DENIED")
    before = db.summary(project)
    check(before["counts"] == {"started": 5, "finished": 4, "pending": 1, "queried": 1,
          "analyzed": 1, "reused": 1, "failed": 1, "analysis_calls": 2, "query_calls": 2,
          "query_records": 3, "result_bytes": 1296, "measured_results": 3, "duration_ms": 48}, "AGGREGATES_EXACT")
    check(before["coverage"] == "INCOMPLETE", "PENDING_COVERAGE_INCOMPLETE")
    check(db.summary(project, task)["counts"]["started"] == 3, "TASK_SCOPE_EXCLUDES_UNBOUND")
    CUSTOMER.operate(state, "stop")
    CUSTOMER.operate(state, "start")
    check(Database(state).summary(project) == before, "POSTGRES_RESTART_EXACT_READBACK")
    return {"schema": SCHEMA, "status": "VERIFIED", "scope": "DISPOSABLE_POSTGRESQL_SQL",
            "checks": checks, "catalog": catalog, "fixture": fixture, "counts": before["counts"],
            "runtime_mcp_usage": "NOT_TESTED", "graphify_execution": "NOT_RUN"}


def native(runtime: Path, db: Database, arguments=(), requests=None):
    result = subprocess.run([str(runtime), *arguments], input=requests,
        env=CUSTOMER.environment(db.config, db.password), capture_output=True, text=True,
        encoding="utf-8", timeout=180, creationflags=subprocess.CREATE_NO_WINDOW)
    codes = re.findall(r"^(?:LATTICE|LATTICED|GRAPH_USAGE)_[A-Z_]+$", result.stderr, re.M)
    require(result.returncode == 0, "NATIVE_REJECTED:" + (codes[-1] if codes else "PROCESS_ERROR"))
    return result.stdout


def mcp(runtime: Path, db: Database, calls):
    requests = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": {"name": "graph-usage-acceptance", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"}]
    requests += [{"jsonrpc": "2.0", "id": index, "method": "tools/call",
        "params": {"name": name, "arguments": args}} for index, (name, args) in enumerate(calls, 2)]
    output = native(runtime, db, requests="".join(json.dumps(value) + "\n" for value in requests))
    require(len(output) < 2_000_000, "MCP_OUTPUT_BOUND_EXCEEDED")
    replies = [json.loads(line) for line in output.splitlines()]
    require(len(replies) == len(calls) + 1 and len({r.get("id") for r in replies}) == len(replies), "MCP_REPLIES_REJECTED")
    replies = {r["id"]: r for r in replies}
    require(replies[1].get("result", {}).get("protocolVersion") == "2025-11-25", "MCP_INITIALIZATION_REJECTED")
    return [replies[index]["result"] for index in range(2, len(calls) + 2)]


def verify_mcp(state: Path, runtime: Path) -> dict:
    db = Database(state)
    fixture = json.loads((state / "usage-fixture.json").read_bytes())
    selection = {"project_id": fixture["project_id"], "task_ref": fixture["task_ref"]}
    query = {**selection, "commit": fixture["commit"], "query": "greeting", "limit": 8}
    runtime_sha = INSTALL.digest(runtime)
    baseline = mcp(runtime, db, [("lattice_graph_usage", selection)])[0]
    require(not baseline.get("isError"), "MCP_BASELINE_REJECTED")
    before = baseline["structuredContent"]
    queries = [query, query, {**query, "project_id": "unknown-fixture-project"}, {**query, "task_ref": "f" * 64}]
    replies = mcp(runtime, db, [("lattice_code_relations", item) for item in queries] + [
        ("lattice_graph_usage", {**selection, "task_ref": "f" * 64}), ("lattice_graph_usage", selection)])
    require(all(item.get("isError") is True for item in replies[:5]), "MCP_NEGATIVE_EXPECTED")
    after = replies[5]["structuredContent"]
    require(not replies[5].get("isError"), "MCP_SUMMARY_REJECTED")
    for key, delta in (("started", 2), ("finished", 2), ("failed", 2), ("pending", 0), ("queried", 0),
                       ("query_calls", 0), ("analysis_calls", 0), ("result_bytes", 0), ("measured_results", 0)):
        require(after["counts"][key] - before["counts"][key] == delta, "MCP_COUNTER_REJECTED:" + key)
    existing = {item["usage_id"] for item in before["recent"]}
    added = [item for item in after["recent"] if item["usage_id"] not in existing]
    require(len(added) == 2 and len({item["usage_id"] for item in added}) == 2, "MCP_RETRY_ID_NOT_UNIQUE")
    for item in added:
        require(item["start"]["task_ref"] == fixture["task_ref"] and item["finish"]["outcome"] == "FAILED"
                and item["finish"]["error_code"] == replies[0]["structuredContent"]["code"], "MCP_FAILURE_RECORD_REJECTED")
    reread = mcp(runtime, db, [("lattice_graph_usage", selection)])[0]
    require(reread["structuredContent"] == after, "MCP_PROCESS_RESTART_READBACK_REJECTED")
    require(INSTALL.digest(runtime) == runtime_sha, "NATIVE_CHANGED_DURING_ACCEPTANCE")
    return {"schema": SCHEMA, "status": "VERIFIED", "scope": "NATIVE_MCP_FAILURE_AND_RESTART",
            "runtime_sha256": runtime_sha, "fixture": fixture, "baseline_coverage": before["coverage"],
            "coverage": after["coverage"], "counts": after["counts"], "usage_ids": [item["usage_id"] for item in added],
            "error_codes": [item["structuredContent"]["code"] for item in replies[:5]],
            "query_calls": "ZERO_BEFORE_RELATION_STORE_ENTRY", "runtime_restart_readback": "VERIFIED",
            "graphify_execution": "NOT_RUN", "successful_query": "NOT_TESTED"}


def verify_bootstrap(state: Path, bundle: Path, runtime: Path):
    prepare(state, bundle)
    db = Database(state)
    before = db.catalog()
    require(before["shape"] == {"relations": 26, "functions": 16, "types": 16}, "OLD_SCHEMA_REQUIRED")
    native(runtime, db, ["--postgres-bootstrap"])
    after = db.catalog()
    require(after["shape"] == {"relations": 31, "functions": 19, "types": 20}, "NEW_SCHEMA_MISSING")
    native(runtime, db, ["--postgres-bootstrap"])
    require(db.catalog() == after, "BOOTSTRAP_REPLAY_CHANGED_CATALOG")
    return {"schema": SCHEMA, "status": "VERIFIED", "scope": "NATIVE_OLD_SCHEMA_UPGRADE",
            "runtime_sha256": INSTALL.digest(runtime), "before": before, "after": after,
            "bootstrap_replay": "VERIFIED", "fresh_owned_state": str(state)}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--output", type=Path, help="new result file; an existing result is never overwritten")
    parser.add_argument("--stage", choices=("prepare", "sql", "mcp", "bootstrap"), required=True)
    args = parser.parse_args()
    try:
        output = args.output or args.state / ("usage-" + args.stage + "-result.json")
        require(not output.exists(), "FRESH_RESULT_PATH_REQUIRED")
        if args.stage == "prepare":
            require(args.bundle is not None, "BUNDLE_REQUIRED")
            result = prepare(args.state, args.bundle)
        elif args.stage == "sql":
            result = verify(args.state)
        else:
            require(args.runtime is not None, "RUNTIME_REQUIRED")
            if args.stage == "bootstrap":
                require(args.bundle is not None, "BUNDLE_REQUIRED")
                result = verify_bootstrap(args.state, args.bundle, args.runtime)
            else:
                result = verify_mcp(args.state, args.runtime)
        if args.stage != "prepare" or args.output is not None:
            save(output, result)
        print(json.dumps(result, ensure_ascii=False))
        return 0
    except Exception as error:
        # No arbitrary process stderr, SQL payload, installation config or secret.
        code = str(error) if isinstance(error, (RuntimeError, CUSTOMER.Rejected, INSTALL.Rejected)) else type(error).__name__
        print(json.dumps({"schema": SCHEMA, "status": "BLOCKED", "code": code}))
        return 2


if __name__ == "__main__":
    sys.exit(main())
