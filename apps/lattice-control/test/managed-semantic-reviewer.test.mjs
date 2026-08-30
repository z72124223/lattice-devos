import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  MANAGED_REVIEW_PACKET_SCHEMA,
  MANAGED_REVIEW_RESULT_SCHEMA,
  ManagedSemanticReviewerTransport,
  deterministicManagedReviewProcessFence,
  prepareManagedSemanticReviewLaunch,
  validateManagedSemanticReviewPacket,
} from "../src/managed-semantic-reviewer.mjs";

const reviewerPath = fileURLToPath(new URL("../src/managed-semantic-reviewer.mjs", import.meta.url));

const sha256 = (value) => createHash("sha256").update(value, "utf8").digest("hex");
const NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF =
  `execution-environment:sha256:${"0".repeat(63)}1`;
const WSL_EXECUTION_ENVIRONMENT_REF = `execution-environment:sha256:${"7".repeat(64)}`;
const WSL_WORKTREE_REF = `worktree:sha256:${"9".repeat(64)}`;
const WSL_REVIEW_CWD = "/home/lattice/tasks/review/worktree";
const WSL_REVIEW_UNC = String.raw`\\wsl.localhost\Ubuntu\home\lattice\tasks\review\worktree`;
const WSL_CREDENTIAL_AUTHORITY_REF = `wsl2-credential-authority:sha256:${"a".repeat(64)}`;
const WSL_CREDENTIAL_SEAL_DIGEST = `credential-seal:sha256:${"b".repeat(64)}`;
const WSL_PROCESS_FENCE_AUTHORITY_REF = `wsl2-process-fence-authority:sha256:${"c".repeat(64)}`;
const WSL_VERIFICATION_TOOLCHAIN_REF = `wsl2-verification-toolchain:sha256:${"d".repeat(64)}`;

function packet(overrides = {}) {
  const currentSecond = Math.floor(Date.now() / 1_000) * 1_000;
  const createdAt = new Date(currentSecond).toISOString().replace(".000Z", "Z");
  const deadlineAt = new Date(currentSecond + 600_000).toISOString().replace(".000Z", "Z");
  const values = {
    task_ref: "a".repeat(64),
    project_digest: "b".repeat(64),
    spec_digest: "c".repeat(64),
    base_commit: "d".repeat(40),
    result_commit: "e".repeat(40),
    tree: "f".repeat(40),
    diff_digest: "1".repeat(64),
    changed_paths_digest: "2".repeat(64),
    subject_digest: "4".repeat(64),
    attempt: overrides.attempt ?? 1,
  };
  const prompt = [
    `[LATTICE_MANAGED_REVIEW task_ref=${values.task_ref} attempt=${values.attempt} subject_digest=${values.subject_digest}]`,
    "Perform an independent read-only semantic review.",
    `task_ref=${values.task_ref}`,
    `project_digest=${values.project_digest}`,
    `spec_digest=${values.spec_digest}`,
    `base_commit=${values.base_commit}`,
    `result_commit=${values.result_commit}`,
    `tree=${values.tree}`,
    `diff_digest=${values.diff_digest}`,
    `changed_paths_digest=${values.changed_paths_digest}`,
    "Return lattice.managed-semantic-review/1.0 JSON.",
  ].join("\n");
  return {
    schema: MANAGED_REVIEW_PACKET_SCHEMA,
    task_ref: values.task_ref,
    attempt: values.attempt,
    project_digest: values.project_digest,
    spec_digest: values.spec_digest,
    verification_policy_digest: "3".repeat(64),
    base_commit: values.base_commit,
    result_commit: values.result_commit,
    tree: values.tree,
    diff_digest: values.diff_digest,
    changed_paths_digest: values.changed_paths_digest,
    subject_digest: values.subject_digest,
    prompt_digest: sha256(prompt),
    cwd: "C:\\disposable\\review-repo",
    execution_environment_ref: NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF,
    worktree_ref: null,
    execution_preflight_continuation: null,
    prompt,
    created_at: createdAt,
    deadline_at: deadlineAt,
    max_total_tokens: 20_000,
    max_model_calls: 1,
    model_call_identity: `managed-review-${values.task_ref}-${values.attempt}`,
    model: "gpt-5.6-terra",
    reasoning: "medium",
    auth_context: {
      schema: "lattice.managed-codex-auth-context/1.0",
      codex_home_digest: `codex-home:sha256:${"5".repeat(64)}`,
      config_digest: `codex-config:sha256:${"6".repeat(64)}`,
    },
    restart: null,
    ...overrides,
  };
}

function wslPacket(overrides = {}) {
  return packet({
    cwd: WSL_REVIEW_CWD,
    execution_environment_ref: WSL_EXECUTION_ENVIRONMENT_REF,
    worktree_ref: WSL_WORKTREE_REF,
    execution_preflight_continuation: { retry_of: null, reconnect_of: null },
    ...overrides,
  });
}

function wslDescriptor(overrides = {}) {
  const descriptor = {
    schema: "lattice.execution-environment.wsl2-linux/1.1",
    kind: "WSL2_LINUX",
    distribution: "Ubuntu",
    linux: {
      cwd: WSL_REVIEW_CWD,
      repository_head: "d".repeat(40),
      codex_home: "/home/lattice/codex-home",
      config_digest: `codex-config:sha256:${"6".repeat(64)}`,
    },
    credential_authority: {
      kind: "LINUX_KEYRING",
      authority_digest: WSL_CREDENTIAL_AUTHORITY_REF,
    },
    process_fence: {
      kind: "SYSTEMD_USER_SERVICE_CGROUP_V2",
      identity_digest: WSL_PROCESS_FENCE_AUTHORITY_REF,
    },
    verification_toolchain: {
      task_ref: "a".repeat(64),
      identity_digest: WSL_VERIFICATION_TOOLCHAIN_REF,
    },
    path_mapping: {
      windows_path: WSL_REVIEW_UNC,
      linux_path: WSL_REVIEW_CWD,
    },
    identity_digest: WSL_EXECUTION_ENVIRONMENT_REF,
  };
  return {
    ...descriptor,
    ...overrides,
    linux: { ...descriptor.linux, ...overrides.linux },
    path_mapping: { ...descriptor.path_mapping, ...overrides.path_mapping },
  };
}

test("WSL reviewer keeps the execution environment on base commit while reviewing a distinct result commit", async () => {
  const reviewPacket = wslPacket();
  const descriptor = wslDescriptor();
  let observedLaunch = false;
  const accepted = await prepareManagedSemanticReviewLaunch(reviewPacket, {
    executionEnvironmentJson: JSON.stringify(descriptor),
    validateWsl2ExecutionEnvironment: (value) => Object.freeze(structuredClone(value)),
    preflightWsl2ExecutionEnvironment: async (value, context) => {
      assert.equal(value.linux.repository_head, reviewPacket.base_commit);
      assert.notEqual(reviewPacket.base_commit, reviewPacket.result_commit);
      return wslPreflight(value, context.processFence);
    },
    buildWsl2CodexLaunch: (value, options) => {
      observedLaunch = true;
      return {
        command: String.raw`C:\Windows\System32\wsl.exe`,
        args: [],
        processFence: options.fence,
        codexIdentity: {
          execution_environment_ref: value.identity_digest,
          credential_authority_ref: WSL_CREDENTIAL_AUTHORITY_REF,
          codex_home_digest: `codex-home:sha256:${"5".repeat(64)}`,
          credential_seal_digest: WSL_CREDENTIAL_SEAL_DIGEST,
          process_fence_authority_ref: WSL_PROCESS_FENCE_AUTHORITY_REF,
          process_fence: options.fence,
          linux_cwd: WSL_REVIEW_CWD,
          repository_head: reviewPacket.base_commit,
        },
      };
    },
    processFence: () => "f".repeat(64),
  });
  assert.equal(observedLaunch, true);
  assert.equal(accepted.launchSpec.codexIdentity.repository_head, reviewPacket.base_commit);
  assert.equal(reviewPacket.result_commit, "e".repeat(40));
  assert.equal(reviewPacket.tree, "f".repeat(40));
  assert.equal(reviewPacket.diff_digest, "1".repeat(64));
});

test("WSL reviewer rejects a descriptor HEAD substituted to the unreferenced result commit", async () => {
  const reviewPacket = wslPacket();
  const substituted = wslDescriptor({
    linux: { repository_head: reviewPacket.result_commit },
  });
  const noEffect = () => assert.fail("descriptor HEAD substitution must reject before preflight");
  await assert.rejects(
    prepareManagedSemanticReviewLaunch(reviewPacket, {
      executionEnvironmentJson: JSON.stringify(substituted),
      validateWsl2ExecutionEnvironment: (value) => Object.freeze(structuredClone(value)),
      preflightWsl2ExecutionEnvironment: noEffect,
      buildWsl2CodexLaunch: noEffect,
      processFence: () => "f".repeat(64),
    }),
    { code: "MANAGED_REVIEW_EXECUTION_ENVIRONMENT_MISMATCH" },
  );
});

function wslPreflight(value, processFence, {
  attempt = 1,
  providerEffectCount = 0,
  retryOf = null,
  reconnectOf = null,
} = {}) {
  return {
    environment: value,
    receipt: {
      task_ref: value.verification_toolchain.task_ref,
      attempt,
      worktree_ref: WSL_WORKTREE_REF,
      execution_environment_ref: value.identity_digest,
      linux_cwd: value.linux.cwd,
      repository_head: value.linux.repository_head,
      codex_home_digest: `codex-home:sha256:${"5".repeat(64)}`,
      credential_authority_ref: value.credential_authority.authority_digest,
      credential_seal_digest: WSL_CREDENTIAL_SEAL_DIGEST,
      verification_toolchain_ref: value.verification_toolchain.identity_digest,
      process_fence: {
        fence: processFence,
        authority_ref: value.process_fence.identity_digest,
      },
      effect_counters: {
        thread_start: 0,
        turn_start: 0,
        provider_effect_count: providerEffectCount,
      },
      provider_effect_count: providerEffectCount,
      continuation: {
        attempt,
        retry_of: retryOf,
        reconnect_of: reconnectOf,
      },
      receipt_digest: `wsl2-preflight:sha256:${"e".repeat(64)}`,
    },
  };
}

function turnAuthorization(reviewPacket, threadId = "review-thread-cli") {
  return {
    schema: "lattice.managed-semantic-review-turn-control/1.0",
    action: "AUTHORIZE_TURN_START",
    task_ref: reviewPacket.task_ref,
    attempt: reviewPacket.attempt,
    subject_digest: reviewPacket.subject_digest,
    prompt_digest: reviewPacket.prompt_digest,
    thread_id: threadId,
    model_call_identity: reviewPacket.model_call_identity,
  };
}

function exactTurnInterrupt(
  reviewPacket,
  threadId = "review-thread-cli",
  turnId = "review-turn-cli",
) {
  return {
    schema: "lattice.managed-semantic-review-turn-control/1.0",
    action: "INTERRUPT_EXACT_TURN",
    task_ref: reviewPacket.task_ref,
    attempt: reviewPacket.attempt,
    subject_digest: reviewPacket.subject_digest,
    prompt_digest: reviewPacket.prompt_digest,
    thread_id: threadId,
    turn_id: turnId,
    model_call_identity: reviewPacket.model_call_identity,
  };
}

async function runReviewerProcess(root, command, controls = [], timeoutMs = 5_000, endInput = true) {
  const child = spawn(process.execPath, [reviewerPath], {
    cwd: root,
    env: { ...process.env, LATTICE_CODEX_BIN: process.execPath },
    stdio: ["pipe", "pipe", "pipe"],
    windowsHide: true,
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => { stdout += chunk; });
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  child.stdin.write(`${JSON.stringify(command)}\n`);
  for (const control of controls) child.stdin.write(`${JSON.stringify(control)}\n`);
  if (endInput) child.stdin.end();
  const { exitCode, timedOut } = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      child.kill();
      resolve({ exitCode: null, timedOut: true });
    }, timeoutMs);
    child.once("error", reject);
    child.once("exit", (code) => {
      clearTimeout(timer);
      resolve({ exitCode: code, timedOut: false });
    });
  });
  return {
    exitCode,
    timedOut,
    stdout,
    stderr,
    records: stdout.trim().split(/\r?\n/u).filter(Boolean).map((line) => JSON.parse(line)),
  };
}

class ScriptedCodex extends EventEmitter {
  constructor(finalText) {
    super();
    this.finalText = finalText;
    this.calls = [];
    this.connectionGeneration = 7;
    this.appServerSessionId = `app-server-session:sha256:${"7".repeat(64)}`;
    this.active = false;
  }

  async readAuthReadiness() {
    this.calls.push(["readAuthReadiness"]);
    return {
      schema: "lattice.codex-auth-readiness/1.0",
      ready: true,
      authMode: "chatgpt",
      appServerGeneration: this.connectionGeneration,
      appServerSessionId: this.appServerSessionId,
    };
  }

  async startThread(options) {
    this.calls.push(["startThread", options]);
    return { id: "review-thread-exact" };
  }

  async waitForThreadStarted(threadId) {
    this.calls.push(["waitForThreadStarted", threadId]);
    return { id: threadId };
  }

  async startTurn(threadId, promptText, options) {
    this.calls.push(["startTurn", threadId, promptText, options]);
    return { id: "review-turn-exact" };
  }

  async authorizeManagedReviewTurnStart({ lifecycle, threadId }) {
    this.calls.push(["authorizeTurnStart", threadId, lifecycle?.event_type]);
  }

  async waitForTurnStarted(threadId, turnId) {
    this.calls.push(["waitForTurnStarted", threadId, turnId]);
    this.active = true;
    return { id: turnId, status: "inProgress" };
  }

  async waitForTurnCompleted(threadId, turnId) {
    this.calls.push(["waitForTurnCompleted", threadId, turnId]);
    this.emit("notification", {
      method: "thread/tokenUsage/updated",
      params: {
        threadId,
        turnId,
        tokenUsage: {
          total: { inputTokens: 100, outputTokens: 20, totalTokens: 120 },
          modelContextWindow: 200_000,
        },
      },
    });
    this.active = false;
    return {
      id: turnId,
      status: "completed",
      items: [{ id: "final", type: "agentMessage", text: this.finalText }],
    };
  }

  async readThread(threadId, options) {
    this.calls.push(["readThread", threadId, options]);
    return {
      id: threadId,
      turns: [{ id: "review-turn-exact", status: this.active ? "inProgress" : "completed" }],
    };
  }

  async resumeThread(threadId, { expectedTurnId, effectIdentity } = {}) {
    this.calls.push(["resumeThread", threadId, expectedTurnId, { effectIdentity }]);
    this.active = true;
    return { id: threadId, cwd: packet().cwd, turns: [{ id: expectedTurnId, status: "inProgress" }] };
  }

  async resumeEmptyThread(threadId, options) {
    this.calls.push(["resumeEmptyThread", threadId, options]);
    return { id: threadId, cwd: packet().cwd, turns: [] };
  }

  async interruptTurn(threadId, turnId, options) {
    this.calls.push(["interruptTurn", threadId, turnId, options]);
    this.active = false;
    return { id: turnId, status: "interrupted" };
  }

  notificationSnapshot() { return []; }
  isTurnActive() { return this.active; }
}

function markedTurn(reviewPacket, status, finalText = null) {
  const items = [{
    id: "review-user",
    type: "userMessage",
    content: [{ type: "text", text: reviewPacket.prompt }],
  }];
  if (finalText !== null) items.push({ id: "review-final", type: "agentMessage", text: finalText });
  return {
    id: "review-turn-exact",
    status,
    items,
    tokenUsage: {
      total: { inputTokens: 100, outputTokens: 20, totalTokens: 120 },
      modelContextWindow: 200_000,
    },
  };
}

test("review packet is exact, digest-bound, Terra medium, and one model call", () => {
  assert.equal(
    validateManagedSemanticReviewPacket(packet()).execution_environment_ref,
    NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF,
  );
  const missingEnvironmentRef = packet();
  delete missingEnvironmentRef.execution_environment_ref;
  assert.throws(
    () => validateManagedSemanticReviewPacket(missingEnvironmentRef),
    /invalid closed shape/iu,
  );
  assert.throws(
    () => validateManagedSemanticReviewPacket(packet({
      execution_environment_ref: `execution-environment:sha256:${"A".repeat(64)}`,
    })),
    /execution environment ref/iu,
  );
  assert.throws(
    () => validateManagedSemanticReviewPacket(wslPacket({ worktree_ref: null })),
    /worktree ref/iu,
  );
  assert.throws(
    () => validateManagedSemanticReviewPacket(wslPacket({
      worktree_ref: `worktree:sha256:${"A".repeat(64)}`,
    })),
    /worktree ref/iu,
  );
  assert.throws(() => validateManagedSemanticReviewPacket(packet({ model: "gpt-5.6-luna" })), /Terra/iu);
  assert.throws(() => validateManagedSemanticReviewPacket(packet({ reasoning: "high" })), /medium/iu);
  assert.throws(() => validateManagedSemanticReviewPacket(packet({ max_model_calls: 2 })), /one model call/iu);
  assert.throws(() => validateManagedSemanticReviewPacket(packet({ prompt_digest: "9".repeat(64) })), /digest-substituted/iu);
  assert.throws(
    () => validateManagedSemanticReviewPacket(packet({
      created_at: "2026-08-27T11:59:00Z",
      deadline_at: "2026-08-27T12:14:00.001Z",
    })),
    /closed 900 second window/iu,
  );
});

test("review bridge fails closed when WSL descriptor, ref, or Linux cwd is missing or substituted", async () => {
  const descriptor = wslDescriptor();
  const dependencies = {
    validateWsl2ExecutionEnvironment: (value) => Object.freeze(structuredClone(value)),
    preflightWsl2ExecutionEnvironment: async () => assert.fail("preflight must not run"),
    buildWsl2CodexLaunch: () => assert.fail("launch must not be built"),
    processFence: () => "f".repeat(64),
  };
  await assert.rejects(
    prepareManagedSemanticReviewLaunch(wslPacket(), {
      ...dependencies,
      executionEnvironmentJson: null,
    }),
    { code: "MANAGED_REVIEW_EXECUTION_ENVIRONMENT_REQUIRED" },
  );
  await assert.rejects(
    prepareManagedSemanticReviewLaunch(wslPacket({
      execution_environment_ref: `execution-environment:sha256:${"8".repeat(64)}`,
    }), {
      ...dependencies,
      executionEnvironmentJson: JSON.stringify(descriptor),
    }),
    { code: "MANAGED_REVIEW_EXECUTION_ENVIRONMENT_MISMATCH" },
  );
  await assert.rejects(
    prepareManagedSemanticReviewLaunch(wslPacket({ cwd: WSL_REVIEW_UNC }), {
      ...dependencies,
      executionEnvironmentJson: JSON.stringify(descriptor),
    }),
    { code: "MANAGED_REVIEW_EXECUTION_ENVIRONMENT_MISMATCH" },
  );
  await assert.rejects(
    prepareManagedSemanticReviewLaunch(wslPacket({
      execution_preflight_continuation: null,
    }), {
      ...dependencies,
      executionEnvironmentJson: JSON.stringify(descriptor),
    }),
    { code: "MANAGED_REVIEW_EXECUTION_PREFLIGHT_CONTINUATION_REQUIRED" },
  );
});

test("WSL reviewer attempt lineage fails closed before preflight when durable continuation is absent", async () => {
  const descriptor = wslDescriptor();
  const noEffect = () => assert.fail("missing durable lineage must reject before preflight");
  await assert.rejects(
    prepareManagedSemanticReviewLaunch(wslPacket({
      attempt: 2,
      execution_preflight_continuation: { retry_of: null, reconnect_of: null },
    }), {
      executionEnvironmentJson: JSON.stringify(descriptor),
      validateWsl2ExecutionEnvironment: (value) => Object.freeze(structuredClone(value)),
      preflightWsl2ExecutionEnvironment: noEffect,
      buildWsl2CodexLaunch: noEffect,
      processFence: () => "f".repeat(64),
    }),
    { code: "MANAGED_REVIEW_EXECUTION_PREFLIGHT_CONTINUATION_REQUIRED" },
  );
});

test("WSL reviewer attempt two preserves the durable attempt and retry receipt through launch", async () => {
  const descriptor = wslDescriptor();
  const retryOf = `wsl2-preflight:sha256:${"1".repeat(64)}`;
  const reviewPacket = wslPacket({
    attempt: 2,
    execution_preflight_continuation: { retry_of: retryOf, reconnect_of: null },
  });
  const launchIdentity = {
    execution_environment_ref: WSL_EXECUTION_ENVIRONMENT_REF,
    credential_authority_ref: WSL_CREDENTIAL_AUTHORITY_REF,
    codex_home_digest: `codex-home:sha256:${"5".repeat(64)}`,
    credential_seal_digest: WSL_CREDENTIAL_SEAL_DIGEST,
    process_fence_authority_ref: WSL_PROCESS_FENCE_AUTHORITY_REF,
    process_fence: deterministicManagedReviewProcessFence(reviewPacket, descriptor),
    linux_cwd: WSL_REVIEW_CWD,
    repository_head: "d".repeat(40),
  };
  const accepted = await prepareManagedSemanticReviewLaunch(reviewPacket, {
    executionEnvironmentJson: JSON.stringify(descriptor),
    validateWsl2ExecutionEnvironment: (value) => Object.freeze(structuredClone(value)),
    preflightWsl2ExecutionEnvironment: async (value, context) => {
      assert.equal(context.attempt, 2);
      assert.equal(context.retryOf, retryOf);
      assert.equal(context.reconnectOf, null);
      return wslPreflight(value, context.processFence, { attempt: 2, retryOf });
    },
    buildWsl2CodexLaunch: (_value, options) => {
      assert.equal(options.attempt, 2);
      assert.equal(options.retryOf, retryOf);
      assert.equal(options.reconnectOf, null);
      return {
        command: String.raw`C:\Windows\System32\wsl.exe`,
        args: [],
        processFence: options.fence,
        codexIdentity: launchIdentity,
      };
    },
    processFence: () => "f".repeat(64),
  });
  assert.equal(accepted.launchSpec.codexIdentity.execution_environment_ref,
    WSL_EXECUTION_ENVIRONMENT_REF);
});

test("native reviewer keeps the existing native connector path without WSL preflight", async () => {
  const result = await prepareManagedSemanticReviewLaunch(packet(), {
    executionEnvironmentJson: null,
    nativeCodexBin: String.raw`C:\managed\codex.exe`,
    validateWsl2ExecutionEnvironment: () => assert.fail("native review must not validate WSL"),
    preflightWsl2ExecutionEnvironment: async () => assert.fail("native review must not preflight WSL"),
    buildWsl2CodexLaunch: () => assert.fail("native review must not build a WSL launch"),
    processFence: () => assert.fail("native review must not create a WSL fence"),
  });
  assert.deepEqual(result, {
    codexBin: String.raw`C:\managed\codex.exe`,
    launchSpec: null,
  });
});

test("WSL reviewer requires zero-provider-effect preflight before creating its exact launch spec", async () => {
  const descriptor = wslDescriptor();
  const reviewPacket = wslPacket();
  const expectedFence = deterministicManagedReviewProcessFence(reviewPacket, descriptor);
  const calls = [];
  const baseDependencies = {
    executionEnvironmentJson: JSON.stringify(descriptor),
    nativeCodexBin: String.raw`C:\must-not-be-used\codex.exe`,
    validateWsl2ExecutionEnvironment: (value) => {
      calls.push("validate");
      return Object.freeze(structuredClone(value));
    },
  };
  await assert.rejects(
    prepareManagedSemanticReviewLaunch(reviewPacket, {
      ...baseDependencies,
      preflightWsl2ExecutionEnvironment: async (value, context) => {
        calls.push("preflight");
        assert.deepEqual(context, {
          processFence: expectedFence,
          taskRef: "a".repeat(64),
          attempt: 1,
          worktreeRef: WSL_WORKTREE_REF,
          retryOf: null,
          reconnectOf: null,
        });
        return wslPreflight(value, context.processFence, { providerEffectCount: 1 });
      },
      buildWsl2CodexLaunch: () => assert.fail("non-zero provider effect must block launch"),
    }),
    { code: "MANAGED_REVIEW_EXECUTION_PREFLIGHT_REJECTED" },
  );

  calls.length = 0;
  const expectedLaunch = Object.freeze({
    command: String.raw`C:\Windows\System32\wsl.exe`,
    args: ["-d", "Ubuntu", "--exec", "/usr/bin/systemd-run"],
    processFence: expectedFence,
    codexIdentity: {
      execution_environment_ref: WSL_EXECUTION_ENVIRONMENT_REF,
      credential_authority_ref: WSL_CREDENTIAL_AUTHORITY_REF,
      codex_home_digest: `codex-home:sha256:${"5".repeat(64)}`,
      credential_seal_digest: WSL_CREDENTIAL_SEAL_DIGEST,
      process_fence_authority_ref: WSL_PROCESS_FENCE_AUTHORITY_REF,
      process_fence: expectedFence,
      linux_cwd: WSL_REVIEW_CWD,
      repository_head: "d".repeat(40),
    },
  });
  const accepted = await prepareManagedSemanticReviewLaunch(reviewPacket, {
    ...baseDependencies,
    preflightWsl2ExecutionEnvironment: async (value, context) => {
      calls.push("preflight");
      return wslPreflight(value, context.processFence);
    },
    buildWsl2CodexLaunch: (value, options) => {
      calls.push("build");
      assert.equal(value.linux.cwd, WSL_REVIEW_CWD);
      assert.equal(options.preflightReceipt.provider_effect_count, 0);
      assert.equal(options.fence, options.preflightReceipt.process_fence.fence);
      assert.equal(options.attempt, 1);
      assert.equal(options.retryOf, null);
      assert.equal(options.reconnectOf, null);
      return expectedLaunch;
    },
  });
  assert.deepEqual(calls, ["validate", "preflight", "build"]);
  assert.equal(accepted.codexBin, null);
  assert.equal(accepted.launchSpec, expectedLaunch);
  assert.equal(accepted.processFence, expectedFence);
  assert.equal(
    deterministicManagedReviewProcessFence(reviewPacket, structuredClone(descriptor)),
    expectedFence,
  );
});

test("WSL reviewer rejects substituted worktree, Codex home, and process fence receipts before launch", async () => {
  const descriptor = wslDescriptor();
  const mutations = [
    (receipt) => { receipt.worktree_ref = `worktree:sha256:${"8".repeat(64)}`; },
    (receipt) => { receipt.codex_home_digest = `codex-home:sha256:${"8".repeat(64)}`; },
    (receipt) => { receipt.process_fence.fence = "8".repeat(64); },
  ];
  for (const mutate of mutations) {
    await assert.rejects(
      prepareManagedSemanticReviewLaunch(wslPacket(), {
        executionEnvironmentJson: JSON.stringify(descriptor),
        validateWsl2ExecutionEnvironment: (value) => Object.freeze(structuredClone(value)),
        preflightWsl2ExecutionEnvironment: async (value, context) => {
          const observed = wslPreflight(value, context.processFence);
          mutate(observed.receipt);
          return observed;
        },
        buildWsl2CodexLaunch: () => assert.fail("substituted receipt must block launch"),
        processFence: () => "f".repeat(64),
      }),
      { code: "MANAGED_REVIEW_EXECUTION_PREFLIGHT_REJECTED" },
    );
  }
});

test("review restart accepts Rust-canonical fractional seconds and rejects trailing zeroes", () => {
  assert.equal(
    validateManagedSemanticReviewPacket(packet({
      created_at: "2026-08-27T11:59:00.12Z",
      deadline_at: "2026-08-27T12:10:00Z",
    })).created_at,
    "2026-08-27T11:59:00.12Z",
  );
  assert.throws(
    () => validateManagedSemanticReviewPacket(packet({
      created_at: "2026-08-27T11:59:00.120Z",
      deadline_at: "2026-08-27T12:10:00Z",
    })),
    /canonical UTC timestamp/iu,
  );
  assert.equal(
    validateManagedSemanticReviewPacket(packet({
      restart: {
        mode: "RETAINED",
        thread_id: "review-thread-exact",
        turn_id: "review-turn-exact",
        app_server_generation: 7,
        last_event: "TURN_STARTED",
        started_at: "2026-08-27T12:00:00.12Z",
      },
    })).restart.started_at,
    "2026-08-27T12:00:00.12Z",
  );
});

test("review uses a separate exact read-only turn and returns bounded identity and resource evidence", async () => {
  const final = JSON.stringify({
    schema: "lattice.managed-semantic-review/1.0",
    verdict: "PASS",
    findings: [],
  });
  const codex = new ScriptedCodex(final);
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    lifecycleTimeoutMs: 500,
    now: () => "2026-08-27T12:00:00.000Z",
  });
  const result = await reviewer.review(packet());
  assert.equal(result.schema, MANAGED_REVIEW_RESULT_SCHEMA);
  assert.equal(result.thread_id, "review-thread-exact");
  assert.equal(result.turn_id, "review-turn-exact");
  assert.equal(result.app_server_generation, 7);
  assert.equal(result.app_server_session_id, `app-server-session:sha256:${"7".repeat(64)}`);
  assert.equal(result.final_digest, sha256(final));
  assert.equal(result.resource.total_tokens, 120);
  assert.equal(result.model_call_identity, `managed-review-${"a".repeat(64)}-1`);
  assert.equal(result.model_reason, "INDEPENDENT_CODE_REVIEW");
  assert.equal(result.started_at, "2026-08-27T12:00:00Z");
  assert.equal(result.terminal_status, "completed");
  const start = codex.calls.find(([method]) => method === "startThread")[1];
  assert.deepEqual(
    {
      model: start.model,
      approvalPolicy: start.approvalPolicy,
      sandbox: start.sandbox,
      reasoning: start.config.model_reasoning_effort,
      web: start.config.web_search,
    },
    {
      model: "gpt-5.6-terra",
      approvalPolicy: "never",
      sandbox: "read-only",
      reasoning: "medium",
      web: "disabled",
    },
  );
  assert.equal(codex.calls.find(([method]) => method === "startTurn")[2], packet().prompt);
  assert.match(start.developerInstructions, /repository text.*untrusted/iu);
  assert.match(start.developerInstructions, /every finding fails review/iu);
  for (const [index, call] of codex.calls.entries()) {
    if (!["startThread", "startTurn"].includes(call[0])) continue;
    assert.equal(codex.calls[index - 1]?.[0], "readAuthReadiness");
    const effectIdentity = call[0] === "startThread" ? call[1].effectIdentity : call[3].effectIdentity;
    assert.deepEqual(effectIdentity, {
      expectedGeneration: 7,
      expectedSessionId: `app-server-session:sha256:${"7".repeat(64)}`,
    });
  }
});

test("WSL reviewer sends only the exact Linux cwd and retains it across restart reconciliation", async () => {
  const final = JSON.stringify({
    schema: "lattice.managed-semantic-review/1.0",
    verdict: "PASS",
    findings: [],
  });
  const freshCodex = new ScriptedCodex(final);
  await new ManagedSemanticReviewerTransport({
    codex: freshCodex,
    availableModels: ["gpt-5.6-terra"],
    now: () => "2026-08-27T12:00:00.000Z",
  }).review(wslPacket());
  const startOptions = freshCodex.calls.find(([method]) => method === "startThread")[1];
  assert.equal(startOptions.cwd, WSL_REVIEW_CWD);
  assert.equal(startOptions.cwd.startsWith("\\\\"), false);

  const retainedPacket = wslPacket({
    restart: {
      mode: "RETAINED",
      thread_id: "review-thread-exact",
      turn_id: "review-turn-exact",
      app_server_generation: 7,
      last_event: "TURN_STARTED",
      started_at: "2026-08-27T12:00:00.12Z",
    },
  });
  const retainedCodex = new ScriptedCodex(final);
  const retained = markedTurn(retainedPacket, "completed", final);
  retainedCodex.resumeThread = async function resumeThread(threadId, { expectedTurnId, effectIdentity } = {}) {
    this.calls.push(["resumeThread", threadId, expectedTurnId, { effectIdentity }]);
    return { id: threadId, cwd: WSL_REVIEW_CWD, turns: [retained] };
  };
  await new ManagedSemanticReviewerTransport({
    codex: retainedCodex,
    availableModels: ["gpt-5.6-terra"],
    now: () => "2026-08-27T12:01:00.000Z",
  }).review(retainedPacket);
  assert.equal(retainedPacket.execution_environment_ref, WSL_EXECUTION_ENVIRONMENT_REF);
  assert.equal(retainedCodex.calls.filter(([method]) => method === "startThread").length, 0);
  assert.equal(retainedCodex.calls.filter(([method]) => method === "startTurn").length, 0);
  assert.equal(retainedCodex.calls.filter(([method]) => method === "resumeThread").length, 1);
});

test("reviewer performs its own readiness check and blocks identity drift before provider effect", async () => {
  const codex = new ScriptedCodex("{}");
  codex.startThread = async function startThread(options) {
    this.calls.push(["startThread", options]);
    assert.equal(options.effectIdentity.expectedSessionId, this.appServerSessionId);
    const error = new Error("effect identity changed");
    error.code = "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED";
    throw error;
  };
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
  });

  await assert.rejects(
    () => reviewer.review(packet()),
    (error) => error.code === "MANAGED_REVIEW_AUTH_EFFECT_IDENTITY_CHANGED",
  );
  assert.deepEqual(codex.calls.map(([method]) => method), ["readAuthReadiness", "startThread"]);
});

test("token overflow fails closed and interrupts the exact active reviewer turn", async () => {
  const codex = new ScriptedCodex("{}");
  codex.waitForTurnCompleted = async function waitForTurnCompleted(threadId, turnId) {
    this.calls.push(["waitForTurnCompleted", threadId, turnId]);
    this.emit("notification", {
      method: "thread/tokenUsage/updated",
      params: {
        threadId,
        turnId,
        tokenUsage: { total: { totalTokens: 101 } },
      },
    });
    return new Promise(() => {});
  };
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    lifecycleTimeoutMs: 100,
  });
  await assert.rejects(
    () => reviewer.review(packet({ max_total_tokens: 100 })),
    (error) => error.code === "MANAGED_REVIEW_TOKEN_BUDGET_EXCEEDED",
  );
  assert.ok(codex.calls.some(([method]) => method === "interruptTurn"));
});

test("unavailable Terra and exact lifecycle substitutions fail before success", async () => {
  const final = '{"schema":"lattice.managed-semantic-review/1.0","verdict":"PASS","findings":[]}';
  const unavailable = new ManagedSemanticReviewerTransport({
    codex: new ScriptedCodex(final),
    availableModels: ["gpt-5.6-luna"],
  });
  await assert.rejects(() => unavailable.review(packet()), (error) => error.code === "MANAGED_REVIEW_MODEL_UNAVAILABLE");

  const mismatch = new ScriptedCodex(final);
  mismatch.waitForTurnStarted = async () => {
    mismatch.active = true;
    return { id: "other-turn", status: "inProgress" };
  };
  const reviewer = new ManagedSemanticReviewerTransport({ codex: mismatch, availableModels: ["gpt-5.6-terra"] });
  await assert.rejects(() => reviewer.review(packet()), (error) => error.code === "MANAGED_REVIEW_EXACT_LIFECYCLE_MISMATCH");
  assert.ok(mismatch.calls.some(([method]) => method === "readThread"));
  assert.ok(mismatch.calls.some(([method]) => method === "interruptTurn"));
});

test("an exception after exact start reconciles and interrupts the exact turn", async () => {
  const codex = new ScriptedCodex("{}");
  codex.waitForTurnCompleted = async () => {
    const error = new Error("timeout");
    error.code = "CODEX_APP_SERVER_TIMEOUT";
    throw error;
  };
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    lifecycleTimeoutMs: 100,
  });
  await assert.rejects(() => reviewer.review(packet()), /timeout/iu);
  assert.deepEqual(
    codex.calls.filter(([method]) => ["readThread", "interruptTurn"].includes(method)).map(([method]) => method),
    ["readThread", "interruptTurn"],
  );
});

test("review completion may run beyond the 30 second lifecycle RPC window within its packet deadline", async () => {
  const final = '{"schema":"lattice.managed-semantic-review/1.0","verdict":"PASS","findings":[]}';
  const codex = new ScriptedCodex(final);
  codex.waitForTurnCompleted = async function waitForTurnCompleted(threadId, turnId, options) {
    this.calls.push(["waitForTurnCompleted", threadId, turnId, options]);
    assert.ok(options.timeoutMs > 30_000, "completion must receive the packet deadline window");
    assert.ok(options.timeoutMs <= 900_000, "completion must retain the product hard cap");
    await new Promise((resolve) => setTimeout(resolve, 35));
    this.emit("notification", {
      method: "thread/tokenUsage/updated",
      params: { threadId, turnId, tokenUsage: { total: { totalTokens: 1 } } },
    });
    this.active = false;
    return { id: turnId, status: "completed", items: [{ type: "agentMessage", text: final }] };
  };
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    lifecycleTimeoutMs: 30_000,
  });

  const result = await reviewer.review(packet());
  assert.equal(result.terminal_status, "completed");
  assert.equal(codex.calls.filter(([method]) => method === "interruptTurn").length, 0);
});

test("packet deadline timeout with exact cleanup keeps the fixed review timeout reason", async () => {
  const codex = new ScriptedCodex("{}");
  codex.waitForTurnCompleted = async function waitForTurnCompleted(threadId, turnId, options) {
    this.calls.push(["waitForTurnCompleted", threadId, turnId, options]);
    const error = new Error("provider deadline elapsed");
    error.code = "CODEX_APP_SERVER_TIMEOUT";
    error.method = "turn/completed";
    throw error;
  };
  const events = [];
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    lifecycleTimeoutMs: 100,
    onLifecycle: async (event) => { events.push(event); },
  });

  await assert.rejects(
    () => reviewer.review(packet()),
    (error) => error.code === "MANAGED_REVIEW_TIMEOUT",
  );
  assert.deepEqual(
    codex.calls.filter(([method]) => ["readThread", "interruptTurn"].includes(method)).map(([method]) => method),
    ["readThread", "interruptTurn"],
  );
  assert.equal(events.at(-1).event_type, "TURN_TERMINAL");
  assert.equal(events.at(-1).terminal_status, "interrupted");
});

test("packet deadline timeout remains cleanup-ambiguous when exact terminal cannot be proven", async () => {
  const codex = new ScriptedCodex("{}");
  codex.waitForTurnCompleted = async function waitForTurnCompleted() {
    const error = new Error("provider deadline elapsed");
    error.code = "CODEX_APP_SERVER_TIMEOUT";
    error.method = "turn/completed";
    throw error;
  };
  codex.readThread = async function readThread(threadId) {
    this.calls.push(["readThread", threadId]);
    throw new Error("read disconnected");
  };
  codex.resumeThread = async function resumeThread(threadId, { expectedTurnId } = {}) {
    this.calls.push(["resumeThread", threadId, expectedTurnId]);
    throw new Error("resume disconnected");
  };
  const events = [];
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    lifecycleTimeoutMs: 100,
    onLifecycle: async (event) => { events.push(event); },
  });

  await assert.rejects(
    () => reviewer.review(packet()),
    (error) => error.code === "MANAGED_REVIEW_CLEANUP_AMBIGUOUS",
  );
  assert.equal(events.some((event) => event.event_type === "TURN_TERMINAL"), false);
  assert.deepEqual(
    codex.calls.filter(([method]) => ["readThread", "resumeThread"].includes(method)).map(([method]) => method),
    ["readThread", "resumeThread"],
  );
});

test("graceful cancellation after exact start interrupts exactly once and emits the exact terminal", async () => {
  const final = '{"schema":"lattice.managed-semantic-review/1.0","verdict":"PASS","findings":[]}';
  const codex = new ScriptedCodex(final);
  codex.waitForTurnCompleted = async function waitForTurnCompleted(threadId, turnId) {
    this.calls.push(["waitForTurnCompleted", threadId, turnId]);
    await new Promise((resolve) => setTimeout(resolve, 25));
    this.emit("notification", {
      method: "thread/tokenUsage/updated",
      params: { threadId, turnId, tokenUsage: { total: { totalTokens: 1 } } },
    });
    this.active = false;
    return { id: turnId, status: "completed", items: [{ type: "agentMessage", text: final }] };
  };
  const events = [];
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    lifecycleTimeoutMs: 100,
    onLifecycle: async (event) => { events.push(event); },
    waitForExactInterrupt: async ({ packet: reviewPacket, threadId, turnId }) => (
      exactTurnInterrupt(reviewPacket, threadId, turnId)
    ),
  });

  await assert.rejects(
    () => reviewer.review(packet()),
    (error) => error.code === "MANAGED_REVIEW_CANCELLED_AFTER_EXACT_START",
  );
  assert.deepEqual(
    codex.calls.filter(([method]) => method === "interruptTurn").map((call) => call.slice(0, 3)),
    [["interruptTurn", "review-thread-exact", "review-turn-exact"]],
  );
  assert.equal(events.at(-1).event_type, "TURN_TERMINAL");
  assert.equal(events.at(-1).thread_id, "review-thread-exact");
  assert.equal(events.at(-1).turn_id, "review-turn-exact");
  assert.equal(events.at(-1).terminal_status, "interrupted");
});

test("each exact lifecycle event is exposed to the durable sink before the next stage", async () => {
  const final = '{"schema":"lattice.managed-semantic-review/1.0","verdict":"PASS","findings":[]}';
  const events = [];
  const reviewer = new ManagedSemanticReviewerTransport({
    codex: new ScriptedCodex(final),
    availableModels: ["gpt-5.6-terra"],
    now: () => "2026-08-27T12:00:00.120Z",
    onLifecycle: async (event) => { events.push(event); },
  });
  const result = await reviewer.review(packet());
  assert.deepEqual(events.map((event) => event.event_type), [
    "THREAD_START_ACCEPTED",
    "THREAD_STARTED",
    "TURN_START_ACCEPTED",
    "TURN_STARTED",
    "TURN_TERMINAL",
  ]);
  assert.deepEqual(events.map((event) => event.sequence), [1, 2, 3, 4, 5]);
  assert.ok(events.every((event) => event.thread_id === "review-thread-exact"));
  assert.ok(events.slice(2).every((event) => event.turn_id === "review-turn-exact"));
  assert.ok(events.every((event) => event.model_call_identity === `managed-review-${"a".repeat(64)}-1`));
  assert.ok(events.every((event) => event.observed_at === "2026-08-27T12:00:00.12Z"));
  assert.equal(result.started_at, events[3].observed_at);
  assert.equal(result.terminal_at, events[4].observed_at);
});

test("reviewer turn/start waits for exact durable dispatch authorization", async () => {
  const final = '{"schema":"lattice.managed-semantic-review/1.0","verdict":"PASS","findings":[]}';
  const codex = new ScriptedCodex(final);
  const durableEvents = [];
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    onLifecycle: async (event) => { durableEvents.push(event); },
    authorizeTurnStart: async ({ lifecycle, threadId }) => {
      assert.equal(lifecycle, durableEvents.at(-1));
      assert.equal(lifecycle.event_type, "THREAD_STARTED");
      assert.equal(threadId, "review-thread-exact");
      assert.equal(codex.calls.some(([method]) => method === "startTurn"), false);
      codex.calls.push(["durableTurnAuthorization", threadId]);
    },
  });
  await reviewer.review(packet());
  assert.ok(
    codex.calls.findIndex(([method]) => method === "durableTurnAuthorization")
      < codex.calls.findIndex(([method]) => method === "startTurn"),
  );

  const deniedCodex = new ScriptedCodex(final);
  const denied = new ManagedSemanticReviewerTransport({
    codex: deniedCodex,
    availableModels: ["gpt-5.6-terra"],
    authorizeTurnStart: async () => {
      const error = new Error("durable review turn claim replayed");
      error.code = "MANAGED_REVIEW_TURN_DISPATCH_RECONCILIATION_REQUIRED";
      throw error;
    },
  });
  await assert.rejects(
    () => denied.review(packet()),
    (error) => error.code === "MANAGED_REVIEW_TURN_DISPATCH_RECONCILIATION_REQUIRED",
  );
  assert.equal(deniedCodex.calls.some(([method]) => method === "startTurn"), false);
});

test("restart after accepted empty thread reuses it and never starts a duplicate thread", async () => {
  const final = '{"schema":"lattice.managed-semantic-review/1.0","verdict":"PASS","findings":[]}';
  const codex = new ScriptedCodex(final);
  const reviewPacket = packet({
    restart: {
      mode: "RETAINED",
      thread_id: "review-thread-exact",
      turn_id: null,
      app_server_generation: 7,
      last_event: "THREAD_START_ACCEPTED",
      started_at: null,
    },
  });
  const result = await new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    now: () => "2026-08-27T12:00:00.000Z",
  }).review(reviewPacket);
  assert.equal(result.thread_id, "review-thread-exact");
  assert.equal(codex.calls.filter(([method]) => method === "startThread").length, 0);
  assert.equal(codex.calls.filter(([method]) => method === "resumeEmptyThread").length, 1);
  assert.equal(codex.calls.filter(([method]) => method === "startTurn").length, 1);
});

test("review discovery with no exact candidate fails closed without starting provider work", async () => {
  const codex = new ScriptedCodex("{}");
  codex.listThreads = async function listThreads(options) {
    this.calls.push(["listThreads", options]);
    return { data: [], nextCursor: null };
  };
  const reviewPacket = packet({
    restart: {
      mode: "DISCOVER",
      thread_id: null,
      turn_id: null,
      app_server_generation: null,
      last_event: null,
      started_at: null,
    },
  });
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
  });
  await assert.rejects(
    () => reviewer.review(reviewPacket),
    (error) => error.code === "MANAGED_REVIEW_DISPATCH_RECONCILIATION_REQUIRED",
  );
  assert.equal(codex.calls.filter(([method]) => method === "listThreads").length, 1);
  assert.equal(codex.calls.filter(([method]) => method === "startThread").length, 0);
  assert.equal(codex.calls.filter(([method]) => method === "startTurn").length, 0);
});

test("restart after turn acceptance fails closed and interrupts without a duplicate turn", async () => {
  const codex = new ScriptedCodex("{}");
  const events = [];
  const reviewPacket = packet({
    restart: {
      mode: "RETAINED",
      thread_id: "review-thread-exact",
      turn_id: "review-turn-exact",
      app_server_generation: 7,
      last_event: "TURN_START_ACCEPTED",
      started_at: null,
    },
  });
  const retained = markedTurn(reviewPacket, "inProgress");
  codex.resumeThread = async function resumeThread(threadId, { expectedTurnId } = {}) {
    this.calls.push(["resumeThread", threadId, expectedTurnId]);
    this.active = true;
    return { id: threadId, cwd: reviewPacket.cwd, turns: [retained] };
  };
  codex.readThread = async function readThread(threadId) {
    this.calls.push(["readThread", threadId]);
    return { id: threadId, cwd: reviewPacket.cwd, turns: [retained] };
  };
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    onLifecycle: async (event) => { events.push(event); },
  });
  await assert.rejects(
    () => reviewer.review(reviewPacket),
    (error) => error.code === "MANAGED_REVIEW_PRESTART_TERMINAL",
  );
  assert.equal(codex.calls.filter(([method]) => method === "startThread").length, 0);
  assert.equal(codex.calls.filter(([method]) => method === "startTurn").length, 0);
  assert.equal(codex.calls.filter(([method]) => method === "interruptTurn").length, 1);
  assert.equal(events.at(-1).event_type, "TURN_TERMINAL");
  assert.equal(events.at(-1).terminal_status, "interrupted");
});

test("prestart exact terminal is durable failed-review evidence and never success", async () => {
  const codex = new ScriptedCodex("{}");
  const reviewPacket = packet({
    restart: {
      mode: "RETAINED",
      thread_id: "review-thread-exact",
      turn_id: "review-turn-exact",
      app_server_generation: 7,
      last_event: "TURN_START_ACCEPTED",
      started_at: null,
    },
  });
  const retained = markedTurn(reviewPacket, "failed");
  codex.resumeThread = async function resumeThread(threadId, { expectedTurnId } = {}) {
    this.calls.push(["resumeThread", threadId, expectedTurnId]);
    return { id: threadId, cwd: reviewPacket.cwd, turns: [retained] };
  };
  codex.readThread = async function readThread(threadId) {
    this.calls.push(["readThread", threadId]);
    return { id: threadId, cwd: reviewPacket.cwd, turns: [retained] };
  };
  const events = [];
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    onLifecycle: async (event) => { events.push(event); },
  });
  await assert.rejects(
    () => reviewer.review(reviewPacket),
    (error) => error.code === "MANAGED_REVIEW_PRESTART_TERMINAL",
  );
  assert.equal(codex.calls.filter(([method]) => method === "startThread").length, 0);
  assert.equal(codex.calls.filter(([method]) => method === "startTurn").length, 0);
  assert.equal(codex.calls.filter(([method]) => method === "interruptTurn").length, 0);
  assert.equal(events.at(-1).event_type, "TURN_TERMINAL");
  assert.equal(events.at(-1).terminal_status, "failed");
});

test("retained exact-start reviewer completes without opening a new thread or turn", async () => {
  const final = '{"schema":"lattice.managed-semantic-review/1.0","verdict":"PASS","findings":[]}';
  const codex = new ScriptedCodex(final);
  const reviewPacket = packet({
    restart: {
      mode: "RETAINED",
      thread_id: "review-thread-exact",
      turn_id: "review-turn-exact",
      app_server_generation: 7,
      last_event: "TURN_STARTED",
      started_at: "2026-08-27T12:00:00.12Z",
    },
  });
  const retained = markedTurn(reviewPacket, "completed", final);
  codex.resumeThread = async function resumeThread(threadId, { expectedTurnId } = {}) {
    this.calls.push(["resumeThread", threadId, expectedTurnId]);
    return { id: threadId, cwd: reviewPacket.cwd, turns: [retained] };
  };
  const result = await new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    now: () => "2026-08-27T12:01:00.000Z",
  }).review(reviewPacket);
  assert.equal(result.terminal_status, "completed");
  assert.equal(result.resource.total_tokens, 120);
  assert.equal(codex.calls.filter(([method]) => method === "startThread").length, 0);
  assert.equal(codex.calls.filter(([method]) => method === "startTurn").length, 0);
  assert.equal(codex.calls.filter(([method]) => method === "authorizeTurnStart").length, 0);
});

test("completed review without trusted terminal usage fails closed", async () => {
  const final = '{"schema":"lattice.managed-semantic-review/1.0","verdict":"PASS","findings":[]}';
  const codex = new ScriptedCodex(final);
  codex.waitForTurnCompleted = async function waitForTurnCompleted(threadId, turnId) {
    this.calls.push(["waitForTurnCompleted", threadId, turnId]);
    this.active = false;
    return { id: turnId, status: "completed", items: [{ id: "final", type: "agentMessage", text: final }] };
  };
  const reviewer = new ManagedSemanticReviewerTransport({
    codex,
    availableModels: ["gpt-5.6-terra"],
    resourceGraceMs: 0,
  });
  await assert.rejects(
    () => reviewer.review(packet()),
    (error) => error.code === "MANAGED_REVIEW_RESOURCE_OBSERVATION_MISSING",
  );
});

test("reviewer bridge starts turn only after one exact durable authorization", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-managed-review-ack-"));
  const fakeAppServer = `
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt" }, requiresOpenaiAuth: true } });
  else if (message.method === "model/list") send({ id: message.id, result: { data: [{ id: "gpt-5.6-terra" }] } });
  else if (message.method === "thread/start") {
    send({ id: message.id, result: { thread: { id: "review-thread-cli" } } });
    send({ method: "thread/started", params: { thread: { id: "review-thread-cli" } } });
  } else if (message.method === "turn/start") {
    send({ id: message.id, result: { turn: { id: "review-turn-cli", status: "inProgress" } } });
    send({ method: "turn/started", params: { threadId: "review-thread-cli", turn: { id: "review-turn-cli", status: "inProgress" } } });
    send({ method: "thread/tokenUsage/updated", params: { threadId: "review-thread-cli", turnId: "review-turn-cli", tokenUsage: { total: { inputTokens: 10, outputTokens: 2, totalTokens: 12 }, modelContextWindow: 200000 } } });
    send({ method: "turn/completed", params: { threadId: "review-thread-cli", turn: { id: "review-turn-cli", status: "completed", items: [{ type: "agentMessage", text: '{"schema":"lattice.managed-semantic-review/1.0","verdict":"PASS","findings":[]}' }] } } });
  }
}
`;
  await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
  const reviewPacket = packet({ cwd: root });

  const noAuthorization = await runReviewerProcess(root, reviewPacket, [], 5_000, false);
  assert.equal(noAuthorization.timedOut, true);
  assert.deepEqual(
    noAuthorization.records.map((record) => record.event_type).filter(Boolean),
    ["THREAD_START_ACCEPTED", "THREAD_STARTED"],
  );

  const wrongAuthorization = await runReviewerProcess(
    root,
    reviewPacket,
    [turnAuthorization(reviewPacket, "review-thread-other")],
  );
  assert.equal(wrongAuthorization.exitCode, 5);
  assert.equal(wrongAuthorization.records.at(-1).error, "MANAGED_REVIEW_TURN_DISPATCH_RECONCILIATION_REQUIRED");
  assert.equal(wrongAuthorization.records.some((record) => record.event_type === "TURN_START_ACCEPTED"), false);

  const exactAuthorization = await runReviewerProcess(
    root,
    reviewPacket,
    [turnAuthorization(reviewPacket)],
    5_000,
    false,
  );
  assert.equal(exactAuthorization.exitCode, 0, exactAuthorization.stderr);
  assert.equal(
    exactAuthorization.records.filter((record) => record.event_type === "TURN_START_ACCEPTED").length,
    1,
  );
});

test("reviewer bridge keeps only fixed safe provider rejection codes", async (t) => {
  for (const [method, rpcCode, expected] of [
    ["thread/start", -32602, "MANAGED_REVIEW_THREAD_START_RPC_INVALID_PARAMS"],
    ["thread/start", -32603, "MANAGED_REVIEW_THREAD_START_RPC_REJECTED"],
    ["turn/start", -32602, "MANAGED_REVIEW_TURN_START_RPC_INVALID_PARAMS"],
    ["turn/start", -32603, "MANAGED_REVIEW_TURN_START_RPC_REJECTED"],
  ]) {
    const root = await mkdtemp(path.join(tmpdir(), "lattice-managed-review-rpc-"));
    t.after(() => rm(root, { recursive: true, force: true }));
    const fakeAppServer = `
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt" }, requiresOpenaiAuth: true } });
  else if (message.method === "model/list") send({ id: message.id, result: { data: [{ id: "gpt-5.6-terra" }] } });
  else if (message.method === "thread/start" && ${JSON.stringify(method)} === "thread/start") {
    send({ id: message.id, error: { code: ${rpcCode}, message: "secret provider detail", data: { token: "must-not-cross" } } });
  } else if (message.method === "thread/start") {
    send({ id: message.id, result: { thread: { id: "review-thread-cli" } } });
    send({ method: "thread/started", params: { thread: { id: "review-thread-cli" } } });
  } else if (message.method === "turn/start") {
    send({ id: message.id, error: { code: ${rpcCode}, message: "secret provider detail", data: { token: "must-not-cross" } } });
  }
}
`;
    await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
    const reviewPacket = packet({ cwd: root });
    const controls = method === "turn/start" ? [turnAuthorization(reviewPacket)] : [];
    const result = await runReviewerProcess(root, reviewPacket, controls);
    assert.equal(result.exitCode, 5, result.stderr);
    assert.equal(result.records.at(-1).error, expected);
    assert.equal(result.stdout.includes("secret provider detail"), false);
    assert.equal(result.stdout.includes("must-not-cross"), false);
  }
});

test("reviewer bridge keeps control open and interrupts only its exact active turn", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-managed-review-cancel-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const interruptMarker = path.join(root, "interrupt-marker.txt");
  const markerJson = JSON.stringify(interruptMarker);
  const fakeAppServer = `
import { appendFileSync } from "node:fs";
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt" }, requiresOpenaiAuth: true } });
  else if (message.method === "model/list") send({ id: message.id, result: { data: [{ id: "gpt-5.6-terra" }] } });
  else if (message.method === "thread/start") {
    send({ id: message.id, result: { thread: { id: "review-thread-cli" } } });
    send({ method: "thread/started", params: { thread: { id: "review-thread-cli" } } });
  } else if (message.method === "turn/start") {
    send({ id: message.id, result: { turn: { id: "review-turn-cli", status: "inProgress" } } });
    send({ method: "turn/started", params: { threadId: "review-thread-cli", turn: { id: "review-turn-cli", status: "inProgress" } } });
  } else if (message.method === "turn/interrupt") {
    appendFileSync(${markerJson}, message.params.threadId + "/" + message.params.turnId + "\\n");
    send({ id: message.id, result: {} });
    send({ method: "turn/completed", params: { threadId: "review-thread-cli", turn: { id: "review-turn-cli", status: "interrupted", items: [] } } });
  }
}
`;
  await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
  const reviewPacket = packet({ cwd: root });
  const result = await runReviewerProcess(root, reviewPacket, [
    turnAuthorization(reviewPacket),
    exactTurnInterrupt(reviewPacket),
  ]);

  assert.equal(result.exitCode, 5, result.stderr);
  assert.equal(result.timedOut, false);
  assert.equal(result.records.at(-1).error, "MANAGED_REVIEW_CANCELLED_AFTER_EXACT_START");
  assert.deepEqual(
    result.records.filter((record) => record.event_type === "TURN_TERMINAL").map((record) => ({
      thread_id: record.thread_id,
      turn_id: record.turn_id,
      terminal_status: record.terminal_status,
    })),
    [{
      thread_id: "review-thread-cli",
      turn_id: "review-turn-cli",
      terminal_status: "interrupted",
    }],
  );
  assert.equal(await readFile(interruptMarker, "utf8"), "review-thread-cli/review-turn-cli\n");
});
