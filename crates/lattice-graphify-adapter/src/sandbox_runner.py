"""Fixed private-copy bootstrap for the pinned Graphify sandbox.

This file is embedded into the Rust binary and passed to the reviewed system
Python with ``-I -S -B -c``.  It accepts only compile-time-shaped arguments.
"""

import ctypes
import errno
import hashlib
import os
import pathlib
import stat
import struct
import subprocess
import sys


MAGIC = b"LATTICE_GRAPHIFY_PRIVATE_V1\n"
CHILD = """import runpy,sys
sys.path.insert(0, '/runtime/site-packages')
sys.argv = ['graphify', *sys.argv[1:]]
runpy.run_module('graphify', run_name='__main__')
"""


def fail(code):
    raise SystemExit(code)


def parse_positive(value, maximum):
    if not value.isascii() or not value.isdecimal():
        fail(64)
    parsed = int(value)
    if parsed <= 0 or parsed > maximum:
        fail(64)
    return parsed


def checked_hex(value):
    if len(value) != 64 or any(char not in "0123456789abcdef" for char in value):
        fail(64)
    return value


def copy_regular(source, destination):
    flags = os.O_RDONLY | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(source, flags)
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            fail(65)
        destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        digest = hashlib.sha256()
        total = 0
        with open(destination, "xb", buffering=0) as output:
            while True:
                chunk = os.read(descriptor, 1024 * 1024)
                if not chunk:
                    break
                output.write(chunk)
                digest.update(chunk)
                total += len(chunk)
        after = os.fstat(descriptor)
        if (before.st_dev, before.st_ino, before.st_size) != (
            after.st_dev,
            after.st_ino,
            after.st_size,
        ) or total != before.st_size:
            fail(65)
        return total, digest.hexdigest()
    finally:
        os.close(descriptor)


def validate_relative(relative):
    text = relative.as_posix()
    if not text or not text.isascii() or "\x00" in text or "\r" in text or "\n" in text:
        fail(65)
    if any(component in ("", ".", "..") for component in relative.parts):
        fail(65)
    return text


def copy_tree(source_root, destination_root, runtime):
    entries = []
    pending = [(pathlib.Path(source_root), pathlib.PurePosixPath())]
    while pending:
        directory, prefix = pending.pop()
        try:
            children = sorted(os.scandir(directory), key=lambda entry: entry.name.encode("utf-8"))
        except OSError:
            fail(65)
        for entry in children:
            relative = prefix / entry.name
            text = validate_relative(relative)
            try:
                metadata = entry.stat(follow_symlinks=False)
            except OSError:
                fail(65)
            if entry.is_symlink():
                fail(65)
            if stat.S_ISDIR(metadata.st_mode):
                if not runtime and text == ".git":
                    continue
                pending.append((pathlib.Path(entry.path), relative))
                continue
            if not stat.S_ISREG(metadata.st_mode):
                fail(65)
            in_pycache = "__pycache__" in relative.parts
            bytecode = relative.suffix.lower() in (".pyc", ".pyo")
            if runtime and in_pycache and bytecode:
                continue
            if runtime and (in_pycache or bytecode):
                fail(65)
            length, digest = copy_regular(entry.path, pathlib.Path(destination_root) / text)
            entries.append((text, length, digest))
    entries.sort(key=lambda item: item[0].encode("utf-8"))
    return entries


def runtime_manifest(entries):
    encoded = bytearray()
    for relative, length, digest in entries:
        encoded.extend(relative.encode("ascii"))
        encoded.extend(b"\x00")
        encoded.extend(str(length).encode("ascii"))
        encoded.extend(b"\x00")
        encoded.extend(digest.encode("ascii"))
        encoded.extend(b"\n")
    return hashlib.sha256(encoded).hexdigest()


def snapshot_manifest(entries):
    digest = hashlib.sha256()
    for relative, length, content_digest in entries:
        for field in (
            relative.encode("ascii"),
            content_digest.encode("ascii"),
            length.to_bytes(8, "big"),
        ):
            digest.update(len(field).to_bytes(8, "big"))
            digest.update(field)
    return digest.hexdigest()


def require_shape(entries, expected_count, expected_bytes, expected_digest, manifest, code):
    if len(entries) != expected_count or sum(item[1] for item in entries) != expected_bytes:
        fail(code)
    if manifest(entries) != expected_digest:
        fail(code)


def apply_write_landlock():
    libc = ctypes.CDLL(None, use_errno=True)
    syscall = libc.syscall
    syscall.restype = ctypes.c_long
    create_ruleset, add_rule, restrict_self = 444, 445, 446
    version = syscall(create_ruleset, 0, 0, 1)
    if version < 3:
        fail(67)
    write_access = (1 << 1) | sum(1 << bit for bit in range(4, 13))
    write_access |= (1 << 13) | (1 << 14)

    class RulesetAttr(ctypes.Structure):
        _fields_ = [("handled_access_fs", ctypes.c_uint64)]

    class PathBeneathAttr(ctypes.Structure):
        _fields_ = [("allowed_access", ctypes.c_uint64), ("parent_fd", ctypes.c_int32)]

    ruleset_attr = RulesetAttr(write_access)
    ruleset_fd = syscall(create_ruleset, ctypes.byref(ruleset_attr), ctypes.sizeof(ruleset_attr), 0)
    if ruleset_fd < 0:
        fail(67)
    try:
        allowed_paths = [("/output", write_access), ("/tmp", write_access)]
        dev_null_access = (1 << 1) | (1 << 14)
        allowed_paths.append(("/dev/null", dev_null_access))
        for allowed_path, allowed_access in allowed_paths:
            path_fd = os.open(allowed_path, os.O_PATH | os.O_CLOEXEC)
            try:
                path_attr = PathBeneathAttr(allowed_access, path_fd)
                if syscall(add_rule, ruleset_fd, 1, ctypes.byref(path_attr), 0) != 0:
                    fail(67)
            finally:
                os.close(path_fd)
        if libc.prctl(38, 1, 0, 0, 0) != 0:
            fail(67)
        if syscall(restrict_self, ruleset_fd, 0) != 0:
            fail(67)
    finally:
        os.close(ruleset_fd)


def verify_truncate_denial():
    try:
        os.truncate("/runtime/install-report.json", 0)
    except OSError as error:
        if error.errno not in (errno.EACCES, errno.EPERM):
            fail(67)
    else:
        fail(67)


def graphify(command, diagnostic_limit):
    environment = {
        "HOME": "/home/lattice",
        "TMPDIR": "/tmp",
        "XDG_CACHE_HOME": "/tmp/cache",
        "XDG_CONFIG_HOME": "/tmp/config",
        "PYTHONDONTWRITEBYTECODE": "1",
        "PYTHONHASHSEED": "0",
        "PYTHONNOUSERSITE": "1",
        "PYTHONSAFEPATH": "1",
        "PYTHONUTF8": "1",
        "GRAPHIFY_QUERY_LOG_DISABLE": "1",
        "GRAPHIFY_MAX_WORKERS": "1",
        "NO_COLOR": "1",
        "CI": "1",
        "TZ": "UTC",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
    }
    result = subprocess.run(
        [sys.executable, "-I", "-S", "-B", "-c", CHILD, *command],
        cwd="/output",
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        close_fds=True,
        check=False,
    )
    if result.returncode != 0:
        fail(68)
    if len(result.stdout) > diagnostic_limit or len(result.stderr) > diagnostic_limit:
        fail(69)
    return result.stdout, result.stderr


def read_graph(limit):
    root = os.open("/output", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        directory = os.open(
            "graphify-out",
            os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW,
            dir_fd=root,
        )
        try:
            graph = os.open("graph.json", os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=directory)
            try:
                metadata = os.fstat(graph)
                if not stat.S_ISREG(metadata.st_mode) or metadata.st_size <= 0 or metadata.st_size > limit:
                    fail(70)
                chunks = []
                total = 0
                while True:
                    chunk = os.read(graph, min(1024 * 1024, limit + 1 - total))
                    if not chunk:
                        break
                    chunks.append(chunk)
                    total += len(chunk)
                    if total > limit:
                        fail(70)
                if total != metadata.st_size:
                    fail(70)
                return b"".join(chunks)
            finally:
                os.close(graph)
        finally:
            os.close(directory)
    finally:
        os.close(root)


def emit(fields):
    output = sys.stdout.buffer
    output.write(MAGIC)
    for field in fields:
        output.write(struct.pack(">Q", len(field)))
        output.write(field)
    output.flush()


def main(arguments):
    if len(arguments) != 10 or arguments[0] != "extract":
        fail(64)
    runtime_digest = checked_hex(arguments[1])
    runtime_count = parse_positive(arguments[2], 1_000_000)
    runtime_bytes = parse_positive(arguments[3], 8 * 1024 * 1024 * 1024)
    install_digest = checked_hex(arguments[4])
    source_digest = checked_hex(arguments[5])
    source_count = parse_positive(arguments[6], 1_000_000)
    source_bytes = parse_positive(arguments[7], 8 * 1024 * 1024 * 1024)
    graph_limit = parse_positive(arguments[8], 1024 * 1024 * 1024)
    diagnostic_limit = parse_positive(arguments[9], 64 * 1024 * 1024)

    runtime_entries = copy_tree(
        "/runtime-input/site-packages", "/runtime/site-packages", True
    )
    runtime_entries = [
        ("site-packages/" + relative, length, digest)
        for relative, length, digest in runtime_entries
    ]
    report_length, report_digest = copy_regular(
        "/runtime-input/install-report.json", pathlib.Path("/runtime/install-report.json")
    )
    if report_digest != install_digest:
        fail(71)
    runtime_entries.append(("install-report.json", report_length, report_digest))
    runtime_entries.sort(key=lambda item: item[0].encode("utf-8"))
    require_shape(
        runtime_entries, runtime_count, runtime_bytes, runtime_digest, runtime_manifest, 72
    )

    source_entries = copy_tree("/source-input", "/source", False)
    pathlib.Path("/source/.git").mkdir(mode=0o700)
    require_shape(
        source_entries, source_count, source_bytes, source_digest, snapshot_manifest, 73
    )

    apply_write_landlock()
    verify_truncate_denial()
    version_stdout, version_stderr = graphify(["--version"], diagnostic_limit)
    help_stdout, help_stderr = graphify(["--help"], diagnostic_limit)
    extract_stdout, extract_stderr = graphify(
        [
            "extract",
            "/source",
            "--code-only",
            "--no-cluster",
            "--max-workers",
            "1",
            "--out",
            "/output",
        ],
        diagnostic_limit,
    )
    graph = read_graph(graph_limit)
    emit(
        (
            version_stdout,
            version_stderr,
            help_stdout,
            help_stderr,
            extract_stdout,
            extract_stderr,
            graph,
        )
    )


if __name__ == "__main__":
    main(sys.argv[1:])
