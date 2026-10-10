"""Focused subprocess checks; never load a model or allocate near the 8 GiB cap."""

import json
from pathlib import Path
import subprocess
import sys
import textwrap
import unittest


HERE = Path(__file__).resolve().parent


class LayaLimitsTests(unittest.TestCase):
    def child(self, code):
        setup = f"import sys; sys.path.insert(0, {str(HERE)!r})\n"
        result = subprocess.run(
            [sys.executable, "-I", "-B", "-c", setup + textwrap.dedent(code)],
            cwd=HERE, capture_output=True, text=True, timeout=20,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)

    @unittest.skipUnless(sys.platform == "win32", "Windows Job Object API")
    def test_kernel_readback_and_peak_working_set(self):
        data = self.child("""
            import ctypes, json, struct
            from ctypes import wintypes
            import laya_limits as limits
            assert 'torch' not in sys.modules and 'transformers' not in sys.modules
            first = limits.apply_memory_limit()
            assert first == limits.apply_memory_limit()
            # Independently inspect the documented x64 Windows structure layout.
            kernel = ctypes.WinDLL('kernel32', use_last_error=True)
            query = kernel.QueryInformationJobObject
            query.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                              wintypes.DWORD, ctypes.c_void_p]
            query.restype = wintypes.BOOL
            buffer = ctypes.create_string_buffer(144)
            assert query(limits._job_handle, 9, buffer, len(buffer), None)
            flags = struct.unpack_from('<I', buffer.raw, 16)[0]
            process_limit, job_limit = struct.unpack_from('<QQ', buffer.raw, 112)
            in_job = wintypes.BOOL()
            kernel.GetCurrentProcess.restype = wintypes.HANDLE
            kernel.IsProcessInJob.argtypes = [wintypes.HANDLE, wintypes.HANDLE,
                                              ctypes.POINTER(wintypes.BOOL)]
            kernel.IsProcessInJob.restype = wintypes.BOOL
            assert kernel.IsProcessInJob(kernel.GetCurrentProcess(), limits._job_handle,
                                         ctypes.byref(in_job)) and in_job.value
            peak = limits.get_peak_rss()
            assert isinstance(peak, int) and peak > 0
            assert limits.get_peak_rss() >= peak
            print(json.dumps({'flags': flags, 'process': process_limit,
                              'job': job_limit, 'peak_rss': peak, 'readback': first}))
        """)
        self.assertEqual(data["flags"] & 0x2300, 0x2300)
        self.assertEqual(data["process"], 8 * 1024**3)
        self.assertEqual(data["job"], 8 * 1024**3)
        self.assertEqual(data["readback"]["process_commit_limit_bytes"], data["process"])
        self.assertTrue(data["readback"]["kill_on_job_close"])

    @unittest.skipUnless(sys.platform == "win32", "Windows Job Object API")
    def test_job_creation_configuration_and_assignment_fail_closed(self):
        for failing in ("CreateJobObjectW", "SetInformationJobObject", "AssignProcessToJobObject"):
            with self.subTest(failing=failing):
                data = self.child(f"""
                    import ctypes, json
                    import laya_limits as limits
                    closed = []
                    class Kernel:
                        def CreateJobObjectW(self, *args):
                            return 0 if {failing!r} == 'CreateJobObjectW' else 123
                        def SetInformationJobObject(self, *args):
                            return {failing!r} != 'SetInformationJobObject'
                        def GetCurrentProcess(self):
                            return -1
                        def AssignProcessToJobObject(self, *args):
                            return {failing!r} != 'AssignProcessToJobObject'
                        def CloseHandle(self, handle):
                            closed.append(handle)
                    limits._kernel32 = lambda: Kernel()
                    ctypes.set_last_error(5)
                    try:
                        limits.apply_memory_limit()
                    except RuntimeError as error:
                        assert {failing!r} in str(error)
                    else:
                        raise AssertionError('Unbounded startup was permitted')
                    assert limits._job_handle is None and 'torch' not in sys.modules
                    print(json.dumps({{'closed': closed}}))
                """)
                self.assertEqual(data["closed"], [] if failing == "CreateJobObjectW" else [123])

    def test_socket_dns_and_datagram_paths_are_denied(self):
        data = self.child("""
            import _socket, json, os, socket
            import laya_limits as limits
            # Retained references and sockets exercise audit coverage beyond patches.
            tcp = socket.socket()
            udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            saved_connect = tcp.connect
            saved_connect_ex = tcp.connect_ex
            saved_sendto = udp.sendto
            saved_dns = socket.getaddrinfo
            limits.deny_network()
            limits.deny_network()
            actions = {
                'new_socket': lambda: socket.socket(),
                'raw_new_socket': lambda: _socket.socket(),
                'connect': lambda: tcp.connect(('127.0.0.1', 9)),
                'connect_ex': lambda: tcp.connect_ex(('127.0.0.1', 9)),
                'saved_connect': lambda: saved_connect(('127.0.0.1', 9)),
                'saved_connect_ex': lambda: saved_connect_ex(('127.0.0.1', 9)),
                'raw_connect': lambda: _socket.socket.connect(tcp, ('127.0.0.1', 9)),
                'create_connection': lambda: socket.create_connection(('example.invalid', 443)),
                'getaddrinfo': lambda: socket.getaddrinfo('example.invalid', 443),
                'saved_dns': lambda: saved_dns('example.invalid', 443),
                'raw_dns': lambda: _socket.getaddrinfo('example.invalid', 443),
                'gethostbyname': lambda: socket.gethostbyname('example.invalid'),
                'gethostbyname_ex': lambda: _socket.gethostbyname_ex('example.invalid'),
                'gethostbyaddr': lambda: _socket.gethostbyaddr('192.0.2.1'),
                'getnameinfo': lambda: _socket.getnameinfo(('192.0.2.1', 443), 0),
                'send': lambda: tcp.send(b'x'),
                'sendall': lambda: tcp.sendall(b'x'),
                'sendto': lambda: udp.sendto(b'x', ('127.0.0.1', 9)),
                'saved_sendto': lambda: saved_sendto(b'x', ('127.0.0.1', 9)),
                'raw_sendto': lambda: _socket.socket.sendto(udp, b'x', ('127.0.0.1', 9)),
                'bind': lambda: tcp.bind(('127.0.0.1', 0)),
            }
            for name, action in actions.items():
                try:
                    action()
                except limits.NetworkAccessDenied:
                    continue
                raise AssertionError(name + ' was not blocked')
            tcp.close()
            udp.close()
            assert os.environ['HF_HUB_OFFLINE'] == '1'
            assert os.environ['TRANSFORMERS_OFFLINE'] == '1'
            print(json.dumps({'blocked': sorted(actions), 'count': len(actions)}))
        """)
        self.assertEqual(data["count"], 21)

    def test_rejected_audit_hook_installation_fails_closed(self):
        data = self.child("""
            import json
            import laya_limits as limits
            def reject_new_hooks(event, args):
                if event == 'sys.addaudithook':
                    raise RuntimeError('registration blocked')
            sys.addaudithook(reject_new_hooks)
            try:
                limits.deny_network()
            except RuntimeError as error:
                assert 'Could not install' in str(error)
            else:
                raise AssertionError('Missing audit guard was accepted')
            print(json.dumps({'rejected': True}))
        """)
        self.assertTrue(data["rejected"])


if __name__ == "__main__":
    unittest.main()
