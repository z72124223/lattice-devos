import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
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

  async startThread({ cwd, model }) {
    this.connected = true;
    this.threads += 1;
    this.startOptions = { cwd, model };
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

  async resumeThread(threadId) {
    this.connected = true;
    this.resumed.push(threadId);
    return this.resumeResult ?? {
      id: threadId,
      turns: [{ id: "turn-1", status: "completed" }],
    };
  }

  async startTurn(threadId, text) {
    this.turns += 1;
    const turn = { id: `turn-${this.turns}`, items: [], status: "inProgress" };
    this.lastTurn = { threadId, text, turnId: turn.id };
    this.turnStarts.push(this.lastTurn);
    this.emit("turnStartAccepted", this.lastTurn);
    this.beforeTurnResult?.({ threadId, text });
    if (this.autoTurnStarted) {
      this.emit("notification", {
        method: "turn/started",
        params: { threadId, turn },
      });
    }
    return turn;
  }

  respond(id, result) {
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
          || !["interrupted", "failed"].includes(turn.status)
        ) return;
        clearTimeout(timer);
        this.off("notification", listener);
        resolve(turn);
      };
      const timer = setTimeout(() => {
        this.off("notification", listener);
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
    this.connected = false;
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
    assert.match(pageHtml, /安裝證據由 AI 自動管理，不需要手動輸入/u);
    assert.doesNotMatch(pageHtml, /id="receipt-form"/u);
    assert.doesNotMatch(pageHtml, /\/api\/installation-receipts/u);
    assert.doesNotMatch(pageHtml, /來源 commit|安裝位置|產物 SHA-256|收據指紋/u);
    assert.match(pageHtml, /async function poll/u);
    assert.match(pageHtml, /async function poll\(\) \{\s*try \{\s*await refresh\(\);/u);
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
