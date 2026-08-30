import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import test from "node:test";

import {
  ManagedCodexWorkerTransport,
  normalizeCodexMeaningfulProgress,
  normalizeCodexResourceObservation,
  validateManagedCodexWorkerPacket,
} from "../src/managed-codex-worker.mjs";

const digest = (kind, character) => `${kind}:sha256:${character.repeat(64)}`;
const CLAIMED_AT = "2026-08-26T14:00:00Z";
const claimedSecond = Math.floor(Date.parse(CLAIMED_AT) / 1_000);

function authContext() {
  return Object.freeze({
    schema: "lattice.managed-codex-auth-context/1.0",
    codex_home_digest: digest("codex-home", "9"),
    config_digest: digest("codex-config", "a"),
  });
}

function marker(workerPacket) {
  return `[LATTICE_MANAGED_ATTEMPT task_ref=${workerPacket.task_ref} attempt=${workerPacket.attempt} packet_digest=${workerPacket.packet_digest}]`;
}

function markerThread(workerPacket, status = "inProgress") {
  return {
    id: "thread-marker-recovered",
    cwd: workerPacket.cwd,
    createdAt: claimedSecond,
    turns: [{
      id: "turn-marker-recovered",
      status,
      items: [{
        type: "userMessage",
        content: [{ type: "text", text: `${marker(workerPacket)}\nopaque bounded objective` }],
      }],
    }],
  };
}

function packet(overrides = {}) {
  return {
    schema: "lattice.foreman-attempt-packet/1.0",
    task_ref: "taskref-phase4-001",
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
    deadline_at: "2026-08-26T14:30:00Z",
    heartbeat_timeout_ms: 30_000,
    writer_fence: 42,
    prior_terminal_evidence_ref: null,
    continuation: null,
    continuation_digest: null,
    cwd: "C:\\disposable\\repo",
    prompt: "Make the bounded local change and stop after the focused test.",
    ...overrides,
  };
}

function retainedAttempt(
  overrides = {},
  { includeProgress = true, includeHeartbeat = true } = {},
) {
  const retained = {
    task_ref: "taskref-phase4-001",
    attempt: 1,
    packet_digest: digest("attempt-packet", "7"),
    thread_id: "thread-retained",
    turn_id: "turn-retained",
    attempt_started_at: "2026-08-26T14:00:00Z",
    attempt_deadline_at: "2026-08-26T14:15:00Z",
    ...overrides,
  };
  if (includeProgress && !Object.hasOwn(retained, "last_meaningful_progress_at")) {
    retained.last_meaningful_progress_at = "2026-08-26T14:00:00Z";
  }
  if (includeHeartbeat && !Object.hasOwn(retained, "last_heartbeat_at")) {
    retained.last_heartbeat_at = "2026-08-26T14:00:00Z";
  }
  return retained;
}

class ScriptedCodex extends EventEmitter {
  constructor({ resumeStatus = "inProgress", terminalStatus = "completed" } = {}) {
    super();
    this.calls = [];
    this.resumeStatus = resumeStatus;
    this.terminalStatus = terminalStatus;
    this.active = false;
    this.connectionGeneration = 1;
    this.appServerSessionId = `app-server-session:sha256:${"8".repeat(64)}`;
  }

  async connect() {
    this.calls.push(["connect"]);
  }

  async readAuthReadiness() {
    return Object.freeze({
      schema: "lattice.codex-auth-readiness/1.0",
      ready: true,
      authMode: "chatgpt",
      appServerGeneration: 1,
      appServerSessionId: this.appServerSessionId,
    });
  }

  async listThreads(options) {
    this.calls.push(["listThreads", options]);
    return { data: [], nextCursor: null };
  }

  async startThread(options) {
    this.calls.push(["startThread", options]);
    return { id: "thread-exact" };
  }

  async waitForThreadStarted(threadId) {
    this.calls.push(["waitForThreadStarted", threadId]);
    return { id: threadId };
  }

  async startTurn(threadId, prompt) {
    this.calls.push(["startTurn", threadId, prompt]);
    return { id: "turn-exact", status: "inProgress" };
  }

  async waitForTurnStarted(threadId, turnId) {
    this.calls.push(["waitForTurnStarted", threadId, turnId]);
    this.active = true;
    return { id: turnId, status: "inProgress" };
  }

  async waitForTurnCompleted(threadId, turnId, options) {
    this.calls.push(["waitForTurnCompleted", threadId, turnId, options]);
    this.active = false;
    return { id: turnId, status: this.terminalStatus };
  }

  async resumeThread(threadId, { expectedTurnId } = {}) {
    this.calls.push(["resumeThread", threadId, expectedTurnId]);
    this.active = this.resumeStatus === "inProgress";
    return {
      id: threadId,
      cwd: "C:\\disposable\\repo",
      turns: [{ id: expectedTurnId, status: this.resumeStatus }],
    };
  }

  async readThread(threadId, options = {}) {
    this.calls.push(["readThread", threadId, options]);
    return {
      id: threadId,
      cwd: "C:\\disposable\\repo",
      turns: [{ id: "turn-exact", status: this.active ? "inProgress" : this.terminalStatus }],
    };
  }

  isTurnActive(threadId, turnId) {
    this.calls.push(["isTurnActive", threadId, turnId]);
    return this.active;
  }

  async interruptTurn(threadId, turnId) {
    this.calls.push(["interruptTurn", threadId, turnId]);
    this.active = false;
    return { id: turnId, status: "interrupted" };
  }
}

function transport(codex, events, overrides = {}) {
  return new ManagedCodexWorkerTransport({
    codex,
    authContext: authContext(),
    availableModels: ["gpt-5.6-luna", "gpt-5.6-terra", "gpt-5.6-sol"],
    eventSink: async (event) => events.push(event),
    now: () => "2026-08-26T14:00:00Z",
    lifecycleTimeoutMs: 500,
    dispatchBackoffMs: 0,
    ...overrides,
  });
}

test("packet validation is strict and unavailable or unapproved models never start", async () => {
  assert.equal(validateManagedCodexWorkerPacket(packet()).model, "gpt-5.6-terra");
  for (const model of ["gpt-5.6", "gpt-5.5", "gpt-5.6-terra-latest"]) {
    assert.throws(
      () => validateManagedCodexWorkerPacket(packet({ model })),
      /model.*allowlist/iu,
    );
  }
  assert.throws(
    () => validateManagedCodexWorkerPacket(packet({ prompt: "Bearer ghp_abcdefghijklmnopqrstuvwxyz123456" })),
    /secret|credential/iu,
  );
  assert.throws(
    () => validateManagedCodexWorkerPacket(packet({ cwd: "relative/repo" })),
    /absolute.*worktree|cwd/iu,
  );
  assert.throws(
    () => validateManagedCodexWorkerPacket(packet({ base_commit: "HEAD" })),
    /base_commit/iu,
  );
  assert.throws(
    () => validateManagedCodexWorkerPacket(packet({ max_model_calls: 0 })),
    /model-call bounds/iu,
  );
  assert.throws(
    () => validateManagedCodexWorkerPacket(packet({ remaining_total_tokens: 0 })),
    /token.*model-call bounds/iu,
  );
  assert.throws(
    () => validateManagedCodexWorkerPacket(packet({ remaining_total_tokens: 100_001 })),
    /token.*model-call bounds/iu,
  );
  assert.throws(
    () => validateManagedCodexWorkerPacket(packet({ remaining_model_calls: 0 })),
    /token.*model-call bounds/iu,
  );
  assert.throws(
    () => validateManagedCodexWorkerPacket(packet({ non_model_external_spend_allowed: true })),
    /external-cost policy/iu,
  );
  assert.equal(
    validateManagedCodexWorkerPacket(packet({
      external_cost_status: "LIMIT_MICROS",
      external_cost_limit_micros: 0,
    })).external_cost_limit_micros,
    0,
  );
  assert.throws(
    () => validateManagedCodexWorkerPacket(packet({
      external_cost_status: "LIMIT_MICROS",
      external_cost_limit_micros: -1,
    })),
    /external-cost policy/iu,
  );
  const repair = validateManagedCodexWorkerPacket(packet({
    attempt: 2,
    prior_terminal_evidence_ref: digest("evidence", "9"),
    continuation: "Preserve verified work and repair only the closed failure.",
    continuation_digest: digest("continuation", "a"),
  }));
  assert.equal(repair.attempt, 2);
  assert.throws(
    () => validateManagedCodexWorkerPacket(packet({ attempt: 2 })),
    /repair continuation|prior_terminal/iu,
  );

  const codex = new ScriptedCodex();
  const events = [];
  const worker = transport(codex, events, { availableModels: ["gpt-5.6-luna"] });
  await assert.rejects(worker.start(packet(), CLAIMED_AT), (error) => {
    assert.equal(error.code, "MANAGED_CODEX_MODEL_UNAVAILABLE");
    return true;
  });
  assert.deepEqual(codex.calls, []);
  assert.deepEqual(events, []);
});

test("missing keyring-backed account readiness blocks before any thread provider effect", async () => {
  const codex = new ScriptedCodex();
  codex.readAuthReadiness = async () => Object.freeze({
    schema: "lattice.codex-auth-readiness/1.0",
    ready: false,
    authMode: null,
    appServerGeneration: 1,
    appServerSessionId: codex.appServerSessionId,
  });
  const events = [];

  await assert.rejects(transport(codex, events).start(packet(), CLAIMED_AT), (error) => {
    assert.equal(error.code, "MANAGED_CODEX_AUTH_READINESS_NOT_VERIFIED");
    return true;
  });

  assert.deepEqual(codex.calls, []);
  assert.deepEqual(events, []);
});

test("provider effects fail closed when the App Server identity changes after readiness", async () => {
  class IdentityDriftCodex extends ScriptedCodex {
    async startThread(options) {
      assert.deepEqual(options.effectIdentity, {
        expectedGeneration: 1,
        expectedSessionId: `app-server-session:sha256:${"8".repeat(64)}`,
      });
      this.connectionGeneration = 2;
      this.appServerSessionId = `app-server-session:sha256:${"9".repeat(64)}`;
      const error = new Error("identity changed before provider dispatch");
      error.code = "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED";
      throw error;
    }
  }

  const codex = new IdentityDriftCodex();
  const events = [];
  await assert.rejects(transport(codex, events).start(packet(), CLAIMED_AT), (error) => {
    assert.equal(error.code, "MANAGED_CODEX_AUTH_EFFECT_IDENTITY_CHANGED");
    return true;
  });
  assert.deepEqual(events, []);
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
});

test("probe returns only generation and sealed home/config credential readiness", async () => {
  const codex = new ScriptedCodex();
  const result = await transport(codex, []).probe(packet());

  assert.deepEqual(result, {
    model: "gpt-5.6-terra",
    available: true,
    auth_readiness: {
      schema: "lattice.managed-codex-auth-readiness/1.0",
      ready: true,
      auth_mode: "chatgpt",
      app_server_generation: 1,
      app_server_session_id: `app-server-session:sha256:${"8".repeat(64)}`,
      codex_home_digest: digest("codex-home", "9"),
      config_digest: digest("codex-config", "a"),
    },
  });
});

test("start emits bounded evidence in accepted then exact-started then terminal order", async () => {
  const codex = new ScriptedCodex();
  const events = [];
  const result = await transport(codex, events).start(packet(), CLAIMED_AT);

  assert.deepEqual(
    codex.calls.map(([operation]) => operation),
    [
      "listThreads",
      "listThreads",
      "listThreads",
      "startThread",
      "waitForThreadStarted",
      "startTurn",
      "waitForTurnStarted",
      "waitForTurnCompleted",
    ],
  );
  assert.deepEqual(codex.calls[3][1], {
    cwd: "C:\\disposable\\repo",
    model: "gpt-5.6-terra",
    approvalPolicy: "never",
    sandbox: "workspace-write",
    ephemeral: false,
    serviceName: "lattice_managed_foreman",
    developerInstructions: [
      "Operate only inside the supplied worktree and bounded task packet.",
      "Do not push, merge, deploy, publish, pay, send external messages, or permanently delete data.",
    ].join(" "),
    config: {
      model_reasoning_effort: "medium",
      web_search: "disabled",
      sandbox_workspace_write: { network_access: false },
    },
    effectIdentity: {
      expectedGeneration: 1,
      expectedSessionId: `app-server-session:sha256:${"8".repeat(64)}`,
    },
  });
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    [
      "THREAD_START_ACCEPTED",
      "THREAD_STARTED",
      "TURN_START_ACCEPTED",
      "TURN_STARTED",
      "TURN_TERMINAL",
    ],
  );
  assert.equal(result.thread_id, "thread-exact");
  assert.equal(result.turn_id, "turn-exact");
  assert.equal(result.status, "completed");
  for (const event of events) {
    const encoded = JSON.stringify(event);
    assert.ok(encoded.length <= 4_096);
    assert.doesNotMatch(encoded, /bounded local change|disposable\\repo|Bearer|password/iu);
    assert.match(event.evidence_digest, /^managed-worker-event:sha256:[a-f0-9]{64}$/u);
    assert.equal(event.project_ref, digest("project", "1"));
    assert.equal(event.spec_ref, digest("spec", "2"));
    assert.equal(event.approval_ref, digest("approval", "3"));
    assert.equal(event.base_commit, "a".repeat(40));
  }
});

test("dispatch accepts exact PostgreSQL sub-millisecond claim time without weakening canonical time", async () => {
  const codex = new ScriptedCodex();
  const events = [];
  const result = await transport(codex, events).start(
    packet(),
    "2026-08-26T14:00:00.8419858Z",
  );
  assert.equal(result.status, "completed");
  assert.equal(codex.calls.filter(([operation]) => operation === "startThread").length, 1);

  const rejected = new ScriptedCodex();
  await assert.rejects(
    transport(rejected, []).start(packet(), "2026-08-26T14:00:00.8419850Z"),
    /canonical UTC timestamp/u,
  );
  assert.deepEqual(rejected.calls, []);
});

test("turn start waits for the durable thread-acceptance authorization", async () => {
  const codex = new ScriptedCodex();
  const events = [];
  let releaseAuthorization;
  const authorization = new Promise((resolve) => { releaseAuthorization = resolve; });
  const worker = transport(codex, events, {
    turnStartAuthorizer: async (identity) => {
      assert.deepEqual(identity, {
        task_ref: "taskref-phase4-001",
        attempt: 1,
        packet_digest: digest("attempt-packet", "7"),
        thread_id: "thread-exact",
      });
      await authorization;
    },
  });

  const pending = worker.start(packet(), CLAIMED_AT);
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    ["THREAD_START_ACCEPTED", "THREAD_STARTED"],
  );

  releaseAuthorization();
  const result = await pending;
  assert.equal(result.status, "completed");
  assert.equal(codex.calls.filter(([operation]) => operation === "startTurn").length, 1);
});

test("continue-turn resumes only one retained empty thread before authorization", async () => {
  const workerPacket = packet();
  const empty = {
    id: "thread-retained-empty",
    cwd: workerPacket.cwd,
    createdAt: claimedSecond,
    turns: [],
  };
  const codex = new ScriptedCodex();
  codex.resumeEmptyThread = async (threadId) => {
    codex.calls.push(["resumeEmptyThread", threadId]);
    return structuredClone(empty);
  };
  const events = [];
  const result = await transport(codex, events).continueTurn(workerPacket, {
    task_ref: workerPacket.task_ref,
    attempt: workerPacket.attempt,
    packet_digest: workerPacket.packet_digest,
    thread_id: empty.id,
  });

  assert.equal(result.status, "completed");
  assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
  assert.deepEqual(
    codex.calls.map(([operation]) => operation),
    ["connect", "resumeEmptyThread", "startTurn", "waitForTurnStarted", "waitForTurnCompleted"],
  );
  assert.equal(codex.calls.filter(([operation]) => operation === "startTurn").length, 1);
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    ["THREAD_RECONCILED_EMPTY", "TURN_START_ACCEPTED", "TURN_STARTED", "TURN_TERMINAL"],
  );
});

test("continue-turn rejects retained nonempty or ambiguous threads without starting a turn", async () => {
  const workerPacket = packet();
  for (const turns of [
    [{ id: "turn-unexpected", status: "inProgress" }],
    [{ id: "turn-one", status: "completed" }, { id: "turn-two", status: "completed" }],
  ]) {
    const codex = new ScriptedCodex();
    codex.resumeEmptyThread = async (threadId) => {
      codex.calls.push(["resumeEmptyThread", threadId]);
      return {
        id: "thread-retained-empty",
        cwd: workerPacket.cwd,
        createdAt: claimedSecond,
        turns,
      };
    };
    await assert.rejects(
      transport(codex, []).continueTurn(workerPacket, {
        task_ref: workerPacket.task_ref,
        attempt: workerPacket.attempt,
        packet_digest: workerPacket.packet_digest,
        thread_id: "thread-retained-empty",
      }),
      (error) => error.code === "MANAGED_CODEX_RETAINED_EMPTY_THREAD_REQUIRED",
    );
    assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
    assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  }
});

test("capacity wait does not consume the exact-start execution window", async () => {
  const codex = new ScriptedCodex();
  const events = [];
  const result = await transport(codex, events, {
    now: () => "2026-08-26T14:20:00Z",
  }).start(packet({
    max_duration_seconds: 60,
    deadline_at: "2026-08-26T14:30:00Z",
  }), CLAIMED_AT);

  assert.equal(result.status, "completed");
  const exactStart = events.find(({ event_type }) => event_type === "TURN_STARTED");
  assert.equal(exactStart.observed_at, "2026-08-26T14:20:00Z");
  assert.equal(exactStart.attempt_deadline_at, "2026-08-26T14:21:00Z");
  const terminalWait = codex.calls.find(([operation]) => operation === "waitForTurnCompleted");
  assert.equal(terminalWait[3].timeoutMs, 60_000);
});

test("exact start never extends the immutable task-level deadline", async () => {
  const codex = new ScriptedCodex();
  const events = [];
  const result = await transport(codex, events, {
    now: () => "2026-08-26T14:20:00Z",
  }).start(packet({
    max_duration_seconds: 900,
    deadline_at: "2026-08-26T14:21:00Z",
  }), CLAIMED_AT);

  assert.equal(result.status, "completed");
  const exactStart = events.find(({ event_type }) => event_type === "TURN_STARTED");
  assert.equal(exactStart.observed_at, "2026-08-26T14:20:00Z");
  assert.equal(exactStart.attempt_deadline_at, "2026-08-26T14:21:00Z");
  const terminalWait = codex.calls.find(([operation]) => operation === "waitForTurnCompleted");
  assert.equal(terminalWait[3].timeoutMs, 60_000);
});

test("exact start normalizes millisecond trailing zeros for Rust replay", async () => {
  const codex = new ScriptedCodex();
  const events = [];
  await transport(codex, events, {
    now: () => "2026-08-26T14:20:00.120Z",
  }).start(packet({ max_duration_seconds: 60 }), CLAIMED_AT);

  const exactStart = events.find(({ event_type }) => event_type === "TURN_STARTED");
  assert.equal(exactStart.observed_at, "2026-08-26T14:20:00.12Z");
  assert.equal(exactStart.attempt_deadline_at, "2026-08-26T14:21:00.12Z");
});

test("post-claim marker recovery retains ids but never fabricates exact start for active or terminal turns", async () => {
  const workerPacket = packet();
  for (const status of ["inProgress", "completed"]) {
    const retained = markerThread(workerPacket, status);
    const codex = new ScriptedCodex();
    codex.listThreads = async (options) => {
      codex.calls.push(["listThreads", options]);
      return { data: [{ ...retained, turns: [] }], nextCursor: null };
    };
    codex.readThread = async (threadId, options) => {
      codex.calls.push(["readThread", threadId, options]);
      return structuredClone(retained);
    };
    codex.resumeThread = async (threadId, options) => {
      codex.calls.push(["resumeThread", threadId, options.expectedTurnId]);
      codex.active = status === "inProgress";
      return structuredClone(retained);
    };
    const events = [];
    await assert.rejects(
      transport(codex, events).start(workerPacket, CLAIMED_AT),
      (error) => error.code === "MANAGED_CODEX_EXACT_START_EVIDENCE_LOST_AFTER_DISPATCH",
    );

    assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
    assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
    assert.equal(codex.calls.some(([operation]) => operation === "waitForTurnCompleted"), false);
    assert.deepEqual(
      codex.calls.map(([operation]) => operation),
      ["listThreads", "readThread", "resumeThread"],
    );
    assert.deepEqual(
      events.map(({ event_type }) => event_type),
      ["THREAD_START_ACCEPTED", "THREAD_STARTED", "TURN_START_ACCEPTED"],
    );
    assert.equal(events[0].recovered_via, "THREAD_LIST_READ");
    assert.equal(events[2].recovered_via, "EXACT_MARKER_THREAD_READ");
  }
});

test("restart-only claimed dispatch distinguishes proven no candidate from ambiguous reconciliation", async () => {
  const codex = new ScriptedCodex();
  const events = [];

  const recovered = await transport(codex, events).recoverClaimedDispatch(packet(), CLAIMED_AT);

  assert.deepEqual(recovered, { kind: "PROVEN_NO_PROVIDER_CANDIDATE" });
  assert.equal(codex.calls.filter(([operation]) => operation === "listThreads").length, 3);
  assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  assert.deepEqual(events, []);

  const ambiguous = new ScriptedCodex();
  ambiguous.listThreads = async () => ({ data: [{ id: "malformed" }], nextCursor: null });
  await assert.rejects(
    transport(ambiguous, []).recoverClaimedDispatch(packet(), CLAIMED_AT),
    (error) => error.code === "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
  );
  assert.equal(ambiguous.calls.some(([operation]) => operation === "startThread"), false);
});

test("restart-only claimed dispatch rejects a same-second empty thread as unattributable", async () => {
  const workerPacket = packet();
  const fractionalClaimedAt = "2026-08-26T14:00:00.900Z";
  const empty = {
    id: "thread-empty-restart",
    cwd: workerPacket.cwd,
    createdAt: Math.floor(Date.parse(fractionalClaimedAt) / 1_000),
    turns: [],
  };
  const codex = new ScriptedCodex();
  codex.listThreads = async (options) => {
    codex.calls.push(["listThreads", options]);
    return { data: [{ ...empty, turns: [] }], nextCursor: null };
  };
  codex.readThread = async (threadId, options) => {
    codex.calls.push(["readThread", threadId, options]);
    return structuredClone(empty);
  };
  const events = [];

  await assert.rejects(
    transport(codex, events).recoverClaimedDispatch(workerPacket, fractionalClaimedAt),
    (error) => error.code === "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
  );
  assert.deepEqual(
    codex.calls.map(([operation]) => operation),
    ["listThreads", "readThread"],
  );
  assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  assert.deepEqual(events, []);
});

test("restart-only marker recovery interrupts exact active prestart and records failed-start terminal", async () => {
  const workerPacket = packet();
  const retained = markerThread(workerPacket, "inProgress");
  const codex = new ScriptedCodex();
  codex.listThreads = async (options) => {
    codex.calls.push(["listThreads", options]);
    return { data: [{ ...retained, turns: [] }], nextCursor: null };
  };
  codex.readThread = async (threadId, options) => {
    codex.calls.push(["readThread", threadId, options]);
    return structuredClone(retained);
  };
  codex.resumeThread = async (threadId, options) => {
    codex.calls.push(["resumeThread", threadId, options.expectedTurnId]);
    codex.active = true;
    return structuredClone(retained);
  };
  const events = [];

  const recovered = await transport(codex, events)
    .recoverClaimedDispatch(workerPacket, CLAIMED_AT);

  assert.deepEqual(recovered, {
    kind: "FAILED_START_TERMINAL",
    thread_id: retained.id,
    turn_id: retained.turns[0].id,
    status: "failed",
    provider_terminal_status: "interrupted",
  });
  assert.equal(codex.calls.filter(([operation]) => operation === "interruptTurn").length, 1);
  assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    ["THREAD_START_ACCEPTED", "TURN_START_ACCEPTED", "INTERRUPT_REQUESTED", "PRESTART_TERMINAL"],
  );
  assert.equal(events.some(({ event_type }) => event_type === "TURN_STARTED"), false);
  assert.equal(events.at(-1).status, "failed");
  assert.equal(events.at(-1).provider_terminal_status, "interrupted");
});

test("restart-only marker recovery retains an already terminal provider result as failed-start", async () => {
  const workerPacket = packet();
  const retained = markerThread(workerPacket, "completed");
  const codex = new ScriptedCodex();
  codex.listThreads = async (options) => {
    codex.calls.push(["listThreads", options]);
    return { data: [{ ...retained, turns: [] }], nextCursor: null };
  };
  codex.readThread = async () => structuredClone(retained);
  codex.resumeThread = async () => structuredClone(retained);
  const events = [];

  const recovered = await transport(codex, events)
    .recoverClaimedDispatch(workerPacket, CLAIMED_AT);

  assert.equal(recovered.status, "failed");
  assert.equal(recovered.provider_terminal_status, "completed");
  assert.equal(codex.calls.some(([operation]) => operation === "interruptTurn"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    ["THREAD_START_ACCEPTED", "TURN_START_ACCEPTED", "PRESTART_TERMINAL"],
  );
  assert.equal(events.some(({ event_type }) => event_type === "TURN_STARTED"), false);
});

test("prestart retained exact ids reconcile without requiring an execution window", async () => {
  const workerPacket = packet();
  const retained = markerThread(workerPacket, "completed");
  const codex = new ScriptedCodex();
  codex.readThread = async (threadId, options) => {
    codex.calls.push(["readThread", threadId, options]);
    return structuredClone(retained);
  };
  codex.resumeThread = async (threadId, options) => {
    codex.calls.push(["resumeThread", threadId, options.expectedTurnId]);
    return structuredClone(retained);
  };
  const events = [];

  const recovered = await transport(codex, events).recoverPrestart(workerPacket, {
    task_ref: workerPacket.task_ref,
    attempt: workerPacket.attempt,
    packet_digest: workerPacket.packet_digest,
    thread_id: retained.id,
    turn_id: retained.turns[0].id,
  });

  assert.equal(recovered.status, "failed");
  assert.equal(recovered.provider_terminal_status, "completed");
  assert.deepEqual(
    codex.calls.map(([operation]) => operation),
    ["connect", "resumeThread"],
  );
  assert.equal(events.some(({ event_type }) => event_type === "TURN_STARTED"), false);
});

test("prestart retained thread without a turn is proven exact-empty without starting a turn", async () => {
  const workerPacket = packet();
  const empty = {
    id: "thread-retained-prestart-empty",
    cwd: workerPacket.cwd,
    createdAt: claimedSecond,
    turns: [],
  };
  const codex = new ScriptedCodex();
  codex.readThread = async (threadId, options) => {
    codex.calls.push(["readThread", threadId, options]);
    return structuredClone(empty);
  };
  codex.resumeEmptyThread = async (threadId) => {
    codex.calls.push(["resumeEmptyThread", threadId]);
    return structuredClone(empty);
  };
  const events = [];

  const recovered = await transport(codex, events).recoverPrestart(workerPacket, {
    task_ref: workerPacket.task_ref,
    attempt: workerPacket.attempt,
    packet_digest: workerPacket.packet_digest,
    thread_id: empty.id,
  });

  assert.deepEqual(recovered, {
    kind: "EXACT_EMPTY_THREAD",
    thread_id: empty.id,
  });
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  assert.deepEqual(events.map(({ event_type }) => event_type), ["THREAD_START_ACCEPTED"]);
});

test("bounded dispatch backoff rejects an unattributable empty candidate on the third pass", async () => {
  const workerPacket = packet();
  const empty = {
    id: "thread-delayed-visible",
    cwd: workerPacket.cwd,
    createdAt: claimedSecond,
    turns: [],
  };
  const codex = new ScriptedCodex();
  let listPass = 0;
  codex.listThreads = async (options) => {
    codex.calls.push(["listThreads", options]);
    listPass += 1;
    return { data: listPass === 3 ? [structuredClone(empty)] : [], nextCursor: null };
  };
  codex.readThread = async () => structuredClone(empty);
  await assert.rejects(
    transport(codex, [], { dispatchBackoffMs: 1 }).start(workerPacket, CLAIMED_AT),
    (error) => error.code === "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
  );

  assert.equal(codex.calls.filter(([operation]) => operation === "listThreads").length, 3);
  assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
});

test("fresh dispatch rejects one empty post-claim candidate before any start RPC", async () => {
  const workerPacket = packet();
  const empty = {
    id: "thread-empty-recovered",
    cwd: workerPacket.cwd,
    createdAt: claimedSecond,
    turns: [],
  };
  const codex = new ScriptedCodex();
  codex.listThreads = async (options) => {
    codex.calls.push(["listThreads", options]);
    return { data: [structuredClone(empty)], nextCursor: null };
  };
  codex.readThread = async (threadId, options) => {
    codex.calls.push(["readThread", threadId, options]);
    return structuredClone(empty);
  };
  const events = [];
  await assert.rejects(
    transport(codex, events).start(workerPacket, CLAIMED_AT),
    (error) => error.code === "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
  );

  assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  assert.deepEqual(
    codex.calls.map(([operation]) => operation),
    ["listThreads", "readThread"],
  );
  assert.deepEqual(events, []);
});

test("multiple or substituted post-claim candidates fail closed without a start RPC", async () => {
  const workerPacket = packet();
  for (const scenario of ["multiple", "substituted"]) {
    const codex = new ScriptedCodex();
    const exact = markerThread(workerPacket);
    codex.listThreads = async (options) => {
      codex.calls.push(["listThreads", options]);
      return {
        data: scenario === "multiple"
          ? [
            { ...exact, id: "thread-candidate-one", turns: [] },
            { ...exact, id: "thread-candidate-two", turns: [] },
          ]
          : [{ ...exact, turns: [] }],
        nextCursor: null,
      };
    };
    if (scenario === "substituted") {
      codex.readThread = async (threadId, options) => {
        codex.calls.push(["readThread", threadId, options]);
        return {
          ...exact,
          turns: [{
            id: "turn-substituted",
            status: "inProgress",
            items: [{ type: "userMessage", content: [{ type: "text", text: "unrelated work" }] }],
          }],
        };
      };
    }
    await assert.rejects(
      transport(codex, []).start(workerPacket, CLAIMED_AT),
      (error) => error.code === "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
    );
    assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
    assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  }
});

test("exact lifecycle mismatches fail closed before advancing", async () => {
  const codex = new ScriptedCodex();
  codex.waitForThreadStarted = async () => ({ id: "other-thread" });
  const events = [];
  await assert.rejects(
    transport(codex, events).start(packet(), CLAIMED_AT),
    /exact thread\/started/iu,
  );
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  assert.deepEqual(events.map(({ event_type }) => event_type), ["THREAD_START_ACCEPTED"]);
});

test("resource observations are exact-correlated, bounded counters with unavailable cost", async () => {
  const codex = new ScriptedCodex();
  codex.waitForTurnCompleted = async (threadId, turnId) => {
    codex.calls.push(["waitForTurnCompleted", threadId, turnId]);
    codex.emit("notification", {
      method: "thread/tokenUsage/updated",
      params: {
        threadId: "other-thread",
        turnId,
        tokenUsage: { total: { totalTokens: 999_999 } },
      },
    });
    codex.emit("notification", {
      method: "thread/tokenUsage/updated",
      params: {
        threadId,
        turnId,
        tokenUsage: {
          total: {
            inputTokens: 120,
            cachedInputTokens: 20,
            outputTokens: 30,
            reasoningOutputTokens: 10,
            totalTokens: 150,
            unsafeText: "Bearer secret-secret-secret",
          },
          modelContextWindow: 200_000,
        },
      },
    });
    return { id: turnId, status: "completed" };
  };
  const events = [];
  await transport(codex, events).start(packet(), CLAIMED_AT);

  const resources = events.filter(({ event_type }) => event_type === "RESOURCE_OBSERVATION");
  assert.equal(resources.length, 2);
  assert.deepEqual(
    resources.map(({ usage_scope }) => usage_scope),
    ["CUMULATIVE_INTERMEDIATE", "CUMULATIVE_TERMINAL"],
  );
  assert.deepEqual(
    {
      input_tokens: resources[0].input_tokens,
      cached_input_tokens: resources[0].cached_input_tokens,
      output_tokens: resources[0].output_tokens,
      reasoning_output_tokens: resources[0].reasoning_output_tokens,
      total_tokens: resources[0].total_tokens,
      model_context_window: resources[0].model_context_window,
      external_cost_status: resources[0].external_cost_status,
    },
    {
      input_tokens: 120,
      cached_input_tokens: 20,
      output_tokens: 30,
      reasoning_output_tokens: 10,
      total_tokens: 150,
      model_context_window: 200_000,
      external_cost_status: "UNAVAILABLE",
    },
  );
  assert.doesNotMatch(JSON.stringify(resources), /unsafeText|secret-secret/iu);
  assert.equal(events.at(-1).event_type, "TURN_TERMINAL");

  assert.equal(
    normalizeCodexResourceObservation({
      method: "thread/tokenUsage/updated",
      params: {
        threadId: "thread-exact",
        turnId: "other-turn",
        tokenUsage: { total: { totalTokens: 999 } },
      },
    }, { threadId: "thread-exact", turnId: "turn-exact" }),
    null,
  );
});

test("exact provider item activity advances meaningful progress without retaining item content", async () => {
  const codex = new ScriptedCodex();
  codex.waitForTurnCompleted = async (threadId, turnId) => {
    codex.calls.push(["waitForTurnCompleted", threadId, turnId]);
    return new Promise((resolve) => {
      const timer = setInterval(() => codex.emit("notification", {
        method: "item/commandExecution/outputDelta",
        params: {
          threadId,
          turnId,
          delta: "NEVER_RETAIN_PROVIDER_OUTPUT",
        },
      }), 20);
      setTimeout(() => {
        clearInterval(timer);
        resolve({ id: turnId, status: "completed" });
      }, 160);
    });
  };
  const events = [];
  const now = () => new Date().toISOString();
  await transport(codex, events, { now }).start(packet({
    deadline_at: new Date(Date.now() + 2_000).toISOString(),
    heartbeat_timeout_ms: 100,
  }), new Date(Date.now() - 1_000).toISOString());

  assert.equal(events.some(({ event_type }) => event_type === "STALL_CLASSIFIED"), false);
  assert.equal(events.some(({ event_type }) => event_type === "MEANINGFUL_PROGRESS"), true);
  assert.doesNotMatch(JSON.stringify(events), /NEVER_RETAIN_PROVIDER_OUTPUT/u);
  assert.deepEqual(
    normalizeCodexMeaningfulProgress({
      method: "item/completed",
      params: { threadId: "thread-exact", turnId: "turn-exact", item: { secret: "ignored" } },
    }, { threadId: "thread-exact", turnId: "turn-exact" }),
    { progress_kind: "ITEM_COMPLETED" },
  );
});

test("exact active provider reads emit bounded heartbeat evidence before a later terminal", async () => {
  const codex = new ScriptedCodex();
  codex.waitForTurnCompleted = async (threadId, turnId) => {
    codex.calls.push(["waitForTurnCompleted", threadId, turnId]);
    await new Promise((resolve) => setTimeout(resolve, 1_100));
    return { id: turnId, status: "completed" };
  };
  const events = [];
  await transport(codex, events).start(packet({ heartbeat_timeout_ms: 2_000 }), CLAIMED_AT);

  const heartbeats = events.filter(({ event_type }) => event_type === "HEARTBEAT");
  assert.equal(heartbeats.length, 1);
  assert.equal(heartbeats[0].heartbeat_kind, "EXACT_PROVIDER_READ_ACTIVE");
  assert.equal(heartbeats[0].thread_id, "thread-exact");
  assert.equal(heartbeats[0].turn_id, "turn-exact");
});

test("continuous exact heartbeats keep a long in-progress turn healthy without inventing progress", async () => {
  const codex = new ScriptedCodex();
  codex.waitForTurnCompleted = async (threadId, turnId) => {
    codex.calls.push(["waitForTurnCompleted", threadId, turnId]);
    await new Promise((resolve) => setTimeout(resolve, 2_600));
    codex.active = false;
    return { id: turnId, status: "completed" };
  };
  const events = [];
  const result = await transport(codex, events, {
    now: () => new Date().toISOString(),
  }).start(packet({
    deadline_at: new Date(Date.now() + 10_000).toISOString(),
    heartbeat_timeout_ms: 1_000,
  }), new Date(Date.now() - 1_000).toISOString());

  assert.equal(result.status, "completed");
  assert.equal(events.filter(({ event_type }) => event_type === "HEARTBEAT").length >= 2, true);
  assert.equal(events.some(({ event_type }) => event_type === "MEANINGFUL_PROGRESS"), false);
  assert.equal(events.some(({ event_type }) => event_type === "STALL_CLASSIFIED"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "interruptTurn"), false);
  assert.equal(codex.calls.filter(([operation]) => operation === "readThread").length >= 2, true);
});

test("cached local activity cannot mint a heartbeat when exact provider read fails", async () => {
  const codex = new ScriptedCodex();
  codex.waitForTurnCompleted = async () => new Promise(() => {});
  codex.readThread = async function readThread(threadId, options) {
    this.calls.push(["readThread", threadId, options]);
    const error = new Error("simulated provider read disconnect");
    error.code = "CODEX_APP_SERVER_PROCESS_EXITED";
    throw error;
  };
  const events = [];
  const result = await transport(codex, events, { now: () => new Date().toISOString() }).start(packet({
      deadline_at: new Date(Date.now() + 1_000).toISOString(),
      heartbeat_timeout_ms: 80,
    }), new Date(Date.now() - 1_000).toISOString());
  assert.equal(result.status, "interrupted");
  assert.equal(events.some(({ event_type }) => event_type === "HEARTBEAT"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "readThread"), true);
  assert.equal(codex.calls.some(([operation]) => operation === "interruptTurn"), true);
});

test("an older exact turn cannot mint heartbeat evidence after a foreign latest turn", async () => {
  const codex = new ScriptedCodex();
  codex.waitForTurnCompleted = async () => new Promise(() => {});
  codex.readThread = async function readThread(threadId, options) {
    this.calls.push(["readThread", threadId, options]);
    return {
      id: threadId,
      cwd: "C:\\disposable\\repo",
      turns: [
        { id: "turn-exact", status: "inProgress" },
        { id: "turn-foreign", status: "inProgress" },
      ],
    };
  };
  codex.resumeThread = codex.readThread;
  const events = [];
  await assert.rejects(
    transport(codex, events, { now: () => new Date().toISOString() }).start(packet({
      deadline_at: new Date(Date.now() + 1_000).toISOString(),
      heartbeat_timeout_ms: 80,
    }), new Date(Date.now() - 1_000).toISOString()),
    ({ code }) => code === "MANAGED_CODEX_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED"
      || code === "MANAGED_CODEX_EXACT_LIFECYCLE_MISMATCH",
  );
  assert.equal(events.some(({ event_type }) => event_type === "HEARTBEAT"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "readThread"), true);
});

test("heartbeat provider read racing an exact terminal does not override terminal success", async () => {
  const codex = new ScriptedCodex();
  codex.readThread = async function readThread(threadId, options) {
    this.calls.push(["readThread", threadId, options]);
    return {
      id: threadId,
      cwd: "C:\\disposable\\repo",
      turns: [{ id: "turn-exact", status: "completed" }],
    };
  };
  codex.waitForTurnCompleted = async function waitForTurnCompleted(threadId, turnId) {
    this.calls.push(["waitForTurnCompleted", threadId, turnId]);
    await new Promise((resolve) => setTimeout(resolve, 80));
    this.active = false;
    return { id: turnId, status: "completed" };
  };
  const events = [];
  const result = await transport(codex, events, {
    now: () => new Date().toISOString(),
  }).start(packet({
    deadline_at: new Date(Date.now() + 1_000).toISOString(),
    heartbeat_timeout_ms: 100,
  }), new Date(Date.now() - 1_000).toISOString());
  assert.equal(result.status, "completed");
  assert.equal(events.filter(({ event_type }) => event_type === "TURN_TERMINAL").length, 1);
  assert.equal(events.some(({ event_type }) => event_type === "HEARTBEAT"), false);
});

test("secondary heartbeat observer failure cannot erase an exact provider terminal", async () => {
  const codex = new ScriptedCodex();
  codex.readThread = async function readThread(threadId, options) {
    this.calls.push(["readThread", threadId, options]);
    throw new Error("simulated secondary heartbeat observer failure");
  };
  codex.waitForTurnCompleted = async function waitForTurnCompleted(threadId, turnId) {
    this.calls.push(["waitForTurnCompleted", threadId, turnId]);
    await new Promise((resolve) => setTimeout(resolve, 80));
    this.active = false;
    return { id: turnId, status: "completed" };
  };
  const events = [];
  const result = await transport(codex, events, {
    now: () => new Date().toISOString(),
  }).start(packet({
    deadline_at: new Date(Date.now() + 1_000).toISOString(),
    heartbeat_timeout_ms: 100,
  }), new Date(Date.now() - 1_000).toISOString());
  assert.equal(result.status, "completed");
  assert.equal(events.filter(({ event_type }) => event_type === "TURN_TERMINAL").length, 1);
  assert.equal(events.some(({ event_type }) => event_type === "HEARTBEAT"), false);
});

test("measurable usage reaching the replay-derived token remainder interrupts only the exact active turn", async () => {
  const codex = new ScriptedCodex();
  let resolveTerminal;
  codex.waitForTurnCompleted = async (threadId, turnId) => {
    codex.calls.push(["waitForTurnCompleted", threadId, turnId]);
    codex.emit("notification", {
      method: "thread/tokenUsage/updated",
      params: {
        threadId,
        turnId,
        tokenUsage: { total: { totalTokens: 100 } },
      },
    });
    return new Promise((resolve) => { resolveTerminal = resolve; });
  };
  codex.interruptTurn = async (threadId, turnId) => {
    codex.calls.push(["interruptTurn", threadId, turnId]);
    const terminal = { id: turnId, status: "interrupted" };
    resolveTerminal(terminal);
    return terminal;
  };
  const events = [];
  const result = await transport(codex, events).start(packet({
    remaining_total_tokens: 100,
  }), CLAIMED_AT);

  assert.equal(result.status, "interrupted");
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    [
      "THREAD_START_ACCEPTED",
      "THREAD_STARTED",
      "TURN_START_ACCEPTED",
      "TURN_STARTED",
      "RESOURCE_OBSERVATION",
      "STALL_CLASSIFIED",
      "INTERRUPT_REQUESTED",
      "INTERRUPT_TERMINAL",
      "RESOURCE_OBSERVATION",
      "TURN_TERMINAL",
    ],
  );
  assert.equal(codex.calls.filter(([operation]) => operation === "interruptTurn").length, 1);
  assert.equal(events[5].stall_reason, "TOKEN_BUDGET_EXCEEDED");
});

test("restart resumes and reconciles retained exact ids without opening a new thread or turn", async () => {
  const codex = new ScriptedCodex({ resumeStatus: "inProgress", terminalStatus: "completed" });
  const events = [];
  const retained = retainedAttempt();
  const result = await transport(codex, events).resume(packet(), retained);

  assert.deepEqual(
    codex.calls.map(([operation]) => operation),
    ["connect", "resumeThread", "isTurnActive", "waitForTurnCompleted"],
  );
  assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    ["RECONCILE_STARTED", "RECONCILED_ACTIVE", "TURN_TERMINAL"],
  );
  assert.equal(result.status, "completed");
});

test("restart, active reconnect, and stall reconciliation reject cross-worktree substitution", async () => {
  const substitutedResume = async function substitutedResume(threadId, { expectedTurnId } = {}) {
    this.calls.push(["resumeThread", threadId, expectedTurnId]);
    this.active = true;
    return {
      id: threadId,
      cwd: "C:\\foreign\\repo",
      turns: [{ id: expectedTurnId, status: "inProgress" }],
    };
  };

  const restartCodex = new ScriptedCodex({ resumeStatus: "inProgress" });
  restartCodex.resumeThread = substitutedResume.bind(restartCodex);
  await assert.rejects(
    transport(restartCodex, []).resume(packet(), retainedAttempt()),
    (error) => error.code === "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
  );
  assert.equal(restartCodex.calls.some(([operation]) => operation === "startTurn"), false);

  const stallCodex = new ScriptedCodex({ resumeStatus: "inProgress" });
  stallCodex.resumeThread = substitutedResume.bind(stallCodex);
  await assert.rejects(
    transport(stallCodex, []).recoverTimedStall(
      packet(),
      retainedAttempt({}, { includeProgress: false, includeHeartbeat: false }),
      {
        observed_at: "2026-08-26T14:01:00Z",
        last_heartbeat_at: "2026-08-26T14:00:00Z",
        last_meaningful_progress_at: "2026-08-26T14:00:50Z",
        interrupt: true,
      },
    ),
    (error) => error.code === "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
  );
  assert.equal(stallCodex.calls.some(([operation]) => operation === "interruptTurn"), false);

  const reconnectCodex = new ScriptedCodex({ resumeStatus: "inProgress" });
  reconnectCodex.waitForTurnCompleted = async (threadId, turnId) => {
    reconnectCodex.calls.push(["waitForTurnCompleted", threadId, turnId]);
    const error = new Error("simulated App Server process exit");
    error.code = "CODEX_APP_SERVER_PROCESS_EXITED";
    throw error;
  };
  reconnectCodex.resumeThread = substitutedResume.bind(reconnectCodex);
  await assert.rejects(
    transport(reconnectCodex, []).start(packet(), CLAIMED_AT),
    (error) => error.code === "MANAGED_CODEX_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED",
  );
  assert.equal(reconnectCodex.calls.filter(([operation]) => operation === "startTurn").length, 1);
  assert.equal(reconnectCodex.calls.some(([operation]) => operation === "interruptTurn"), false);
});

test("active transport exit reconnects and resumes the exact active turn before waiting again", async () => {
  const codex = new ScriptedCodex({ resumeStatus: "inProgress" });
  let terminalWaits = 0;
  codex.waitForTurnCompleted = async (threadId, turnId) => {
    codex.calls.push(["waitForTurnCompleted", threadId, turnId]);
    terminalWaits += 1;
    if (terminalWaits === 1) {
      const error = new Error("simulated App Server process exit");
      error.code = "CODEX_APP_SERVER_PROCESS_EXITED";
      throw error;
    }
    codex.active = false;
    return { id: turnId, status: "completed" };
  };
  codex.readThread = async (threadId) => {
    codex.calls.push(["readThread", threadId]);
    return {
      id: threadId,
      cwd: "C:\\disposable\\repo",
      turns: [{ id: "turn-exact", status: "inProgress" }],
    };
  };
  const events = [];

  const result = await transport(codex, events).start(packet(), CLAIMED_AT);

  assert.equal(result.status, "completed");
  assert.equal(codex.calls.filter(([operation]) => operation === "startThread").length, 1);
  assert.equal(codex.calls.filter(([operation]) => operation === "startTurn").length, 1);
  assert.equal(codex.calls.filter(([operation]) => operation === "interruptTurn").length, 0);
  assert.deepEqual(
    codex.calls.slice(-6).map(([operation]) => operation),
    ["waitForTurnCompleted", "connect", "resumeThread", "readThread", "isTurnActive", "waitForTurnCompleted"],
  );
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
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
  assert.equal(events.every(({ attempt }) => attempt === 1), true);
});

test("active transport exit closes from the exact reconciled terminal without a replacement effect", async () => {
  const codex = new ScriptedCodex({ resumeStatus: "completed" });
  codex.waitForTurnCompleted = async (threadId, turnId) => {
    codex.calls.push(["waitForTurnCompleted", threadId, turnId]);
    const error = new Error("simulated App Server process exit");
    error.code = "CODEX_APP_SERVER_PROCESS_EXITED";
    throw error;
  };
  codex.readThread = async (threadId) => {
    codex.calls.push(["readThread", threadId]);
    return {
      id: threadId,
      cwd: "C:\\disposable\\repo",
      turns: [{ id: "turn-exact", status: "completed" }],
    };
  };
  const events = [];

  const result = await transport(codex, events).start(packet(), CLAIMED_AT);

  assert.equal(result.status, "completed");
  assert.equal(codex.calls.filter(([operation]) => operation === "startThread").length, 1);
  assert.equal(codex.calls.filter(([operation]) => operation === "startTurn").length, 1);
  assert.equal(codex.calls.filter(([operation]) => operation === "waitForTurnCompleted").length, 1);
  assert.equal(codex.calls.filter(([operation]) => operation === "interruptTurn").length, 0);
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    [
      "THREAD_START_ACCEPTED",
      "THREAD_STARTED",
      "TURN_START_ACCEPTED",
      "TURN_STARTED",
      "RECONCILE_STARTED",
      "RECONCILED_TERMINAL",
    ],
  );
  assert.equal(events.every(({ attempt }) => attempt === 1), true);
});

test("active transport reconciliation exhausts after two exact resume attempts with a typed blocker", async () => {
  const codex = new ScriptedCodex();
  codex.waitForTurnCompleted = async (threadId, turnId) => {
    codex.calls.push(["waitForTurnCompleted", threadId, turnId]);
    const error = new Error("simulated RPC disconnect");
    error.code = "CODEX_APP_SERVER_TRANSPORT_ERROR";
    throw error;
  };
  codex.resumeThread = async (threadId, { expectedTurnId } = {}) => {
    codex.calls.push(["resumeThread", threadId, expectedTurnId]);
    const error = new Error("simulated reconciliation disconnect");
    error.code = "CODEX_APP_SERVER_TRANSPORT_ERROR";
    throw error;
  };
  const events = [];

  await assert.rejects(
    transport(codex, events).start(packet(), CLAIMED_AT),
    (error) => error.code === "MANAGED_CODEX_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED",
  );

  assert.equal(codex.calls.filter(([operation]) => operation === "startThread").length, 1);
  assert.equal(codex.calls.filter(([operation]) => operation === "startTurn").length, 1);
  assert.equal(codex.calls.filter(([operation]) => operation === "connect").length, 2);
  assert.equal(codex.calls.filter(([operation]) => operation === "resumeThread").length, 2);
  assert.equal(codex.calls.filter(([operation]) => operation === "interruptTurn").length, 0);
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    [
      "THREAD_START_ACCEPTED",
      "THREAD_STARTED",
      "TURN_START_ACCEPTED",
      "TURN_STARTED",
      "RECONCILE_STARTED",
      "RECONCILE_STARTED",
    ],
  );
  assert.equal(events.every(({ attempt }) => attempt === 1), true);
});

test("restart keeps the durable exact-start deadline instead of granting a fresh window", async () => {
  const codex = new ScriptedCodex({ resumeStatus: "inProgress", terminalStatus: "completed" });
  const events = [];
  const retained = retainedAttempt({
    attempt_started_at: "2026-08-26T13:50:00.000Z",
    attempt_deadline_at: "2026-08-26T14:05:00.000Z",
    last_heartbeat_at: "2026-08-26T14:04:00Z",
    last_meaningful_progress_at: "2026-08-26T14:04:00Z",
  });
  await transport(codex, events, {
    now: () => "2026-08-26T14:04:00Z",
  }).resume(packet(), retained);

  const terminalWait = codex.calls.find(([operation]) => operation === "waitForTurnCompleted");
  assert.equal(terminalWait[3].timeoutMs, 60_000);
  assert.equal(events.some(({ event_type }) => event_type === "TURN_STARTED"), false);
});

test("restart rejects missing or tampered exact-start execution windows before provider access", async () => {
  for (const retained of [
    (() => {
      const value = retainedAttempt();
      delete value.attempt_started_at;
      return value;
    })(),
    (() => {
      const value = retainedAttempt();
      delete value.last_heartbeat_at;
      return value;
    })(),
    retainedAttempt({ attempt_deadline_at: "2026-08-26T14:16:00Z" }),
  ]) {
    const codex = new ScriptedCodex();
    await assert.rejects(
      transport(codex, []).resume(packet(), retained),
      /attempt_started_at|execution deadline|heartbeat/iu,
    );
    assert.deepEqual(codex.calls, []);
  }
});

test("retained exact work reconciles even when its model is no longer start-available", async () => {
  const codex = new ScriptedCodex({ resumeStatus: "completed" });
  const events = [];
  const retained = retainedAttempt();
  const worker = transport(codex, events, { availableModels: [] });
  const result = await worker.resume(packet(), retained);

  assert.equal(result.status, "completed");
  assert.deepEqual(
    codex.calls.map(([operation]) => operation),
    ["connect", "resumeThread"],
  );
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    ["RECONCILE_STARTED", "RECONCILED_TERMINAL"],
  );
});

test("graceful shutdown accepts only an exact bridge interrupt control", async () => {
  const codex = new ScriptedCodex();
  codex.active = true;
  codex.isTurnActive = (threadId, turnId) => {
    codex.calls.push(["isTurnActive", threadId, turnId]);
    return codex.active && threadId === "thread-retained" && turnId === "turn-retained";
  };
  const events = [];
  const workerPacket = packet();
  const control = {
    schema: "lattice.managed-codex-worker-control/1.0",
    operation: "interrupt",
    task_ref: workerPacket.task_ref,
    attempt: workerPacket.attempt,
    packet_digest: workerPacket.packet_digest,
    thread_id: "thread-retained",
    turn_id: "turn-retained",
  };

  const terminal = await transport(codex, events).interruptActive(workerPacket, control);

  assert.deepEqual(terminal, { id: "turn-retained", status: "interrupted" });
  assert.deepEqual(
    codex.calls.map(([operation]) => operation),
    ["isTurnActive", "interruptTurn"],
  );
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    ["INTERRUPT_REQUESTED", "INTERRUPT_TERMINAL"],
  );

  const substituted = { ...control, turn_id: "turn-substituted" };
  codex.active = true;
  await assert.rejects(
    transport(codex, []).interruptActive(workerPacket, substituted),
    (error) => error.code === "MANAGED_CODEX_RETAINED_IDENTITY_MISMATCH",
  );
  assert.equal(codex.calls.filter(([operation]) => operation === "interruptTurn").length, 1);
});

test("terminal restart reconciliation emits a closed cumulative usage sample when retained read supplies it", async () => {
  const codex = new ScriptedCodex({ resumeStatus: "completed" });
  codex.resumeThread = async (threadId, { expectedTurnId } = {}) => {
    codex.calls.push(["resumeThread", threadId, expectedTurnId]);
    return {
      id: threadId,
      cwd: "C:\\disposable\\repo",
      turns: [{
        id: expectedTurnId,
        status: "completed",
        tokenUsage: { total: { inputTokens: 40, outputTokens: 5, totalTokens: 45 } },
      }],
    };
  };
  const events = [];
  await transport(codex, events).resume(packet(), retainedAttempt());

  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    ["RECONCILE_STARTED", "RESOURCE_OBSERVATION", "RECONCILED_TERMINAL"],
  );
  assert.equal(events[1].usage_scope, "CUMULATIVE_TERMINAL");
  assert.equal(events[1].total_tokens, 45);
});

test("restart exact reconciliation refreshes liveness without inventing meaningful progress", async () => {
  const codex = new ScriptedCodex({ resumeStatus: "inProgress" });
  codex.waitForTurnCompleted = async (threadId, turnId) => {
    codex.calls.push(["waitForTurnCompleted", threadId, turnId]);
    await new Promise((resolve) => setTimeout(resolve, 5));
    codex.active = false;
    return { id: turnId, status: "completed" };
  };
  const events = [];
  const retained = retainedAttempt();
  const result = await transport(codex, events, {
    now: () => "2026-08-26T14:01:00Z",
  }).resume(packet({ heartbeat_timeout_ms: 30_000 }), retained);

  assert.equal(result.status, "completed");
  assert.equal(codex.calls.some(([operation]) => operation === "startThread"), false);
  assert.equal(codex.calls.some(([operation]) => operation === "startTurn"), false);
  assert.equal(codex.calls.filter(([operation]) => operation === "interruptTurn").length, 0);
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    [
      "RECONCILE_STARTED",
      "RECONCILED_ACTIVE",
      "TURN_TERMINAL",
    ],
  );
  assert.equal(events.some(({ event_type }) => event_type === "MEANINGFUL_PROGRESS"), false);
});

test("heartbeat stall is classified only after exact active reconciliation and exact interrupt terminal", async () => {
  const codex = new ScriptedCodex({ resumeStatus: "inProgress" });
  const events = [];
  const retained = retainedAttempt({}, { includeProgress: false, includeHeartbeat: false });
  const result = await transport(codex, events).recoverTimedStall(packet(), retained, {
    observed_at: "2026-08-26T14:01:00Z",
    last_heartbeat_at: "2026-08-26T14:00:00Z",
    last_meaningful_progress_at: "2026-08-26T14:00:50Z",
    interrupt: true,
  });

  assert.equal(result.stall_reason, "HEARTBEAT_TIMEOUT_ACTIVE_TURN");
  assert.equal(result.terminal.status, "interrupted");
  assert.deepEqual(
    codex.calls.map(([operation]) => operation),
    ["connect", "resumeThread", "isTurnActive", "interruptTurn"],
  );
  assert.deepEqual(
    events.map(({ event_type }) => event_type),
    [
      "RECONCILE_STARTED",
      "RECONCILED_ACTIVE",
      "STALL_CLASSIFIED",
      "INTERRUPT_REQUESTED",
      "INTERRUPT_TERMINAL",
    ],
  );
  assert.equal(codex.calls.some(([operation]) => operation.includes("retry")), false);
});

test("expired prestart dispatch deadline fails closed without opening Codex", async () => {
  const codex = new ScriptedCodex();
  const events = [];
  await assert.rejects(
    transport(codex, events).start(packet({ deadline_at: "2026-08-26T13:59:00Z" }), CLAIMED_AT),
    (error) => error.code === "MANAGED_CODEX_PRESTART_DEADLINE_EXCEEDED",
  );
  assert.deepEqual(codex.calls, []);
  assert.deepEqual(events, []);
});

test("elapsed time cannot become a stall when reconciliation is terminal or not exact-active", async () => {
  const retained = retainedAttempt({}, { includeProgress: false, includeHeartbeat: false });
  const terminalCodex = new ScriptedCodex({ resumeStatus: "completed" });
  const terminalEvents = [];
  const terminal = await transport(terminalCodex, terminalEvents).recoverTimedStall(
    packet(),
    retained,
    {
      observed_at: "2026-08-26T14:01:00Z",
      last_heartbeat_at: "2026-08-26T14:00:00Z",
      last_meaningful_progress_at: "2026-08-26T14:00:00Z",
      interrupt: true,
    },
  );
  assert.equal(terminal.kind, "TERMINAL");
  assert.equal(terminalCodex.calls.some(([operation]) => operation === "interruptTurn"), false);
  assert.equal(terminalEvents.some(({ event_type }) => event_type === "STALL_CLASSIFIED"), false);

  const healthyCodex = new ScriptedCodex({ resumeStatus: "inProgress" });
  const healthyEvents = [];
  const healthy = await transport(healthyCodex, healthyEvents).recoverTimedStall(
    packet(),
    retained,
    {
      observed_at: "2026-08-26T14:00:10Z",
      last_heartbeat_at: "2026-08-26T14:00:05Z",
      last_meaningful_progress_at: "2026-08-26T14:00:00Z",
      interrupt: true,
    },
  );
  assert.deepEqual(healthy, { kind: "HEALTHY" });
  assert.deepEqual(healthyCodex.calls, []);
  assert.deepEqual(healthyEvents, []);
});

test("deadline classification still requires the retained exact active turn", async () => {
  const retained = retainedAttempt({
    attempt_started_at: "2026-08-26T13:45:00Z",
    attempt_deadline_at: "2026-08-26T14:00:00Z",
  }, { includeProgress: false, includeHeartbeat: false });
  const inactiveCodex = new ScriptedCodex({ resumeStatus: "inProgress" });
  inactiveCodex.isTurnActive = () => false;
  const inactiveEvents = [];
  const inactive = await transport(inactiveCodex, inactiveEvents).recoverTimedStall(
    packet(),
    retained,
    {
      observed_at: "2026-08-26T14:00:01Z",
      last_heartbeat_at: "2026-08-26T14:00:00Z",
      last_meaningful_progress_at: "2026-08-26T14:00:00Z",
      interrupt: true,
    },
  );
  assert.deepEqual(inactive, { kind: "NOT_EXACT_ACTIVE" });
  assert.equal(inactiveEvents.some(({ event_type }) => event_type === "STALL_CLASSIFIED"), false);
  assert.equal(inactiveCodex.calls.some(([operation]) => operation === "interruptTurn"), false);

  const activeCodex = new ScriptedCodex({ resumeStatus: "inProgress" });
  const activeEvents = [];
  const active = await transport(activeCodex, activeEvents).recoverTimedStall(
    packet(),
    retained,
    {
      observed_at: "2026-08-26T14:00:01Z",
      last_heartbeat_at: "2026-08-26T14:00:01Z",
      last_meaningful_progress_at: "2026-08-26T14:00:00Z",
      interrupt: false,
    },
  );
  assert.deepEqual(active, {
    kind: "STALL",
    stall_reason: "DEADLINE_EXCEEDED",
    terminal: null,
  });
  assert.equal(activeCodex.calls.some(([operation]) => operation === "interruptTurn"), false);
});
