#!/usr/bin/env python3
"""Export a portable, secret-free Codex preference profile for LATTICE installs."""
from __future__ import annotations
import argparse, json, os, re
from pathlib import Path
import tomllib

SCHEMA = "lattice.codex-portable-profile.v1"
SECRET = re.compile(r"(password|token|secret|credential|api[_-]?key|auth|cookie|private|identity|digest|port|launcher|home|root|path)", re.I)
KEEP_TOP = {"model", "model_provider", "approval_policy", "sandbox_mode", "network_access", "reasoning_effort", "desktop", "features", "history", "ui", "locale"}

def safe_value(key: str, value):
    if SECRET.search(key): return None
    if isinstance(value, dict):
        return {k: v for k, raw in value.items() if (v := safe_value(k, raw)) is not None}
    if isinstance(value, list): return [x for x in (safe_value(key, item) for item in value) if x is not None]
    return value if isinstance(value, (str, int, float, bool)) or value is None else None

def export(config: Path, output: Path) -> dict:
    data = tomllib.loads(config.read_text(encoding="utf-8")) if config.exists() else {}
    portable = {key: safe_value(key, value) for key, value in data.items() if key in KEEP_TOP}
    portable = {key: value for key, value in portable.items() if value is not None}
    result = {"schema": SCHEMA, "source": "local Codex preferences", "secrets_included": False,
              "absolute_paths_included": False, "preferences": portable,
              "apply_policy": "rebuild_local_paths_and_MCP; never_copy_identity_or_credentials"}
    output.parent.mkdir(parents=True, exist_ok=True); output.write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    return result

def main() -> int:
    p = argparse.ArgumentParser(); p.add_argument("--config", type=Path, default=Path(os.environ.get("CODEX_HOME", Path.home()/".codex"))/"config.toml"); p.add_argument("--output", type=Path, required=True)
    args = p.parse_args(); result = export(args.config.resolve(), args.output.resolve()); print(json.dumps(result, ensure_ascii=True)); return 0

if __name__ == "__main__": raise SystemExit(main())
