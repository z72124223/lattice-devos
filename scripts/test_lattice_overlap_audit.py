import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("audit", Path(__file__).with_name("lattice-overlap-audit.py"))
A = importlib.util.module_from_spec(spec)
spec.loader.exec_module(A)


class McpAuditTests(unittest.TestCase):
    def check(self, text):
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "config.toml"
            config.write_text(text, encoding="utf-8")
            result = A.mcp_duplicates(config)
            self.assertEqual(config.read_text(encoding="utf-8"), text)
            return result

    def test_duplicate_enabled_servers_warn_without_transport_secrets(self):
        warnings = self.check('[mcp_servers.first]\nurl="https://example.test/private-token"\n'
                              '[mcp_servers.second]\nurl="https://example.test/private-token"\n')
        self.assertEqual(len(warnings), 1)
        self.assertIn("first", warnings[0])
        self.assertIn("second", warnings[0])
        self.assertNotIn("private-token", warnings[0])

    def test_disabled_server_is_not_a_duplicate(self):
        self.assertEqual(self.check('[mcp_servers.a]\ncommand="latticed.exe"\n'
                                   '[mcp_servers.b]\ncommand="latticed.exe"\nenabled=false\n'), [])

    def test_different_arguments_are_distinct(self):
        self.assertEqual(self.check('[mcp_servers.a]\ncommand="python"\nargs=["a.py"]\n'
                                   '[mcp_servers.b]\ncommand="python"\nargs=["b.py"]\n'), [])

    def test_unreadable_config_does_not_claim_clear(self):
        self.assertEqual(len(self.check('broken = "')), 1)


if __name__ == "__main__":
    unittest.main()
