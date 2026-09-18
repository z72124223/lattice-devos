#!/usr/bin/env python3
"""Read-only audit for duplicated Codex/LATTICE skills, hooks and workflows."""
from __future__ import annotations
import argparse, json, os, re
from pathlib import Path

PATTERNS = {
    "lattice_startup": re.compile(r"lattice_runtime_status|lattice_task_submit|LATTICE-first|LATTICE MCP", re.I),
    "lattice_install": re.compile(r"lattice.*(install|connect|prepare|bootstrap)|graphify.*(install|refresh)", re.I),
    "repeat_workflow": re.compile(r"heartbeat|schedule|automation|lattice_runtime_status|lattice_task_submit", re.I),
}

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
        try: text = path.read_text(encoding="utf-8", errors="replace")
        except OSError: continue
        kinds = [name for name, pattern in PATTERNS.items() if pattern.search(text)]
        # Generic skill prose is not a conflict. Keep only files that mention
        # LATTICE/MCP behavior or an actual scheduler/heartbeat mechanism.
        if kinds and ("lattice_startup" in kinds or "lattice_install" in kinds or
                      ("repeat_workflow" in kinds and re.search(r"heartbeat|automation|schedule", text, re.I))):
            hits.append({"path": str(path), "matches": kinds,
                         "warning": "可能與 LATTICE 安裝後的 MCP/AGENTS/heartbeat 行為重複；未經同意不會修改。"})
    grouped = {}
    for hit in hits:
        for kind in hit["matches"]: grouped[kind] = grouped.get(kind, 0) + 1
    warnings = []
    if grouped.get("lattice_startup", 0) > 1: warnings.append("多個來源可能要求重複執行 lattice_runtime_status")
    plugin_hits = [hit for hit in hits if "plugins" in Path(hit["path"]).parts and
                   ("lattice_startup" in hit["matches"] or "lattice_install" in hit["matches"])]
    if plugin_hits: warnings.append("已安裝的 Codex plugin 可能提供與 LATTICE/Graphify 相同的 MCP 或工作流")
    if grouped.get("lattice_install", 0) > 1: warnings.append("多個來源可能重複安裝或初始化 LATTICE/Graphify")
    if grouped.get("repeat_workflow", 0) > 1 and grouped.get("lattice_startup", 0): warnings.append("LATTICE 啟動規則與 heartbeat/automation 可能重複觸發")
    return {"schema":"lattice.overlap-audit.v1", "status":"WARNING" if warnings else "CLEAR",
            "warnings": warnings, "matches": hits,
            "user_choice_required": bool(warnings),
            "policy":"read_only_until_user_approves_each_change"}

def main() -> int:
    p = argparse.ArgumentParser(); p.add_argument("--codex-home", type=Path, default=Path(os.environ.get("CODEX_HOME", Path.home()/".codex")))
    p.add_argument("--project", type=Path, default=Path.cwd()); p.add_argument("--report", type=Path, required=True)
    args = p.parse_args(); result = audit(args.codex_home.resolve(), args.project.resolve())
    args.report.parent.mkdir(parents=True, exist_ok=True); args.report.write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    print(json.dumps(result, ensure_ascii=True)); return 0

if __name__ == "__main__": raise SystemExit(main())
