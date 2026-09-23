"""Source binding tests use fake SDKs in disposable worktree directories only."""

import json
from pathlib import Path
import subprocess
import sys
import textwrap
import unittest


HERE = Path(__file__).resolve().parent


class LayaSourceTests(unittest.TestCase):
    def scenario(self, body):
        setup = f"import sys; sys.path.insert(0, {str(HERE)!r})\n" + textwrap.dedent('''
            import hashlib, json, tempfile, types
            from pathlib import Path
            from laya_source import SDK_REVISION, load_verified_source, verify_loaded_modules
            def freeze(root):
                entries = []
                for name in ('pyproject.toml', 'laya/__init__.py', 'laya/child.py'):
                    data = (root / name).read_bytes()
                    entries.append({'file': name, 'size': len(data), 'git_blob':
                        hashlib.sha1(f'blob {len(data)}\\0'.encode() + data).hexdigest()})
                return {'sdk_revision': SDK_REVISION, 'sdk_files': entries}
            def expect_rejection(action):
                try:
                    action()
                except (ValueError, FileNotFoundError):
                    return
                raise AssertionError('Unverified source was accepted')
        ''')
        script = setup + f"with tempfile.TemporaryDirectory(prefix='.laya-source-test-', dir={str(HERE)!r}) as directory:\n"
        fixture = '''
            root = Path(directory).resolve()
            (root / 'laya').mkdir()
            (root / 'pyproject.toml').write_text('[project]\\nversion = "0.1.0"\\n', encoding='utf8')
            (root / 'laya/__init__.py').write_text('from . import child\\n__version__ = "0.1.0"\\n', encoding='utf8')
            (root / 'laya/child.py').write_text('VALUE = 7\\n', encoding='utf8')
            manifest = freeze(root)
        '''
        script += textwrap.indent(textwrap.dedent(fixture) + textwrap.dedent(body), "    ")
        result = subprocess.run([sys.executable, "-I", "-S", "-B", "-c", script],
                                capture_output=True, text=True, timeout=20, cwd=HERE)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)

    def test_normal_version_and_loaded_module_binding(self):
        data = self.scenario('''
            module, version = load_verified_source(root, manifest)
            assert version == module.__version__ == '0.1.0'
            assert module.child.VALUE == 7
            assert Path(module.__file__) == root / 'laya/__init__.py'
            assert verify_loaded_modules(root, manifest) == ['laya', 'laya.child']
            assert not list(root.rglob('*.pyc'))
            (root / 'laya/child.py').write_text('VALUE = 8\\n', encoding='utf8')
            expect_rejection(lambda: verify_loaded_modules(root, manifest))
            print(json.dumps({'version': version, 'post_load_tamper_rejected': True}))
        ''')
        self.assertEqual(data["version"], "0.1.0")

    def test_tamper_before_import_and_revision_mismatch(self):
        data = self.scenario('''
            (root / 'laya/child.py').write_text('VALUE = 9\\n', encoding='utf8')
            expect_rejection(lambda: load_verified_source(root, manifest))
            assert 'laya' not in sys.modules
            manifest = freeze(root)
            manifest['sdk_revision'] = '0' * 40
            expect_rejection(lambda: load_verified_source(root, manifest))
            assert 'laya' not in sys.modules
            print(json.dumps({'rejected': True}))
        ''')
        self.assertTrue(data["rejected"])

    def test_preloaded_and_wrong_module_origins_are_rejected(self):
        data = self.scenario('''
            sys.modules['laya.preloaded'] = types.ModuleType('laya.preloaded')
            expect_rejection(lambda: load_verified_source(root, manifest))
            del sys.modules['laya.preloaded']
            module, _ = load_verified_source(root, manifest)
            expect_rejection(lambda: load_verified_source(root, manifest))
            module.child.__file__ = str(root / 'laya/__init__.py')
            expect_rejection(lambda: verify_loaded_modules(root, manifest))
            print(json.dumps({'rejected': True}))
        ''')
        self.assertTrue(data["rejected"])

    def test_version_mismatch_and_unmanifested_executable_files(self):
        data = self.scenario('''
            for name in ('injected.py', 'cached.pyc', 'native.pyd'):
                extra = root / 'laya' / name
                extra.write_bytes(b'not part of the manifest')
                expect_rejection(lambda: load_verified_source(root, manifest))
                assert 'laya' not in sys.modules
                extra.unlink()
            (root / 'laya/__init__.py').write_text('__version__ = "9.9.9"\\n', encoding='utf8')
            manifest = freeze(root)
            expect_rejection(lambda: load_verified_source(root, manifest))
            print(json.dumps({'rejected': True}))
        ''')
        self.assertTrue(data["rejected"])


if __name__ == "__main__":
    unittest.main()
