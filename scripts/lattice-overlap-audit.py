#!/usr/bin/env python3
"""Audit overlaps; explicitly review exact duplicates for reversible disabling."""
from __future__ import annotations
import argparse, hashlib, importlib.util, json, os, re, tomllib, uuid
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("lattice_overlap_config", Path(__file__).with_name("lattice-mcp-config.py"))
CONFIG = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONFIG)
Rejected = CONFIG.Rejected


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def bounded_label(value: str) -> str:
    # Printable Unicode keeps Chinese names identifiable, while excluding
    # control/bidi characters and invisible leading/trailing whitespace.
    return value if 1 <= len(value) <= 100 and value.isprintable() and value == value.strip() else "名稱已隱藏"


def mcp_signature(server: dict) -> str | None:
    if not isinstance(server, dict) or server.get("enabled", True) is not True:
        return None
    command, url, args = server.get("command"), server.get("url"), server.get("args", [])
    if not ((isinstance(command, str) and isinstance(args, list) and all(isinstance(x, str) for x in args))
            or isinstance(url, str)):
        return None
    # Equal endpoint is insufficient: environment, headers, tool scopes and all
    # other options must also match. Never expose those values in audit output.
    normalized = {key: value for key, value in server.items() if key != "enabled"}
    return digest(json.dumps(normalized, sort_keys=True, default=str).encode())


def item_id(kind: str, target: str, evidence: str) -> str:
    return digest(json.dumps([kind, target, evidence]).encode())


def skill_manifest(folder: Path) -> str | None:
    """Only bounded, regular, complete skill trees qualify for an exact match."""
    try:
        CONFIG.regular_path(folder)
        if not (folder / "SKILL.md").is_file():
            return None
        entries, total = [], 0
        for path in sorted(folder.rglob("*")):
            CONFIG.regular_path(path)
            if len(entries) >= 200 or ".git" in path.relative_to(folder).parts:
                return None
            relative = path.relative_to(folder).as_posix()
            if path.is_dir():
                entries.append([relative, "directory"])
            elif path.is_file():
                total += path.stat().st_size
                if total > 5 * 1024 * 1024:
                    return None
                entries.append([relative, digest(path.read_bytes())])
            else:
                return None
        return digest(json.dumps(entries, sort_keys=True).encode())
    except (OSError, Rejected):
        return None


def review_items(codex_home: Path) -> list[dict]:
    result = []
    config = codex_home / "config.toml"
    try:
        raw = CONFIG.read_config(config)
        servers = CONFIG.parse(raw).get("mcp_servers", {})
        groups = {}
        if isinstance(servers, dict):
            for name, server in servers.items():
                if signature := mcp_signature(server):
                    groups.setdefault(signature, []).append(name)
        for names in groups.values():
            if len(names) < 2:
                continue
            for name in names:
                # The installer owns this block and its sealed management state.
                if name == "lattice" and CONFIG.BEGIN in raw:
                    continue
                try:
                    disable_mcp_bytes(raw, name)
                except Rejected:
                    continue
                result.append({"id": item_id("mcp", name, digest(raw)), "kind": "mcp", "target": name,
                               "label": bounded_label(name), "path": str(config), "source_sha256": digest(raw),
                               "alternatives": [bounded_label(x) for x in names if x != name],
                               "reason": "EXACT_ENABLED_MCP_CONFIGURATION_DUPLICATE", "can_disable": True})
    except (OSError, Rejected, ValueError):
        pass
    skills = codex_home / "skills"
    groups = {}
    if skills.is_dir():
        for folder in sorted(skills.iterdir()):
            if (folder.name.startswith(".") or not folder.is_dir()
                    or bounded_label(folder.name) != folder.name):
                continue
            if signature := skill_manifest(folder):
                groups.setdefault(signature, []).append(folder)
    for signature, folders in groups.items():
        if len(folders) < 2:
            continue
        for folder in folders:
            result.append({"id": item_id("skill", folder.name, signature), "kind": "skill", "target": folder.name,
                           "label": bounded_label(folder.name), "path": str(folder), "source_sha256": signature,
                           "alternatives": [bounded_label(x.name) for x in folders if x != folder],
                           "reason": "EXACT_COMPLETE_SKILL_TREE_DUPLICATE", "can_disable": True})
    return result

PATTERNS = {
    "lattice_startup": re.compile(r"lattice_runtime_status|lattice_task_submit|LATTICE-first|LATTICE MCP", re.I),
    "lattice_install": re.compile(r"lattice.*(install|connect|prepare|bootstrap)|graphify.*(install|refresh)", re.I),
    "repeat_workflow": re.compile(r"heartbeat|schedule|automation|lattice_runtime_status|lattice_task_submit", re.I),
}

def mcp_duplicates(config: Path) -> list[str]:
    """Compare configured, enabled MCP transports without exposing credentials."""
    if not config.exists():
        return []
    try:
        servers = tomllib.loads(config.read_text(encoding="utf-8")).get("mcp_servers", {})
        if not isinstance(servers, dict):
            raise ValueError("invalid MCP table")
    except (OSError, ValueError):
        return ["無法讀取 Codex MCP 設定，尚未完成重複檢查。"]
    groups = {}
    for name, server in servers.items():
        if not isinstance(server, dict) or server.get("enabled") is False:
            continue
        command, url, args = server.get("command"), server.get("url"), server.get("args", [])
        if isinstance(command, str) and isinstance(args, list) and all(isinstance(arg, str) for arg in args):
            signature = ("stdio", os.path.normcase(command), tuple(args))
        elif isinstance(url, str):
            signature = ("http", url.rstrip("/"))
        else:
            continue
        groups.setdefault(signature, []).append(name)
    # Never include URLs, arguments, environment or headers in the report.
    return ["以下啟用中的 MCP 設定指向相同服務，可能重複提供工具：" + ", ".join(bounded_label(name) for name in names)
            for names in groups.values() if len(names) > 1]

def files(root: Path, suffixes: tuple[str, ...], limit: int = 500):
    if not root.is_dir(): return []
    result = []
    for path in root.rglob("*"):
        if len(result) >= limit: break
        if path.is_file() and path.suffix.lower() in suffixes and ".git" not in path.parts:
            result.append(path)
    return result

def audit(codex_home: Path, project: Path) -> dict:
    candidates = []
    candidates += files(codex_home / "skills", (".md", ".toml", ".json"))
    candidates += files(codex_home / "automations", (".toml", ".md", ".json"))
    candidates += files(codex_home / "plugins", (".toml", ".md", ".json", ".yaml", ".yml"))
    candidates += [p for p in (codex_home / "AGENTS.md", codex_home / "config.toml", project / "AGENTS.md") if p.is_file()]
    hits = []
    for path in candidates:
        try:
            CONFIG.regular_path(path)
            if path.stat().st_size > 256 * 1024:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
        except (OSError, Rejected): continue
        kinds = [name for name, pattern in PATTERNS.items() if pattern.search(text)]
        if path.is_relative_to(codex_home / "plugins") and path.suffix.lower() == ".json":
            try:
                metadata = json.loads(text)
                servers = metadata.get("mcpServers", metadata.get("mcp_servers", {})) if isinstance(metadata, dict) else {}
                if isinstance(servers, dict) and servers and re.search(r"\b(?:lattice(?:d)?|graphify)\b", json.dumps(servers), re.I):
                    kinds.append("plugin_mcp_candidate")
            except ValueError:
                pass
        # Generic skill prose is not a conflict. Keep only files that mention
        # LATTICE/MCP behavior or an actual scheduler/heartbeat mechanism.
        if kinds and any(kind in kinds for kind in ("lattice_startup", "lattice_install", "plugin_mcp_candidate")):
            hits.append({"path": str(path), "matches": kinds,
                         "action": "MANUAL_REVIEW_ONLY",
                         "warning": "文字可能與 LATTICE 行為重複；尚不足以判定可停用，不會修改陌生格式。"})
    grouped = {}
    for hit in hits:
        for kind in hit["matches"]: grouped[kind] = grouped.get(kind, 0) + 1
    warnings = mcp_duplicates(codex_home / "config.toml")
    if grouped.get("lattice_startup", 0) > 1: warnings.append("多個來源可能要求重複執行 lattice_runtime_status")
    plugin_hits = [hit for hit in hits if "plugins" in Path(hit["path"]).parts and
                   any(kind in hit["matches"] for kind in ("lattice_startup", "lattice_install", "plugin_mcp_candidate"))]
    if plugin_hits: warnings.append("已安裝的 Codex plugin 可能提供與 LATTICE/Graphify 相同的 MCP 或工作流")
    if grouped.get("lattice_install", 0) > 1: warnings.append("多個來源可能重複安裝或初始化 LATTICE/Graphify")
    if grouped.get("repeat_workflow", 0) > 1 and grouped.get("lattice_startup", 0): warnings.append("LATTICE 啟動規則與 heartbeat/automation 可能重複觸發")
    items = review_items(codex_home)
    if any(item["kind"] == "skill" for item in items):
        warnings.append("部分全域技能的完整內容完全相同，可逐項選擇保留或備份後停用。")
    return {"schema":"lattice.overlap-audit.v1", "status":"WARNING" if warnings or items else "CLEAR",
            "warnings": warnings, "matches": hits,
            "review_items": items, "user_choice_required": bool(warnings or items),
            "assessment": "candidate_overlaps_not_proof_of_duplicate_execution",
            "scope": "bounded_local_config_and_plugin_files; remote_enabled_state_not_verified",
            "policy":"read_only_until_user_approves_each_change"}


def disable_mcp_bytes(raw: bytes, name: str) -> bytes:
    """Recognized normal TOML tables only; semantic readback guards other text."""
    data = CONFIG.parse(raw)
    servers = data.get("mcp_servers", {})
    selected = servers.get(name) if isinstance(servers, dict) else None
    signature = mcp_signature(selected)
    if signature is None or sum(mcp_signature(server) == signature for server in servers.values()) < 2:
        raise Rejected("OVERLAP_LAST_INSTANCE_MUST_REMAIN")
    text = raw.decode("utf-8-sig")
    lines = text.splitlines(keepends=True)
    newline = "\r\n" if "\r\n" in text else "\n"
    # Match exact header variants; quoted exotic names and multiline TOML are
    # intentionally left for manual review, rather than guessing an edit.
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,100}", name):
        raise Rejected("OVERLAP_FORMAT_MANUAL_REVIEW_ONLY")
    header = re.compile(rf'''^[ \t]*\[[ \t]*mcp_servers\.(?:{re.escape(name)}|"{re.escape(name)}"|'{re.escape(name)}')[ \t]*\][ \t]*(?:\#[^\r\n]*)?(?:\r?\n)?$''')
    locations = [i for i, line in enumerate(lines) if header.fullmatch(line)]
    if len(locations) != 1:
        raise Rejected("OVERLAP_FORMAT_MANUAL_REVIEW_ONLY")
    start = locations[0]
    end = next((i for i in range(start + 1, len(lines)) if lines[i].lstrip().startswith("[")), len(lines))
    if "enabled" in selected:
        setting = re.compile(r'''^(?P<prefix>[ \t]*(?:enabled|"enabled"|'enabled')[ \t]*=[ \t]*)true(?P<suffix>[ \t]*(?:\#[^\r\n]*)?(?:\r?\n)?)$''')
        matches = [(i, match) for i in range(start + 1, end) if (match := setting.fullmatch(lines[i]))]
        if len(matches) != 1:
            raise Rejected("OVERLAP_FORMAT_MANUAL_REVIEW_ONLY")
        index, match = matches[0]
        lines[index] = match["prefix"] + "false" + match["suffix"]
    else:
        if not lines[start].endswith("\n"):
            lines[start] += newline
        lines.insert(start + 1, "enabled = false" + newline)
    after = (b"\xef\xbb\xbf" if raw.startswith(b"\xef\xbb\xbf") else b"") + "".join(lines).encode("utf-8")
    selected["enabled"] = False
    if CONFIG.parse(after) != data:
        raise Rejected("OVERLAP_UNRELATED_SETTINGS_CHANGE_REJECTED")
    return after


class SkillDisableRejected(Rejected):
    def __init__(self, code: str, reason: str, source: Path, backup: Path | None):
        super().__init__(code)
        self.details = {"reason": reason, "source": str(source), "restored": backup is None}
        if backup is not None:
            self.details["backup"] = str(backup)


def restore_skill_backup(codex_home: Path, source: Path, backup: Path, reason: str) -> None:
    """Undo a failed move only into the still-absent original Windows path."""
    restored = False
    try:
        CONFIG.regular_path(source)
        CONFIG.regular_path(backup)
        home = codex_home.resolve()
        if (source.resolve().parent == home / "skills"
                and backup.resolve().parent == home / ".lattice-overlap-backups" / "skills"
                and not os.path.lexists(source) and backup.is_dir()):
            # Windows rename refuses an existing destination, including a path
            # another process creates after the absence check. Never replace it.
            backup.rename(source)
            restored = source.is_dir() and not os.path.lexists(backup)
    except (OSError, Rejected):
        pass
    raise SkillDisableRejected(
        "OVERLAP_SKILL_DISABLE_ROLLED_BACK" if restored else "OVERLAP_SKILL_RESTORE_REQUIRED",
        reason, source, None if restored else backup)


def disable_item(codex_home: Path, project: Path, selected_id: str) -> dict:
    """Called only after explicit choice; revalidate exact evidence under lock."""
    CONFIG.regular_path(codex_home)
    items = review_items(codex_home)
    item = next((candidate for candidate in items if candidate["id"] == selected_id), None)
    if item is None:
        raise Rejected("OVERLAP_ITEM_CHANGED_OR_NO_LONGER_DUPLICATE")
    config = codex_home / "config.toml"
    folder, _, pending = CONFIG.locations(config)
    lock = folder if item["kind"] == "mcp" else codex_home / ".lattice-overlap-review"
    with CONFIG.manager_lock(lock):
        fresh = next((candidate for candidate in review_items(codex_home) if candidate["id"] == selected_id), None)
        if fresh is None:
            raise Rejected("OVERLAP_ITEM_CHANGED_OR_NO_LONGER_DUPLICATE")
        if item["kind"] == "mcp":
            if pending.exists():
                raise Rejected("CONFIG_RECOVERY_REQUIRED")
            raw = CONFIG.read_config(config)
            if digest(raw) != item["source_sha256"]:
                raise Rejected("OVERLAP_ITEM_CHANGED")
            after = disable_mcp_bytes(raw, item["target"])
            backup = folder / (uuid.uuid4().hex + ".overlap.config.bak")
            descriptor = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "wb") as stream:
                CONFIG.copy_access(config, backup)
                stream.write(raw)
                stream.flush()
                os.fsync(stream.fileno())
            if os.name == "nt":
                CONFIG.write_config(config, raw, after, True)
            else:
                if CONFIG.read_config(config) != raw:
                    raise Rejected("OVERLAP_ITEM_CHANGED")
                CONFIG.atomic_write(config, after)
            if CONFIG.read_config(config) != after:
                raise Rejected("OVERLAP_CONFIG_READBACK_MISMATCH")
        else:
            source = codex_home / "skills" / item["target"]
            archive = codex_home / ".lattice-overlap-backups" / "skills"
            CONFIG.regular_path(source)
            CONFIG.regular_path(archive)
            home = codex_home.resolve()
            # Both resolved move endpoints must remain in this Codex home.
            if (source.resolve().parent != home / "skills"
                    or archive.resolve() != home / ".lattice-overlap-backups" / "skills"):
                raise Rejected("OVERLAP_MOVE_SCOPE_REJECTED")
            if skill_manifest(source) != item["source_sha256"]:
                raise Rejected("OVERLAP_ITEM_CHANGED")
            archive.mkdir(parents=True, exist_ok=True, mode=0o700)
            backup = archive / (uuid.uuid4().hex + "-" + source.name)
            source.rename(backup)
            try:
                if os.path.lexists(source) or skill_manifest(backup) != item["source_sha256"]:
                    raise Rejected("OVERLAP_SKILL_BACKUP_READBACK_MISMATCH")
                if not any(not sibling.name.startswith(".") and sibling.is_dir()
                           and skill_manifest(sibling) == item["source_sha256"]
                           for sibling in (home / "skills").iterdir()):
                    raise Rejected("OVERLAP_LAST_INSTANCE_MUST_REMAIN")
            except (OSError, Rejected) as error:
                reason = str(error) if isinstance(error, Rejected) else "OVERLAP_SKILL_READBACK_IO_REJECTED"
                restore_skill_backup(codex_home, source, backup, reason)
    return {"item_id": selected_id, "kind": item["kind"], "label": item["label"],
            "status": "DISABLED_BACKUP_SAVED", "backup": str(backup), "reload_required": True}


def ask_yes_no(message: str, title: str) -> bool:
    if os.name != "nt":
        raise Rejected("OVERLAP_REVIEW_DIALOG_UNAVAILABLE")
    import ctypes
    from ctypes import wintypes
    api = ctypes.WinDLL("user32", use_last_error=True)
    api.MessageBoxW.argtypes = [wintypes.HWND, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.UINT]
    api.MessageBoxW.restype = ctypes.c_int
    choice = api.MessageBoxW(None, message, title, 0x4 | 0x20 | 0x100)
    if choice not in (6, 7):
        raise Rejected("OVERLAP_REVIEW_DIALOG_UNAVAILABLE")
    return choice == 6


def confirm_disable(item: dict) -> bool:
    category = "MCP 設定" if item["kind"] == "mcp" else "完整技能目錄"
    return ask_yes_no(
        f'發現完全相同的{category}：{item["label"]}\n'
        f'另外仍啟用的相同項目：{", ".join(item["alternatives"])}\n\n'
        f'是否備份並停用「{item["label"]}」？\n'
        '選「是」會保留完整備份，不永久刪除。\n選「否」保留此項。\n'
        '只憑名稱或文字相似的項目不會提供自動停用。\n變更在重新啟動 Codex 後生效。',
        "LATTICE：逐項檢查重複設定")


def review(codex_home: Path, project: Path, *, interactive: bool = False) -> dict:
    initial = audit(codex_home, project)
    if not interactive:
        return {**initial, "user_acknowledged": not initial["user_choice_required"],
                "choice": "REVIEW_REQUIRED" if initial["user_choice_required"] else "NO_CONFLICT",
                "resolution": [], "after_audit": initial}
    resolutions = []
    for planned in initial["review_items"]:
        # Earlier approved changes can eliminate duplicates or change the config
        # hash. Offer the new exact plan, never reuse the stale approval.
        current = next((item for item in review_items(codex_home)
                        if item["kind"] == planned["kind"] and item["target"] == planned["target"]), None)
        if current is None:
            continue
        if confirm_disable(current):
            resolutions.append(disable_item(codex_home, project, current["id"]))
        else:
            resolutions.append({"item_id": current["id"], "kind": current["kind"],
                                "label": current["label"], "status": "KEPT"})
    after = audit(codex_home, project)
    acknowledged = True
    if after["user_choice_required"]:
        message = "以下項目仍需保留或人工確認：\n\n" + "\n".join(after["warnings"][:12])
        message += "\n\n文字相似的技能、工作流和插件不代表重複執行。\n未知格式及原生排程不會自動修改。\n要保留目前設定並繼續安裝嗎？"
        acknowledged = ask_yes_no(message, "LATTICE：保留其餘設定")
    changed = any(item["status"] == "DISABLED_BACKUP_SAVED" for item in resolutions)
    return {**after, "user_acknowledged": acknowledged,
            "choice": "STOP" if not acknowledged else "BACKUP_DISABLED_AND_KEPT_REMAINDER" if changed else "KEEP_EXISTING",
            "resolution": resolutions, "after_audit": after}

def main() -> int:
    p = argparse.ArgumentParser(); p.add_argument("--codex-home", type=Path, default=Path(os.environ.get("CODEX_HOME", Path.home()/".codex")))
    p.add_argument("--project", type=Path, default=Path.cwd()); p.add_argument("--report", type=Path, required=True)
    p.add_argument("--review", action="store_true")
    p.add_argument("--interactive", action="store_true")
    args = p.parse_args()
    try:
        home, project = args.codex_home.absolute(), args.project.absolute()
        CONFIG.regular_path(home)
        CONFIG.regular_path(project)
        report_path = args.report.absolute()
        CONFIG.regular_path(report_path)
        resolved_report = report_path.resolve()
        if (resolved_report in {home / "config.toml", home / "AGENTS.md", project / "AGENTS.md"}
                or any(resolved_report.is_relative_to(home / name) for name in ("skills", "plugins", "automations"))):
            raise Rejected("OVERLAP_REPORT_OVERLAPS_SOURCE")
        result = review(home, project, interactive=args.interactive) if args.review else audit(home, project)
        CONFIG.regular_path(args.report)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        CONFIG.atomic_write(args.report, CONFIG.json_bytes(result))
        print(json.dumps(result, ensure_ascii=True))
        return 0
    except (OSError, Rejected) as error:
        result = {"status": "BLOCKED", "code": str(error) if isinstance(error, Rejected) else "OVERLAP_IO_REJECTED"}
        if isinstance(error, SkillDisableRejected):
            result.update(error.details)
        print(json.dumps(result))
        return 2

if __name__ == "__main__": raise SystemExit(main())
