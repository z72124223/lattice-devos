#!/usr/bin/env python3
"""Safe single-entry Windows installer for a verified LATTICE bundle."""
from __future__ import annotations
import argparse, hashlib, json, os, platform, shutil, subprocess, time
from pathlib import Path

SCHEMA = "lattice.one-click-install.v1"
REQUIRED = ("bundle.json", "bin/latticed.exe", "postgres/bin", "git/cmd/git.exe", "node/node.exe", "graphify")

def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""): h.update(chunk)
    return h.hexdigest()

def command(name: str) -> dict:
    found = shutil.which(name)
    if not found: return {"name": name, "status": "MISSING"}
    try:
        result = subprocess.run([found, "--version"], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=10,
                                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        return {"name": name, "status": "PASS" if result.returncode == 0 else "UNVERIFIED", "path": found,
                "version": (result.stdout or result.stderr).strip()[:160]}
    except (OSError, subprocess.TimeoutExpired): return {"name": name, "status": "UNVERIFIED", "path": found}

def preflight(bundle: Path, state: Path, source: Path, wsl: Path | None) -> dict:
    checks = [{"check": "windows", "status": "PASS" if os.name == "nt" else "BLOCKED", "version": platform.version() if os.name == "nt" else None}]
    for name in ("python", "git", "node", "wsl"): checks.append({"check": name, **command(name)})
    if not bundle.is_dir(): checks.append({"check": "bundle", "status": "BLOCKED", "code": "BUNDLE_MISSING"})
    else:
        missing = [name for name in REQUIRED if not (bundle / name).exists()]
        checks.append({"check": "bundle", "status": "PASS" if not missing else "BLOCKED", "missing": missing})
        manifest = bundle / "bundle.json"
        try:
            data = json.loads(manifest.read_text(encoding="utf-8"))
            checks.append({"check": "manifest", "status": "PASS" if data.get("schema") == "lattice.bundle.v1" else "BLOCKED",
                           "schema": data.get("schema"), "cores": data.get("cores"), "full_dependency_portability": data.get("full_dependency_portability")})
        except (OSError, ValueError): checks.append({"check": "manifest", "status": "BLOCKED", "code": "BUNDLE_MANIFEST_UNREADABLE"})
    checks += [{"check": "state", "status": "PASS" if not state.exists() else "BLOCKED", "code": None if not state.exists() else "STATE_ALREADY_EXISTS"},
               {"check": "graph-source", "status": "PASS" if source.is_dir() else "BLOCKED", "code": None if source.is_dir() else "GRAPH_SOURCE_MISSING"},
               {"check": "wsl-path", "status": "PASS" if wsl and wsl.is_file() else "BLOCKED", "code": None if wsl and wsl.is_file() else "WSL_LAUNCHER_MISSING"}]
    blocked = [x for x in checks if x["status"] == "BLOCKED"]
    return {"schema": SCHEMA, "status": "READY" if not blocked else "BLOCKED", "created_at": int(time.time()),
            "bundle": str(bundle), "state": str(state), "checks": checks,
            "blocked_codes": [x.get("code") for x in blocked if x.get("code")]}

def run_install(bundle: Path, state: Path, source: Path, wsl: Path) -> dict:
    python = bundle / "python/python.exe"
    if not python.is_file(): return {"status": "BLOCKED", "code": "BUNDLED_PYTHON_MISSING"}
    command = [str(python), "-I", "-B", str(bundle / "bin/lattice-bundle.py"), "install", "--bundle", str(bundle),
               "--sha256", digest(bundle / "bundle.json"), "--state", str(state), "--graph-source", str(source), "--wsl", str(wsl)]
    result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", errors="replace",
                            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    try: payload = json.loads(result.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError): payload = {"status": "BLOCKED", "code": "INSTALL_OUTPUT_UNREADABLE"}
    payload["exit_code"] = result.returncode
    return payload

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, default=Path(__file__).resolve().parents[1] / "bundle")
    parser.add_argument("--state", type=Path, default=Path(os.environ.get("LOCALAPPDATA", Path.home())) / "LATTICE" / "runtime")
    parser.add_argument("--graph-source", type=Path, required=True); parser.add_argument("--wsl", type=Path, required=True)
    parser.add_argument("--report", type=Path); parser.add_argument("--install", action="store_true")
    args = parser.parse_args(); bundle, state, source, wsl = (p.resolve() for p in (args.bundle, args.state, args.graph_source, args.wsl))
    report = preflight(bundle, state, source, wsl)
    if args.install and report["status"] == "READY":
        report["installation"] = run_install(bundle, state, source, wsl)
        report["status"] = "INSTALLED" if report["installation"].get("status") not in ("BLOCKED", "FAILED") else "BLOCKED"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True); args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    # Windows consoles are often CP950; keep the machine-readable stdout ASCII-safe.
    print(json.dumps(report, ensure_ascii=True)); return 0 if report["status"] in ("READY", "INSTALLED") else 2

if __name__ == "__main__": raise SystemExit(main())
