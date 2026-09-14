import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("platform_test", Path(__file__).with_name("lattice-wsl-platform.py"))
P = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(P)


@unittest.skipUnless(os.name == "nt", "Windows WSL platform")
class PlatformTests(unittest.TestCase):
    def test_interrupted_projection_recovers_only_the_sealed_owned_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wsl = root / "wsl.exe"; wsl.write_bytes(b"fixture-launcher")
            name = "LATTICE-Graphify-" + "a" * 32
            data = {"schema": P.SCHEMA, "root": str(root), "image_sha256": P.IMAGE_SHA,
                    "distribution": name, "install_root": str(root / "distribution"),
                    "linux_user": "lattice", "wsl": str(wsl), "launcher_sha256": P.M.file_digest(wsl),
                    "registration_id": None}
            P.save(root, data)
            registration = {"id": "fixture-registration", "version": 2, "base": str(root / "distribution")}
            completed = {**data, "registration_id": registration["id"]}
            real_write = P.M.CONFIG.atomic_write
            def interrupt(path, body):
                if path.name == "platform.json": raise OSError("simulated interruption")
                real_write(path, body)
            with patch.object(P.M.CONFIG, "atomic_write", side_effect=interrupt):
                with self.assertRaises(OSError): P.save(root, completed)
            with patch.object(P, "registrations", return_value={name: registration}):
                with self.assertRaises(P.M.Rejected): P.owned(root)
                self.assertEqual(P.owned(root, recover_public=True)[0], completed)
                self.assertEqual(P.owned(root)[0], completed)
                (root / "platform.json").write_bytes(b"unrecognized edit")
                with self.assertRaises(P.M.Rejected): P.owned(root, recover_public=True)
                self.assertEqual((root / "platform.json").read_bytes(), b"unrecognized edit")

    def test_archive_lock_prevents_write_and_replace_until_import_returns(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "image.wsl"
            path.write_bytes(b"fixture-image")
            with P.pinned_archive(path, P.M.file_digest(path)):
                self.assertEqual(path.read_bytes(), b"fixture-image")
                with self.assertRaises(OSError):
                    path.write_bytes(b"changed")
                with self.assertRaises(OSError):
                    path.rename(path.with_suffix(".moved"))
            path.write_bytes(b"unlocked")

    def test_wrong_archive_never_imports_or_creates_root(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "image.wsl"
            path.write_bytes(b"wrong-image")
            with patch.object(P, "provision_locked", side_effect=AssertionError("No import")):
                with self.assertRaisesRegex(P.M.Rejected, "WSL_IMAGE_DIGEST_REJECTED"):
                    P.provision(Path(directory) / "new", path, Path("wsl.exe"), "0" * 64)
            self.assertFalse((Path(directory) / "new").exists())

    def test_service_account_rejects_privilege_or_password_changes(self):
        good = {"passwd": b"root:x:0:0:root:/root:/bin/bash\nlattice:x:1000:1000::/home/lattice:/bin/bash\n",
                "group": b"root:x:0:\nlattice:x:1000:\nsudo:x:27:\n",
                "shadow": b"root:!:20000:0:99999:7:::\nlattice:!:20000:0:99999:7:::\n"}
        def identity(files):
            class File:
                def __init__(self, path): self.path = path
                def read_bytes(self): return files[self.path.name]
            with patch.object(P.M, "regular", side_effect=File):
                return P.user_identity("LATTICE-Graphify-" + "a" * 32)
        self.assertEqual(set(identity(good)), {"passwd", "group", "shadow"})
        for name, source, replacement in (("passwd", b":1000:1000:", b":0:0:"),
                                           ("group", b"sudo:x:27:", b"sudo:x:27:lattice"),
                                           ("shadow", b"lattice:!:", b"lattice:$6$unlocked:")):
            with self.subTest(name=name), self.assertRaisesRegex(P.M.Rejected, "WSL_SERVICE_USER_IDENTITY_REJECTED"):
                identity({**good, name: good[name].replace(source, replacement)})

    def test_changed_platform_module_is_rejected_before_python_import(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / "bin/lattice-wsl-platform.py"
            script.parent.mkdir()
            script.write_text("raise AssertionError('must never execute')", encoding="utf-8")
            config = {"root": str(root), "graphify_platform": str(root / "platform"), "files": {str(script): "0" * 64}}
            with patch.object(P.M.importlib.util, "spec_from_file_location", side_effect=AssertionError("No import")):
                with self.assertRaisesRegex(P.M.Rejected, "CUSTOMER_COMPONENT_CHANGED"):
                    P.M.platform_for_config(config)


if __name__ == "__main__":
    unittest.main()
