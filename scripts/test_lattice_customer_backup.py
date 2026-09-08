import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("backup_test", Path(__file__).with_name("lattice-customer-backup.py"))
M = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(M)


class BackupTests(unittest.TestCase):
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
