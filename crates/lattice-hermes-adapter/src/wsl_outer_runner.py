"""Pinned outer WSL runner for the inherited-FD containment bridge.

This process remains outside bubblewrap. It creates one AF_UNIX socketpair for
the control stream and a second for the bubblewrap child's FD2 Codex proxy,
then bridges the proxy peer through bounded Windows-owner pipes. No filesystem
Unix socket crosses the Windows/WSL boundary.
"""

import hashlib
import json
import os
import select
import socket
import stat
import subprocess
import sys
import threading
import time


MAGIC = b"LATTICE_HERMES_SOCKETPAIR_V1\n"
STARTUP_MAGIC = b"LATTICE_HERMES_PRODUCTION_START_V1\n"
INIT_MAGIC = b"LATTICE_HERMES_PRODUCTION_INIT_V1\n"
HTTP_REQUEST_MAGIC = b"LATTICE_HERMES_HTTP_REQUEST_V1\n"
HTTP_RESPONSE_MAGIC = b"LATTICE_HERMES_HTTP_RESPONSE_V1\n"
CONTAINMENT_MAGIC = b"LATTICE_HERMES_CONTAINED_V2\n"
EXPECTED_BWRAP_SHA256 = "8e19e40e7d5f7a7e8b488c7926feb040eab6ed10c58fa360e266d2f70670e92b"
EXPECTED_PYTHON_VERSION = (3, 12, 13)
EXPECTED_OFFICIAL_RUNTIME_ROOT = (
    "/var/tmp/lattice-runtime-targets/"
    "hermes-v2026.8.3-cpython-3.12.13-pbs-20260804-errorfix-v1"
)
EXPECTED_OFFLINE_MANIFEST_SHA256 = (
    "e3a3272b6cead30cd2df1af755df031766475595fdacfb080d0886671b6d1fbb"
)
EXPECTED_RUNTIME_TREE_SHA256 = (
    "cb0e331bcb2b4fe2fd0977401d246819aadb800b645ca31ec233ad4e25b96929"
)
EXPECTED_HERMES_ENTRYPOINT_SHA256 = (
    "5f0937f77b6df59262dad536c1f6ed1447295584cdd129eed403b84f5bc826a8"
)
EXPECTED_RUNTIME_PYTHON_SHA256 = (
    "b4274ebd5b568c6b6dc5f1668d1d747c574c0e0d605f41e09f26c51b2446971b"
)
EXPECTED_GATEWAY_API_SERVER_SHA256 = (
    "8272ff767069e67c4a210899e1adb6a8f9763a1eecb9328f6807307c104c0523"
)
MAX_DIAGNOSTIC_BYTES = 4096
OFFICIAL_HERMES_CONFIG = b"""_config_version: 33
model:
  provider: openai-api
  default: gpt-5.6-sol
  openai_runtime: codex_app_server
  api_mode: codex_app_server
  base_url: http://127.0.0.1:9/v1
platform_toolsets:
  api_server: []
plugins:
  enabled: []
mcp_servers: {}
"""
MAX_CONTROL_BYTES = 2 * 1024 * 1024
MAX_PROXY_COPY_BYTES = 64 * 1024


def fail(code):
    os.write(2, ("HERMES_OUTER_FAIL:%d\n" % code).encode("ascii"))
    raise SystemExit(code)


def write_all_descriptor(descriptor, payload):
    offset = 0
    while offset < len(payload):
        try:
            written = os.write(descriptor, payload[offset:])
        except OSError:
            return False
        if written <= 0:
            return False
        offset += written
    return True


class ProxyRelay:
    def __init__(self, connection):
        self.connection = connection
        self.stop = threading.Event()
        self.failed = threading.Event()
        self.threads = []

    def start(self):
        self.connection.settimeout(0.1)
        self.threads = [
            threading.Thread(target=self.copy_child_to_host, daemon=True),
            threading.Thread(target=self.copy_host_to_child, daemon=True),
        ]
        for worker in self.threads:
            worker.start()

    def mark_failed(self):
        if not self.stop.is_set():
            self.failed.set()

    def copy_child_to_host(self):
        while not self.stop.is_set():
            try:
                payload = self.connection.recv(MAX_PROXY_COPY_BYTES)
            except socket.timeout:
                continue
            except OSError:
                self.mark_failed()
                return
            if not payload or not write_all_descriptor(1, payload):
                self.mark_failed()
                return

    def copy_host_to_child(self):
        while not self.stop.is_set():
            try:
                readable, _, _ = select.select([0], [], [], 0.1)
            except (OSError, ValueError):
                self.mark_failed()
                return
            if not readable:
                continue
            try:
                payload = os.read(0, MAX_PROXY_COPY_BYTES)
                if not payload:
                    self.mark_failed()
                    return
                self.connection.sendall(payload)
            except (OSError, socket.timeout):
                self.mark_failed()
                return

    def check(self):
        if self.failed.is_set():
            fail(75)

    def close(self):
        self.stop.set()
        try:
            self.connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.connection.close()
        for worker in self.threads:
            worker.join(0.5)


def emit_bounded_startup_proxy_evidence(connection):
    digest = hashlib.sha256()
    byte_count = 0
    try:
        connection.setblocking(False)
        while byte_count < MAX_DIAGNOSTIC_BYTES:
            try:
                payload = connection.recv(
                    min(MAX_PROXY_COPY_BYTES, MAX_DIAGNOSTIC_BYTES - byte_count)
                )
            except BlockingIOError:
                break
            except OSError:
                break
            if not payload:
                break
            digest.update(payload)
            byte_count += len(payload)
    except OSError:
        pass
    try:
        os.write(
            2,
            (
                "HERMES_OUTER_PROXY_EVIDENCE:%d:%s\n"
                % (byte_count, digest.hexdigest())
            ).encode("ascii"),
        )
    except OSError:
        pass


def checked_nonce(value):
    if len(value) != 64 or any(char not in "0123456789abcdef" for char in value):
        fail(64)
    return value


def is_digest(value):
    return isinstance(value, str) and len(value) == 64 and all(
        char in "0123456789abcdef" for char in value
    )


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb", buffering=0) as source:
        while True:
            chunk = source.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def validate_official_runtime_identity(runtime_root):
    if runtime_root != EXPECTED_OFFICIAL_RUNTIME_ROOT:
        fail(65)
    expected_files = (
        ("offline-runtime-manifest.json", 925, EXPECTED_OFFLINE_MANIFEST_SHA256, False),
        (
            "provenance/runtime-tree-manifest.json",
            2673882,
            EXPECTED_RUNTIME_TREE_SHA256,
            False,
        ),
        ("python/bin/hermes", 182, EXPECTED_HERMES_ENTRYPOINT_SHA256, True),
        (
            "python/bin/python3.12",
            102380768,
            EXPECTED_RUNTIME_PYTHON_SHA256,
            True,
        ),
        (
            "python/lib/python3.12/site-packages/gateway/platforms/api_server.py",
            325578,
            EXPECTED_GATEWAY_API_SERVER_SHA256,
            False,
        ),
    )
    try:
        root_metadata = os.lstat(runtime_root)
        if not stat.S_ISDIR(root_metadata.st_mode) or os.path.realpath(runtime_root) != runtime_root:
            fail(65)
        for relative, expected_size, expected_sha256, executable in expected_files:
            path = os.path.join(runtime_root, relative)
            metadata = os.lstat(path)
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_size != expected_size
                or os.path.realpath(path) != path
                or (executable and not os.access(path, os.X_OK))
                or file_sha256(path) != expected_sha256
            ):
                fail(65)
    except OSError:
        fail(65)


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


def read_exact(connection, length):
    output = bytearray()
    while len(output) < length:
        chunk = connection.recv(length - len(output))
        if not chunk:
            fail(76)
        output.extend(chunk)
    return bytes(output)


def receive_frame(connection, magic, limit=MAX_CONTROL_BYTES):
    if read_exact(connection, len(magic)) != magic:
        fail(76)
    length = int.from_bytes(read_exact(connection, 4), "big")
    if length == 0 or length > limit:
        fail(76)
    return read_exact(connection, length)


def send_frame(connection, magic, payload):
    if not payload or len(payload) > MAX_CONTROL_BYTES:
        fail(76)
    connection.sendall(magic + len(payload).to_bytes(4, "big") + payload)


def canonical_json(value):
    return json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True).encode(
        "ascii"
    )


def config_digest(init):
    value = {
        "api_key_sha256": hashlib.sha256(init["api_key"].encode("utf-8")).hexdigest(),
        "endpoint": init["endpoint"],
        "hermes_config_sha256": (
            hashlib.sha256(OFFICIAL_HERMES_CONFIG).hexdigest()
            if init["mode"] == "official"
            else None
        ),
        "model": init["model"],
        "nonce": init["nonce"],
        "schema": "lattice.hermes.production-config.v2",
    }
    return hashlib.sha256(canonical_json(value)).hexdigest()


def read_secret_bundle(path, nonce):
    if (
        not path.startswith("/mnt/")
        or any(part == ".." for part in path.split("/"))
        or not path.endswith("/launch-secret.json")
    ):
        fail(64)
    try:
        with open(path, "rb", buffering=0) as source:
            encoded = source.read(MAX_CONTROL_BYTES + 1)
        os.unlink(path)
        value = json.loads(encoded.decode("ascii"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        fail(72)
    if canonical_json(value) != encoded or set(value) != {
        "api_key",
        "broker_receipt_sha256",
        "config_sha256",
        "deadline_millis",
        "endpoint",
        "fixture_reflection",
        "mode",
        "model",
        "nonce",
        "runtime_manifest_sha256",
    }:
        fail(72)
    if (
        value["nonce"] != nonce
        or value["endpoint"] != "127.0.0.1:0"
        or value["config_sha256"] != "0" * 64
        or not is_digest(value["broker_receipt_sha256"])
        or not is_digest(value["runtime_manifest_sha256"])
    ):
        fail(72)
    return value


def production_bwrap_command(runtime_root, runner_fd, mode):
    if not runtime_root.startswith("/var/tmp/lattice-runtime-targets/"):
        fail(64)
    if mode not in ("official", "scripted_fixture") or runner_fd < 3:
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
        "--file",
        str(runner_fd),
        "/runner.py",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--dir",
        "/work",
        "--dir",
        "/config-input",
        "--dir",
        "/request-input",
        "--dir",
        "/broker-input",
        "--tmpfs",
        "/state",
        "--tmpfs",
        "/output",
        "--tmpfs",
        "/tmp",
        "--clearenv",
        "--setenv",
        "PATH",
        "/runtime-input/python/bin:/usr/bin:/bin",
        "--setenv",
        "HOME",
        "/state/hermes",
        "--setenv",
        "HERMES_HOME",
        "/state/hermes",
        "--setenv",
        "CODEX_HOME",
        "/state/codex-unavailable",
        "--setenv",
        "LATTICE_CODEX_BROKER_READ_FD",
        "0",
        "--setenv",
        "LATTICE_CODEX_BROKER_WRITE_FD",
        "1",
        "--setenv",
        "LATTICE_HERMES_CODEX_PROXY_FD",
        "2",
        "--setenv",
        "TMPDIR",
        "/tmp",
        "--setenv",
        "PYTHONDONTWRITEBYTECODE",
        "1",
        "--setenv",
        "PYTHONHASHSEED",
        "0",
        "--setenv",
        "PYTHONNOUSERSITE",
        "1",
        "--setenv",
        "PYTHONSAFEPATH",
        "1",
        "--setenv",
        "PYTHONUTF8",
        "1",
        "--setenv",
        "NO_COLOR",
        "1",
        "--setenv",
        "CI",
        "1",
        "--setenv",
        "TZ",
        "UTC",
        "--setenv",
        "LANG",
        "C.UTF-8",
        "--setenv",
        "LC_ALL",
        "C.UTF-8",
        "--chdir",
        "/work",
        "/runtime-input/python/bin/python3.12",
        "-I",
        "-S",
        "-B",
        "/runner.py",
        "contained-reflection",
    ]


def receive_containment_frame(peer, init):
    output = bytearray(read_exact(peer, len(CONTAINMENT_MAGIC)))
    if bytes(output) != CONTAINMENT_MAGIC:
        fail(73)
    fields = []
    for index in range(13):
        length_bytes = read_exact(peer, 4)
        length = int.from_bytes(length_bytes, "big")
        bound = 64 if index <= 9 else (10 if index == 10 else (32 if index == 11 else MAX_CONTROL_BYTES))
        if length == 0 or length > bound:
            fail(73)
        field = read_exact(peer, length)
        output.extend(length_bytes)
        output.extend(field)
        fields.append(field)
    expected_socketpair = hashlib.sha256(
        bytes.fromhex(init["nonce"]) + b"LATTICE_HERMES_PRODUCTION_SOCKETPAIR_V1"
    ).hexdigest().encode("ascii")
    expected = (
        init["runtime_manifest_sha256"].encode("ascii"),
        init["config_sha256"].encode("ascii"),
        init["broker_receipt_sha256"].encode("ascii"),
        EXPECTED_BWRAP_SHA256.encode("ascii"),
        expected_socketpair,
        hashlib.sha256(init["api_key"].encode("utf-8")).hexdigest().encode("ascii"),
        hashlib.sha256(bytes.fromhex(init["nonce"])).hexdigest().encode("ascii"),
        init["endpoint"].encode("ascii"),
        init["mode"].encode("ascii"),
    )
    if (
        fields[0] != expected[0]
        or fields[1] != expected[1]
        or fields[3] != expected[2]
        or fields[4] != expected[3]
        or fields[5] != expected[4]
        or fields[6] != expected[5]
        or fields[7] != expected[6]
        or fields[9] != expected[7]
        or fields[11] != expected[8]
    ):
        fail(73)
    try:
        namespace_pid = int(fields[10].decode("ascii"))
        attestation = json.loads(fields[12].decode("ascii"))
    except (ValueError, UnicodeDecodeError, json.JSONDecodeError):
        fail(73)
    if (
        namespace_pid <= 0
        or canonical_json(attestation) != fields[12]
        or attestation.get("namespace_pid") != namespace_pid
        or attestation.get("endpoint") != init["endpoint"]
        or attestation.get("api_key_sha256") != expected[5].decode("ascii")
        or attestation.get("nonce_sha256") != expected[6].decode("ascii")
        or attestation.get("mode") != init["mode"]
        or attestation.get("schema") != "lattice.hermes.containment-attestation.v2"
        or not isinstance(attestation.get("net_namespace"), str)
        or not attestation["net_namespace"].startswith("net:[")
    ):
        fail(73)
    return bytes(output), namespace_pid


def emit_startup(frame, outer_pid, bwrap_pid):
    payload = canonical_json(
        {
            "bwrap_pid": bwrap_pid,
            "containment_frame_hex": frame.hex(),
            "containment_frame_sha256": hashlib.sha256(frame).hexdigest(),
            "outer_pid": outer_pid,
            "schema": "lattice.hermes.production-start.v1",
        }
    )
    sys.stdout.buffer.write(STARTUP_MAGIC)
    sys.stdout.buffer.write(len(payload).to_bytes(8, "big"))
    sys.stdout.buffer.write(payload)
    sys.stdout.buffer.flush()


def read_http_request(connection, deadline):
    output = bytearray()
    connection.settimeout(max(0.001, deadline - time.monotonic()))
    header_end = None
    expected_length = None
    while expected_length is None or len(output) < expected_length:
        chunk = connection.recv(8192)
        if not chunk:
            fail(77)
        output.extend(chunk)
        if len(output) > MAX_CONTROL_BYTES:
            fail(77)
        if header_end is None:
            marker = output.find(b"\r\n\r\n")
            if marker < 0:
                continue
            header_end = marker + 4
            try:
                lines = bytes(output[:marker]).decode("ascii").split("\r\n")
                lengths = [
                    value.strip()
                    for name, value in (line.split(":", 1) for line in lines[1:])
                    if name.lower() == "content-length"
                ]
                if len(lengths) != 1 or not lengths[0].isdigit():
                    fail(77)
                body_length = int(lengths[0])
            except (ValueError, UnicodeDecodeError):
                fail(77)
            expected_length = header_end + body_length
            if expected_length > MAX_CONTROL_BYTES:
                fail(77)
    if expected_length is None or len(output) != expected_length:
        fail(77)
    return bytes(output)


def open_runner_source(path, expected_sha256):
    if not path.startswith("/mnt/") or not is_digest(expected_sha256):
        fail(64)
    flags = os.O_RDONLY | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags)
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size <= 0 or metadata.st_size > MAX_CONTROL_BYTES:
            fail(67)
        digest = hashlib.sha256()
        while True:
            chunk = os.read(descriptor, 65536)
            if not chunk:
                break
            digest.update(chunk)
        if digest.hexdigest() != expected_sha256:
            fail(67)
        os.lseek(descriptor, 0, os.SEEK_SET)
        os.unlink(path)
        return descriptor
    except OSError:
        fail(67)


def production(runtime_root, nonce, secret_path, runner_path, runner_sha256):
    if sys.version_info[:3] != EXPECTED_PYTHON_VERSION:
        fail(65)
    checked_nonce(nonce)
    if file_sha256("/usr/bin/bwrap") != EXPECTED_BWRAP_SHA256:
        fail(66)
    init = read_secret_bundle(secret_path, nonce)
    if init["mode"] == "official":
        validate_official_runtime_identity(runtime_root)
    runner_fd = open_runner_source(runner_path, runner_sha256)
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen(4)
    listener.settimeout(0.1)
    endpoint = listener.getsockname()
    init["endpoint"] = "%s:%d" % endpoint
    init["config_sha256"] = config_digest(init)
    peer, child_endpoint = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    proxy_peer, proxy_child_endpoint = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    process = None
    proxy_relay = None
    deadline = time.monotonic() + init["deadline_millis"] / 1000.0
    deadline_rearmed = False
    try:
        process = subprocess.Popen(
            production_bwrap_command(runtime_root, runner_fd, init["mode"]),
            stdin=child_endpoint,
            stdout=child_endpoint,
            stderr=proxy_child_endpoint,
            close_fds=True,
            pass_fds=(runner_fd,),
        )
        os.close(runner_fd)
        runner_fd = -1
        child_endpoint.close()
        proxy_child_endpoint.close()
        peer.settimeout(max(0.001, deadline - time.monotonic()))
        send_frame(peer, INIT_MAGIC, canonical_json(init))
        try:
            frame, _namespace_pid = receive_containment_frame(peer, init)
        except SystemExit:
            return_code = process.poll()
            if return_code is None:
                try:
                    return_code = process.wait(timeout=min(0.25, max(0.001, deadline - time.monotonic())))
                except subprocess.TimeoutExpired:
                    return_code = None
            if return_code is not None and 64 <= return_code <= 79:
                fail(return_code)
            raise
        emit_startup(frame, os.getpid(), process.pid)
        proxy_relay = ProxyRelay(proxy_peer)
        proxy_relay.start()
        while time.monotonic() < deadline:
            proxy_relay.check()
            if process.poll() is not None:
                fail(75)
            try:
                connection, _address = listener.accept()
            except socket.timeout:
                continue
            try:
                if not deadline_rearmed:
                    deadline = time.monotonic() + init["deadline_millis"] / 1000.0
                    deadline_rearmed = True
                request = read_http_request(connection, deadline)
                peer.settimeout(max(0.001, deadline - time.monotonic()))
                send_frame(peer, HTTP_REQUEST_MAGIC, request)
                response = receive_frame(peer, HTTP_RESPONSE_MAGIC)
                connection.sendall(response)
            finally:
                connection.close()
        fail(79)
    except (OSError, subprocess.SubprocessError, socket.timeout):
        fail(75)
    finally:
        listener.close()
        peer.close()
        child_endpoint.close()
        proxy_child_endpoint.close()
        if proxy_relay is not None:
            proxy_relay.close()
        if runner_fd >= 0:
            os.close(runner_fd)
        if process is not None:
            if process.poll() is None:
                process.kill()
            try:
                process.wait(timeout=2.0)
            except subprocess.SubprocessError:
                process.kill()
                process.wait()
        if proxy_relay is None:
            emit_bounded_startup_proxy_evidence(proxy_peer)
            proxy_peer.close()


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
            "bwrap_sha256": bwrap_sha256,
            "descendants_reaped": True,
            "nonce_binding_sha256": binding,
            "python_version": "3.12.13",
            "schema": "lattice.hermes.socketpair-receipt.v1",
        }
    )


def main(arguments):
    if len(arguments) == 3 and arguments[0] == "socketpair-canary":
        socketpair_canary(arguments[1], checked_nonce(arguments[2]))
        return
    if len(arguments) == 6 and arguments[0] == "production":
        production(
            arguments[1],
            checked_nonce(arguments[2]),
            arguments[3],
            arguments[4],
            arguments[5],
        )
        return
    fail(64)


if __name__ == "__main__":
    main(sys.argv[1:])
