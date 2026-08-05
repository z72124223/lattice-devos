"""Offline socketpair tests for the in-bwrap Hermes sandbox runner."""

import importlib.util
import json
import os
import pathlib
import socket
import subprocess
import unittest


RUNNER_PATH = pathlib.Path(__file__).parents[1] / "src" / "hermes_sandbox_runner.py"
SPEC = importlib.util.spec_from_file_location("lattice_hermes_sandbox_runner", RUNNER_PATH)
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


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
    def __init__(self):
        runtime_root = pathlib.Path(
            "/var/tmp/lattice-runtime-targets/"
            "hermes-v2026.8.3-cpython-3.12.13-pbs-20260804"
        )
        bwrap = pathlib.Path("/usr/bin/bwrap")
        python = runtime_root / "python" / "bin" / "python3.12"
        if os.name != "posix" or not bwrap.is_file() or not python.is_file():
            raise unittest.SkipTest("requires the pinned WSL bwrap/Python runtime")
        self.peer, child = socket.socketpair()
        self.peer.settimeout(2.0)
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
            stderr=subprocess.PIPE,
            close_fds=True,
        )
        child.close()

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
        try:
            self.process.wait(timeout=2.0)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=2.0)
            raise AssertionError("sandbox runner did not stop after broker EOF")
        stderr = self.process.stderr.read(RUNNER.MAX_CONTROL_BYTES + 1)
        self.process.stderr.close()
        if stderr:
            raise AssertionError("sandbox runner wrote unexpected stderr")


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
            started = self.relay(
                runner,
                request("POST", "/v1/runs", init["api_key"], submitted),
            )
            self.assertIn(b"HTTP/1.1 202 Accepted", started)
            self.assertIn(b'"run_id":"run_contained_fixture"', started)

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

    def test_official_mode_is_blocked_without_staged_package(self):
        runner = BwrapRunner()
        init = self.init("official")
        try:
            self.send_init(runner, init)
            runner.wait()
            self.assertEqual(runner.exit_code, 74)
            self.assertEqual(runner.peer.recv(1), b"")
        finally:
            runner.close()


if __name__ == "__main__":
    unittest.main()
