"""Load the reviewed local SDK source tree; not a hostile-code import sandbox."""

import hashlib
import importlib
from pathlib import Path, PurePosixPath, PureWindowsPath
import sys
import tomllib


SDK_REVISION = "010bacef009c855ccba814b51f7c8e1d38ab5e3f"


def _verified_tree(sdk_dir, manifest):
    if manifest.get("sdk_revision") != SDK_REVISION:
        raise ValueError("SDK revision is not the frozen revision")
    root = Path(sdk_dir)
    if not root.is_absolute():
        raise ValueError("SDK directory must be explicit and absolute")
    root = root.resolve(strict=True)
    files = {}
    contents = {}
    for item in manifest["sdk_files"]:
        name = item["file"]
        relative = PurePosixPath(name)
        if (not name or "\\" in name or relative.is_absolute()
                or PureWindowsPath(name).drive or ".." in relative.parts
                or relative.as_posix() != name or name in files):
            raise ValueError("Invalid or duplicate SDK manifest path")
        target = root.joinpath(*relative.parts).resolve(strict=True)
        if not target.is_relative_to(root) or not target.is_file():
            raise ValueError("SDK manifest path escapes the fixed source tree")
        data = target.read_bytes()
        if len(data) != item["size"]:
            raise ValueError(f"SDK size mismatch: {name}")
        if item.get("sha256"):
            actual = hashlib.sha256(data).hexdigest()
            expected = item["sha256"]
        else:
            actual = hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()
            expected = item["git_blob"]
        if actual != expected:
            raise ValueError(f"SDK hash mismatch: {name}")
        files[name] = target
        contents[name] = data
    if "laya/__init__.py" not in files or "pyproject.toml" not in files:
        raise ValueError("SDK manifest lacks its package entry point or pyproject")
    # -B prevents new caches but does not prevent reading existing bytecode.
    # This trusted, newly downloaded tree must contain only manifested Python sources.
    for path in (root / "laya").rglob("*"):
        if path.suffix.lower() in (".pyc", ".pyo", ".pyd"):
            raise ValueError("SDK bytecode or native extension is not permitted")
        if path.suffix.lower() == ".py":
            relative = path.relative_to(root).as_posix()
            if relative not in files or path.resolve(strict=True) != files[relative]:
                raise ValueError("SDK contains an unmanifested Python source")
    version = tomllib.loads(contents["pyproject.toml"].decode("utf8"))["project"]["version"]
    if not isinstance(version, str) or not version:
        raise ValueError("SDK project.version must be a nonempty string")
    return root, files, version


def verify_loaded_modules(sdk_dir, manifest):
    """Rehash the frozen files and check every currently loaded laya module."""
    _, files, version = _verified_tree(sdk_dir, manifest)
    if "laya" not in sys.modules:
        raise ValueError("The SDK has not been loaded")
    loaded = []
    for name, module in list(sys.modules.items()):
        if name != "laya" and not name.startswith("laya."):
            continue
        stem = name.replace(".", "/")
        allowed = {files[key] for key in (stem + ".py", stem + "/__init__.py") if key in files}
        origin = getattr(module, "__file__", None)
        if not isinstance(origin, str) or not Path(origin).is_absolute():
            raise ValueError(f"SDK module has no fixed source origin: {name}")
        if Path(origin).resolve(strict=True) not in allowed:
            raise ValueError(f"SDK module origin is not manifested: {name}")
        spec_origin = getattr(getattr(module, "__spec__", None), "origin", None)
        if not isinstance(spec_origin, str) or Path(spec_origin).resolve(strict=True) not in allowed:
            raise ValueError(f"SDK module spec origin is not manifested: {name}")
        loaded.append(name)
    if getattr(sys.modules["laya"], "__version__", None) != version:
        raise ValueError("SDK __version__ differs from verified pyproject project.version")
    return sorted(loaded)


def load_verified_source(sdk_dir, manifest):
    """Return (laya, declared_version) after byte, version, and origin checks.

    The caller must establish resource/network guards before calling this. No
    site.addsitedir, .pth processing, pip installation, or model loading occurs here.
    """
    if any(name == "laya" or name.startswith("laya.") for name in sys.modules):
        raise ValueError("Refusing a previously imported laya module")
    root, _, version = _verified_tree(sdk_dir, manifest)
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(root))
    importlib.invalidate_caches()
    module = importlib.import_module("laya")
    verify_loaded_modules(root, manifest)
    return module, version
