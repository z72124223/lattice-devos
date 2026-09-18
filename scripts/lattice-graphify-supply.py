#!/usr/bin/env python3
"""Fetch the pinned public Graphify wheel into a reviewable supply directory."""
from __future__ import annotations
import argparse, hashlib, json, shutil, tempfile, urllib.request, zipfile
from pathlib import Path

URL = "https://files.pythonhosted.org/packages/0e/39/ea6555c576729c9ff597e20cf2c10851774fc81b1d392036ad212963b06b/graphifyy-0.9.33-py3-none-any.whl"
SHA256 = "c32b5792c783a6e66b1100b35bc65df3538e3f69b9df45fb098c9634c1b8eb01"

def main() -> int:
    p = argparse.ArgumentParser(); p.add_argument("--output", type=Path, required=True); args = p.parse_args()
    out = args.output.resolve()
    if out.exists(): print(json.dumps({"status":"BLOCKED","code":"OUTPUT_ALREADY_EXISTS"})); return 2
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="lattice-graphify-") as tmp:
        wheel = Path(tmp) / "graphifyy-0.9.33-py3-none-any.whl"
        urllib.request.urlretrieve(URL, wheel)
        h = hashlib.sha256(wheel.read_bytes()).hexdigest()
        if h != SHA256: print(json.dumps({"status":"BLOCKED","code":"GRAPHIFY_WHEEL_DIGEST_MISMATCH","sha256":h})); return 2
        out.mkdir()
        with zipfile.ZipFile(wheel) as archive: archive.extractall(out / "site-packages")
        (out / "provenance.json").write_text(json.dumps({"package":"graphifyy","version":"0.9.33","url":URL,"sha256":SHA256,"license":"Apache-2.0"}, indent=2)+"\n", encoding="utf-8")
    print(json.dumps({"status":"GRAPHIFY_WHEEL_VERIFIED","path":str(out),"sha256":SHA256})); return 0

if __name__ == "__main__": raise SystemExit(main())
