import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer as createHttpServer, request as httpRequest } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { createLatticeServer } from "../src/server.mjs";
import { LatticeControlService } from "../src/service.mjs";
import { LatticeStore } from "../src/store.mjs";

class FakeCodex extends EventEmitter {
  connected = false;
  threads = 0;
  turns = 0;
  threadStarts = [];
  turnStarts = [];
  responses = [];
  rejectedRequests = [];
  deferredRequests = new Set();
  notificationLog = [];
  activeTurns = new Map();
  interruptCalls = [];
  archived = [];
  resumed = [];
  emptyResumed = [];
  readResults = new Map();
  readCalls = [];
  listResult = null;
  listCalls = 0;
  closeCalls = 0;
  readinessCalls = 0;
  freshReadCalls = [];
  freshReadResult = null;
  disconnectOnInterruptTimeout = false;

  constructor({ autoThreadStarted = true, autoTurnStarted = true } = {}) {
    super();
    this.autoThreadStarted = autoThreadStarted;
    this.autoTurnStarted = autoTurnStarted;
  }

  emit(eventName, ...args) {
    if (eventName === "notification") {
      const message = args[0];
      this.notificationLog.push(message);
      const threadId = message.params?.threadId;
      const turnId = message.params?.turn?.id;
      if (message.method === "turn/started" && threadId && turnId) {
        this.activeTurns.set(threadId, turnId);
      } else if (
        message.method === "turn/completed"
        && threadId
        && this.activeTurns.get(threadId) === turnId
      ) {
        this.activeTurns.delete(threadId);
      }
    }
    return super.emit(eventName, ...args);
  }

  waitForThreadStarted(threadId) {
    return this.#waitForNotification(
      (message) => message.method === "thread/started" && message.params?.thread?.id === threadId,
      (message) => message.params.thread,
    );
  }

  waitForTurnStarted(threadId, turnId) {
    return this.#waitForNotification(
      (message) => message.method === "turn/started"
        && message.params?.threadId === threadId
        && message.params?.turn?.id === turnId,
      (message) => message.params.turn,
    );
  }

  waitForTurnCompleted(threadId, turnId, { statuses = ["completed", "interrupted", "failed"] } = {}) {
    return this.#waitForNotification(
      (message) => message.method === "turn/completed"
        && message.params?.threadId === threadId
        && message.params?.turn?.id === turnId
        && statuses.includes(message.params.turn.status),
      (message) => message.params.turn,
    );
  }

  #waitForNotification(matches, select) {
    const observed = this.notificationLog.findLast(matches);
    if (observed) return Promise.resolve(select(observed));
    return new Promise((resolve) => {
      const listener = (message) => {
        if (!matches(message)) return;
        this.off("notification", listener);
        resolve(select(message));
      };
      this.on("notification", listener);
    });
  }

  async readAuthReadiness() {
    this.readinessCalls += 1;
    this.connected = true;
    return {
      ready: true,
      authMode: "chatgpt",
      appServerGeneration: 1,
      appServerSessionId: "fake-app-server-session",
    };
  }

  async startThread({ cwd, model, sandbox, approvalPolicy, effectIdentity }) {
    this.connected = true;
    this.threads += 1;
    this.startOptions = { cwd, model, sandbox, approvalPolicy, effectIdentity };
    const thread = { id: `thread-${this.threads}` };
    this.threadStarts.push({ ...this.startOptions, threadId: thread.id });
    this.emit("threadStartAccepted", thread);
    if (this.autoThreadStarted) {
      this.emit("notification", {
        method: "thread/started",
        params: { thread },
      });
    }
    return thread;
  }

  async listThreads() {
    this.listCalls += 1;
    return this.listResult ?? { data: [], nextCursor: null };
  }

  async readThread(threadId) {
    this.readCalls.push(threadId);
    const result = this.readResults.get(threadId);
    if (!result) {
      const error = new Error(`Codex thread ${threadId} is not recoverable`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    return structuredClone(result);
  }

  async resumeEmptyThread(threadId) {
    const thread = await this.readThread(threadId);
    if (thread.turns.length !== 0) throw new Error("thread is not empty");
    this.emptyResumed.push(threadId);
    this.connected = true;
    return thread;
  }

  async resumeThread(threadId) {
    this.connected = true;
    this.resumed.push(threadId);
    await this.beforeResumeResult?.({ threadId });
    if (this.resumeError) throw this.resumeError;
    return this.resumeResult ?? {
      id: threadId,
      turns: [{ id: "turn-1", status: "completed" }],
    };
  }

  async readThreadFresh(threadId) {
    this.freshReadCalls.push(threadId);
    if (this.freshReadError) throw this.freshReadError;
    if (this.freshReadResult) return structuredClone(this.freshReadResult);
    return this.readThread(threadId);
  }

  async startTurn(threadId, text, { model = null } = {}) {
    await this.beforeTurnDispatch?.({ threadId, text });
    this.turns += 1;
    const turn = { id: `turn-${this.turns}`, items: [], status: "inProgress" };
    this.lastTurn = { threadId, text, turnId: turn.id, model };
    this.turnStarts.push(this.lastTurn);
    this.emit("turnStartAccepted", this.lastTurn);
    await this.beforeTurnResult?.({ threadId, text });
    if (this.autoTurnStarted) {
      this.emit("notification", {
        method: "turn/started",
        params: { threadId, turn },
      });
    }
    return turn;
  }

  respond(id, result) {
    this.beforeRespond?.({ id, result });
    this.deferredRequests.delete(id);
    this.responses.push({ id, result });
  }

  deferServerRequest(id) {
    this.deferredRequests.add(id);
  }

  rejectServerRequest(id, error) {
    this.deferredRequests.delete(id);
    this.rejectedRequests.push({ id, error });
  }

  isTurnActive(threadId, turnId) {
    return this.activeTurns.get(threadId) === turnId;
  }

  hasActiveTurnOtherThan(threadId, turnId) {
    return [...this.activeTurns].some(
      ([activeThreadId, activeTurnId]) => activeThreadId !== threadId || activeTurnId !== turnId,
    );
  }

  interruptTurn(threadId, turnId, { timeoutMs = 250 } = {}) {
    if (!this.isTurnActive(threadId, turnId)) {
      return Promise.reject(new Error(`Codex turn ${threadId}/${turnId} is not active`));
    }
    this.interruptCalls.push({ threadId, turnId, timeoutMs });
    this.emit("interruptAccepted", { threadId, turnId });
    return new Promise((resolve, reject) => {
      const listener = (message) => {
        const turn = message.params?.turn;
        if (
          message.method !== "turn/completed"
          || message.params?.threadId !== threadId
          || turn?.id !== turnId
          || !["completed", "interrupted", "failed"].includes(turn.status)
        ) return;
        clearTimeout(timer);
        this.off("notification", listener);
        resolve(turn);
      };
      const timer = setTimeout(() => {
        this.off("notification", listener);
        if (this.disconnectOnInterruptTimeout) {
          this.emit("disconnect", { code: null, signal: "client-close" });
        }
        reject(new Error(`interrupt ${threadId}/${turnId} timed out`));
      }, timeoutMs);
      timer.unref?.();
      this.on("notification", listener);
    });
  }

  async archiveThread(threadId) {
    this.archived.push(threadId);
    return {};
  }

  async close() {
    this.closeCalls += 1;
    this.connected = false;
    this.activeTurns.clear();
  }
}

async function bounded(promise, label, timeoutMs = 250) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((resolve, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} test guard timed out`)), timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

let conversationFenceSequence = 0;
function acquirePrimaryConversationFence(store, projectId) {
  store.ensurePrimaryConversation(projectId);
  conversationFenceSequence += 1;
  const lease = store.acquirePrimaryConversationLease({
    ownerId: `test:${process.pid}:${conversationFenceSequence}`,
    ownerPid: process.pid,
    ttlMs: 15_000,
  });
  return { ownerId: lease.owner_id, generation: lease.generation };
}

test("installation receipts are normalized, append-only, idempotent, and durable", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-control-receipt-"));
  const databasePath = path.join(directory, "control.db");
  const artifactPath = path.join(directory, "bin", "lattice.exe");
  let store;
  try {
    store = new LatticeStore(databasePath);
    const project = store.createProject({ name: "LATTICE", rootPath: directory });
    const input = {
      projectId: project.id,
      component: " LATTICE-CLI ",
      sourceCommitSha: "A".repeat(40),
      artifactPath,
      artifactSha256: "B".repeat(64),
    };

    const first = store.createInstallationReceipt(input);
    assert.equal(first.created, true);
    assert.equal(first.receipt.schema_version, "lattice.control.installation-receipt.v1");
    assert.equal(first.receipt.observation_kind, "OBSERVED_AFTER_INSTALL");
    assert.equal(first.receipt.authority, "NON_AUTHORITATIVE");
    assert.equal(first.receipt.project_id, project.id);
    assert.equal(first.receipt.project_name, "LATTICE");
    assert.equal(first.receipt.component, "lattice-cli");
    assert.equal(first.receipt.source_commit_sha, "a".repeat(40));
    assert.equal(first.receipt.artifact_path, path.normalize(artifactPath));
    assert.equal(first.receipt.artifact_sha256, "b".repeat(64));
    assert.match(first.receipt.receipt_digest, /^[a-f0-9]{64}$/u);
    assert.ok(Date.parse(first.receipt.recorded_at));

    const retry = store.createInstallationReceipt(input);
    assert.equal(retry.created, false);
    assert.deepEqual(retry.receipt, first.receipt);
    assert.equal(store.listInstallationReceipts().length, 1);

    const changed = store.createInstallationReceipt({
      ...input,
      artifactSha256: "C".repeat(64),
    });
    assert.equal(changed.created, true);
    assert.notEqual(changed.receipt.id, first.receipt.id);
    assert.equal(store.listInstallationReceipts().length, 2);
    assert.deepEqual(
      store.listInstallationReceipts({ limit: 1, offset: 1 }),
      [first.receipt],
    );
    assert.throws(
      () => store.listInstallationReceipts({ limit: 0 }),
      /receipt limit/u,
    );
    const otherProject = store.createProject({ name: "Other", rootPath: directory });
    const otherProjectReceipt = store.createInstallationReceipt({
      ...input,
      projectId: otherProject.id,
    });
    assert.equal(otherProjectReceipt.created, true);
    assert.notEqual(otherProjectReceipt.receipt.receipt_digest, first.receipt.receipt_digest);
    assert.equal(store.listInstallationReceipts().length, 3);
    assert.throws(
      () => store.database.prepare("UPDATE installation_receipts SET component = 'changed'").run(),
      /append-only/u,
    );
    assert.throws(
      () => store.database.prepare("DELETE FROM installation_receipts").run(),
      /append-only/u,
    );

    const beforeRestart = store.listInstallationReceipts();
    store.close();
    store = new LatticeStore(databasePath);
    assert.deepEqual(store.listInstallationReceipts(), beforeRestart);
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("installation receipt evidence fails closed when identifiers, hashes, or paths are invalid", () => {
  const store = new LatticeStore();
  const project = store.createProject({ name: "LATTICE", rootPath: process.cwd() });
  const valid = {
    projectId: project.id,
    component: "lattice-cli",
    sourceCommitSha: "a".repeat(40),
    artifactPath: path.resolve("lattice.exe"),
    artifactSha256: "b".repeat(64),
  };
  const invalidInputs = [
    { ...valid, component: "" },
    { ...valid, component: "../lattice-cli" },
    { ...valid, sourceCommitSha: "a".repeat(39) },
    { ...valid, sourceCommitSha: `${"a".repeat(39)}z` },
    { ...valid, artifactPath: "relative/lattice.exe" },
    { ...valid, artifactSha256: "b".repeat(63) },
    { ...valid, artifactSha256: `${"b".repeat(63)}z` },
  ];
  try {
    for (const input of invalidInputs) {
      assert.throws(() => store.createInstallationReceipt(input), TypeError);
    }
    assert.throws(
      () => store.createInstallationReceipt({ ...valid, projectId: "missing" }),
      /project not found/u,
    );
    assert.deepEqual(store.listInstallationReceipts(), []);
  } finally {
    store.close();
  }
});

test("installation receipt pages follow append order even when the wall clock moves backward", () => {
  const store = new LatticeStore();
  const project = store.createProject({ name: "Clock", rootPath: process.cwd() });
  try {
    store.createInstallationReceipt({
      projectId: project.id,
      component: "lattice-cli",
      sourceCommitSha: "a".repeat(40),
      artifactPath: path.resolve("first.exe"),
      artifactSha256: "b".repeat(64),
    });
    store.database.prepare(`
      INSERT INTO installation_receipts (
        id, schema_version, observation_kind, authority, project_id, component,
        source_commit_sha, artifact_path, artifact_sha256, receipt_digest, recorded_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `).run(
      "later-append",
      "lattice.control.installation-receipt.v1",
      "OBSERVED_AFTER_INSTALL",
      "NON_AUTHORITATIVE",
      project.id,
      "lattice-cli",
      "c".repeat(40),
      path.resolve("later.exe"),
      "d".repeat(64),
      "e".repeat(64),
      "2000-01-01T00:00:00.000Z",
    );

    assert.equal(store.listInstallationReceipts({ limit: 1 })[0].id, "later-append");
  } finally {
    store.close();
  }
});

test("work survives restart and keeps the same Codex thread through verification and archive", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-control-"));
  const databasePath = path.join(directory, "control.db");
  let firstService;
  let firstStore;
  let secondService;
  let secondStore;
  try {
    const firstCodex = new FakeCodex();
    firstStore = new LatticeStore(databasePath);
    firstService = new LatticeControlService({ store: firstStore, codex: firstCodex });
    const project = firstService.createProject({ name: "LATTICE", rootPath: directory });
    const created = firstService.createWorkItem({
      projectId: project.id,
      title: "修復登入",
      objective: "Diagnose and fix the login failure, then run the focused tests.",
      priority: "urgent",
    });

    const started = await firstService.start(created.id);
    assert.equal(started.status, "running");
    assert.equal(started.codex_thread_id, "thread-1");
    assert.equal(firstCodex.startOptions.model, "gpt-5.6-terra");
    assert.equal(firstCodex.startOptions.cwd, path.resolve(directory));

    firstCodex.emit("notification", {
      method: "item/started",
      params: { threadId: "thread-1", turnId: "turn-1", item: { type: "commandExecution" } },
    });
    assert.equal(firstStore.getWorkItem(created.id).progress, "Running commandExecution");

    firstCodex.emit("serverRequest", {
      id: 42,
      method: "item/commandExecution/requestApproval",
      params: {
        threadId: "thread-1",
        turnId: "turn-1",
        reason: "Run focused tests",
        command: "npm test",
      },
    });
    assert.equal(firstStore.getWorkItem(created.id).status, "waiting_approval");
    await firstService.approve(created.id, "accept");
    assert.deepEqual(firstCodex.responses, [{ id: 42, result: { decision: "accept" } }]);

    firstCodex.emit("notification", {
      method: "turn/completed",
      params: { threadId: "thread-1", turn: { id: "turn-1", status: "completed" } },
    });
    assert.equal(firstStore.getWorkItem(created.id).status, "codex_done");
    firstService.close();
    firstStore.close();
    firstService = null;
    firstStore = null;

    const secondCodex = new FakeCodex();
    secondStore = new LatticeStore(databasePath);
    secondService = new LatticeControlService({ store: secondStore, codex: secondCodex });
    const restored = secondService.workItem(created.id);
    assert.equal(restored.item.codex_thread_id, "thread-1");
    assert.ok(restored.events.some((event) => event.kind === "approval_requested"));
    assert.ok(restored.events.some((event) => event.kind === "turn_completed"));

    await secondService.resume(created.id, "Review the completed change once more.");
    assert.deepEqual(secondCodex.resumed, ["thread-1"]);
    assert.equal(secondCodex.turns, 0, "reconciling completed work must not start another turn");
    assert.equal(secondStore.getWorkItem(created.id).codex_turn_id, "turn-1");
    assert.equal(
      secondStore.listEvents(created.id).filter(({ kind }) => kind === "turn_completed").length,
      1,
      "fresh-process reconciliation must not duplicate a completed turn",
    );

    const verified = secondService.verify(created.id, "Focused tests passed.");
    assert.equal(verified.status, "verified");
    const archived = await secondService.archive(created.id);
    assert.equal(archived.status, "archived");
    assert.deepEqual(secondCodex.archived, ["thread-1"]);
    secondService.close();
    secondStore.close();
    secondService = null;
    secondStore = null;
  } finally {
    secondService?.close();
    secondStore?.close();
    firstService?.close();
    firstStore?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("start remains non-running until the exact turn/started notification", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex({ autoTurnStarted: false });
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Readiness", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Wait for exact active turn",
      objective: "Do not claim running from an accepted RPC response.",
    });
    const accepted = new Promise((resolve) => codex.once("turnStartAccepted", resolve));
    const starting = service.start(item.id);
    const { threadId, turnId } = await accepted;
    await new Promise((resolve) => setImmediate(resolve));

    assert.notEqual(store.getWorkItem(item.id).status, "running");

    codex.emit("notification", {
      method: "turn/started",
      params: {
        threadId: "thread-foreign",
        turn: { id: turnId, items: [], status: "inProgress" },
      },
    });
    await new Promise((resolve) => setImmediate(resolve));
    assert.notEqual(store.getWorkItem(item.id).status, "running");

    codex.emit("notification", {
      method: "turn/started",
      params: {
        threadId,
        turn: { id: "turn-stale", items: [], status: "inProgress" },
      },
    });
    await new Promise((resolve) => setImmediate(resolve));
    assert.notEqual(store.getWorkItem(item.id).status, "running");

    codex.emit("notification", {
      method: "turn/started",
      params: { threadId, turn: { id: turnId, items: [], status: "inProgress" } },
    });
    const started = await starting;
    assert.equal(started.status, "running");
    assert.equal(started.codex_thread_id, threadId);
    assert.equal(started.codex_turn_id, turnId);
  } finally {
    service.close();
    store.close();
  }
});

test("owned shutdown stops admission and persists the exact active-turn interrupt terminal", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex, lifecycleTimeoutMs: 500 });
  try {
    const project = service.createProject({ name: "Shutdown", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Drain active turn",
      objective: "Interrupt and persist the exact terminal before owned shutdown.",
    });
    const running = await service.start(item.id);
    assert.equal(running.status, "running");
    assert.equal(service.reconciliationRequired(), false,
      "a turn started by the current process was mistaken for inherited ambiguity");
    const interruptAccepted = new Promise((resolve) => codex.once("interruptAccepted", resolve));
    const shuttingDown = service.shutdown({ timeoutMs: 500 });
    await interruptAccepted;
    await assert.rejects(
      service.resume(item.id),
      (error) => error?.code === "CONTROL_SHUTTING_DOWN",
    );
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: running.codex_thread_id,
        turn: { id: running.codex_turn_id, status: "interrupted" },
      },
    });
    const result = await shuttingDown;
    assert.equal(codex.interruptCalls.length, 1);
    assert.equal(codex.interruptCalls[0].threadId, running.codex_thread_id);
    assert.equal(codex.interruptCalls[0].turnId, running.codex_turn_id);
    assert.ok(codex.interruptCalls[0].timeoutMs > 0 && codex.interruptCalls[0].timeoutMs <= 500);
    assert.equal(result.clean, true);
    assert.equal(result.reconciliation_required, false);
    assert.equal(store.getWorkItem(item.id).status, "failed");
    assert.equal(store.listEvents(item.id).filter(({ kind }) => kind === "turn_completed").length, 1);
  } finally {
    service.close();
    store.close();
  }
});

test("owned shutdown leaves ambiguous timeout state for next-start reconciliation", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  codex.disconnectOnInterruptTimeout = true;
  const service = new LatticeControlService({ store, codex, lifecycleTimeoutMs: 25 });
  try {
    const project = service.createProject({ name: "Shutdown timeout", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Preserve ambiguous turn",
      objective: "Do not invent a terminal when bounded interruption times out.",
    });
    await service.start(item.id);
    const result = await service.shutdown({ timeoutMs: 25 });
    assert.equal(result.clean, false);
    assert.equal(result.reconciliation_required, true);
    assert.equal(store.getWorkItem(item.id).status, "running");
    assert.equal(service.reconciliationRequired(), true);
    assert.equal(store.listEvents(item.id).filter(({ kind, payload }) => (
      kind === "codex_disconnected" && payload.controlled_shutdown === true
    )).length, 1, "client-close disconnect erased the controlled-shutdown ambiguity");
  } finally {
    service.close();
    store.close();
  }
});

test("owned shutdown is clean when no active effect exists", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const result = await service.shutdown({ timeoutMs: 250 });
    assert.deepEqual(result, { clean: true, reconciliation_required: false });
    assert.deepEqual(codex.interruptCalls, []);
  } finally {
    service.close();
    store.close();
  }
});

test("fresh Control marks only inherited active rows as reconciliation-required", async () => {
  const store = new LatticeStore();
  const firstCodex = new FakeCodex();
  const first = new LatticeControlService({ store, codex: firstCodex });
  let second;
  try {
    const project = first.createProject({ name: "Inherited", rootPath: process.cwd() });
    const item = first.createWorkItem({
      projectId: project.id,
      title: "Inherited active turn",
      objective: "Separate current-process activity from crash-recovery ambiguity.",
    });
    await first.start(item.id);
    assert.equal(first.reconciliationRequired(), false);
    first.close();

    second = new LatticeControlService({ store, codex: new FakeCodex() });
    assert.equal(second.reconciliationRequired(), true);
  } finally {
    second?.close();
    first.close();
    store.close();
  }
});

test("owned shutdown applies one deadline to a never-settling operation", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const never = new Promise(() => {});
  codex.beforeTurnResult = async () => never;
  const service = new LatticeControlService({ store, codex, lifecycleTimeoutMs: 40 });
  try {
    const project = service.createProject({ name: "Stuck drain", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Never settling start",
      objective: "Force the parent-owned hard-kill fallback without closing the store early.",
    });
    const accepted = new Promise((resolve) => codex.once("turnStartAccepted", resolve));
    void service.start(item.id);
    await accepted;
    await assert.rejects(
      bounded(service.shutdown({ timeoutMs: 40 }), "bounded service shutdown", 500),
      (error) => error?.code === "CONTROL_SHUTDOWN_DRAIN_TIMEOUT",
    );
    assert.equal(store.getWorkItem(item.id).status, "starting");
    assert.equal(service.reconciliationRequired(), true);
  } finally {
    service.close();
    store.close();
  }
});

test("owned shutdown applies its absolute deadline to Codex close", { timeout: 2_000 }, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-codex-close-deadline-"));
  const codex = new FakeCodex();
  codex.close = async () => new Promise(() => {});
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
    mcpHealth: {
      current: async () => ({ work_mcp: "HEALTHY", decision_mcp: "HEALTHY" }),
    },
  });
  try {
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    await assert.rejects(
      bounded(application.shutdownOwned({ timeoutMs: 40 }), "bounded Codex close", 500),
      (error) => error?.code === "CONTROL_SHUTDOWN_DRAIN_TIMEOUT",
    );
    assert.doesNotThrow(() => application.store.listWorkItems());
  } finally {
    if (application.server.listening) {
      await new Promise((resolve) => application.server.close(resolve));
    }
    application.service.close();
    try { application.store.close(); } catch { /* closed only after a complete owned shutdown */ }
    await rm(directory, { recursive: true, force: true });
  }
});

test("owned shutdown retains listener ownership until services and store are closed", { timeout: 2_000 }, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-listener-ownership-"));
  const codex = new FakeCodex();
  let closeStartedResolve;
  let closeReleaseResolve;
  const closeStarted = new Promise((resolve) => { closeStartedResolve = resolve; });
  const closeRelease = new Promise((resolve) => { closeReleaseResolve = resolve; });
  codex.close = async () => {
    codex.closeCalls += 1;
    closeStartedResolve();
    await closeRelease;
  };
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
    mcpHealth: {
      current: async () => ({ work_mcp: "HEALTHY", decision_mcp: "HEALTHY" }),
    },
  });
  const contender = createHttpServer();
  let shuttingDown;
  try {
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    shuttingDown = application.shutdownOwned({ timeoutMs: 750 });
    await bounded(closeStarted, "Codex close admission", 250);
    const bindResult = await new Promise((resolve) => {
      contender.once("error", (error) => resolve({ error }));
      contender.listen(port, "127.0.0.1", () => resolve({ error: null }));
    });
    if (contender.listening) {
      await new Promise((resolve) => contender.close(resolve));
    }
    assert.equal(bindResult.error?.code, "EADDRINUSE");
    closeReleaseResolve();
    assert.deepEqual(await shuttingDown, { clean: true, reconciliation_required: false });
  } finally {
    closeReleaseResolve?.();
    await shuttingDown?.catch(() => {});
    if (contender.listening) await new Promise((resolve) => contender.close(resolve));
    if (application.server.listening) {
      await new Promise((resolve) => application.server.close(resolve));
    }
    application.service.close();
    try { application.store.close(); } catch { /* closed by graceful shutdown */ }
    await rm(directory, { recursive: true, force: true });
  }
});

test("owned shutdown keeps its listener bound when store close fails", { timeout: 2_000 }, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-store-close-ownership-"));
  let mcpHealthCalls = 0;
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex: new FakeCodex(),
    mcpHealth: {
      current: async () => {
        mcpHealthCalls += 1;
        return { work_mcp: "HEALTHY", decision_mcp: "HEALTHY" };
      },
    },
  });
  const originalStoreClose = application.store.close.bind(application.store);
  const contender = createHttpServer();
  try {
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    application.store.close = () => {
      const error = new Error("simulated SQLite close failure");
      error.code = "CONTROL_STORE_CLOSE_FAILED";
      throw error;
    };
    await assert.rejects(
      application.shutdownOwned({ timeoutMs: 750 }),
      (error) => error?.code === "CONTROL_STORE_CLOSE_FAILED",
    );

    const bindResult = await new Promise((resolve) => {
      contender.once("error", (error) => resolve({ error }));
      contender.listen(port, "127.0.0.1", () => resolve({ error: null }));
    });
    if (contender.listening) await new Promise((resolve) => contender.close(resolve));
    assert.equal(bindResult.error?.code, "EADDRINUSE");

    const rejected = await fetch(`http://127.0.0.1:${port}/api/runtime`);
    assert.equal(rejected.status, 503);
    assert.equal((await rejected.json()).code, "CONTROL_SHUTTING_DOWN");
    const rejectedPost = await fetch(`http://127.0.0.1:${port}/api/work-items`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    });
    assert.equal(rejectedPost.status, 503);
    assert.equal((await rejectedPost.json()).code, "CONTROL_SHUTTING_DOWN");
    assert.equal(mcpHealthCalls, 0, "shutdown rejections must not reach MCP health probes");
  } finally {
    application.store.close = originalStoreClose;
    if (contender.listening) await new Promise((resolve) => contender.close(resolve));
    if (application.server.listening) {
      await new Promise((resolve) => application.server.close(resolve));
    }
    application.service.close();
    try { originalStoreClose(); } catch { /* closed during cleanup */ }
    await rm(directory, { recursive: true, force: true });
  }
});

test("owned shutdown drains an admitted runtime GET before closing its store", { timeout: 2_000 }, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-runtime-get-drain-"));
  let probeEnteredResolve;
  let probeReleaseResolve;
  const probeEntered = new Promise((resolve) => { probeEnteredResolve = resolve; });
  const probeRelease = new Promise((resolve) => { probeReleaseResolve = resolve; });
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex: new FakeCodex(),
    mcpHealth: {
      current: async () => {
        probeEnteredResolve();
        await probeRelease;
        return { work_mcp: "HEALTHY", decision_mcp: "HEALTHY" };
      },
    },
  });
  const originalStoreClose = application.store.close.bind(application.store);
  let storeClosed = false;
  let shuttingDown;
  try {
    application.store.close = () => {
      storeClosed = true;
      originalStoreClose();
    };
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    const runtimeRequest = fetch(`http://127.0.0.1:${port}/api/runtime`);
    await bounded(probeEntered, "runtime probe admission", 250);
    shuttingDown = application.shutdownOwned({ timeoutMs: 750 });
    await new Promise((resolve) => setTimeout(resolve, 25));
    assert.equal(storeClosed, false);

    probeReleaseResolve();
    assert.equal((await runtimeRequest).status, 200);
    assert.deepEqual(await shuttingDown, { clean: true, reconciliation_required: false });
    assert.equal(storeClosed, true);
  } finally {
    probeReleaseResolve?.();
    await shuttingDown?.catch(() => {});
    if (application.server.listening) {
      await new Promise((resolve) => application.server.close(resolve));
    }
    application.service.close();
    if (!storeClosed) {
      try { originalStoreClose(); } catch { /* closed during cleanup */ }
    }
    await rm(directory, { recursive: true, force: true });
  }
});

test("owned shutdown rescans and interrupts a turn resumed after the first snapshot", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex, lifecycleTimeoutMs: 500 });
  try {
    const project = service.createProject({ name: "Late resume", rootPath: process.cwd() });
    const accepted = new Promise((resolve) => codex.once("turnStartAccepted", resolve));
    const sending = service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "late-resume-message-001",
      text: "關閉開始後不得漏掉晚到的 active turn。",
    });
    const started = await accepted;
    codex.emit("notification", {
      method: "turn/started",
      params: {
        threadId: started.threadId,
        turn: { id: started.turnId, status: "inProgress", items: [] },
      },
    });
    const sent = await sending;
    codex.emit("disconnect", { code: 17, signal: null });
    assert.equal(service.primaryConversation().status, "failed");

    let resumeEnteredResolve;
    let resumeReleaseResolve;
    const resumeEntered = new Promise((resolve) => { resumeEnteredResolve = resolve; });
    const resumeRelease = new Promise((resolve) => { resumeReleaseResolve = resolve; });
    codex.beforeResumeResult = async () => {
      resumeEnteredResolve();
      await resumeRelease;
    };
    codex.resumeResult = {
      id: sent.codex_thread_id,
      turns: [{ id: sent.codex_turn_id, status: "inProgress", items: [] }],
    };
    const reconnecting = service.reconnectPrimaryConversation();
    await bounded(resumeEntered, "deferred resume", 250);
    const shuttingDown = service.shutdown({ timeoutMs: 600 });
    await new Promise((resolve) => setTimeout(resolve, 180));
    const interruptAccepted = new Promise((resolve) => codex.once("interruptAccepted", resolve));
    resumeReleaseResolve();
    const driveTerminal = (async () => {
      const interrupted = await bounded(interruptAccepted, "late resume interrupt", 400);
      codex.emit("notification", {
        method: "turn/completed",
        params: {
          threadId: interrupted.threadId,
          turn: { id: interrupted.turnId, status: "interrupted", items: [] },
        },
      });
    })();
    const outcome = await bounded(
      Promise.all([shuttingDown, driveTerminal]).then(([value]) => value),
      "late resume shutdown",
      900,
    );
    await Promise.allSettled([reconnecting]);
    assert.equal(codex.interruptCalls.length, 1);
    assert.equal(outcome.clean, true);
    assert.equal(store.getWorkItem(sent.id).status, "failed");
  } finally {
    service.close();
    store.close();
  }
});

test("owned shutdown rejects a slow POST after its body crosses the admission boundary", { timeout: 2_000 }, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-slow-post-shutdown-"));
  const databasePath = path.join(directory, "control.db");
  const application = createLatticeServer({
    databasePath,
    codex: new FakeCodex(),
    mcpHealth: {
      current: async () => ({ work_mcp: "HEALTHY", decision_mcp: "HEALTHY" }),
    },
  });
  try {
    const project = application.service.createProject({ name: "Slow body", rootPath: directory });
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    const payload = JSON.stringify({
      projectId: project.id,
      title: "Must not be written",
      objective: "The body completed only after shutdown stopped admission.",
    });
    const requestObserved = new Promise((resolve) => application.server.once("request", resolve));
    let clientRequest;
    const responsePromise = new Promise((resolve, reject) => {
      clientRequest = httpRequest({
        hostname: "127.0.0.1",
        port,
        path: "/api/work-items",
        method: "POST",
        headers: {
          "content-type": "application/json",
          "content-length": Buffer.byteLength(payload),
        },
      }, (response) => {
        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("end", () => resolve({
          status: response.statusCode,
          body: JSON.parse(Buffer.concat(chunks).toString("utf8")),
        }));
      });
      clientRequest.on("error", reject);
    });
    const split = Math.max(1, Math.floor(payload.length / 2));
    clientRequest.write(payload.slice(0, split));
    await requestObserved;
    const shuttingDown = application.shutdownOwned({ timeoutMs: 750 });
    clientRequest.end(payload.slice(split));
    const response = await responsePromise;
    assert.equal(response.status, 503);
    assert.equal(response.body.code, "CONTROL_SHUTTING_DOWN");
    assert.deepEqual(await shuttingDown, { clean: true, reconciliation_required: false });
    const replay = new LatticeStore(databasePath);
    try {
      assert.equal(replay.listWorkItems().length, 0);
    } finally {
      replay.close();
    }
  } finally {
    if (application.server.listening) {
      await new Promise((resolve) => application.server.close(resolve));
    }
    application.service.close();
    try { application.store.close(); } catch { /* already closed by graceful shutdown */ }
    await rm(directory, { recursive: true, force: true });
  }
});

test("fresh Control exposes inherited ambiguity but admits only exact recovery mutations", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-recovery-admission-"));
  const databasePath = path.join(directory, "control.db");
  const seed = new LatticeStore(databasePath);
  const project = seed.createProject({ name: "Recovery gate", rootPath: directory });
  const item = seed.createWorkItem({
    projectId: project.id,
    title: "Inherited active",
    objective: "Require an exact terminal before accepting new effects.",
  });
  seed.updateWorkItem(item.id, {
    status: "running",
    codex_thread_id: "thread-1",
    codex_turn_id: "turn-1",
    progress: "inherited active turn",
  });
  seed.appendEvent(item.id, "codex_started", {
    threadId: "thread-1",
    turnId: "turn-1",
    confirmedBy: "turn/started",
  });
  seed.close();

  const application = createLatticeServer({
    databasePath,
    codex: new FakeCodex(),
    mcpHealth: {
      current: async () => ({ work_mcp: "HEALTHY", decision_mcp: "HEALTHY" }),
    },
  });
  try {
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    const origin = `http://127.0.0.1:${port}`;
    const runtimeBefore = await (await fetch(`${origin}/api/runtime`)).json();
    assert.equal(runtimeBefore.reconciliation_required, true);

    const denied = await fetch(`${origin}/api/work-items`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        projectId: project.id,
        title: "Denied until recovery",
        objective: "Must not be persisted while inherited state is ambiguous.",
      }),
    });
    assert.equal(denied.status, 409);
    assert.equal((await denied.json()).code, "CONTROL_RECONCILIATION_REQUIRED");

    const unrelatedConversation = await fetch(`${origin}/api/conversation/reconnect`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    });
    assert.equal(unrelatedConversation.status, 409);
    assert.equal((await unrelatedConversation.json()).code, "CONTROL_RECONCILIATION_REQUIRED");
    const unrelatedItem = await fetch(`${origin}/api/work-items/not-the-ambiguous-item/reconcile`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    });
    assert.equal(unrelatedItem.status, 409);
    assert.equal((await unrelatedItem.json()).code, "CONTROL_RECONCILIATION_REQUIRED");

    const reconciled = await fetch(`${origin}/api/work-items/${item.id}/reconcile`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    });
    assert.equal(reconciled.status, 200);
    assert.equal((await (await fetch(`${origin}/api/runtime`)).json()).reconciliation_required, false);
    assert.equal(application.store.listWorkItems().filter(({ title }) => (
      title === "Denied until recovery"
    )).length, 0);
  } finally {
    await new Promise((resolve) => application.server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});

test("failed primary recovery keeps admission closed until an exact terminal", { timeout: 3_000 }, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-primary-recovery-gate-"));
  const databasePath = path.join(directory, "control.db");
  let firstStore = new LatticeStore(databasePath);
  const firstCodex = new FakeCodex();
  let firstService = new LatticeControlService({ store: firstStore, codex: firstCodex });
  let application;
  try {
    const project = firstService.createProject({ name: "Primary recovery", rootPath: directory });
    const accepted = new Promise((resolve) => firstCodex.once("turnStartAccepted", resolve));
    const sending = firstService.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "primary-recovery-message-001",
      text: "只有精確終態可以解除 inherited recovery gate。",
    });
    const started = await accepted;
    firstCodex.emit("notification", {
      method: "turn/started",
      params: {
        threadId: started.threadId,
        turn: { id: started.turnId, status: "inProgress", items: [] },
      },
    });
    const sent = await sending;
    firstService.close();
    firstService = null;
    firstStore.close();
    firstStore = null;

    const recoveringCodex = new FakeCodex();
    recoveringCodex.resumeError = Object.assign(new Error("resume transport unavailable"), {
      code: "CODEX_APP_SERVER_TRANSPORT_ERROR",
    });
    application = createLatticeServer({
      databasePath,
      codex: recoveringCodex,
      mcpHealth: {
        current: async () => ({ work_mcp: "HEALTHY", decision_mcp: "HEALTHY" }),
      },
    });
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    const origin = `http://127.0.0.1:${port}`;
    assert.equal((await (await fetch(`${origin}/api/runtime`)).json()).reconciliation_required, true);

    const failedRecovery = await fetch(`${origin}/api/conversation/reconnect`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    });
    assert.notEqual(failedRecovery.status, 200);
    assert.equal(application.store.getWorkItem(sent.id).status, "running");
    assert.equal((await (await fetch(`${origin}/api/runtime`)).json()).reconciliation_required, true);
    const denied = await fetch(`${origin}/api/work-items`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        projectId: project.id,
        title: "Still denied",
        objective: "A transport error is not terminal evidence.",
      }),
    });
    assert.equal(denied.status, 409);

    recoveringCodex.resumeError = null;
    recoveringCodex.resumeResult = {
      id: sent.codex_thread_id,
      turns: [{ id: sent.codex_turn_id, status: "interrupted", items: [] }],
    };
    const reconciled = await fetch(`${origin}/api/conversation/reconnect`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    });
    assert.equal(reconciled.status, 200);
    assert.equal((await (await fetch(`${origin}/api/runtime`)).json()).reconciliation_required, false);
  } finally {
    firstService?.close();
    firstStore?.close();
    if (application?.server.listening) {
      await new Promise((resolve) => application.server.close(resolve));
    }
    await rm(directory, { recursive: true, force: true });
  }
});

test("generic interrupt timeout keeps durable reconciliation active across restart", { timeout: 2_000 }, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-generic-interrupt-recovery-"));
  const databasePath = path.join(directory, "control.db");
  let store = new LatticeStore(databasePath);
  let service;
  let replayStore;
  let replayService;
  try {
    const codex = new FakeCodex();
    service = new LatticeControlService({ store, codex, lifecycleTimeoutMs: 30 });
    const project = service.createProject({ name: "Generic recovery", rootPath: directory });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Inherited generic turn",
      objective: "An interrupt timeout must remain ambiguous after restart.",
    });
    const peer = service.createWorkItem({
      projectId: project.id,
      title: "Peer turn sharing the App Server",
      objective: "The shared transport loss makes every active peer ambiguous.",
    });
    store.updateWorkItem(item.id, {
      status: "running",
      codex_thread_id: "generic-thread-1",
      codex_turn_id: "generic-turn-1",
    });
    store.updateWorkItem(peer.id, {
      status: "running",
      codex_thread_id: "generic-thread-peer",
      codex_turn_id: "generic-turn-peer",
    });
    store.appendEvent(item.id, "codex_started", {
      threadId: "generic-thread-1",
      turnId: "generic-turn-1",
      confirmedBy: "turn/started",
    });
    codex.activeTurns.set("generic-thread-1", "generic-turn-1");
    codex.disconnectOnInterruptTimeout = true;
    await assert.rejects(
      bounded(service.interrupt(item.id), "generic interrupt timeout", 250),
      /timed out/u,
    );
    assert.equal(store.getWorkItem(item.id).status, "running");
    assert.equal(store.getWorkItem(peer.id).status, "running");
    assert.equal(service.reconciliationRequired(item.id), true);
    assert.equal(service.reconciliationRequired(peer.id), true);
    service.close();
    service = null;
    store.close();
    store = null;

    replayStore = new LatticeStore(databasePath);
    replayService = new LatticeControlService({ store: replayStore, codex: new FakeCodex() });
    assert.equal(replayStore.getWorkItem(item.id).status, "running");
    assert.equal(replayStore.getWorkItem(peer.id).status, "running");
    assert.equal(replayService.reconciliationRequired(item.id), true);
    assert.equal(replayService.reconciliationRequired(peer.id), true);
  } finally {
    replayService?.close();
    replayStore?.close();
    service?.close();
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("primary interrupt timeout keeps durable reconciliation active across restart", { timeout: 2_000 }, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-primary-interrupt-recovery-"));
  const databasePath = path.join(directory, "control.db");
  let firstStore = new LatticeStore(databasePath);
  let firstService = new LatticeControlService({
    store: firstStore,
    codex: new FakeCodex(),
    lifecycleTimeoutMs: 30,
  });
  let replayStore;
  let replayService;
  try {
    const firstCodex = firstService.codex;
    const project = firstService.createProject({ name: "Primary interrupt", rootPath: directory });
    const accepted = new Promise((resolve) => firstCodex.once("turnStartAccepted", resolve));
    const sending = firstService.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "primary-interrupt-message-001",
      text: "Interrupt timeout must remain recovery-only after restart.",
    });
    const started = await accepted;
    firstCodex.emit("notification", {
      method: "turn/started",
      params: {
        threadId: started.threadId,
        turn: { id: started.turnId, status: "inProgress", items: [] },
      },
    });
    const sent = await sending;
    firstCodex.disconnectOnInterruptTimeout = true;
    await assert.rejects(
      bounded(firstService.interruptPrimaryConversation(), "primary interrupt timeout", 250),
      /timed out/u,
    );
    assert.equal(firstStore.getWorkItem(sent.id).status, "running");
    assert.equal(firstService.reconciliationRequired(sent.id), true);
    firstService.close();
    firstService = null;
    firstStore.close();
    firstStore = null;

    replayStore = new LatticeStore(databasePath);
    replayService = new LatticeControlService({ store: replayStore, codex: new FakeCodex() });
    assert.equal(replayStore.getWorkItem(sent.id).status, "running");
    assert.equal(replayService.reconciliationRequired(sent.id), true);
  } finally {
    replayService?.close();
    replayStore?.close();
    firstService?.close();
    firstStore?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("parallel start calls for one work item create only one Codex thread", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex({ autoTurnStarted: false });
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Single dispatch", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "One work item, one thread",
      objective: "Reject or coalesce a concurrent duplicate start.",
    });
    const accepted = new Promise((resolve) => codex.once("turnStartAccepted", resolve));
    const results = Promise.allSettled([
      service.start(item.id),
      service.start(item.id),
    ]);
    await accepted;
    await new Promise((resolve) => setImmediate(resolve));

    for (const { threadId, turnId } of codex.turnStarts) {
      codex.emit("notification", {
        method: "turn/started",
        params: { threadId, turn: { id: turnId, items: [], status: "inProgress" } },
      });
    }
    const settled = await results;

    assert.equal(codex.threadStarts.length, 1);
    assert.equal(codex.turnStarts.length, 1);
    assert.ok(settled.some(({ status }) => status === "fulfilled"));
  } finally {
    service.close();
    store.close();
  }
});

test("two control processes atomically claim one work item before creating a thread", { timeout: 2_000 }, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-control-claim-"));
  const databasePath = path.join(directory, "control.db");
  const firstStore = new LatticeStore(databasePath);
  const firstCodex = new FakeCodex({ autoTurnStarted: false });
  const firstService = new LatticeControlService({ store: firstStore, codex: firstCodex });
  let secondStore;
  let secondService;
  try {
    const project = firstService.createProject({ name: "Atomic claim", rootPath: directory });
    const item = firstService.createWorkItem({
      projectId: project.id,
      title: "One durable owner",
      objective: "Only one process may establish the Codex thread.",
    });
    secondStore = new LatticeStore(databasePath);
    const secondCodex = new FakeCodex({ autoTurnStarted: false });
    secondService = new LatticeControlService({ store: secondStore, codex: secondCodex });

    const starts = [firstService.start(item.id), secondService.start(item.id)];
    await new Promise((resolve) => setImmediate(resolve));
    const owner = [firstCodex, secondCodex].find((codex) => codex.turnStarts.length === 1);
    assert.ok(owner, "one process must own the accepted turn");
    assert.equal(firstCodex.threadStarts.length + secondCodex.threadStarts.length, 1);
    const { threadId, turnId } = owner.turnStarts[0];
    owner.emit("notification", {
      method: "turn/started",
      params: { threadId, turn: { id: turnId, status: "inProgress" } },
    });
    const settled = await bounded(Promise.allSettled(starts), "cross-process atomic claim");
    assert.equal(settled.filter(({ status }) => status === "fulfilled").length, 1);
    assert.equal(settled.filter(({ status }) => status === "rejected").length, 1);
  } finally {
    secondService?.close();
    secondStore?.close();
    firstService.close();
    firstStore.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("only the exact turn terminal completes work and duplicate terminals are idempotent", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex({ autoTurnStarted: false });
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Terminal correlation", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Correlate terminal event",
      objective: "Ignore stale and duplicate turn completion notifications.",
    });
    const accepted = new Promise((resolve) => codex.once("turnStartAccepted", resolve));
    const starting = service.start(item.id);
    const { threadId, turnId } = await accepted;
    codex.emit("notification", {
      method: "turn/started",
      params: { threadId, turn: { id: turnId, items: [], status: "inProgress" } },
    });
    await starting;

    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: "thread-foreign",
        turn: { id: turnId, items: [], status: "completed" },
      },
    });
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId,
        turn: { id: "turn-stale", items: [], status: "completed" },
      },
    });
    const afterStale = store.getWorkItem(item.id);
    const eventsAfterStale = store.listEvents(item.id).filter(({ kind }) => kind === "turn_completed");

    const exactTerminal = {
      method: "turn/completed",
      params: {
        threadId,
        turn: { id: turnId, items: [], status: "completed" },
      },
    };
    codex.emit("notification", exactTerminal);
    const afterExact = store.getWorkItem(item.id);
    const eventsAfterExact = store.listEvents(item.id).filter(({ kind }) => kind === "turn_completed");
    codex.emit("notification", exactTerminal);
    const eventsAfterDuplicate = store.listEvents(item.id)
      .filter(({ kind }) => kind === "turn_completed");
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId,
        turn: { id: turnId, items: [], status: "failed", error: { message: "late conflict" } },
      },
    });
    const afterConflict = store.getWorkItem(item.id);
    const eventsAfterConflict = store.listEvents(item.id)
      .filter(({ kind }) => kind === "turn_completed");

    assert.equal(afterStale.status, "running");
    assert.equal(eventsAfterStale.length, 0);
    assert.equal(afterExact.status, "codex_done");
    assert.equal(eventsAfterExact.length, 1);
    assert.equal(eventsAfterExact[0].payload.threadId, threadId);
    assert.equal(eventsAfterExact[0].payload.turnId, turnId);
    assert.equal(eventsAfterDuplicate.length, 1);
    assert.equal(afterConflict.status, "codex_done", "the first exact terminal is authoritative");
    assert.equal(eventsAfterConflict.length, 1, "a conflicting late terminal is not a second outcome");
  } finally {
    service.close();
    store.close();
  }
});

test("MCP startup status persists every diagnostic field", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex({ autoTurnStarted: false });
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "MCP diagnostics", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Preserve MCP startup status",
      objective: "Keep the exact App Server diagnostic fields for inspection.",
    });
    const accepted = new Promise((resolve) => codex.once("turnStartAccepted", resolve));
    const starting = service.start(item.id);
    const { threadId, turnId } = await accepted;
    codex.emit("notification", {
      method: "turn/started",
      params: { threadId, turn: { id: turnId, items: [], status: "inProgress" } },
    });
    await starting;

    const diagnostic = {
      threadId,
      name: "github",
      status: "failed",
      error: "OAuth token expired",
      failureReason: "reauthenticationRequired",
    };
    codex.emit("notification", {
      method: "mcpServer/startupStatus/updated",
      params: diagnostic,
    });

    const event = store.listEvents(item.id)
      .find(({ kind }) => kind === "mcp_server_startup_status_updated");
    assert.ok(event);
    for (const [key, value] of Object.entries(diagnostic)) {
      assert.deepEqual(event.payload[key], value);
    }
  } finally {
    service.close();
    store.close();
  }
});

test("oversized MCP diagnostics stay bounded without failing the primary conversation", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Bounded MCP diagnostics", rootPath: process.cwd() });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "bounded-mcp-diagnostic-001",
      text: "超大的 MCP 診斷不能讓主要對話失敗。",
    });
    codex.emit("notification", {
      method: "mcpServer/startupStatus/updated",
      params: {
        threadId: sent.codex_thread_id,
        name: "diagnostic-server",
        status: "failed",
        error: "錯".repeat(20_000),
        failureReason: "reason".repeat(5_000),
      },
    });

    assert.equal(service.primaryConversation().status, "running");
    const events = store.listEvents("primary");
    const diagnostic = events.filter(
      ({ kind }) => kind === "mcp_server_startup_status_updated",
    ).at(-1);
    assert.ok(diagnostic);
    assert.equal(diagnostic.payload.truncated, true);
    assert.ok(Buffer.byteLength(JSON.stringify(diagnostic.payload), "utf8") <= 16_384);
    assert.match(diagnostic.payload.error, /\[truncated\]$/u);
    assert.equal(events.some(({ kind }) => kind === "conversation_notification_failed"), false);
  } finally {
    service.close();
    store.close();
  }
});

test("an approval for the exact active turn is retained instead of auto-declined", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Fast", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Immediate approval",
      objective: "Run one command.",
    });
    const started = await service.start(item.id);
    codex.emit("serverRequest", {
      id: 7,
      method: "item/commandExecution/requestApproval",
      params: {
        threadId: started.codex_thread_id,
        turnId: started.codex_turn_id,
        reason: "Immediate approval",
      },
    });

    assert.equal(started.codex_thread_id, "thread-1");
    assert.equal(store.getWorkItem(item.id).status, "waiting_approval");
    assert.equal(store.getWorkItem(item.id).approval.requestId, 7);
    assert.deepEqual(codex.responses, []);
    const declined = await service.approve(item.id, "decline");
    assert.equal(declined.status, "running");
    assert.equal(declined.failure_summary, null);
  } finally {
    store.close();
  }
});

test("approval cancel waits for the exact interrupted terminal before failing work", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex, lifecycleTimeoutMs: 100 });
  try {
    const project = service.createProject({ name: "Cancel terminal", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Wait after cancel",
      objective: "Do not synthesize a terminal from an approval response.",
    });
    const started = await service.start(item.id);
    codex.emit("serverRequest", {
      id: 73,
      method: "item/commandExecution/requestApproval",
      params: {
        threadId: started.codex_thread_id,
        turnId: started.codex_turn_id,
        reason: "Cancel this turn",
      },
    });

    let settled = false;
    const cancelling = Promise.resolve(service.approve(item.id, "cancel"));
    cancelling.then(
      () => { settled = true; },
      () => { settled = true; },
    );
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(store.getWorkItem(item.id).status, "running");
    assert.equal(settled, false);

    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: started.codex_thread_id,
        turn: { id: "turn-stale", status: "interrupted" },
      },
    });
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(settled, false);

    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: started.codex_thread_id,
        turn: { id: started.codex_turn_id, status: "interrupted" },
      },
    });
    const cancelled = await bounded(cancelling, "approval cancel terminal");
    assert.equal(cancelled.status, "failed");
    assert.equal(cancelled.failure_summary, "Codex turn interrupted");
    assert.deepEqual(codex.responses.at(-1), { id: 73, result: { decision: "cancel" } });
  } finally {
    service.close();
    store.close();
  }
});

test("approval cancel accepts an exact natural completion without closing transport", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex, lifecycleTimeoutMs: 100 });
  try {
    const project = service.createProject({ name: "Cancel completion", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Natural completion during cancel",
      objective: "Accept an exact terminal that wins the cancellation race.",
    });
    const started = await service.start(item.id);
    codex.emit("serverRequest", {
      id: 74,
      method: "item/commandExecution/requestApproval",
      params: {
        threadId: started.codex_thread_id,
        turnId: started.codex_turn_id,
        itemId: "command-natural-completion",
      },
    });
    const cancelling = service.approve(item.id, "cancel");
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: started.codex_thread_id,
        turn: { id: started.codex_turn_id, status: "completed" },
      },
    });

    const completed = await bounded(cancelling, "approval cancel natural completion");
    assert.equal(completed.status, "codex_done");
    assert.equal(codex.closeCalls, 0);
    assert.deepEqual(codex.responses.at(-1), { id: 74, result: { decision: "cancel" } });
  } finally {
    service.close();
    store.close();
  }
});

test("an interrupted turn never becomes completed work", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Interrupt", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Interrupted",
      objective: "Start and interrupt.",
    });
    await service.start(item.id);
    codex.emit("notification", {
      method: "turn/completed",
      params: { threadId: "thread-1", turn: { id: "turn-1", status: "interrupted" } },
    });
    const result = store.getWorkItem(item.id);
    assert.equal(result.status, "failed");
    assert.equal(result.failure_summary, "Codex turn interrupted");
  } finally {
    store.close();
  }
});

test("service interrupt requires the exact active turn and waits for its exact terminal", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex, lifecycleTimeoutMs: 100 });
  try {
    const project = service.createProject({ name: "Interrupt lifecycle", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Interrupt exact active turn",
      objective: "Wait for the correlated interrupted terminal.",
    });
    const started = await bounded(service.start(item.id), "initial turn start");
    const threadId = started.codex_thread_id;
    const turnId = started.codex_turn_id;

    codex.activeTurns.delete(threadId);
    await assert.rejects(
      bounded(service.interrupt(item.id), "inactive interrupt"),
      /not confirmed active|no active Codex turn/iu,
    );
    assert.equal(codex.interruptCalls.length, 0);

    codex.emit("notification", {
      method: "turn/started",
      params: { threadId, turn: { id: turnId, status: "inProgress" } },
    });
    const accepted = new Promise((resolve) => codex.once("interruptAccepted", resolve));
    let settled = false;
    const interrupting = service.interrupt(item.id);
    interrupting.then(
      () => { settled = true; },
      () => { settled = true; },
    );
    await bounded(accepted, "interrupt RPC acceptance");
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(settled, false);

    codex.emit("notification", {
      method: "turn/completed",
      params: { threadId: "other-thread", turn: { id: turnId, status: "interrupted" } },
    });
    codex.emit("notification", {
      method: "turn/completed",
      params: { threadId, turn: { id: "other-turn", status: "failed" } },
    });
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(settled, false);
    assert.equal(store.getWorkItem(item.id).status, "running");

    codex.emit("notification", {
      method: "turn/completed",
      params: { threadId, turn: { id: turnId, status: "interrupted" } },
    });
    const interrupted = await bounded(interrupting, "exact interrupt terminal");
    assert.equal(interrupted.status, "failed");
    assert.deepEqual(codex.interruptCalls, [{ threadId, turnId, timeoutMs: 100 }]);
  } finally {
    service.close();
    store.close();
  }
});

test("an interrupted work item gets one correlated retry and never a second", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex, lifecycleTimeoutMs: 100 });
  try {
    const project = service.createProject({ name: "Bounded retry", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Retry once",
      objective: "Resume an interrupted turn exactly once.",
    });
    const first = await bounded(service.start(item.id), "initial retry fixture start");
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: first.codex_thread_id,
        turn: { id: first.codex_turn_id, status: "interrupted" },
      },
    });
    assert.equal(store.getWorkItem(item.id).status, "failed");

    codex.resumeResult = {
      id: first.codex_thread_id,
      turns: [{ id: first.codex_turn_id, status: "interrupted" }],
    };
    codex.autoTurnStarted = false;
    const accepted = new Promise((resolve) => codex.once("turnStartAccepted", resolve));
    const retrying = service.resume(item.id, "Retry after the confirmed interruption.");
    const retry = await bounded(accepted, "retry RPC acceptance");
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(store.getWorkItem(item.id).status, "starting");

    codex.emit("notification", {
      method: "turn/started",
      params: {
        threadId: "other-thread",
        turn: { id: retry.turnId, status: "inProgress" },
      },
    });
    codex.emit("notification", {
      method: "turn/started",
      params: {
        threadId: retry.threadId,
        turn: { id: "other-turn", status: "inProgress" },
      },
    });
    await new Promise((resolve) => setImmediate(resolve));
    assert.notEqual(store.getWorkItem(item.id).status, "running");

    codex.emit("notification", {
      method: "turn/started",
      params: {
        threadId: retry.threadId,
        turn: { id: retry.turnId, status: "inProgress" },
      },
    });
    const retried = await bounded(retrying, "exact retry turn start");
    assert.equal(retried.status, "running");
    assert.equal(retried.codex_turn_id, retry.turnId);

    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: retry.threadId,
        turn: { id: retry.turnId, status: "failed", error: { message: "retry failed" } },
      },
    });
    codex.resumeResult = {
      id: retry.threadId,
      turns: [
        { id: first.codex_turn_id, status: "interrupted" },
        { id: retry.turnId, status: "failed", error: { message: "retry failed" } },
      ],
    };
    await assert.rejects(
      bounded(service.resume(item.id, "Do not run twice."), "second retry rejection"),
      /retry.*already used/iu,
    );
    assert.equal(codex.turns, 2);
    assert.equal(
      store.listEvents(item.id).filter(({ kind }) => kind === "codex_retry_claimed").length,
      1,
    );
  } finally {
    service.close();
    store.close();
  }
});

test("unsupported and wrong-turn server requests are explicitly rejected without waiting", { timeout: 2_000 }, async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex, lifecycleTimeoutMs: 100 });
  try {
    const project = service.createProject({ name: "Request correlation", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "Reject unrelated requests",
      objective: "Do not wait on unsupported or stale approval requests.",
    });
    const started = await bounded(service.start(item.id), "request fixture start");

    codex.emit("serverRequest", {
      id: 81,
      method: "account/login/request",
      params: { threadId: started.codex_thread_id, turnId: started.codex_turn_id },
    });
    assert.equal(codex.rejectedRequests.length, 1);
    assert.equal(codex.rejectedRequests[0].id, 81);
    assert.equal(codex.rejectedRequests[0].error.code, -32601);

    codex.emit("serverRequest", {
      id: 82,
      method: "item/commandExecution/requestApproval",
      params: {
        threadId: started.codex_thread_id,
        turnId: "turn-stale",
        reason: "Stale approval",
      },
    });
    assert.deepEqual(codex.responses.at(-1), { id: 82, result: { decision: "decline" } });
    assert.equal(store.getWorkItem(item.id).status, "running");
    assert.equal(store.getWorkItem(item.id).approval, null);
    assert.equal(codex.deferredRequests.size, 0);
  } finally {
    service.close();
    store.close();
  }
});

test("closing control detaches Codex notifications before its store is closed", () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  service.close();
  store.close();

  assert.doesNotThrow(() => codex.emit("disconnect", { code: 0, signal: null }));
  assert.doesNotThrow(() => codex.emit("notification", { method: "turn/completed", params: {} }));
});

test("a new Codex thread receives a bounded continuation packet derived from its LATTICE work item", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Continuation", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "續接登入修復",
      objective: "找出登入失敗原因並完成聚焦驗證。",
      priority: "urgent",
    });
    store.updateWorkItem(item.id, {
      progress: "已定位登入逾時",
      failure_summary: "上一次 turn 被中斷",
    });
    store.appendEvent(item.id, "diagnostic_recorded", { ignored: "not part of the packet" });

    const packet = service.continuation(item.id);
    assert.deepEqual(packet, {
      schema_version: "lattice.control.continuation.v1",
      project: { name: "Continuation", root_path: path.resolve(process.cwd()) },
      work: {
        id: item.id,
        title: "續接登入修復",
        objective: "找出登入失敗原因並完成聚焦驗證。",
        priority: "urgent",
        status: "draft",
        codex_thread_id: null,
      },
      current: {
        progress: "已定位登入逾時",
        failure_summary: "上一次 turn 被中斷",
        verification_notes: null,
        next_action: "Start the work in a new Codex thread.",
      },
      evidence: { latest_event: "diagnostic_recorded" },
    });

    await service.start(item.id);
    assert.match(codex.lastTurn.text, /lattice\.control\.continuation\.v1/u);
    assert.match(codex.lastTurn.text, /找出登入失敗原因/u);
    assert.doesNotMatch(codex.lastTurn.text, /not part of the packet/u);
  } finally {
    store.close();
  }
});

test("continuation packets bound oversized untrusted work text", () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Bounded", rootPath: process.cwd() });
    const item = service.createWorkItem({
      projectId: project.id,
      title: "A".repeat(600),
      objective: "B".repeat(3_000),
    });
    store.updateWorkItem(item.id, { failure_summary: "C".repeat(3_000) });

    const packet = service.continuation(item.id);
    assert.match(packet.work.title, /\[truncated\]$/u);
    assert.match(packet.work.objective, /\[truncated\]$/u);
    assert.match(packet.current.failure_summary, /\[truncated\]$/u);
    assert.ok(packet.work.title.length <= 256);
    assert.ok(packet.work.objective.length <= 2_048);
    assert.ok(packet.current.failure_summary.length <= 2_048);
  } finally {
    store.close();
  }
});

test("the primary conversation keeps one UI identity, persists real replies, and deduplicates sends", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-primary-conversation-"));
  const databasePath = path.join(directory, "control.db");
  let firstStore;
  let firstService;
  let restartedStore;
  let restartedService;
  try {
    firstStore = new LatticeStore(databasePath);
    const firstCodex = new FakeCodex();
    firstService = new LatticeControlService({ store: firstStore, codex: firstCodex });
    const project = firstService.createProject({ name: "Primary chat", rootPath: directory });

    const sent = await firstService.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "message-001",
      text: "請回覆第一則訊息。",
    });
    assert.equal(sent.id, "primary");
    assert.equal(sent.status, "running");
    assert.equal(sent.messages.length, 1);
    assert.deepEqual(sent.messages[0], {
      id: "message-001",
      role: "user",
      text: "請回覆第一則訊息。",
      delivery_status: "accepted",
      created_at: sent.messages[0].created_at,
      turn_id: "turn-1",
    });
    assert.equal(firstCodex.threadStarts.length, 1);
    assert.equal(firstCodex.turnStarts.length, 1);
    assert.equal(firstCodex.turnStarts[0].model, "gpt-5.6-luna");
    assert.equal(firstCodex.threadStarts[0].model, "gpt-5.6-luna");
    assert.equal(firstCodex.threadStarts[0].sandbox, "read-only");
    assert.equal(firstCodex.threadStarts[0].approvalPolicy, "never");
    assert.deepEqual(firstCodex.threadStarts[0].effectIdentity, {
      expectedGeneration: 1,
      expectedSessionId: "fake-app-server-session",
    });

    firstCodex.emit("notification", {
      method: "item/completed",
      params: {
        threadId: "thread-1",
        turnId: "turn-1",
        item: {
          id: "agent-message-001",
          type: "agentMessage",
          phase: "final_answer",
          text: "這是 Codex 的真實第一則回覆。",
        },
      },
    });
    firstCodex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: "thread-1",
        turn: {
          id: "turn-1",
          status: "completed",
          items: [{
            id: "agent-message-001",
            type: "agentMessage",
            phase: "final_answer",
            text: "這是 Codex 的真實第一則回覆。",
          }],
        },
      },
    });

    const completed = firstService.primaryConversation();
    assert.equal(completed.status, "codex_done");
    assert.equal(completed.messages.at(-1).role, "assistant");
    assert.equal(completed.messages.at(-1).text, "這是 Codex 的真實第一則回覆。");

    const duplicate = await firstService.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "message-001",
      text: "請回覆第一則訊息。",
    });
    assert.equal(duplicate.id, sent.id);
    assert.equal(firstCodex.threadStarts.length, 1);
    assert.equal(firstCodex.turnStarts.length, 1, "an exact retry must not start another turn");

    const terminalReconnect = await firstService.reconnectPrimaryConversation();
    assert.equal(terminalReconnect.status, "codex_done");
    const startEvidence = firstStore.listEvents("primary")
      .filter(({ kind }) => kind === "codex_started");
    assert.equal(startEvidence.length, 1);
    assert.equal(startEvidence[0].payload.confirmedBy, "turn/started");

    firstService.close();
    firstService = null;
    firstStore.close();
    firstStore = null;

    restartedStore = new LatticeStore(databasePath);
    const restartedCodex = new FakeCodex();
    restartedCodex.turns = 1;
    restartedCodex.resumeResult = {
      id: "thread-1",
      turns: [{
        id: "turn-1",
        status: "completed",
        items: [{
          id: "agent-message-001",
          type: "agentMessage",
          phase: "final_answer",
          text: "這是 Codex 的真實第一則回覆。",
        }],
      }],
    };
    restartedService = new LatticeControlService({ store: restartedStore, codex: restartedCodex });
    const reloaded = restartedService.primaryConversation();
    assert.equal(reloaded.id, sent.id);
    assert.equal(reloaded.messages.at(-1).text, "這是 Codex 的真實第一則回覆。");

    const continued = await restartedService.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "message-002",
      text: "請沿用同一條對話繼續。",
    });
    assert.equal(continued.id, sent.id);
    assert.equal(continued.codex_thread_id, "thread-1");
    assert.equal(continued.codex_turn_id, "turn-2");
    assert.deepEqual(restartedCodex.resumed, ["thread-1"]);
    assert.equal(restartedCodex.threadStarts.length, 0, "a recoverable thread must be resumed");
    assert.equal(restartedCodex.turnStarts.length, 1);
  } finally {
    restartedService?.close();
    restartedStore?.close();
    firstService?.close();
    firstStore?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("the primary conversation records a verifiable thread handoff when the project changes", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const firstProject = service.createProject({ name: "First project", rootPath: process.cwd() });
    const secondProject = service.createProject({ name: "Second project", rootPath: tmpdir() });
    const first = await service.sendPrimaryConversationMessage({
      projectId: firstProject.id,
      clientMessageId: "handoff-message-001",
      text: "先在第一個專案對話。",
    });
    codex.emit("notification", {
      method: "item/completed",
      params: {
        threadId: first.codex_thread_id,
        turnId: first.codex_turn_id,
        item: {
          id: "handoff-reply-001",
          type: "agentMessage",
          phase: "final_answer",
          text: "第一個專案的回覆。",
        },
      },
    });
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: first.codex_thread_id,
        turn: { id: first.codex_turn_id, status: "completed", items: [] },
      },
    });

    const second = await service.sendPrimaryConversationMessage({
      projectId: secondProject.id,
      clientMessageId: "handoff-message-002",
      text: "切到第二個專案，但保持同一個使用者對話。",
    });
    assert.equal(second.id, first.id);
    assert.equal(second.codex_thread_id, "thread-2");
    assert.equal(second.project_id, secondProject.id);
    assert.equal(codex.threadStarts.length, 2);
    assert.equal(codex.threadStarts[1].cwd, path.resolve(tmpdir()));
    assert.match(codex.turnStarts[1].text, /第一個專案的回覆/u);
    assert.match(codex.turnStarts[1].text, /切到第二個專案/u);
    assert.deepEqual(second.handoffs, [{
      from_thread_id: "thread-1",
      to_thread_id: "thread-2",
      reason: "project_changed",
      created_at: second.handoffs[0].created_at,
    }]);
  } finally {
    service.close();
    store.close();
  }
});

test("reconnect restores an active primary conversation without resending its message", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Reconnect", rootPath: process.cwd() });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "reconnect-message-001",
      text: "這一則只能送出一次。",
    });
    codex.emit("disconnect", { code: 17, signal: null });
    const disconnected = service.primaryConversation();
    assert.equal(disconnected.status, "failed");
    assert.match(disconnected.status_text, /連線中斷|重新連線/u);

    codex.resumeResult = {
      id: sent.codex_thread_id,
      turns: [{ id: sent.codex_turn_id, status: "inProgress", items: [] }],
    };
    const reconnected = await service.reconnectPrimaryConversation();
    assert.equal(reconnected.id, sent.id);
    assert.equal(reconnected.status, "running");
    assert.match(reconnected.status_text, /等待開始/u);
    assert.equal(codex.turnStarts.length, 1, "reconnect must not replay the user message");
    assert.deepEqual(codex.resumed, [sent.codex_thread_id]);
  } finally {
    service.close();
    store.close();
  }
});

test("primary conversation distinguishes queued work from visible reply generation", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Queue status", rootPath: process.cwd() });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "queue-status-message-001",
      text: "排隊時不可假裝正在產生回覆。",
    });

    const queued = service.primaryConversation();
    assert.equal(queued.status, "running");
    assert.equal(queued.can_interrupt, true);
    assert.match(queued.status_text, /等待開始/u);
    assert.doesNotMatch(queued.status_text, /正在回覆/u);

    store.database.prepare(`
      UPDATE work_items
      SET updated_at = ?
      WHERE id = ?
    `).run(new Date(Date.now() - 31_000).toISOString(), sent.id);
    const delayed = service.primaryConversation();
    assert.equal(delayed.can_interrupt, true);
    assert.match(delayed.status_text, /尚未開始執行/u);
    assert.match(delayed.status_text, /30 秒/u);

    codex.emit("notification", {
      method: "item/started",
      params: {
        threadId: sent.codex_thread_id,
        turnId: sent.codex_turn_id,
        item: { id: "queue-status-item-001", type: "userMessage" },
      },
    });
    assert.match(service.primaryConversation().status_text, /正在回覆/u);
  } finally {
    service.close();
    store.close();
  }
});

test("primary conversation records first activity and cancels no longer queued work", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({
    store,
    codex,
    conversationStartTimeoutMs: 25,
  });
  try {
    const project = service.createProject({ name: "Fast start", rootPath: process.cwd() });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "fast-start-message-001",
      text: "開始後應立即留下第一個活動時間。",
    });
    codex.emit("notification", {
      method: "item/completed",
      params: {
        threadId: sent.codex_thread_id,
        turnId: sent.codex_turn_id,
        item: {
          id: "fast-start-item-001",
          type: "agentMessage",
          phase: "final_answer",
          text: "已開始回覆。",
        },
      },
    });

    await new Promise((resolve) => setTimeout(resolve, 50));
    assert.equal(codex.interruptCalls.length, 0);
    const firstActivity = store.listEvents("primary")
      .find(({ kind }) => kind === "conversation_first_activity");
    assert.equal(firstActivity.payload.threadId, sent.codex_thread_id);
    assert.equal(firstActivity.payload.turnId, sent.codex_turn_id);
    assert.equal(firstActivity.payload.type, "agentMessage");
    assert.ok(Number.isInteger(firstActivity.payload.queueDurationMs));
    assert.ok(firstActivity.payload.queueDurationMs >= 0);
  } finally {
    service.close();
    store.close();
  }
});

test("primary conversation safely interrupts an exact turn that produces no activity before its SLA", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({
    store,
    codex,
    lifecycleTimeoutMs: 250,
    conversationStartTimeoutMs: 25,
  });
  try {
    const project = service.createProject({ name: "Queue timeout", rootPath: process.cwd() });
    const interruptAccepted = new Promise((resolve) => {
      codex.once("interruptAccepted", ({ threadId, turnId }) => {
        setImmediate(() => {
          codex.emit("notification", {
            method: "turn/completed",
            params: { threadId, turn: { id: turnId, status: "interrupted", items: [] } },
          });
          resolve({ threadId, turnId });
        });
      });
    });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "queue-timeout-message-001",
      text: "排隊太久時只中斷這一個回合。",
    });

    const interrupted = await bounded(interruptAccepted, "queue timeout interrupt", 500);
    await new Promise((resolve) => setImmediate(resolve));
    assert.deepEqual(interrupted, {
      threadId: sent.codex_thread_id,
      turnId: sent.codex_turn_id,
    });
    assert.deepEqual(codex.interruptCalls.map(({ threadId, turnId }) => ({ threadId, turnId })), [
      { threadId: sent.codex_thread_id, turnId: sent.codex_turn_id },
    ]);
    const conversation = service.primaryConversation();
    assert.equal(conversation.status, "failed");
    assert.equal(conversation.can_send, true);
    assert.match(conversation.status_text, /安全停止/u);
    assert.equal(codex.turnStarts.length, 1, "the timed-out message must never be resent");
    const timeout = store.listEvents("primary")
      .find(({ kind }) => kind === "conversation_start_timeout");
    assert.equal(timeout.payload.threadId, sent.codex_thread_id);
    assert.equal(timeout.payload.turnId, sent.codex_turn_id);
  } finally {
    service.close();
    store.close();
  }
});

test("a natural reply that wins the queue-timeout race remains completed", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({
    store,
    codex,
    lifecycleTimeoutMs: 250,
    conversationStartTimeoutMs: 25,
  });
  try {
    const project = service.createProject({ name: "Timeout race", rootPath: process.cwd() });
    codex.once("interruptAccepted", ({ threadId, turnId }) => {
      setImmediate(() => {
        const reply = {
          id: "timeout-race-reply-001",
          type: "agentMessage",
          phase: "final_answer",
          text: "自然完成的回覆必須保留。",
        };
        codex.emit("notification", {
          method: "item/completed",
          params: { threadId, turnId, item: reply },
        });
        codex.emit("notification", {
          method: "turn/completed",
          params: { threadId, turn: { id: turnId, status: "completed", items: [reply] } },
        });
      });
    });
    await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "timeout-race-message-001",
      text: "讓自然完成贏過逾時中斷。",
    });

    await new Promise((resolve) => setTimeout(resolve, 75));
    const conversation = service.primaryConversation();
    assert.equal(conversation.status, "codex_done");
    assert.equal(conversation.messages.at(-1).text, "自然完成的回覆必須保留。");
    assert.equal(
      store.listEvents("primary").some(({ kind }) => kind === "conversation_start_timeout"),
      false,
    );
  } finally {
    service.close();
    store.close();
  }
});

test("the owning adapter repairs a missed terminal notification without resending the message", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({
    store,
    codex,
    conversationObservationIntervalMs: 10,
  });
  try {
    const project = service.createProject({ name: "Fresh terminal", rootPath: process.cwd() });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "fresh-terminal-message-001",
      text: "這一則只能送出一次，遺漏終態時要自行核對。",
    });
    assert.equal(service.primaryConversation().status, "running");

    codex.readResults.set(sent.codex_thread_id, {
      id: sent.codex_thread_id,
      turns: [{
        id: sent.codex_turn_id,
        status: "interrupted",
        items: [],
      }],
    });
    const immediate = await service.refreshPrimaryConversationObservation();
    assert.equal(immediate.status, "running");
    assert.deepEqual(codex.readCalls, [], "a new turn must receive a grace period before probing");
    await new Promise((resolve) => setTimeout(resolve, 20));
    const reconciled = await service.refreshPrimaryConversationObservation();

    assert.equal(reconciled.status, "failed");
    assert.match(reconciled.last_error, /interrupted/u);
    assert.deepEqual(codex.readCalls, [sent.codex_thread_id]);
    assert.deepEqual(codex.freshReadCalls, [], "an active turn must never launch a second App Server probe");
    assert.equal(codex.turnStarts.length, 1, "fresh observation must not replay the user message");
    assert.equal(codex.closeCalls, 1, "a proven terminal must discard the stale active adapter");
    assert.equal(codex.connected, true, "the replacement adapter must be ready for the next message");
    assert.equal(
      store.listEvents("primary").filter(({ kind }) => kind === "turn_completed").length,
      1,
    );

    codex.resumeResult = {
      id: sent.codex_thread_id,
      turns: [{ id: sent.codex_turn_id, status: "interrupted", items: [] }],
    };
    const continued = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "fresh-terminal-message-002",
      text: "已確認終態後必須能開始下一則。",
    });
    assert.equal(continued.status, "running");
    assert.equal(continued.codex_turn_id, "turn-2");
    assert.equal(codex.turnStarts.length, 2);
    assert.deepEqual(codex.resumed, [sent.codex_thread_id], "the replacement adapter must resume the saved thread");
  } finally {
    service.close();
    store.close();
  }
});

test("reconnect repairs an active projection when its exact terminal event already exists", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-terminal-projection-"));
  const databasePath = path.join(directory, "control.db");
  let firstStore;
  let firstService;
  let restartedStore;
  let restartedService;
  try {
    firstStore = new LatticeStore(databasePath);
    const firstCodex = new FakeCodex();
    firstService = new LatticeControlService({ store: firstStore, codex: firstCodex });
    const project = firstService.createProject({ name: "Terminal projection", rootPath: directory });
    const sent = await firstService.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "terminal-projection-message-001",
      text: "終態投影必須可重建。",
    });
    firstCodex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: sent.codex_thread_id,
        turn: { id: sent.codex_turn_id, status: "interrupted", items: [] },
      },
    });
    assert.equal(firstService.primaryConversation().status, "failed");
    firstService.close();
    firstService = null;
    firstStore.database.prepare(`
      UPDATE work_items SET status = 'running', progress = 'stale projection'
      WHERE id = 'primary'
    `).run();
    firstStore.close();
    firstStore = null;

    restartedStore = new LatticeStore(databasePath);
    const restartedCodex = new FakeCodex();
    restartedCodex.resumeResult = {
      id: sent.codex_thread_id,
      turns: [{ id: sent.codex_turn_id, status: "interrupted", items: [] }],
    };
    restartedService = new LatticeControlService({
      store: restartedStore,
      codex: restartedCodex,
    });

    const reconciled = await restartedService.reconnectPrimaryConversation();
    assert.equal(reconciled.status, "failed");
    assert.match(reconciled.last_error, /interrupted/u);
    assert.equal(restartedCodex.turnStarts.length, 0);
  } finally {
    restartedService?.close();
    restartedStore?.close();
    firstService?.close();
    firstStore?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a late completed terminal with a saved final reply supersedes a premature interruption", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Late completion", rootPath: process.cwd() });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "late-completion-message-001",
      text: "回覆完成後必須離開正在回覆狀態。",
    });
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: sent.codex_thread_id,
        turn: { id: sent.codex_turn_id, status: "interrupted", items: [] },
      },
    });
    assert.equal(service.primaryConversation().status, "failed");

    const finalReply = {
      id: "late-completion-final-001",
      type: "agentMessage",
      phase: "final_answer",
      text: "LATTICE_CHAT_OK",
    };
    codex.emit("notification", {
      method: "item/completed",
      params: {
        threadId: sent.codex_thread_id,
        turnId: sent.codex_turn_id,
        item: finalReply,
      },
    });
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: sent.codex_thread_id,
        turn: {
          id: sent.codex_turn_id,
          status: "completed",
          items: [finalReply],
        },
      },
    });

    const completed = service.primaryConversation();
    assert.equal(completed.status, "codex_done");
    assert.equal(completed.can_send, true);
    assert.equal(completed.messages.at(-1).text, "LATTICE_CHAT_OK");
    assert.equal(
      store.primaryConversationTerminalEvent(sent.codex_thread_id, sent.codex_turn_id)
        .payload.status,
      "completed",
    );
    assert.equal(store.listEvents("primary").some(
      ({ kind, payload }) => kind === "turn_terminal_superseded"
        && payload.previousStatus === "interrupted"
        && payload.status === "completed",
    ), true);
  } finally {
    service.close();
    store.close();
  }
});

test("an owning-adapter read leaves a genuinely active turn running and is throttled", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({
    store,
    codex,
    conversationObservationIntervalMs: 10,
  });
  try {
    const project = service.createProject({ name: "Fresh active", rootPath: process.cwd() });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "fresh-active-message-001",
      text: "真正仍在執行的回覆不可被檢查中斷。",
    });
    codex.readResults.set(sent.codex_thread_id, {
      id: sent.codex_thread_id,
      turns: [{ id: sent.codex_turn_id, status: "inProgress", items: [] }],
    });

    const immediate = await service.refreshPrimaryConversationObservation();
    assert.equal(immediate.status, "running");
    assert.deepEqual(codex.freshReadCalls, [], "a new turn must not be probed immediately");
    await new Promise((resolve) => setTimeout(resolve, 20));
    const first = await service.refreshPrimaryConversationObservation();
    const second = await service.refreshPrimaryConversationObservation();

    assert.equal(first.status, "running");
    assert.equal(second.status, "running");
    assert.deepEqual(codex.readCalls, [sent.codex_thread_id]);
    assert.deepEqual(codex.freshReadCalls, []);
    assert.equal(codex.turnStarts.length, 1);
    assert.equal(codex.closeCalls, 0, "a passive observation must not close the active adapter");
  } finally {
    service.close();
    store.close();
  }
});

test("an active primary conversation survives a Control restart and resumes the exact turn", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-active-restart-"));
  const databasePath = path.join(directory, "control.db");
  let firstStore;
  let firstService;
  let restartedStore;
  let restartedService;
  try {
    firstStore = new LatticeStore(databasePath);
    const firstCodex = new FakeCodex();
    firstService = new LatticeControlService({ store: firstStore, codex: firstCodex });
    const project = firstService.createProject({ name: "Active restart", rootPath: directory });
    const sent = await firstService.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "active-restart-message-001",
      text: "Control 重啟時不能重送這一則。",
    });
    firstService.close();
    firstService = null;
    firstStore.close();
    firstStore = null;

    restartedStore = new LatticeStore(databasePath);
    const restartedCodex = new FakeCodex();
    restartedCodex.turns = 1;
    restartedCodex.resumeResult = {
      id: sent.codex_thread_id,
      turns: [{ id: sent.codex_turn_id, status: "inProgress", items: [] }],
    };
    restartedService = new LatticeControlService({ store: restartedStore, codex: restartedCodex });
    const beforeReconnect = restartedService.primaryConversation();
    assert.equal(beforeReconnect.id, sent.id);
    assert.match(beforeReconnect.status_text, /Control 已恢復|重新連線/u);
    assert.equal(beforeReconnect.can_interrupt, false);

    const resumed = await restartedService.reconnectPrimaryConversation();
    assert.equal(resumed.id, sent.id);
    assert.equal(resumed.codex_thread_id, sent.codex_thread_id);
    assert.equal(resumed.codex_turn_id, sent.codex_turn_id);
    assert.equal(resumed.status, "running");
    assert.deepEqual(restartedCodex.resumed, [sent.codex_thread_id]);
    assert.equal(restartedCodex.turnStarts.length, 0);
  } finally {
    restartedService?.close();
    restartedStore?.close();
    firstService?.close();
    firstStore?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a newer message cannot pass an earlier durable claim that is still unaccepted", () => {
  const store = new LatticeStore();
  try {
    const project = store.createProject({ name: "Ordered claims", rootPath: process.cwd() });
    const firstInput = {
      projectId: project.id,
      clientMessageId: "ordered-claim-message-001",
      text: "第一則訊息已保存，但尚未交給 Codex。",
    };
    const fence = acquirePrimaryConversationFence(store, project.id);
    const first = store.claimPrimaryConversationMessage({ ...firstInput, fence });
    store.updateWorkItem("primary", {
      status: "failed",
      failure_summary: "模擬 thread/start 前停止",
    }, fence);

    assert.throws(
      () => store.claimPrimaryConversationMessage({
        projectId: project.id,
        clientMessageId: "ordered-claim-message-002",
        text: "第二則訊息不得越過第一則。",
        fence,
      }),
      { code: "CONVERSATION_BUSY", status: 409 },
    );
    const retry = store.claimPrimaryConversationMessage({ ...firstInput, fence });
    assert.equal(retry.claimed, false, "the exact saved message remains the only retryable claim");
    assert.equal(store.listEvents("primary")
      .filter(({ kind }) => kind === "conversation_message_claimed").length, 1);
    assert.equal(first.event.payload.clientMessageId, retry.event.payload.clientMessageId);
  } finally {
    store.close();
  }
});

test("a saved message ignores an unbound empty thread after Control stops between claim and binding", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-claim-crash-"));
  const databasePath = path.join(directory, "control.db");
  const seedStore = new LatticeStore(databasePath);
  const project = seedStore.createProject({ name: "Claim crash", rootPath: directory });
  const input = {
    projectId: project.id,
    clientMessageId: "claim-crash-message-001",
    text: "這則訊息必須從 claim 復原且只送一次。",
  };
  const seedFence = acquirePrimaryConversationFence(seedStore, project.id);
  const claim = seedStore.claimPrimaryConversationMessage({ ...input, fence: seedFence });
  seedStore.releasePrimaryConversationLease(seedFence);
  seedStore.close();

  const codex = new FakeCodex();
  const remoteThreads = ["thread-claim-crash-a", "thread-claim-crash-b"].map((id) => ({
    id,
    cwd: path.resolve(directory),
    createdAt: Math.floor(Date.parse(claim.event.created_at) / 1_000),
    turns: [],
  }));
  codex.listResult = { data: remoteThreads, nextCursor: null };
  for (const remoteThread of remoteThreads) codex.readResults.set(remoteThread.id, remoteThread);
  const store = new LatticeStore(databasePath);
  const service = new LatticeControlService({ store, codex });
  try {
    const recovered = await service.sendPrimaryConversationMessage(input);
    assert.equal(recovered.status, "running");
    assert.equal(recovered.codex_thread_id, "thread-1");
    assert.equal(recovered.messages[0].delivery_status, "accepted");
    assert.deepEqual(codex.emptyResumed, []);
    assert.equal(codex.threadStarts.length, 1, "an unmarked empty thread must never be adopted");
    assert.equal(codex.turnStarts.length, 1);
    assert.equal(codex.listCalls, 0, "unbound claims must not scan or adopt ambient Codex threads");
    assert.equal(store.listEvents("primary").some(
      ({ kind, payload }) => kind === "conversation_unbound_claim_restarted"
        && payload.clientMessageId === input.clientMessageId,
    ), true);
  } finally {
    service.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a saved message resumes its bound empty thread after Control stops before turn dispatch", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-bind-crash-"));
  const databasePath = path.join(directory, "control.db");
  const seedStore = new LatticeStore(databasePath);
  const project = seedStore.createProject({ name: "Binding crash", rootPath: directory });
  const input = {
    projectId: project.id,
    clientMessageId: "bind-crash-message-001",
    text: "綁定後的訊息只能建立一個 turn。",
  };
  const seedFence = acquirePrimaryConversationFence(seedStore, project.id);
  seedStore.claimPrimaryConversationMessage({ ...input, fence: seedFence });
  seedStore.bindPrimaryConversationThread({
    projectId: project.id,
    threadId: "thread-bind-crash",
    previousThreadId: null,
    reason: "initial",
    fence: seedFence,
  });
  seedStore.releasePrimaryConversationLease(seedFence);
  seedStore.close();

  const codex = new FakeCodex();
  codex.readResults.set("thread-bind-crash", {
    id: "thread-bind-crash",
    cwd: path.resolve(directory),
    createdAt: Math.floor(Date.now() / 1_000),
    turns: [],
  });
  const store = new LatticeStore(databasePath);
  const service = new LatticeControlService({ store, codex });
  try {
    const recovered = await service.sendPrimaryConversationMessage(input);
    assert.equal(recovered.status, "running");
    assert.equal(recovered.codex_thread_id, "thread-bind-crash");
    assert.deepEqual(codex.emptyResumed, ["thread-bind-crash"]);
    assert.equal(codex.threadStarts.length, 0);
    assert.equal(codex.turnStarts.length, 1);
  } finally {
    service.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a marker-bound remote turn is adopted after Control stops before saving turn acceptance", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-turn-crash-"));
  const databasePath = path.join(directory, "control.db");
  const seedStore = new LatticeStore(databasePath);
  const project = seedStore.createProject({ name: "Turn crash", rootPath: directory });
  const input = {
    projectId: project.id,
    clientMessageId: "turn-crash-message-001",
    text: "遠端已接受的 turn 不得重送。",
  };
  const seedFence = acquirePrimaryConversationFence(seedStore, project.id);
  const claim = seedStore.claimPrimaryConversationMessage({ ...input, fence: seedFence });
  seedStore.bindPrimaryConversationThread({
    projectId: project.id,
    threadId: "thread-turn-crash",
    previousThreadId: null,
    reason: "initial",
    fence: seedFence,
  });
  seedStore.releasePrimaryConversationLease(seedFence);
  seedStore.close();

  const marker = `[LATTICE_CONTROL_MESSAGE id=${input.clientMessageId} digest=${claim.event.payload.promptDigest}]`;
  const remoteTurn = {
    id: "turn-crash-accepted",
    status: "inProgress",
    items: [{
      id: "turn-crash-user",
      type: "userMessage",
      content: [{ type: "text", text: `${marker}\n${input.text}` }],
    }],
  };
  const remoteThread = {
    id: "thread-turn-crash",
    cwd: path.resolve(directory),
    createdAt: Math.floor(Date.parse(claim.event.created_at) / 1_000),
    turns: [remoteTurn],
  };
  const codex = new FakeCodex();
  codex.readResults.set(remoteThread.id, remoteThread);
  codex.resumeResult = remoteThread;
  const store = new LatticeStore(databasePath);
  const service = new LatticeControlService({ store, codex });
  try {
    const recovered = await service.sendPrimaryConversationMessage(input);
    assert.equal(recovered.status, "running");
    assert.equal(recovered.codex_turn_id, remoteTurn.id);
    assert.equal(recovered.messages[0].delivery_status, "accepted");
    assert.equal(codex.turnStarts.length, 0, "the exact marker turn must not be replayed");
    assert.deepEqual(codex.resumed, [remoteThread.id]);
  } finally {
    service.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("concurrent reuse of one message ID with different content never shares a success", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex({ autoThreadStarted: false });
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Concurrent conflict", rootPath: process.cwd() });
    const threadAccepted = new Promise((resolve) => codex.once("threadStartAccepted", resolve));
    const first = service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "concurrent-conflict-message",
      text: "第一個內容。",
    });
    const thread = await threadAccepted;
    await assert.rejects(
      service.sendPrimaryConversationMessage({
        projectId: project.id,
        clientMessageId: "concurrent-conflict-message",
        text: "第二個不同內容。",
      }),
      { code: "CONVERSATION_MESSAGE_CONFLICT" },
    );
    codex.emit("notification", { method: "thread/started", params: { thread } });
    await first;
    assert.equal(codex.turnStarts.length, 1);
  } finally {
    service.close();
    store.close();
  }
});

test("an unrecoverable saved thread fails closed without starting a replacement", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "No replacement", rootPath: process.cwd() });
    const first = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "no-replacement-message-001",
      text: "先建立可驗證的舊回合。",
    });
    codex.emit("notification", {
      method: "item/completed",
      params: {
        threadId: first.codex_thread_id,
        turnId: first.codex_turn_id,
        item: {
          id: "no-replacement-final",
          type: "agentMessage",
          phase: "final_answer",
          text: "舊回合完成。",
        },
      },
    });
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: first.codex_thread_id,
        turn: { id: first.codex_turn_id, status: "completed", items: [] },
      },
    });
    const notRecoverable = new Error("saved thread missing");
    notRecoverable.code = "CODEX_THREAD_NOT_RECOVERABLE";
    codex.resumeError = notRecoverable;
    await assert.rejects(
      service.sendPrimaryConversationMessage({
        projectId: project.id,
        clientMessageId: "no-replacement-message-002",
        text: "不得自動另開 thread。",
      }),
      { code: "CONVERSATION_RECONCILIATION_REQUIRED" },
    );
    assert.equal(codex.threadStarts.length, 1);
    assert.equal(store.primaryConversationMessage("no-replacement-message-002"), null);
  } finally {
    service.close();
    store.close();
  }
});

test("ambiguous marker turns in the exact saved thread fail closed without replay", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-marker-ambiguous-"));
  const databasePath = path.join(directory, "control.db");
  const seedStore = new LatticeStore(databasePath);
  const project = seedStore.createProject({ name: "Ambiguous marker", rootPath: directory });
  const input = {
    projectId: project.id,
    clientMessageId: "ambiguous-marker-message-001",
    text: "同一 marker 若出現兩次必須停止。",
  };
  const seedFence = acquirePrimaryConversationFence(seedStore, project.id);
  const claim = seedStore.claimPrimaryConversationMessage({ ...input, fence: seedFence });
  seedStore.bindPrimaryConversationThread({
    projectId: project.id,
    threadId: "thread-marker-ambiguous",
    previousThreadId: null,
    reason: "initial",
    fence: seedFence,
  });
  seedStore.releasePrimaryConversationLease(seedFence);
  seedStore.close();
  const marker = `[LATTICE_CONTROL_MESSAGE id=${input.clientMessageId} digest=${claim.event.payload.promptDigest}]`;
  const codex = new FakeCodex();
  const candidate = {
    id: "thread-marker-ambiguous",
    cwd: path.resolve(directory),
    createdAt: Math.floor(Date.parse(claim.event.created_at) / 1_000),
    turns: [0, 1].map((index) => ({
      id: `turn-marker-${index}`,
      status: "inProgress",
      items: [{
        type: "userMessage",
        content: [{ type: "text", text: `${marker}\n${input.text}` }],
      }],
    })),
  };
  codex.readResults.set(candidate.id, candidate);
  const store = new LatticeStore(databasePath);
  const service = new LatticeControlService({ store, codex });
  try {
    await assert.rejects(
      service.sendPrimaryConversationMessage(input),
      { code: "CONVERSATION_RECONCILIATION_REQUIRED" },
    );
    assert.equal(codex.threadStarts.length, 0);
    assert.equal(codex.turnStarts.length, 0);
    assert.equal(codex.listCalls, 0);
  } finally {
    service.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a terminal marker turn restores its real final reply without a second dispatch", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-marker-terminal-"));
  const databasePath = path.join(directory, "control.db");
  const seedStore = new LatticeStore(databasePath);
  const project = seedStore.createProject({ name: "Terminal marker", rootPath: directory });
  const input = {
    projectId: project.id,
    clientMessageId: "terminal-marker-message-001",
    text: "找回已完成的 marker turn。",
  };
  const seedFence = acquirePrimaryConversationFence(seedStore, project.id);
  const claim = seedStore.claimPrimaryConversationMessage({ ...input, fence: seedFence });
  seedStore.bindPrimaryConversationThread({
    projectId: project.id,
    threadId: "thread-marker-terminal",
    previousThreadId: null,
    reason: "initial",
    fence: seedFence,
  });
  seedStore.releasePrimaryConversationLease(seedFence);
  seedStore.close();
  const marker = `[LATTICE_CONTROL_MESSAGE id=${input.clientMessageId} digest=${claim.event.payload.promptDigest}]`;
  const turn = {
    id: "turn-marker-terminal",
    status: "completed",
    items: [
      {
        type: "userMessage",
        content: [{ type: "text", text: `${marker}\n${input.text}` }],
      },
      {
        id: "terminal-marker-final",
        type: "agentMessage",
        phase: "final_answer",
        text: "已找回的真實最終回覆。",
      },
    ],
  };
  const thread = {
    id: "thread-marker-terminal",
    cwd: path.resolve(directory),
    createdAt: Math.floor(Date.parse(claim.event.created_at) / 1_000),
    turns: [turn],
  };
  const codex = new FakeCodex();
  codex.readResults.set(thread.id, thread);
  codex.resumeResult = thread;
  const store = new LatticeStore(databasePath);
  const service = new LatticeControlService({ store, codex });
  try {
    const recovered = await service.sendPrimaryConversationMessage(input);
    assert.equal(recovered.status, "codex_done");
    assert.equal(recovered.messages.at(-1).text, "已找回的真實最終回覆。");
    assert.equal(codex.turnStarts.length, 0);
    assert.equal(store.listEvents("primary").some(
      ({ kind, payload }) => kind === "codex_started"
        && payload.confirmedBy === "marker-thread/read",
    ), true);
  } finally {
    service.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a completed primary turn without a final reply stays failed and reconnectable", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Missing final", rootPath: process.cwd() });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "missing-final-message-001",
      text: "沒有 final reply 不得宣稱完成。",
    });
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: sent.codex_thread_id,
        turn: { id: sent.codex_turn_id, status: "completed", items: [] },
      },
    });
    const failed = service.primaryConversation();
    assert.equal(failed.status, "failed");
    assert.equal(failed.can_send, false);
    assert.equal(failed.can_reconnect, true);
    assert.doesNotMatch(failed.status_text, /已收到 Codex 回覆/u);
    assert.equal(store.listEvents("primary").some(
      ({ kind }) => kind === "conversation_terminal_missing_reply",
    ), true);
  } finally {
    service.close();
    store.close();
  }
});

test("invalid oversized final output becomes a recorded failure instead of escaping the listener", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Bounded final", rootPath: process.cwd() });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "oversized-final-message-001",
      text: "超限輸出要留下錯誤。",
    });
    assert.doesNotThrow(() => codex.emit("notification", {
      method: "item/completed",
      params: {
        threadId: sent.codex_thread_id,
        turnId: sent.codex_turn_id,
        item: {
          id: "oversized-final",
          type: "agentMessage",
          phase: "final_answer",
          text: "X".repeat(262_145),
        },
      },
    }));
    assert.equal(service.primaryConversation().status, "failed");
    assert.equal(store.listEvents("primary").some(
      ({ kind }) => kind === "conversation_notification_failed",
    ), true);
  } finally {
    service.close();
    store.close();
  }
});

test("an unexpected approval request in the read-only conversation is declined without stranding UI", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Approval boundary", rootPath: process.cwd() });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "approval-boundary-message-001",
      text: "主對話只允許唯讀回覆。",
    });
    codex.emit("serverRequest", {
      id: 801,
      method: "item/fileChange/requestApproval",
      params: {
        threadId: sent.codex_thread_id,
        turnId: sent.codex_turn_id,
        reason: "write request must be declined",
      },
    });
    assert.deepEqual(codex.responses.at(-1), { id: 801, result: { decision: "decline" } });
    assert.equal(service.primaryConversation().status, "running");
    assert.equal(store.listEvents("primary").some(
      ({ kind }) => kind === "conversation_approval_declined",
    ), true);
  } finally {
    service.close();
    store.close();
  }
});

test("a primary server response detects writer takeover and closes without a stale event", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-fenced-response-"));
  const databasePath = path.join(directory, "control.db");
  const firstStore = new LatticeStore(databasePath);
  const secondStore = new LatticeStore(databasePath);
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store: firstStore, codex });
  let takeoverFence;
  try {
    const project = service.createProject({ name: "Response fence", rootPath: directory });
    const sent = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "response-fence-message-001",
      text: "approval 回應也必須受 writer generation 保護。",
    });
    const lease = firstStore.database.prepare(`
      SELECT owner_id, generation FROM conversation_writer_leases WHERE conversation_id = 'primary'
    `).get();
    const firstFence = { ownerId: lease.owner_id, generation: lease.generation };
    codex.beforeRespond = () => {
      assert.equal(firstStore.releasePrimaryConversationLease(firstFence), true);
      const takeover = secondStore.acquirePrimaryConversationLease({
        ownerId: "response-fence-takeover",
        ownerPid: process.pid,
        ttlMs: 15_000,
      });
      takeoverFence = { ownerId: takeover.owner_id, generation: takeover.generation };
      codex.beforeRespond = null;
    };

    codex.emit("serverRequest", {
      id: 802,
      method: "item/fileChange/requestApproval",
      params: {
        threadId: sent.codex_thread_id,
        turnId: sent.codex_turn_id,
        reason: "writer changes during response",
      },
    });
    await new Promise((resolve) => setImmediate(resolve));

    assert.deepEqual(codex.responses.at(-1), { id: 802, result: { decision: "decline" } });
    assert.equal(codex.closeCalls, 1, "the stale adapter must close after the post-effect fence fails");
    assert.equal(secondStore.listEvents("primary").filter(
      ({ kind }) => kind === "conversation_approval_declined",
    ).length, 0);
    assert.equal(takeoverFence.generation, firstFence.generation + 1);
  } finally {
    if (takeoverFence) secondStore.releasePrimaryConversationLease(takeoverFence);
    service.close();
    firstStore.close();
    secondStore.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a primary disconnect race contains writer takeover and leaves zero stale mutation", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-fenced-disconnect-"));
  const databasePath = path.join(directory, "control.db");
  const firstStore = new LatticeStore(databasePath);
  const secondStore = new LatticeStore(databasePath);
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store: firstStore, codex });
  let takeoverFence;
  try {
    const project = service.createProject({ name: "Disconnect fence", rootPath: directory });
    await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "disconnect-fence-message-001",
      text: "disconnect callback 也不能跨 generation 寫入。",
    });
    const lease = firstStore.database.prepare(`
      SELECT owner_id, generation FROM conversation_writer_leases WHERE conversation_id = 'primary'
    `).get();
    const firstFence = { ownerId: lease.owner_id, generation: lease.generation };
    const updateWorkItem = firstStore.updateWorkItem.bind(firstStore);
    let transferOnUpdate = true;
    firstStore.updateWorkItem = (...args) => {
      if (transferOnUpdate && args[0] === "primary") {
        transferOnUpdate = false;
        assert.equal(firstStore.releasePrimaryConversationLease(firstFence), true);
        const takeover = secondStore.acquirePrimaryConversationLease({
          ownerId: "disconnect-fence-takeover",
          ownerPid: process.pid,
          ttlMs: 15_000,
        });
        takeoverFence = { ownerId: takeover.owner_id, generation: takeover.generation };
      }
      return updateWorkItem(...args);
    };

    assert.doesNotThrow(() => codex.emit("disconnect", { code: 17, signal: null }));
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(secondStore.getWorkItem("primary").status, "running");
    assert.equal(secondStore.listEvents("primary").filter(
      ({ kind }) => kind === "codex_disconnected",
    ).length, 0);
    assert.equal(codex.closeCalls, 1);
  } finally {
    if (takeoverFence) secondStore.releasePrimaryConversationLease(takeoverFence);
    service.close();
    firstStore.close();
    secondStore.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("the reserved primary ID fails closed when an older generic work item already uses it", async () => {
  const store = new LatticeStore();
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Identity collision", rootPath: process.cwd() });
    const timestamp = new Date().toISOString();
    store.database.prepare(`
      INSERT INTO work_items (
        id, project_id, title, objective, priority, status, created_at, updated_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
    `).run("primary", project.id, "Legacy", "Not a conversation", "normal", "draft", timestamp, timestamp);
    store.database.prepare(`
      INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
      VALUES (?, ?, ?, ?)
    `).run("primary", "created", JSON.stringify({ priority: "normal" }), timestamp);
    await assert.rejects(
      service.sendPrimaryConversationMessage({
        projectId: project.id,
        clientMessageId: "identity-collision-message",
        text: "不得接管保留 ID。",
      }),
      { code: "CONVERSATION_IDENTITY_COLLISION" },
    );
    assert.equal(codex.threadStarts.length, 0);
  } finally {
    service.close();
    store.close();
  }
});

test("the SQLite writer lease prevents two Control instances from owning one conversation", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-conversation-dedupe-"));
  const databasePath = path.join(directory, "control.db");
  const firstStore = new LatticeStore(databasePath);
  const secondStore = new LatticeStore(databasePath);
  const firstCodex = new FakeCodex();
  const secondCodex = new FakeCodex();
  const firstService = new LatticeControlService({ store: firstStore, codex: firstCodex });
  const secondService = new LatticeControlService({ store: secondStore, codex: secondCodex });
  try {
    const project = firstService.createProject({ name: "Atomic conversation", rootPath: directory });
    const input = {
      projectId: project.id,
      clientMessageId: "atomic-message-001",
      text: "跨程序也只能送出一次。",
    };
    const first = await firstService.sendPrimaryConversationMessage(input);
    await assert.rejects(
      secondService.sendPrimaryConversationMessage(input),
      { code: "CONVERSATION_WRITER_BUSY" },
    );
    assert.equal(first.id, "primary");
    assert.equal(firstCodex.turnStarts.length, 1);
    assert.equal(secondCodex.turnStarts.length, 0);
    assert.equal(firstStore.listEvents("primary")
      .filter(({ kind }) => kind === "conversation_message_claimed").length, 1);
    firstService.close();
    secondCodex.resumeResult = {
      id: first.codex_thread_id,
      turns: [{ id: first.codex_turn_id, status: "inProgress", items: [] }],
    };
    const resumed = await secondService.reconnectPrimaryConversation();
    assert.equal(resumed.status, "running");
    assert.equal(secondCodex.turnStarts.length, 0, "ownership transfer must not replay the message");
    await assert.rejects(
      secondService.sendPrimaryConversationMessage({ ...input, text: "不同內容不得重用 ID。" }),
      /already used for different content/u,
    );
  } finally {
    firstService.close();
    secondService.close();
    firstStore.close();
    secondStore.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("writer lease generations fence every stale primary conversation mutation", () => {
  const store = new LatticeStore();
  try {
    const project = store.createProject({ name: "Fence epochs", rootPath: process.cwd() });
    store.ensurePrimaryConversation(project.id);
    const firstLease = store.acquirePrimaryConversationLease({
      ownerId: "fence-owner-a",
      ownerPid: process.pid,
      ttlMs: 15_000,
    });
    const firstFence = { ownerId: firstLease.owner_id, generation: firstLease.generation };
    const sameOwner = store.acquirePrimaryConversationLease({
      ownerId: "fence-owner-a",
      ownerPid: process.pid,
      ttlMs: 15_000,
    });
    assert.equal(sameOwner.generation, firstFence.generation);
    assert.equal(store.renewPrimaryConversationLease({ ...firstFence, ttlMs: 15_000 }).generation,
      firstFence.generation);
    assert.equal(store.releasePrimaryConversationLease(firstFence), true);

    const secondLease = store.acquirePrimaryConversationLease({
      ownerId: "fence-owner-b",
      ownerPid: process.pid,
      ttlMs: 15_000,
    });
    const secondFence = { ownerId: secondLease.owner_id, generation: secondLease.generation };
    assert.equal(secondFence.generation, firstFence.generation + 1);
    const input = {
      projectId: project.id,
      clientMessageId: "fenced-message-001",
      text: "舊 epoch 不得寫入。",
    };
    assert.throws(
      () => store.claimPrimaryConversationMessage({ ...input, fence: firstFence }),
      { code: "CONVERSATION_WRITER_LOST" },
    );
    const claim = store.claimPrimaryConversationMessage({ ...input, fence: secondFence });
    assert.throws(
      () => store.bindPrimaryConversationThread({
        projectId: project.id,
        threadId: "thread-fenced",
        reason: "initial",
        fence: firstFence,
      }),
      { code: "CONVERSATION_WRITER_LOST" },
    );
    store.bindPrimaryConversationThread({
      projectId: project.id,
      threadId: "thread-fenced",
      reason: "initial",
      fence: secondFence,
    });
    store.recordPrimaryConversationDispatchIntent({
      clientMessageId: input.clientMessageId,
      threadId: "thread-fenced",
      promptDigest: claim.event.payload.promptDigest,
      fence: secondFence,
    });
    assert.equal(store.releasePrimaryConversationLease(secondFence), true);

    const thirdLease = store.acquirePrimaryConversationLease({
      ownerId: "fence-owner-c",
      ownerPid: process.pid,
      ttlMs: 15_000,
    });
    const thirdFence = { ownerId: thirdLease.owner_id, generation: thirdLease.generation };
    assert.equal(thirdFence.generation, secondFence.generation + 1);
    assert.throws(
      () => store.acceptPrimaryConversationTurn({
        clientMessageId: input.clientMessageId,
        threadId: "thread-fenced",
        turnId: "turn-fenced",
        fence: secondFence,
      }),
      { code: "CONVERSATION_WRITER_LOST" },
    );
    store.acceptPrimaryConversationTurn({
      clientMessageId: input.clientMessageId,
      threadId: "thread-fenced",
      turnId: "turn-fenced",
      fence: thirdFence,
    });
    assert.throws(
      () => store.recordPrimaryConversationReply({
        threadId: "thread-fenced",
        turnId: "turn-fenced",
        messageId: "reply-fenced",
        text: "過期 writer 不得保存回覆。",
        fence: secondFence,
      }),
      { code: "CONVERSATION_WRITER_LOST" },
    );
    assert.throws(
      () => store.updateWorkItem("primary", { progress: "stale" }, secondFence),
      { code: "CONVERSATION_WRITER_LOST" },
    );
    assert.throws(
      () => store.appendEvent("primary", "stale_event", {}, secondFence),
      { code: "CONVERSATION_WRITER_LOST" },
    );
    assert.equal(store.recordPrimaryConversationReply({
      threadId: "thread-fenced",
      turnId: "turn-fenced",
      messageId: "reply-fenced",
      text: "新 writer 可以保存回覆。",
      fence: thirdFence,
    }), true);
    store.releasePrimaryConversationLease(thirdFence);
  } finally {
    store.close();
  }
});

test("a lost writer with a deferred turn dispatch cannot accept or cause a second turn", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-fenced-turn-"));
  const databasePath = path.join(directory, "control.db");
  const firstStore = new LatticeStore(databasePath);
  const firstCodex = new FakeCodex();
  let releaseTurn;
  const turnGate = new Promise((resolve) => { releaseTurn = resolve; });
  firstCodex.beforeTurnResult = () => turnGate;
  const firstService = new LatticeControlService({ store: firstStore, codex: firstCodex });
  const secondStore = new LatticeStore(databasePath);
  const secondCodex = new FakeCodex();
  const secondService = new LatticeControlService({ store: secondStore, codex: secondCodex });
  try {
    const project = firstService.createProject({ name: "Deferred fence", rootPath: directory });
    const input = {
      projectId: project.id,
      clientMessageId: "deferred-fence-message-001",
      text: "lease 轉移後不得再接受這個 turn。",
    };
    const turnDispatched = new Promise((resolve) => firstCodex.once("turnStartAccepted", resolve));
    const firstSend = firstService.sendPrimaryConversationMessage(input);
    await turnDispatched;
    const lease = firstStore.database.prepare(`
      SELECT owner_id, generation FROM conversation_writer_leases WHERE conversation_id = 'primary'
    `).get();
    const firstFence = { ownerId: lease.owner_id, generation: lease.generation };
    assert.equal(firstStore.releasePrimaryConversationLease(firstFence), true);
    secondCodex.readResults.set("thread-1", { id: "thread-1", turns: [] });

    await assert.rejects(
      secondService.sendPrimaryConversationMessage(input),
      { code: "CONVERSATION_RECONCILIATION_REQUIRED" },
    );
    assert.equal(secondCodex.turnStarts.length, 0, "takeover must not repeat a dispatched intent");
    releaseTurn();
    await assert.rejects(firstSend, { code: "CONVERSATION_WRITER_LOST" });
    const events = secondStore.listEvents("primary");
    assert.equal(events.filter(({ kind }) => kind === "conversation_turn_dispatch_intended").length, 1);
    assert.equal(events.filter(({ kind }) => kind === "conversation_message_accepted").length, 0);
    assert.equal(firstCodex.turnStarts.length, 1);
    assert.equal(secondCodex.turnStarts.length, 0);
  } finally {
    releaseTurn?.();
    firstService.close();
    secondService.close();
    firstStore.close();
    secondStore.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a proven pre-dispatch identity rejection retries the same claim exactly once", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-pre-dispatch-retry-"));
  const store = new LatticeStore(path.join(directory, "control.db"));
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Pre-dispatch retry", rootPath: directory });
    codex.beforeTurnDispatch = () => {
      codex.beforeTurnDispatch = null;
      const error = new Error("effect identity changed before dispatch");
      error.code = "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED";
      throw error;
    };
    await assert.rejects(
      service.sendPrimaryConversationMessage({
        projectId: project.id,
        clientMessageId: "pre-dispatch-retry-message-001",
        text: "只有可證明沒有寫入 provider 的錯誤可重試一次。",
      }),
      { code: "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED" },
    );
    assert.equal(codex.turnStarts.length, 0);
    const failed = service.primaryConversation();
    codex.readResults.set(failed.codex_thread_id, { id: failed.codex_thread_id, turns: [] });

    const reconnected = await service.reconnectPrimaryConversation();
    assert.equal(reconnected.status, "running");
    assert.equal(codex.turnStarts.length, 1);
    const events = store.listEvents("primary");
    assert.equal(events.filter(({ kind }) => kind === "conversation_turn_dispatch_intended").length, 1);
    assert.equal(events.filter(({ kind }) => kind === "conversation_turn_dispatch_not_sent").length, 1);
    assert.equal(events.filter(({ kind }) => kind === "conversation_turn_dispatch_retry_intended").length, 1);
    assert.equal(events.filter(({ kind }) => kind === "conversation_message_accepted").length, 1);

    codex.readResults.set(failed.codex_thread_id, { id: failed.codex_thread_id, turns: [] });
    assert.equal(events.find(({ kind }) => kind === "conversation_turn_dispatch_retry_intended")
      .payload.clientMessageId, "pre-dispatch-retry-message-001");
  } finally {
    service.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a later message retries one proven pre-dispatch rejection after exact terminal history", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-later-pre-dispatch-retry-"));
  const store = new LatticeStore(path.join(directory, "control.db"));
  const codex = new FakeCodex();
  const service = new LatticeControlService({ store, codex });
  try {
    const project = service.createProject({ name: "Later pre-dispatch retry", rootPath: directory });
    const first = await service.sendPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "later-retry-message-001",
      text: "先完成第一回合。",
    });
    codex.emit("notification", {
      method: "item/completed",
      params: {
        threadId: first.codex_thread_id,
        turnId: first.codex_turn_id,
        item: {
          id: "later-retry-reply-001",
          type: "agentMessage",
          phase: "final_answer",
          text: "第一回合完成。",
        },
      },
    });
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: first.codex_thread_id,
        turn: { id: first.codex_turn_id, status: "completed", items: [] },
      },
    });
    assert.equal(service.primaryConversation().status, "codex_done");

    codex.beforeTurnDispatch = () => {
      codex.beforeTurnDispatch = null;
      const error = new Error("effect identity changed before later dispatch");
      error.code = "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED";
      throw error;
    };
    await assert.rejects(
      service.sendPrimaryConversationMessage({
        projectId: project.id,
        clientMessageId: "later-retry-message-002",
        text: "第二回合在 provider 寫入前失敗。",
      }),
      { code: "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED" },
    );
    assert.equal(codex.turnStarts.length, 1, "the rejected second dispatch had zero provider effect");
    codex.readResults.set(first.codex_thread_id, { id: first.codex_thread_id, turns: [] });
    await assert.rejects(
      service.reconnectPrimaryConversation(),
      { code: "CONVERSATION_RECONCILIATION_REQUIRED" },
    );
    assert.equal(codex.turnStarts.length, 1, "an empty rollout must not erase exact terminal history");
    codex.readResults.set(first.codex_thread_id, {
      id: first.codex_thread_id,
      turns: [{ id: first.codex_turn_id, status: "completed", items: [] }],
    });
    codex.resumeResult = {
      id: first.codex_thread_id,
      turns: [{ id: first.codex_turn_id, status: "completed", items: [] }],
    };

    const reconnected = await service.reconnectPrimaryConversation();
    assert.equal(reconnected.status, "running");
    assert.equal(reconnected.codex_turn_id, "turn-2");
    assert.equal(codex.turnStarts.length, 2, "only the second message's one retry was dispatched");
    const secondStarts = codex.turnStarts.filter(({ text }) => text.includes("第二回合在 provider 寫入前失敗"));
    assert.equal(secondStarts.length, 1);
  } finally {
    service.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("the loopback conversation API serves one responsive chat entry and durable final replies", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-conversation-http-"));
  const codex = new FakeCodex();
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
  });
  try {
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const address = application.server.address();
    const origin = `http://127.0.0.1:${address.port}`;

    const pageHtml = await (await fetch(`${origin}/`)).text();
    assert.equal(pageHtml.match(/id="conversation-form"/gu)?.length, 1);
    assert.equal(pageHtml.match(/data-core-target=/gu)?.length, 4);
    assert.match(pageHtml, /id="core-conversation"/u);
    assert.match(pageHtml, /id="core-work-graph"/u);
    assert.match(pageHtml, /id="core-work-tree"/u);
    assert.match(pageHtml, /id="core-decisions"/u);
    assert.doesNotMatch(pageHtml, /id="work-form"|id="items"|id="project-form"/u);
    assert.doesNotMatch(pageHtml, /id="conversation-project"/u);
    assert.match(pageHtml, /@media\s*\(max-width:/u);
    assert.match(pageHtml, /localStorage/u);
    assert.match(pageHtml, /conversation\?\.can_send === true/u);
    assert.match(pageHtml, /pendingForCurrentContext=pending\?\.projectId===currentProjectId\(\)/u);
    assert.match(pageHtml, /!pendingForCurrentContext/u);
    assert.match(pageHtml, /typeof parsed\.projectId === "string"/u);
    assert.match(pageHtml, /typeof parsed\.text === "string"/u);
    assert.match(pageHtml, /safeMessageId\.test\(parsed\.clientMessageId\)/u);
    assert.match(pageHtml, /<textarea id="message"[^>]*rows="4"[^>]*enterkeyhint="send"/u);
    assert.match(pageHtml, /\.command-dock textarea \{[^}]*min-height:96px;[^}]*max-height:min\(34dvh,280px\);[^}]*resize:vertical;/u);
    assert.match(pageHtml, /event\.key!=="Enter"\|\|event\.shiftKey\|\|event\.isComposing\|\|event\.keyCode===229/u);
    assert.match(pageHtml, /nodes\.form\.requestSubmit\(\)/u);
    assert.match(pageHtml, /conversation\?\.can_send === true/u);
    assert.doesNotMatch(pageHtml, /conversation\?\.status==="not_started"\|\|/u);
    assert.doesNotMatch(pageHtml, /readyForFirstMessage/u);
    assert.match(pageHtml, /assertSharedWorkSnapshot/u);
    assert.match(pageHtml, /renderWorkGraph\(workSnapshot\.graph\)/u);
    assert.match(pageHtml, /renderWorkTree\(workSnapshot\.tree\)/u);
    assert.match(pageHtml, /if\(state\.pollPromise\)return state\.pollPromise/u);
    assert.doesNotMatch(pageHtml, /api\("\/api\/work-snapshot"/u);

    const projectResponse = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "Conversation API", rootPath: directory }),
    });
    assert.equal(projectResponse.status, 201);
    const project = await projectResponse.json();

    const send = () => fetch(`${origin}/api/conversation/messages`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        projectId: project.id,
        clientMessageId: "http-message-001",
        text: "請透過真實接頭回覆。",
      }),
    });
    const firstResponse = await send();
    assert.equal(firstResponse.status, 200);
    const first = await firstResponse.json();
    assert.equal(first.id, "primary");
    assert.equal(first.status, "running");
    assert.equal(codex.turnStarts.length, 1);
    const controlState = await (await fetch(`${origin}/api/state`)).json();
    assert.equal(controlState.workItems.length, 0, "the chat projection is not a task card");

    const duplicateResponse = await send();
    assert.equal(duplicateResponse.status, 200);
    assert.equal((await duplicateResponse.json()).id, first.id);
    assert.equal(codex.turnStarts.length, 1);

    codex.emit("notification", {
      method: "item/completed",
      params: {
        threadId: first.codex_thread_id,
        turnId: first.codex_turn_id,
        item: {
          id: "http-agent-message-001",
          type: "agentMessage",
          phase: "final_answer",
          text: "這是經由 App Server 事件保存的最終回覆。",
        },
      },
    });
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: first.codex_thread_id,
        turn: { id: first.codex_turn_id, status: "completed", items: [] },
      },
    });
    const replayResponse = await fetch(`${origin}/api/conversation`);
    assert.equal(replayResponse.status, 200);
    const replay = await replayResponse.json();
    assert.equal(replay.status, "codex_done");
    assert.equal(replay.messages.at(-1).text, "這是經由 App Server 事件保存的最終回覆。");

    const legacyRead = await fetch(`${origin}/api/work-items/primary`);
    assert.equal(legacyRead.status, 409);
    assert.equal((await legacyRead.json()).code, "PRIMARY_CONVERSATION_ROUTE_REQUIRED");
    const legacyVerify = await fetch(`${origin}/api/work-items/primary/verify`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ notes: "must not mutate the conversation" }),
    });
    assert.equal(legacyVerify.status, 409);
    assert.equal((await legacyVerify.json()).code, "PRIMARY_CONVERSATION_ROUTE_REQUIRED");
    assert.equal((await (await fetch(`${origin}/api/conversation`)).json()).status, "codex_done");
  } finally {
    await new Promise((resolve) => application.server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});

test("Control can prewarm Codex without choosing a project or starting a thread", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-codex-prewarm-"));
  const codex = new FakeCodex();
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
    prewarmCodex: true,
  });
  try {
    const result = await application.codexPrewarm;
    assert.equal(result.ready, true);
    assert.equal(codex.readinessCalls, 1);
    assert.equal(codex.threadStarts.length, 0);
    assert.equal(application.service.primaryConversation().can_send, false);
    assert.equal(application.service.primaryConversation().codex_connected, true);
  } finally {
    application.service.close();
    await application.codex.close();
    application.store.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a failed Codex prewarm leaves Control alive and starts no provider work", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-codex-prewarm-failure-"));
  const codex = new FakeCodex();
  codex.readAuthReadiness = async () => {
    throw new Error("prewarm unavailable");
  };
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
    prewarmCodex: true,
  });
  try {
    const result = await application.codexPrewarm;
    assert.equal(result.ready, false);
    assert.match(result.error.message, /prewarm unavailable/u);
    assert.equal(codex.threadStarts.length, 0);
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    const response = await fetch(`http://127.0.0.1:${port}/api/conversation`);
    assert.equal(response.status, 200);
    assert.equal((await response.json()).codex_connected, false);
  } finally {
    if (application.server.listening) {
      await new Promise((resolve) => application.server.close(resolve));
    }
    await rm(directory, { recursive: true, force: true });
  }
});

test("new work selects a proven project, readies Codex, and enables the primary conversation", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-new-work-http-"));
  const codex = new FakeCodex();
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
  });
  try {
    const firstProject = application.service.createProject({
      name: "First project",
      rootPath: directory,
    });
    const selectedProject = application.service.createProject({
      name: "Selected project",
      rootPath: directory,
    });
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    const origin = `http://127.0.0.1:${port}`;

    const ambiguous = await (await fetch(`${origin}/api/four-core`)).json();
    assert.equal(ambiguous.context.reason, "ambiguous_project_context");
    assert.equal(ambiguous.conversation.can_send, false);

    const pageHtml = await (await fetch(`${origin}/`)).text();
    assert.match(pageHtml, /id="new-work-dialog"/u);
    assert.match(pageHtml, /api\("\/api\/conversation",\{method:"POST"/u);

    const response = await fetch(`${origin}/api/conversation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ projectId: selectedProject.id }),
    });
    assert.equal(response.status, 200);
    const conversation = await response.json();
    assert.equal(conversation.id, "primary");
    assert.equal(conversation.project_id, selectedProject.id);
    assert.equal(conversation.status, "draft");
    assert.equal(conversation.codex_connected, true);
    assert.equal(conversation.can_send, true);
    assert.equal(codex.threadStarts.length, 0, "choosing a project must not send a message");

    const selectedSurface = await (await fetch(`${origin}/api/four-core`)).json();
    assert.equal(selectedSurface.context.status, "ready");
    assert.equal(selectedSurface.context.source, "primary_conversation");
    assert.equal(selectedSurface.context.project_id, selectedProject.id);
    assert.notEqual(selectedSurface.context.project_id, firstProject.id);

    const unknownResponse = await fetch(`${origin}/api/conversation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ projectId: "missing-project" }),
    });
    assert.equal(unknownResponse.status, 400);
    assert.equal(
      (await (await fetch(`${origin}/api/conversation`)).json()).project_id,
      selectedProject.id,
    );

    const sendResponse = await fetch(`${origin}/api/conversation/messages`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        projectId: selectedProject.id,
        clientMessageId: "new-work-message-001",
        text: "開始這個新工作。",
      }),
    });
    assert.equal(sendResponse.status, 200);
    assert.equal((await sendResponse.json()).status, "running");

    const busySwitch = await fetch(`${origin}/api/conversation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ projectId: firstProject.id }),
    });
    assert.equal(busySwitch.status, 409);
    assert.equal((await busySwitch.json()).code, "CONVERSATION_BUSY");
    assert.equal(
      (await (await fetch(`${origin}/api/conversation`)).json()).project_id,
      selectedProject.id,
    );
  } finally {
    await new Promise((resolve) => application.server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});

test("concurrent first new-work starts perform readiness only under the SQLite writer lease", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-new-work-first-lease-"));
  const databasePath = path.join(directory, "control.db");
  const firstStore = new LatticeStore(databasePath);
  const secondStore = new LatticeStore(databasePath);
  const firstCodex = new FakeCodex();
  const secondCodex = new FakeCodex();
  let firstReadinessCalls = 0;
  let secondReadinessCalls = 0;
  const firstReadiness = firstCodex.readAuthReadiness.bind(firstCodex);
  const secondReadiness = secondCodex.readAuthReadiness.bind(secondCodex);
  firstCodex.readAuthReadiness = async () => {
    firstReadinessCalls += 1;
    await new Promise((resolve) => setTimeout(resolve, 20));
    return firstReadiness();
  };
  secondCodex.readAuthReadiness = async () => {
    secondReadinessCalls += 1;
    await new Promise((resolve) => setTimeout(resolve, 20));
    return secondReadiness();
  };
  const firstService = new LatticeControlService({ store: firstStore, codex: firstCodex });
  const secondService = new LatticeControlService({ store: secondStore, codex: secondCodex });
  try {
    const firstProject = firstService.createProject({ name: "First project", rootPath: directory });
    const secondProject = firstService.createProject({ name: "Second project", rootPath: directory });
    const results = await Promise.allSettled([
      firstService.startPrimaryConversation({ projectId: firstProject.id }),
      secondService.startPrimaryConversation({ projectId: secondProject.id }),
    ]);
    assert.equal(results.filter(({ status }) => status === "fulfilled").length, 1);
    assert.equal(results.filter(({ status }) => status === "rejected").length, 1);
    assert.equal(results.find(({ status }) => status === "rejected").reason.code,
      "CONVERSATION_WRITER_BUSY");
    assert.equal(firstReadinessCalls + secondReadinessCalls, 1);
    const winnerIsFirst = results[0].status === "fulfilled";
    assert.equal(
      (winnerIsFirst ? firstService : secondService).primaryConversation().can_send,
      true,
    );
    assert.equal(
      (winnerIsFirst ? secondService : firstService).primaryConversation().can_send,
      false,
    );
  } finally {
    firstService.close();
    secondService.close();
    firstStore.close();
    secondStore.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("new work keeps ambiguous context disabled when Codex readiness fails", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-new-work-auth-failure-"));
  const codex = new FakeCodex();
  codex.readAuthReadiness = async () => ({
    ready: false,
    authMode: null,
    appServerGeneration: null,
    appServerSessionId: null,
  });
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
  });
  try {
    application.service.createProject({ name: "First project", rootPath: directory });
    const selectedProject = application.service.createProject({
      name: "Selected project",
      rootPath: directory,
    });
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    const origin = `http://127.0.0.1:${port}`;

    const response = await fetch(`${origin}/api/conversation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ projectId: selectedProject.id }),
    });
    assert.equal(response.status, 503);
    assert.equal((await response.json()).code, "CONVERSATION_CODEX_AUTH_REQUIRED");

    const conversation = await (await fetch(`${origin}/api/conversation`)).json();
    assert.equal(conversation.project_id, null);
    assert.equal(conversation.can_send, false);
    const surface = await (await fetch(`${origin}/api/four-core`)).json();
    assert.equal(surface.context.status, "not_ready");
    assert.equal(surface.context.reason, "ambiguous_project_context");
  } finally {
    await new Promise((resolve) => application.server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});

test("new work cannot switch away from an accepted turn without a verified terminal reply", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-new-work-unresolved-turn-"));
  const codex = new FakeCodex();
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
  });
  try {
    const firstProject = application.service.createProject({
      name: "First project",
      rootPath: directory,
    });
    const secondProject = application.service.createProject({
      name: "Second project",
      rootPath: directory,
    });
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    const origin = `http://127.0.0.1:${port}`;

    const selected = await fetch(`${origin}/api/conversation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ projectId: firstProject.id }),
    });
    assert.equal(selected.status, 200);
    const sent = await fetch(`${origin}/api/conversation/messages`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        projectId: firstProject.id,
        clientMessageId: "new-work-unresolved-message-001",
        text: "這個 turn 尚未留下可驗證的結尾。",
      }),
    });
    assert.equal(sent.status, 200);
    const running = await sent.json();
    codex.activeTurns.delete(running.codex_thread_id);
    codex.emit("disconnect", { code: 1, signal: null });

    const switchResponse = await fetch(`${origin}/api/conversation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ projectId: secondProject.id }),
    });
    assert.equal(switchResponse.status, 409);
    assert.equal((await switchResponse.json()).code, "CONVERSATION_RECONCILIATION_REQUIRED");
    const after = await (await fetch(`${origin}/api/conversation`)).json();
    assert.equal(after.project_id, firstProject.id);
    assert.equal(after.can_send, false);
    assert.equal(after.can_reconnect, true);
  } finally {
    await new Promise((resolve) => application.server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});

test("new work cannot switch after a completed turn whose final reply is missing", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-new-work-missing-final-"));
  const codex = new FakeCodex();
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
  });
  try {
    const firstProject = application.service.createProject({
      name: "First project",
      rootPath: directory,
    });
    const secondProject = application.service.createProject({
      name: "Second project",
      rootPath: directory,
    });
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    const origin = `http://127.0.0.1:${port}`;

    assert.equal((await fetch(`${origin}/api/conversation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ projectId: firstProject.id }),
    })).status, 200);
    const sent = await (await fetch(`${origin}/api/conversation/messages`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        projectId: firstProject.id,
        clientMessageId: "new-work-missing-final-message-001",
        text: "完成事件不能取代最終回覆。",
      }),
    })).json();
    codex.emit("notification", {
      method: "turn/completed",
      params: {
        threadId: sent.codex_thread_id,
        turn: { id: sent.codex_turn_id, status: "completed", items: [] },
      },
    });

    const switchResponse = await fetch(`${origin}/api/conversation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ projectId: secondProject.id }),
    });
    assert.equal(switchResponse.status, 409);
    assert.equal((await switchResponse.json()).code, "CONVERSATION_RECONCILIATION_REQUIRED");
    const after = await (await fetch(`${origin}/api/conversation`)).json();
    assert.equal(after.project_id, firstProject.id);
    assert.equal(after.can_send, false);
    assert.equal(after.can_reconnect, true);
  } finally {
    await new Promise((resolve) => application.server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});

test("schema v6 upgrades transactionally to v7 conversation read indexes", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-control-v6-v7-"));
  const databasePath = path.join(directory, "control.db");
  let store = new LatticeStore(databasePath);
  try {
    const project = store.createProject({ name: "Migration proof", rootPath: directory });
    const preservedWork = store.createWorkItem({
      projectId: project.id,
      title: "Preserved work",
      objective: "Prove additive access paths preserve Control truth.",
    });
    const oversizedLegacyDiagnostic = {
      threadId: "legacy-thread",
      name: "legacy-mcp",
      status: "failed",
      error: "X".repeat(20_000),
      failureReason: null,
    };
    store.close();
    store = null;

    const v6 = new DatabaseSync(databasePath);
    try {
      for (const indexName of [
        "work_events_work_item_kind_id",
        "work_events_client_message_lookup",
        "work_events_thread_turn_lookup",
        "work_events_message_lookup",
        "work_events_idempotent_payload_lookup",
      ]) v6.exec(`DROP INDEX ${indexName};`);
      v6.prepare(`
        INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
        VALUES (?, 'mcp_server_startup_status_updated', ?, ?)
      `).run(
        preservedWork.id,
        JSON.stringify(oversizedLegacyDiagnostic),
        new Date().toISOString(),
      );
      v6.exec("PRAGMA user_version = 6;");
    } finally {
      v6.close();
    }

    store = new LatticeStore(databasePath);
    assert.equal(store.database.prepare("PRAGMA user_version").get().user_version, 7);
    assert.equal(store.listWorkItems().length, 1);
    const indexes = new Set(store.database.prepare(`
      SELECT name FROM sqlite_master
      WHERE type = 'index' AND name LIKE 'work_events_%'
    `).all().map(({ name }) => name));
    for (const indexName of [
      "work_events_work_item_kind_id",
      "work_events_client_message_lookup",
      "work_events_thread_turn_lookup",
      "work_events_message_lookup",
      "work_events_idempotent_payload_lookup",
    ]) assert.ok(indexes.has(indexName));
    const idempotentIndexSql = store.database.prepare(`
      SELECT sql FROM sqlite_master WHERE name = 'work_events_idempotent_payload_lookup'
    `).get().sql;
    assert.match(idempotentIndexSql, /length\(CAST\(payload_json AS BLOB\)\) <= 16384/u);
    assert.equal(store.hasEventPayload(
      preservedWork.id,
      "mcp_server_startup_status_updated",
      oversizedLegacyDiagnostic,
    ), false);
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("the four-core product API resolves one proven context and shares work projection identity", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-four-core-http-"));
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex: new FakeCodex(),
  });
  try {
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();
    const origin = `http://127.0.0.1:${port}`;

    const originalFourCoreSurface = application.service.fourCoreSurface
      .bind(application.service);
    application.service.fourCoreSurface = () => {
      const error = new Error("x".repeat(5_000));
      error.code = "Y".repeat(5_000);
      throw error;
    };
    const boundedErrorResponse = await fetch(`${origin}/api/four-core`);
    assert.equal(boundedErrorResponse.status, 400);
    const boundedError = await boundedErrorResponse.json();
    assert.ok(boundedError.error.length <= 2_048);
    assert.match(boundedError.error, /\[truncated\]$/u);
    assert.equal(boundedError.code, "CONTROL_REQUEST_FAILED");
    application.service.fourCoreSurface = originalFourCoreSurface;

    const empty = await (await fetch(`${origin}/api/four-core`)).json();
    assert.equal(empty.context.status, "not_ready");
    assert.equal(empty.context.reason, "no_project_context");
    assert.equal(empty.work_snapshot, null);
    assert.equal(empty.decisions, null);

    const project = application.service.createProject({ name: "Four core", rootPath: directory });
    const unique = await (await fetch(`${origin}/api/four-core`)).json();
    assert.equal(unique.context.status, "ready");
    assert.equal(unique.context.source, "unique_control_project");
    assert.equal(unique.context.project_id, project.id);
    assert.equal("root_path" in unique.context, false);
    assert.equal(unique.conversation.can_send, false, "a unique project is not sendable before App Server readiness is proven");

    application.service.createProject({ name: "Ambiguous", rootPath: directory });
    const ambiguous = await (await fetch(`${origin}/api/four-core`)).json();
    assert.equal(ambiguous.context.status, "not_ready");
    assert.equal(ambiguous.context.reason, "ambiguous_project_context");
    assert.equal(ambiguous.context.project_id, null);
    assert.equal(ambiguous.work_snapshot, null);

    application.store.ensurePrimaryConversation(project.id);
    const goal = application.service.createWorkItem({
      projectId: project.id,
      title: "完成四核心介面",
      objective: "把三項既有能力接到同一產品畫面。",
      priority: "high",
    });
    const foundation = application.service.createWorkItem({
      projectId: project.id,
      title: "既有資料核心",
      objective: "沿用同一 Control store。",
      priority: "normal",
    });
    const interfaceWork = application.service.createWorkItem({
      projectId: project.id,
      title: "四核心 UI",
      objective: "呈現對話、圖譜、樹與決策。",
      priority: "urgent",
    });
    application.store.updateWorkItem(goal.id, { status: "running", progress: "正在整合 UI" });
    let snapshot = application.store.getWorkSnapshot({ projectId: project.id });
    snapshot = application.store.setWorkRelations({
      projectId: project.id,
      workItemId: foundation.id,
      parentId: goal.id,
      expectedRevision: snapshot.revision,
      expectedDigest: snapshot.digest,
    }).snapshot;
    snapshot = application.store.setWorkRelations({
      projectId: project.id,
      workItemId: interfaceWork.id,
      parentId: goal.id,
      dependsOn: [foundation.id],
      blocker: { status: "blocked", reason: "等待真實瀏覽器驗收" },
      expectedRevision: snapshot.revision,
      expectedDigest: snapshot.digest,
    }).snapshot;

    let decisionState = application.store.getCurrentDecisionsPacket({
      scope: project.id,
      limit: 32,
    });
    const firstDecision = application.store.recordDecision({
      scope: project.id,
      subject: "product.navigation",
      content: "使用左側四核心導覽。",
      rationale: "初始桌面方向。",
      source: { kind: "approved_document", reference: "document:four-core-acceptance#initial" },
      clientRequestId: "four-core-decision-001",
      expectedRevision: decisionState.revision,
      expectedDigest: decisionState.digest,
    });
    decisionState = application.store.getCurrentDecisionsPacket({ scope: project.id, limit: 32 });
    application.store.recordDecision({
      scope: project.id,
      subject: "product.navigation",
      content: "桌面使用四分頁，手機使用底部四分頁。",
      rationale: "同一資訊架構適應兩種比例。",
      source: { kind: "approved_document", reference: "document:four-core-acceptance#replacement" },
      supersedesDecisionId: firstDecision.decision.id,
      clientRequestId: "four-core-decision-002",
      expectedRevision: decisionState.revision,
      expectedDigest: decisionState.digest,
    });

    const response = await fetch(`${origin}/api/four-core`);
    assert.equal(response.status, 200);
    const surface = await response.json();
    assert.equal(surface.context.status, "ready");
    assert.equal(surface.context.source, "primary_conversation");
    assert.equal(surface.context.project_id, project.id);
    assert.equal(surface.work_snapshot.revision, surface.work_snapshot.tree.revision);
    assert.equal(surface.work_snapshot.revision, surface.work_snapshot.graph.revision);
    assert.equal(surface.work_snapshot.digest, surface.work_snapshot.tree.digest);
    assert.equal(surface.work_snapshot.digest, surface.work_snapshot.graph.digest);
    assert.deepEqual(
      surface.work_snapshot.graph.nodes.find(({ id }) => id === foundation.id).reverse_dependents,
      [interfaceWork.id],
    );
    assert.equal(
      surface.work_snapshot.tree.nodes.find(({ id }) => id === interfaceWork.id).blocker.reasons[0].reason,
      "等待真實瀏覽器驗收",
    );
    assert.equal(surface.decisions.decisions.length, 1);
    assert.equal(surface.decisions.decisions[0].supersedes_decision_id, firstDecision.decision.id);
    assert.equal("rationale" in surface.decisions.decisions[0], false);
    assert.equal(surface.conversation.messages_truncated, false);
    assert.equal(surface.conversation.handoffs_truncated, false);
    assert.equal(
      surface.conversation.can_send,
      false,
      "direct store setup must not claim Codex readiness before the new-work handshake",
    );
    let currentWorkSnapshot = surface.work_snapshot;

    const queryPlan = (sql, ...args) => application.store.database
      .prepare(`EXPLAIN QUERY PLAN ${sql}`).all(...args)
      .map(({ detail }) => detail).join("\n");
    assert.match(queryPlan(
      "SELECT id FROM work_events WHERE work_item_id = ? AND kind = ? ORDER BY id DESC LIMIT 1",
      "primary",
      "conversation_message_claimed",
    ), /work_events_work_item_kind_id/u);
    assert.match(queryPlan(
      "SELECT id FROM work_events WHERE work_item_id = ? AND kind = ? "
        + "AND json_extract(payload_json, '$.clientMessageId') = ? ORDER BY id DESC LIMIT 1",
      "primary",
      "conversation_message_claimed",
      "indexed-message",
    ), /work_events_client_message_lookup/u);
    assert.match(queryPlan(
      "SELECT id FROM work_events WHERE work_item_id = ? AND kind = ? "
        + "AND json_extract(payload_json, '$.threadId') = ? "
        + "AND json_extract(payload_json, '$.turnId') = ? ORDER BY id DESC LIMIT 1",
      "primary",
      "turn_completed",
      "indexed-thread",
      "indexed-turn",
    ), /work_events_thread_turn_lookup/u);
    assert.match(queryPlan(
      "SELECT id FROM work_events WHERE work_item_id = ? AND kind = ? "
        + "AND json_extract(payload_json, '$.messageId') = ? ORDER BY id DESC LIMIT 1",
      "primary",
      "conversation_assistant_message",
      "indexed-reply",
    ), /work_events_message_lookup/u);
    const idempotentIndexSql = application.store.database.prepare(`
      SELECT sql FROM sqlite_master WHERE name = 'work_events_idempotent_payload_lookup'
    `).get().sql;
    const idempotentPredicate = idempotentIndexSql.match(/\bWHERE\s+([\s\S]+)$/u)[1];
    assert.match(queryPlan(
      `SELECT id FROM work_events WHERE work_item_id = ? AND kind = ? AND payload_json = ?
        AND (${idempotentPredicate}) LIMIT 1`,
      "primary",
      "turn_completed",
      "{}",
    ), /work_events_idempotent_payload_lookup/u);
    const boundedMultiKindPlan = queryPlan(`
      SELECT id FROM (
        SELECT id FROM (
          SELECT id FROM work_events WHERE work_item_id = ? AND kind = ?
          ORDER BY id DESC LIMIT ?
        )
        UNION ALL
        SELECT id FROM (
          SELECT id FROM work_events WHERE work_item_id = ? AND kind = ?
          ORDER BY id DESC LIMIT ?
        )
      ) ORDER BY id DESC LIMIT ?
    `, "primary", "conversation_message_claimed", 65,
    "primary", "conversation_assistant_message", 65, 65);
    assert.equal((boundedMultiKindPlan.match(/work_events_work_item_kind_id/gu) ?? []).length, 2);

    const originalListEvents = application.store.listEvents.bind(application.store);
    const originalListProjects = application.store.listProjects.bind(application.store);
    const originalCurrentDecisions = application.store.getCurrentDecisionsPacket
      .bind(application.store);
    const originalConversationWindow = application.store.primaryConversationWindow
      .bind(application.store);
    const observedConversationWindows = [];
    let currentDecisionReads = 0;
    application.store.listEvents = () => {
      throw new Error("four-core polling must not materialize the complete event history");
    };
    application.store.listProjects = () => {
      throw new Error("four-core polling must not materialize the complete project catalog");
    };
    application.store.getCurrentDecisionsPacket = (options) => {
      currentDecisionReads += 1;
      return originalCurrentDecisions(options);
    };
    application.store.primaryConversationWindow = (options) => {
      observedConversationWindows.push(options);
      return originalConversationWindow(options);
    };
    application.service.primaryConversationCache = null;
    application.service.fourCoreDecisionCache = null;
    try {
      const recentSurface = application.service.fourCoreSurface();
      assert.equal(recentSurface.conversation.history_truncated, false);
      const unchangedSurface = application.service.fourCoreSurface();
      assert.equal(unchangedSurface.decisions.digest, recentSurface.decisions.digest);
      assert.equal(observedConversationWindows.length, 1);
      assert.equal(currentDecisionReads, 1);
      application.store.updateWorkItem(foundation.id, { progress: "unrelated work mutation" });
      const workChangedSurface = application.service.fourCoreSurface();
      currentWorkSnapshot = workChangedSurface.work_snapshot;
      assert.equal(workChangedSurface.decisions.digest, recentSurface.decisions.digest);
      assert.equal(currentDecisionReads, 1);
      application.service.fourCoreWorkNode({
        workItemId: foundation.id,
        expectedRevision: currentWorkSnapshot.revision,
        expectedDigest: currentWorkSnapshot.digest,
      });
      application.service.fourCoreDecisionHistory({
        decisionId: surface.decisions.decisions[0].id,
        expectedRevision: surface.decisions.revision,
        expectedDigest: surface.decisions.digest,
      });
      assert.equal(observedConversationWindows.length, 2);
      assert.equal(currentDecisionReads, 1);
      assert.ok(observedConversationWindows.every((options) => (
        options.maximumMessages === 64
        && options.maximumMessageBytes === 524_288
        && options.maximumHandoffs === 32
        && options.maximumHandoffBytes === 65_536
      )));
    } finally {
      application.store.listEvents = originalListEvents;
      application.store.listProjects = originalListProjects;
      application.store.getCurrentDecisionsPacket = originalCurrentDecisions;
      application.store.primaryConversationWindow = originalConversationWindow;
    }

    const currentDecisionPlan = application.store.database.prepare(`
      EXPLAIN QUERY PLAN
      SELECT id FROM decisions
      WHERE status = 'current' AND scope = ?
      ORDER BY subject ASC, id ASC LIMIT ?
    `).all(project.id, 33).map(({ detail }) => detail).join("\n");
    assert.match(currentDecisionPlan, /decisions_current_scope_subject/u);

    const insertEvent = application.store.database.prepare(`
      INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
      VALUES ('primary', ?, ?, ?)
    `);
    application.store.database.exec("BEGIN IMMEDIATE;");
    try {
      for (let index = 0; index < 40; index += 1) {
        const clientMessageId = `bounded-message-${index}`;
        const turnId = `bounded-turn-${index}`;
        const createdAt = new Date(Date.UTC(2026, 8, 1, 0, 0, index)).toISOString();
        insertEvent.run("conversation_message_claimed", JSON.stringify({
          clientMessageId,
          projectId: project.id,
          text: `message ${index}`,
          promptDigest: createHash("sha256").update(`message ${index}`, "utf8").digest("hex"),
        }), createdAt);
        insertEvent.run("conversation_message_accepted", JSON.stringify({
          clientMessageId,
          threadId: "bounded-thread",
          turnId,
        }), createdAt);
        insertEvent.run("conversation_assistant_message", JSON.stringify({
          messageId: `bounded-reply-${index}`,
          threadId: "bounded-thread",
          turnId,
          text: `reply ${index} ${"x".repeat(20_000)}`,
        }), createdAt);
        insertEvent.run("conversation_thread_handoff", JSON.stringify({
          fromThreadId: `from-${index}`,
          toThreadId: `to-${index}`,
          reason: "bounded acceptance",
        }), createdAt);
      }
      application.store.database.exec("COMMIT;");
    } catch (error) {
      application.store.database.exec("ROLLBACK;");
      throw error;
    }
    const boundedSurface = application.service.fourCoreSurface();
    assert.equal(boundedSurface.conversation.messages_truncated, true);
    assert.ok(boundedSurface.conversation.messages.length <= 64);
    assert.ok(Buffer.byteLength(JSON.stringify(boundedSurface.conversation), "utf8") < 600_000);
    assert.equal(boundedSurface.conversation.handoffs_truncated, true);
    assert.equal(boundedSurface.conversation.handoffs.length, 32);

    const oldReplayResponse = await fetch(`${origin}/api/conversation/messages`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        projectId: project.id,
        clientMessageId: "bounded-message-0",
        text: "message 0",
      }),
    });
    assert.equal(oldReplayResponse.status, 200);
    const oldReplay = await oldReplayResponse.json();
    assert.equal(oldReplay.acknowledged_client_message_id, "bounded-message-0");
    assert.equal(oldReplay.messages.some(({ id }) => id === "bounded-message-0"), false);
    assert.equal(application.store.database.prepare(`
      SELECT COUNT(*) AS count FROM work_events
      WHERE work_item_id = 'primary' AND kind = 'conversation_message_claimed'
        AND json_extract(payload_json, '$.clientMessageId') = 'bounded-message-0'
    `).get().count, 1);

    const mutationResponse = await fetch(`${origin}/api/conversation/messages`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        projectId: project.id,
        clientMessageId: "bounded-live-mutation",
        text: "確認長期對話仍可有界送出",
      }),
    });
    assert.equal(mutationResponse.status, 200);
    const mutationConversation = await mutationResponse.json();
    assert.equal(mutationConversation.acknowledged_client_message_id, "bounded-live-mutation");
    assert.equal(mutationConversation.messages_truncated, true);
    assert.equal(
      mutationConversation.messages.filter(({ id }) => id === "bounded-live-mutation").length,
      1,
    );
    assert.ok(Buffer.byteLength(JSON.stringify(mutationConversation), "utf8") < 600_000);

    const workDetailResponse = await fetch(
      `${origin}/api/four-core/work/${encodeURIComponent(foundation.id)}`
        + `?revision=${currentWorkSnapshot.revision}&digest=${currentWorkSnapshot.digest}`,
    );
    assert.equal(workDetailResponse.status, 200);
    const workDetail = await workDetailResponse.json();
    assert.deepEqual(workDetail.graph_node.reverse_dependents, [interfaceWork.id]);
    assert.equal(workDetail.revision, currentWorkSnapshot.revision);
    assert.equal(workDetail.digest, currentWorkSnapshot.digest);

    const currentDecision = surface.decisions.decisions[0];
    const decisionDetailResponse = await fetch(
      `${origin}/api/four-core/decisions/${encodeURIComponent(currentDecision.id)}`
        + `?revision=${surface.decisions.revision}&digest=${surface.decisions.digest}`,
    );
    assert.equal(decisionDetailResponse.status, 200);
    const decisionDetail = await decisionDetailResponse.json();
    assert.equal(decisionDetail.lineage.length, 2);
    assert.deepEqual(decisionDetail.lineage.map(({ status }) => status), ["superseded", "current"]);
    assert.equal(JSON.stringify(decisionDetail).includes("rationale"), false);
  } finally {
    await new Promise((resolve) => application.server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});

test("the conversation HTTP API rejects a newer ID while an earlier durable claim is unresolved", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-conversation-order-http-"));
  const codex = new FakeCodex();
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
  });
  try {
    const project = application.service.createProject({ name: "HTTP claim order", rootPath: directory });
    const seedFence = acquirePrimaryConversationFence(application.store, project.id);
    application.store.claimPrimaryConversationMessage({
      projectId: project.id,
      clientMessageId: "http-ordered-message-001",
      text: "這一則已保存但尚未 accepted。",
      fence: seedFence,
    });
    application.store.updateWorkItem("primary", {
      status: "failed",
      failure_summary: "模擬 thread/start 前停止",
    }, seedFence);
    application.store.releasePrimaryConversationLease(seedFence);
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const { port } = application.server.address();

    const response = await fetch(`http://127.0.0.1:${port}/api/conversation/messages`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        projectId: project.id,
        clientMessageId: "http-ordered-message-002",
        text: "這一則不得越過前一則。",
      }),
    });
    assert.equal(response.status, 409);
    assert.equal((await response.json()).code, "CONVERSATION_BUSY");
    assert.equal(codex.threadStarts.length, 0);
    assert.equal(codex.turnStarts.length, 0);
    assert.equal(application.store.listEvents("primary")
      .filter(({ kind }) => kind === "conversation_message_claimed").length, 1);
  } finally {
    await new Promise((resolve) => application.server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});

test("local HTTP API persists projects and work items without starting Codex", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-control-http-"));
  const codex = new FakeCodex();
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex,
  });
  try {
    await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
    const address = application.server.address();
    const origin = `http://127.0.0.1:${address.port}`;

    const page = await fetch(`${origin}/`);
    assert.equal(page.status, 200);
    const pageHtml = await page.text();
    assert.match(pageHtml, /LATTICE Control/u);
    assert.match(pageHtml, /對話.*工作圖譜.*工作樹.*決策記憶/su);
    assert.match(pageHtml, /data-lattice-shell="desktop-cockpit"/u);
    assert.match(pageHtml, /class="side-rail"/u);
    assert.match(pageHtml, /class="workspace-canvas"/u);
    assert.match(pageHtml, /id="desktop-inspector"/u);
    assert.match(pageHtml, /id="graph-edge-layer"/u);
    assert.match(pageHtml, /id="recent-work-list"/u);
    assert.match(pageHtml, /class="composer command-dock"/u);
    assert.doesNotMatch(pageHtml, /Long-lived workspace/u);
    assert.doesNotMatch(pageHtml, /Codex 用量 68%/u);
    assert.doesNotMatch(pageHtml, /id="receipt-form"/u);
    assert.doesNotMatch(pageHtml, /\/api\/installation-receipts/u);
    assert.doesNotMatch(pageHtml, /來源 commit|安裝位置|產物 SHA-256|收據指紋/u);
    assert.match(pageHtml, /async function poll/u);
    assert.match(pageHtml, /if\(state\.pollPromise\)return state\.pollPromise/u);
    assert.match(pageHtml, /state\.pollPromise=\(async\(\)=>\{try\{\s*await refresh\(\);/u);
    assert.equal(pageHtml.match(/await refresh\(\);/gu)?.length, 1);

    const projectResponse = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "Demo", rootPath: directory }),
    });
    assert.equal(projectResponse.status, 201);
    const project = await projectResponse.json();

    const receiptInput = {
      projectId: project.id,
      component: "lattice-cli",
      sourceCommitSha: "A".repeat(40),
      artifactPath: path.join(directory, "bin", "lattice.exe"),
      artifactSha256: "B".repeat(64),
    };
    const receiptResponse = await fetch(`${origin}/api/installation-receipts`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(receiptInput),
    });
    assert.equal(receiptResponse.status, 201);
    const receipt = await receiptResponse.json();
    assert.equal(receipt.observation_kind, "OBSERVED_AFTER_INSTALL");
    assert.equal(receipt.authority, "NON_AUTHORITATIVE");

    const retryResponse = await fetch(`${origin}/api/installation-receipts`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(receiptInput),
    });
    assert.equal(retryResponse.status, 200);
    assert.deepEqual(await retryResponse.json(), receipt);

    const receiptListResponse = await fetch(
      `${origin}/api/installation-receipts?limit=1&offset=0`,
    );
    assert.equal(receiptListResponse.status, 200);
    assert.deepEqual(await receiptListResponse.json(), [receipt]);

    const receiptReplayResponse = await fetch(
      `${origin}/api/installation-receipts/${encodeURIComponent(receipt.id)}`,
    );
    assert.equal(receiptReplayResponse.status, 200);
    assert.deepEqual(await receiptReplayResponse.json(), receipt);

    const invalidListResponse = await fetch(`${origin}/api/installation-receipts?limit=0`);
    assert.equal(invalidListResponse.status, 400);

    const invalidReceiptResponse = await fetch(`${origin}/api/installation-receipts`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ...receiptInput, artifactPath: "relative/lattice.exe" }),
    });
    assert.equal(invalidReceiptResponse.status, 400);

    const itemResponse = await fetch(`${origin}/api/work-items`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        projectId: project.id,
        title: "First useful path",
        objective: "Keep it small.",
        priority: "high",
      }),
    });
    assert.equal(itemResponse.status, 201);
    const item = await itemResponse.json();

    const continuationResponse = await fetch(`${origin}/api/work-items/${item.id}/continuation`);
    assert.equal(continuationResponse.status, 200);
    assert.deepEqual(await continuationResponse.json(), {
      schema_version: "lattice.control.continuation.v1",
      project: { name: "Demo", root_path: path.resolve(directory) },
      work: {
        id: item.id,
        title: "First useful path",
        objective: "Keep it small.",
        priority: "high",
        status: "draft",
        codex_thread_id: null,
      },
      current: {
        progress: null,
        failure_summary: null,
        verification_notes: null,
        next_action: "Start the work in a new Codex thread.",
      },
      evidence: { latest_event: "created" },
    });

    const state = await (await fetch(`${origin}/api/state`)).json();
    assert.equal(state.codexConnected, false);
    assert.equal(state.projects.length, 1);
    assert.equal(state.workItems.length, 1);
    assert.equal(state.workItems[0].status, "draft");
    assert.equal("installationReceipts" in state, false);
    assert.equal(state.installationReceiptCount, 1);
  } finally {
    await new Promise((resolve) => application.server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});
