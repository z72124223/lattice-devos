import importlib.util
from contextlib import contextmanager
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("backup_test", Path(__file__).with_name("lattice-customer-backup.py"))
M = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(M)


class BackupTests(unittest.TestCase):
    def test_finalize_restored_catalog_bootstraps_before_probe_and_ready(self):
        for failure in (None, "bootstrap", "probe", "journal"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                state = Path(directory).resolve()
                (state / "installation.json").write_bytes(b"sealed fixture installation")
                (state / "projects.json").write_bytes(b'{"projects":[]}')
                (state / "restore.in-progress").write_bytes(b"pending restore")
                config = {"root": str(state), "run_id": "a" * 32, "system_id": "fixture-system"}
                journal = {"schema": "lattice.customer-restore.v1", "root": str(state),
                           "installation_sha256": M.M.file_digest(state / "installation.json"),
                           "catalog_sha256": M.M.file_digest(state / "projects.json"),
                           "proofs": {}, "requests": {}, "results": {}, "backup_sha256": "b" * 64}
                if failure == "journal": journal["installation_sha256"] = "0" * 64
                pending = state / "restore.pending.dpapi"
                pending.write_bytes(M.M.CONFIG.json_bytes(journal))
                events = []; active = set()

                @contextmanager
                def lock(name):
                    events.append(name + "-enter"); active.add(name)
                    try: yield
                    finally:
                        active.remove(name); events.append(name + "-exit")

                def step(name, *arguments):
                    self.assertEqual(active, {"lease", "manager"})
                    self.assertFalse((state / "ready.json").exists())
                    self.assertFalse((state / "restore-receipt.json").exists())
                    self.assertEqual(arguments[:2], (config, "fixture-secret"))
                    if name == "bootstrap": self.assertEqual(arguments[2:], ("--postgres-bootstrap",))
                    events.append(name)
                    if failure == name: raise M.M.Rejected("FIXTURE_" + name.upper() + "_FAILED")

                fake_update = SimpleNamespace(probe=lambda *args: step("probe", *args))
                fake_spec = SimpleNamespace(loader=SimpleNamespace(exec_module=lambda module: None))
                with (patch.object(M.M, "load", return_value=(config, "fixture-secret")) as load,
                      patch.object(M.M, "dpapi", side_effect=lambda data, **kwargs: data),
                      patch.object(M.M, "runtime_lease", side_effect=lambda root, **kwargs: lock("lease")) as lease,
                      patch.object(M.M.CONFIG, "manager_lock", side_effect=lambda path: lock("manager")),
                      patch.object(M.M, "start", side_effect=lambda *args: step("start", *args)),
                      patch.object(M.M, "runtime_action", side_effect=lambda *args: step("bootstrap", *args)),
                      patch.object(M.importlib.util, "spec_from_file_location", return_value=fake_spec),
                      patch.object(M.importlib.util, "module_from_spec", return_value=fake_update)):
                    if failure:
                        code = "RESTORE_JOURNAL_IDENTITY_REJECTED" if failure == "journal" else "FIXTURE_" + failure.upper() + "_FAILED"
                        with self.assertRaisesRegex(M.M.Rejected, code): M.finalize_restore(state)
                    else:
                        result = M.finalize_restore(state)
                        self.assertEqual(result["status"], "CUSTOMER_RESTORE_IDENTITY_VERIFIED")
                        self.assertEqual(result["workflow"], "READBACK_REQUIRED")
                    lease.assert_called_once_with(state, exclusive=True)
                    self.assertEqual(load.call_count, 2)
                    load.assert_called_with(state, allow_restore=True)
                steps = {None: ["start", "bootstrap", "probe"], "bootstrap": ["start", "bootstrap"],
                         "probe": ["start", "bootstrap", "probe"], "journal": []}[failure]
                self.assertEqual(events, ["lease-enter", "manager-enter", *steps, "manager-exit", "lease-exit"])
                if failure:
                    self.assertTrue(pending.exists()); self.assertTrue((state / "restore.in-progress").exists())
                    self.assertFalse((state / "ready.json").exists())
                    self.assertFalse((state / "restore-receipt.json").exists())
                else:
                    self.assertFalse(pending.exists()); self.assertFalse((state / "restore.in-progress").exists())
                    self.assertTrue((state / "ready.json").is_file())

    def test_restore_copies_verified_crt_and_seals_destination_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); bundle = root / 'bundle'; state = root / 'restored'
            platform = root / 'platform'; old_root = root / 'original'
            crt = {'vcruntime140.dll': b'fixture vc runtime', 'msvcp140.dll': b'fixture cpp runtime'}
            fixtures = {'bundle/bin/latticed.exe': b'runtime', 'bundle/python/python.exe': b'python',
                        'bundle/postgres/bin/postgres.exe': b'postgres', 'platform/platform.json': b'{}',
                        'windows/System32/wsl.exe': b'wsl', **{'bundle/bin/' + name: data for name, data in crt.items()}}
            for name, data in fixtures.items():
                path = root / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(data)
            manifest = {'runtime_sha256': M.M.file_digest(bundle / 'bin/latticed.exe'),
                        'files': {p.relative_to(bundle).as_posix(): {'sha256': M.M.file_digest(p)}
                                  for p in bundle.rglob('*') if p.is_file()}}
            configs = ('postgresql.conf', 'postgresql.auto.conf', 'pg_hba.conf', 'pg_ident.conf')
            old = {'root': str(old_root), 'postgres_bin': str(old_root / 'pg'), 'run_id': 'a' * 32,
                   'system_id': 'fixture-system', 'files': {
                       str(old_root / 'pg/postgres.exe'): manifest['files']['postgres/bin/postgres.exe']['sha256'],
                       **{str(old_root / 'cluster' / name): M.M.CONFIG.digest(b'config') for name in configs}}}
            metadata = {'source': old, 'password': 'fixture-secret', 'catalog': {'projects': []}, 'registry': {}}
            def decrypt_fixture(*args):
                (state / 'cluster').mkdir(parents=True)
                for name in configs: (state / 'cluster' / name).write_bytes(b'config')
                return metadata
            with (patch.object(M.B, 'verify', return_value=manifest),
                  patch.object(M.sys, 'executable', str(bundle / 'python/python.exe')),
                  patch.dict(M.os.environ, {'SystemRoot': str(root / 'windows')}),
                  patch.object(M, 'decrypt_backup', side_effect=decrypt_fixture),
                  patch.object(M.M, 'platform_for_config'), patch.object(M.M, 'checked'),
                  patch.object(M.M, 'control_identifier', return_value='fixture-system'),
                  patch.object(M.M, 'free_port', return_value=55432), patch.object(M.M, 'SCRIPTS', ()),
                  patch.object(M.M, 'VC_RUNTIME_FILES', {name: M.M.CONFIG.digest(data) for name, data in crt.items()}),
                  patch.object(M.M, 'dpapi', side_effect=lambda data, **kwargs: data),
                  patch.object(M, 'finalize_restore', return_value={'status': 'fixture'}) as finalize):
                M.restore(root / 'archive', 'archive-sha', root / 'keys/key', state, bundle, 'bundle-sha', platform)
            finalize.assert_called_once_with(state)
            public = (state / 'installation.json').read_bytes(); config = json.loads(public)
            sealed = json.loads((state / 'credentials.dpapi').read_bytes())
            self.assertEqual(sealed['installation_sha256'], M.M.CONFIG.digest(public))
            for name, data in crt.items():
                target = state / 'bin' / name
                self.assertEqual(target.read_bytes(), data)
                self.assertEqual(config['files'][str(target)], M.M.CONFIG.digest(data))

    def test_snapshot_head_must_match_accepted_project_branch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / '.git').mkdir()
            head = root / '.git/HEAD'; head.write_bytes(b'ref: refs/heads/main\n')
            roots = {'projects/p': root}; snapshots = {'projects/p': M.inventory(root)}
            observations = {'p': {'accepted_ref': 'refs/heads/main'}}
            M.verify_snapshot_heads(roots, snapshots, observations)
            head.write_bytes(b'ref: refs/heads/changed\n')
            with self.assertRaisesRegex(M.M.Rejected, 'BACKUP_PROJECT_HEAD_CHANGED'):
                M.verify_snapshot_heads(roots, snapshots, observations)
            snapshots['projects/p'] = M.inventory(root)
            with self.assertRaisesRegex(M.M.Rejected, 'BACKUP_PROJECT_HEAD_CHANGED'):
                M.verify_snapshot_heads(roots, snapshots, observations)

    def test_backup_metadata_obeys_restore_bound_before_encryption(self):
        with patch.object(M, 'MAX_METADATA', 1024):
            self.assertLess(len(M.bounded_json({'entries': ['small']})), 1024)
            with self.assertRaisesRegex(M.M.Rejected, 'BACKUP_METADATA_BOUND_EXCEEDED'):
                M.bounded_json({'entries': ['path-and-digest-' * 100]})

    def test_interrupted_decryption_gate_blocks_even_complete_credential_pair(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); archive = root / 'archive'; archive.mkdir()
            (archive / 'metadata.gcm').write_bytes(b'fixture')
            public = {'schema': M.SCHEMA, 'encryption': 'AES-256-GCM', 'metadata_sha256': M.M.file_digest(archive / 'metadata.gcm')}
            (archive / 'backup.json').write_bytes(M.M.CONFIG.json_bytes(public))
            output = root / 'restore'
            with patch.object(M, 'crypto', side_effect=M.M.Rejected('simulated interruption')):
                with self.assertRaises(M.M.Rejected): M.decrypt_backup(archive, M.M.file_digest(archive / 'backup.json'), root / 'key', output, root / 'node.exe')
            self.assertTrue((output / 'restore.in-progress').is_file())
            (output / 'installation.json').write_bytes(b'complete installation')
            (output / 'credentials.dpapi').write_bytes(b'complete credential pair')
            with self.assertRaisesRegex(M.M.Rejected, 'CUSTOMER_RESTORE_FINALIZATION_REQUIRED'): M.M.load(output)

    def test_encryption_verifies_the_bytes_actually_read_against_snapshot(self):
        node = Path('C:/Program Files/nodejs/node.exe')
        if not node.is_file(): self.skipTest('explicit Node runtime unavailable')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); key = root / 'key'; key.write_bytes(bytes(range(32)))
            source = root / 'source'; source.write_bytes(b'changed')
            target = root / 'ciphertext'
            with self.assertRaises(M.M.Rejected):
                M.crypto(node, 'encrypt', key, [{'input': str(source), 'output': str(target), 'expected_sha256': '0' * 64, 'expected_bytes': 7}])
            self.assertFalse(target.exists())

    def test_untrusted_manifest_rejected_before_creating_restore_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); archive = root / 'archive'; archive.mkdir()
            (archive / 'backup.json').write_bytes(b'{}')
            output = root / 'restore'
            with patch.object(M, 'crypto', side_effect=AssertionError('No crypto or process')):
                with self.assertRaisesRegex(M.M.Rejected, 'BACKUP_MANIFEST_DIGEST_REJECTED'):
                    M.decrypt_backup(archive, '0' * 64, root / 'key', output, root / 'node.exe')
            self.assertFalse(output.exists())

    def test_pending_restore_blocks_regular_runtime_before_loading_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'restore.pending.dpapi').write_bytes(b'pending')
            with patch.object(M.M, 'dpapi', side_effect=AssertionError('No credential read')):
                with self.assertRaisesRegex(M.M.Rejected, 'CUSTOMER_RESTORE_FINALIZATION_REQUIRED'): M.M.load(root)

    def test_archive_paths_reject_escapes_and_windows_aliases(self):
        for name in ('../x', '/x', 'C:/x', 'cluster/file:stream', 'projects/CON', 'cluster/a.', 'cluster/a\\b', 'cluster//x'):
            with self.subTest(name=name), self.assertRaises(M.M.Rejected): M.relative(name)
        self.assertEqual(str(M.relative('cluster/base/1/123')), 'cluster/base/1/123')

    def test_real_crypto_roundtrip_and_tamper_preserve_existing_data(self):
        node = Path('C:/Program Files/nodejs/node.exe')
        if not node.is_file(): self.skipTest('explicit Node runtime unavailable')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); key = root / 'key'; key.write_bytes(bytes(range(32)))
            for data in (b'', b'customer secret\x00\xff' * 10000):
                source = root / 'source'; source.write_bytes(data)
                encrypted = root / ('empty.gcm' if not data else 'data.gcm')
                output = root / ('empty.out' if not data else 'data.out')
                M.crypto(node, 'encrypt', key, [{'input': str(source), 'output': str(encrypted)}])
                self.assertEqual(encrypted.stat().st_size, len(data) + 28)
                M.crypto(node, 'decrypt', key, [{'input': str(encrypted), 'output': str(output)}])
                self.assertEqual(output.read_bytes(), data)
                other_key = root / 'other-key'; other_key.write_bytes(bytes(reversed(range(32))))
                wrong_output = root / 'wrong-key.out'
                with self.assertRaises(M.M.Rejected): M.crypto(node, 'decrypt', other_key, [{'input': str(encrypted), 'output': str(wrong_output)}])
                self.assertFalse(wrong_output.exists())
                damaged = bytearray(encrypted.read_bytes()); damaged[-1] ^= 1; encrypted.write_bytes(damaged)
                failed = root / 'failed.out'
                with self.assertRaises(M.M.Rejected): M.crypto(node, 'decrypt', key, [{'input': str(encrypted), 'output': str(failed)}])
                self.assertFalse(failed.exists())
                with self.assertRaises(M.M.Rejected): M.crypto(node, 'decrypt', key, [{'input': str(encrypted), 'output': str(output)}])
                self.assertEqual(output.read_bytes(), data)


if __name__ == '__main__': unittest.main()
