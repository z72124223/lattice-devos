import importlib.util
import copy
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("environment_acceptance", Path(__file__).with_name("verify-lattice-customer-environment.py"))
M = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(M)
CONFIG = importlib.util.spec_from_file_location("config_acceptance", Path(__file__).with_name("lattice-mcp-config.py"))
C = importlib.util.module_from_spec(CONFIG); CONFIG.loader.exec_module(C)


class DestinationTests(unittest.TestCase):
    def test_reference_cannot_authorize_missing_or_cross_project_bindings(self):
        args = types.SimpleNamespace(project_id='p', completed_task='child', commit='c', query='q')
        task = {'status': 'COMPLETED', 'result_digest': 'digest', 'task_ref': 'child', 'project_id': 'p'}
        valid = {'task': task, 'product': {'project': {'id': 'p'}, 'tasks': [{'task_ref': 'child', 'ledger': task}, {'task_ref': 'parent', 'ledger': {'project_id': 'p'}}],
                 'product': {'metadata': [{'task_ref': 'child', 'parent_ref': 'parent'}], 'decisions': [{'task_ref': 'child'}]}},
                 'decisions': {'scope': 'p', 'decisions': ['decision']}, 'graph': {'registered_project_id': 'p', 'commit': 'c', 'query': 'q', 'records': ['relation'], 'source_receipt_digest': 'receipt'}}
        M.verify_bindings(valid, args)
        for mutation in ('parent', 'decisions', 'project', 'graph_project', 'commit', 'duplicate_metadata'):
            bad = copy.deepcopy(valid)
            if mutation == 'parent': bad['product']['product']['metadata'][0]['parent_ref'] = None
            elif mutation == 'decisions': bad['decisions']['decisions'] = []
            elif mutation == 'project': bad['task']['project_id'] = 'other'
            elif mutation == 'graph_project': bad['graph']['registered_project_id'] = 'other'
            elif mutation == 'commit': bad['graph']['commit'] = 'other'
            else: bad['product']['product']['metadata'] *= 2
            with self.subTest(mutation=mutation), self.assertRaisesRegex(RuntimeError, 'WORKFLOW_AND_RELATION_BINDINGS_REJECTED'):
                M.verify_bindings(bad, args)

    def test_failed_process_retains_bounded_output_and_phase(self):
        args = types.SimpleNamespace(project_id='p', completed_task='t', commit='c', query='q')
        evidence = {}; child = types.SimpleNamespace(args=['python'], returncode=2, communicate=lambda *a, **k: ('x'*70000, 'native failure'))
        module = types.SimpleNamespace(closed_environment=lambda: {})
        with patch.object(M.subprocess, 'Popen', return_value=child):
            with self.assertRaisesRegex(RuntimeError, 'CONFIGURED_MCP_PROCESS_REJECTED'):
                M.snapshot(module, {'command': 'python', 'args': []}, args, evidence, 'before')
        self.assertEqual(evidence['phase'], 'before')
        self.assertEqual(len(evidence['before_process']['stdout']), 65536)
        self.assertEqual(evidence['before_process']['stderr'], 'native failure')

    def test_normalized_dotdot_cannot_enter_bundle_or_customer_project(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / "intermediate").mkdir()
            for name in ("bundle", "project"):
                source = root / name; source.mkdir()
                candidate = root / "intermediate" / ".." / name / "new-evidence"
                with self.assertRaisesRegex(RuntimeError, "EVIDENCE_SOURCE_OVERLAP"):
                    M.evidence_destination(types.SimpleNamespace(CONFIG=C), candidate, [source])
                self.assertFalse((source / "new-evidence").exists())

    def test_fresh_sibling_is_canonical_and_existing_directory_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); source = root / "source"; source.mkdir()
            self.assertEqual(M.evidence_destination(types.SimpleNamespace(CONFIG=C), source / ".." / "evidence", [source]), root.resolve() / "evidence")
            with self.assertRaisesRegex(RuntimeError, "FRESH_EVIDENCE_DIRECTORY_REQUIRED"):
                M.evidence_destination(types.SimpleNamespace(CONFIG=C), source, [source])


if __name__ == "__main__": unittest.main()
