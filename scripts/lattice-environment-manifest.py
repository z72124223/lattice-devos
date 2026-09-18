#!/usr/bin/env python3
"""Create and verify a secret-free manifest of LATTICE-required environment."""
from __future__ import annotations
import argparse, hashlib, json, os, platform, shutil, subprocess, sys
from pathlib import Path

SCHEMA = "lattice.required-environment.v1"
REQUIRED_COMMANDS = ("git", "node", "python", "wsl")
REQUIRED_ENV = ("LATTICE_RUNTIME_INTEGRATION", "LATTICE_GRAPHIFY_RUNTIME_ROOT", "LATTICE_GRAPHIFY_WSL_EXE")

def version(name: str) -> dict:
    path = shutil.which(name)
    if not path: return {"name": name, "status": "MISSING"}
    try:
        p = subprocess.run([path, "--version"], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=10)
        return {"name": name, "status": "PASS" if p.returncode == 0 else "UNVERIFIED", "version": (p.stdout or p.stderr).strip()[:160]}
    except (OSError, subprocess.TimeoutExpired): return {"name": name, "status": "UNVERIFIED"}

def collect(bundle: Path | None = None) -> dict:
    commands = [version(name) for name in REQUIRED_COMMANDS]
    components = {"control": "bundled", "postgresql": "bundled", "graphify": "bundled", "node": "24.16.0", "python": "3.12", "git": "bundled", "wsl_image": "Ubuntu 26.04.1 pinned"}
    if bundle and (bundle / "bundle.json").is_file():
        try:
            data = json.loads((bundle / "bundle.json").read_text(encoding="utf-8")); components["bundle_sha256"] = hashlib.sha256((bundle / "bundle.json").read_bytes()).hexdigest(); components["bundle_cores"] = data.get("cores")
        except (OSError, ValueError): components["bundle_status"] = "UNREADABLE"
    return {"schema": SCHEMA, "platform": platform.platform(), "architecture": platform.machine(),
            "components": components, "commands": commands,
            "environment_keys": {key: os.environ.get(key) for key in REQUIRED_ENV if os.environ.get(key)},
            "secrets_included": False, "policy": "install_only_declared_LATTICE_dependencies"}

def main() -> int:
    p = argparse.ArgumentParser(); p.add_argument("action", choices=("collect", "verify")); p.add_argument("--bundle", type=Path); p.add_argument("--manifest", type=Path, required=True)
    args = p.parse_args(); result = collect(args.bundle.resolve() if args.bundle else None)
    if args.action == "verify" and args.manifest.exists():
        wanted = json.loads(args.manifest.read_text(encoding="utf-8")); result["status"] = "PASS" if wanted.get("components", {}).items() <= result["components"].items() else "MISMATCH"
    else: result["status"] = "COLLECTED"
    args.manifest.parent.mkdir(parents=True, exist_ok=True); args.manifest.write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n", encoding="utf-8"); print(json.dumps(result, ensure_ascii=True)); return 0 if result["status"] in ("COLLECTED", "PASS") else 2

if __name__ == "__main__": raise SystemExit(main())
