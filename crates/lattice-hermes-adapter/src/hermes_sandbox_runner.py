"""Pinned in-bwrap bootstrap for one owned Hermes reflection endpoint.

The Rust owner embeds this source and supplies one fixed AF_UNIX socketpair.
The bootstrap applies Landlock before consuming the one-run secret, verifies
all currently defined cross-bindings, emits a bounded V2 containment frame,
and relays bounded HTTP to a loopback endpoint inside the private namespace.

Scripted fixture mode is explicitly labelled and never imports or invokes
Hermes or a model. Official mode starts only the pinned Hermes gateway,
verifies its authenticated capabilities before attestation, and relays its
bounded loopback HTTP without synthesizing a model result.
"""

import ctypes
import errno
import hashlib
import json
import os
import pathlib
import re
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import threading
import time


INIT_MAGIC = b"LATTICE_HERMES_PRODUCTION_INIT_V1\n"
HTTP_REQUEST_MAGIC = b"LATTICE_HERMES_HTTP_REQUEST_V1\n"
HTTP_RESPONSE_MAGIC = b"LATTICE_HERMES_HTTP_RESPONSE_V1\n"
CONTAINMENT_MAGIC = b"LATTICE_HERMES_CONTAINED_V2\n"
CODEX_PROXY_MAGIC = b"LATTICE_HERMES_CODEX_PROXY_V1\n"
CODEX_SHIM_MAGIC = b"LATTICE_HERMES_CODEX_SHIM_V1\n"
MAX_CONTROL_BYTES = 2 * 1024 * 1024
MAX_REFLECTION_BYTES = 64 * 1024
MAX_EXECUTABLE_BYTES = 32 * 1024 * 1024
MAX_RUNTIME_FILE_BYTES = 128 * 1024 * 1024
MAX_CODEX_PROXY_DATA_BYTES = 64 * 1024
CODEX_PROXY_HEADER_BYTES = 41
CODEX_PROXY_STREAM_ID = 1
CODEX_PROXY_OPEN = 1
CODEX_PROXY_DATA = 2
CODEX_PROXY_CLOSE = 3
CODEX_PROXY_ERROR = 4
CODEX_PROXY_TERMINAL = 5
PROXY_ERROR_PROTOCOL = 1
PROXY_ERROR_BINDING = 2
PROXY_ERROR_SEQUENCE = 3
PROXY_ERROR_SIZE = 4
PROXY_ERROR_STATE = 5
PROXY_ERROR_DEADLINE = 6
PROXY_ERROR_IO = 7
EXPECTED_BWRAP_SHA256 = "0abea81db798ebf6b4742ac0664802d97521547a353c2a0dbdc21d76cbbfd2c0"
OFFICIAL_HERMES_CANDIDATE = "/runtime-input/python/bin/hermes"
OFFICIAL_RUNTIME_PYTHON = "/runtime-input/python/bin/python3.12"
OFFICIAL_RUNTIME_MANIFEST = "/runtime-input/offline-runtime-manifest.json"
OFFICIAL_RUNTIME_TREE_MANIFEST = "/runtime-input/provenance/runtime-tree-manifest.json"
OFFICIAL_GATEWAY_API_SERVER = (
    "/runtime-input/python/lib/python3.12/site-packages/gateway/platforms/api_server.py"
)
OFFICIAL_HERMES_SHA256 = "5f0937f77b6df59262dad536c1f6ed1447295584cdd129eed403b84f5bc826a8"
OFFICIAL_RUNTIME_PYTHON_SHA256 = (
    "b4274ebd5b568c6b6dc5f1668d1d747c574c0e0d605f41e09f26c51b2446971b"
)
OFFICIAL_RUNTIME_MANIFEST_SHA256 = (
    "e3a3272b6cead30cd2df1af755df031766475595fdacfb080d0886671b6d1fbb"
)
OFFICIAL_RUNTIME_TREE_SHA256 = (
    "cb0e331bcb2b4fe2fd0977401d246819aadb800b645ca31ec233ad4e25b96929"
)
OFFICIAL_GATEWAY_API_SERVER_SHA256 = (
    "8272ff767069e67c4a210899e1adb6a8f9763a1eecb9328f6807307c104c0523"
)
OFFICIAL_HERMES_CONFIG_PATH = "/state/hermes/config.yaml"
OFFICIAL_OPENAI_SENTINEL = "lattice-codex-app-server-only"
CODEX_PROXY_SOCKET_PATH = "/state/codex-proxy.sock"
CODEX_SHIM_PATH = "/state/bin/codex"
OFFICIAL_HERMES_CONFIG = b"""_config_version: 33
model:
  provider: openai-api
  default: gpt-5.6-luna
  openai_runtime: codex_app_server
  api_mode: codex_app_server
  base_url: http://127.0.0.1:9/v1
platform_toolsets:
  api_server: []
plugins:
  enabled: []
mcp_servers: {}
"""
FIXTURE_CODEX_REQUEST = (
    b'{"id":0,"method":"initialize","params":{}}\n'
    b'{"id":1,"method":"thread/start","params":{}}\n'
    b'{"id":2,"method":"turn/start","params":{}}\n'
)
FIXTURE_CODEX_RESPONSE = (
    b'{"id":0,"result":{"ok":true}}\n'
    b'{"id":1,"result":{"ok":true}}\n'
    b'{"id":2,"result":{"ok":true}}\n'
)
CODEX_SHIM_SOURCE = r'''#!/runtime-input/python/bin/python3.12
import os
import socket
import sys
import threading

MAGIC = b"LATTICE_HERMES_CODEX_SHIM_V1\n"
MAX_CHUNK = 65536


def abort():
    os._exit(70)


def write_all(descriptor, payload):
    offset = 0
    while offset < len(payload):
        written = os.write(descriptor, payload[offset:])
        if written <= 0:
            abort()
        offset += written


def main():
    if sys.argv[1:] != ["app-server"]:
        return 70
    path = os.environ.get("LATTICE_HERMES_CODEX_PROXY_SOCKET")
    binding_text = os.environ.get("LATTICE_HERMES_CODEX_PROXY_BINDING", "")
    timeout_text = os.environ.get("LATTICE_HERMES_CODEX_PROXY_TIMEOUT_MILLIS", "")
    try:
        binding = bytes.fromhex(binding_text)
        timeout = int(timeout_text) / 1000.0
    except (ValueError, TypeError):
        return 70
    if path != "/state/codex-proxy.sock" or len(binding) != 32 or timeout <= 0:
        return 70
    channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    channel.settimeout(timeout)
    try:
        channel.connect(path)
        channel.sendall(MAGIC + binding)
    except OSError:
        channel.close()
        return 70
    failed = []

    def copy_input():
        try:
            while True:
                chunk = os.read(0, MAX_CHUNK)
                if not chunk:
                    channel.shutdown(socket.SHUT_WR)
                    return
                channel.sendall(chunk)
        except OSError:
            failed.append(True)

    input_thread = threading.Thread(target=copy_input, daemon=True)
    input_thread.start()
    host_closed_cleanly = False
    try:
        while True:
            chunk = channel.recv(MAX_CHUNK)
            if not chunk:
                host_closed_cleanly = True
                break
            write_all(1, chunk)
    except OSError:
        failed.append(True)
    finally:
        channel.close()
    if host_closed_cleanly:
        return 70 if failed else 0
    input_thread.join(timeout)
    if input_thread.is_alive() or failed:
        return 70
    return 0


try:
    raise SystemExit(main())
except SystemExit:
    raise
except BaseException:
    raise SystemExit(70)
'''


def fail(code):
    raise SystemExit(code)


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def is_digest(value):
    return isinstance(value, str) and len(value) == 64 and all(
        char in "0123456789abcdef" for char in value
    )


class CodexProxyViolation(Exception):
    def __init__(self, error_code):
        super().__init__(error_code)
        self.error_code = error_code


def canonical_json(value):
    return json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True).encode(
        "ascii"
    )


def remaining_seconds(deadline):
    if deadline is None:
        return None
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        fail(79)
    return max(0.001, remaining)


def read_exact(connection, length, deadline=None, allow_initial_eof=False):
    output = bytearray()
    while len(output) < length:
        connection.settimeout(remaining_seconds(deadline))
        try:
            chunk = connection.recv(length - len(output))
        except socket.timeout:
            fail(79)
        except OSError:
            fail(76)
        if not chunk:
            if allow_initial_eof and not output:
                return None
            fail(76)
        output.extend(chunk)
    return bytes(output)


def receive_frame(connection, magic, deadline=None, allow_eof=False):
    observed_magic = read_exact(
        connection,
        len(magic),
        deadline,
        allow_initial_eof=allow_eof,
    )
    if observed_magic is None:
        return None
    if observed_magic != magic:
        fail(76)
    length = int.from_bytes(read_exact(connection, 4, deadline), "big")
    if length == 0 or length > MAX_CONTROL_BYTES:
        fail(76)
    return read_exact(connection, length, deadline)


def send_frame(connection, magic, payload, deadline=None):
    if not payload or len(payload) > MAX_CONTROL_BYTES:
        fail(76)
    connection.settimeout(remaining_seconds(deadline))
    try:
        connection.sendall(magic + len(payload).to_bytes(4, "big") + payload)
    except socket.timeout:
        fail(79)
    except OSError:
        fail(76)


def apply_write_landlock():
    libc = ctypes.CDLL(None, use_errno=True)
    syscall = libc.syscall
    syscall.restype = ctypes.c_long
    create_ruleset, add_rule, restrict_self = 444, 445, 446
    abi = syscall(create_ruleset, 0, 0, 1)
    if abi < 3:
        fail(67)
    write_access = (1 << 1) | sum(1 << bit for bit in range(4, 15))

    class RulesetAttr(ctypes.Structure):
        _fields_ = [("handled_access_fs", ctypes.c_uint64)]

    class PathBeneathAttr(ctypes.Structure):
        _fields_ = [("allowed_access", ctypes.c_uint64), ("parent_fd", ctypes.c_int32)]

    ruleset_attr = RulesetAttr(write_access)
    ruleset = syscall(create_ruleset, ctypes.byref(ruleset_attr), ctypes.sizeof(ruleset_attr), 0)
    if ruleset < 0:
        fail(67)
    try:
        for allowed in ("/state", "/output", "/tmp"):
            descriptor = os.open(allowed, os.O_PATH | os.O_CLOEXEC)
            try:
                rule = PathBeneathAttr(write_access, descriptor)
                if syscall(add_rule, ruleset, 1, ctypes.byref(rule), 0) != 0:
                    fail(67)
            finally:
                os.close(descriptor)
        descriptor = os.open("/dev/null", os.O_PATH | os.O_CLOEXEC)
        try:
            rule = PathBeneathAttr((1 << 1) | (1 << 14), descriptor)
            if syscall(add_rule, ruleset, 1, ctypes.byref(rule), 0) != 0:
                fail(67)
        finally:
            os.close(descriptor)
        if libc.prctl(38, 1, 0, 0, 0) != 0:
            fail(67)
        if syscall(restrict_self, ruleset, 0) != 0:
            fail(67)
    finally:
        os.close(ruleset)
    return abi


def require_empty_work():
    if list(pathlib.Path("/work").iterdir()):
        fail(68)


def verify_write_boundaries():
    for denied in (
        "/work/denied",
        "/runtime-input/denied",
        "/config-input/denied",
        "/request-input/denied",
    ):
        try:
            pathlib.Path(denied).write_bytes(b"denied")
        except OSError as error:
            if error.errno not in (errno.EACCES, errno.EPERM, errno.EROFS):
                fail(69)
        else:
            fail(69)
    for allowed in ("/state/canary", "/output/canary", "/tmp/canary"):
        path = pathlib.Path(allowed)
        path.write_bytes(b"lattice")
        path.unlink()


def verify_network_is_private():
    probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    probe.settimeout(0.1)
    try:
        if probe.connect_ex(("1.1.1.1", 53)) == 0:
            fail(70)
    finally:
        probe.close()


def verify_broker_socket():
    if os.environ.get("LATTICE_CODEX_BROKER_READ_FD") != "0":
        fail(71)
    if os.environ.get("LATTICE_CODEX_BROKER_WRITE_FD") != "1":
        fail(71)
    try:
        read_stat = os.fstat(0)
        write_stat = os.fstat(1)
        if (read_stat.st_dev, read_stat.st_ino) != (write_stat.st_dev, write_stat.st_ino):
            fail(71)
        broker = socket.fromfd(0, socket.AF_UNIX, socket.SOCK_STREAM)
        write_probe = socket.fromfd(1, socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            if (
                broker.getsockopt(socket.SOL_SOCKET, socket.SO_TYPE) != socket.SOCK_STREAM
                or write_probe.getsockopt(socket.SOL_SOCKET, socket.SO_TYPE) != socket.SOCK_STREAM
                or broker.getpeername() != write_probe.getpeername()
            ):
                fail(71)
        finally:
            write_probe.close()
        return broker
    except OSError:
        fail(71)


def verify_codex_proxy_socket():
    if os.environ.get("LATTICE_HERMES_CODEX_PROXY_FD") != "2":
        fail(71)
    try:
        proxy_stat = os.fstat(2)
        for descriptor in (0, 1):
            control_stat = os.fstat(descriptor)
            if (proxy_stat.st_dev, proxy_stat.st_ino) == (
                control_stat.st_dev,
                control_stat.st_ino,
            ):
                fail(71)
        proxy = socket.fromfd(2, socket.AF_UNIX, socket.SOCK_STREAM)
        if proxy.getsockopt(socket.SOL_SOCKET, socket.SO_TYPE) != socket.SOCK_STREAM:
            fail(71)
        proxy.getpeername()
        return proxy
    except OSError:
        fail(71)


def parse_init_payload(encoded):
    try:
        value = json.loads(encoded.decode("ascii"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        fail(72)
    if not isinstance(value, dict) or canonical_json(value) != encoded or set(value) != {
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
        not isinstance(value["api_key"], str)
        or not value["api_key"]
        or len(value["api_key"].encode("utf-8")) > 4096
        or any(ord(char) < 32 or ord(char) == 127 for char in value["api_key"])
        or not is_digest(value["broker_receipt_sha256"])
        or not is_digest(value["config_sha256"])
        or not is_digest(value["runtime_manifest_sha256"])
        or not is_digest(value["nonce"])
        or type(value["deadline_millis"]) is not int
        or value["deadline_millis"] <= 0
        or value["deadline_millis"] > 300_000
        or value["mode"] not in ("official", "scripted_fixture")
        or not isinstance(value["model"], str)
        or not value["model"]
        or len(value["model"].encode("utf-8")) > 256
        or any(ord(char) < 32 or ord(char) == 127 for char in value["model"])
    ):
        fail(72)
    try:
        host, port_text = value["endpoint"].rsplit(":", 1)
        port = int(port_text)
    except (AttributeError, ValueError):
        fail(72)
    if (
        host != "127.0.0.1"
        or port <= 0
        or port > 65535
        or len(value["endpoint"].encode("ascii", errors="ignore")) != len(value["endpoint"])
        or len(value["endpoint"]) > 64
    ):
        fail(72)
    if value["mode"] == "scripted_fixture":
        reflection = value["fixture_reflection"]
        if not isinstance(reflection, str) or not reflection:
            fail(72)
        encoded_reflection = reflection.encode("utf-8")
        if len(encoded_reflection) > MAX_REFLECTION_BYTES:
            fail(72)
        try:
            parsed_reflection = json.loads(encoded_reflection)
        except json.JSONDecodeError:
            fail(72)
        if not isinstance(parsed_reflection, dict) or canonical_json(parsed_reflection) != encoded_reflection:
            fail(72)
    else:
        if (
            value["fixture_reflection"] is not None
            or value["model"] != "hermes-agent"
            or len(value["api_key"]) < 16
            or not value["api_key"].isascii()
        ):
            fail(72)
    return value


def parse_init(connection):
    return parse_init_payload(receive_frame(connection, INIT_MAGIC))


def config_digest(init):
    value = {
        "api_key_sha256": sha256_bytes(init["api_key"].encode("utf-8")),
        "endpoint": init["endpoint"],
        "hermes_config_sha256": (
            sha256_bytes(OFFICIAL_HERMES_CONFIG) if init["mode"] == "official" else None
        ),
        "model": init["model"],
        "nonce": init["nonce"],
        "schema": "lattice.hermes.production-config.v2",
    }
    return sha256_bytes(canonical_json(value))


def codex_proxy_binding(init):
    if not is_digest(init.get("nonce")) or not is_digest(init.get("broker_receipt_sha256")):
        raise CodexProxyViolation(PROXY_ERROR_BINDING)
    return hashlib.sha256(
        bytes.fromhex(init["nonce"])
        + bytes.fromhex(init["broker_receipt_sha256"])
        + b"LATTICE_HERMES_CODEX_PROXY_V1"
    ).digest()


def validate_codex_proxy_payload(kind, payload):
    if kind in (CODEX_PROXY_OPEN, CODEX_PROXY_CLOSE, CODEX_PROXY_TERMINAL):
        if payload:
            raise CodexProxyViolation(PROXY_ERROR_SIZE)
    elif kind == CODEX_PROXY_DATA:
        if not payload or len(payload) > MAX_CODEX_PROXY_DATA_BYTES:
            raise CodexProxyViolation(PROXY_ERROR_SIZE)
    elif kind == CODEX_PROXY_ERROR:
        if len(payload) != 2 or int.from_bytes(payload, "big") not in range(1, 8):
            raise CodexProxyViolation(PROXY_ERROR_SIZE)
    else:
        raise CodexProxyViolation(PROXY_ERROR_PROTOCOL)


def encode_codex_proxy_frame(kind, sequence, binding, payload):
    if (
        not isinstance(sequence, int)
        or isinstance(sequence, bool)
        or sequence < 0
        or sequence > 0xFFFFFFFF
        or not isinstance(binding, bytes)
        or len(binding) != 32
        or not isinstance(payload, bytes)
    ):
        raise CodexProxyViolation(PROXY_ERROR_STATE)
    validate_codex_proxy_payload(kind, payload)
    body = (
        bytes((kind,))
        + CODEX_PROXY_STREAM_ID.to_bytes(4, "big")
        + sequence.to_bytes(4, "big")
        + binding
        + payload
    )
    return CODEX_PROXY_MAGIC + len(body).to_bytes(4, "big") + body


def decode_codex_proxy_body(body, expected_sequence, expected_binding):
    if (
        not isinstance(body, bytes)
        or len(body) < CODEX_PROXY_HEADER_BYTES
        or len(body) > CODEX_PROXY_HEADER_BYTES + MAX_CODEX_PROXY_DATA_BYTES
    ):
        raise CodexProxyViolation(PROXY_ERROR_SIZE)
    kind = body[0]
    stream_id = int.from_bytes(body[1:5], "big")
    sequence = int.from_bytes(body[5:9], "big")
    binding = body[9:41]
    payload = body[41:]
    if kind not in (
        CODEX_PROXY_OPEN,
        CODEX_PROXY_DATA,
        CODEX_PROXY_CLOSE,
        CODEX_PROXY_ERROR,
        CODEX_PROXY_TERMINAL,
    ):
        raise CodexProxyViolation(PROXY_ERROR_PROTOCOL)
    if stream_id != CODEX_PROXY_STREAM_ID:
        raise CodexProxyViolation(PROXY_ERROR_STATE)
    if sequence != expected_sequence:
        raise CodexProxyViolation(PROXY_ERROR_SEQUENCE)
    if binding != expected_binding:
        raise CodexProxyViolation(PROXY_ERROR_BINDING)
    validate_codex_proxy_payload(kind, payload)
    return kind, payload


def codex_proxy_timeout(deadline):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise CodexProxyViolation(PROXY_ERROR_DEADLINE)
    return max(0.001, remaining)


def codex_proxy_read_exact(connection, length, deadline):
    output = bytearray()
    while len(output) < length:
        connection.settimeout(codex_proxy_timeout(deadline))
        try:
            chunk = connection.recv(length - len(output))
        except socket.timeout as failure:
            raise CodexProxyViolation(PROXY_ERROR_DEADLINE) from failure
        except OSError as failure:
            raise CodexProxyViolation(PROXY_ERROR_IO) from failure
        if not chunk:
            raise CodexProxyViolation(PROXY_ERROR_IO)
        output.extend(chunk)
    return bytes(output)


def send_codex_proxy_frame(connection, kind, sequence, binding, payload, deadline):
    frame = encode_codex_proxy_frame(kind, sequence, binding, payload)
    connection.settimeout(codex_proxy_timeout(deadline))
    try:
        connection.sendall(frame)
    except socket.timeout as failure:
        raise CodexProxyViolation(PROXY_ERROR_DEADLINE) from failure
    except OSError as failure:
        raise CodexProxyViolation(PROXY_ERROR_IO) from failure


def receive_codex_proxy_frame(connection, expected_sequence, binding, deadline):
    magic = codex_proxy_read_exact(connection, len(CODEX_PROXY_MAGIC), deadline)
    if magic != CODEX_PROXY_MAGIC:
        raise CodexProxyViolation(PROXY_ERROR_PROTOCOL)
    length = int.from_bytes(codex_proxy_read_exact(connection, 4, deadline), "big")
    if length < CODEX_PROXY_HEADER_BYTES or length > (
        CODEX_PROXY_HEADER_BYTES + MAX_CODEX_PROXY_DATA_BYTES
    ):
        raise CodexProxyViolation(PROXY_ERROR_SIZE)
    body = codex_proxy_read_exact(connection, length, deadline)
    return decode_codex_proxy_body(body, expected_sequence, binding)


def install_codex_shim():
    source = CODEX_SHIM_SOURCE.encode("utf-8")
    if (
        not source.startswith(b"#!/runtime-input/python/bin/python3.12\n")
        or len(source) > MAX_REFLECTION_BYTES
    ):
        fail(80)
    try:
        bin_path = pathlib.Path("/state/bin")
        bin_path.mkdir(mode=0o700)
        bin_metadata = os.lstat(bin_path)
        descriptor = os.open(
            CODEX_SHIM_PATH,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
            0o700,
        )
        try:
            offset = 0
            while offset < len(source):
                written = os.write(descriptor, source[offset:])
                if written <= 0:
                    fail(80)
                offset += written
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        os.chmod(CODEX_SHIM_PATH, 0o700)
        metadata = os.lstat(CODEX_SHIM_PATH)
        with open(CODEX_SHIM_PATH, "rb", buffering=0) as installed:
            observed = installed.read(len(source) + 1)
    except OSError:
        fail(80)
    if (
        observed != source
        or not stat.S_ISDIR(bin_metadata.st_mode)
        or bin_metadata.st_uid != os.geteuid()
        or stat.S_IMODE(bin_metadata.st_mode) != 0o700
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or sha256_bytes(observed) != sha256_bytes(source)
    ):
        fail(80)
    return sha256_bytes(source)


def install_official_hermes_config():
    source = OFFICIAL_HERMES_CONFIG
    if not source or len(source) > MAX_REFLECTION_BYTES:
        fail(80)
    try:
        home = pathlib.Path("/state/hermes")
        home.mkdir(mode=0o700)
        home_metadata = os.lstat(home)
        descriptor = os.open(
            OFFICIAL_HERMES_CONFIG_PATH,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
            0o600,
        )
        try:
            offset = 0
            while offset < len(source):
                written = os.write(descriptor, source[offset:])
                if written <= 0:
                    fail(80)
                offset += written
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        os.chmod(OFFICIAL_HERMES_CONFIG_PATH, 0o600)
        metadata = os.lstat(OFFICIAL_HERMES_CONFIG_PATH)
        descriptor = os.open(
            OFFICIAL_HERMES_CONFIG_PATH,
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
        )
        try:
            observed = os.read(descriptor, len(source) + 1)
        finally:
            os.close(descriptor)
    except OSError:
        fail(80)
    if (
        observed != source
        or not stat.S_ISDIR(home_metadata.st_mode)
        or home_metadata.st_uid != os.geteuid()
        or stat.S_IMODE(home_metadata.st_mode) != 0o700
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o600
    ):
        fail(80)
    return sha256_bytes(observed)


def create_codex_proxy_listener():
    path = pathlib.Path(CODEX_PROXY_SOCKET_PATH)
    if os.path.lexists(path):
        fail(80)
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    previous_umask = os.umask(0o077)
    try:
        listener.bind(CODEX_PROXY_SOCKET_PATH)
    except OSError:
        listener.close()
        fail(80)
    finally:
        os.umask(previous_umask)
    try:
        os.chmod(CODEX_PROXY_SOCKET_PATH, 0o600)
        metadata = os.lstat(CODEX_PROXY_SOCKET_PATH)
    except OSError:
        listener.close()
        fail(80)
    if (
        not stat.S_ISSOCK(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o600
    ):
        listener.close()
        fail(80)
    listener.listen(1)
    return listener


class CodexProxyBridge:
    def __init__(self, host_connection, binding, deadline):
        self.host_connection = host_connection
        self.binding = binding
        self.deadline = deadline
        self.listener = create_codex_proxy_listener()
        self.shim_sha256 = install_codex_shim()
        self.send_sequence = 0
        self.receive_sequence = 0
        self.failure_code = None
        self.cancelled = False
        self.done = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def emit_error(self, error_code):
        try:
            send_codex_proxy_frame(
                self.host_connection,
                CODEX_PROXY_ERROR,
                self.send_sequence,
                self.binding,
                error_code.to_bytes(2, "big"),
                self.deadline,
            )
            self.send_sequence += 1
        except (CodexProxyViolation, OSError):
            pass

    def close_listener(self):
        try:
            self.listener.close()
        except OSError:
            pass
        try:
            os.unlink(CODEX_PROXY_SOCKET_PATH)
        except FileNotFoundError:
            pass
        except OSError:
            if not self.cancelled:
                self.failure_code = 80

    def rearm(self, deadline):
        if deadline <= time.monotonic():
            raise CodexProxyViolation(PROXY_ERROR_DEADLINE)
        self.deadline = deadline

    def accept_shim(self):
        while True:
            if time.monotonic() >= self.deadline:
                raise CodexProxyViolation(PROXY_ERROR_DEADLINE)
            self.listener.settimeout(min(0.1, codex_proxy_timeout(self.deadline)))
            try:
                local, _ = self.listener.accept()
                break
            except socket.timeout:
                continue
            except OSError as failure:
                if self.cancelled:
                    return None
                raise CodexProxyViolation(PROXY_ERROR_IO) from failure
        self.close_listener()
        try:
            credentials = local.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
            _pid, uid, _gid = struct.unpack("3i", credentials)
        except (OSError, struct.error) as failure:
            local.close()
            raise CodexProxyViolation(PROXY_ERROR_BINDING) from failure
        if uid != os.geteuid():
            local.close()
            raise CodexProxyViolation(PROXY_ERROR_BINDING)
        handshake = codex_proxy_read_exact(
            local,
            len(CODEX_SHIM_MAGIC) + len(self.binding),
            self.deadline,
        )
        if handshake != CODEX_SHIM_MAGIC + self.binding:
            local.close()
            raise CodexProxyViolation(PROXY_ERROR_BINDING)
        return local

    def relay(self, local):
        send_codex_proxy_frame(
            self.host_connection,
            CODEX_PROXY_OPEN,
            self.send_sequence,
            self.binding,
            b"",
            self.deadline,
        )
        self.send_sequence += 1
        kind, payload = receive_codex_proxy_frame(
            self.host_connection,
            self.receive_sequence,
            self.binding,
            self.deadline,
        )
        self.receive_sequence += 1
        if kind != CODEX_PROXY_OPEN or payload:
            raise CodexProxyViolation(PROXY_ERROR_STATE)
        local_closed = False
        host_closed = False
        while not (local_closed and host_closed):
            readable = []
            if not local_closed:
                readable.append(local)
            if not host_closed:
                readable.append(self.host_connection)
            try:
                ready, _, _ = select.select(
                    readable,
                    [],
                    [],
                    codex_proxy_timeout(self.deadline),
                )
            except OSError as failure:
                raise CodexProxyViolation(PROXY_ERROR_IO) from failure
            if not ready:
                raise CodexProxyViolation(PROXY_ERROR_DEADLINE)
            if local in ready:
                try:
                    payload = local.recv(MAX_CODEX_PROXY_DATA_BYTES)
                except OSError as failure:
                    raise CodexProxyViolation(PROXY_ERROR_IO) from failure
                kind = CODEX_PROXY_DATA if payload else CODEX_PROXY_CLOSE
                send_codex_proxy_frame(
                    self.host_connection,
                    kind,
                    self.send_sequence,
                    self.binding,
                    payload,
                    self.deadline,
                )
                self.send_sequence += 1
                if not payload:
                    local_closed = True
            if self.host_connection in ready:
                kind, payload = receive_codex_proxy_frame(
                    self.host_connection,
                    self.receive_sequence,
                    self.binding,
                    self.deadline,
                )
                self.receive_sequence += 1
                if kind == CODEX_PROXY_DATA:
                    if host_closed:
                        raise CodexProxyViolation(PROXY_ERROR_STATE)
                    local.settimeout(codex_proxy_timeout(self.deadline))
                    try:
                        local.sendall(payload)
                    except OSError as failure:
                        raise CodexProxyViolation(PROXY_ERROR_IO) from failure
                elif kind == CODEX_PROXY_CLOSE:
                    try:
                        local.shutdown(socket.SHUT_WR)
                    except OSError as failure:
                        raise CodexProxyViolation(PROXY_ERROR_IO) from failure
                    host_closed = True
                elif kind == CODEX_PROXY_ERROR:
                    raise CodexProxyViolation(PROXY_ERROR_STATE)
                else:
                    raise CodexProxyViolation(PROXY_ERROR_STATE)
        send_codex_proxy_frame(
            self.host_connection,
            CODEX_PROXY_TERMINAL,
            self.send_sequence,
            self.binding,
            b"",
            self.deadline,
        )
        self.send_sequence += 1

    def run(self):
        local = None
        try:
            local = self.accept_shim()
            if local is not None:
                self.relay(local)
        except CodexProxyViolation as violation:
            if not self.cancelled:
                self.failure_code = 79 if violation.error_code == PROXY_ERROR_DEADLINE else 80
                self.emit_error(violation.error_code)
        except BaseException:
            if not self.cancelled:
                self.failure_code = 80
                self.emit_error(PROXY_ERROR_IO)
        finally:
            if local is not None:
                local.close()
            self.close_listener()
            self.done.set()

    def environment(self):
        timeout_millis = max(1, int((self.deadline - time.monotonic()) * 1000))
        return {
            "CODEX_HOME": "/state/codex-unavailable",
            "HOME": "/state/hermes",
            "HERMES_HOME": "/state/hermes",
            "LANG": "C.UTF-8",
            "LATTICE_HERMES_CODEX_PROXY_BINDING": self.binding.hex(),
            "LATTICE_HERMES_CODEX_PROXY_SOCKET": CODEX_PROXY_SOCKET_PATH,
            "LATTICE_HERMES_CODEX_PROXY_TIMEOUT_MILLIS": str(timeout_millis),
            "LC_ALL": "C.UTF-8",
            "NO_COLOR": "1",
            "PATH": "/state/bin:/runtime-input/python/bin:/usr/bin:/bin",
            "PYTHONDONTWRITEBYTECODE": "1",
            "PYTHONHASHSEED": "0",
            "PYTHONNOUSERSITE": "1",
            "PYTHONSAFEPATH": "1",
            "PYTHONUTF8": "1",
            "TERMINAL_CWD": "/work",
            "TMPDIR": "/tmp",
        }

    def wait(self):
        timeout = max(0.001, self.deadline - time.monotonic())
        if not self.done.wait(timeout):
            fail(79)
        if self.failure_code is not None:
            fail(self.failure_code)

    def close(self):
        if not self.done.is_set():
            self.cancelled = True
            self.close_listener()
            self.done.wait(0.5)


def exercise_fixture_codex_proxy(bridge):
    process = None
    try:
        process = subprocess.Popen(
            ["codex", "app-server"],
            cwd="/work",
            env=bridge.environment(),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            close_fds=True,
        )
        timeout = codex_proxy_timeout(bridge.deadline)
        output, stderr = process.communicate(FIXTURE_CODEX_REQUEST, timeout=timeout)
    except subprocess.TimeoutExpired:
        if process is not None:
            process.kill()
            process.communicate()
        bridge.wait()
        fail(79)
    except CodexProxyViolation as violation:
        if process is not None:
            process.kill()
            process.communicate()
        fail(79 if violation.error_code == PROXY_ERROR_DEADLINE else 80)
    except OSError:
        if process is not None:
            process.kill()
            process.communicate()
        fail(80)
    bridge.wait()
    if process.returncode != 0 or stderr or output != FIXTURE_CODEX_RESPONSE:
        fail(80)
    try:
        second = subprocess.Popen(
            ["codex", "app-server"],
            cwd="/work",
            env=bridge.environment(),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            close_fds=True,
        )
        second_output, second_stderr = second.communicate(
            b"",
            timeout=codex_proxy_timeout(bridge.deadline),
        )
    except (OSError, subprocess.TimeoutExpired, CodexProxyViolation):
        if "second" in locals():
            second.kill()
            second.communicate()
        fail(80)
    if second.returncode == 0 or second_output or second_stderr:
        fail(80)


def digest_file(path, limit):
    try:
        size = pathlib.Path(path).stat().st_size
    except OSError:
        fail(73)
    if size <= 0 or size > limit:
        fail(73)
    digest = hashlib.sha256()
    try:
        with open(path, "rb", buffering=0) as source:
            while True:
                chunk = source.read(1024 * 1024)
                if not chunk:
                    break
                digest.update(chunk)
    except OSError:
        fail(73)
    return digest.hexdigest()


def validate_official_runtime_identity():
    expected_files = (
        (OFFICIAL_HERMES_CANDIDATE, 182, OFFICIAL_HERMES_SHA256, True),
        (
            OFFICIAL_RUNTIME_PYTHON,
            102380768,
            OFFICIAL_RUNTIME_PYTHON_SHA256,
            True,
        ),
        (
            OFFICIAL_RUNTIME_MANIFEST,
            925,
            OFFICIAL_RUNTIME_MANIFEST_SHA256,
            False,
        ),
        (
            OFFICIAL_RUNTIME_TREE_MANIFEST,
            2673882,
            OFFICIAL_RUNTIME_TREE_SHA256,
            False,
        ),
        (
            OFFICIAL_GATEWAY_API_SERVER,
            325578,
            OFFICIAL_GATEWAY_API_SERVER_SHA256,
            False,
        ),
    )
    try:
        for path, expected_size, expected_sha256, executable in expected_files:
            metadata = os.lstat(path)
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_size != expected_size
                or os.path.realpath(path) != path
                or (executable and not os.access(path, os.X_OK))
                or digest_file(path, MAX_RUNTIME_FILE_BYTES) != expected_sha256
            ):
                fail(73)
    except OSError:
        fail(73)


def validate_observations(bwrap_sha256, net_namespace, namespace_pid):
    if (
        bwrap_sha256 != EXPECTED_BWRAP_SHA256
        or not isinstance(namespace_pid, int)
        or isinstance(namespace_pid, bool)
        or namespace_pid <= 0
        or namespace_pid > 0xFFFFFFFF
        or not isinstance(net_namespace, str)
        or re.fullmatch(r"net:\[[0-9]+\]", net_namespace) is None
    ):
        fail(73)


def verified_bindings(init, bwrap_sha256, net_namespace, namespace_pid):
    validate_observations(bwrap_sha256, net_namespace, namespace_pid)
    if config_digest(init) != init["config_sha256"]:
        fail(73)
    nonce_bytes = bytes.fromhex(init["nonce"])
    return {
        "api_key_sha256": sha256_bytes(init["api_key"].encode("utf-8")),
        "broker_receipt_sha256": init["broker_receipt_sha256"],
        "bwrap_sha256": bwrap_sha256,
        "config_sha256": init["config_sha256"],
        "endpoint": init["endpoint"],
        "mode": init["mode"],
        "namespace_pid": namespace_pid,
        "net_namespace": net_namespace,
        "nonce_sha256": sha256_bytes(nonce_bytes),
        "request_sha256": sha256_bytes(
            nonce_bytes + b"LATTICE_HERMES_PRODUCTION_REQUEST_V1"
        ),
        "runtime_manifest_sha256": init["runtime_manifest_sha256"],
        "socketpair_binding_sha256": sha256_bytes(
            nonce_bytes + b"LATTICE_HERMES_PRODUCTION_SOCKETPAIR_V1"
        ),
        "transcript_sha256": sha256_bytes(
            nonce_bytes + b"LATTICE_HERMES_PRODUCTION_READY_V1"
        ),
    }


def containment_frame(bindings):
    attestation = canonical_json(
        {
            "api_key_sha256": bindings["api_key_sha256"],
            "endpoint": bindings["endpoint"],
            "mode": bindings["mode"],
            "namespace_pid": bindings["namespace_pid"],
            "net_namespace": bindings["net_namespace"],
            "nonce_sha256": bindings["nonce_sha256"],
            "schema": "lattice.hermes.containment-attestation.v2",
        }
    )
    fields = (
        bindings["runtime_manifest_sha256"].encode("ascii"),
        bindings["config_sha256"].encode("ascii"),
        bindings["request_sha256"].encode("ascii"),
        bindings["broker_receipt_sha256"].encode("ascii"),
        bindings["bwrap_sha256"].encode("ascii"),
        bindings["socketpair_binding_sha256"].encode("ascii"),
        bindings["api_key_sha256"].encode("ascii"),
        bindings["nonce_sha256"].encode("ascii"),
        bindings["transcript_sha256"].encode("ascii"),
        bindings["endpoint"].encode("ascii"),
        str(bindings["namespace_pid"]).encode("ascii"),
        bindings["mode"].encode("ascii"),
        attestation,
    )
    bounds = (64,) * 10 + (16, 32, MAX_REFLECTION_BYTES)
    output = bytearray(CONTAINMENT_MAGIC)
    for field, bound in zip(fields, bounds, strict=True):
        if not field or len(field) > bound:
            fail(73)
        output.extend(len(field).to_bytes(4, "big"))
        output.extend(field)
    if any(len(field) != 64 or not is_digest(field.decode("ascii")) for field in fields[:9]):
        fail(73)
    return bytes(output)


def http_response(status, content_type, body):
    reason = {200: "OK", 202: "Accepted", 401: "Unauthorized", 404: "Not Found"}[status]
    return (
        ("HTTP/1.1 %d %s\r\n" % (status, reason)).encode("ascii")
        + ("Content-Type: %s\r\n" % content_type).encode("ascii")
        + ("Content-Length: %d\r\n" % len(body)).encode("ascii")
        + b"Connection: close\r\n\r\n"
        + body
    )


def parse_http_request(request):
    if not request or len(request) > MAX_CONTROL_BYTES:
        fail(77)
    try:
        header, body = request.split(b"\r\n\r\n", 1)
        if len(header) > 64 * 1024:
            fail(77)
        lines = header.decode("ascii").split("\r\n")
        method, path, protocol = lines[0].split(" ")
        headers = {}
        for line in lines[1:]:
            name, value = line.split(":", 1)
            name = name.strip().lower()
            if not name or name in headers:
                fail(77)
            headers[name] = value.strip()
    except (ValueError, UnicodeDecodeError):
        fail(77)
    if protocol != "HTTP/1.1" or "transfer-encoding" in headers:
        fail(77)
    content_length = headers.get("content-length", "0")
    if not content_length.isascii() or not content_length.isdigit():
        fail(77)
    if int(content_length) != len(body):
        fail(77)
    return method, path, headers, body


def fixture_response(init, request, state, codex_bridge):
    method, path, headers, body = parse_http_request(request)
    if headers.get("authorization") != "Bearer " + init["api_key"]:
        return http_response(401, "application/json", b"{}")
    if method == "GET" and path == "/v1/capabilities":
        payload = canonical_json(
            {
                "auth": {"required": True, "type": "bearer"},
                "features": {
                    "admin_config_rw": False,
                    "memory_write_api": False,
                    "run_events_sse": True,
                    "run_status": True,
                    "run_stop": True,
                    "run_submission": True,
                },
                "model": init["model"],
                "object": "hermes.api_server.capabilities",
                "platform": "hermes-agent",
                "runtime": {
                    "mode": "server_agent",
                    "split_runtime": False,
                    "tool_execution": "server",
                },
            }
        )
        return http_response(200, "application/json", payload)
    if method == "POST" and path == "/v1/runs":
        try:
            submitted = json.loads(body.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            fail(77)
        if (
            not isinstance(submitted, dict)
            or submitted.get("model") != init["model"]
            or not isinstance(submitted.get("session_id"), str)
            or not submitted["session_id"]
        ):
            fail(77)
        if state.get("codex_proxy_used"):
            fail(80)
        state["codex_proxy_used"] = True
        exercise_fixture_codex_proxy(codex_bridge)
        state["session_id"] = submitted["session_id"]
        return http_response(
            202,
            "application/json",
            canonical_json({"run_id": "run_contained_fixture", "status": "started"}),
        )
    if method == "GET" and path == "/v1/runs/run_contained_fixture/events":
        event = canonical_json(
            {
                "event": "run.completed",
                "output": init["fixture_reflection"],
                "run_id": "run_contained_fixture",
                "timestamp": 1.0,
            }
        )
        return http_response(200, "text/event-stream", b"data: " + event + b"\n\n: stream closed\n\n")
    if method == "GET" and path == "/v1/runs/run_contained_fixture":
        payload = canonical_json(
            {
                "model": init["model"],
                "object": "hermes.run",
                "output": init["fixture_reflection"],
                "run_id": "run_contained_fixture",
                "session_id": state.get("session_id", ""),
                "status": "completed",
                "usage": {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0},
            }
        )
        return http_response(200, "application/json", payload)
    if method == "POST" and path == "/v1/runs/run_contained_fixture/stop":
        return http_response(
            200,
            "application/json",
            canonical_json({"run_id": "run_contained_fixture", "status": "stopping"}),
        )
    return http_response(404, "application/json", b"{}")


def read_socket_to_eof(connection, deadline):
    output = bytearray()
    while True:
        connection.settimeout(remaining_seconds(deadline))
        try:
            chunk = connection.recv(8192)
        except socket.timeout:
            fail(79)
        except OSError:
            fail(77)
        if not chunk:
            return bytes(output)
        output.extend(chunk)
        if len(output) > MAX_CONTROL_BYTES:
            fail(78)


class ScriptedFixtureEndpoint:
    def __init__(self, init, deadline, codex_bridge):
        host, port_text = init["endpoint"].rsplit(":", 1)
        self.init = init
        self.codex_bridge = codex_bridge
        self.state = {}
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self.listener.bind((host, int(port_text)))
            self.listener.listen(1)
            self.listener.settimeout(remaining_seconds(deadline))
        except OSError:
            self.listener.close()
            fail(75)

    def close(self):
        self.listener.close()

    def relay(self, request, deadline):
        host, port_text = self.init["endpoint"].rsplit(":", 1)
        client = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        accepted = None
        try:
            client.settimeout(remaining_seconds(deadline))
            client.connect((host, int(port_text)))
            self.listener.settimeout(remaining_seconds(deadline))
            accepted, address = self.listener.accept()
            if address[0] != "127.0.0.1":
                fail(75)
            client.sendall(request)
            client.shutdown(socket.SHUT_WR)
            observed = read_socket_to_eof(accepted, deadline)
            response = fixture_response(
                self.init,
                observed,
                self.state,
                self.codex_bridge,
            )
            if not response or len(response) > MAX_CONTROL_BYTES:
                fail(78)
            accepted.settimeout(remaining_seconds(deadline))
            accepted.sendall(response)
            accepted.shutdown(socket.SHUT_WR)
            returned = read_socket_to_eof(client, deadline)
            if returned != response:
                fail(78)
            return returned
        except socket.timeout:
            fail(79)
        except OSError:
            fail(75)
        finally:
            client.close()
            if accepted is not None:
                accepted.close()


def expected_official_capabilities(init):
    return {
        "auth": {"required": True, "type": "bearer"},
        "features": {
            "admin_config_rw": False,
            "memory_write_api": False,
            "run_events_sse": True,
            "run_status": True,
            "run_stop": True,
            "run_submission": True,
        },
        "model": init["model"],
        "object": "hermes.api_server.capabilities",
        "platform": "hermes-agent",
        "runtime": {
            "mode": "server_agent",
            "split_runtime": False,
            "tool_execution": "server",
        },
    }


def validate_official_capabilities(response, init):
    if not response or len(response) > MAX_CONTROL_BYTES:
        fail(78)
    try:
        header, body = response.split(b"\r\n\r\n", 1)
        lines = header.decode("ascii").split("\r\n")
        protocol, status, _reason = lines[0].split(" ", 2)
        headers = {}
        for line in lines[1:]:
            name, value = line.split(":", 1)
            name = name.strip().lower()
            if not name or name in headers:
                fail(75)
            headers[name] = value.strip()
        content_length = headers["content-length"]
        capabilities = json.loads(body.decode("utf-8"))
    except (KeyError, ValueError, UnicodeDecodeError, json.JSONDecodeError):
        fail(75)
    if (
        protocol != "HTTP/1.1"
        or status != "200"
        or "transfer-encoding" in headers
        or not content_length.isascii()
        or not content_length.isdigit()
        or int(content_length) != len(body)
    ):
        fail(75)
    expected = expected_official_capabilities(init)
    if not isinstance(capabilities, dict):
        fail(75)
    for name in ("object", "platform", "model"):
        if capabilities.get(name) != expected[name]:
            fail(75)
    for section in ("auth", "runtime", "features"):
        observed = capabilities.get(section)
        if not isinstance(observed, dict) or any(
            observed.get(name) != value for name, value in expected[section].items()
        ):
            fail(75)


class OfficialHermesEndpoint:
    def __init__(self, init, deadline, codex_bridge):
        executable = pathlib.Path(OFFICIAL_HERMES_CANDIDATE)
        try:
            metadata = executable.stat()
        except OSError:
            fail(74)
        if not stat.S_ISREG(metadata.st_mode) or not os.access(executable, os.X_OK):
            fail(74)
        validate_official_runtime_identity()
        config_sha256 = install_official_hermes_config()
        if config_sha256 != sha256_bytes(OFFICIAL_HERMES_CONFIG):
            fail(80)
        host, port_text = init["endpoint"].rsplit(":", 1)
        environment = codex_bridge.environment()
        environment.update(
            {
                "API_SERVER_ENABLED": "true",
                "API_SERVER_HOST": host,
                "API_SERVER_KEY": init["api_key"],
                "API_SERVER_MODEL_NAME": init["model"],
                "API_SERVER_PORT": port_text,
                "CI": "1",
                "HERMES_SAFE_MODE": "1",
                "OPENAI_API_KEY": OFFICIAL_OPENAI_SENTINEL,
                "OPENAI_BASE_URL": "http://127.0.0.1:9/v1",
                "TZ": "UTC",
            }
        )
        try:
            self.process = subprocess.Popen(
                [
                    OFFICIAL_HERMES_CANDIDATE,
                    "gateway",
                    "run",
                    "--no-supervise",
                    "--external-supervisor",
                    "--quiet",
                ],
                cwd="/work",
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                close_fds=True,
                start_new_session=True,
            )
        except OSError:
            fail(74)
        self.init = init
        self.host = host
        self.port = int(port_text)
        try:
            self.wait_until_ready(deadline)
        except BaseException:
            self.close()
            raise

    def ensure_running(self):
        if self.process.poll() is not None:
            fail(75)

    def exchange(self, request, deadline):
        connection = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            timeout = remaining_seconds(deadline)
            if timeout is None or timeout <= 0:
                raise TimeoutError
            connection.settimeout(timeout)
            connection.connect((self.host, self.port))
            connection.sendall(request)
            connection.shutdown(socket.SHUT_WR)
            output = bytearray()
            while True:
                chunk = connection.recv(8192)
                if not chunk:
                    return bytes(output)
                output.extend(chunk)
                if len(output) > MAX_CONTROL_BYTES:
                    fail(78)
        finally:
            connection.close()

    def wait_until_ready(self, deadline):
        probe = (
            b"GET /v1/capabilities HTTP/1.1\r\n"
            b"Host: 127.0.0.1\r\n"
            + ("Authorization: Bearer %s\r\n" % self.init["api_key"]).encode("ascii")
            + b"Accept: application/json\r\n"
            b"Connection: close\r\n"
            b"Content-Length: 0\r\n\r\n"
        )
        while True:
            self.ensure_running()
            now = time.monotonic()
            if now >= deadline:
                fail(79)
            try:
                response = self.exchange(probe, min(deadline, now + 0.25))
            except (OSError, TimeoutError, socket.timeout):
                time.sleep(min(0.02, max(0.0, deadline - time.monotonic())))
                continue
            validate_official_capabilities(response, self.init)
            return

    def relay(self, request, deadline):
        parse_http_request(request)
        self.ensure_running()
        try:
            response = self.exchange(request, deadline)
        except (TimeoutError, socket.timeout):
            fail(79)
        except OSError:
            fail(75)
        self.ensure_running()
        if not response:
            fail(75)
        return response

    def close(self):
        if self.process.poll() is not None:
            return
        try:
            os.killpg(self.process.pid, signal.SIGTERM)
        except OSError:
            pass
        try:
            self.process.wait(timeout=0.5)
            return
        except subprocess.TimeoutExpired:
            pass
        try:
            os.killpg(self.process.pid, signal.SIGKILL)
        except OSError:
            pass
        try:
            self.process.wait(timeout=0.5)
        except subprocess.TimeoutExpired:
            fail(80)


def serve_contained_reflection(
    connection,
    codex_proxy_connection,
    bwrap_sha256,
    net_namespace,
    namespace_pid,
):
    init = parse_init(connection)
    bindings = verified_bindings(init, bwrap_sha256, net_namespace, namespace_pid)
    deadline = time.monotonic() + init["deadline_millis"] / 1000.0
    codex_bridge = CodexProxyBridge(
        codex_proxy_connection,
        codex_proxy_binding(init),
        deadline,
    )
    endpoint = None
    deadline_rearmed = False
    try:
        if init["mode"] == "official":
            endpoint = OfficialHermesEndpoint(init, deadline, codex_bridge)
        else:
            endpoint = ScriptedFixtureEndpoint(init, deadline, codex_bridge)
        connection.settimeout(remaining_seconds(deadline))
        try:
            connection.sendall(containment_frame(bindings))
        except socket.timeout:
            fail(79)
        except OSError:
            fail(76)
        while True:
            request = receive_frame(
                connection,
                HTTP_REQUEST_MAGIC,
                deadline,
                allow_eof=True,
            )
            if request is None:
                return
            if not deadline_rearmed:
                deadline = time.monotonic() + init["deadline_millis"] / 1000.0
                codex_bridge.rearm(deadline)
                deadline_rearmed = True
            response = endpoint.relay(request, deadline)
            send_frame(connection, HTTP_RESPONSE_MAGIC, response, deadline)
    finally:
        try:
            if endpoint is not None:
                endpoint.close()
        finally:
            codex_bridge.close()


def main(arguments):
    if arguments != ["contained-reflection"]:
        fail(64)
    require_empty_work()
    apply_write_landlock()
    verify_write_boundaries()
    verify_network_is_private()
    broker = verify_broker_socket()
    codex_proxy = verify_codex_proxy_socket()
    try:
        bwrap_sha256 = digest_file("/usr/bin/bwrap", MAX_EXECUTABLE_BYTES)
        try:
            net_namespace = os.readlink("/proc/self/ns/net")
        except OSError:
            fail(73)
        serve_contained_reflection(
            broker,
            codex_proxy,
            bwrap_sha256,
            net_namespace,
            os.getpid(),
        )
    finally:
        broker.close()
        codex_proxy.close()


if __name__ == "__main__":
    main(sys.argv[1:])
