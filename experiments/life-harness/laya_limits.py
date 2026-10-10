"""Small, process-local limits for the trusted Windows offline Laya worker.

Call apply_memory_limit() and deny_network() before importing torch or the SDK.
The Job Object limits committed memory, not RSS. get_peak_rss() reports the
separate Windows PeakWorkingSetSize counter, in bytes.

deny_network() guards CPython socket paths; it is NOT a firewall or a sandbox.
Native libraries, subprocesses, hostile code, and previously connected raw
descriptors are outside that guarantee. Use only reviewed local SDK/model code.
"""

import ctypes
from ctypes import wintypes
import os
import socket
import sys


MEMORY_LIMIT_BYTES = 8 * 1024**3
_LIMIT_FLAGS = 0x100 | 0x200 | 0x2000  # process/job commit, kill on last handle close
_EXTENDED_LIMIT_INFORMATION = 9
_job_handle = None
_network_denied = False
_AUDIT_PROBE = "life_harness.network_guard_probe"


class _BasicLimits(ctypes.Structure):
    _fields_ = [
        ("PerProcessUserTimeLimit", ctypes.c_int64),
        ("PerJobUserTimeLimit", ctypes.c_int64),
        ("LimitFlags", wintypes.DWORD),
        ("MinimumWorkingSetSize", ctypes.c_size_t),
        ("MaximumWorkingSetSize", ctypes.c_size_t),
        ("ActiveProcessLimit", wintypes.DWORD),
        ("Affinity", ctypes.c_size_t),
        ("PriorityClass", wintypes.DWORD),
        ("SchedulingClass", wintypes.DWORD),
    ]


class _IoCounters(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint64) for name in (
        "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
        "ReadTransferCount", "WriteTransferCount", "OtherTransferCount",
    )]


class _JobLimits(ctypes.Structure):
    _fields_ = [
        ("BasicLimitInformation", _BasicLimits),
        ("IoInfo", _IoCounters),
        ("ProcessMemoryLimit", ctypes.c_size_t),
        ("JobMemoryLimit", ctypes.c_size_t),
        ("PeakProcessMemoryUsed", ctypes.c_size_t),
        ("PeakJobMemoryUsed", ctypes.c_size_t),
    ]


class _MemoryCounters(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in (
            "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
            "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
            "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage",
        )
    ]


def _kernel32():
    if sys.platform != "win32" or ctypes.sizeof(ctypes.c_void_p) != 8:
        raise RuntimeError("The bounded Laya worker requires 64-bit Windows")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    for name, arguments, result in (
        ("CreateJobObjectW", [ctypes.c_void_p, wintypes.LPCWSTR], wintypes.HANDLE),
        ("SetInformationJobObject", [wintypes.HANDLE, ctypes.c_int,
                                     ctypes.c_void_p, wintypes.DWORD], wintypes.BOOL),
        ("QueryInformationJobObject", [wintypes.HANDLE, ctypes.c_int,
                                       ctypes.c_void_p, wintypes.DWORD,
                                       ctypes.c_void_p], wintypes.BOOL),
        ("AssignProcessToJobObject", [wintypes.HANDLE, wintypes.HANDLE], wintypes.BOOL),
        ("GetCurrentProcess", [], wintypes.HANDLE),
        ("CloseHandle", [wintypes.HANDLE], wintypes.BOOL),
    ):
        function = getattr(kernel, name)
        function.argtypes = arguments
        function.restype = result
    return kernel


def _require_success(result, operation):
    if not result:
        error = ctypes.WinError(ctypes.get_last_error())
        raise RuntimeError(f"{operation} failed; refusing an unbounded worker") from error


def apply_memory_limit():
    """Assign this process to an 8 GiB job; fail closed and return OS readback.

The handle is deliberately retained until OS process teardown, with no atexit
close: closing the last handle while inside this job would kill the worker
before its output buffers finish flushing. Call from the single startup thread.
"""
    global _job_handle
    if _job_handle is None and any(name in sys.modules for name in ("torch", "transformers")):
        raise RuntimeError("Apply the memory limit before importing torch/transformers")
    kernel = _kernel32()
    if _job_handle is None:
        limits = _JobLimits()
        limits.BasicLimitInformation.LimitFlags = _LIMIT_FLAGS
        limits.ProcessMemoryLimit = MEMORY_LIMIT_BYTES
        limits.JobMemoryLimit = MEMORY_LIMIT_BYTES
        # NULL security attributes create a non-inheritable, unnamed handle.
        handle = kernel.CreateJobObjectW(None, None)
        _require_success(handle, "CreateJobObjectW")
        try:
            _require_success(kernel.SetInformationJobObject(
                handle, _EXTENDED_LIMIT_INFORMATION,
                ctypes.byref(limits), ctypes.sizeof(limits),
            ), "SetInformationJobObject")
            _require_success(kernel.AssignProcessToJobObject(
                handle, kernel.GetCurrentProcess(),
            ), "AssignProcessToJobObject")
        except BaseException:
            # The process has not joined this job if either operation failed.
            kernel.CloseHandle(handle)
            raise
        _job_handle = handle  # Never close a successfully assigned job here.

    actual = _JobLimits()
    _require_success(kernel.QueryInformationJobObject(
        _job_handle, _EXTENDED_LIMIT_INFORMATION,
        ctypes.byref(actual), ctypes.sizeof(actual), None,
    ), "QueryInformationJobObject")
    if (actual.BasicLimitInformation.LimitFlags & _LIMIT_FLAGS != _LIMIT_FLAGS
            or actual.ProcessMemoryLimit != MEMORY_LIMIT_BYTES
            or actual.JobMemoryLimit != MEMORY_LIMIT_BYTES):
        raise RuntimeError("Job Object readback does not enforce the required 8 GiB limits")
    return {
        "process_commit_limit_bytes": int(actual.ProcessMemoryLimit),
        "job_commit_limit_bytes": int(actual.JobMemoryLimit),
        "kill_on_job_close": True,
    }


def get_peak_rss():
    """Return this process's Windows PeakWorkingSetSize in bytes, or raise."""
    kernel = _kernel32()
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    psapi.GetProcessMemoryInfo.argtypes = [
        wintypes.HANDLE, ctypes.POINTER(_MemoryCounters), wintypes.DWORD,
    ]
    psapi.GetProcessMemoryInfo.restype = wintypes.BOOL
    counters = _MemoryCounters()
    counters.cb = ctypes.sizeof(counters)
    _require_success(psapi.GetProcessMemoryInfo(
        kernel.GetCurrentProcess(), ctypes.byref(counters), counters.cb,
    ), "GetProcessMemoryInfo")
    return int(counters.PeakWorkingSetSize)


class NetworkAccessDenied(PermissionError):
    """A guarded Python networking operation was attempted in offline mode."""


def _deny_socket_operation(*args, **kwargs):
    raise NetworkAccessDenied("Network access is disabled for the offline Laya worker")


_NETWORK_AUDIT_EVENTS = frozenset({
    "socket.__new__", "socket.bind", "socket.connect", "socket.getaddrinfo",
    "socket.gethostbyaddr", "socket.gethostbyname", "socket.getnameinfo",
    "socket.sendmsg", "socket.sendto",
})


def _network_audit_hook(event, args):
    if event in _NETWORK_AUDIT_EVENTS or event == _AUDIT_PROBE:
        _deny_socket_operation()


def deny_network():
    """Permanently deny supported socket paths in this CPython worker process.

The audit hook also covers direct _socket calls and saved connect/DNS functions.
Patched socket send methods cover normal use of pre-existing Python sockets.
This does not promise protection from native network stacks or hostile code.
"""
    global _network_denied
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["TRANSFORMERS_OFFLINE"] = "1"
    os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
    if _network_denied:
        return
    sys.addaudithook(_network_audit_hook)
    # An earlier audit hook can reject registration without raising to this
    # caller. Verify installation before allowing the worker to continue.
    try:
        sys.audit(_AUDIT_PROBE)
    except NetworkAccessDenied:
        pass
    else:
        raise RuntimeError("Could not install the offline network guard")
    for name in ("connect", "connect_ex", "send", "sendall", "sendto", "sendmsg",
                 "sendfile", "bind", "listen", "accept"):
        if hasattr(socket.socket, name):
            setattr(socket.socket, name, _deny_socket_operation)
    for name in ("create_connection", "create_server", "getaddrinfo", "gethostbyname",
                 "gethostbyname_ex", "gethostbyaddr", "getnameinfo"):
        if hasattr(socket, name):
            setattr(socket, name, _deny_socket_operation)
    _network_denied = True
