#!/usr/bin/env python3
"""Explicit Windows customer Runtime setup. No author config, scheduler, or downloads.

The installation owns one new directory and one loopback PostgreSQL cluster.
Credentials and installation identity are protected with the current user's DPAPI.
Normal STDIO startup verifies the prepared cluster; it never migrates a database.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager, nullcontext
import ctypes
from ctypes import wintypes
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import stat
import subprocess
import sys
import unicodedata
import uuid

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("lattice_config", Path(__file__).with_name("lattice-mcp-config.py"))
CONFIG = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONFIG)
Rejected = CONFIG.Rejected
SCHEMA = "lattice.customer-runtime.v1"
TOOLS = ("initdb", "pg_ctl", "postgres", "psql", "pg_controldata")
SCRIPTS = ("lattice-customer-runtime.py", "lattice-mcp-config.py", "lattice-runtime-update.py")
BASE_ENV = ("SystemRoot", "WINDIR", "COMSPEC", "PATH", "PATHEXT", "TEMP", "TMP", "USERPROFILE", "LOCALAPPDATA")


@contextmanager
def runtime_lease(root: Path, *, exclusive: bool = False):
    """Shared MCP connections; only version/schema switches need exclusive access."""
    import msvcrt
    class Overlapped(ctypes.Structure):
        _fields_ = [("internal", ctypes.c_size_t), ("internal_high", ctypes.c_size_t),
                    ("offset", wintypes.DWORD), ("offset_high", wintypes.DWORD), ("event", wintypes.HANDLE)]
    path = root / ".runtime-lease"
    CONFIG.regular_path(path)
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.LockFileEx.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(Overlapped)]
    kernel.LockFileEx.restype = wintypes.BOOL
    kernel.UnlockFileEx.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(Overlapped)]
    kernel.UnlockFileEx.restype = wintypes.BOOL
    with path.open("a+b") as stream:
        handle = msvcrt.get_osfhandle(stream.fileno())
        overlap = Overlapped()
        if not kernel.LockFileEx(handle, 3 if exclusive else 1, 0, 1, 0, ctypes.byref(overlap)):
            raise Rejected("CUSTOMER_RUNTIME_IN_USE")
        try:
            yield
        finally:
            kernel.UnlockFileEx(handle, 0, 1, 0, ctypes.byref(overlap))


def closed_environment() -> dict[str, str]:
    # Never inherit LATTICE_*, PG*, provider credentials, PYTHONPATH or CODEX_HOME.
    result = {k: os.environ[k] for k in BASE_ENV if k in os.environ}
    result.update(LANG="C", LC_ALL="C", PYTHONDONTWRITEBYTECODE="1", PYTHONSAFEPATH="1")
    return result


def regular(path: Path, *, directory: bool = False) -> Path:
    if not path.is_absolute():
        raise Rejected("ABSOLUTE_PATH_REQUIRED")
    CONFIG.regular_path(path)
    if not (path.is_dir() if directory else path.is_file()):
        raise Rejected("REQUIRED_PATH_MISSING")
    return path


def file_digest(path: Path) -> str:
    regular(path)
    return CONFIG.digest(path.read_bytes())


def dpapi(data: bytes, *, decrypt: bool = False) -> bytes:
    if os.name != "nt":
        raise Rejected("CUSTOMER_RUNTIME_WINDOWS_REQUIRED")

    class Blob(ctypes.Structure):
        _fields_ = [("length", wintypes.DWORD), ("data", ctypes.POINTER(ctypes.c_ubyte))]

    crypt = ctypes.WinDLL("crypt32", use_last_error=True)
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    fn = crypt.CryptUnprotectData if decrypt else crypt.CryptProtectData
    fn.argtypes = [ctypes.POINTER(Blob), ctypes.c_void_p, ctypes.c_void_p,
                   ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(Blob)]
    fn.restype = wintypes.BOOL
    kernel.LocalFree.argtypes = [ctypes.c_void_p]
    kernel.LocalFree.restype = ctypes.c_void_p
    buf = (ctypes.c_ubyte * len(data)).from_buffer_copy(data)
    source, output = Blob(len(data), buf), Blob()
    if not fn(ctypes.byref(source), None, None, None, None, 1, ctypes.byref(output)):
        raise Rejected("CUSTOMER_CREDENTIAL_PROTECTION_REJECTED")
    try:
        return ctypes.string_at(output.data, output.length)
    finally:
        kernel.LocalFree(output.data)


def invoke(command: list[str], *, env: dict | None = None, input_text: str | None = None,
           quiet: bool = False, timeout: int = 45) -> subprocess.CompletedProcess:
    return subprocess.run(command, input=input_text, env=env or closed_environment(),
                          stdout=subprocess.DEVNULL if quiet else subprocess.PIPE,
                          stderr=subprocess.DEVNULL if quiet else subprocess.PIPE,
                          text=True, encoding="utf-8", errors="replace", timeout=timeout,
                          creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)


def checked(command: list[str], code: str, **kwargs) -> str:
    result = invoke(command, **kwargs)
    if result.returncode:
        # Subprocess output can contain connection details or secrets. Keep it private.
        raise Rejected(code)
    return (result.stdout or "").strip()


def private_new_root(root: Path) -> None:
    if os.name != "nt":
        raise Rejected("CUSTOMER_RUNTIME_WINDOWS_REQUIRED")
    CONFIG.regular_path(root)
    if not root.is_absolute() or not root.parent.is_dir() or root.exists():
        raise Rejected("FRESH_CUSTOMER_DIRECTORY_REQUIRED")
    root.mkdir()  # Never adopt an existing directory or change customer ACLs.
    system = Path(os.environ["SystemRoot"]) / "System32"
    user = checked([str(system / "whoami.exe")], "CUSTOMER_IDENTITY_UNAVAILABLE")
    checked([str(system / "icacls.exe"), str(root), "/inheritance:r", "/grant:r",
             user + ":(OI)(CI)F", "SYSTEM:(OI)(CI)F"], "PRIVATE_DIRECTORY_REJECTED")


def free_port() -> int:
    while True:
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        if port > 1024 and port not in (5432, 58743):
            return port


def pg(config: dict, tool: str) -> str:
    return str(Path(config["postgres_bin"]) / (tool + ".exe"))


def environment(config: dict, password: str) -> dict:
    root = Path(config["root"])
    identity = {"schema": SCHEMA, "run_id": config["run_id"], "system_id": config["system_id"]}
    observation = CONFIG.digest(CONFIG.json_bytes(identity))
    authority = CONFIG.digest(CONFIG.json_bytes({**identity, "epoch": 1, "revision": 1, "observation": observation}))
    env = closed_environment()
    env.update({
        "LATTICE_TASK019_HOST": "127.0.0.1", "LATTICE_TASK019_PORT": str(config["port"]),
        "LATTICE_TASK019_RUN_ID": config["run_id"], "LATTICE_TASK019_PASSWORD": password,
        "LATTICE_DELIVERY_CODEX_MODE": "OFFICIAL_CODEX_APP_SERVER",
        "LATTICE_FULL_CHAIN_RUN_MODE": "RESUME_EXISTING",
        "LATTICE_RUNTIME_INTEGRATION": "GRAPHIFY", "LATTICE_HERMES_MODE": "TASK_ONLY",
        "LATTICE_MANAGED_FOREMAN_MODE": "DISABLED",
        "LATTICE_CUSTOMER_CATALOG_PATH": str(root / "projects.json"),
        "LATTICE_DELIVERY_GIT_EXE": config["git"],
        "LATTICE_DEPENDENCY_WORKTREE_ROOT": str(root / "dependencies"),
        "LATTICE_GRAPHIFY_WORK_ROOT": str(root / "graph-work"),
        "LATTICE_STORE_DAEMON_INSTANCE_ID": "customer-" + config["run_id"],
        "LATTICE_STORE_DAEMON_EPOCH": "1", "LATTICE_STORE_AUTHORITY_REVISION": "1",
        "LATTICE_STORE_OBSERVATION_DIGEST": observation,
        "LATTICE_STORE_AUTHORITY_HEAD_DIGEST": authority,
        "LATTICE_TASK_INGRESS_KIND": "CODEX_LOCAL_MCP",
        "LATTICE_TASK_INGRESS_PROFILE_SHA256": CONFIG.digest(CONFIG.json_bytes({"kind": "CODEX_LOCAL_MCP", **identity})),
    })
    for key, variable in (("graphify_runtime", "LATTICE_GRAPHIFY_RUNTIME_ROOT"),
                          ("graph_source", "LATTICE_GRAPHIFY_SOURCE_ROOT"), ("wsl", "LATTICE_GRAPHIFY_WSL_EXE")):
        if config.get(key):
            env[variable] = config[key]
    return env


def verify_dependency_file_set(config: dict) -> set[Path]:
    if config.get("dependency_root"):
        dependency_root = regular(Path(config["dependency_root"]), directory=True)
        expected = {Path(path): digest for path, digest in config["files"].items() if Path(path).is_relative_to(dependency_root)}
        actual = set()
        for directory, dirs, files in os.walk(dependency_root, followlinks=False):
            for name in dirs:
                regular(Path(directory) / name, directory=True)
            for name in files:
                path = Path(directory) / name
                info = path.lstat()
                if not stat.S_ISREG(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
                    raise Rejected("PATH_REDIRECTION_REJECTED")
                if path not in expected:
                    raise Rejected("CUSTOMER_DEPENDENCY_FILE_SET_CHANGED")
                # Root and directory redirects were checked once above. Avoid
                # rewalking every ancestor for every immutable dependency file.
                if CONFIG.digest(path.read_bytes()) != expected[path]:
                    raise Rejected("CUSTOMER_COMPONENT_CHANGED")
                actual.add(path)
            if len(actual) > len(expected):
                raise Rejected("CUSTOMER_DEPENDENCY_FILE_SET_CHANGED")
        if actual != set(expected):
            raise Rejected("CUSTOMER_DEPENDENCY_FILE_SET_CHANGED")
        return actual
    return set()


def load(root: Path) -> tuple[dict, str]:
    regular(root, directory=True)
    if (root / "update.pending.dpapi").exists():
        raise Rejected("CUSTOMER_UPDATE_RECOVERY_REQUIRED")
    public = regular(root / "installation.json").read_bytes()
    sealed = json.loads(dpapi(regular(root / "credentials.dpapi").read_bytes(), decrypt=True))
    if sealed["installation_sha256"] != CONFIG.digest(public):
        raise Rejected("CUSTOMER_INSTALLATION_CHANGED")
    config = json.loads(public)
    if config["schema"] != SCHEMA or Path(config["root"]) != root:
        raise Rejected("CUSTOMER_INSTALLATION_IDENTITY_REJECTED")
    dependencies = verify_dependency_file_set(config)
    if not isinstance(config["port"], int) or not 1024 < config["port"] <= 65535 or config["port"] in (5432, 58743):
        raise Rejected("CUSTOMER_PORT_REJECTED")
    if len(config["run_id"]) != 32 or any(c not in "0123456789abcdef" for c in config["run_id"]):
        raise Rejected("CUSTOMER_INSTALLATION_IDENTITY_REJECTED")
    regular(root / "cluster", directory=True)
    for file, expected in config["files"].items():
        if Path(file) in dependencies:
            continue
        if file_digest(Path(file)) != expected:
            raise Rejected("CUSTOMER_COMPONENT_CHANGED")
    offline = checked([pg(config, "pg_controldata"), str(root / "cluster")], "CUSTOMER_CLUSTER_CONTROL_UNREADABLE")
    if config["system_id"] != control_identifier(offline):
        raise Rejected("CUSTOMER_CLUSTER_IDENTITY_REJECTED")
    return config, sealed["password"]


def control_identifier(output: str) -> str:
    values = [line.split(":", 1)[1].strip() for line in output.splitlines()
              if line.startswith("Database system identifier:")]
    if len(values) != 1 or not values[0].isdigit():
        raise Rejected("CUSTOMER_CLUSTER_CONTROL_REJECTED")
    return values[0]


def running(config: dict) -> bool:
    result = invoke([pg(config, "pg_ctl"), "-D", str(Path(config["root"]) / "cluster"), "status"], quiet=True)
    if result.returncode not in (0, 3):
        raise Rejected("CUSTOMER_CLUSTER_STATUS_REJECTED")
    return result.returncode == 0


def verify_running(config: dict, password: str) -> None:
    env = closed_environment()
    env["PGPASSWORD"] = password
    env["PGCONNECT_TIMEOUT"] = "5"
    result = checked([pg(config, "psql"), "-X", "-h", "127.0.0.1", "-p", str(config["port"]),
                      "-U", "runtime_bootstrap", "-d", "postgres", "-A", "-t", "-v", "ON_ERROR_STOP=1"],
                     "CUSTOMER_CLUSTER_CONNECTION_REJECTED", env=env,
                     input_text="SELECT json_build_object('id',system_identifier::text,'data',current_setting('data_directory'),'port',current_setting('port')::int,'listen',current_setting('listen_addresses')) FROM pg_control_system();")
    observed = json.loads(result)
    if (observed["id"] != config["system_id"] or Path(observed["data"]) != Path(config["root"]) / "cluster"
            or observed["port"] != config["port"] or observed["listen"] != "127.0.0.1"):
        raise Rejected("CUSTOMER_CLUSTER_IDENTITY_REJECTED")


def start(config: dict, password: str) -> None:
    if not running(config):
        root = Path(config["root"])
        # PostgreSQL reads auto.conf after postgresql.conf. Validate effective
        # settings before any listener starts, not merely after connecting.
        for name, expected in (("listen_addresses", "127.0.0.1"), ("port", str(config["port"])),
                               ("data_directory", str(root / "cluster"))):
            actual = checked([pg(config, "postgres"), "-D", str(root / "cluster"), "-C", name],
                             "CUSTOMER_EFFECTIVE_POSTGRES_CONFIG_REJECTED")
            matches = Path(actual) == Path(expected) if name == "data_directory" else actual == expected
            if not matches:
                raise Rejected("CUSTOMER_EFFECTIVE_POSTGRES_CONFIG_REJECTED")
        checked([pg(config, "pg_ctl"), "-D", str(root / "cluster"), "-l", str(root / "postgres.log"),
                 "-w", "-t", "30", "start"], "CUSTOMER_CLUSTER_START_REJECTED", quiet=True)
    verify_running(config, password)


def runtime_action(config: dict, password: str, action: str) -> dict | None:
    result = invoke([config["runtime"], action], env=environment(config, password), timeout=120)
    if result.returncode:
        codes = [line for line in result.stderr.splitlines()
                 if re.fullmatch(r"(?:LATTICE|GRAPHIFY)_[A-Z0-9_]{1,120}", line)]
        suffix = ":" + codes[-1] if codes else ""
        raise Rejected("CUSTOMER_RUNTIME_" + action.strip("-").replace("-", "_").upper() + "_REJECTED" + suffix)
    return json.loads(result.stdout) if result.stdout.strip() else None


def prepare(root: Path, runtime: Path, expected: str, postgres_bin: Path, git: Path,
            graphify_runtime: Path | None = None, graph_source: Path | None = None, wsl: Path | None = None,
            dependency_files: dict[str, str] | None = None, dependency_root: Path | None = None) -> dict:
    regular(runtime)
    if file_digest(runtime) != expected:
        raise Rejected("RUNTIME_DIGEST_MISMATCH")
    regular(postgres_bin, directory=True)
    regular(git)
    for tool in TOOLS:
        regular(postgres_bin / (tool + ".exe"))
    for directory in (graphify_runtime, graph_source):
        if directory:
            regular(directory, directory=True)
    if wsl:
        regular(wsl)
    if bool(graphify_runtime) != bool(wsl):
        raise Rejected("GRAPHIFY_RUNTIME_AND_WSL_REQUIRED_TOGETHER")
    for file, wanted in (dependency_files or {}).items():
        if file_digest(Path(file)) != wanted:
            raise Rejected("CUSTOMER_DEPENDENCY_CHANGED")
    private_new_root(root)
    (root / "bin").mkdir()
    target = root / "bin" / "latticed.exe"
    shutil.copyfile(runtime, target)
    if file_digest(target) != expected:
        raise Rejected("RUNTIME_COPY_CHANGED")
    for name in SCRIPTS:
        source = regular(Path(__file__).resolve().with_name(name))
        copied = root / "bin" / name
        shutil.copyfile(source, copied)
        if file_digest(source) != file_digest(copied):
            raise Rejected("CUSTOMER_LAUNCHER_COPY_CHANGED")
    password = secrets.token_hex(32)
    password_file = root / "initdb-password.tmp"
    try:
        with password_file.open("xb") as stream:
            stream.write((password + "\n").encode())
        checked([str(postgres_bin / "initdb.exe"), "-D", str(root / "cluster"), "-U", "runtime_bootstrap",
                 "--auth=scram-sha-256", "--encoding=UTF8", "--locale=C", "--data-checksums",
                 "--pwfile=" + str(password_file)], "CUSTOMER_INITDB_REJECTED")
    finally:
        password_file.unlink(missing_ok=True)
    port = free_port()
    with (root / "cluster" / "postgresql.conf").open("a", encoding="utf-8") as stream:
        stream.write("\n# This customer installation only\nlisten_addresses = '127.0.0.1'\nport = " + str(port) + "\n")
    system_id = control_identifier(checked([str(postgres_bin / "pg_controldata.exe"), str(root / "cluster")], "CUSTOMER_CLUSTER_CONTROL_UNREADABLE"))
    paths = [target, git, Path(sys.executable), *(root / "bin" / name for name in SCRIPTS),
             *(postgres_bin / (tool + ".exe") for tool in TOOLS),
             root / "cluster" / "postgresql.conf", root / "cluster" / "postgresql.auto.conf",
             root / "cluster" / "pg_hba.conf", root / "cluster" / "pg_ident.conf"]
    if wsl:
        paths.append(wsl)
    config = {"schema": SCHEMA, "root": str(root), "runtime": str(target), "postgres_bin": str(postgres_bin),
              "git": str(git), "python": str(Path(sys.executable)), "port": port, "run_id": secrets.token_hex(16), "system_id": system_id,
              "files": {**(dependency_files or {}), **{str(file): file_digest(file) for file in paths}},
              "dependency_root": str(dependency_root) if dependency_root else None,
              "graphify_runtime": str(graphify_runtime) if graphify_runtime else None,
              "graph_source": str(graph_source) if graph_source else None, "wsl": str(wsl) if wsl else None}
    data = CONFIG.json_bytes(config)
    (root / "installation.json").write_bytes(data)
    (root / "credentials.dpapi").write_bytes(dpapi(CONFIG.json_bytes({"password": password, "installation_sha256": CONFIG.digest(data)})))
    (root / "projects.json").write_bytes(CONFIG.json_bytes({"schema": "lattice.customer-project-catalog.v1", "projects": []}))
    for name in ("graph-work", "dependencies"):
        (root / name).mkdir()
    # From this point, any interrupted initialization remains bound to this cluster.
    return operate(root, "recover")


def register_project(root: Path, project_root: Path, name: str) -> dict:
    """Retain a locator; native Runtime alone establishes PostgreSQL Registry identity."""
    regular(project_root, directory=True)
    project_root = project_root.resolve()
    if (not name or name.strip() != name or name != unicodedata.normalize("NFC", name)
            or len(name) > 64 or any(unicodedata.category(c) == "Cc" for c in name)):
        raise Rejected("CUSTOMER_PROJECT_NAME_REJECTED")
    load(root)
    with CONFIG.manager_lock(root / ".operations"):
        config, _ = load(root)
        repo = checked([config["git"], "-C", str(project_root), "rev-parse", "--show-toplevel"], "CUSTOMER_GIT_ROOT_REJECTED")
        if Path(repo) != project_root:
            raise Rejected("CUSTOMER_GIT_ROOT_REJECTED")
        checked([config["git"], "-C", str(project_root), "symbolic-ref", "--quiet", "HEAD"], "CUSTOMER_ATTACHED_BRANCH_REQUIRED")
        checked([config["git"], "-C", str(project_root), "rev-parse", "--verify", "HEAD^{commit}"], "CUSTOMER_INITIAL_COMMIT_REQUIRED")
        path = regular(root / "projects.json")
        if path.stat().st_size > 2 * 1024 * 1024:
            raise Rejected("CUSTOMER_CATALOG_BOUND_EXCEEDED")
        catalog = json.loads(path.read_bytes())
        if catalog["schema"] != "lattice.customer-project-catalog.v1" or not isinstance(catalog["projects"], list):
            raise Rejected("CUSTOMER_CATALOG_REJECTED")
        for project in catalog["projects"]:
            if Path(project["canonical_path"]) == project_root:
                if project["name"] != name:
                    raise Rejected("CUSTOMER_PROJECT_LOCATOR_CONFLICT")
                return {"status": "LOCATOR_REPLAYED", "project_id": project["id"], "registry_authority": "PENDING_NATIVE_OBSERVATION"}
        if len(catalog["projects"]) >= 4096:
            raise Rejected("CUSTOMER_CATALOG_BOUND_EXCEEDED")
        project_id = str(uuid.uuid4())
        catalog["projects"].append({"schema_version": catalog["schema"], "record_kind": "CUSTOMER_LOCAL_LOCATOR",
            "id": project_id, "control_project_id": project_id, "name": name, "canonical_path": str(project_root),
            "registry_authority": "NONE", "registry_project_id": None})
        data = CONFIG.json_bytes(catalog)
        if len(data) > 2 * 1024 * 1024:
            raise Rejected("CUSTOMER_CATALOG_BOUND_EXCEEDED")
        CONFIG.atomic_write(path, data)
        if path.read_bytes() != data:
            raise Rejected("CUSTOMER_CATALOG_READBACK_REJECTED")
        return {"status": "LOCATOR_SAVED", "project_id": project_id, "registry_authority": "PENDING_NATIVE_OBSERVATION"}


def import_result(root: Path, request: Path, node: Path, expected: str) -> dict:
    """Operator action: the existing native importer verifies and retains evidence."""
    load(root)
    regular(request)
    if file_digest(node) != expected:
        raise Rejected("CUSTOMER_NODE_DIGEST_MISMATCH")
    with CONFIG.manager_lock(root / ".operations"):
        config, password = load(root)
        verify_running(config, password)
        env = environment(config, password)
        env["LATTICE_LOCAL_RESULT_NODE_EXE"] = str(node)
        result = invoke([config["runtime"], "--local-result-import", str(request)], env=env, timeout=180)
        if result.returncode:
            codes = [line for line in result.stderr.splitlines()
                     if re.fullmatch(r"[A-Z][A-Z0-9_]{1,120}", line)]
            raise Rejected("CUSTOMER_RESULT_IMPORT_REJECTED" + (":" + codes[-1] if codes else ""))
        return json.loads(result.stdout)


def operate(root: Path, action: str, config_path: Path | None = None) -> dict | int:
    config, password = load(root)
    if action == "serve":
        if not (root / "ready.json").is_file():
            raise Rejected("CUSTOMER_INITIALIZATION_INCOMPLETE")
        if not running(config):
            raise Rejected("CUSTOMER_CLUSTER_STOPPED_USE_START")
        verify_running(config, password)
        # STDIO belongs entirely to the native MCP server. No secret or wrapper log.
        with runtime_lease(root):
            config, password = load(root)
            return subprocess.call([config["runtime"]], env=environment(config, password),
                                   stdin=sys.stdin, stdout=sys.stdout, stderr=sys.stderr,
                                   creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    with (runtime_lease(root, exclusive=True) if action in ("recover", "stop") else nullcontext()), CONFIG.manager_lock(root / ".operations"):
        # Revalidate after taking the cross-process operation lock.
        config, password = load(root)
        operation_evidence = None
        if action in ("start", "recover"):
            if action == "start" and not (root / "ready.json").is_file():
                raise Rejected("CUSTOMER_INITIALIZATION_INCOMPLETE_USE_RECOVER")
            start(config, password)
            if action == "recover":
                runtime_action(config, password, "--postgres-initialize")
                runtime_action(config, password, "--postgres-bootstrap")
                CONFIG.atomic_write(root / "ready.json", CONFIG.json_bytes({"schema": SCHEMA, "run_id": config["run_id"], "system_id": config["system_id"]}))
        elif action == "stop":
            if running(config):
                verify_running(config, password)
                checked([pg(config, "pg_ctl"), "-D", str(root / "cluster"), "-m", "fast", "-w", "-t", "30", "stop"], "CUSTOMER_CLUSTER_STOP_REJECTED", quiet=True)
            if running(config):
                raise Rejected("CUSTOMER_CLUSTER_STOP_NOT_CONFIRMED")
        elif action in ("graphify-preflight", "graphify-refresh"):
            if not config.get("graphify_runtime"):
                raise Rejected("GRAPHIFY_CUSTOMER_DEPENDENCIES_NOT_CONFIGURED")
            if action == "graphify-refresh":
                verify_running(config, password)
                if not config.get("graph_source"):
                    raise Rejected("GRAPHIFY_CUSTOMER_SOURCE_NOT_CONFIGURED")
            operation_evidence = runtime_action(config, password, "--graphify-runtime-preflight" if action == "graphify-preflight" else "--graphify-refresh")
            if action == "graphify-preflight":
                operation_evidence = {"component": "graphify", "status": "IDENTITY_VERIFIED", "workflow": "NOT_VERIFIED"}
        elif action in ("connect", "reconnect"):
            if config_path is None:
                raise Rejected("CUSTOMER_CODEX_CONFIG_REQUIRED")
            if not (root / "ready.json").is_file():
                raise Rejected("CUSTOMER_INITIALIZATION_INCOMPLETE")
            python = Path(config["python"])
            CONFIG.change(config_path, "install" if action == "connect" else "update", python, config["files"][str(python)],
                          ["-I", "-B", "-S", config.get("launcher", str(root / "bin" / SCRIPTS[0])), "serve", "--state", str(root)])
        elif action == "status":
            if running(config):
                verify_running(config, password)
        else:
            raise Rejected("CUSTOMER_ACTION_REJECTED")
        return {"schema": SCHEMA, "status": "STOPPED" if not running(config) else "RUNNING_IDENTITY_VERIFIED",
                "action": action, "state": str(root), "run_id": config["run_id"],
                "initialized": (root / "ready.json").is_file(), "host": "127.0.0.1", "port": config["port"],
                "graphify": "CONFIGURED_NOT_VERIFIED" if config.get("graphify_runtime") else "NOT_CONFIGURED",
                "workflow": "NOT_VERIFIED", "customer_settings": "PRESERVED", "operation_evidence": operation_evidence}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "recover", "start", "stop", "status", "serve", "connect", "reconnect", "update-runtime", "recover-update", "rollback-runtime", "register-project", "import-result", "graphify-preflight", "graphify-refresh"])
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--sha256")
    parser.add_argument("--postgres-bin", type=Path)
    parser.add_argument("--git", type=Path)
    parser.add_argument("--graphify-runtime", type=Path)
    parser.add_argument("--graph-source", type=Path)
    parser.add_argument("--wsl", type=Path)
    parser.add_argument("--codex-config", type=Path)
    parser.add_argument("--project-root", type=Path)
    parser.add_argument("--project-name")
    parser.add_argument("--evidence-request", type=Path)
    parser.add_argument("--node", type=Path)
    parser.add_argument("--node-sha256")
    parser.add_argument("--update-id")
    args = parser.parse_args()
    try:
        if args.action == "prepare":
            if not all((args.runtime, args.sha256, args.postgres_bin, args.git)):
                raise Rejected("CUSTOMER_COMPONENT_ARGUMENTS_REQUIRED")
            result = prepare(args.state, args.runtime, args.sha256, args.postgres_bin, args.git,
                             args.graphify_runtime, args.graph_source, args.wsl)
        elif args.action in ("update-runtime", "recover-update", "rollback-runtime"):
            if args.action == "rollback-runtime" and not args.update_id:
                raise Rejected("UPDATE_ID_REQUIRED")
            spec = importlib.util.spec_from_file_location("updater", Path(__file__).with_name("lattice-runtime-update.py"))
            updater = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(updater)
            result = updater.apply(args.state, args.runtime, args.sha256,
                                   recover=args.action == "recover-update",
                                   rollback=args.update_id if args.action == "rollback-runtime" else None)
        elif args.action == "register-project":
            if not args.project_root or not args.project_name:
                raise Rejected("CUSTOMER_PROJECT_ARGUMENTS_REQUIRED")
            result = register_project(args.state, args.project_root, args.project_name)
        elif args.action == "import-result":
            if not all((args.evidence_request, args.node, args.node_sha256)):
                raise Rejected("CUSTOMER_RESULT_ARGUMENTS_REQUIRED")
            result = import_result(args.state, args.evidence_request, args.node, args.node_sha256)
        else:
            result = operate(args.state, args.action, args.codex_config)
        if isinstance(result, int):
            return result
        print(json.dumps(result))
        return 0
    except Rejected as error:
        code = str(error)
    except PermissionError:
        code = "PERMISSION_DENIED_PRESERVED"
    except subprocess.TimeoutExpired:
        code = "CUSTOMER_OPERATION_TIMED_OUT_RETAINED_FOR_REVIEW"
    except (OSError, ValueError, KeyError, TypeError):
        code = "CUSTOMER_STATE_OR_DEPENDENCY_REJECTED"
    print(json.dumps({"schema": SCHEMA, "status": "BLOCKED", "code": code}),
          file=sys.stderr if args.action == "serve" else sys.stdout)
    return 2


if __name__ == "__main__":
    sys.exit(main())
