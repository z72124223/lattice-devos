import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

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


class OverlapReviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lattice-overlap-review-")
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name).resolve() / "codex"
        self.project = Path(self.temp.name).resolve() / "project"
        self.home.mkdir()
        self.project.mkdir()
        self.config = self.home / "config.toml"

    def duplicate_mcp(self):
        before = (b'model="gpt-6-astra"\n[features]\nshell_snapshot=true\n'
                  b'[mcp_servers.first]\ncommand="python"\nargs=["tool.py"]\n'
                  b'[mcp_servers.first.env]\nAPI_KEY="synthetic-secret"\n'
                  b'[mcp_servers.second]\nenabled=true # keep comment\ncommand="python"\nargs=["tool.py"]\n'
                  b'[mcp_servers.second.env]\nAPI_KEY="synthetic-secret"\n')
        self.config.write_bytes(before)
        return before

    def make_skill(self, name, script=b"identical helper"):
        folder = self.home / "skills" / name
        folder.mkdir(parents=True)
        (folder / "SKILL.md").write_bytes(b"---\nname: fixture\n---\nInspect lattice_runtime_status.\n")
        (folder / "helper.py").write_bytes(script)
        return folder

    def test_keep_choices_and_noninteractive_review_never_change_inputs(self):
        before = self.duplicate_mcp()
        with patch.object(A, "confirm_disable", return_value=False) as prompt:
            quiet = A.review(self.home, self.project)
            prompt.assert_not_called()
            self.assertFalse(quiet["user_acknowledged"])
            with patch.object(A, "ask_yes_no", return_value=True):
                interactive = A.review(self.home, self.project, interactive=True)
        self.assertTrue(interactive["user_acknowledged"])
        self.assertEqual(interactive["choice"], "KEEP_EXISTING")
        self.assertTrue(all(x["status"] == "KEPT" for x in interactive["resolution"]))
        self.assertEqual(self.config.read_bytes(), before)
        self.assertEqual(list(self.home.rglob("*.bak")), [])

    def test_exact_duplicate_mcp_disable_has_private_backup_and_preserves_other_bytes(self):
        before = self.duplicate_mcp()
        item = next(x for x in A.review_items(self.home) if x["target"] == "second")
        result = A.disable_item(self.home, self.project, item["id"])
        self.assertEqual(result["status"], "DISABLED_BACKUP_SAVED")
        self.assertEqual(Path(result["backup"]).read_bytes(), before)
        self.assertEqual(self.config.read_bytes(), before.replace(b"enabled=true #", b"enabled=false #"))
        self.assertEqual(A.review_items(self.home), [])
        with self.assertRaisesRegex(A.Rejected, "LAST_INSTANCE"):
            A.disable_mcp_bytes(self.config.read_bytes(), "first")

    def test_one_remaining_instance_is_preserved_during_interactive_review(self):
        self.duplicate_mcp()
        with patch.object(A, "confirm_disable", return_value=True) as prompt:
            with patch.object(A, "ask_yes_no", return_value=True):
                result = A.review(self.home, self.project, interactive=True)
        self.assertEqual(prompt.call_count, 1)
        self.assertEqual(len(result["resolution"]), 1)
        servers = A.CONFIG.parse(self.config.read_bytes())["mcp_servers"]
        self.assertEqual(sum(server.get("enabled", True) is True for server in servers.values()), 1)

    def test_changed_configuration_invalidates_prior_item(self):
        before = self.duplicate_mcp()
        item = A.review_items(self.home)[0]
        self.config.write_bytes(before + b"# changed by customer\n")
        with self.assertRaisesRegex(A.Rejected, "ITEM_CHANGED"):
            A.disable_item(self.home, self.project, item["id"])
        self.assertEqual(self.config.read_bytes(), before + b"# changed by customer\n")

    def test_equal_endpoint_with_different_credentials_is_warning_only(self):
        text = ('[mcp_servers.first]\nurl="https://example.test/private-url-secret"\n'
                '[mcp_servers.first.http_headers]\nAuthorization="secret-one"\n'
                '[mcp_servers.second]\nurl="https://example.test/private-url-secret"\n'
                '[mcp_servers.second.http_headers]\nAuthorization="secret-two"\n')
        self.config.write_text(text, encoding="utf-8")
        result = A.audit(self.home, self.project)
        self.assertEqual(result["status"], "WARNING")
        self.assertEqual(result["review_items"], [])
        output = json.dumps(result)
        for secret in ("private-url-secret", "secret-one", "secret-two"):
            self.assertNotIn(secret, output)

    def test_identical_skill_trees_move_to_backup_without_removing_last_copy(self):
        first = self.make_skill("first")
        second = self.make_skill("second")
        item = next(x for x in A.review_items(self.home) if x["target"] == "second")
        result = A.disable_item(self.home, self.project, item["id"])
        self.assertTrue(first.exists())
        self.assertFalse(second.exists())
        backup = Path(result["backup"])
        self.assertTrue(backup.is_relative_to(self.home / ".lattice-overlap-backups"))
        self.assertEqual((backup / "helper.py").read_bytes(), b"identical helper")
        self.assertEqual(A.review_items(self.home), [])
        with self.assertRaisesRegex(A.Rejected, "NO_LONGER_DUPLICATE"):
            A.disable_item(self.home, self.project, item["id"])

    def test_same_skill_name_or_markdown_is_insufficient_when_scripts_differ(self):
        self.make_skill("first", b"first behavior")
        self.make_skill("second", b"second behavior")
        self.assertEqual(A.review_items(self.home), [])

    def test_chinese_skill_names_remain_distinct_in_confirmation(self):
        self.make_skill("中文技能甲")
        self.make_skill("中文技能乙")
        item = next(x for x in A.review_items(self.home) if x["target"] == "中文技能乙")
        self.assertEqual(item["label"], "中文技能乙")
        self.assertEqual(item["alternatives"], ["中文技能甲"])
        with patch.object(A, "ask_yes_no", return_value=False) as prompt:
            self.assertFalse(A.confirm_disable(item))
        self.assertIn("是否備份並停用「中文技能乙」", prompt.call_args.args[0])
        self.assertIn("另外仍啟用的相同項目：中文技能甲", prompt.call_args.args[0])

    def test_unidentifiable_skill_names_are_manual_only(self):
        for name in ("\nname", "name\x00", "name\u202e", " " * 2, "x" * 101):
            self.assertEqual(A.bounded_label(name), "名稱已隱藏")
        self.make_skill("normal")
        self.make_skill("name\u202e")
        result = A.audit(self.home, self.project)
        self.assertEqual(result["status"], "WARNING")
        self.assertEqual(result["review_items"], [])
        self.assertTrue(all(x["action"] == "MANUAL_REVIEW_ONLY" for x in result["matches"]))

    def test_changed_backup_is_restored_to_absent_original_path(self):
        self.make_skill("first")
        second = self.make_skill("second")
        item = next(x for x in A.review_items(self.home) if x["target"] == "second")
        rename = Path.rename

        def move_then_change(path, target):
            result = rename(path, target)
            if path == second:
                (Path(target) / "helper.py").write_bytes(b"concurrent change")
            return result

        with patch.object(Path, "rename", move_then_change):
            with self.assertRaisesRegex(A.SkillDisableRejected, "DISABLE_ROLLED_BACK") as caught:
                A.disable_item(self.home, self.project, item["id"])
        self.assertEqual((second / "helper.py").read_bytes(), b"concurrent change")
        self.assertTrue(caught.exception.details["restored"])
        self.assertNotIn("backup", caught.exception.details)
        self.assertEqual(caught.exception.details["reason"], "OVERLAP_SKILL_BACKUP_READBACK_MISMATCH")

    def test_lost_last_identical_copy_after_move_restores_selected_skill(self):
        first = self.make_skill("first")
        second = self.make_skill("second")
        item = next(x for x in A.review_items(self.home) if x["target"] == "second")
        rename = Path.rename

        def move_then_change_alternative(path, target):
            result = rename(path, target)
            if path == second:
                (first / "helper.py").write_bytes(b"different concurrent behavior")
            return result

        with patch.object(Path, "rename", move_then_change_alternative):
            with self.assertRaisesRegex(A.SkillDisableRejected, "DISABLE_ROLLED_BACK") as caught:
                A.disable_item(self.home, self.project, item["id"])
        self.assertEqual(caught.exception.details["reason"], "OVERLAP_LAST_INSTANCE_MUST_REMAIN")
        self.assertEqual((second / "helper.py").read_bytes(), b"identical helper")
        self.assertEqual((first / "helper.py").read_bytes(), b"different concurrent behavior")

    def test_recreated_source_is_not_overwritten_and_cli_reports_backup(self):
        self.make_skill("first")
        second = self.make_skill("second")
        item = next(x for x in A.review_items(self.home) if x["target"] == "second")
        rename = Path.rename

        def move_then_recreate_source(path, target):
            result = rename(path, target)
            if path == second:
                second.mkdir()
                (second / "customer.txt").write_bytes(b"keep new source")
            return result

        with patch.object(Path, "rename", move_then_recreate_source):
            with self.assertRaisesRegex(A.SkillDisableRejected, "RESTORE_REQUIRED") as caught:
                A.disable_item(self.home, self.project, item["id"])
        self.assertEqual((second / "customer.txt").read_bytes(), b"keep new source")
        backup = Path(caught.exception.details["backup"])
        self.assertEqual((backup / "helper.py").read_bytes(), b"identical helper")
        self.assertFalse(caught.exception.details["restored"])
        stdout = io.StringIO()
        argv = [str(Path(A.__file__)), "--review", "--codex-home", str(self.home),
                "--project", str(self.project), "--report", str(self.home / "report.json")]
        with patch.object(sys, "argv", argv), patch.object(sys, "stdout", stdout):
            with patch.object(A, "review", side_effect=caught.exception):
                self.assertEqual(A.main(), 2)
        report = json.loads(stdout.getvalue())
        self.assertEqual(report["backup"], str(backup))
        self.assertEqual(report["source"], str(second))
        self.assertEqual(report["code"], "OVERLAP_SKILL_RESTORE_REQUIRED")

    def test_restore_io_failure_preserves_and_reports_backup(self):
        first = self.make_skill("first")
        second = self.make_skill("second")
        item = next(x for x in A.review_items(self.home) if x["target"] == "second")
        rename = Path.rename

        def move_then_refuse_restore(path, target):
            if path != second:
                raise PermissionError("synthetic failure")
            result = rename(path, target)
            (first / "helper.py").write_bytes(b"changed alternative")
            return result

        with patch.object(Path, "rename", move_then_refuse_restore):
            with self.assertRaisesRegex(A.SkillDisableRejected, "RESTORE_REQUIRED") as caught:
                A.disable_item(self.home, self.project, item["id"])
        self.assertFalse(second.exists())
        self.assertEqual((Path(caught.exception.details["backup"]) / "helper.py").read_bytes(), b"identical helper")

    def test_changed_skill_contents_invalidate_approval(self):
        self.make_skill("first")
        second = self.make_skill("second")
        item = next(x for x in A.review_items(self.home) if x["target"] == "second")
        (second / "helper.py").write_bytes(b"customer change")
        with self.assertRaisesRegex(A.Rejected, "ITEM_CHANGED"):
            A.disable_item(self.home, self.project, item["id"])
        self.assertTrue(second.exists())

    def test_plugins_and_native_schedules_are_manual_only(self):
        plugin = self.home / "plugins" / "fixture"
        plugin.mkdir(parents=True)
        (plugin / "plugin.json").write_text('{"instruction":"lattice_runtime_status"}', encoding="utf-8")
        automation = self.home / "automations" / "fixture"
        automation.mkdir(parents=True)
        original = 'status="ACTIVE"\nprompt="lattice_runtime_status heartbeat"\n'
        (automation / "automation.toml").write_text(original, encoding="utf-8")
        with patch.object(A, "confirm_disable") as prompt:
            with patch.object(A, "ask_yes_no", return_value=False):
                result = A.review(self.home, self.project, interactive=True)
        prompt.assert_not_called()
        self.assertFalse(result["user_acknowledged"])
        self.assertEqual(result["choice"], "STOP")
        self.assertTrue(all(item["action"] == "MANUAL_REVIEW_ONLY" for item in result["matches"]))
        self.assertEqual((automation / "automation.toml").read_text(encoding="utf-8"), original)

    def test_generic_scheduler_documentation_is_not_a_duplicate_workflow(self):
        folder = self.home / "skills" / "docs"
        folder.mkdir(parents=True)
        (folder / "SKILL.md").write_text("Explain automation, schedules and heartbeat.", encoding="utf-8")
        self.assertEqual(A.audit(self.home, self.project)["matches"], [])

    def test_unknown_toml_layout_never_gets_disable_option(self):
        self.config.write_text('mcp_servers={first={command="python"},second={command="python"}}\n', encoding="utf-8")
        self.assertEqual(A.review_items(self.home), [])
        self.assertEqual(A.audit(self.home, self.project)["status"], "WARNING")

    def test_plugin_mcp_manifest_is_report_only_without_exposing_secrets(self):
        plugin = self.home / "plugins" / "fixture"
        plugin.mkdir(parents=True)
        (plugin / ".mcp.json").write_text(json.dumps({"mcpServers": {"graphify": {
            "command": "graphify", "env": {"API_KEY": "private-value"}}}}), encoding="utf-8")
        result = A.audit(self.home, self.project)
        self.assertEqual(result["status"], "WARNING")
        self.assertIn("plugin_mcp_candidate", result["matches"][0]["matches"])
        self.assertEqual(result["review_items"], [])
        self.assertNotIn("private-value", json.dumps(result))

    def test_cli_noninteractive_review_and_report_source_protection(self):
        before = self.duplicate_mcp()
        command = [sys.executable, "-B", str(Path(A.__file__)), "--review", "--codex-home", str(self.home),
                   "--project", str(self.project)]
        report = self.home / "overlap-report.json"
        result = subprocess.run(command + ["--report", str(report)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout)["choice"], "REVIEW_REQUIRED")
        self.assertEqual(self.config.read_bytes(), before)
        rejected = subprocess.run(command + ["--report", str(self.config)], capture_output=True, text=True)
        self.assertEqual(rejected.returncode, 2)
        self.assertEqual(json.loads(rejected.stdout)["code"], "OVERLAP_REPORT_OVERLAPS_SOURCE")
        self.assertEqual(self.config.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
