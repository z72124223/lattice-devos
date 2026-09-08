"""Provision one new owned WSL2 distribution from a pinned official Ubuntu image.

Never adopts a distribution, edits global WSL configuration, installs host features,
changes an existing account, or stops another distribution. A failed import is retained.
"""
from __future__ import annotations
import argparse
from contextlib import contextmanager
import ctypes
from ctypes import wintypes
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import uuid
import winreg

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("customer", Path(__file__).with_name("lattice-customer-runtime.py"))
M = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(M)
SCHEMA = "lattice.wsl-platform.v1"
IMAGE_SHA = "48d56724b5c8e60f24893e83e73bbb58c60b3ca22fba3da977075420acd54104"
SYSTEM_FILES = {
    "/usr/bin/python3.14": "b8d8288faefdd300201f43fcf00f6f539a27218eeed3a3dff5ab10b9c4c99700",
    "/usr/bin/bwrap": "0abea81db798ebf6b4742ac0664802d97521547a353c2a0dbdc21d76cbbfd2c0",
    "/usr/lib/os-release": "cf72627ff81aef345d0a9f3807eb80f1035e0f80fe2932d12d94efc02fd68104",
}


@contextmanager
def pinned_archive(path, expected):
    """Deny write/delete sharing for the exact archive throughout WSL import."""
    import msvcrt
    M.regular(path)
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    handle = kernel.CreateFileW(str(path), 0x80000000, 1, None, 3, 0x80, None)
    if handle == ctypes.c_void_p(-1).value:
        raise M.Rejected("WSL_ARCHIVE_LOCK_REJECTED")
    descriptor = msvcrt.open_osfhandle(handle, os.O_RDONLY | os.O_BINARY)
    with os.fdopen(descriptor, "rb") as stream:
        digest = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b""): digest.update(chunk)
        if digest.hexdigest() != expected or not os.path.samestat(os.fstat(stream.fileno()), path.stat()):
            raise M.Rejected("WSL_IMAGE_DIGEST_REJECTED")
        yield


def user_identity(distribution):
    def read(name):
        return M.regular(Path("\\\\wsl.localhost\\" + distribution + "\\etc\\" + name)).read_bytes()
    files = {name: read(name) for name in ("passwd", "group", "shadow")}
    rows = {name: [line.split(":") for line in body.decode("utf-8").splitlines()] for name, body in files.items()}
    users = [r for r in rows["passwd"] if r[0] == "lattice"]
    groups = [r for r in rows["group"] if r[0] == "lattice"]
    secrets = [r for r in rows["shadow"] if r[0] == "lattice"]
    if (len(users) != 1 or len(users[0]) != 7 or users[0][2:4] != ["1000", "1000"] or users[0][5] != "/home/lattice"
            or len(groups) != 1 or len(groups[0]) != 4 or groups[0][2] != "1000"
            or any(len(r) != 4 or (r[0] != "lattice" and "lattice" in r[3].split(",")) for r in rows["group"])
            or len(secrets) != 1 or len(secrets[0]) != 9 or not secrets[0][1].startswith(("!", "*"))):
        raise M.Rejected("WSL_SERVICE_USER_IDENTITY_REJECTED")
    return {name: M.CONFIG.digest(body) for name, body in files.items()}


def registrations():
    result = {}
    try:
        key = winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Lxss")
    except FileNotFoundError:
        return result
    with key:
        for index in range(winreg.QueryInfoKey(key)[0]):
            identifier = winreg.EnumKey(key, index)
            with winreg.OpenKey(key, identifier) as child:
                name = winreg.QueryValueEx(child, "DistributionName")[0]
                if name in result:
                    raise M.Rejected("WSL_DUPLICATE_REGISTRATION")
                result[name] = {"id": identifier, "base": winreg.QueryValueEx(child, "BasePath")[0], "version": winreg.QueryValueEx(child, "Version")[0]}
    return result


def validate_name(name):
    suffix = name.removeprefix("LATTICE-Graphify-")
    if name == suffix or len(suffix) != 32 or any(c not in "0123456789abcdef" for c in suffix):
        raise M.Rejected("WSL_PLATFORM_NAME_REJECTED")


def signed_launcher(wsl, expected):
    if M.file_digest(wsl) != expected or wsl.resolve() != (Path(os.environ["SystemRoot"]) / "System32/wsl.exe").resolve():
        raise M.Rejected("WSL_LAUNCHER_IDENTITY_REJECTED")
    powershell = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
    env = M.closed_environment(); env["LATTICE_PLATFORM_LAUNCHER"] = str(wsl)
    code = "$s=Get-AuthenticodeSignature -LiteralPath $env:LATTICE_PLATFORM_LAUNCHER; if ($s.Status -ne 'Valid' -or $s.SignerCertificate.Subject -notmatch '^CN=Microsoft Windows, O=Microsoft Corporation,') { exit 2 }; Write-Output 'MICROSOFT_SIGNATURE_VALID'"
    value = M.checked([str(powershell), "-NoProfile", "-NonInteractive", "-Command", code], "WSL_MICROSOFT_SIGNATURE_REJECTED", env=env)
    if value != "MICROSOFT_SIGNATURE_VALID" or M.file_digest(wsl) != expected:
        raise M.Rejected("WSL_LAUNCHER_IDENTITY_REJECTED")


def owned(root, *, recover_public=False):
    M.regular(root, directory=True)
    public = M.regular(root / "platform.json").read_bytes()
    sealed = json.loads(M.dpapi(M.regular(root / "platform.dpapi").read_bytes(), decrypt=True))
    public_digest = M.CONFIG.digest(public)
    repair = sealed.get("sha256") != public_digest
    if repair:
        if not recover_public or public_digest != sealed.get("previous_sha256") or "data" not in sealed:
            raise M.Rejected("WSL_PLATFORM_IDENTITY_REJECTED")
        data = sealed["data"]
        if M.CONFIG.digest(M.CONFIG.json_bytes(data)) != sealed.get("sha256"):
            raise M.Rejected("WSL_PLATFORM_IDENTITY_REJECTED")
    else:
        data = json.loads(public)
    if data.get("schema") != SCHEMA or Path(data["root"]) != root or data.get("image_sha256") != IMAGE_SHA:
        raise M.Rejected("WSL_PLATFORM_IDENTITY_REJECTED")
    validate_name(data["distribution"])
    if Path(data["install_root"]) != root / "distribution" or data.get("linux_user") != "lattice":
        raise M.Rejected("WSL_PLATFORM_SCOPE_REJECTED")
    if M.file_digest(Path(data["wsl"])) != data["launcher_sha256"]:
        raise M.Rejected("WSL_LAUNCHER_CHANGED")
    registration = registrations().get(data["distribution"])
    if not registration or registration["version"] != 2 or Path(registration["base"]).resolve() != (root / "distribution").resolve():
        raise M.Rejected("WSL_PLATFORM_REGISTRATION_REJECTED")
    if data.get("registration_id") and data["registration_id"] != registration["id"]:
        raise M.Rejected("WSL_PLATFORM_REGISTRATION_CHANGED")
    if repair:
        M.CONFIG.atomic_write(root / "platform.json", M.CONFIG.json_bytes(data))
    return data, registration


def verify(root):
    data, registration = owned(root)
    if not data.get("registration_id"):
        raise M.Rejected("WSL_PLATFORM_PROVISIONING_INCOMPLETE")
    if user_identity(data["distribution"]) != data.get("user_files"):
        raise M.Rejected("WSL_SERVICE_USER_CHANGED")
    for linux_path, expected in SYSTEM_FILES.items():
        path = Path("\\\\wsl.localhost\\" + data["distribution"] + linux_path.replace("/", "\\"))
        if M.file_digest(path) != expected:
            raise M.Rejected("WSL_PLATFORM_SYSTEM_FILE_CHANGED")
    return data


def save(root, data):
    public = M.CONFIG.json_bytes(data)
    previous = M.file_digest(root / "platform.json") if (root / "platform.json").exists() else None
    # One atomic DPAPI record authorizes the new projection and its exact prior
    # projection. Recovery never accepts arbitrary edits or chooses a new identity.
    sealed = {"sha256": M.CONFIG.digest(public), "data": data, "previous_sha256": previous}
    M.CONFIG.atomic_write(root / "platform.dpapi", M.dpapi(M.CONFIG.json_bytes(sealed)))
    M.CONFIG.atomic_write(root / "platform.json", public)


def provision(root, archive, wsl, expected):
    with pinned_archive(archive, IMAGE_SHA):
        return provision_locked(root, archive, wsl, expected)


def provision_locked(root, archive, wsl, expected):
    # Validate all inputs before allocating any new distribution or directory.
    M.CONFIG.regular_path(root)
    if root.exists() or not root.is_absolute() or not root.parent.is_dir():
        raise M.Rejected("FRESH_WSL_PLATFORM_ROOT_REQUIRED")
    signed_launcher(wsl, expected)
    before = registrations()
    name = "LATTICE-Graphify-" + uuid.uuid4().hex
    if name in before:
        raise M.Rejected("WSL_DISTRIBUTION_ALREADY_EXISTS")
    M.private_new_root(root)
    data = {"schema": SCHEMA, "root": str(root), "distribution": name, "install_root": str(root / "distribution"),
            "wsl": str(wsl), "launcher_sha256": expected, "image_sha256": IMAGE_SHA, "linux_user": "lattice", "registration_id": None}
    save(root, data)
    result = M.invoke([str(wsl), "--import", name, data["install_root"], str(archive), "--version", "2"], timeout=180)
    if result.returncode:
        raise M.Rejected("WSL_IMPORT_FAILED_ASSETS_RETAINED")
    data, registration = owned(root)
    # This is the freshly imported image only. No existing account is adopted.
    for selector in ("lattice", "1000"):
        result = M.invoke([str(wsl), "-d", name, "--user", "root", "--exec", "/usr/bin/getent", "passwd", selector], timeout=60)
        if result.returncode != 2:
            raise M.Rejected("WSL_SERVICE_ID_ALREADY_EXISTS")
    M.checked([str(wsl), "-d", name, "--user", "root", "--exec", "/usr/sbin/useradd", "--create-home", "--uid", "1000", "--user-group", "--shell", "/bin/bash", "--password", "!", "lattice"], "WSL_SERVICE_USER_CREATION_REJECTED", timeout=60)
    result = M.invoke([str(wsl), "-d", name, "--user", "lattice", "--exec", "/usr/bin/id", "-u"], timeout=60)
    if result.returncode or result.stdout.strip() != "1000":
        raise M.Rejected("WSL_UNPRIVILEGED_USER_REJECTED")
    after = registrations()
    if any(after.get(key) != value for key, value in before.items()):
        raise M.Rejected("WSL_OTHER_REGISTRATION_CHANGED")
    data["registration_id"] = registration["id"]
    data["user_files"] = user_identity(name)
    save(root, data)
    verify(root)
    return {"status": "WSL_PLATFORM_IDENTITY_VERIFIED", "root": str(root), "distribution": name,
            "image_sha256": IMAGE_SHA, "launcher_sha256": expected, "other_registrations": "PRESERVED", "graphify_workflow": "NOT_VERIFIED"}


def recover(root):
    # Only finalize the existing owned registration; never imports or adopts another.
    data, registration = owned(root, recover_public=True)
    identities = user_identity(data["distribution"])
    if data.get("user_files") and data["user_files"] != identities:
        raise M.Rejected("WSL_SERVICE_USER_CHANGED")
    for path, expected in SYSTEM_FILES.items():
        source = Path("\\\\wsl.localhost\\" + data["distribution"] + path.replace("/", "\\"))
        if M.file_digest(source) != expected: raise M.Rejected("WSL_PLATFORM_SYSTEM_FILE_CHANGED")
    data["registration_id"] = registration["id"]; data["user_files"] = identities
    save(root, data); verify(root)
    return {"status": "WSL_PLATFORM_IDENTITY_VERIFIED", "distribution": data["distribution"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("provision", "verify", "recover")); parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--archive", type=Path); parser.add_argument("--wsl", type=Path); parser.add_argument("--wsl-sha256")
    args = parser.parse_args()
    if args.action == "provision":
        if not all((args.archive, args.wsl, args.wsl_sha256)): raise M.Rejected("WSL_PROVISION_ARGUMENTS_REQUIRED")
        result = provision(args.root, args.archive, args.wsl, args.wsl_sha256)
    elif args.action == "recover":
        result = recover(args.root)
    else:
        data = verify(args.root); result = {"status": "WSL_PLATFORM_IDENTITY_VERIFIED", "distribution": data["distribution"]}
    print(json.dumps(result))


if __name__ == "__main__":
    try: main()
    except (M.Rejected, OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({"status": "BLOCKED", "code": str(error) if isinstance(error, M.Rejected) else "WSL_PLATFORM_IO_REJECTED"})); sys.exit(2)
