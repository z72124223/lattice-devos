#!/usr/bin/env python3
"""Single-entry Windows installation and three-core acceptance for LATTICE."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time

SCHEMA = "lattice.one-click-install.v1"
SAMPLE_SCHEMA = "lattice.installer-sample.v1"
REQUIRED = ("bundle.json", "bin/latticed.exe", "bin/lattice-bundle.py",
            "bin/lattice-customer-runtime.py", "bin/lattice-wsl-platform.py", "python/python.exe",
            "postgres/bin", "git/cmd/git.exe", "node/node.exe", "graphify")
RUNNING = "RUNNING_IDENTITY_VERIFIED"


class Rejected(Exception):
    pass


def progress(message: str) -> None:
    if hasattr(sys.stderr, "reconfigure"):
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    print(message, file=sys.stderr, flush=True)


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def invoke(arguments: list[str], *, env: dict | None = None, input_text: str | None = None,
           timeout: int = 900) -> subprocess.CompletedProcess:
    return subprocess.run(arguments, capture_output=True, text=True, encoding="utf-8", errors="replace",
                          timeout=timeout, env=env, input=input_text,
                          creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)


def run_json(arguments: list[str], *, allow_reboot: bool = False, timeout: int = 900) -> dict:
    """Require both a successful process and a readable, successful response."""
    try:
        result = invoke(arguments, timeout=timeout)
    except subprocess.TimeoutExpired:
        return {"status": "BLOCKED", "code": "INSTALL_STEP_TIMED_OUT", "exit_code": 2}
    except OSError:
        return {"status": "BLOCKED", "code": "INSTALL_STEP_COULD_NOT_START", "exit_code": 2}
    payload = None
    # Native wrappers emit JSON on stdout; errors may also use stderr. Never
    # include arbitrary stderr in reports, which could contain credentials.
    for output in (result.stdout, result.stderr):
        for candidate in [output.strip(), *reversed(output.strip().splitlines())]:
            try:
                parsed = json.loads(candidate)
            except ValueError:
                continue
            if isinstance(parsed, dict):
                payload = parsed
                break
        if payload is not None:
            break
    if payload is None:
        payload = {"status": "BLOCKED", "code": "INSTALL_OUTPUT_UNREADABLE"}
    if result.returncode and not (allow_reboot and result.returncode == 3 and payload.get("status") == "REBOOT_REQUIRED"):
        payload["status"] = "BLOCKED"
        payload.setdefault("code", "INSTALL_STEP_FAILED")
    payload["exit_code"] = result.returncode
    return payload


def successful(payload: dict, *statuses: str) -> bool:
    return payload.get("exit_code") == 0 and payload.get("status") in statuses


def blocked(payload: dict, code: str) -> dict:
    return {**payload, "status": "BLOCKED", "code": payload.get("code", code)}


def confirm_keep_existing(audit: dict) -> bool:
    if os.name != "nt":
        return False
    import ctypes
    warnings = "\n".join(str(item) for item in audit.get("warnings", []))[:5000]
    text = ("偵測到可能重複的技能、工作流程或 MCP：\n\n" + warnings +
            "\n\n按「是」：保留原設定並繼續安裝。\n按「否」：停止安裝，先檢視重複檢查報告。"
            "\n\n本程式不會自動刪除或修改這些候選；繼續不表示重複已解決。")
    return ctypes.windll.user32.MessageBoxW(None, text, "LATTICE 安裝前確認", 0x124) == 6


def overlaps(first: Path, second: Path) -> bool:
    return first == second or first.is_relative_to(second) or second.is_relative_to(first)


def preflight(bundle: Path, state: Path, source: Path, wsl: Path, *, managed_sample: bool = False) -> dict:
    checks = [{"check": "windows", "status": "PASS" if os.name == "nt" else "BLOCKED",
               "code": None if os.name == "nt" else "WINDOWS_REQUIRED", "version": platform.version()}]
    missing = [name for name in REQUIRED if not (bundle / name).exists()]
    checks.append({"check": "bundle", "status": "PASS" if not missing else "BLOCKED",
                   "code": "BUNDLE_INCOMPLETE" if missing else None, "missing": missing})
    try:
        data = json.loads((bundle / "bundle.json").read_text(encoding="utf-8"))
        valid = data.get("schema") in ("lattice.bundle.v1", "lattice.windows-dependency-bundle.v1")
        checks.append({"check": "manifest", "status": "PASS" if valid else "BLOCKED",
                       "code": None if valid else "BUNDLE_MANIFEST_SCHEMA_REJECTED"})
    except (OSError, ValueError, AttributeError):
        checks.append({"check": "manifest", "status": "BLOCKED", "code": "BUNDLE_MANIFEST_UNREADABLE"})
    recoverable = all((state / name).is_file() for name in ("installation.json", "credentials.dpapi"))
    state_ok = not state.exists() or recoverable
    checks.append({"check": "state", "status": "PASS" if state_ok else "BLOCKED",
                   "mode": "RECOVER" if recoverable else "NEW", "code": None if state_ok else "STATE_INCOMPLETE_PRESERVED"})
    source_ok = source.is_dir() or (managed_sample and not source.exists())
    checks.append({"check": "graph-source", "status": "PASS" if source_ok else "BLOCKED",
                   "mode": "MANAGED_SAMPLE" if managed_sample else "EXISTING_PROJECT",
                   "code": None if source_ok else "GRAPH_SOURCE_MISSING"})
    scope_ok = not any(overlaps(a, b) for a, b in ((bundle, state), (bundle, source), (state, source)))
    checks.append({"check": "path-scope", "status": "PASS" if scope_ok else "BLOCKED",
                   "code": None if scope_ok else "INSTALL_PATHS_OVERLAP"})
    checks.append({"check": "wsl-path", "status": "PASS" if wsl.is_file() else "BLOCKED",
                   "code": None if wsl.is_file() else "WSL_LAUNCHER_MISSING"})
    failures = [entry for entry in checks if entry["status"] == "BLOCKED"]
    return {"schema": SCHEMA, "status": "BLOCKED" if failures else "READY", "created_at": int(time.time()),
            "bundle": str(bundle), "state": str(state), "source": str(source), "checks": checks,
            "blocked_codes": [entry["code"] for entry in failures if entry.get("code")]}


def write_workspace_hook(source: Path) -> None:
    begin, end = "<!-- BEGIN LATTICE MANAGED v1 -->", "<!-- END LATTICE MANAGED v1 -->"
    hook = (begin + "\n## LATTICE 範例工作區\n"
            "本專案沿用 Codex 全域 AGENTS.md 的 LATTICE 規則，此處不另行要求啟動檢查。\n" + end)
    path = source / "AGENTS.md"
    existing = path.read_text(encoding="utf-8") if path.exists() else ""
    if begin in existing or end in existing:
        if existing.count(begin) != 1 or existing.count(end) != 1 or existing.index(end) < existing.index(begin):
            raise Rejected("WORKSPACE_HOOK_CONFLICT_PRESERVED")
        start, finish = existing.index(begin), existing.index(end) + len(end)
        updated = existing[:start] + hook + existing[finish:]
    else:
        updated = existing + ("\n\n" if existing else "") + hook + "\n"
    if updated != existing:
        path.write_text(updated, encoding="utf-8")


def install_global_hook(codex_home: Path) -> dict:
    """Manage only our global guidance block; retain all other user instructions."""
    path = codex_home / "AGENTS.md"
    before = path.read_bytes() if path.exists() else b""
    existing = before.decode("utf-8")
    begin, end = "<!-- BEGIN LATTICE GLOBAL STARTUP v1 -->", "<!-- END LATTICE GLOBAL STARTUP v1 -->"
    hook = (begin + "\n## LATTICE 全域工作規則\n"
            "每個新 Codex 工作階段開始時，先呼叫一次 `lattice_runtime_status`；同一階段不要因工作區規則重複啟動。\n"
            "收到工作請求時，先使用可用的 LATTICE 任務工具核對既有工作；延續既有工作，必要時才建立新工作，勿重複送件。\n"
            "LATTICE 固定為 Control、PostgreSQL、Graphify 三核心；Hermes 永久退役，不得啟用反思功能。\n"
            "LATTICE 無法連線時如實說明，不得捏造任務身分、狀態或驗收結果。\n" + end)
    if begin in existing or end in existing:
        if existing.count(begin) != 1 or existing.count(end) != 1 or existing.index(end) < existing.index(begin):
            raise Rejected("GLOBAL_HOOK_CONFLICT_PRESERVED")
        start, finish = existing.index(begin), existing.index(end) + len(end)
        updated = existing[:start] + hook + existing[finish:]
    elif re.search(r"\blattice_runtime_status\b", existing):
        return {"status": "REUSED", "path": str(path), "existing_guidance": "PRESERVED"}
    else:
        updated = existing + ("\n\n" if existing else "") + hook + "\n"
    after = updated.encode("utf-8")
    if after == before:
        return {"status": "UNCHANGED", "path": str(path)}
    stamp = str(time.time_ns())
    backup = path.with_name("AGENTS.md.lattice-backup-" + stamp)
    with backup.open("xb") as stream:
        stream.write(before)
    if backup.read_bytes() != before:
        raise Rejected("GLOBAL_HOOK_BACKUP_READBACK_REJECTED")
    pending = path.with_name("AGENTS.md.lattice-pending-" + stamp)
    with pending.open("xb") as stream:
        stream.write(after)
    if path.exists():
        shutil.copystat(path, backup)
        shutil.copystat(path, pending)
    if (path.read_bytes() if path.exists() else b"") != before:
        raise Rejected("GLOBAL_HOOK_CHANGED_DURING_INSTALL_PRESERVED")
    os.replace(pending, path)
    if path.read_bytes() != after:
        raise Rejected("GLOBAL_HOOK_READBACK_REJECTED")
    return {"status": "INSTALLED", "path": str(path), "backup": str(backup), "readback": "VERIFIED"}


def git_command(bundle: Path, source: Path, *arguments: str) -> str:
    env = {key: value for key, value in os.environ.items() if not key.upper().startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0")
    result = invoke([str(bundle / "git/cmd/git.exe"), "-c", "core.hooksPath=" + os.devnull,
                     "-c", "core.fsmonitor=false", "-c", "commit.gpgsign=false", "-C", str(source), *arguments], env=env)
    if result.returncode:
        raise Rejected("INSTALL_PROJECT_GIT_REJECTED")
    return result.stdout.strip()


def prepare_source(bundle: Path, source: Path, *, managed_sample: bool) -> None:
    marker = source / ".lattice-sample.json"
    if managed_sample and not source.exists():
        source.parent.mkdir(parents=True, exist_ok=True)
        source.mkdir()  # Never adopt or overwrite an existing directory.
        marker.write_text(json.dumps({"schema": SAMPLE_SCHEMA}) + "\n", encoding="utf-8")
        (source / "README.md").write_text("# LATTICE welcome project\n\nA local sample used to verify the three LATTICE cores.\n", encoding="utf-8")
        (source / "sample.py").write_text('def greeting(name: str) -> str:\n    return f"Welcome to LATTICE, {name}!"\n\n\ndef main():\n    return greeting("friend")\n', encoding="utf-8")
        write_workspace_hook(source)
        git_command(bundle, source, "init", "--template=", "--initial-branch=main")
        git_command(bundle, source, "add", "--", ".lattice-sample.json", "README.md", "sample.py", "AGENTS.md")
        git_command(bundle, source, "-c", "user.name=LATTICE Setup", "-c", "user.email=setup@localhost",
                    "commit", "-m", "Initialize LATTICE acceptance sample")
    elif managed_sample:
        try:
            owned = json.loads(marker.read_text(encoding="utf-8")) == {"schema": SAMPLE_SCHEMA}
        except (OSError, ValueError):
            owned = False
        if not owned:
            raise Rejected("SAMPLE_DIRECTORY_NOT_OWNED_PRESERVED")
    if Path(git_command(bundle, source, "rev-parse", "--show-toplevel")).resolve() != source:
        raise Rejected("CUSTOMER_GIT_ROOT_REJECTED")
    git_command(bundle, source, "symbolic-ref", "--quiet", "HEAD")
    git_command(bundle, source, "rev-parse", "--verify", "HEAD^{commit}")
    if git_command(bundle, source, "status", "--porcelain", "--untracked-files=normal"):
        raise Rejected("GRAPH_SOURCE_DIRTY_PRESERVED")


def provision_platform(python: list[str], bundle: Path, state: Path, wsl: Path,
                       supplied: Path | None) -> Path:
    platform_script = str(bundle / "bin/lattice-wsl-platform.py")
    if supplied and (supplied / "platform.json").is_file():
        return supplied  # prepare() verifies the sealed platform identity.
    assets = supplied or bundle / "platform"
    archives = sorted(assets.glob("*.wsl")) if assets.is_dir() else []
    if len(archives) != 1:
        raise Rejected("ONE_PINNED_WSL_ARCHIVE_REQUIRED")
    # prepare() owns a *new* runtime directory, so a WSL distribution must live
    # beside it. Preserve interrupted imports and never unregister another distro.
    root = state.with_name(state.name + "-wsl-platform")
    index = 1
    while root.exists() and not (root / "platform.json").is_file():
        index += 1
        root = state.with_name(state.name + f"-wsl-platform-{index}")
    if root.exists():
        payload = run_json(python + [platform_script, "verify", "--root", str(root)])
    else:
        state.parent.mkdir(parents=True, exist_ok=True)
        payload = run_json(python + [platform_script, "provision", "--root", str(root), "--archive", str(archives[0]),
                                     "--wsl", str(wsl), "--wsl-sha256", digest(wsl)])
    if not successful(payload, "WSL_PLATFORM_IDENTITY_VERIFIED"):
        raise Rejected(payload.get("code", "WSL_PLATFORM_PROVISION_FAILED"))
    return root


def mcp_batch(common: list[str], calls: list[tuple[str, dict]]) -> list[dict]:
    requests = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": {"name": "lattice-installer-acceptance", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"}]
    requests += [{"jsonrpc": "2.0", "id": index, "method": "tools/call", "params": {"name": name, "arguments": arguments}}
                 for index, (name, arguments) in enumerate(calls, 2)]
    result = invoke(common + ["serve"], input_text="".join(json.dumps(request) + "\n" for request in requests), timeout=600)
    if result.returncode or len(result.stdout) > 2_000_000:
        raise Rejected("INSTALL_MCP_PROCESS_FAILED")
    replies = {}
    for line in result.stdout.splitlines():
        reply = json.loads(line)
        if not isinstance(reply, dict) or reply.get("jsonrpc") != "2.0":
            raise Rejected("INSTALL_MCP_PROTOCOL_REJECTED")
        if "id" in reply:
            if reply["id"] in replies:
                raise Rejected("INSTALL_MCP_DUPLICATE_RESPONSE")
            replies[reply["id"]] = reply
    initialized = replies.get(1, {}).get("result", {})
    if not isinstance(initialized, dict) or initialized.get("protocolVersion") != "2025-11-25":
        raise Rejected("INSTALL_MCP_INITIALIZATION_REJECTED")
    outputs = []
    for index, (name, _) in enumerate(calls, 2):
        reply = replies.get(index, {})
        result = reply.get("result", {})
        if not isinstance(result, dict):
            raise Rejected("INSTALL_MCP_RESPONSE_MISSING:" + name)
        if "error" in reply or result.get("isError") is True:
            raise Rejected("INSTALL_MCP_TOOL_REJECTED:" + name)
        payload = result.get("structuredContent")
        if not isinstance(payload, dict):
            raise Rejected("INSTALL_MCP_RESPONSE_MISSING:" + name)
        outputs.append(payload)
    return outputs


def verify_mcp(common: list[str], bundle: Path, state: Path, source: Path,
               locator: dict, graph: dict, *, managed_sample: bool) -> dict:
    config = json.loads((state / "installation.json").read_text(encoding="utf-8"))
    run_id = config.get("run_id")
    if not isinstance(run_id, str) or not re.fullmatch(r"[a-f0-9]{32}", run_id):
        raise Rejected("INSTALL_RUN_ID_REJECTED")
    if managed_sample:
        query = "sample.py"
    else:
        tracked = git_command(bundle, source, "-c", "core.quotepath=false", "ls-files", "--",
                              "*.py", "*.rs", "*.ts", "*.js", "*.go", "*.java", "*.cpp", "*.c")
        if not tracked:
            raise Rejected("MCP_ACCEPTANCE_SOURCE_FILE_REQUIRED")
        query = Path(tracked.splitlines()[0]).name
    health, submitted = mcp_batch(common, [
        ("lattice_runtime_status", {}),
        ("lattice_task_submit", {
            "client_request_id": "installer-" + run_id, "project_id": locator["project_id"],
            "objective": "Verify this local LATTICE installation retains a task and reads Graphify code relations from PostgreSQL across Runtime process restart."}),
    ])
    if health.get("runtime_integration") != "GRAPHIFY" or health.get("graphify_runtime_status") != "PREPARED":
        raise Rejected("MCP_RUNTIME_HEALTH_NOT_VERIFIED")
    if (submitted.get("status") != "SUBMITTED" or submitted.get("task_state") != "DRAFT"
            or submitted.get("project_id") != locator["project_id"] or not submitted.get("task_ref")):
        raise Rejected("MCP_TASK_SUBMISSION_NOT_VERIFIED")
    # A second native Runtime process must retrieve the durable task and graph.
    retained, relations = mcp_batch(common, [
        ("lattice_task_status", {"task_ref": submitted["task_ref"]}),
        ("lattice_code_relations", {"project_id": submitted["project_id"], "commit": graph["commit"], "query": query, "limit": 32}),
    ])
    if (retained.get("status") != "SUBMITTED" or retained.get("task_state") != "DRAFT"
            or any(not submitted.get(key) or retained.get(key) != submitted[key]
                   for key in ("task_ref", "project_id", "objective_digest"))):
        raise Rejected("MCP_TASK_RESTART_READBACK_REJECTED")
    if (relations.get("schema_version") != "lattice.code-relations.v1"
            or relations.get("registered_project_id") != submitted["project_id"]
            or relations.get("commit") != graph["commit"]
            or not graph.get("receipt_digest") or relations.get("source_receipt_digest") != graph["receipt_digest"]
            or not isinstance(relations.get("records"), list) or not relations["records"]):
        raise Rejected("MCP_GRAPH_RESTART_READBACK_REJECTED")
    return {"status": "VERIFIED", "task_ref": retained["task_ref"], "project_id": retained["project_id"],
            "task_state": retained["task_state"], "commit": graph["commit"],
            "source_receipt_digest": graph["receipt_digest"], "relation_count": len(relations["records"]),
            "runtime_process_restart": "VERIFIED", "postgres_process_restart": "NOT_TESTED"}


def run_install(bundle: Path, state: Path, source: Path, wsl: Path, platform_root: Path | None,
                codex_config: Path | None, project_name: str, *, managed_sample: bool = False,
                global_hook: bool = False) -> dict:
    steps = []
    try:
        if codex_config is None:
            raise Rejected("CODEX_CONFIG_REQUIRED")
        python = [str(bundle / "python/python.exe"), "-I", "-B", "-S"]
        bundle_script = str(bundle / "bin/lattice-bundle.py")
        bundle_digest = digest(bundle / "bundle.json")
        # Verify before executing Git, provisioning WSL or changing customer state.
        # The bundle is immutable: do not delete even seemingly disposable .pyc files.
        progress("正在核對已保存的安裝材料，檔案較多，請保持視窗開啟。")
        verification = run_json(python + [bundle_script, "verify", "--bundle", str(bundle), "--sha256", bundle_digest])
        if not successful(verification, "LOCAL_BUNDLE_VERIFIED"):
            return blocked(verification, "BUNDLE_VERIFICATION_FAILED")
        prepare_source(bundle, source, managed_sample=managed_sample)
        runner = str(bundle / "bin/lattice-customer-runtime.py")
        common = python + [runner, "--state", str(state)]
        if state.exists():
            # This read selects no authority. recover() verifies the DPAPI-bound
            # installation, all dependency hashes and the exact cluster itself.
            config = json.loads((state / "installation.json").read_text(encoding="utf-8"))
            expected = {"root": state, "dependency_root": bundle, "graph_source": source, "wsl": wsl}
            if any(not config.get(key) or Path(config[key]).resolve() != path for key, path in expected.items()):
                raise Rejected("CUSTOMER_INSTALLATION_ARGUMENT_CONFLICT")
            payload = run_json(common + ["recover"])
            action = "recover"
        else:
            progress("正在建立 LATTICE 專用的 WSL 環境。")
            platform_root = provision_platform(python, bundle, state, wsl, platform_root)
            arguments = python + [bundle_script, "install", "--bundle", str(bundle), "--sha256", bundle_digest,
                                  "--state", str(state), "--graph-source", str(source), "--wsl", str(wsl),
                                  "--graphify-platform", str(platform_root)]
            progress("正在初始化 LATTICE 與獨立資料庫。")
            payload = run_json(arguments)
            action = "install"
        steps.append({**payload, "action": action})
        if not successful(payload, RUNNING) or payload.get("initialized") is not True:
            return {**blocked(payload, "RUNTIME_INITIALIZATION_NOT_VERIFIED"), "post_install_steps": steps}
        for action, extra, accepted in (
            ("register-project", ["--project-root", str(source), "--project-name", project_name], ("LOCATOR_SAVED", "LOCATOR_REPLAYED")),
            ("graphify-preflight", [], (RUNNING,)),
            ("graphify-refresh", [], (RUNNING,)),
        ):
            progress({"register-project": "正在登記範例或指定的專案。", "graphify-preflight": "正在核對 Graphify 環境。",
                      "graphify-refresh": "正在分析程式並保存 Graphify 關係資料。"}[action])
            payload = run_json(common + [action] + extra)
            steps.append({**payload, "action": action})
            if not successful(payload, *accepted):
                return {**blocked(payload, "INSTALL_STEP_NOT_VERIFIED"), "post_install_steps": steps}
            if action == "register-project":
                locator = payload
            if action == "graphify-refresh":
                evidence = payload.get("operation_evidence")
                if not isinstance(evidence, dict) or evidence.get("component") != "graphify" or evidence.get("status") != "PERSISTED":
                    raise Rejected("GRAPHIFY_REFRESH_NOT_PERSISTED")
        progress("正在透過 MCP 保存任務，並從新程序讀回任務與程式關係。")
        mcp = verify_mcp(common, bundle, state, source, locator, evidence, managed_sample=managed_sample)
        steps.append({**mcp, "action": "mcp-acceptance"})
        payload = run_json(common + ["connect", "--codex-config", str(codex_config)])
        steps.append({**payload, "action": "connect"})
        if not successful(payload, RUNNING):
            return {**blocked(payload, "CODEX_CONNECTION_NOT_VERIFIED"), "post_install_steps": steps}
        if global_hook:
            steps.append({**install_global_hook(codex_config.parent), "action": "global-startup-hook"})
        # Do not dirty a customer's project after accepting its committed graph.
        # Only our managed sample includes a hook, committed before acceptance.
        return {"status": "INSTALLED", "post_install_steps": steps, "source": str(source)}
    except Rejected as error:
        code = str(error)
    except subprocess.TimeoutExpired:
        code = "INSTALL_STEP_TIMED_OUT"
    except (OSError, ValueError, KeyError, TypeError):
        code = "INSTALL_STATE_OR_DEPENDENCY_REJECTED"
    return {"status": "BLOCKED", "code": code, "post_install_steps": steps}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, default=Path(__file__).resolve().parents[1] / "bundle")
    parser.add_argument("--state", type=Path, default=Path(os.environ.get("LOCALAPPDATA", Path.home())) / "LATTICE" / "runtime")
    parser.add_argument("--graph-source", type=Path, help="existing committed Git project; otherwise create a separate managed sample")
    parser.add_argument("--wsl", type=Path, default=Path(os.environ.get("SystemRoot", "C:/Windows")) / "System32" / "wsl.exe")
    parser.add_argument("--graphify-platform", type=Path)
    parser.add_argument("--codex-config", type=Path, default=Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))) / "config.toml")
    parser.add_argument("--project-name", default="LATTICE project")
    parser.add_argument("--report", type=Path)
    parser.add_argument("--install", action="store_true")
    parser.add_argument("--interactive", action="store_true", help="allow installer consent dialogs; never used by unattended checks")
    parser.add_argument("--install-global-hook", action="store_true", help="add managed LATTICE startup guidance for all Codex projects")
    parser.add_argument("--overlap-report", type=Path)
    parser.add_argument("--ack-overlap-warning", action="store_true", help="keep existing settings and acknowledge warnings; never deletes originals")
    args = parser.parse_args()
    bundle, state, wsl = (path.resolve() for path in (args.bundle, args.state, args.wsl))
    source = args.graph_source.resolve() if args.graph_source else state.with_name(state.name + "-sample")
    report = preflight(bundle, state, source, wsl, managed_sample=args.graph_source is None)
    audit = Path(__file__).with_name("lattice-overlap-audit.py")
    overlap_report = args.overlap_report or state.parent / "overlap-audit.json"
    audit_result = run_json([sys.executable, "-I", "-B", "-S", str(audit), "--codex-home", str(args.codex_config.parent),
                             "--project", str(source), "--report", str(overlap_report)])
    report["overlap_audit"] = audit_result
    if not successful(audit_result, "CLEAR", "WARNING"):
        report["status"] = "BLOCKED"
        report["blocked_codes"].append(audit_result.get("code", "OVERLAP_AUDIT_FAILED"))
    elif audit_result["status"] == "WARNING":
        accepted = args.ack_overlap_warning or (args.install and args.interactive and confirm_keep_existing(audit_result))
        if accepted:
            audit_result["user_acknowledged"] = True
            audit_result["choice"] = "KEEP_EXISTING_AND_CONTINUE"
            audit_result["resolution"] = "CANDIDATES_RETAINED_NOT_DEDUPLICATED"
        elif report["status"] == "READY":
            report["status"] = "WARNING_REVIEW_REQUIRED"
            audit_result["choice"] = "STOP_FOR_REVIEW"
    if args.install:
        helper = Path(__file__).with_name("lattice-wsl-host.py")
        host_command = [sys.executable, "-I", "-B", "-S", str(helper)]
        host = run_json(host_command + ["check", "--wsl", str(wsl)], allow_reboot=True)
        audit_ready = successful(audit_result, "CLEAR") or (successful(audit_result, "WARNING") and audit_result.get("user_acknowledged"))
        can_prepare_host = not any(code != "WSL_LAUNCHER_MISSING" for code in report["blocked_codes"])
        if args.interactive and audit_ready and can_prepare_host and host.get("code") == "WSL_SETUP_REQUIRED":
            host = run_json(host_command + ["ensure", "--wsl", str(wsl)], allow_reboot=True, timeout=1860)
            if successful(host, "READY"):
                host = run_json(host_command + ["check", "--wsl", str(wsl)], allow_reboot=True)
                refreshed = preflight(bundle, state, source, wsl, managed_sample=args.graph_source is None)
                report.update(refreshed)
        report["wsl_host"] = host
        if not successful(host, "READY"):
            report["status"] = "REBOOT_REQUIRED" if host.get("status") == "REBOOT_REQUIRED" and host.get("exit_code") == 3 else "BLOCKED"
            report["blocked_codes"].append(host.get("code", "WSL_HOST_NOT_READY"))
    if args.install and report["status"] == "READY":
        # Downloads/extracted archives are disposable. Retain verified dependencies
        # under the installation's local data folder before binding runtime paths.
        destination = state.parent / "bundles"
        helper = Path(__file__).with_name("lattice-install-assets.py")
        progress("正在驗證並保存安裝包；這一步會檢查一萬多個檔案，可能需要數分鐘。")
        staging = run_json([sys.executable, "-I", "-B", "-S", str(helper), "--bundle", str(bundle),
                            "--destination-root", str(destination)], timeout=3600)
        report["bundle_staging"] = staging
        manifest_sha = staging.get("manifest_sha256")
        valid_path = (isinstance(manifest_sha, str) and re.fullmatch(r"[a-f0-9]{64}", manifest_sha)
                      and isinstance(staging.get("bundle"), str)
                      and Path(staging["bundle"]).resolve() == (destination / manifest_sha[:24]).resolve())
        if not successful(staging, "STAGED", "REUSED") or not valid_path:
            report["status"] = "BLOCKED"
            report["blocked_codes"].append(staging.get("code", "BUNDLE_STAGING_NOT_VERIFIED"))
        else:
            bundle = Path(staging["bundle"]).resolve()
            report["bundle"] = str(bundle)
            report["installation"] = run_install(bundle, state, source, wsl,
                                                   args.graphify_platform.resolve() if args.graphify_platform else None,
                                                   args.codex_config.resolve(), args.project_name,
                                                   managed_sample=args.graph_source is None,
                                                   global_hook=args.install_global_hook)
            report["status"] = "INSTALLED" if report["installation"].get("status") == "INSTALLED" else "BLOCKED"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=True))
    return 0 if report["status"] in ("READY", "INSTALLED") else 3 if report["status"] == "REBOOT_REQUIRED" else 2


if __name__ == "__main__":
    raise SystemExit(main())
