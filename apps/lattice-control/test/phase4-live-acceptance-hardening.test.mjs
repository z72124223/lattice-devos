import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const root = new URL("../../../", import.meta.url);
const readRepositoryFile = (relative) => readFile(new URL(relative, root), "utf8");
const execFileAsync = promisify(execFile);

function quotedValue(source, name) {
  const match = source.match(new RegExp(`^\\s*${name}\\s*=\\s*"([^"]+)"\\s*$`, "mu"));
  assert.ok(match, `missing ${name}`);
  return match[1];
}

function quotedList(source, name) {
  const match = source.match(new RegExp(`^\\s*${name}\\s*=\\s*\\[([^\\]]*)\\]\\s*$`, "mu"));
  assert.ok(match, `missing ${name}`);
  return [...match[1].matchAll(/"([A-Za-z_][A-Za-z0-9_]*)"/gu)].map((entry) => entry[1]);
}

test("WSL2 Codex task shell preserves required non-secret paths without exposing credential homes", async () => {
  const config = await readRepositoryFile("apps/lattice-control/config/wsl2-codex-config.toml");
  const inherit = quotedValue(config, "inherit");
  const includeOnly = quotedList(config, "include_only");
  const outer = {
    HOME: "/home/zk/lattice-phase4/home",
    CODEX_HOME: "/home/zk/lattice-phase4/codex-home",
    PATH: "/usr/bin:/bin",
    LANG: "C.UTF-8",
    LC_ALL: "C.UTF-8",
    LATTICE_FAKE_API_TOKEN: "must-not-cross",
  };
  const inherited = inherit === "all" ? outer : {};
  const effective = Object.fromEntries(Object.entries(inherited)
    .filter(([name]) => includeOnly.includes(name)));

  assert.equal(inherit, "all");
  assert.equal(effective.HOME, outer.HOME);
  assert.equal(effective.PATH, outer.PATH);
  assert.equal(effective.LANG, outer.LANG);
  assert.equal(effective.LC_ALL, outer.LC_ALL);
  assert.equal("CODEX_HOME" in effective, false);
  assert.equal("LATTICE_FAKE_API_TOKEN" in effective, false);
});

test("failure receipt persistence is create-new, digest-bound, and substitution resistant", async () => {
  const script = fileURLToPath(new URL("scripts/test-phase4-managed-foreman.ps1", root));
  const { stdout, stderr } = await execFileAsync(
    process.platform === "win32" ? "pwsh.exe" : "pwsh",
    ["-NoLogo", "-NoProfile", "-File", script, "-StaticReceiptPersistenceSelfTestOnly"],
    { windowsHide: true, maxBuffer: 1024 * 1024 },
  );
  assert.equal(stderr.trim(), "");
  const receipt = JSON.parse(stdout.trim());
  assert.deepEqual(receipt, {
    schema: "lattice.phase4-failure-receipt-persistence-selftest.v1",
    status: "PASS",
    create_new: true,
    overwrite_rejected: true,
    digest_verified: true,
    digest_failure_preserved_receipt: true,
  });
});

test("late MCP status response contaminates the session and cannot be reused", async () => {
  const script = fileURLToPath(new URL("scripts/test-phase4-managed-foreman.ps1", root));
  const { stdout, stderr } = await execFileAsync(
    process.platform === "win32" ? "pwsh.exe" : "pwsh",
    ["-NoLogo", "-NoProfile", "-File", script, "-StaticMcpPollingSelfTestOnly"],
    { windowsHide: true, maxBuffer: 1024 * 1024 },
  );
  assert.equal(stderr.trim(), "");
  const receipt = JSON.parse(stdout.trim());
  assert.deepEqual(receipt, {
    schema: "lattice.phase4-mcp-status-timeout-selftest.v1",
    status: "PASS",
    typed_timeout: true,
    diagnostic_bound: true,
    late_response_rejected: true,
    session_reuse_rejected: true,
    extra_tool_calls: 0,
  });
});

test("managed foreman binds reviewed WSL bytes and never writes the credential home", async () => {
  const source = await readRepositoryFile("scripts/test-phase4-managed-foreman.ps1");
  const ownedProcess = await readRepositoryFile("scripts/phase4-owned-process.ps1");
  const materializerSource = await readRepositoryFile(
    "scripts/materialize-phase4-wsl2-live-environment.mjs",
  );
  const preflightSource = await readRepositoryFile(
    "apps/lattice-control/src/wsl2-execution-preflight.mjs",
  );
  for (const required of [
    "$script:Wsl2SupervisorSource",
    "$wslSupervisorSourceSha256",
    "--expected-supervisor-sha256",
    "ExpectedSupervisorSha256",
    "descriptor.linux.supervisor_path",
    "descriptor.linux.supervisor_sha256",
    "function Invoke-Phase4WslFailureSubtreeCleanup",
    "function Close-Phase4WslTaskOwnedUnits",
    "'(?:preflight|provider|reviewer)'",
    "$unitPrefix + '-preflight-*.service'",
    "$unitPrefix + '-provider-*.service'",
    "$unitPrefix + '-reviewer-*.service'",
    "--signal=SIGTERM",
    "--signal=SIGKILL",
    "'stop', '--force'",
    "function Get-Phase4CanonicalUserServiceCgroupPath",
    "@('--no-block', 'stop')",
    "$settleDeadline = [DateTimeOffset]::UtcNow.AddSeconds(10)",
    "phase4-owned-process.ps1",
    "Start-Phase4OwnedProcessJob",
    "$script:WindowsOemCodePage = (Get-Culture).TextInfo.OEMCodePage",
    "-OutputEncodingCodePage (Get-Culture).TextInfo.OEMCodePage",
    "Stop-Phase4OwnedProcessJob",
    "WriteStandardInput",
    "ReadStandardOutputToEndBounded",
    "ReadStandardErrorToEndBounded",
    "ReadStandardOutputLineBounded",
    "ContainsProcessHandle",
    "$script:WslOpenCommandUnits",
    "Close-Phase4WslOpenCommandUnits",
    "$script:LastPhase4ProcessTreeTerminationProven",
    "$script:postgresLauncherTerminalProven",
    "function Get-Phase4OwnedPostgresProcessRecord",
    "$script:postgresProcessIdentity",
    "$processStart.ToUnixTimeSeconds()",
    "Assert-Phase4OwnedLivePostgres",
    "$wslFinalDurableEvidence = Get-Phase4WslDurableEvidence",
    "durable_provider_effect_status",
    "real_codex_attempt_evidence",
    "reconciliation_required",
    "foreman_process_tree_stopped",
    "failure_subtree_cleanup",
    "function Write-Phase4AtomicCreateNewUtf8File",
    "[IO.FileMode]::CreateNew",
    "[IO.File]::Move($temporaryPath, $finalPath, $false)",
    "final-failure-receipt.json",
    "final-failure-receipt.sha256",
  ]) {
    assert.equal(source.includes(required), true, `missing ${required}`);
  }
  for (const forbidden of [
    "function Sync-Phase4WslCodexConfig",
    "LATTICE_PHASE4_TARGET_CONFIG_B64",
    "PHASE4_WSL2_CODEX_CONFIG_SYNCHRONIZATION",
    "allowed_current_sha256",
    "active_codex_home_process_count",
  ]) {
    assert.equal(source.includes(forbidden), false, `credential-home mutation remains: ${forbidden}`);
  }
  const materialization = source.slice(
    source.lastIndexOf("$failureStage = 'WSL2_BOOTSTRAP_MATERIALIZATION'"),
    source.lastIndexOf("$failureStage = 'WSL2_SUBSTITUTION_GATES'"),
  );
  assert.equal((materialization.match(/ExpectedWsl2CodexConfigSha256/gu) ?? []).length, 2);
  assert.equal((materialization.match(/ExpectedSupervisorSha256/gu) ?? []).length, 2);
  assert.ok(materialization.indexOf("$bootstrapMaterialization.descriptor.linux.config_digest")
    < materialization.indexOf("$finalMaterialization.descriptor.linux.config_digest"));
  assert.equal((materialization.match(/\/runtime-v4\/wsl2-codex-supervisor\.mjs/gu) ?? []).length, 2);
  assert.equal((materializerSource.match(/\/runtime-v4/gu) ?? []).length, 3);
  assert.equal(materializerSource.includes("/runtime-v3"), false);
  assert.equal(preflightSource.includes("--property=TimeoutStopSec=5s"), true);
  assert.equal(preflightSource.includes("[\"--user\", \"--no-block\", \"stop\", launch.unit]"),
    true);
  assert.equal(preflightSource.includes("record.cgroup_path === canonicalCgroup"), true);
  const reviewedGate = materializerSource.indexOf("assertReviewedSupervisorDigest(");
  assert.ok(reviewedGate > 0);
  assert.ok(reviewedGate < materializerSource.indexOf("configDigestResult"));
  assert.ok(reviewedGate < materializerSource.indexOf("preflightWsl2ExecutionEnvironment(environment, context)"));
  const failureReceipt = source.slice(source.indexOf("status = 'FAIL'"));
  const durableCount = failureReceipt.slice(
    failureReceipt.indexOf("durable_provider_effect_count"),
    failureReceipt.indexOf("provider_fence"),
  );
  assert.equal(/else\s*\{\s*0\s*\}/u.test(durableCount), false);
  const observed = source.slice(source.indexOf("last_observed_provider_effect_count"));
  assert.ok(observed.indexOf("$wslFinalDurableEvidenceBeforeCleanup")
    < observed.indexOf("$wslEnvironmentAfter"));
  assert.ok(observed.indexOf("$wslEnvironmentAfter") < observed.indexOf("$wslEnvironmentBefore"));
  assert.ok(observed.indexOf("$wslEnvironmentBefore") < observed.indexOf("$wslProviderEffectsAfterFence"));
  const activeStartBegin = source.indexOf("$failureStage = 'WSL2_ACTIVE_ACCEPTED_START'");
  const activeStart = source.slice(
    activeStartBegin,
    source.indexOf("$failureStage = 'WSL2_PROVIDER_FENCE_CAPTURE'", activeStartBegin),
  );
  const statusCall = activeStart.indexOf("$candidate = Invoke-Phase4McpStatusForGate");
  const retainedStatus = activeStart.indexOf("$lastManagedStatus = $candidate");
  const blockedStatus = activeStart.indexOf("$candidateBlocked = [string]$candidate.task_state -in");
  assert.ok(statusCall >= 0 && statusCall < retainedStatus);
  assert.ok(retainedStatus < blockedStatus);
  assert.equal(activeStart.includes(
    "$transientApprovalGate = Test-Phase4TransientActiveApprovalGate -Status $candidate",
  ), true);
  assert.equal(activeStart.includes(
    "$availableActiveStatusCalls = [long]$script:MaximumMcpToolCalls -",
  ), true);
  assert.equal(activeStart.includes("$activeStartPollOrigin = [DateTimeOffset]::UtcNow"), true);
  assert.equal(source.includes(
    "$script:MinimumMcpStatusResponseBudgetMilliseconds = 5000",
  ), true);
  assert.equal(source.includes("$script:McpStatusResponseCleanupGraceSeconds"), false);
  assert.equal(activeStart.includes("[long]$ProcessTimeoutSeconds * 2"), true);
  assert.equal(activeStart.includes(
    "[long][Math]::Min([long]$ProcessTimeoutSeconds, 180) + 120",
  ), true);
  assert.equal(activeStart.includes(
    "$activeStartFinalPollLeadMilliseconds = "
      + "$script:MinimumMcpStatusResponseBudgetMilliseconds",
  ), true);
  assert.equal(activeStart.includes("$activeStartStatusResponseTimeoutSeconds ="), true);
  assert.equal(activeStart.includes(
    "[int][Math]::Min(900, [long]$activeStartWindowSeconds)",
  ), true);
  assert.equal(activeStart.includes("$activeStartPollDelayMilliseconds ="), true);
  assert.equal(activeStart.includes("([double]$availableActiveStatusCalls - 1.0)"), true);
  assert.equal(activeStart.includes("$poll -lt $availableActiveStatusCalls"), true);
  assert.equal(activeStart.includes("$remainingActiveStartMilliseconds = [long][Math]::Ceiling("),
    true);
  assert.equal(activeStart.includes(
    "-TimeoutMilliseconds ([int][Math]::Min(",
  ), true);
  assert.equal(activeStart.includes(
    "-TimeoutSeconds $activeStartStatusResponseTimeoutSeconds",
  ), true);
  assert.equal(activeStart.includes(
    "-TimeoutCode 'PHASE4_WSL2_ACTIVE_STATUS_RESPONSE_TIMEOUT'",
  ), true);
  assert.ok(activeStart.indexOf("$lastManagedStatus = $candidate")
    < activeStart.indexOf("if ([DateTimeOffset]::UtcNow -ge $deadline) { break }"));
  assert.equal((activeStart.match(/if \(\[DateTimeOffset\]::UtcNow -ge \$deadline\) \{ break \}/gu)
    ?? []).length >= 2, true);
  assert.equal(activeStart.includes("$activeStartNextPollAt = $activeStartPollOrigin.AddMilliseconds("),
    true);
  assert.equal(activeStart.includes(
    "Start-Sleep -Milliseconds ([int]$activeStartSleepMilliseconds)",
  ), true);
  assert.equal(activeStart.includes("if ($poll -ge 600)"), false);
  assert.equal(activeStart.includes("Start-Sleep -Milliseconds 100"), false);
  assert.equal(activeStart.includes("if ($candidateBlocked -and -not $transientApprovalGate)"), true);
  assert.equal(activeStart.includes("$managedFailureCode = [string]$candidate.failure_code"), true);
  assert.equal(activeStart.includes("throw $managedFailureCode"), true);
  const activeStatusAssertion = activeStart.indexOf(
    "Assert-Phase4ActiveManagedStatus -Status $candidate -ExpectedTaskRef $taskRef",
  );
  assert.ok(activeStatusAssertion >= 0
    && activeStatusAssertion < activeStart.indexOf("$candidateEvidence = Get-Phase4ActiveRestartEvidence"));
  assert.equal(activeStart.includes(
    "[long]$firstActiveStatus.foreman_generation -ne [long]$checkpoint.generation",
  ), true);
  assert.equal(activeStart.includes(
    "[string]$firstActiveStatus.foreman_checkpoint_digest -cne",
  ), true);
  assert.equal(activeStart.includes("PHASE4_WSL2_FOREMAN_CHECKPOINT_A_REJECTED"), true);
  const missedWindow = activeStart.slice(activeStart.indexOf("if ($null -eq $firstActiveStatus"));
  assert.ok(missedWindow.indexOf("$managedFailureCode = [string]$lastManagedStatus.failure_code")
    < missedWindow.indexOf("throw 'PHASE4_WSL2_ACTIVE_RESTART_WINDOW_MISSED'"));
  assert.ok(missedWindow.indexOf("throw $managedFailureCode")
    < missedWindow.indexOf("throw 'PHASE4_WSL2_ACTIVE_RESTART_WINDOW_MISSED'"));
  const transientGate = source.slice(
    source.indexOf("function Test-Phase4TransientActiveApprovalGate"),
    source.indexOf("function Assert-Phase4ActiveManagedStatus"),
  );
  for (const required of [
    "[string]$Status.status -ceq 'BLOCKED'",
    "[string]$Status.task_state -ceq 'AWAITING_EXECUTION_APPROVAL'",
    "[string]$Status.failure_code -ceq 'LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED'",
    "$null -eq $Status.attempt",
    "-not [bool]$Status.worker_running",
    "$null -eq $Status.thread_id",
    "$null -eq $Status.turn_id",
  ]) {
    assert.equal(transientGate.includes(required), true, `missing transient gate: ${required}`);
  }
  assert.equal(source.includes("PHASE4_STATIC_ACTIVE_APPROVAL_GATE_REJECTED"), true);
  assert.equal(source.includes("PHASE4_STATIC_ACTIVE_STATUS_CALL_BUDGET_REJECTED"), true);
  assert.equal(source.includes("$script:MaximumMcpToolCalls = 56"), true);
  assert.equal(source.includes(
    "$Session.tool_call_count -ge $script:MaximumMcpToolCalls",
  ), true);
  const reconnectBegin = source.indexOf("$failureStage = 'WSL2_ACTIVE_RECONNECT'");
  const reconnect = source.slice(
    reconnectBegin,
    source.indexOf("$failureStage = 'MANAGED_POLL'", reconnectBegin),
  );
  for (const required of [
    "$reservedTerminalStatusCalls = [long]$script:MaximumMcpStatusPolls",
    "$availableReconnectStatusCalls = [long]$script:MaximumMcpToolCalls -",
    "$reconnectPollOrigin = [DateTimeOffset]::UtcNow",
    "$reconnectFinalPollLeadMilliseconds = "
      + "$script:MinimumMcpStatusResponseBudgetMilliseconds",
    "$reconnectPollDelayMilliseconds =",
    "$reconnectStatusResponseTimeoutSeconds = "
      + "[int][Math]::Min(900, [long]$reconnectWindowSeconds)",
    "([double]$availableReconnectStatusCalls - 1.0)",
    "$poll -lt $availableReconnectStatusCalls",
    "$remainingReconnectMilliseconds = [long][Math]::Ceiling(",
    "-TimeoutMilliseconds ([int][Math]::Min(",
    "-TimeoutSeconds $reconnectStatusResponseTimeoutSeconds",
    "-TimeoutCode 'PHASE4_WSL2_RECONNECT_STATUS_RESPONSE_TIMEOUT'",
    "$reconnectNextPollAt = $reconnectPollOrigin.AddMilliseconds(",
    "$remainingReconnectMilliseconds = [long][Math]::Ceiling(",
  ]) {
    assert.equal(reconnect.includes(required), true, `missing reconnect budget guard: ${required}`);
  }
  assert.ok(reconnect.indexOf("$candidate = Invoke-Phase4McpStatusForGate")
    < reconnect.indexOf("if ([DateTimeOffset]::UtcNow -ge $deadline) { break }"));
  assert.equal((reconnect.match(/if \(\[DateTimeOffset\]::UtcNow -ge \$deadline\) \{ break \}/gu)
    ?? []).length >= 2, true);
  for (const required of [
    "[string]$candidate.task_state -ceq 'EXECUTING'",
    "[string]$candidate.status -ceq 'RUNNING'",
    "[bool]$candidate.worker_running",
    "Assert-Phase4ActiveManagedStatus -Status $candidate -ExpectedTaskRef $taskRef",
    "[string]$candidate.thread_id -ceq [string]$before.thread_id",
    "[string]$candidate.turn_id -ceq [string]$before.turn_id",
    "[string]$value.thread_id -ceq [string]$candidate.thread_id",
    "[string]$value.turn_id -ceq [string]$candidate.turn_id",
    "[string]$value.writer_status -ceq 'ACTIVE'",
    "[string]$value.writer_attempt_id -ceq [string]$value.attempt_id",
    "[long]$value.writer_current_fence -eq [long]$value.writer_fence",
    "[long]$value.writer_process_id -eq [long]$wslReconnectForeman.process_id",
    "PHASE4_WSL2_RECONNECT_FOREMAN_CHECKPOINT_REJECTED",
  ]) {
    assert.equal(reconnect.includes(required), true, `missing reconnect active binding: ${required}`);
  }
  const reconnectStatusAssertion = reconnect.indexOf(
    "Assert-Phase4ActiveManagedStatus -Status $candidate -ExpectedTaskRef $taskRef",
  );
  assert.ok(reconnectStatusAssertion >= 0
    && reconnectStatusAssertion < reconnect.indexOf("$candidateEvidence = Get-Phase4ActiveRestartEvidence"));
  const terminalPoll = source.slice(source.indexOf("$failureStage = 'MANAGED_POLL'"));
  assert.equal(terminalPoll.includes(
    "$availableTerminalStatusCalls = [long]$script:MaximumMcpToolCalls -",
  ), true);
  assert.equal(terminalPoll.includes(
    "if ($Wsl2LinuxLive -and $availableTerminalStatusCalls -lt",
  ), true);
  for (const required of [
    "$poll -lt [long]$script:MaximumMcpStatusPolls",
    "$terminalPollOrigin = [DateTimeOffset]::UtcNow",
    "$terminalFinalPollLeadMilliseconds = "
      + "$script:MinimumMcpStatusResponseBudgetMilliseconds",
    "$terminalPollDelayMilliseconds =",
    "$terminalStatusResponseTimeoutSeconds = "
      + "[int][Math]::Min(900, [long]$AcceptanceTimeoutSeconds)",
    "([double]$script:MaximumMcpStatusPolls - 1.0)",
    "$remainingTerminalMilliseconds = [long][Math]::Ceiling(",
    "-TimeoutMilliseconds ([int][Math]::Min(",
    "-TimeoutSeconds $terminalStatusResponseTimeoutSeconds",
    "-TimeoutCode 'PHASE4_MANAGED_TERMINAL_STATUS_RESPONSE_TIMEOUT'",
    "$terminalResponseWithinDeadline = [DateTimeOffset]::UtcNow -lt $deadline",
    "if (-not $terminalResponseWithinDeadline) { break }",
    "$terminalNextPollAt = $terminalPollOrigin.AddMilliseconds(",
    "Start-Sleep -Milliseconds ([int]$terminalSleepMilliseconds)",
  ]) {
    assert.equal(terminalPoll.includes(required), true, `missing terminal deadline guard: ${required}`);
  }
  const mcpTool = source.slice(
    source.indexOf("function Invoke-Phase4McpTool"),
    source.indexOf("function Invoke-Phase4FormalForemanCheckpoint"),
  );
  assert.equal(mcpTool.includes("[ValidateRange(0, 900000)][int]$TimeoutMilliseconds = 0"), true);
  assert.equal(mcpTool.includes("-TimeoutMilliseconds $TimeoutMilliseconds"), true);
  assert.equal(mcpTool.includes("PHASE4_MCP_SESSION_CONTAMINATED"), true);
  const statusGate = source.slice(
    source.indexOf("function Invoke-Phase4McpStatusForGate"),
    source.indexOf("function Test-Phase4TransientActiveApprovalGate", source.indexOf(
      "function Invoke-Phase4McpStatusForGate",
    )),
  );
  for (const required of [
    "$Session.response_contaminated = $true",
    "$TimeoutDiagnostic.Value = [ordered]@{",
    "request_id = [long]$Session.next_id - 1",
    "poll_ordinal = [long]$PollOrdinal",
    "remaining_at_dispatch_milliseconds = $RemainingAtDispatchMilliseconds",
    "configured_response_timeout_seconds = $TimeoutSeconds",
    "effective_response_timeout_milliseconds = [long][Math]::Min(",
    "last_completed_candidate = Get-Phase4ManagedStatusDiagnostic -Status $LastCompletedStatus",
    "throw $TimeoutCode",
  ]) {
    assert.equal(statusGate.includes(required), true, `missing status timeout gate: ${required}`);
  }
  const persistedFailure = source.slice(source.indexOf("$failureReceipt = [ordered]@{"));
  assert.equal(persistedFailure.includes("mcp_status_timeout = $mcpStatusTimeoutDiagnostic"), true);
  assert.equal(persistedFailure.includes(
    "failure_receipt_persistence = $failureReceiptPersistence",
  ), true);
  assert.equal(persistedFailure.includes("Write-Phase4AtomicCreateNewUtf8File"), true);
  assert.equal(persistedFailure.includes("Get-Phase4FileSha256 -Path $failureReceiptPath"), true);
  assert.equal(source.includes(
    "Get-Phase4OwnerMarker -RunRoot $runRoot -RunId $runId -MarkerPath $markerPath",
  ), true);
  assert.equal(source.includes(
    "Assert-Phase4ContainedPath -Root $runRoot",
  ), true);
  assert.equal(source.includes("$failureReceiptPersistence.receipt_status = 'FAILED'"), true);
  assert.equal(source.includes("PHASE4_FAILURE_RECEIPT_PERSISTENCE_REJECTED"), true);
  assert.equal(source.includes("PHASE4_FAILURE_RECEIPT_DIGEST_PERSISTENCE_REJECTED"), true);
  assert.equal(source.includes("digest_status = 'PASS'"), false);
  assert.equal(source.includes("PHASE4_STATIC_RECONNECT_STATUS_CALL_BUDGET_REJECTED"), true);
  const fallback = source.slice(
    source.indexOf("if ($null -eq $wslFinalDurableEvidence)"),
    source.indexOf("if ($null -eq $failureCode)", source.indexOf("if ($null -eq $wslFinalDurableEvidence)")),
  );
  assert.ok(fallback.indexOf("Assert-Phase4OwnedLivePostgres")
    < fallback.indexOf("Get-Phase4WslDurableEvidence"));
  assert.equal((source.match(/read_managed_evidence_v1\(\s*decode\('\$TaskRef','hex'\),\s*1::smallint\s*\)/gu)
    ?? []).length, 5);
  assert.equal(source.includes("read_managed_evidence_v1(decode('$TaskRef','hex'),1)"), false);
  assert.equal(source.includes("pg_catalog.coalesce("), false);
  assert.equal((source.match(/\bCOALESCE\(/gu) ?? []).length >= 5, true);
  assert.equal(source.includes("$script:postgresStartMayOwnProcess = $true"), true);
  assert.equal(source.includes("$script:postgresStartMayOwnProcess = $false"), true);
  const finalCleanup = source.slice(source.lastIndexOf("finally {"));
  assert.equal(finalCleanup.includes("$script:postgresStartMayOwnProcess"), true);
  assert.match(finalCleanup,
    /if \(\$runRootCreated -and \$null -ne \$postgresPort -and\s+\$script:postgresStartMayOwnProcess\)/u);
  assert.equal((source.match(/Get-Phase4CanonicalUserServiceCgroupPath/gu) ?? []).length >= 6, true);
  assert.equal(source.includes("'open_marker_count'"), true);
  assert.equal(source.includes("$value.open_marker_count -ne [long]$value.count"), true);
  assert.equal(source.includes("closed.payload->>'status'='CLOSED'"), true);
  assert.equal(source.includes("closed.payload->>'status'='RECONCILED'"), true);
  assert.equal((source.match(/\$settleDeadline = \[DateTimeOffset\]::UtcNow\.AddSeconds\(10\)/gu)
    ?? []).length, 2);
  assert.equal(source.includes("PHASE4_WSL2_FAILURE_SUBTREE_RECONCILIATION_REQUIRED"), true);
  assert.equal(source.includes("status = $(if ($durableReconciliationRequired) { 'PHYSICAL_ONLY' }"),
    true);
  const processTimeout = source.slice(source.indexOf("function Invoke-Phase4Process"),
    source.indexOf("function ConvertTo-Phase4WslUncPath"));
  assert.equal(processTimeout.includes("Start-Phase4OwnedProcessJob"), true);
  assert.equal(processTimeout.includes("Stop-Phase4OwnedProcessJob"), true);
  assert.equal(processTimeout.includes("try { $process.Kill($true) } catch {}"), false);
  assert.equal(source.includes("Get-CimInstance Win32_Process"), false);
  for (const required of [
    "CREATE_SUSPENDED",
    "CREATE_UNICODE_ENVIRONMENT",
    "EXTENDED_STARTUPINFO_PRESENT",
    "PROC_THREAD_ATTRIBUTE_HANDLE_LIST",
    "JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE",
    "AssignProcessToJobObject",
    "ResumeThread",
    "TerminateJobObject",
    "ActiveProcessCount",
  ]) {
    assert.equal(ownedProcess.includes(required), true, `missing exact job boundary ${required}`);
  }
  const postgresIdentity = source.slice(
    source.indexOf("function Get-Phase4OwnedPostgresProcessRecord"),
    source.indexOf("function Invoke-Phase4Psql"),
  );
  assert.equal(postgresIdentity.includes("[string]$lines[1]"), true);
  assert.equal(postgresIdentity.includes("[string]$lines[2]"), true);
  assert.equal(postgresIdentity.includes("[string]$lines[3]"), true);
  const postgresInit = source.slice(
    source.indexOf("$failureStage = 'POSTGRES_INIT'"),
    source.indexOf("$failureStage = 'WSL2_DRAFT_SUBMIT'"),
  );
  assert.equal(
    postgresInit.includes("-OutputEncodingCodePage $script:WindowsOemCodePage"),
    true,
  );
  const psql = source.slice(
    source.indexOf("function Invoke-Phase4Psql"),
    source.indexOf("function Get-Phase4WslDurableEvidence"),
  );
  assert.equal(psql.includes("PGCLIENTENCODING = 'UTF8'"), true);
  assert.equal(psql.includes("LC_ALL = 'C'"), true);
  assert.equal(psql.includes("-OutputEncodingCodePage $script:WindowsOemCodePage"), false);
});

test("managed repository outbox receipt proves the exact six tests and live provider-effect count", async () => {
  const source = await readRepositoryFile("scripts/test-phase4-managed-repository-outbox.ps1");
  for (const required of [
    "function Assert-ManagedRepositoryOutboxCargoEvidence",
    "function Get-ManagedRepositoryProviderDispatchCount",
    "provider_dispatch_snapshots",
    "stage_evidence",
    "stdout_sha256",
    "$script:postgresStartMayOwnProcess = $true",
    "cleanup_failures",
    "PHASE4_MANAGED_REPOSITORY_PASSWORD_CLEANUP_FAILED",
    "PHASE4_MANAGED_REPOSITORY_FINAL_STOP_FAILED",
    "artifacts_retained = $true",
    "postgres_repository_wsl_claim_exact_replays_across_fresh_process_without_provider_effect",
    "$script:windowsOemCodePage = (Get-Culture).TextInfo.OEMCodePage",
    "-OutputEncodingCodePage $script:windowsOemCodePage",
  ]) {
    assert.equal(source.includes(required), true, `missing ${required}`);
  }
  assert.equal(source.includes("provider_dispatch_before = 0"), false);
  assert.equal(source.includes("provider_dispatch_after = 0"), false);
  assert.equal(source.includes("$mayRemove ="), false);
  assert.equal(source.includes("[switch]$KeepArtifacts"), false);
  const start = source.slice(source.indexOf("function Start-OwnedPostgres"),
    source.indexOf("function Stop-OwnedPostgres"));
  assert.ok(start.indexOf("$script:postgresStartMayOwnProcess = $true")
    < start.indexOf("Start-Phase4OwnedProcessJob"));
  const finalizer = source.slice(source.indexOf("finally {"));
  assert.ok(finalizer.includes("Remove-Item -LiteralPath $passwordPath -Force"));
  assert.ok(finalizer.includes("Test-Path -LiteralPath $passwordPath"));
  const stop = source.slice(source.indexOf("function Stop-OwnedPostgres"),
    source.indexOf("foreach ($binary"));
  assert.equal(stop.includes("$script:postgresStartMayOwnProcess = $false"), true);
  assert.equal(stop.includes("$absenceDeadline"), true);
  assert.equal(stop.includes("$absenceObservations"), true);
  assert.equal(source.includes("$script:postgresLauncherTerminalProven"), true);
  assert.equal(source.includes("Assert-OwnedProcessIdentityAbsent"), true);
  assert.equal(source.includes("Get-OwnedPostgresProcessRecord"), true);
  assert.equal(source.includes("process_start_utc_ticks"), true);
  assert.equal(source.includes("$Failure + '_TERMINATION_FAILED'"), true);
  assert.equal(source.includes("phase4-owned-process.ps1"), true);
  assert.equal(source.includes("Get-CimInstance Win32_Process"), false);
  const cargoTimeout = source.slice(source.indexOf("function Invoke-CargoStage"),
    source.indexOf("function Get-ManagedRepositoryProviderDispatchCount"));
  assert.equal(cargoTimeout.includes("Invoke-OutboxOwnedProcess"), true);
  assert.equal(cargoTimeout.includes("$process.Kill($true)"), false);
});
