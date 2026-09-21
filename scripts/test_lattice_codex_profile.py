import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("lattice-codex-profile.py")
SPEC = importlib.util.spec_from_file_location("lattice_codex_profile", SCRIPT)
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


class ProfileTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lattice-profile-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.config = self.root / "config.toml"
        self.profile = self.root / "profile.json"

    def make_profile(self, **preferences):
        self.profile.write_text(json.dumps({"schema": M.SCHEMA, "preferences": preferences}), encoding="utf-8")

    def test_legacy_export_cli_is_compatible_and_only_exports_preferences(self):
        self.config.write_text('model = "gpt-6-astra"\nmodel_reasoning_effort = "high"\n'
                               'approval_policy = "never"\nsandbox_mode = "danger-full-access"\n'
                               '[mcp_servers.other]\ncommand = "C:/private/tool.exe"\n'
                               '[features]\nexperimental_api_key = "synthetic-secret"\n', encoding="utf-8")
        result = subprocess.run([sys.executable, "-B", str(SCRIPT), "--config", str(self.config),
                                 "--output", str(self.profile)], capture_output=True, text=True, check=True)
        exported = json.loads(result.stdout)
        self.assertEqual(exported["preferences"], {"model": "gpt-6-astra", "model_reasoning_effort": "high"})
        self.assertNotIn("synthetic-secret", result.stdout)
        self.assertNotIn("C:/private", self.profile.read_text(encoding="utf-8"))

    def test_absolute_paths_secrets_and_wrong_types_are_not_portable(self):
        for value in ("C:/private/model", r"C:\private\model", "/home/private/model", r"\\host\share",
                      "sk-test-secret", "gpt-6-api-key", "gpt-6\nsecret", True, 10, ["gpt-6-astra"], None):
            with self.subTest(value=value):
                self.assertIsNone(M.safe_value("model", value))
        self.assertIsNone(M.safe_value("features", {"safe": "/private"}))
        self.assertIsNone(M.safe_value("reasoning_effort", "high"))

    def test_custom_provider_settings_are_never_exported(self):
        self.config.write_text('model = "gpt-6-astra"\nmodel_provider = "custom"\n'
                               'model_reasoning_effort = "high"\npersonality = "friendly"\n'
                               '[model_providers.custom]\nbase_url = "https://private.invalid"\n', encoding="utf-8")
        result = M.export(self.config, self.profile)
        self.assertEqual(result["preferences"], {"personality": "friendly"})

    def test_conflict_requires_exact_plan_and_explicit_confirmation_then_preserves_other_bytes(self):
        before = (b'\xef\xbb\xbf# existing\r\nmodel = "gpt-5.6-luna" # chosen\r\n'
                  b'approval_policy = "on-request"\r\nsandbox_mode = "read-only"\r\n'
                  b'[mcp_servers.other]\r\ncommand = "C:/customer/tool.exe"\r\n'
                  b'[mcp_servers.other.env]\r\nSYNTHETIC_API_KEY = "private-placeholder"\r\n'
                  b'[profiles.other]\r\nmodel = "gpt-5.6-terra"\r\n')
        self.config.write_bytes(before)
        self.make_profile(model="gpt-6-astra", model_reasoning_effort="high")
        preview = M.plan(self.config, self.profile)
        self.assertEqual(preview["status"], "REVIEW_REQUIRED")
        self.assertEqual(preview["changes"][0], {"key": "model", "before": "gpt-5.6-luna", "after": "gpt-6-astra", "conflict": True})
        with self.assertRaisesRegex(M.Rejected, "CONFLICT_CONFIRMATION"):
            M.apply(self.config, self.profile, preview["plan_sha256"])
        self.assertEqual(self.config.read_bytes(), before)
        result = M.apply(self.config, self.profile, preview["plan_sha256"], confirm_conflicts=True)
        self.assertEqual(Path(result["backup"]).read_bytes(), before)
        after = self.config.read_bytes()
        expected = b'\xef\xbb\xbfmodel_reasoning_effort = "high"\r\n' + before[3:].replace(b'model = "gpt-5.6-luna"', b'model = "gpt-6-astra"', 1)
        self.assertEqual(after, expected)
        again = M.plan(self.config, self.profile)
        self.assertEqual(M.apply(self.config, self.profile, again["plan_sha256"])["status"], "UNCHANGED")

    def test_empty_config_additions_do_not_require_conflict_override(self):
        self.make_profile(model="gpt-6-astra", personality="pragmatic")
        preview = M.plan(self.config, self.profile)
        self.assertEqual(preview["status"], "READY")
        result = M.apply(self.config, self.profile, preview["plan_sha256"])
        self.assertEqual(result["status"], "PREFERENCES_APPLIED")
        self.assertEqual(Path(result["backup"]).read_bytes(), b"")
        self.assertEqual(M.CONFIG.parse(self.config.read_bytes()), {"model": "gpt-6-astra", "personality": "pragmatic"})

    def test_changed_config_or_profile_invalidates_review(self):
        self.config.write_text('personality = "friendly"\n', encoding="utf-8")
        self.make_profile(model="gpt-6-astra")
        preview = M.plan(self.config, self.profile)
        self.config.write_text('personality = "pragmatic"\n', encoding="utf-8")
        with self.assertRaisesRegex(M.Rejected, "PLAN_STALE"):
            M.apply(self.config, self.profile, preview["plan_sha256"], confirm_conflicts=True)
        newer = M.plan(self.config, self.profile)
        self.make_profile(model="gpt-5.6-luna")
        with self.assertRaisesRegex(M.Rejected, "PLAN_STALE"):
            M.apply(self.config, self.profile, newer["plan_sha256"], confirm_conflicts=True)
        self.assertEqual(self.config.read_text(encoding="utf-8"), 'personality = "pragmatic"\n')

    def test_malicious_profiles_cannot_change_policy_mcp_provider_or_paths(self):
        for preferences in ({"approval_policy": "never"}, {"mcp_servers": {}}, {"model_provider": "custom"},
                            {"model": "C:/private/model"}, {"model": "sk-private"}, {"model_verbosity": "LOUD"},
                            {"features": {"x": True}}):
            with self.subTest(preferences=preferences):
                self.make_profile(**preferences)
                with self.assertRaisesRegex(M.Rejected, "PROFILE_PREFERENCE_REJECTED"):
                    M.plan(self.config, self.profile)
        self.assertFalse(self.config.exists())

    def test_target_custom_provider_requires_manual_review_even_with_confirmation(self):
        self.config.write_text('model_provider = "private"\nmodel = "gpt-5.6-luna"\n', encoding="utf-8")
        self.make_profile(model="gpt-6-astra")
        preview = M.plan(self.config, self.profile)
        self.assertEqual(preview["status"], "PROVIDER_REVIEW_REQUIRED")
        with self.assertRaisesRegex(M.Rejected, "PROVIDER_REVIEW_REQUIRED"):
            M.apply(self.config, self.profile, preview["plan_sha256"], confirm_conflicts=True)

    def test_preview_redacts_unsafe_existing_values(self):
        self.config.write_text('model = "sk-synthetic-secret"\n', encoding="utf-8")
        self.make_profile(model="gpt-6-astra")
        preview = M.plan(self.config, self.profile)
        self.assertNotIn("sk-synthetic-secret", json.dumps(preview))
        self.assertTrue(preview["changes"][0]["conflict"])

    def test_multiline_toml_cannot_trick_writer_into_changing_unrelated_instructions(self):
        before = b'developer_instructions = """\nmodel = "gpt-5.6-luna"\n"""\nmodel = "gpt-5.6-luna"\n'
        self.config.write_bytes(before)
        self.make_profile(model="gpt-6-astra")
        preview = M.plan(self.config, self.profile)
        with self.assertRaisesRegex(M.Rejected, "LAYOUT_REQUIRES_MANUAL_REVIEW"):
            M.apply(self.config, self.profile, preview["plan_sha256"], confirm_conflicts=True)
        self.assertEqual(self.config.read_bytes(), before)

    def test_cli_plan_apply_and_output_overlap(self):
        self.make_profile(model="gpt-6-astra")
        command = [sys.executable, "-B", str(SCRIPT)]
        preview = json.loads(subprocess.check_output(command + ["plan", "--config", str(self.config), "--profile", str(self.profile)], text=True))
        applied = json.loads(subprocess.check_output(command + ["apply", "--config", str(self.config), "--profile", str(self.profile),
                                                    "--plan-sha256", preview["plan_sha256"]], text=True))
        self.assertEqual(applied["status"], "PREFERENCES_APPLIED")
        before = self.config.read_bytes()
        result = subprocess.run(command + ["--config", str(self.config), "--output", str(self.config)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.config.read_bytes(), before)

    def test_output_alias_cannot_overwrite_source_config(self):
        before = b'model = "gpt-6-astra"\n'
        self.config.write_bytes(before)
        folder = self.root / "nested"
        folder.mkdir()
        with self.assertRaisesRegex(M.Rejected, "OUTPUT_OVERLAPS_CONFIG"):
            M.export(self.config, folder / ".." / "config.toml")
        self.assertEqual(self.config.read_bytes(), before)

    def test_setup_applies_missing_defaults_and_replays_without_dialog(self):
        self.make_profile(model="gpt-6-astra", model_reasoning_effort="high")
        with patch.object(M, "confirm_preferences") as dialog:
            self.assertEqual(M.setup(self.config, self.profile)["status"], "PREFERENCES_APPLIED")
            self.assertEqual(M.setup(self.config, self.profile, interactive=True)["status"], "UNCHANGED")
            dialog.assert_not_called()

    def test_setup_preserves_conflicts_without_interaction_or_when_declined(self):
        before = b'model = "gpt-5.6-luna"\n'
        self.config.write_bytes(before)
        self.make_profile(model="gpt-6-astra", model_reasoning_effort="high")
        with patch.object(M, "confirm_preferences", return_value=False) as dialog:
            result = M.setup(self.config, self.profile)
            self.assertEqual(result["status"], "REVIEW_REQUIRED")
            dialog.assert_not_called()
            result = M.setup(self.config, self.profile, interactive=True)
            self.assertEqual(result["status"], "USER_PREFERENCES_PRESERVED")
            dialog.assert_called_once()
            self.assertEqual(dialog.call_args.args[0]["changes"][0]["before"], "gpt-5.6-luna")
        self.assertEqual(self.config.read_bytes(), before)
        self.assertEqual(list(self.root.rglob("*.bak")), [])

    def test_setup_yes_applies_only_reviewed_changes_with_backup(self):
        before = b'model = "gpt-5.6-luna"\nsandbox_mode = "read-only"\n'
        self.config.write_bytes(before)
        self.make_profile(model="gpt-6-astra")
        with patch.object(M, "confirm_preferences", return_value=True) as dialog:
            result = M.setup(self.config, self.profile, interactive=True)
            dialog.assert_called_once()
        self.assertEqual(result["status"], "PREFERENCES_APPLIED")
        self.assertEqual(Path(result["backup"]).read_bytes(), before)
        self.assertEqual(self.config.read_bytes(), before.replace(b"gpt-5.6-luna", b"gpt-6-astra"))

    def test_setup_dialog_does_not_authorize_changes_after_review(self):
        self.config.write_text('model = "gpt-5.6-luna"\n', encoding="utf-8")
        self.make_profile(model="gpt-6-astra")
        def changed_while_dialog_open(preview):
            self.config.write_text('model = "gpt-5.6-terra"\n', encoding="utf-8")
            return True
        with patch.object(M, "confirm_preferences", side_effect=changed_while_dialog_open):
            with self.assertRaisesRegex(M.Rejected, "PLAN_STALE"):
                M.setup(self.config, self.profile, interactive=True)
        self.assertEqual(self.config.read_text(encoding="utf-8"), 'model = "gpt-5.6-terra"\n')

    def test_setup_provider_rejection_never_offers_override(self):
        before = b'model_provider = "custom"\n'
        self.config.write_bytes(before)
        self.make_profile(model="gpt-6-astra")
        with patch.object(M, "confirm_preferences") as dialog:
            self.assertEqual(M.setup(self.config, self.profile, interactive=True)["status"], "PROVIDER_REVIEW_REQUIRED")
            dialog.assert_not_called()
        result = subprocess.run([sys.executable, "-B", str(SCRIPT), "setup", "--config", str(self.config),
                                 "--profile", str(self.profile)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stdout)["status"], "PROVIDER_REVIEW_REQUIRED")
        self.assertEqual(self.config.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
