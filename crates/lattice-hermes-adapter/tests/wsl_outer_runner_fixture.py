"""Deterministic exception-boundary tests for the outer WSL relay."""

import importlib.util
import pathlib
import socket
import unittest


RUNNER_PATH = pathlib.Path(__file__).parents[1] / "src" / "wsl_outer_runner.py"
SPEC = importlib.util.spec_from_file_location("lattice_hermes_wsl_outer_runner", RUNNER_PATH)
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


class FakeConnection:
    def __init__(self, failure=None):
        self.failure = failure
        self.payloads = []

    def sendall(self, payload):
        self.payloads.append(payload)
        if self.failure is not None:
            raise self.failure


class ObserverResponseTests(unittest.TestCase):
    def test_success_sends_the_exact_response_once(self):
        connection = FakeConnection()
        RUNNER.send_response_to_observer(connection, b"HTTP/1.1 200 OK\r\n\r\n")
        self.assertEqual(connection.payloads, [b"HTTP/1.1 200 OK\r\n\r\n"])

    def test_abandoned_client_delivery_errors_are_ignored(self):
        for failure in (BrokenPipeError(), ConnectionResetError()):
            with self.subTest(failure=type(failure).__name__):
                connection = FakeConnection(failure)
                RUNNER.send_response_to_observer(connection, b"response")
                self.assertEqual(connection.payloads, [b"response"])

    def test_other_socket_errors_remain_fail_closed(self):
        for failure in (OSError("other socket error"), socket.timeout("deadline")):
            with self.subTest(failure=type(failure).__name__):
                connection = FakeConnection(failure)
                with self.assertRaises(type(failure)):
                    RUNNER.send_response_to_observer(connection, b"response")
                self.assertEqual(connection.payloads, [b"response"])


if __name__ == "__main__":
    unittest.main()
