"""Pinned in-bwrap bootstrap for one owned Hermes reflection endpoint.

The Rust owner embeds this source and supplies one fixed AF_UNIX socketpair.
The bootstrap applies Landlock before consuming the one-run secret, verifies
all currently defined cross-bindings, emits a bounded V2 containment frame,
and relays bounded HTTP to a loopback endpoint inside the private namespace.

Scripted fixture mode is explicitly labelled and never imports or invokes
Hermes or a model. Official mode remains fail-closed because the current
pinned closure does not contain its verified Hermes v2026.8.3 executable.
"""

import ctypes
import errno
import hashlib
import json
import os
import pathlib
import re
import socket
import sys
import time


INIT_MAGIC = b"LATTICE_HERMES_PRODUCTION_INIT_V1\n"
HTTP_REQUEST_MAGIC = b"LATTICE_HERMES_HTTP_REQUEST_V1\n"
HTTP_RESPONSE_MAGIC = b"LATTICE_HERMES_HTTP_RESPONSE_V1\n"
CONTAINMENT_MAGIC = b"LATTICE_HERMES_CONTAINED_V2\n"
MAX_CONTROL_BYTES = 2 * 1024 * 1024
MAX_REFLECTION_BYTES = 64 * 1024
MAX_EXECUTABLE_BYTES = 32 * 1024 * 1024
EXPECTED_BWRAP_SHA256 = "8e19e40e7d5f7a7e8b488c7926feb040eab6ed10c58fa360e266d2f70670e92b"
OFFICIAL_HERMES_CANDIDATE = "/runtime-input/python/bin/hermes"


def fail(code):
    raise SystemExit(code)


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def is_digest(value):
    return isinstance(value, str) and len(value) == 64 and all(
        char in "0123456789abcdef" for char in value
    )


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
    elif value["fixture_reflection"] is not None:
        fail(72)
    return value


def parse_init(connection):
    return parse_init_payload(receive_frame(connection, INIT_MAGIC))


def config_digest(init):
    value = {
        "api_key_sha256": sha256_bytes(init["api_key"].encode("utf-8")),
        "endpoint": init["endpoint"],
        "model": init["model"],
        "nonce": init["nonce"],
        "schema": "lattice.hermes.production-config.v1",
    }
    return sha256_bytes(canonical_json(value))


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


def fixture_response(init, request, state):
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
    def __init__(self, init, deadline):
        host, port_text = init["endpoint"].rsplit(":", 1)
        self.init = init
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
            response = fixture_response(self.init, observed, self.state)
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


def reject_unstaged_official_server():
    # This transport slice must never turn fixture evidence into an official
    # attestation. The current pinned closure lacks this verified executable;
    # an unexpected future file also requires the separate official lifecycle.
    pathlib.Path(OFFICIAL_HERMES_CANDIDATE).is_file()
    fail(74)


def serve_contained_reflection(connection, bwrap_sha256, net_namespace, namespace_pid):
    init = parse_init(connection)
    bindings = verified_bindings(init, bwrap_sha256, net_namespace, namespace_pid)
    if init["mode"] == "official":
        reject_unstaged_official_server()
    deadline = time.monotonic() + init["deadline_millis"] / 1000.0
    fixture = ScriptedFixtureEndpoint(init, deadline)
    try:
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
            response = fixture.relay(request, deadline)
            send_frame(connection, HTTP_RESPONSE_MAGIC, response, deadline)
    finally:
        fixture.close()


def main(arguments):
    if arguments != ["contained-reflection"]:
        fail(64)
    require_empty_work()
    apply_write_landlock()
    verify_write_boundaries()
    verify_network_is_private()
    broker = verify_broker_socket()
    try:
        bwrap_sha256 = digest_file("/usr/bin/bwrap", MAX_EXECUTABLE_BYTES)
        try:
            net_namespace = os.readlink("/proc/self/ns/net")
        except OSError:
            fail(73)
        serve_contained_reflection(broker, bwrap_sha256, net_namespace, os.getpid())
    finally:
        broker.close()


if __name__ == "__main__":
    main(sys.argv[1:])
