import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("one_click", Path(__file__).with_name("lattice-one-click-install.py"))
I = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(I)


class InstallerTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "nt", "requires the real Windows CMD parser")
    def test_actual_cmd_with_chinese_text_preserves_missing_bundle_failure(self):
        with tempfile.TemporaryDirectory(prefix="lattice-cmd-entry-") as directory:
            entry = Path(directory) / "Install-LATTICE.cmd"
            shutil.copyfile(Path(__file__).resolve().parents[1] / entry.name, entry)
            result = subprocess.run([str(Path(os.environ["SystemRoot"]) / "System32/cmd.exe"),
                                     "/d", "/c", str(entry)], capture_output=True,
                                    env=dict(os.environ, LATTICE_INSTALL_UNATTENDED="1"), timeout=15)
            self.assertEqual(result.returncode, 2, result.stdout.decode("utf-8", errors="replace"))
            self.assertIn("缺少內附 Python".encode("utf-8"), result.stdout)
            self.assertNotIn(b"is not recognized", result.stdout + result.stderr)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lattice-installer-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.bundle = self.root / "bundle"
        for name in I.REQUIRED:
            path = self.bundle / name
            if name in ("postgres/bin", "graphify"):
                path.mkdir(parents=True, exist_ok=True)
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture", encoding="utf-8")
        (self.bundle / "bundle.json").write_text(json.dumps({"schema": "lattice.windows-dependency-bundle.v1"}), encoding="utf-8")
        (self.bundle / "platform").mkdir()
        (self.bundle / "platform/ubuntu.wsl").write_bytes(b"fixture")
        self.state = self.root / "customer/runtime"
        self.source = self.root / "project"
        self.source.mkdir()
        self.wsl = self.root / "wsl.exe"
        self.wsl.write_bytes(b"launcher")
        self.codex = self.root / "isolated-codex/config.toml"
        self.codex.parent.mkdir()
        self.calls = []

    def staged(self):
        return {"status": "STAGED", "exit_code": 0, "manifest_sha256": "a" * 64,
                "bundle": str(self.state.parent / "bundles" / ("a" * 24))}

    def response(self, arguments):
        self.calls.append(arguments)
        self.assertEqual(arguments[1:4], ["-I", "-B", "-S"])
        if "lattice-bundle.py" in arguments[4]:
            if arguments[5] == "verify":
                return {"status": "LOCAL_BUNDLE_VERIFIED", "exit_code": 0}
            self.assertFalse(self.state.exists(), "WSL provisioning must not allocate runtime state")
            return {"status": I.RUNNING, "action": "recover", "initialized": True, "exit_code": 0}
        if "lattice-wsl-platform.py" in arguments[4]:
            root = Path(arguments[arguments.index("--root") + 1])
            self.assertFalse(I.overlaps(root, self.state))
            root.mkdir(parents=True)
            (root / "platform.json").write_text("{}")
            return {"status": "WSL_PLATFORM_IDENTITY_VERIFIED", "exit_code": 0}
        action = arguments[7]
        if action == "register-project":
            return {"status": "LOCATOR_SAVED", "exit_code": 0}
        payload = {"status": I.RUNNING, "initialized": True, "exit_code": 0}
        if action == "graphify-refresh":
            payload["operation_evidence"] = {"component": "graphify", "status": "PERSISTED"}
        return payload

    def install(self, response=None):
        with patch.object(I, "prepare_source"), patch.object(I, "verify_mcp", return_value={"status": "VERIFIED"}), \
                patch.object(I, "run_json", side_effect=response or self.response):
            return I.run_install(self.bundle, self.state, self.source, self.wsl, None, self.codex, "Test")

    def test_fresh_install_keeps_wsl_outside_runtime_and_runs_all_acceptance_steps(self):
        result = self.install()
        self.assertEqual(result["status"], "INSTALLED")
        self.assertEqual([step["action"] for step in result["post_install_steps"]],
                         ["install", "register-project", "graphify-preflight", "graphify-refresh", "mcp-acceptance", "connect"])
        self.assertEqual(self.calls[0][5], "verify")
        self.assertFalse((self.source / "AGENTS.md").exists())
        self.assertFalse(self.codex.exists(), "tests must never touch real or isolated Codex config")

    def test_failed_bundle_verification_keeps_bytecode_and_does_not_prepare_source(self):
        cache = self.bundle / "python/__pycache__/required.pyc"
        cache.parent.mkdir()
        cache.write_bytes(b"manifested-bytecode")
        with patch.object(I, "prepare_source") as prepare, patch.object(I, "run_json", return_value={
            "status": "BLOCKED", "code": "BUNDLE_CONTENT_CHANGED", "exit_code": 2,
        }):
            result = I.run_install(self.bundle, self.state, self.source, self.wsl, None, self.codex, "Test")
        self.assertEqual(result["code"], "BUNDLE_CONTENT_CHANGED")
        self.assertEqual(cache.read_bytes(), b"manifested-bytecode")
        self.assertFalse(self.state.exists())
        prepare.assert_not_called()

    def test_graph_refresh_failure_prevents_connect(self):
        def response(arguments):
            if "graphify-refresh" in arguments:
                self.calls.append(arguments)
                return {"status": "BLOCKED", "code": "GRAPHIFY_FAILED", "exit_code": 2}
            return self.response(arguments)
        result = self.install(response)
        self.assertEqual(result["status"], "BLOCKED")
        self.assertEqual(result["code"], "GRAPHIFY_FAILED")
        self.assertFalse(any("connect" in command for command in self.calls))

    def test_successful_process_without_persisted_graph_is_not_installed(self):
        def response(arguments):
            payload = self.response(arguments)
            if "graphify-refresh" in arguments:
                payload["operation_evidence"] = {"component": "graphify", "status": "IDENTITY_VERIFIED"}
            return payload
        result = self.install(response)
        self.assertEqual(result["code"], "GRAPHIFY_REFRESH_NOT_PERSISTED")
        self.assertFalse(any("connect" in command for command in self.calls))

    def test_connect_failure_cannot_be_reported_installed(self):
        def response(arguments):
            if "connect" in arguments:
                return {"status": "BLOCKED", "code": "LATTICE_CONFIG_CONFLICT", "exit_code": 2}
            return self.response(arguments)
        result = self.install(response)
        self.assertEqual(result["status"], "BLOCKED")
        self.assertEqual(result["code"], "LATTICE_CONFIG_CONFLICT")

    def test_existing_runtime_recovers_without_install_or_provision(self):
        self.state.mkdir(parents=True)
        config = {"root": str(self.state), "dependency_root": str(self.bundle),
                  "graph_source": str(self.source), "wsl": str(self.wsl)}
        (self.state / "installation.json").write_text(json.dumps(config))
        (self.state / "credentials.dpapi").write_bytes(b"sealed")
        result = self.install()
        self.assertEqual(result["status"], "INSTALLED")
        self.assertEqual(result["post_install_steps"][0]["action"], "recover")
        self.assertFalse(any("install" in args or "provision" in args for args in self.calls))
        self.assertEqual(json.loads((self.state / "installation.json").read_text()), config)

    def test_existing_runtime_for_another_project_is_preserved(self):
        self.state.mkdir(parents=True)
        original = json.dumps({"root": str(self.state), "dependency_root": str(self.bundle),
                               "graph_source": str(self.root / "other-project"), "wsl": str(self.wsl)})
        (self.state / "installation.json").write_text(original)
        result = self.install()
        self.assertEqual(result["code"], "CUSTOMER_INSTALLATION_ARGUMENT_CONFLICT")
        self.assertEqual(len(self.calls), 1)
        self.assertEqual((self.state / "installation.json").read_text(), original)

    def test_run_json_rejects_nonzero_even_when_output_claims_success(self):
        with patch.object(I, "invoke", return_value=subprocess.CompletedProcess([], 5, '{"status":"INSTALLED"}', "")):
            result = I.run_json(["fixture"])
        self.assertEqual(result["status"], "BLOCKED")
        self.assertEqual(result["exit_code"], 5)

    def test_run_json_accepts_multiline_error_json_on_stderr(self):
        error = json.dumps({"status": "BLOCKED", "code": "CUSTOMER_INSTALLATION_CHANGED"}, indent=2)
        with patch.object(I, "invoke", return_value=subprocess.CompletedProcess([], 2, "", error)):
            self.assertEqual(I.run_json(["fixture"])["code"], "CUSTOMER_INSTALLATION_CHANGED")

    def test_managed_sample_commits_hook_and_code_before_validation(self):
        sample = self.root / "sample"
        calls = []

        def git(bundle, source, *arguments):
            calls.append(arguments)
            if "commit" in arguments:
                self.assertTrue((sample / "AGENTS.md").is_file())
                self.assertNotIn("lattice_runtime_status", (sample / "AGENTS.md").read_text(encoding="utf-8"))
                self.assertIn("def greeting", (sample / "sample.py").read_text())
                self.assertTrue(any("add" in previous and "AGENTS.md" in previous for previous in calls))
            if arguments == ("rev-parse", "--show-toplevel"):
                return str(sample)
            return "" if arguments[0] == "status" else "verified"

        with patch.object(I, "git_command", side_effect=git):
            I.prepare_source(self.bundle, sample, managed_sample=True)
            first = len(calls)
            I.prepare_source(self.bundle, sample, managed_sample=True)
        self.assertTrue(any("user.email=setup@localhost" in args for args in calls))
        self.assertFalse(any("init" in args or "add" in args or "commit" in args for args in calls[first:]))

    def test_managed_sample_never_adopts_existing_directory(self):
        before = self.source / "user-data.txt"
        before.write_text("keep me")
        with patch.object(I, "git_command") as git, self.assertRaisesRegex(I.Rejected, "NOT_OWNED"):
            I.prepare_source(self.bundle, self.source, managed_sample=True)
        git.assert_not_called()
        self.assertEqual(list(self.source.iterdir()), [before])
        self.assertEqual(before.read_text(), "keep me")

    def test_git_ignores_inherited_config_identity_hooks_and_index(self):
        env = {"GIT_INDEX_FILE": "user-index", "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "alias.init", "GIT_AUTHOR_NAME": "user"}
        with patch.dict(I.os.environ, env), patch.object(I, "invoke", return_value=subprocess.CompletedProcess([], 0, "ok", "")) as run:
            I.git_command(self.bundle, self.source, "status", "--porcelain")
        command = run.call_args.args[0]
        supplied = run.call_args.kwargs["env"]
        self.assertIn("core.hooksPath=" + I.os.devnull, command)
        self.assertIn("core.fsmonitor=false", command)
        self.assertEqual(supplied["GIT_CONFIG_GLOBAL"], I.os.devnull)
        self.assertEqual(supplied["GIT_CONFIG_NOSYSTEM"], "1")
        self.assertTrue(all(key not in supplied for key in env))

    def test_dirty_user_project_is_rejected_without_hook_or_commit(self):
        def git(bundle, source, *args):
            if args == ("rev-parse", "--show-toplevel"):
                return str(source)
            return "?? user-file.txt" if args[0] == "status" else "verified"
        with patch.object(I, "git_command", side_effect=git) as command, self.assertRaisesRegex(I.Rejected, "DIRTY"):
            I.prepare_source(self.bundle, self.source, managed_sample=False)
        self.assertFalse((self.source / "AGENTS.md").exists())
        self.assertFalse(any("commit" in call.args for call in command.call_args_list))

    def test_overlap_audit_failure_blocks_install_and_uses_isolated_codex_home(self):
        argv = ["one-click", "--bundle", str(self.bundle), "--state", str(self.state),
                "--wsl", str(self.wsl), "--codex-config", str(self.codex), "--install"]
        with patch.object(I.sys, "argv", argv), patch.object(I, "preflight", return_value={"status": "READY", "blocked_codes": []}), \
                patch.object(I, "run_json", return_value={"status": "BLOCKED", "code": "AUDIT_FAILED", "exit_code": 2}) as audit, \
                patch.object(I, "run_install") as install, contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(I.main(), 2)
        install.assert_not_called()
        self.assertEqual(json.loads(output.getvalue())["status"], "BLOCKED")
        command = audit.call_args_list[0].args[0]
        self.assertEqual(command[command.index("--codex-home") + 1], str(self.codex.parent))

    def test_no_argument_source_is_separate_sample_not_current_directory(self):
        argv = ["one-click", "--bundle", str(self.bundle), "--state", str(self.state),
                "--wsl", str(self.wsl), "--codex-config", str(self.codex), "--install"]
        with patch.object(I.sys, "argv", argv), patch.object(I, "preflight", return_value={"status": "READY", "blocked_codes": []}), \
                patch.object(I, "run_json", side_effect=[{"status": "CLEAR", "exit_code": 0}, {"status": "READY", "exit_code": 0}, self.staged()]), \
                patch.object(I, "run_install", return_value={"status": "INSTALLED"}) as install, contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(I.main(), 0)
        self.assertEqual(install.call_args.args[2], self.state.with_name("runtime-sample"))
        self.assertEqual(install.call_args.args[0], Path(self.staged()["bundle"]))
        self.assertEqual(install.call_args.kwargs, {"managed_sample": True, "global_hook": False})

    def test_incomplete_state_and_package_source_are_blocked(self):
        self.state.mkdir(parents=True)
        result = I.preflight(self.bundle, self.state, self.bundle, self.wsl)
        self.assertIn("STATE_INCOMPLETE_PRESERVED", result["blocked_codes"])
        self.assertIn("INSTALL_PATHS_OVERLAP", result["blocked_codes"])

    def mcp_fixture(self):
        self.state.mkdir(parents=True)
        (self.state / "installation.json").write_text(json.dumps({"run_id": "a" * 32}))
        submitted = {"status": "SUBMITTED", "task_state": "DRAFT", "task_ref": "test-task", "project_id": "project-id", "objective_digest": "objective"}
        graph = {"commit": "b" * 40, "receipt_digest": "c" * 64}
        replies = [
            [{"runtime_integration": "GRAPHIFY", "graphify_runtime_status": "PREPARED"}, submitted],
            [dict(submitted), {"schema_version": "lattice.code-relations.v1", "registered_project_id": "project-id",
                               "commit": graph["commit"], "source_receipt_digest": graph["receipt_digest"], "records": [{"kind": "NODE"}]}],
        ]
        return graph, replies

    def test_mcp_acceptance_registers_then_reads_task_and_graph_in_second_process(self):
        graph, replies = self.mcp_fixture()
        with patch.object(I, "mcp_batch", side_effect=replies) as rpc:
            result = I.verify_mcp(["python", "runner"], self.bundle, self.state, self.source, {"project_id": "project-id"}, graph, managed_sample=True)
        self.assertEqual(result["status"], "VERIFIED")
        self.assertEqual(result["relation_count"], 1)
        first_calls, second_calls = [call.args[1] for call in rpc.call_args_list]
        self.assertEqual(first_calls[1][0], "lattice_task_submit")
        self.assertEqual(first_calls[1][1]["client_request_id"], "installer-" + "a" * 32)
        self.assertEqual(second_calls[0], ("lattice_task_status", {"task_ref": "test-task"}))
        self.assertEqual(second_calls[1][1]["project_id"], "project-id")

    def test_mcp_acceptance_rejects_empty_graph_or_different_receipt(self):
        graph, replies = self.mcp_fixture()
        for changed in ({"records": []}, {"source_receipt_digest": "wrong"}, {"commit": "wrong"}, {"registered_project_id": "other"}):
            with self.subTest(changed=changed):
                altered = [replies[0], [replies[1][0], {**replies[1][1], **changed}]]
                with patch.object(I, "mcp_batch", side_effect=altered), self.assertRaisesRegex(I.Rejected, "GRAPH_RESTART_READBACK"):
                    I.verify_mcp(["python", "runner"], self.bundle, self.state, self.source, {"project_id": "project-id"}, graph, managed_sample=True)

    def test_fresh_mcp_acceptance_stops_database_before_new_process_readback(self):
        graph, replies = self.mcp_fixture()
        order = []
        def batch(*args):
            order.append("mcp")
            return replies.pop(0)
        def stop(command):
            self.assertEqual(command, ["python", "runner", "stop"])
            order.append("stop")
            return {"status": "STOPPED", "initialized": True, "run_id": "a" * 32, "exit_code": 0}
        with patch.object(I, "mcp_batch", side_effect=batch), patch.object(I, "run_json", side_effect=stop):
            result = I.verify_mcp(["python", "runner"], self.bundle, self.state, self.source,
                                  {"project_id": "project-id"}, graph, managed_sample=True, restart_postgres=True)
        self.assertEqual(order, ["mcp", "stop", "mcp"])
        self.assertEqual(result["postgres_process_restart"], "VERIFIED")

    def test_failed_database_stop_cannot_claim_restart_acceptance(self):
        graph, replies = self.mcp_fixture()
        with patch.object(I, "mcp_batch", return_value=replies[0]) as rpc, \
                patch.object(I, "run_json", return_value={"status": "RUNNING_IDENTITY_VERIFIED", "exit_code": 0}), \
                self.assertRaisesRegex(I.Rejected, "MCP_POSTGRES_STOP_NOT_VERIFIED"):
            I.verify_mcp(["python", "runner"], self.bundle, self.state, self.source,
                         {"project_id": "project-id"}, graph, managed_sample=True, restart_postgres=True)
        self.assertEqual(rpc.call_count, 1)

    def test_mcp_failure_prevents_connect(self):
        with patch.object(I, "prepare_source"), patch.object(I, "run_json", side_effect=self.response), \
                patch.object(I, "verify_mcp", side_effect=I.Rejected("MCP_TASK_RESTART_READBACK_REJECTED")):
            result = I.run_install(self.bundle, self.state, self.source, self.wsl, None, self.codex, "Test")
        self.assertEqual(result["code"], "MCP_TASK_RESTART_READBACK_REJECTED")
        self.assertFalse(any("connect" in args for args in self.calls))

    def test_mcp_batch_checks_initialization_errors_and_structured_output(self):
        initialization = {"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": "2025-11-25"}}
        good = {"jsonrpc": "2.0", "id": 2, "result": {"isError": False, "structuredContent": {"status": "OK"}}}
        output = "\n".join(json.dumps(item) for item in [initialization, good])
        with patch.object(I, "invoke", return_value=subprocess.CompletedProcess([], 0, output, "")) as call:
            self.assertEqual(I.mcp_batch(["runner"], [("lattice_runtime_status", {})]), [{"status": "OK"}])
        self.assertEqual(call.call_args.args[0], ["runner", "serve"])
        requests = [json.loads(line) for line in call.call_args.kwargs["input_text"].splitlines()]
        self.assertEqual(requests[0]["method"], "initialize")
        self.assertEqual(requests[-1]["params"]["arguments"], {})
        good["result"]["isError"] = True
        output = "\n".join(json.dumps(item) for item in [initialization, good])
        with patch.object(I, "invoke", return_value=subprocess.CompletedProcess([], 0, output, "")), self.assertRaisesRegex(I.Rejected, "MCP_TOOL_REJECTED"):
            I.mcp_batch(["runner"], [("lattice_runtime_status", {})])

    def test_reboot_required_preserved_only_for_host_helper(self):
        process = subprocess.CompletedProcess([], 3, '{"status":"REBOOT_REQUIRED","code":"WSL_REBOOT_REQUIRED"}', "")
        with patch.object(I, "invoke", return_value=process):
            self.assertEqual(I.run_json(["host"], allow_reboot=True)["status"], "REBOOT_REQUIRED")
            self.assertEqual(I.run_json(["other"])["status"], "BLOCKED")

    def test_host_failure_messages_are_readable_utf8_and_preserve_exit_codes(self):
        argv = ["one-click", "--bundle", str(self.bundle), "--state", str(self.state), "--wsl", str(self.wsl),
                "--codex-config", str(self.codex), "--install", "--interactive"]
        for code, message, exit_code in (
            ("WSL_CONSENT_DECLINED", "您取消了 WSL 安裝。可再次雙擊 LATTICE 安裝包繼續。", 2),
            ("UAC_CANCELLED", "Windows 未允許啟動 WSL 安裝。請再次執行並確認管理員提示。", 2),
            ("WSL_REBOOT_REQUIRED", "必要功能已安裝。請儲存工作並重新開機，再雙擊 LATTICE 安裝包。", 3),
        ):
            with self.subTest(code=code):
                host = {"status": "REBOOT_REQUIRED" if exit_code == 3 else "BLOCKED",
                        "code": code, "message": message, "exit_code": exit_code}
                replies = [{"status": "CLEAR", "exit_code": 0},
                           {"status": "BLOCKED", "code": "WSL_SETUP_REQUIRED", "exit_code": 2}, host]
                raw = io.BytesIO()
                stderr = io.TextIOWrapper(raw, encoding="ascii")
                self.addCleanup(stderr.close)
                with patch.object(I.sys, "argv", argv), \
                        patch.object(I, "preflight", return_value={"status": "READY", "blocked_codes": []}), \
                        patch.object(I, "run_json", side_effect=replies) as commands, \
                        patch.object(I, "run_install") as install, \
                        contextlib.redirect_stdout(io.StringIO()) as output, contextlib.redirect_stderr(stderr):
                    self.assertEqual(I.main(), exit_code)
                stderr.flush()
                self.assertEqual(raw.getvalue().decode("utf-8").splitlines(), [message])
                self.assertEqual(json.loads(output.getvalue())["wsl_host"], host)
                self.assertIn("ensure", commands.call_args_list[-1].args[0])
                self.assertEqual(commands.call_count, 3)
                install.assert_not_called()

    def test_ready_host_message_does_not_add_failure_progress(self):
        argv = ["one-click", "--bundle", str(self.bundle), "--state", str(self.state), "--wsl", str(self.wsl),
                "--codex-config", str(self.codex), "--install"]
        message = "WSL 主機檢查通過；接著安裝並驗證 LATTICE 專用環境。"
        replies = [{"status": "CLEAR", "exit_code": 0},
                   {"status": "READY", "message": message, "exit_code": 0}, self.staged()]
        with patch.object(I.sys, "argv", argv), \
                patch.object(I, "preflight", return_value={"status": "READY", "blocked_codes": []}), \
                patch.object(I, "run_json", side_effect=replies), \
                patch.object(I, "run_install", return_value={"status": "INSTALLED"}), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()) as stderr:
            self.assertEqual(I.main(), 0)
        self.assertNotIn(message, stderr.getvalue())

    def test_unattended_host_setup_never_requests_uac(self):
        argv = ["one-click", "--bundle", str(self.bundle), "--state", str(self.state), "--wsl", str(self.wsl),
                "--codex-config", str(self.codex), "--install"]
        with patch.object(I.sys, "argv", argv), patch.object(I, "preflight", return_value={"status": "READY", "blocked_codes": []}), \
                patch.object(I, "run_json", side_effect=[{"status": "CLEAR", "exit_code": 0}, {"status": "BLOCKED", "code": "WSL_SETUP_REQUIRED", "exit_code": 2}]) as commands, \
                patch.object(I, "run_install") as install, contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(I.main(), 2)
        self.assertFalse(any("ensure" in call.args[0] for call in commands.call_args_list))
        install.assert_not_called()

    def test_interactive_host_install_reports_reboot_and_does_not_start_runtime(self):
        argv = ["one-click", "--bundle", str(self.bundle), "--state", str(self.state), "--wsl", str(self.wsl),
                "--codex-config", str(self.codex), "--install", "--interactive"]
        replies = [{"status": "CLEAR", "exit_code": 0}, {"status": "BLOCKED", "code": "WSL_SETUP_REQUIRED", "exit_code": 2},
                   {"status": "REBOOT_REQUIRED", "code": "WSL_REBOOT_REQUIRED", "exit_code": 3}]
        with patch.object(I.sys, "argv", argv), patch.object(I, "preflight", return_value={"status": "READY", "blocked_codes": []}), \
                patch.object(I, "run_json", side_effect=replies) as commands, patch.object(I, "run_install") as install, contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(I.main(), 3)
        self.assertEqual(json.loads(output.getvalue())["status"], "REBOOT_REQUIRED")
        self.assertIn("ensure", commands.call_args_list[2].args[0])
        install.assert_not_called()

    def test_warning_consent_preserves_candidates_and_can_decline(self):
        argv = ["one-click", "--bundle", str(self.bundle), "--state", str(self.state), "--wsl", str(self.wsl),
                "--codex-config", str(self.codex), "--install", "--interactive"]
        for accept in (False, True):
            with self.subTest(accept=accept), patch.object(I.sys, "argv", argv), \
                    patch.object(I, "preflight", return_value={"status": "READY", "blocked_codes": []}), \
                    patch.object(I, "run_json", side_effect=[{"status": "WARNING", "exit_code": 0, "user_acknowledged": accept,
                                                             "resolution": [{"status": "KEPT"}]},
                                                            {"status": "READY", "exit_code": 0}, self.staged()]) as commands, \
                    patch.object(I, "run_install", return_value={"status": "INSTALLED"}) as install, contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(I.main(), 0 if accept else 2)
            result = json.loads(output.getvalue())
            if accept:
                self.assertEqual(result["overlap_audit"]["resolution"], [{"status": "KEPT"}])
                install.assert_called_once()
            else:
                self.assertEqual(result["status"], "WARNING_REVIEW_REQUIRED")
                install.assert_not_called()
            self.assertIn("--review", commands.call_args_list[0].args[0])
            self.assertIn("--interactive", commands.call_args_list[0].args[0])

    def test_missing_codex_home_is_reported_before_expensive_installation(self):
        argv = ["one-click", "--bundle", str(self.bundle), "--state", str(self.state), "--wsl", str(self.wsl),
                "--codex-config", str(self.root / "not-started-codex/config.toml"), "--install"]
        with patch.object(I.sys, "argv", argv), patch.object(I, "preflight", return_value={"status": "READY", "blocked_codes": []}), \
                patch.object(I, "run_json", side_effect=[{"status": "CLEAR", "exit_code": 0}, {"status": "READY", "exit_code": 0}]) as commands, \
                patch.object(I, "run_install") as install, contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(I.main(), 2)
        self.assertIn("CODEX_FIRST_RUN_REQUIRED", json.loads(output.getvalue())["blocked_codes"])
        install.assert_not_called()
        self.assertEqual(commands.call_count, 2)

    def test_staging_failure_prevents_installation(self):
        argv = ["one-click", "--bundle", str(self.bundle), "--state", str(self.state), "--wsl", str(self.wsl),
                "--codex-config", str(self.codex), "--install"]
        replies = [{"status": "CLEAR", "exit_code": 0}, {"status": "READY", "exit_code": 0},
                   {"status": "BLOCKED", "exit_code": 2, "code": "STAGED_BUNDLE_CHANGED"}]
        with patch.object(I.sys, "argv", argv), patch.object(I, "preflight", return_value={"status": "READY", "blocked_codes": []}), \
                patch.object(I, "run_json", side_effect=replies), patch.object(I, "run_install") as install, contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(I.main(), 2)
        install.assert_not_called()
        self.assertIn("STAGED_BUNDLE_CHANGED", json.loads(output.getvalue())["blocked_codes"])

    def test_global_hook_appends_once_and_preserves_unrelated_guidance_with_backup(self):
        path = self.codex.parent / "AGENTS.md"
        original = b"# User rules\r\nKeep my work unchanged.\r\n"
        path.write_bytes(original)
        result = I.install_global_hook(self.codex.parent)
        self.assertEqual(result["status"], "INSTALLED")
        self.assertEqual(Path(result["backup"]).read_bytes(), original)
        self.assertTrue(path.read_bytes().startswith(original))
        self.assertIn("Hermes", path.read_text(encoding="utf-8"))
        self.assertEqual(I.install_global_hook(self.codex.parent)["status"], "UNCHANGED")
        self.assertEqual(len(list(self.codex.parent.glob("AGENTS.md.lattice-backup-*"))), 1)

    def test_existing_global_startup_rule_is_reused_without_duplicate_or_backup(self):
        path = self.codex.parent / "AGENTS.md"
        original = b"Before each task, call `lattice_runtime_status`.\n"
        path.write_bytes(original)
        self.assertEqual(I.install_global_hook(self.codex.parent)["status"], "REUSED")
        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(list(self.codex.parent.glob("AGENTS.md.lattice-backup-*")), [])

    def test_malformed_global_managed_block_is_preserved(self):
        path = self.codex.parent / "AGENTS.md"
        original = b"<!-- BEGIN LATTICE GLOBAL STARTUP v1 -->\nuser edits"
        path.write_bytes(original)
        with self.assertRaisesRegex(I.Rejected, "GLOBAL_HOOK_CONFLICT_PRESERVED"):
            I.install_global_hook(self.codex.parent)
        self.assertEqual(path.read_bytes(), original)

    def test_project_connection_is_installed_without_duplicating_existing_startup(self):
        path = self.codex.parent / "AGENTS.md"
        original = b"Before each task, call `lattice_runtime_status`.\n"
        path.write_bytes(original)
        result = I.install_global_hook(self.codex.parent, state=self.state, bundle=self.bundle)
        contents = path.read_text(encoding="utf-8")
        self.assertEqual(result["status"], "INSTALLED")
        self.assertTrue(path.read_bytes().startswith(original))
        self.assertEqual(contents.count("lattice_runtime_status"), 1)
        lines = [json.loads(line) for line in contents.splitlines() if line.startswith("[")]
        self.assertEqual(len(lines), 2)
        self.assertEqual(lines[0][4], str(self.state / "bin/lattice-customer-runtime.py"))
        self.assertEqual(lines[0][5], "register-project")
        self.assertEqual(lines[1][5], "graphify-refresh")
        self.assertIn("--project-id", lines[1])
        self.assertEqual(I.install_global_hook(self.codex.parent, state=self.state, bundle=self.bundle)["status"], "REUSED")
        self.assertEqual(len(list(self.codex.parent.glob("AGENTS.md.lattice-backup-*"))), 1)

    def test_invalid_project_connection_block_preserves_entire_file(self):
        path = self.codex.parent / "AGENTS.md"
        original = b"custom rule\n<!-- BEGIN LATTICE PROJECT CONNECTION v1 -->\n"
        path.write_bytes(original)
        with self.assertRaisesRegex(I.Rejected, "GLOBAL_PROJECT_HOOK_CONFLICT_PRESERVED"):
            I.install_global_hook(self.codex.parent, state=self.state, bundle=self.bundle)
        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(list(self.codex.parent.glob("AGENTS.md.lattice-backup-*")), [])

    def test_project_hook_uses_upgraded_launcher_and_rejects_foreign_path(self):
        self.state.mkdir(parents=True)
        launcher = self.state / "updates" / ("a" * 32) / "lattice-customer-runtime.py"
        installation = self.state / "installation.json"
        installation.write_text(json.dumps({"launcher": str(launcher)}), encoding="utf-8")
        I.install_global_hook(self.codex.parent, state=self.state, bundle=self.bundle)
        path = self.codex.parent / "AGENTS.md"
        contents = path.read_bytes()
        commands = [json.loads(line) for line in contents.decode("utf-8").splitlines() if line.startswith("[")]
        self.assertEqual([command[4] for command in commands], [str(launcher)] * 2)
        installation.write_text(json.dumps({"launcher": str(self.root / "foreign/lattice-customer-runtime.py")}), encoding="utf-8")
        with self.assertRaisesRegex(I.Rejected, "GLOBAL_PROJECT_LAUNCHER_REJECTED"):
            I.install_global_hook(self.codex.parent, state=self.state, bundle=self.bundle)
        self.assertEqual(path.read_bytes(), contents)

    def test_global_hook_only_runs_after_successful_mcp_and_connect(self):
        with patch.object(I, "prepare_source"), patch.object(I, "verify_mcp", return_value={"status": "VERIFIED"}), \
                patch.object(I, "run_json", side_effect=self.response), patch.object(I, "install_global_hook", return_value={"status": "INSTALLED"}) as hook:
            result = I.run_install(self.bundle, self.state, self.source, self.wsl, None, self.codex, "Test", global_hook=True)
        hook.assert_called_once_with(self.codex.parent, state=self.state, bundle=self.bundle)
        self.assertEqual([step["action"] for step in result["post_install_steps"]][-2:], ["connect", "global-startup-hook"])


if __name__ == "__main__":
    unittest.main()
