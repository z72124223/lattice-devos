#!/usr/bin/env python3
"""Fetch the exact Ubuntu WSL image pinned by LATTICE and verify its digest."""
from __future__ import annotations
import argparse, hashlib, json, urllib.request
from pathlib import Path

URL = "https://releases.ubuntu.com/26.04/ubuntu-26.04.1-wsl-amd64.wsl"
SHA256 = "48d56724b5c8e60f24893e83e73bbb58c60b3ca22fba3da977075420acd54104"

def main() -> int:
    parser = argparse.ArgumentParser(); parser.add_argument("--output", type=Path, required=True); args = parser.parse_args()
    output = args.output.resolve()
    if output.exists(): print(json.dumps({"status":"BLOCKED","code":"OUTPUT_ALREADY_EXISTS"})); return 2
    output.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(URL, headers={"User-Agent": "LATTICE-pinned-supply/1"})
    digest = hashlib.sha256(); total = 0
    with urllib.request.urlopen(request, timeout=120) as response, output.open("xb") as stream:
        while True:
            chunk = response.read(1024 * 1024)
            if not chunk: break
            stream.write(chunk); digest.update(chunk); total += len(chunk)
    actual = digest.hexdigest()
    if actual != SHA256:
        output.unlink(missing_ok=True); print(json.dumps({"status":"BLOCKED","code":"WSL_IMAGE_DIGEST_MISMATCH","sha256":actual})); return 2
    output.with_suffix(output.suffix + ".provenance.json").write_text(json.dumps({"source":URL,"sha256":SHA256,"bytes":total,"license":"Ubuntu licensing terms"}, indent=2)+"\n", encoding="utf-8")
    print(json.dumps({"status":"WSL_IMAGE_VERIFIED","path":str(output),"sha256":SHA256,"bytes":total})); return 0

if __name__ == "__main__": raise SystemExit(main())
