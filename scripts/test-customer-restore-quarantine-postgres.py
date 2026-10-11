"""Windows PG17 physical-clone fixture for the customer restore quarantine gate.

No production arguments, cleanup, encrypted-backup or WSL acceptance. Evidence
and both stopped clusters remain under this checkout's .lattice directory.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import uuid

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("restore_fixture", Path(__file__).with_name("lattice-customer-backup.py"))
B = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(B)
M = B.M
RESERVED = {4317, 5432, 55432, 58743, 64272}


def process_identity(pid, postgres):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
    kernel.GetProcessTimes.argtypes = [wintypes.HANDLE, *([ctypes.POINTER(wintypes.FILETIME)] * 4)]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.OpenProcess(0x1000, False, pid)
    if not handle: raise RuntimeError("FIXTURE_PROCESS_UNREADABLE")
    try:
        text = ctypes.create_unicode_buffer(32768); length = wintypes.DWORD(len(text))
        times = [wintypes.FILETIME() for _ in range(4)]
        if not kernel.QueryFullProcessImageNameW(handle, 0, text, ctypes.byref(length)):
            raise RuntimeError("FIXTURE_PROCESS_PATH_UNREADABLE")
        if Path(text.value).resolve() != postgres or not kernel.GetProcessTimes(handle, *(ctypes.byref(t) for t in times)):
            raise RuntimeError("FIXTURE_PROCESS_IDENTITY_REJECTED")
        return {"pid": pid, "created": (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime}
    finally: kernel.CloseHandle(handle)


def tree_digest(root):
    entries = []
    for item in sorted(root.rglob("*")):
        M.CONFIG.regular_path(item)
        if item.is_file(): entries.append([item.relative_to(root).as_posix(), M.file_digest(item)])
    return M.CONFIG.digest(M.CONFIG.json_bytes(entries))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--seed", type=Path, required=True)
    parser.add_argument("--pg-bin", type=Path, default=Path("C:/Program Files/PostgreSQL/17/bin"))
    args = parser.parse_args()
    if os.name != "nt": raise RuntimeError("FIXTURE_WINDOWS_ONLY")
    repo = Path(__file__).resolve().parent.parent
    base = repo / ".lattice/customer-restore-quarantine-fixture"
    M.CONFIG.regular_path(base); base.mkdir(parents=True, exist_ok=True)
    run_id = uuid.uuid4().hex; root = base / run_id; root.mkdir()
    pg = M.regular(args.pg_bin.resolve(), directory=True)
    for name in ("initdb", "postgres", "pg_ctl", "psql", "pg_controldata"): M.regular(pg / (name + ".exe"))
    password = secrets.token_hex(24)
    env = M.closed_environment()
    report = {"schema": "lattice.customer-restore-quarantine-fixture.v1", "status": "RUNNING", "runRoot": str(root),
              "productionDatabaseAccess": False, "encryptedRestoreVerified": False, "wslRestoreVerified": False,
              "scope": "PHYSICAL_PG_CLONE_AND_CUSTOMER_MANAGER_QUARANTINE", "checks": [], "clusters": []}
    counter = 0

    def command(argv, label, extra=None, input_text=None, allowed=(0,), output_stream="stdout"):
        nonlocal counter
        counter += 1
        if Path(argv[0]).name.lower() == "pg_ctl.exe":
            # PostgreSQL inherits pg_ctl's output handles on Windows. A PIPE
            # makes communicate() wait for the server's lifetime after pg_ctl
            # exits; use a fixture-owned file for this process launcher.
            output_path = root / f"{counter:02}-{label}.log"
            with output_path.open("wb") as output:
                result = subprocess.run([str(a) for a in argv], stdin=subprocess.DEVNULL,
                                        stdout=output, stderr=subprocess.STDOUT, env=extra or env,
                                        timeout=45, creationflags=subprocess.CREATE_NO_WINDOW)
            result.stdout = output_path.read_text(encoding="utf-8", errors="replace")
            result.stderr = ""
        else:
            result = subprocess.run([str(a) for a in argv], input=input_text, env=extra or env,
                                    capture_output=True, text=True, encoding="utf-8", errors="replace",
                                    timeout=120, creationflags=subprocess.CREATE_NO_WINDOW)
        record = {"label": label, "exitCode": result.returncode,
                  "stdout": result.stdout.replace(password, "[FIXTURE_PASSWORD]"),
                  "stderr": result.stderr.replace(password, "[FIXTURE_PASSWORD]")}
        (root / f"{counter:02}-{label}.json").write_bytes(M.CONFIG.json_bytes(record))
        if result.returncode not in allowed: raise RuntimeError("FIXTURE_COMMAND_FAILED:" + label)
        return getattr(result, output_stream).strip()

    def check(name): report["checks"].append(name)

    def new_cluster(name):
        state = root / name; state.mkdir()
        port = M.free_port()
        while port in RESERVED or any(c["port"] == port for c in report["clusters"]): port = M.free_port()
        item = {"name": name, "root": str(state), "cluster": str(state / "cluster"), "port": port, "process": None}
        marker = {"kind": "LATTICE_CUSTOMER_RESTORE_SYNTHETIC_FIXTURE", "runId": run_id, "cluster": item["cluster"], "port": port}
        (state / "fixture-owner.json").write_bytes(M.CONFIG.json_bytes(marker))
        item["marker"] = marker; report["clusters"].append(item)
        return item

    def verify_owner(item):
        state = Path(item["root"])
        if state.parent != root or Path(item["cluster"]).parent != state or item["port"] in RESERVED:
            raise RuntimeError("FIXTURE_OWNER_PATH_REJECTED")
        if json.loads((state / "fixture-owner.json").read_bytes()) != item["marker"]:
            raise RuntimeError("FIXTURE_OWNER_MARKER_CHANGED")
        M.regular(Path(item["cluster"]), directory=True)

    def sql(item, query, database="postgres"):
        verify_owner(item)
        return command([pg / "psql.exe", "-X", "-qAt", "-h", "127.0.0.1", "-p", item["port"],
                        "-U", "runtime_bootstrap", "-d", database, "-v", "ON_ERROR_STOP=1", "-f", "-"],
                       item["name"] + "-sql", input_text=query)

    def start(item):
        verify_owner(item)
        command([pg / "pg_ctl.exe", "-D", item["cluster"], "-l", Path(item["root"]) / "postgres.log", "-w", "-t", "30", "start"], item["name"] + "-start")
        pid = int((Path(item["cluster"]) / "postmaster.pid").read_text().splitlines()[0])
        item["process"] = process_identity(pid, (pg / "postgres.exe").resolve())
        identity = json.loads(sql(item, "SELECT json_build_object('data',current_setting('data_directory'),'port',current_setting('port')::int,'listen',current_setting('listen_addresses'))"))
        if Path(identity["data"]).resolve() != Path(item["cluster"]) or identity["port"] != item["port"] or identity["listen"] != "127.0.0.1":
            raise RuntimeError("FIXTURE_SERVER_IDENTITY_REJECTED")

    def stop(item):
        verify_owner(item)
        pid_file = Path(item["cluster"]) / "postmaster.pid"
        if pid_file.exists():
            observed = process_identity(int(pid_file.read_text().splitlines()[0]), (pg / "postgres.exe").resolve())
            if item["process"] is not None and observed != item["process"]: raise RuntimeError("FIXTURE_STOP_PROCESS_CHANGED")
            command([pg / "pg_ctl.exe", "-D", item["cluster"], "-m", "fast", "-w", "-t", "30", "stop"], item["name"] + "-stop")
        if pid_file.exists(): raise RuntimeError("FIXTURE_STOP_FAILED")
        item["stopped"] = True

    def snapshot(item, database):
        tables = json.loads(sql(item, "SELECT coalesce(json_agg(json_build_array(n.nspname,c.relname) ORDER BY n.nspname,c.relname),'[]') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND n.nspname NOT LIKE 'pg_%' AND n.nspname<>'information_schema'", database))
        rows = {}
        for schema, table in tables:
            if not all(part.replace("_", "").isalnum() for part in (schema, table)): raise RuntimeError("FIXTURE_TABLE_NAME_REJECTED")
            rows[schema + "." + table] = json.loads(sql(item, f'SELECT coalesce(json_agg(v ORDER BY v::text COLLATE "C"),\'[]\') FROM (SELECT to_jsonb(r) v FROM ONLY "{schema}"."{table}" r) q', database))
        return rows

    tracked_sources = [Path(__file__).resolve(), Path(B.__file__).resolve(), Path(M.__file__).resolve()]
    report["sourceHashes"] = {str(p.relative_to(repo)): M.file_digest(p) for p in tracked_sources}
    try:
        if " 17." not in command([pg / "postgres.exe", "--version"], "pg-version"): raise RuntimeError("FIXTURE_PG17_REQUIRED")
        binaries = root / "bin"; binaries.mkdir()
        copies = {}
        for label, original in (("runtime", args.runtime), ("seed", args.seed)):
            original = M.regular(original.resolve()); expected = M.file_digest(original)
            target = binaries / original.name; shutil.copyfile(original, target)
            if M.file_digest(original) != expected or M.file_digest(target) != expected: raise RuntimeError("FIXTURE_BINARY_CHANGED")
            copies[label] = target
        report["binaries"] = {key: {"path": str(value), "sha256": M.file_digest(value)} for key, value in copies.items()}
        source = new_cluster("source")
        command([pg / "initdb.exe", "-D", source["cluster"], "-U", "runtime_bootstrap", "--auth-host=trust", "--auth-local=trust", "--encoding=UTF8", "--no-locale", "--data-checksums"], "initdb")
        with (Path(source["cluster"]) / "postgresql.conf").open("a", encoding="utf-8") as stream:
            stream.write(f"\nlisten_addresses='127.0.0.1'\nport={source['port']}\nmax_connections=15\nshared_buffers='16MB'\nlog_statement='none'\n")
        start(source)
        seed_root = root / "seed-projects"; seed_root.mkdir()
        for name in ("project-a", "project-b"):
            (seed_root / name).mkdir(); (seed_root / name / "synthetic.txt").write_text("synthetic retained project", encoding="utf-8")
        (seed_root / "project-purge-fixture.marker").write_text(run_id, encoding="ascii")
        native_env = {**env, "LATTICE_TASK019_HOST": "127.0.0.1", "LATTICE_TASK019_PORT": str(source["port"]),
                      "LATTICE_TASK019_RUN_ID": run_id, "LATTICE_TASK019_PASSWORD": password,
                      "LATTICE_RUNTIME_INTEGRATION": "CORE_ONLY", "LATTICE_FULL_CHAIN_RUN_MODE": "RESUME_EXISTING",
                      "LATTICE_DELIVERY_CODEX_MODE": "OFFICIAL_CODEX_APP_SERVER", "LATTICE_MANAGED_FOREMAN_MODE": "DISABLED",
                      "LATTICE_STORE_DAEMON_INSTANCE_ID": "task050-fresh-process", "LATTICE_STORE_DAEMON_EPOCH": "50",
                      "LATTICE_STORE_AUTHORITY_REVISION": "50", "LATTICE_STORE_OBSERVATION_DIGEST": "a" * 64,
                      "LATTICE_STORE_AUTHORITY_HEAD_DIGEST": "b" * 64, "LATTICE_PURGE_FIXTURE_ONLY": "1",
                      "LATTICE_PURGE_FIXTURE_ROOT": str(seed_root), "LATTICE_REGISTRY_ANCHOR_ROOT": str(root / "anchors")}
        command([copies["runtime"], "--postgres-initialize"], "initialize", native_env)
        command([copies["runtime"], "--postgres-bootstrap"], "bootstrap", native_env)
        database = f"lattice_task019_{run_id[:8]}_base"
        # Match the existing purge fixture's extension-install precondition.
        # Only this newly initialized, empty, owner-checked synthetic database
        # is changed; the candidate restore finalizer never changes admission.
        if sql(source, "SELECT count(*) FROM control.project_registry_commands", database) != "0":
            raise RuntimeError("FIXTURE_EMPTY_REGISTRY_REQUIRED")
        stopped = sql(source, "UPDATE control.runtime_admission SET admission_mode='STOPPED',daemon_instance_id=NULL,daemon_epoch=NULL,authority_revision=0,observation_digest=NULL,authority_head_digest=NULL WHERE singleton AND admission_mode='ACTIVE' AND daemon_instance_id='task050-fresh-process' AND daemon_epoch=50 AND authority_revision=50 AND observation_digest=decode(repeat('a',64),'hex') AND authority_head_digest=decode(repeat('b',64),'hex') RETURNING admission_mode", database)
        if stopped != "STOPPED": raise RuntimeError("FIXTURE_SYNTHETIC_ADMISSION_MISMATCH")
        command([copies["seed"]], "seed", native_env)
        before = snapshot(source, database)
        projects = before["control.project_registry_projects"]
        if len(projects) != 2 or any(p["authority_lifecycle"] != "ACTIVE" for p in projects): raise RuntimeError("FIXTURE_ACTIVE_PROJECTS_REQUIRED")
        if len(before["control.task_submission_envelopes"]) != 2: raise RuntimeError("FIXTURE_TASKS_REQUIRED")
        report["rowDigest"] = M.CONFIG.digest(M.CONFIG.json_bytes(before)); check("native-api-seeded-two-active-projects-and-tasks")
        stop(source)
        old_digest = tree_digest(Path(source["cluster"]))
        restored = new_cluster("restored"); state = Path(restored["root"])
        (state / "restore.in-progress").write_bytes(b"lattice.customer-restore.v1\n")
        shutil.copytree(source["cluster"], restored["cluster"])
        if tree_digest(Path(restored["cluster"])) != old_digest: raise RuntimeError("FIXTURE_COPY_CHANGED")
        check("stopped-physical-backup-copy-byte-identical")
        with (state / "cluster/postgresql.conf").open("a", encoding="utf-8") as stream: stream.write(f"\nport={restored['port']}\n")
        system_id = M.control_identifier(command([pg / "pg_controldata.exe", restored["cluster"]], "restored-control"))
        # Real sealed customer config and DPAPI, deliberately without WSL. The
        # test is the finalization gate, not the complete encrypted restore path.
        config = {"schema": M.SCHEMA, "root": str(state), "run_id": run_id, "system_id": system_id,
                  "port": restored["port"], "postgres_bin": str(pg), "runtime": str(copies["runtime"]),
                  "files": {str(copies["runtime"]): M.file_digest(copies["runtime"])}}
        (state / "installation.json").write_bytes(M.CONFIG.json_bytes(config))
        (state / "credentials.dpapi").write_bytes(M.dpapi(M.CONFIG.json_bytes({"password": password, "installation_sha256": M.file_digest(state / "installation.json")})))
        catalog = {"schema": "lattice.customer-project-catalog.v1", "projects": [{"id": p["project_id"], "activeInHistoricalDatabase": True} for p in projects]}
        (state / "projects.json").write_bytes(M.CONFIG.json_bytes(catalog))
        (state / "restore-proofs").mkdir()
        proofs = {}
        for project in projects:
            proof = state / "restore-proofs" / (project["project_id"] + ".json")
            proof.write_bytes(M.CONFIG.json_bytes({"fixtureHistoricalRow": project}))
            proofs[project["project_id"]] = M.file_digest(proof)
        journal = {"schema": "lattice.customer-restore.v1", "root": str(state), "installation_sha256": M.file_digest(state / "installation.json"),
                   "catalog_sha256": M.file_digest(state / "projects.json"), "backup_sha256": old_digest, "proofs": proofs,
                   "requests": {}, "results": {p["project_id"]: {"current": {"active": True}} for p in projects}}
        pending = state / "restore.pending.dpapi"; pending.write_bytes(M.dpapi(M.CONFIG.json_bytes(journal)))
        pending_digest = M.file_digest(pending); cluster_digest = tree_digest(state / "cluster")
        for attempt in range(2):
            output = json.loads(command([sys.executable, "-I", "-B", "-S", B.__file__, "finalize-restore", "--state", state], f"finalize-{attempt}"))
            if output["status"] != "CUSTOMER_RESTORE_QUARANTINED" or output["database_state"] != "NOT_CHECKED": raise RuntimeError("FIXTURE_QUARANTINE_REPORT_REQUIRED")
            if tree_digest(state / "cluster") != cluster_digest or (state / "cluster/postmaster.pid").exists() or (state / "ready.json").exists(): raise RuntimeError("FIXTURE_FINALIZER_CHANGED_CLUSTER_OR_READY")
        check("real-finalizer-and-retry-preserve-stopped-cluster-and-pending-journal")
        if M.file_digest(pending) != pending_digest or not (state / "restore.in-progress").is_file(): raise RuntimeError("FIXTURE_GATE_CHANGED")
        for action in ("serve", "start", "recover", "connect", "graphify-refresh"):
            output = json.loads(command([sys.executable, "-I", "-B", "-S", M.__file__, action, "--state", state], "blocked-" + action, allowed=(2,),
                                        output_stream="stderr" if action == "serve" else "stdout"))
            if output.get("code") != "CUSTOMER_RESTORE_FINALIZATION_REQUIRED": raise RuntimeError("FIXTURE_GATE_NOT_ENFORCED")
        check("real-wrapper-mcp-and-work-entry-blocked")
        if tree_digest(state / "cluster") != cluster_digest: raise RuntimeError("FIXTURE_GATE_MUTATED_CLUSTER")
        # Only this fixture starts the clone for readback, never the finalizer.
        start(restored)
        if snapshot(restored, database) != before: raise RuntimeError("FIXTURE_RESTORED_ROWS_CHANGED")
        check("clone-readback-preserves-active-history-and-tasks-without-reactivation")
        stop(restored)
        if tree_digest(Path(source["cluster"])) != old_digest: raise RuntimeError("FIXTURE_SOURCE_BACKUP_CHANGED")
        for key, value in report["sourceHashes"].items():
            if M.file_digest(repo / key) != value: raise RuntimeError("FIXTURE_SOURCE_CHANGED")
        report["status"] = "PASS"
    except Exception as error:
        report["status"] = "FAIL"; report["error"] = str(error).replace(password, "[FIXTURE_PASSWORD]")
        raise
    finally:
        stop_errors = []
        for item in reversed(report["clusters"]):
            if Path(item["cluster"]).is_dir():
                try: stop(item)
                except Exception as error: stop_errors.append(str(error))
        report["fixtureStopped"] = not stop_errors
        if stop_errors: report["status"] = "FAIL"; report["stopErrors"] = stop_errors
        (root / "result.json").write_bytes(M.CONFIG.json_bytes(report))
        print(json.dumps({"status": report["status"], "fixtureStopped": report["fixtureStopped"], "evidence": str(root / "result.json")}))
        if stop_errors: raise RuntimeError("FIXTURE_STOP_NOT_CONFIRMED")


if __name__ == "__main__": main()
