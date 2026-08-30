import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { managedConnectorTimeoutMs } from "../src/managed-codex-worker-bridge.mjs";

const bridgePath = fileURLToPath(new URL("../src/managed-codex-worker-bridge.mjs", import.meta.url));
const digest = (kind, character) => `${kind}:sha256:${character.repeat(64)}`;

function packet(cwd) {
  return {
    schema: "lattice.foreman-attempt-packet/1.0",
    task_ref: "taskref-phase4-cli",
    attempt: 1,
    project_ref: digest("project", "1"),
    spec_ref: digest("spec", "2"),
    approval_ref: digest("approval", "3"),
    budget_digest: digest("budget", "4"),
    global_active_limit: 4,
    per_task_active_limit: 1,
    repair_retry_limit: 2,
    max_duration_seconds: 900,
    max_total_tokens: 100_000,
    max_model_calls: 3,
    remaining_total_tokens: 100_000,
    remaining_model_calls: 3,
    external_cost_status: "UNAVAILABLE",
    external_cost_limit_micros: null,
    non_model_external_spend_allowed: false,
    verification_ref: digest("verification", "5"),
    worktree_ref: digest("worktree", "6"),
    execution_environment_ref: digest("execution-environment", "6"),
    base_commit: "a".repeat(40),
    packet_digest: digest("attempt-packet", "7"),
    model_reason_digest: digest("model-selection", "8"),
    model: "gpt-5.6-terra",
    reasoning: "medium",
    deadline_at: new Date(Date.now() + 60_000).toISOString(),
    heartbeat_timeout_ms: 30_000,
    writer_fence: 42,
    prior_terminal_evidence_ref: null,
    continuation: null,
    continuation_digest: null,
    cwd,
    prompt: "NEVER_ECHO_PROMPT_SENTINEL make the bounded edit.",
  };
}

function authContext() {
  return {
    schema: "lattice.managed-codex-auth-context/1.0",
    codex_home_digest: digest("codex-home", "9"),
    config_digest: digest("codex-config", "a"),
  };
}

function authorizeTurnStart(workerPacket, threadId = "thread-cli", overrides = {}) {
  return {
    schema: "lattice.managed-codex-worker-control/1.0",
    operation: "authorize_turn_start",
    task_ref: workerPacket.task_ref,
    attempt: workerPacket.attempt,
    packet_digest: workerPacket.packet_digest,
    thread_id: threadId,
    ...overrides,
  };
}

test("official connector startup timeout is task-bound and globally capped", () => {
  const bounded = packet(process.cwd());
  bounded.heartbeat_timeout_ms = 120_000;
  assert.equal(managedConnectorTimeoutMs(bounded), 120_000);
  bounded.heartbeat_timeout_ms = 86_400_000;
  assert.equal(managedConnectorTimeoutMs(bounded), 120_000);
  bounded.heartbeat_timeout_ms = 5_000;
  assert.equal(managedConnectorTimeoutMs(bounded), 5_000);
});

async function runBridge(cwd, command, environment = {}, {
  controls = [],
  controlDelayMs = 0,
  controlAfterEventType = null,
  endInput = true,
  timeoutMs = 5_000,
} = {}) {
  const child = spawn(process.execPath, [bridgePath], {
    cwd,
    env: { ...process.env, LATTICE_CODEX_BIN: process.execPath, ...environment },
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
  if (controlDelayMs > 0) await new Promise((resolve) => setTimeout(resolve, controlDelayMs));
  if (controlAfterEventType !== null) {
    const marker = `\"event_type\":\"${controlAfterEventType}\"`;
    const deadline = Date.now() + timeoutMs;
    while (!stdout.includes(marker) && child.exitCode === null && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    if (!stdout.includes(marker)) {
      child.kill();
      throw new Error(`bridge did not emit ${controlAfterEventType} before control deadline`);
    }
  }
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

test("bridge rejects missing sealed auth context before connector launch", async () => {
  const result = await runBridge(process.cwd(), {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "probe",
    packet: packet(process.cwd()),
  });

  assert.equal(result.exitCode, 2);
  assert.equal(result.records[0]?.code, "MANAGED_CODEX_INVALID_COMMAND");
});

test("JSONL bridge drives the real connector for start and exact resume without leaking inputs", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-managed-codex-bridge-"));
  const fakeAppServer = `
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt", email: "must-not-escape@example.invalid" }, requiresOpenaiAuth: true } });
  else if (message.method === "model/list") send({ id: message.id, result: { data: [{ id: "gpt-5.6-terra" }] } });
  else if (message.method === "thread/list") send({ id: message.id, result: { data: [], nextCursor: null } });
  else if (message.method === "thread/start") {
    if (process.env.FORBID_START === "1") process.exit(51);
    send({ id: message.id, result: { thread: { id: "thread-cli" } } });
    send({ method: "thread/started", params: { thread: { id: "thread-cli" } } });
  } else if (message.method === "turn/start") {
    send({ id: message.id, result: { turn: { id: "turn-cli", status: "inProgress" } } });
    send({ method: "turn/started", params: { threadId: "thread-cli", turn: { id: "turn-cli", status: "inProgress" } } });
    send({ method: "thread/tokenUsage/updated", params: { threadId: "thread-cli", turnId: "turn-cli", tokenUsage: { total: { inputTokens: 10, outputTokens: 2, totalTokens: 12 }, modelContextWindow: 200000 } } });
    send({ method: "turn/completed", params: { threadId: "thread-cli", turn: { id: "turn-cli", status: process.env.FAKE_TERMINAL_STATUS || "completed" } } });
  } else if (message.method === "thread/resume") {
    const status = process.env.FAKE_RESUME_ACTIVE === "1" ? "inProgress" : "completed";
    send({ id: message.id, result: { thread: { id: "thread-cli", cwd: ${JSON.stringify(root)}, turns: [{ id: "turn-cli", status }] } } });
  } else if (message.method === "thread/read") {
    const status = process.env.FAKE_RESUME_ACTIVE === "1" ? "inProgress" : "completed";
    send({ id: message.id, result: { thread: { id: "thread-cli", cwd: ${JSON.stringify(root)}, turns: [{ id: "turn-cli", status }] } } });
  } else if (message.method === "turn/interrupt") {
    if (process.env.FAKE_RESUME_ACTIVE !== "1") {
      send({ id: message.id, error: { code: -32000, message: "unexpected interrupt" } });
    } else {
      send({ id: message.id, result: {} });
      send({ method: "turn/completed", params: { threadId: "thread-cli", turn: { id: "turn-cli", status: "interrupted" } } });
    }
  }
}
`;
  await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
  const workerPacket = packet(root);

  const probed = await runBridge(root, {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "probe",
    auth_context: authContext(),
    packet: workerPacket,
  }, { FORBID_START: "1" });
  assert.equal(probed.exitCode, 0, probed.stderr);
  assert.equal(probed.records.length, 1);
  assert.equal(probed.records[0].kind, "result");
  assert.match(
    probed.records[0].result.auth_readiness.app_server_session_id,
    /^app-server-session:sha256:[a-f0-9]{64}$/u,
  );
  assert.deepEqual(probed.records[0].result, {
    model: "gpt-5.6-terra",
    available: true,
    auth_readiness: {
      schema: "lattice.managed-codex-auth-readiness/1.0",
      ready: true,
      auth_mode: "chatgpt",
      app_server_generation: 1,
      app_server_session_id: probed.records[0].result.auth_readiness.app_server_session_id,
      codex_home_digest: digest("codex-home", "9"),
      config_digest: digest("codex-config", "a"),
    },
  });
  assert.doesNotMatch(probed.stdout, /must-not-escape@example\.invalid/iu);

  const started = await runBridge(root, {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "start",
    auth_context: authContext(),
    claimed_at: new Date(Date.now() - 1_000).toISOString(),
    packet: workerPacket,
  }, {}, { controls: [authorizeTurnStart(workerPacket)] });
  assert.equal(started.exitCode, 0, started.stderr);
  assert.deepEqual(
    started.records.filter(({ kind }) => kind === "event").map(({ event }) => event.event_type),
    [
      "THREAD_START_ACCEPTED",
      "THREAD_STARTED",
      "TURN_START_ACCEPTED",
      "TURN_STARTED",
      "RESOURCE_OBSERVATION",
      "RESOURCE_OBSERVATION",
      "TURN_TERMINAL",
    ],
  );
  assert.equal(started.records.at(-1).kind, "result");
  assert.equal(started.records.at(-1).result.status, "completed");
  assert.doesNotMatch(started.stdout, /NEVER_ECHO_PROMPT_SENTINEL/iu);
  assert.equal(started.stdout.includes(root), false);

  const retainedStartedAt = new Date(Date.now() - 1_000);
  const retainedDeadlineAt = new Date(
    Math.min(
      retainedStartedAt.getTime() + (workerPacket.max_duration_seconds * 1_000),
      Date.parse(workerPacket.deadline_at),
    ),
  );
  const resumed = await runBridge(root, {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "resume",
    auth_context: authContext(),
    packet: workerPacket,
    retained: {
      task_ref: workerPacket.task_ref,
      attempt: workerPacket.attempt,
      packet_digest: workerPacket.packet_digest,
      thread_id: "thread-cli",
      turn_id: "turn-cli",
      attempt_started_at: retainedStartedAt.toISOString(),
      attempt_deadline_at: retainedDeadlineAt.toISOString(),
      last_heartbeat_at: retainedStartedAt.toISOString(),
      last_meaningful_progress_at: retainedStartedAt.toISOString(),
    },
  }, { FORBID_START: "1" });
  assert.equal(resumed.exitCode, 0, `${resumed.stderr}\n${resumed.stdout}`);
  assert.deepEqual(
    resumed.records.filter(({ kind }) => kind === "event").map(({ event }) => event.event_type),
    ["RECONCILE_STARTED", "RECONCILED_TERMINAL"],
  );
  assert.equal(resumed.records.at(-1).result.status, "completed");
  assert.doesNotMatch(resumed.stdout, /THREAD_START_ACCEPTED|TURN_START_ACCEPTED|NEVER_ECHO_PROMPT_SENTINEL/iu);
  assert.equal(resumed.stdout.includes(root), false);

  const interrupted = await runBridge(root, {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "resume",
    auth_context: authContext(),
    packet: workerPacket,
    retained: {
      task_ref: workerPacket.task_ref,
      attempt: workerPacket.attempt,
      packet_digest: workerPacket.packet_digest,
      thread_id: "thread-cli",
      turn_id: "turn-cli",
      attempt_started_at: retainedStartedAt.toISOString(),
      attempt_deadline_at: retainedDeadlineAt.toISOString(),
      last_heartbeat_at: retainedStartedAt.toISOString(),
      last_meaningful_progress_at: retainedStartedAt.toISOString(),
    },
  }, { FORBID_START: "1", FAKE_RESUME_ACTIVE: "1" }, {
    controls: [{
      schema: "lattice.managed-codex-worker-control/1.0",
      operation: "interrupt",
      task_ref: workerPacket.task_ref,
      attempt: workerPacket.attempt,
      packet_digest: workerPacket.packet_digest,
      thread_id: "thread-cli",
      turn_id: "turn-cli",
    }],
    controlAfterEventType: "RECONCILED_ACTIVE",
  });
  assert.equal(interrupted.exitCode, 6, interrupted.stderr);
  assert.deepEqual(
    interrupted.records.filter(({ kind }) => kind === "event").map(({ event }) => event.event_type),
    [
      "RECONCILE_STARTED",
      "RECONCILED_ACTIVE",
      "INTERRUPT_REQUESTED",
      "INTERRUPT_TERMINAL",
      "TURN_TERMINAL",
    ],
  );
  assert.equal(interrupted.records.at(-1).result.status, "interrupted");

  const failed = await runBridge(root, {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "start",
    auth_context: authContext(),
    claimed_at: new Date(Date.now() - 1_000).toISOString(),
    packet: workerPacket,
  }, { FAKE_TERMINAL_STATUS: "failed" }, { controls: [authorizeTurnStart(workerPacket)] });
  assert.equal(failed.exitCode, 6);
  assert.equal(failed.records.at(-1).kind, "result");
  assert.equal(failed.records.at(-1).result.status, "failed");

  const invalid = await runBridge(root, {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "start",
    auth_context: authContext(),
    claimed_at: new Date(Date.now() - 1_000).toISOString(),
    packet: { prompt: "NEVER_ECHO_PROMPT_SENTINEL" },
  });
  assert.equal(invalid.exitCode, 2);
  assert.equal(invalid.records.length, 1);
  assert.equal(invalid.records[0].kind, "error");
  assert.equal(invalid.records[0].category, 2);
  assert.doesNotMatch(invalid.stdout, /NEVER_ECHO_PROMPT_SENTINEL/iu);
});

test("bridge reconnects one exited App Server to the exact turn and reports bounded exhaustion", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-managed-codex-bridge-disconnect-"));
  const fakeAppServer = `
import readline from "node:readline";
import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
const statePath = process.env.FAKE_STATE_PATH;
const logPath = process.env.FAKE_LOG_PATH;
const prior = existsSync(statePath) ? JSON.parse(readFileSync(statePath, "utf8")) : { generation: 0 };
const generation = prior.generation + 1;
writeFileSync(statePath, JSON.stringify({ generation }), "utf8");
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
const record = (method) => appendFileSync(logPath, method + "\\n", "utf8");
const activeThread = { id: "thread-cli", cwd: ${JSON.stringify(root)}, turns: [{ id: "turn-cli", status: "inProgress" }] };
let reads = 0;
for await (const line of lines) {
  const message = JSON.parse(line);
  record(message.method || "response");
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt", email: "must-not-escape@example.invalid" }, requiresOpenaiAuth: true } });
  else if (message.method === "model/list") send({ id: message.id, result: { data: [{ id: "gpt-5.6-terra" }] } });
  else if (message.method === "thread/list") send({ id: message.id, result: { data: [], nextCursor: null } });
  else if (message.method === "thread/start") {
    send({ id: message.id, result: { thread: { id: "thread-cli" } } });
    send({ method: "thread/started", params: { thread: { id: "thread-cli" } } });
  } else if (message.method === "turn/start") {
    send({ id: message.id, result: { turn: { id: "turn-cli", status: "inProgress" } } });
    send({ method: "turn/started", params: { threadId: "thread-cli", turn: { id: "turn-cli", status: "inProgress" } } });
    setTimeout(() => process.exit(91), 10);
  } else if (message.method === "thread/resume") {
    if (process.env.FAKE_RECONCILE_MODE === "exhaust") process.exit(92);
    send({ id: message.id, result: { thread: activeThread } });
  } else if (message.method === "thread/read") {
    reads += 1;
    send({ id: message.id, result: { thread: activeThread } });
    if (reads === 2) {
      setTimeout(() => send({ method: "turn/completed", params: { threadId: "thread-cli", turn: { id: "turn-cli", status: "completed" } } }), 20);
    }
  } else if (message.method === "turn/interrupt") process.exit(93);
}
`;
  await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
  const workerPacket = packet(root);
  const command = {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "start",
    auth_context: authContext(),
    claimed_at: new Date(Date.now() - 1_000).toISOString(),
    packet: workerPacket,
  };

  const recoveredState = path.join(root, "recovered-state.json");
  const recoveredLog = path.join(root, "recovered.log");
  const recovered = await runBridge(root, command, {
    FAKE_STATE_PATH: recoveredState,
    FAKE_LOG_PATH: recoveredLog,
  }, {
    controls: [authorizeTurnStart(workerPacket)],
    timeoutMs: 15_000,
  });
  assert.equal(recovered.exitCode, 0, recovered.stderr);
  assert.equal(recovered.records.at(-1).kind, "result");
  assert.equal(recovered.records.at(-1).result.status, "completed");
  assert.deepEqual(
    recovered.records.filter(({ kind }) => kind === "event").map(({ event }) => event.event_type),
    [
      "THREAD_START_ACCEPTED",
      "THREAD_STARTED",
      "TURN_START_ACCEPTED",
      "TURN_STARTED",
      "RECONCILE_STARTED",
      "RECONCILED_ACTIVE",
      "TURN_TERMINAL",
    ],
  );
  const recoveredMethods = (await readFile(recoveredLog, "utf8")).trim().split(/\r?\n/u);
  assert.equal(recoveredMethods.filter((method) => method === "thread/start").length, 1);
  assert.equal(recoveredMethods.filter((method) => method === "turn/start").length, 1);
  assert.equal(recoveredMethods.filter((method) => method === "thread/resume").length, 1);
  assert.equal(recoveredMethods.filter((method) => method === "thread/read").length, 2);
  assert.equal(recoveredMethods.filter((method) => method === "turn/interrupt").length, 0);

  const exhaustedState = path.join(root, "exhausted-state.json");
  const exhaustedLog = path.join(root, "exhausted.log");
  const exhausted = await runBridge(root, command, {
    FAKE_STATE_PATH: exhaustedState,
    FAKE_LOG_PATH: exhaustedLog,
    FAKE_RECONCILE_MODE: "exhaust",
  }, {
    controls: [authorizeTurnStart(workerPacket)],
    timeoutMs: 15_000,
  });
  assert.equal(exhausted.exitCode, 5, exhausted.stderr);
  assert.equal(exhausted.records.at(-1).kind, "error");
  assert.equal(
    exhausted.records.at(-1).code,
    "MANAGED_CODEX_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED",
  );
  const exhaustedMethods = (await readFile(exhaustedLog, "utf8")).trim().split(/\r?\n/u);
  assert.equal(exhaustedMethods.filter((method) => method === "thread/start").length, 1);
  assert.equal(exhaustedMethods.filter((method) => method === "turn/start").length, 1);
  assert.equal(exhaustedMethods.filter((method) => method === "thread/resume").length, 2);
  assert.equal(exhaustedMethods.filter((method) => method === "turn/interrupt").length, 0);
});

test("production bridge refuses to start a turn until one exact durable authorization arrives", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-managed-codex-bridge-ack-"));
  const fakeAppServer = `
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt", email: "must-not-escape@example.invalid" }, requiresOpenaiAuth: true } });
  else if (message.method === "model/list") send({ id: message.id, result: { data: [{ id: "gpt-5.6-terra" }] } });
  else if (message.method === "thread/list") send({ id: message.id, result: { data: [], nextCursor: null } });
  else if (message.method === "thread/start") {
    send({ id: message.id, result: { thread: { id: "thread-cli" } } });
    send({ method: "thread/started", params: { thread: { id: "thread-cli" } } });
  } else if (message.method === "turn/start") {
    send({ id: message.id, result: { turn: { id: "turn-cli", status: "inProgress" } } });
    send({ method: "turn/started", params: { threadId: "thread-cli", turn: { id: "turn-cli", status: "inProgress" } } });
    send({ method: "turn/completed", params: { threadId: "thread-cli", turn: { id: "turn-cli", status: "completed" } } });
  }
}
`;
  await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
  const workerPacket = packet(root);
  const command = {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "start",
    auth_context: authContext(),
    claimed_at: new Date(Date.now() - 1_000).toISOString(),
    packet: workerPacket,
  };

  const noAuthorization = await runBridge(root, command, {}, { timeoutMs: 5_000 });
  assert.equal(noAuthorization.timedOut, true);
  assert.deepEqual(
    noAuthorization.records.filter(({ kind }) => kind === "event").map(({ event }) => event.event_type),
    ["THREAD_START_ACCEPTED", "THREAD_STARTED"],
  );
  assert.equal(noAuthorization.stdout.includes("TURN_START_ACCEPTED"), false);

  const wrongAuthorization = await runBridge(root, command, {}, {
    controls: [authorizeTurnStart(workerPacket, "thread-other")],
  });
  assert.equal(wrongAuthorization.exitCode, 5);
  assert.equal(wrongAuthorization.records.at(-1).kind, "error");
  assert.equal(
    wrongAuthorization.records.at(-1).code,
    "MANAGED_CODEX_TURN_START_AUTHORIZATION_REJECTED",
  );
  assert.equal(wrongAuthorization.stdout.includes("TURN_START_ACCEPTED"), false);

  const exactAuthorization = await runBridge(root, command, {}, {
    controls: [authorizeTurnStart(workerPacket)],
  });
  assert.equal(exactAuthorization.exitCode, 0, exactAuthorization.stderr);
  assert.equal(
    exactAuthorization.records.filter(({ kind }) => kind === "event" && kind).filter(
      ({ event }) => event.event_type === "TURN_START_ACCEPTED",
    ).length,
    1,
  );
});

test("production bridge exposes only bounded turn-start RPC rejection evidence", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-managed-codex-bridge-rpc-reject-"));
  const fakeAppServer = `
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt", email: "must-not-escape@example.invalid" }, requiresOpenaiAuth: true } });
  else if (message.method === "model/list") send({ id: message.id, result: { data: [{ id: "gpt-5.6-terra" }] } });
  else if (message.method === "thread/list") send({ id: message.id, result: { data: [], nextCursor: null } });
  else if (message.method === "thread/start") {
    send({ id: message.id, result: { thread: { id: "thread-cli" } } });
    send({ method: "thread/started", params: { thread: { id: "thread-cli" } } });
  } else if (message.method === "turn/start") {
    send({ id: message.id, error: { code: -32602, message: "UNTRUSTED_RPC_MESSAGE_SENTINEL", data: { prompt: "UNTRUSTED_RPC_DATA_SENTINEL" } } });
  }
}
`;
  await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
  const workerPacket = packet(root);
  const rejected = await runBridge(root, {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "start",
    auth_context: authContext(),
    claimed_at: new Date(Date.now() - 1_000).toISOString(),
    packet: workerPacket,
  }, {}, { controls: [authorizeTurnStart(workerPacket)] });

  assert.equal(rejected.exitCode, 5, rejected.stderr);
  assert.deepEqual(rejected.records.at(-1), {
    schema: "lattice.managed-codex-worker-bridge-result/1.0",
    kind: "error",
    category: 5,
    code: "CODEX_APP_SERVER_RPC_REJECTED",
    provider_method: "turn/start",
    provider_rpc_code: -32602,
    task_ref: workerPacket.task_ref,
    attempt: workerPacket.attempt,
    packet_digest: workerPacket.packet_digest,
    message: "managed Codex worker bridge failed closed",
  });
  assert.doesNotMatch(rejected.stdout, /UNTRUSTED_(?:PROMPT|RPC|DATA|MESSAGE)/u);
});

test("production bridge classifies start RPC and exact-start notification timeouts separately", async () => {
  for (const mode of ["turn-start-rpc", "turn-started-notification"]) {
    const root = await mkdtemp(path.join(tmpdir(), `lattice-managed-codex-bridge-${mode}-`));
    const fakeAppServer = `
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt", email: "must-not-escape@example.invalid" }, requiresOpenaiAuth: true } });
  else if (message.method === "model/list") send({ id: message.id, result: { data: [{ id: "gpt-5.6-terra" }] } });
  else if (message.method === "thread/list") send({ id: message.id, result: { data: [], nextCursor: null } });
  else if (message.method === "thread/start") {
    send({ id: message.id, result: { thread: { id: "thread-cli" } } });
    send({ method: "thread/started", params: { thread: { id: "thread-cli" } } });
  } else if (message.method === "turn/start" && process.env.FAKE_TIMEOUT_MODE === "turn-started-notification") {
    send({ id: message.id, result: { turn: { id: "turn-cli", status: "inProgress" } } });
  }
}
    `;
    await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
    const workerPacket = packet(root);
    workerPacket.heartbeat_timeout_ms = 3_000;
    const timedOut = await runBridge(root, {
      schema: "lattice.managed-codex-worker-command/1.0",
      operation: "start",
      auth_context: authContext(),
      claimed_at: new Date(Date.now() - 1_000).toISOString(),
      packet: workerPacket,
    }, { FAKE_TIMEOUT_MODE: mode }, {
      controls: [authorizeTurnStart(workerPacket)],
      timeoutMs: 10_000,
    });

    assert.equal(timedOut.exitCode, 5, timedOut.stderr);
    assert.equal(timedOut.records.at(-1).code, "CODEX_APP_SERVER_TIMEOUT");
    assert.equal(
      timedOut.records.at(-1).provider_method,
      mode === "turn-start-rpc" ? "turn/start" : "turn/started",
    );
    assert.equal(timedOut.records.at(-1).provider_rpc_code, undefined);
    assert.doesNotMatch(timedOut.stdout, /UNTRUSTED_PROMPT_SENTINEL/u);
  }
});

test("production bridge preserves exact-start correlation when the App Server exits before notification", async () => {
  for (const mode of ["thread-started-eof", "turn-started-eof"]) {
    const root = await mkdtemp(path.join(tmpdir(), `lattice-managed-codex-bridge-${mode}-`));
    const fakeAppServer = `
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt" }, requiresOpenaiAuth: true } });
  else if (message.method === "model/list") send({ id: message.id, result: { data: [{ id: "gpt-5.6-terra" }] } });
  else if (message.method === "thread/list") send({ id: message.id, result: { data: [], nextCursor: null } });
  else if (message.method === "thread/start") {
    send({ id: message.id, result: { thread: { id: "thread-cli" } } });
    if (process.env.FAKE_EOF_MODE === "thread-started-eof") setTimeout(() => process.exit(52), 25);
    else send({ method: "thread/started", params: { thread: { id: "thread-cli" } } });
  } else if (message.method === "turn/start") {
    send({ id: message.id, result: { turn: { id: "turn-cli", status: "inProgress" } } });
    if (process.env.FAKE_EOF_MODE === "turn-started-eof") setTimeout(() => process.exit(53), 25);
  }
}
`;
    await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
    const workerPacket = packet(root);
    workerPacket.heartbeat_timeout_ms = 1_000;
    const exited = await runBridge(root, {
      schema: "lattice.managed-codex-worker-command/1.0",
      operation: "start",
      auth_context: authContext(),
      claimed_at: new Date(Date.now() - 1_000).toISOString(),
      packet: workerPacket,
    }, { FAKE_EOF_MODE: mode }, { controls: [authorizeTurnStart(workerPacket)] });

    assert.equal(exited.exitCode, 5, exited.stderr);
    assert.equal(exited.records.at(-1).code, "CODEX_APP_SERVER_PROCESS_EXITED");
    assert.equal(
      exited.records.at(-1).provider_method,
      mode === "thread-started-eof" ? "thread/started" : "turn/started",
    );
    assert.equal(exited.records.some(({ event_type: type }) => type === "TURN_STARTED"), false);
  }
});

test("restart-only bridge reports proven no provider candidate without a fresh start", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-managed-codex-bridge-recover-none-"));
  const fakeAppServer = `
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt" }, requiresOpenaiAuth: true } });
  else if (message.method === "thread/list") send({ id: message.id, result: { data: [], nextCursor: null } });
  else if (message.method === "thread/start" || message.method === "turn/start") process.exit(51);
}
`;
  await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
  const workerPacket = packet(root);

  const recovered = await runBridge(root, {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "recover-dispatch",
    auth_context: authContext(),
    claimed_at: new Date(Date.now() - 1_000).toISOString(),
    packet: workerPacket,
  });

  assert.equal(recovered.exitCode, 0, recovered.stderr);
  assert.equal(recovered.records.at(-1).kind, "result");
  assert.deepEqual(recovered.records.at(-1).result, {
    kind: "PROVEN_NO_PROVIDER_CANDIDATE",
  });
  assert.doesNotMatch(recovered.stdout, /THREAD_START_ACCEPTED|TURN_START_ACCEPTED/iu);
});

test("restart-only bridge rejects an unattributable empty provider thread without a new provider effect", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-managed-codex-bridge-recover-empty-"));
  const createdAt = Math.floor((Date.now() - 500) / 1_000);
  const fakeAppServer = `
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt" }, requiresOpenaiAuth: true } });
  else if (message.method === "thread/list") send({ id: message.id, result: { data: [{ id: "thread-recovered-empty", cwd: ${JSON.stringify(root)}, createdAt: ${createdAt}, turns: [] }], nextCursor: null } });
  else if (message.method === "thread/read") send({ id: message.id, result: { thread: { id: "thread-recovered-empty", cwd: ${JSON.stringify(root)}, createdAt: ${createdAt}, turns: [] } } });
  else if (message.method === "thread/resume") process.exit(52);
  else if (message.method === "thread/start" || message.method === "turn/start") process.exit(51);
}
`;
  await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
  const workerPacket = packet(root);

  const recovered = await runBridge(root, {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "recover-dispatch",
    auth_context: authContext(),
    claimed_at: new Date((createdAt * 1_000) - 1_000).toISOString(),
    packet: workerPacket,
  });

  assert.equal(recovered.exitCode, 4, recovered.stderr);
  assert.equal(recovered.records.at(-1).kind, "error");
  assert.equal(
    recovered.records.at(-1).code,
    "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
  );
  assert.equal(recovered.records.some(({ kind }) => kind === "event"), false);
  assert.doesNotMatch(recovered.stdout, /THREAD_START_ACCEPTED|TURN_START_ACCEPTED|TURN_STARTED/iu);
});

test("restart-only bridge closes a retained terminal turn as failed-start without exact-start evidence", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-managed-codex-bridge-prestart-terminal-"));
  const createdAt = Math.floor((Date.now() - 500) / 1_000);
  const fakeAppServer = `
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
const send = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
const thread = { id: "thread-prestart-terminal", cwd: ${JSON.stringify(root)}, createdAt: ${createdAt}, turns: [{ id: "turn-prestart-terminal", status: "completed" }] };
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "initialize") send({ id: message.id, result: { platformFamily: "windows" } });
  else if (message.method === "account/read") send({ id: message.id, result: { account: { type: "chatgpt" }, requiresOpenaiAuth: true } });
  else if (message.method === "thread/read" || message.method === "thread/resume") send({ id: message.id, result: { thread } });
  else if (message.method === "thread/start" || message.method === "turn/start") process.exit(51);
}
`;
  await writeFile(path.join(root, "app-server"), fakeAppServer, "utf8");
  const workerPacket = packet(root);

  const recovered = await runBridge(root, {
    schema: "lattice.managed-codex-worker-command/1.0",
    operation: "recover-prestart",
    auth_context: authContext(),
    packet: workerPacket,
    retained: {
      task_ref: workerPacket.task_ref,
      attempt: workerPacket.attempt,
      packet_digest: workerPacket.packet_digest,
      thread_id: "thread-prestart-terminal",
      turn_id: "turn-prestart-terminal",
    },
  });

  assert.equal(recovered.exitCode, 6, recovered.stderr);
  assert.deepEqual(
    recovered.records.filter(({ kind }) => kind === "event").map(({ event }) => event.event_type),
    ["THREAD_START_ACCEPTED", "TURN_START_ACCEPTED", "PRESTART_TERMINAL"],
  );
  assert.equal(recovered.records.at(-1).result.status, "failed");
  assert.equal(recovered.records.at(-1).result.provider_terminal_status, "completed");
  assert.doesNotMatch(recovered.stdout, /TURN_STARTED/iu);
});
