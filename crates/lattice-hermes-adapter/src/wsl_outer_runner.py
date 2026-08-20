"""Pinned outer WSL runner for the inherited-FD containment bridge.

This process remains outside bubblewrap. It creates an AF_UNIX socketpair,
connects the child endpoint to the bubblewrap child's stdin/stdout, and keeps
the peer for the bounded framed Windows relay. No filesystem Unix socket
crosses the Windows/WSL boundary.
"""

import hashlib
import json
import os
import socket
import subprocess
import sys


MAGIC = b"LATTICE_HERMES_SOCKETPAIR_V1\n"
APPROVED_BWRAP_PACKAGE_VERSION = "0.11.1-1ubuntu0.1"
APPROVED_BWRAP_PACKAGE_SOURCE = "Ubuntu 26.04 LTS resolute-security USN-8288-1 CVE-2026-41163"
APPROVED_BWRAP_PACKAGE_DEB_SHA256 = (
    "b353088d1003adb3f760deeccfb84c47928a36c8dc102bf680efc94eb19f4408"
)
EXPECTED_BWRAP_SHA256 = "0abea81db798ebf6b4742ac0664802d97521547a353c2a0dbdc21d76cbbfd2c0"
EXPECTED_PYTHON_VERSION = (3, 12, 13)
MAX_DIAGNOSTIC_BYTES = 4096


def fail(code):
    os.write(2, ("HERMES_OUTER_FAIL:%d\n" % code).encode("ascii"))
    raise SystemExit(code)


def checked_nonce(value):
    if len(value) != 64 or any(char not in "0123456789abcdef" for char in value):
        fail(64)
    return value


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb", buffering=0) as source:
        while True:
            chunk = source.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def child_code():
    return """import os,sys
nonce = bytes.fromhex(sys.argv[1])
os.write(1, b'LATTICE_SOCKETPAIR_CANARY:' + nonce)
expected = b'LATTICE_SOCKETPAIR_ACK:' + nonce
received = bytearray()
while len(received) < len(expected):
    chunk = os.read(0, len(expected) - len(received))
    if not chunk:
        raise SystemExit(81)
    received.extend(chunk)
if bytes(received) != expected:
    raise SystemExit(82)
"""


def bwrap_command(runtime_root, nonce):
    if not runtime_root.startswith("/var/tmp/lattice-runtime-targets/"):
        fail(64)
    return [
        "/usr/bin/bwrap",
        "--die-with-parent",
        "--unshare-all",
        "--unshare-user",
        "--disable-userns",
        "--assert-userns-disabled",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind",
        "/lib",
        "/lib",
        "--ro-bind",
        "/lib64",
        "/lib64",
        "--ro-bind",
        runtime_root,
        "/runtime-input",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--clearenv",
        "--setenv",
        "LANG",
        "C.UTF-8",
        "--setenv",
        "LC_ALL",
        "C.UTF-8",
        "--chdir",
        "/tmp",
        "/runtime-input/python/bin/python3.12",
        "-I",
        "-S",
        "-B",
        "-c",
        child_code(),
        nonce,
    ]


def emit_receipt(receipt):
    encoded = json.dumps(receipt, ensure_ascii=True, separators=(",", ":"), sort_keys=True).encode(
        "ascii"
    )
    sys.stdout.buffer.write(MAGIC)
    sys.stdout.buffer.write(len(encoded).to_bytes(8, "big"))
    sys.stdout.buffer.write(encoded)
    sys.stdout.buffer.flush()


def socketpair_canary(runtime_root, nonce):
    if sys.version_info[:3] != EXPECTED_PYTHON_VERSION:
        fail(65)
    bwrap_sha256 = file_sha256("/usr/bin/bwrap")
    if bwrap_sha256 != EXPECTED_BWRAP_SHA256:
        fail(66)
    peer, child_endpoint = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    process = None
    try:
        process = subprocess.Popen(
            bwrap_command(runtime_root, nonce),
            stdin=child_endpoint,
            stdout=child_endpoint,
            stderr=subprocess.PIPE,
            close_fds=True,
        )
        child_endpoint.close()
        expected = b"LATTICE_SOCKETPAIR_CANARY:" + bytes.fromhex(nonce)
        received = bytearray()
        peer.settimeout(5.0)
        while len(received) < len(expected):
            chunk = peer.recv(len(expected) - len(received))
            if not chunk:
                _, child_stderr = process.communicate(timeout=1.0)
                diagnostic = (
                    "HERMES_OUTER_CHILD_EXIT:%s:%s:%s\n"
                    % (
                        process.returncode,
                        hashlib.sha256(b"").hexdigest(),
                        hashlib.sha256(child_stderr).hexdigest(),
                    )
                )
                os.write(2, diagnostic.encode("ascii"))
                fail(67)
            received.extend(chunk)
        if bytes(received) != expected:
            fail(67)
        peer.sendall(b"LATTICE_SOCKETPAIR_ACK:" + bytes.fromhex(nonce))
        _, stderr = process.communicate(timeout=5.0)
        if process.returncode != 0 or stderr or len(stderr) > MAX_DIAGNOSTIC_BYTES:
            fail(68)
    except (OSError, subprocess.SubprocessError):
        if process is not None:
            process.kill()
            process.wait()
        fail(68)
    finally:
        peer.close()
        child_endpoint.close()
    binding = hashlib.sha256(bytes.fromhex(nonce) + b"LATTICE_SOCKETPAIR_CANARY").hexdigest()
    emit_receipt(
        {
            "broker_read_fd": 0,
            "broker_write_fd": 1,
            "bwrap_package_deb_sha256": APPROVED_BWRAP_PACKAGE_DEB_SHA256,
            "bwrap_package_source": APPROVED_BWRAP_PACKAGE_SOURCE,
            "bwrap_package_version": APPROVED_BWRAP_PACKAGE_VERSION,
            "bwrap_sha256": bwrap_sha256,
            "descendants_reaped": True,
            "nonce_binding_sha256": binding,
            "python_version": "3.12.13",
            "schema": "lattice.hermes.socketpair-receipt.v2",
        }
    )


def main(arguments):
    if len(arguments) != 3 or arguments[0] != "socketpair-canary":
        fail(64)
    socketpair_canary(arguments[1], checked_nonce(arguments[2]))


if __name__ == "__main__":
    main(sys.argv[1:])
