"""Pinned in-bwrap bootstrap for one Hermes reflection.

The Rust backend embeds this source and supplies only compile-time-shaped
arguments. The bootstrap applies Landlock before any untrusted Hermes or Codex
code can run. It never installs packages and never opens an external network.
"""

import ctypes
import errno
import os
import pathlib
import socket
import sys


def fail(code):
    raise SystemExit(code)


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
    for descriptor in (0, 1):
        try:
            broker = socket.fromfd(descriptor, socket.AF_UNIX, socket.SOCK_STREAM)
            if broker.getsockopt(socket.SOL_SOCKET, socket.SO_TYPE) != socket.SOCK_STREAM:
                fail(71)
            broker.getpeername()
            broker.close()
        except OSError:
            fail(71)


def main(arguments):
    if arguments != ["contained-reflection"]:
        fail(64)
    require_empty_work()
    apply_write_landlock()
    verify_write_boundaries()
    verify_network_is_private()
    verify_broker_socket()
    # The frozen Hermes dependency closure and a bound broker receipt are not
    # yet admitted. Never execute the runtime without both identities.
    fail(74)


if __name__ == "__main__":
    main(sys.argv[1:])
