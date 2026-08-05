"""Offline socketpair tests for the in-bwrap Hermes sandbox runner."""

import importlib.util
import hashlib
import json
import os
import pathlib
import socket
import subprocess
import threading
import time
import unittest


RUNNER_PATH = pathlib.Path(__file__).parents[1] / "src" / "hermes_sandbox_runner.py"
SPEC = importlib.util.spec_from_file_location("lattice_hermes_sandbox_runner", RUNNER_PATH)
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)

BASE_RUNTIME_ROOT = pathlib.Path(
    "/var/tmp/lattice-runtime-targets/"
    "hermes-v2026.8.3-cpython-3.12.13-pbs-20260804"
)
OFFICIAL_RUNTIME_ROOT = pathlib.Path(
    "/var/tmp/lattice-runtime-targets/"
    "hermes-v2026.8.3-cpython-3.12.13-pbs-20260804-offline-final-2UEmH84h"
)

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


def read_exact(connection, length):
    output = bytearray()
    while len(output) < length:
        chunk = connection.recv(length - len(output))
        if not chunk:
            raise AssertionError("synthetic broker closed before the frame completed")
        output.extend(chunk)
    return bytes(output)


def send_frame(connection, magic, payload):
    connection.sendall(magic + len(payload).to_bytes(4, "big") + payload)


def receive_frame(connection, magic):
    assert read_exact(connection, len(magic)) == magic
    length = int.from_bytes(read_exact(connection, 4), "big")
    assert 0 < length <= RUNNER.MAX_CONTROL_BYTES
    return read_exact(connection, length)


def receive_containment_frame(connection):
    assert read_exact(connection, len(RUNNER.CONTAINMENT_MAGIC)) == RUNNER.CONTAINMENT_MAGIC
    fields = []
    for index in range(13):
        length = int.from_bytes(read_exact(connection, 4), "big")
        bound = 64 if index <= 9 else (16 if index == 10 else (32 if index == 11 else 64 * 1024))
        assert 0 < length <= bound
        fields.append(read_exact(connection, length))
    return fields


def free_loopback_endpoint():
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    endpoint = "%s:%d" % listener.getsockname()
    listener.close()
    return endpoint


def canonical_json(value):
    return json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True).encode(
        "ascii"
    )


def request(method, path, api_key, body=b""):
    return (
        ("%s %s HTTP/1.1\r\n" % (method, path)).encode("ascii")
        + b"Host: 127.0.0.1\r\n"
        + ("Authorization: Bearer %s\r\n" % api_key).encode("ascii")
        + b"Content-Type: application/json\r\n"
        + ("Content-Length: %d\r\n" % len(body)).encode("ascii")
        + b"Connection: close\r\n\r\n"
        + body
    )


class BwrapRunner:
    def __init__(self, runtime_root=BASE_RUNTIME_ROOT):
        bwrap = pathlib.Path("/usr/bin/bwrap")
        python = runtime_root / "python" / "bin" / "python3.12"
        if os.name != "posix" or not bwrap.is_file() or not python.is_file():
            raise unittest.SkipTest("requires the pinned WSL bwrap/Python runtime")
        self.peer, child = socket.socketpair()
        self.proxy_peer, proxy_child = socket.socketpair()
        has_hermes = (runtime_root / "python" / "bin" / "hermes").is_file()
        self.peer.settimeout(7.0 if has_hermes else 2.0)
        self.proxy_peer.settimeout(2.0)
        source = RUNNER_PATH.read_text(encoding="utf-8")
        command = [
            str(bwrap),
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
            str(runtime_root),
            "/runtime-input",
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
        ]
        for name, value in (
            ("PATH", "/runtime-input/python/bin:/usr/bin:/bin"),
            ("HOME", "/state/hermes"),
            ("HERMES_HOME", "/state/hermes"),
            ("CODEX_HOME", "/state/codex-unavailable"),
            ("LATTICE_CODEX_BROKER_READ_FD", "0"),
            ("LATTICE_CODEX_BROKER_WRITE_FD", "1"),
            ("LATTICE_HERMES_CODEX_PROXY_FD", "2"),
            ("TMPDIR", "/tmp"),
            ("PYTHONDONTWRITEBYTECODE", "1"),
            ("PYTHONHASHSEED", "0"),
            ("PYTHONNOUSERSITE", "1"),
            ("PYTHONSAFEPATH", "1"),
            ("PYTHONUTF8", "1"),
            ("NO_COLOR", "1"),
            ("CI", "1"),
            ("TZ", "UTC"),
            ("LANG", "C.UTF-8"),
            ("LC_ALL", "C.UTF-8"),
        ):
            command.extend(("--setenv", name, value))
        command.extend(
            (
                "--chdir",
                "/work",
                "/runtime-input/python/bin/python3.12",
                "-I",
                "-S",
                "-B",
                "-c",
                source,
                "contained-reflection",
            )
        )
        self.process = subprocess.Popen(
            command,
            stdin=child,
            stdout=child,
            stderr=proxy_child,
            close_fds=True,
        )
        child.close()
        proxy_child.close()

    @property
    def exit_code(self):
        return self.process.poll()

    def wait(self):
        try:
            self.process.wait(timeout=2.0)
        except subprocess.TimeoutExpired:
            raise AssertionError("sandbox runner did not reach its terminal state")

    def close(self):
        self.peer.close()
        self.proxy_peer.close()
        try:
            self.process.wait(timeout=2.0)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=2.0)
            raise AssertionError("sandbox runner did not stop after broker EOF")


def host_proxy_frame(kind, sequence, binding, payload):
    body = (
        bytes((kind,))
        + (1).to_bytes(4, "big")
        + sequence.to_bytes(4, "big")
        + binding
        + payload
    )
    return RUNNER.CODEX_PROXY_MAGIC + len(body).to_bytes(4, "big") + body


def receive_host_proxy_frame(connection):
    magic = read_exact(connection, len(RUNNER.CODEX_PROXY_MAGIC))
    if magic != RUNNER.CODEX_PROXY_MAGIC:
        raise AssertionError("wrong Codex proxy frame magic")
    length = int.from_bytes(read_exact(connection, 4), "big")
    if length < 41 or length > 41 + RUNNER.MAX_CODEX_PROXY_DATA_BYTES:
        raise AssertionError("Codex proxy frame length escaped its bound")
    body = read_exact(connection, length)
    return (
        body[0],
        int.from_bytes(body[1:5], "big"),
        int.from_bytes(body[5:9], "big"),
        body[9:41],
        body[41:],
    )


def capture_pre_magic_diagnostic(connection, limit=4096):
    observed = bytearray()
    while len(observed) <= limit:
        chunk = connection.recv(min(512, limit + 1 - len(observed)))
        if not chunk:
            break
        observed.extend(chunk)
        if len(observed) >= len(RUNNER.CODEX_PROXY_MAGIC):
            break
    if bytes(observed).startswith(RUNNER.CODEX_PROXY_MAGIC):
        raise AssertionError("expected a pre-magic launch diagnostic")
    if not observed or len(observed) > limit:
        raise AssertionError("pre-magic diagnostic escaped its strict bound")
    return len(observed), hashlib.sha256(observed).hexdigest()


class SyntheticCodexHost:
    def __init__(self, connection, binding, mode="success"):
        self.connection = connection
        self.binding = binding
        self.mode = mode
        self.failure = None
        self.request_bytes = bytearray()
        self.response_bytes = bytearray()
        self.open_count = 0
        self.error_code = None
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def send(self, kind, sequence, payload=b"", binding=None):
        self.connection.sendall(
            host_proxy_frame(
                kind,
                sequence,
                self.binding if binding is None else binding,
                payload,
            )
        )

    def expect_error(self, expected_code):
        kind, stream_id, sequence, binding, payload = receive_host_proxy_frame(self.connection)
        if (
            kind != RUNNER.CODEX_PROXY_ERROR
            or stream_id != 1
            or sequence != 1
            or binding != self.binding
            or len(payload) != 2
        ):
            raise AssertionError("runner did not emit the strict terminal error frame")
        self.error_code = int.from_bytes(payload, "big")
        if self.error_code != expected_code:
            raise AssertionError("runner emitted the wrong proxy error code")

    def run(self):
        try:
            kind, stream_id, sequence, binding, payload = receive_host_proxy_frame(
                self.connection
            )
            if (
                kind != RUNNER.CODEX_PROXY_OPEN
                or stream_id != 1
                or sequence != 0
                or binding != self.binding
                or payload
            ):
                raise AssertionError("runner did not emit the exact OPEN frame")
            self.open_count += 1
            if self.mode == "wrong_binding":
                self.send(RUNNER.CODEX_PROXY_OPEN, 0, binding=b"\xff" * 32)
                self.expect_error(RUNNER.PROXY_ERROR_BINDING)
                return
            if self.mode == "unknown":
                self.send(9, 0)
                self.expect_error(RUNNER.PROXY_ERROR_PROTOCOL)
                return
            if self.mode == "oversize":
                self.send(RUNNER.CODEX_PROXY_DATA, 0, b"x" * (64 * 1024 + 1))
                self.expect_error(RUNNER.PROXY_ERROR_SIZE)
                return
            if self.mode == "stall":
                while self.connection.recv(1):
                    pass
                return

            self.send(RUNNER.CODEX_PROXY_OPEN, 0)
            receive_sequence = 1
            send_sequence = 1
            responded = False
            while True:
                kind, stream_id, sequence, binding, payload = receive_host_proxy_frame(
                    self.connection
                )
                if stream_id != 1 or sequence != receive_sequence or binding != self.binding:
                    raise AssertionError("runner Codex proxy sequence/binding drifted")
                receive_sequence += 1
                if kind == RUNNER.CODEX_PROXY_DATA:
                    self.request_bytes.extend(payload)
                    if not FIXTURE_CODEX_REQUEST.startswith(self.request_bytes):
                        raise AssertionError("runner changed JSON-RPC request bytes")
                    if bytes(self.request_bytes) == FIXTURE_CODEX_REQUEST and not responded:
                        fragments = (
                            FIXTURE_CODEX_RESPONSE[:7],
                            FIXTURE_CODEX_RESPONSE[7:41],
                            FIXTURE_CODEX_RESPONSE[41:],
                        )
                        for fragment in fragments:
                            self.send(RUNNER.CODEX_PROXY_DATA, send_sequence, fragment)
                            self.response_bytes.extend(fragment)
                            send_sequence += 1
                        responded = True
                elif kind == RUNNER.CODEX_PROXY_CLOSE:
                    if payload or not responded:
                        raise AssertionError("runner closed before the byte relay completed")
                    self.send(RUNNER.CODEX_PROXY_CLOSE, send_sequence)
                    kind, stream_id, sequence, binding, payload = receive_host_proxy_frame(
                        self.connection
                    )
                    if (
                        kind != RUNNER.CODEX_PROXY_TERMINAL
                        or stream_id != 1
                        or sequence != receive_sequence
                        or binding != self.binding
                        or payload
                    ):
                        raise AssertionError("runner omitted the bound terminal frame")
                    return
                else:
                    raise AssertionError("runner emitted an unexpected proxy frame")
        except BaseException as failure:
            self.failure = failure

    def join(self, timeout=3.0):
        self.thread.join(timeout)
        if self.thread.is_alive():
            raise AssertionError("synthetic Codex host did not reach a terminal state")
        if self.failure is not None:
            raise self.failure


class HermesSandboxRunnerFixtureTests(unittest.TestCase):
    def init(self, mode="scripted_fixture"):
        value = {
            "api_key": "offline-fixture-key",
            "broker_receipt_sha256": "b" * 64,
            "config_sha256": "0" * 64,
            "deadline_millis": 5_000,
            "endpoint": free_loopback_endpoint(),
            "fixture_reflection": (
                '{"schema_version":"lattice.hermes.reflection.v1"}'
                if mode == "scripted_fixture"
                else None
            ),
            "mode": mode,
            "model": "hermes-agent",
            "nonce": "01" * 32,
            "runtime_manifest_sha256": "a" * 64,
        }
        value["config_sha256"] = RUNNER.config_digest(value)
        return value

    def send_init(self, runner, init):
        send_frame(runner.peer, RUNNER.INIT_MAGIC, canonical_json(init))

    def relay(self, runner, raw_request):
        send_frame(runner.peer, RUNNER.HTTP_REQUEST_MAGIC, raw_request)
        return receive_frame(runner.peer, RUNNER.HTTP_RESPONSE_MAGIC)

    def run_proxy_rejection(self, mode, deadline_millis=5_000):
        runner = BwrapRunner()
        init = self.init()
        init["deadline_millis"] = deadline_millis
        codex_host = None
        try:
            self.send_init(runner, init)
            receive_containment_frame(runner.peer)
            codex_host = SyntheticCodexHost(
                runner.proxy_peer,
                RUNNER.codex_proxy_binding(init),
                mode,
            )
            submitted = canonical_json(
                {"model": "hermes-agent", "session_id": "offline-session"}
            )
            send_frame(
                runner.peer,
                RUNNER.HTTP_REQUEST_MAGIC,
                request("POST", "/v1/runs", init["api_key"], submitted),
            )
            runner.wait()
            codex_host.join()
            return runner.exit_code, codex_host
        finally:
            runner.close()

    def test_pre_magic_diagnostic_is_hashed_not_parsed_as_codex_data(self):
        host, child = socket.socketpair()
        try:
            diagnostic = b"bwrap: bounded pre-exec failure\n"
            child.sendall(diagnostic)
            child.shutdown(socket.SHUT_WR)
            byte_count, digest = capture_pre_magic_diagnostic(host)
        finally:
            host.close()
            child.close()
        self.assertEqual(byte_count, len(diagnostic))
        self.assertEqual(digest, hashlib.sha256(diagnostic).hexdigest())

    def test_codex_proxy_wire_is_u32_bounded_and_binding_strict(self):
        init = self.init()
        binding = RUNNER.codex_proxy_binding(init)
        frame = RUNNER.encode_codex_proxy_frame(
            RUNNER.CODEX_PROXY_DATA,
            1,
            binding,
            b'{"id":0}\n',
        )
        self.assertEqual(
            frame[: len(RUNNER.CODEX_PROXY_MAGIC)],
            RUNNER.CODEX_PROXY_MAGIC,
        )
        body_offset = len(RUNNER.CODEX_PROXY_MAGIC) + 4
        self.assertEqual(
            int.from_bytes(
                frame[len(RUNNER.CODEX_PROXY_MAGIC) : body_offset],
                "big",
            ),
            len(frame) - body_offset,
        )
        decoded = RUNNER.decode_codex_proxy_body(frame[body_offset:], 1, binding)
        self.assertEqual(decoded, (RUNNER.CODEX_PROXY_DATA, b'{"id":0}\n'))

        with self.assertRaises(RUNNER.CodexProxyViolation) as out_of_order:
            RUNNER.decode_codex_proxy_body(frame[body_offset:], 2, binding)
        self.assertEqual(out_of_order.exception.error_code, RUNNER.PROXY_ERROR_SEQUENCE)

        wrong_binding = bytes.fromhex("ff" * 32)
        wrong = RUNNER.encode_codex_proxy_frame(
            RUNNER.CODEX_PROXY_OPEN,
            0,
            wrong_binding,
            b"",
        )
        with self.assertRaises(RUNNER.CodexProxyViolation) as rejected:
            RUNNER.decode_codex_proxy_body(wrong[body_offset:], 0, binding)
        self.assertEqual(rejected.exception.error_code, RUNNER.PROXY_ERROR_BINDING)

    def test_proxy_wrong_binding_fails_closed_with_terminal_error(self):
        exit_code, host = self.run_proxy_rejection("wrong_binding")
        self.assertEqual(exit_code, 80)
        self.assertEqual(host.open_count, 1)
        self.assertEqual(host.error_code, RUNNER.PROXY_ERROR_BINDING)

    def test_proxy_oversize_frame_fails_closed_with_terminal_error(self):
        exit_code, host = self.run_proxy_rejection("oversize")
        self.assertEqual(exit_code, 80)
        self.assertEqual(host.error_code, RUNNER.PROXY_ERROR_SIZE)

    def test_proxy_unknown_kind_fails_closed_with_terminal_error(self):
        exit_code, host = self.run_proxy_rejection("unknown")
        self.assertEqual(exit_code, 80)
        self.assertEqual(host.error_code, RUNNER.PROXY_ERROR_PROTOCOL)

    def test_proxy_deadline_fails_closed_without_fabricated_data(self):
        exit_code, host = self.run_proxy_rejection("stall", deadline_millis=250)
        self.assertEqual(exit_code, 79)
        self.assertEqual(host.open_count, 1)
        self.assertEqual(host.request_bytes, b"")

    def test_scripted_fixture_runs_full_reflection_over_private_broker(self):
        runner = BwrapRunner()
        init = self.init()
        try:
            self.send_init(runner, init)
            fields = receive_containment_frame(runner.peer)
            self.assertEqual(fields[0], init["runtime_manifest_sha256"].encode("ascii"))
            self.assertEqual(fields[1], init["config_sha256"].encode("ascii"))
            self.assertEqual(fields[3], init["broker_receipt_sha256"].encode("ascii"))
            self.assertEqual(fields[9], init["endpoint"].encode("ascii"))
            namespace_pid = int(fields[10].decode("ascii"))
            self.assertGreater(namespace_pid, 0)
            self.assertEqual(fields[11], b"scripted_fixture")
            attestation = json.loads(fields[12].decode("ascii"))
            self.assertEqual(canonical_json(attestation), fields[12])
            self.assertEqual(attestation["namespace_pid"], namespace_pid)

            capabilities = self.relay(
                runner, request("GET", "/v1/capabilities", init["api_key"])
            )
            self.assertIn(b"HTTP/1.1 200 OK", capabilities)
            self.assertIn(b'"model":"hermes-agent"', capabilities)

            submitted = canonical_json(
                {"model": "hermes-agent", "session_id": "offline-session"}
            )
            codex_host = SyntheticCodexHost(
                runner.proxy_peer,
                RUNNER.codex_proxy_binding(init),
            )
            started = self.relay(
                runner,
                request("POST", "/v1/runs", init["api_key"], submitted),
            )
            self.assertIn(b"HTTP/1.1 202 Accepted", started)
            self.assertIn(b'"run_id":"run_contained_fixture"', started)
            codex_host.join()
            self.assertEqual(codex_host.open_count, 1)
            self.assertEqual(bytes(codex_host.request_bytes), FIXTURE_CODEX_REQUEST)
            self.assertEqual(bytes(codex_host.response_bytes), FIXTURE_CODEX_RESPONSE)

            events = self.relay(
                runner,
                request(
                    "GET",
                    "/v1/runs/run_contained_fixture/events",
                    init["api_key"],
                ),
            )
            self.assertIn(b"HTTP/1.1 200 OK", events)
            self.assertIn(b"run.completed", events)

            status = self.relay(
                runner,
                request("GET", "/v1/runs/run_contained_fixture", init["api_key"]),
            )
            self.assertIn(b"HTTP/1.1 200 OK", status)
            status_value = json.loads(status.split(b"\r\n\r\n", 1)[1])
            self.assertEqual(status_value["output"], init["fixture_reflection"])
        finally:
            runner.close()
        self.assertEqual(runner.exit_code, 0)

    def test_one_wrong_config_binding_fails_closed_before_frame(self):
        runner = BwrapRunner()
        init = self.init()
        init["config_sha256"] = "f" * 64
        try:
            self.send_init(runner, init)
            runner.wait()
            self.assertEqual(runner.exit_code, 73)
            self.assertEqual(runner.peer.recv(1), b"")
        finally:
            runner.close()

    def test_official_mode_without_staged_executable_fails_closed(self):
        runner = BwrapRunner()
        init = self.init("official")
        try:
            self.send_init(runner, init)
            runner.wait()
            self.assertEqual(runner.exit_code, 74)
            self.assertEqual(runner.peer.recv(1), b"")
        finally:
            runner.close()

    def test_official_mode_starts_pinned_gateway_and_reports_capabilities(self):
        hermes = OFFICIAL_RUNTIME_ROOT / "python" / "bin" / "hermes"
        if not hermes.is_file():
            raise unittest.SkipTest("requires the exact pinned Hermes runtime")
        runner = BwrapRunner(OFFICIAL_RUNTIME_ROOT)
        init = self.init("official")
        try:
            self.send_init(runner, init)
            try:
                fields = receive_containment_frame(runner.peer)
            except AssertionError:
                runner.wait()
                self.fail(
                    "official gateway closed before containment frame "
                    "(exit %s)" % runner.exit_code
                )
            self.assertEqual(fields[1], RUNNER.config_digest(init).encode("ascii"))
            self.assertEqual(fields[11], b"official")

            health_response = self.relay(
                runner,
                request("GET", "/health/detailed", init["api_key"]),
            )
            self.assertTrue(health_response.startswith(b"HTTP/1.1 200"))
            health = json.loads(health_response.split(b"\r\n\r\n", 1)[1])
            self.assertEqual(health["status"], "ok")
            self.assertEqual(health["readiness"]["checks"]["config"], {"status": "ok"})
            self.assertEqual(health["readiness"]["checks"]["model"], {"status": "ok"})
            self.assertEqual(health["active_agents"], 0)
            self.assertIs(health["gateway_busy"], False)

            response = self.relay(
                runner,
                request("GET", "/v1/capabilities", init["api_key"]),
            )
            self.assertTrue(response.startswith(b"HTTP/1.1 200"))
            capabilities = json.loads(response.split(b"\r\n\r\n", 1)[1])
            self.assertEqual(capabilities["object"], "hermes.api_server.capabilities")
            self.assertEqual(capabilities["platform"], "hermes-agent")
            self.assertEqual(capabilities["model"], "hermes-agent")
            self.assertEqual(capabilities["auth"], {"required": True, "type": "bearer"})
            self.assertEqual(capabilities["runtime"]["mode"], "server_agent")
            self.assertEqual(capabilities["runtime"]["tool_execution"], "server")
            self.assertIs(capabilities["runtime"]["split_runtime"], False)
            for feature in (
                "run_submission",
                "run_status",
                "run_events_sse",
                "run_stop",
            ):
                self.assertIs(capabilities["features"][feature], True)
            for feature in ("admin_config_rw", "memory_write_api"):
                self.assertIs(capabilities["features"][feature], False)
            self.assertIs(capabilities["features"]["responses_api"], True)
            self.assertEqual(
                capabilities["endpoints"]["runs"],
                {"method": "POST", "path": "/v1/runs"},
            )
            runner.proxy_peer.settimeout(0.1)
            with self.assertRaises(socket.timeout):
                runner.proxy_peer.recv(1)
        finally:
            runner.close()
        self.assertEqual(runner.exit_code, 0)


if __name__ == "__main__":
    unittest.main()
