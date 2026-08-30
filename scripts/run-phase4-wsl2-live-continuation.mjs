import { execFile as execFileCallback, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { promisify } from "node:util";

import {
  canonicalJson,
  validateWsl2ExecutionEnvironment,
} from "../apps/lattice-control/src/wsl2-execution-domain.mjs";
import { runWsl2ExecutionPreflightBridge } from
  "../apps/lattice-control/src/wsl2-execution-preflight-bridge.mjs";
import { runWsl2VerifierBridge } from "../apps/lattice-control/src/wsl2-verifier-bridge.mjs";

const execFile = promisify(execFileCallback);
const CARGO_ARGS = Object.freeze(["test", "--locked", "--offline"]);
const HEX_40 = /^[a-f0-9]{40}$/u;
const HEX_64 = /^[a-f0-9]{64}$/u;
const FILES = Object.freeze({
  context: "acceptance-context.json",
  environment: "execution-environment.json",
  preflight1: "zero-model-preflight.json",
  cargo1: "wsl2-verifier-cargo.json",
  receipt1: "attempt-receipt-attempt-1.json",
  preflight2: "wsl2-preflight-result-attempt-2.json",
  result2: "wsl2-verifier-cargo-interrupted-attempt-2.json",
  receipt2: "attempt-receipt-attempt-2.json",
  preflight3: "wsl2-preflight-result-attempt-3.json",
  result3: "wsl2-verifier-cargo-timeout-attempt-3.json",
  receipt3: "attempt-receipt-attempt-3.json",
  preflight4: "wsl2-preflight-result-attempt-4.json",
  result4: "wsl2-verifier-cargo-recovered-attempt-4.json",
  receipt4: "attempt-receipt-attempt-4.json",
  suite: "wsl2-continuation-suite.json",
});

function fail(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
}

function ensure(condition, code) {
  if (!condition) fail(code);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function digest(domain, value) {
  return `${domain}:sha256:${sha256(canonicalJson(value))}`;
}

function windowsHostEnvironment() {
  return Object.fromEntries(["SystemRoot", "WINDIR"].flatMap((key) => (
    process.env[key] === undefined ? [] : [[key, process.env[key]]]
  )));
}

function parseArgs(argv) {
  ensure(argv.length === 2 && argv[0] === "--evidence-dir" && argv[1].length > 0,
    "PHASE4_WSL2_CONTINUATION_USAGE_REJECTED");
  return argv[1];
}

function readJson(evidenceDir, name) {
  const bytes = readFileSync(path.win32.join(evidenceDir, name));
  ensure(bytes.length > 0 && bytes.length <= 1_048_576,
    "PHASE4_WSL2_CONTINUATION_EVIDENCE_BOUND_REJECTED");
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch {
    fail("PHASE4_WSL2_CONTINUATION_EVIDENCE_REJECTED");
  }
}

function writeExclusive(evidenceDir, name, value) {
  writeFileSync(path.win32.join(evidenceDir, name), `${canonicalJson(value)}\n`, {
    encoding: "utf8",
    flag: "wx",
    mode: 0o600,
  });
}

function linuxToUnc(distribution, linuxPath) {
  ensure(/^[A-Za-z0-9._-]+$/u.test(distribution) && linuxPath.startsWith("/home/")
    && path.posix.normalize(linuxPath) === linuxPath,
  "PHASE4_WSL2_CONTINUATION_PATH_REJECTED");
  return `\\\\wsl.localhost\\${distribution}${linuxPath.replaceAll("/", "\\")}`;
}

async function runWslRaw(environment, executable, args, timeout = 30_000) {
  try {
    const result = await execFile(environment.gateway.windows_path, [
      "-d", environment.distribution, "--exec", "/usr/bin/env", "-i",
      `XDG_RUNTIME_DIR=${environment.process_fence.user_runtime_dir}`,
      "PATH=/usr/bin:/bin", "LANG=C.UTF-8", "LC_ALL=C.UTF-8", executable, ...args,
    ], {
      encoding: "buffer",
      env: windowsHostEnvironment(),
      timeout,
      windowsHide: true,
      maxBuffer: 65_536,
    });
    return { code: 0, signal: null, stdout: result.stdout, stderr: result.stderr };
  } catch (error) {
    const stdout = Buffer.isBuffer(error.stdout) ? error.stdout : Buffer.alloc(0);
    const stderr = Buffer.isBuffer(error.stderr) ? error.stderr : Buffer.alloc(0);
    ensure(stdout.length <= 65_536 && stderr.length <= 65_536,
      "PHASE4_WSL2_CONTINUATION_CONTROL_OUTPUT_BOUND_REJECTED");
    return {
      code: Number.isSafeInteger(error.code) ? error.code : null,
      signal: typeof error.signal === "string" ? error.signal : null,
      stdout,
      stderr,
    };
  }
}

async function waitFor(predicate, attempts = 100) {
  for (let index = 0; index < attempts; index += 1) {
    if (await predicate()) return true;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return false;
}

async function startFaultLock(environment, attempt, processFence, seconds) {
  const lockPath = `${environment.verification_toolchain.cargo_target_dir}/debug/.cargo-build-lock`;
  const unit = `${environment.process_fence.unit_prefix}-fault-${attempt}-${processFence.slice(0, 8)}.service`;
  ensure(/^lattice-wsl2-[a-f0-9]{16}-fault-[2-3]-[a-f0-9]{8}\.service$/u.test(unit),
    "PHASE4_WSL2_CONTINUATION_FAULT_UNIT_REJECTED");
  const startArgs = [
    "--user", "--quiet", `--unit=${unit}`, "--property=Type=exec",
    "--property=KillMode=control-group", "--property=Delegate=no",
    `--property=RuntimeMaxSec=${seconds + 10}`,
    "/usr/bin/flock", "--exclusive", lockPath, "/usr/bin/sleep", String(seconds),
  ];
  const started = await runWslRaw(environment, environment.process_fence.systemd_run_path, startArgs);
  ensure(started.code === 0 && started.signal === null,
    "PHASE4_WSL2_CONTINUATION_FAULT_START_REJECTED");
  const locked = await waitFor(async () => {
    const active = await runWslRaw(environment, environment.process_fence.systemctl_path,
      ["--user", "is-active", unit]);
    if (active.code !== 0 || active.stdout.toString("utf8").trim() !== "active") return false;
    const probe = await runWslRaw(environment, "/usr/bin/flock", [
      "--nonblock", lockPath, "/usr/bin/true",
    ]);
    return probe.code === 1;
  });
  ensure(locked, "PHASE4_WSL2_CONTINUATION_FAULT_LOCK_REJECTED");
  return Object.freeze({
    schema: "lattice.phase4-wsl2-fault-injector/1.0",
    unit,
    lock_path: lockPath,
    process_fence: processFence,
    duration_seconds: seconds,
    command_digest: digest("phase4-wsl2-fault-command", {
      executable: environment.process_fence.systemd_run_path,
      args: startArgs,
    }),
  });
}

async function stopFaultLock(environment, fault) {
  const stopped = await runWslRaw(environment, environment.process_fence.systemctl_path,
    ["--user", "stop", fault.unit], 30_000);
  ensure(stopped.code === 0 && stopped.signal === null,
    "PHASE4_WSL2_CONTINUATION_FAULT_STOP_REJECTED");
  const inactive = await waitFor(async () => {
    const state = await runWslRaw(environment, environment.process_fence.systemctl_path,
      ["--user", "show", "--property=ActiveState", "--value", fault.unit]);
    return state.code === 0 && state.stdout.toString("utf8").trim() === "inactive";
  });
  const released = await runWslRaw(environment, "/usr/bin/flock", [
    "--nonblock", fault.lock_path, "/usr/bin/true",
  ]);
  ensure(inactive && released.code === 0 && released.signal === null,
    "PHASE4_WSL2_CONTINUATION_FAULT_CLEANUP_REJECTED");
  return Object.freeze({
    unit: fault.unit,
    active_state: "inactive",
    lock_released: true,
    zero_provider_effects: true,
  });
}

function attemptReceipt(environment, preflight, result) {
  const subject = {
    schema: "lattice.phase4-attempt-receipt/1.0",
    task_ref: result.task_ref,
    attempt: result.attempt,
    worktree_ref: result.worktree_ref,
    role: result.role,
    execution_environment_ref: environment.identity_digest,
    preflight_receipt_ref: preflight.receipt_digest,
    process_fence: result.process_marker.fence,
    outcome: result.outcome,
    result_digest: result.result_digest,
    provider_effect_count: result.provider_effect_count,
  };
  return { ...subject, receipt_ref: digest("attempt-receipt", subject) };
}

function continuationFence(environment, context, attempt, kind, receiptRef) {
  return sha256(Buffer.from(canonicalJson({
    schema: "lattice.phase4-wsl2-continuation-fence/1.0",
    task_ref: context.taskRef,
    worktree_ref: context.worktreeRef,
    execution_environment_ref: environment.identity_digest,
    attempt,
    kind,
    receipt_ref: receiptRef,
  }), "utf8"));
}

async function continuationPreflight(environment, context, attempt, kind, receiptRef) {
  const processFence = continuationFence(environment, context, attempt, kind, receiptRef);
  const result = await runWsl2ExecutionPreflightBridge({
    schema: "lattice.wsl2-execution-preflight-request/1.0",
    template_descriptor: environment,
    windows_worktree_path: environment.path_mapping.windows_path,
    task_ref: context.taskRef,
    attempt,
    worktree_ref: context.worktreeRef,
    expected_repository_head: environment.linux.repository_head,
    process_fence: processFence,
    retry_of: kind === "RETRY" ? receiptRef : null,
    reconnect_of: kind === "RECONNECT" ? receiptRef : null,
  });
  ensure(result.status === "PASS" && result.task_ref === context.taskRef
    && result.attempt === attempt && result.worktree_ref === context.worktreeRef
    && result.environment.identity_digest === environment.identity_digest
    && canonicalJson(result.environment) === canonicalJson(environment)
    && result.receipt.provider_effect_count === 0
    && result.receipt.timeout.timed_out === false
    && result.receipt.timeout.interrupted === false
    && result.receipt.continuation.retry_of === (kind === "RETRY" ? receiptRef : null)
    && result.receipt.continuation.reconnect_of === (kind === "RECONNECT" ? receiptRef : null),
  "PHASE4_WSL2_CONTINUATION_PREFLIGHT_REJECTED");
  return { result, processFence };
}

function cargoRequest(environment, context, preflight, attempt) {
  return {
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: preflight,
    task_ref: context.taskRef,
    attempt,
    worktree_ref: context.worktreeRef,
    role: "CARGO",
    args: CARGO_ARGS,
  };
}

function interruptAfterMarkerSpawn() {
  let markerObserved = false;
  return (command, args, options) => {
    const child = spawn(command, args, options);
    let observed = "";
    child.stderr?.on("data", (chunk) => {
      if (markerObserved) return;
      observed = `${observed}${Buffer.from(chunk).toString("utf8")}`.slice(-65_536);
      if (observed.includes('"schema":"lattice.wsl2-process-fence/1.1"')) {
        markerObserved = true;
        setImmediate(() => { process.emit("SIGINT"); });
      }
    });
    return child;
  };
}

function assertExpectedResult(result, environment, context, attempt, outcome) {
  ensure(result.schema === "lattice.wsl2-verifier-result/1.0"
    && result.task_ref === context.taskRef && result.attempt === attempt
    && result.worktree_ref === context.worktreeRef && result.role === "CARGO"
    && result.repository_head === environment.linux.repository_head
    && result.provider_effect_count === 0 && result.outcome === outcome
    && result.status === (outcome === "PASS" ? "PASS" : "FAILED")
    && result.exit_receipt.zero_descendants === true
    && result.exit_receipt.credential_seal_intact === true
    && result.exit_receipt.credential_watch_intact === true
    && /^wsl2-verifier-result:sha256:[a-f0-9]{64}$/u.test(result.result_digest),
  `PHASE4_WSL2_CONTINUATION_${outcome}_REJECTED`);
  if (outcome === "INTERRUPTED" || outcome === "TIMED_OUT") {
    ensure((outcome === "INTERRUPTED"
      ? result.outer_cleanup?.reason === outcome
      : result.exit_receipt.timed_out === true
        && (result.outer_cleanup === null || result.outer_cleanup?.reason === outcome))
      && result.outer_post_exit.active_state === "inactive"
      && (result.outer_post_exit.cgroup_exists === false
        || result.outer_post_exit.populated === 0),
    `PHASE4_WSL2_CONTINUATION_${outcome}_CLEANUP_REJECTED`);
  }
}

async function runFaulted(environment, request, fault, dependencies = {}) {
  try {
    return await runWsl2VerifierBridge(request, dependencies);
  } finally {
    fault.cleanup = await stopFaultLock(environment, fault);
  }
}

async function main() {
  const evidenceDir = parseArgs(process.argv.slice(2));
  const environment = validateWsl2ExecutionEnvironment(readJson(evidenceDir, FILES.environment));
  const expectedEvidenceDir = linuxToUnc(environment.distribution,
    `${environment.verification_toolchain.isolation_root}/evidence`);
  ensure(path.win32.normalize(evidenceDir) === path.win32.normalize(expectedEvidenceDir),
    "PHASE4_WSL2_CONTINUATION_EVIDENCE_PATH_REJECTED");
  const context = readJson(evidenceDir, FILES.context);
  const preflight1 = readJson(evidenceDir, FILES.preflight1);
  const cargo1 = readJson(evidenceDir, FILES.cargo1);
  ensure(context.attempt === 1 && HEX_64.test(context.taskRef)
    && /^worktree:sha256:[a-f0-9]{64}$/u.test(context.worktreeRef)
    && HEX_40.test(environment.linux.repository_head)
    && preflight1.provider_effect_count === 0 && cargo1.status === "PASS"
    && cargo1.outcome === "PASS" && cargo1.provider_effect_count === 0,
  "PHASE4_WSL2_CONTINUATION_BASE_REJECTED");
  const receipt1 = attemptReceipt(environment, preflight1, cargo1);
  writeExclusive(evidenceDir, FILES.receipt1, receipt1);

  const second = await continuationPreflight(
    environment, context, 2, "RECONNECT", receipt1.receipt_ref,
  );
  writeExclusive(evidenceDir, FILES.preflight2, second.result);
  const fault2 = { ...await startFaultLock(environment, 2, second.processFence, 60) };
  const result2 = await runFaulted(
    environment,
    cargoRequest(environment, context, second.result.receipt, 2),
    fault2,
    { spawnProcess: interruptAfterMarkerSpawn() },
  );
  assertExpectedResult(result2, environment, context, 2, "INTERRUPTED");
  const receipt2 = attemptReceipt(environment, second.result.receipt, result2);
  writeExclusive(evidenceDir, FILES.result2, { ...result2, fault_injector: fault2 });
  writeExclusive(evidenceDir, FILES.receipt2, receipt2);

  const third = await continuationPreflight(
    environment, context, 3, "RETRY", receipt2.receipt_ref,
  );
  writeExclusive(evidenceDir, FILES.preflight3, third.result);
  const fault3 = { ...await startFaultLock(environment, 3, third.processFence, 215) };
  const result3 = await runFaulted(
    environment,
    cargoRequest(environment, context, third.result.receipt, 3),
    fault3,
  );
  assertExpectedResult(result3, environment, context, 3, "TIMED_OUT");
  const receipt3 = attemptReceipt(environment, third.result.receipt, result3);
  writeExclusive(evidenceDir, FILES.result3, { ...result3, fault_injector: fault3 });
  writeExclusive(evidenceDir, FILES.receipt3, receipt3);

  const fourth = await continuationPreflight(
    environment, context, 4, "RETRY", receipt3.receipt_ref,
  );
  writeExclusive(evidenceDir, FILES.preflight4, fourth.result);
  const result4 = await runWsl2VerifierBridge(
    cargoRequest(environment, context, fourth.result.receipt, 4),
  );
  assertExpectedResult(result4, environment, context, 4, "PASS");
  const receipt4 = attemptReceipt(environment, fourth.result.receipt, result4);
  writeExclusive(evidenceDir, FILES.result4, result4);
  writeExclusive(evidenceDir, FILES.receipt4, receipt4);

  const subject = {
    schema: "lattice.phase4-wsl2-continuation-suite/1.0",
    status: "PASS",
    task_ref: context.taskRef,
    worktree_ref: context.worktreeRef,
    execution_environment_ref: environment.identity_digest,
    repository_head: environment.linux.repository_head,
    sequence: [
      { attempt: 1, action: "BASE", outcome: cargo1.outcome, receipt_ref: receipt1.receipt_ref },
      { attempt: 2, action: "RECONNECT", outcome: result2.outcome, receipt_ref: receipt2.receipt_ref },
      { attempt: 3, action: "RETRY", outcome: result3.outcome, receipt_ref: receipt3.receipt_ref },
      { attempt: 4, action: "RETRY", outcome: result4.outcome, receipt_ref: receipt4.receipt_ref },
    ],
    credential_seal_digest: preflight1.credential_seal_digest,
    descriptor_exact_replay: true,
    repository_head_exact_replay: true,
    fault_injectors_cleaned: fault2.cleanup.lock_released && fault3.cleanup.lock_released,
    provider_effect_count: 0,
  };
  const suite = { ...subject, suite_digest: digest("phase4-wsl2-continuation-suite", subject) };
  writeExclusive(evidenceDir, FILES.suite, suite);
  process.stdout.write(`${canonicalJson(suite)}\n`);
}

// This pre-Foreman prototype self-minted attempt receipts, mislabeled attempt 2 as a
// reconnect, and exceeded the closed three-attempt budget. Keep it as immutable audit
// context, but never let it emit acceptance-looking evidence. The production managed
// Foreman lane is the sole Phase 4 continuation authority.
process.stderr.write(`${JSON.stringify({
  schema: "lattice.phase4-wsl2-continuation-deprecation/1.0",
  status: "REJECTED",
  code: "PHASE4_WSL2_CONTINUATION_RUNNER_DEPRECATED",
  replacement: "scripts/test-phase4-managed-foreman.ps1",
})}\n`);
process.exitCode = 70;
