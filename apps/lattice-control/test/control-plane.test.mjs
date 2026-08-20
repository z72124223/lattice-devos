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
  turns = 0;
  responses = [];
  archived = [];
  resumed = [];

  async startThread({ cwd, model }) {
    this.connected = true;
    this.startOptions = { cwd, model };
    return { id: "thread-1" };
  }

  async resumeThread(threadId) {
    this.connected = true;
    this.resumed.push(threadId);
    return { id: threadId };
  }

  async startTurn(threadId, text) {
    this.turns += 1;
    this.lastTurn = { threadId, text };
    this.beforeTurnResult?.({ threadId, text });
    return { id: `turn-${this.turns}` };
  }

  respond(id, result) {
    this.responses.push({ id, result });
  }

  async archiveThread(threadId) {
    this.archived.push(threadId);
    return {};
  }

  async close() {
    this.connected = false;
  }
}

test("work survives restart and keeps the same Codex thread through verification and archive", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-control-"));
  const databasePath = path.join(directory, "control.db");
  try {
    const firstCodex = new FakeCodex();
    const firstStore = new LatticeStore(databasePath);
    const firstService = new LatticeControlService({ store: firstStore, codex: firstCodex });
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
      params: { threadId: "thread-1", item: { type: "commandExecution" } },
    });
    assert.equal(firstStore.getWorkItem(created.id).progress, "Running commandExecution");

    firstCodex.emit("serverRequest", {
      id: 42,
      method: "item/commandExecution/requestApproval",
      params: { threadId: "thread-1", reason: "Run focused tests", command: "npm test" },
    });
    assert.equal(firstStore.getWorkItem(created.id).status, "waiting_approval");
    firstService.approve(created.id, "accept");
    assert.deepEqual(firstCodex.responses, [{ id: 42, result: { decision: "accept" } }]);

    firstCodex.emit("notification", {
      method: "turn/completed",
      params: { threadId: "thread-1", turn: { id: "turn-1", status: "completed" } },
    });
    assert.equal(firstStore.getWorkItem(created.id).status, "codex_done");
    firstStore.close();

    const secondCodex = new FakeCodex();
    const secondStore = new LatticeStore(databasePath);
    const secondService = new LatticeControlService({ store: secondStore, codex: secondCodex });
    const restored = secondService.workItem(created.id);
    assert.equal(restored.item.codex_thread_id, "thread-1");
    assert.ok(restored.events.some((event) => event.kind === "approval_requested"));
    assert.ok(restored.events.some((event) => event.kind === "turn_completed"));

    await secondService.resume(created.id, "Review the completed change once more.");
    assert.deepEqual(secondCodex.resumed, ["thread-1"]);
    assert.equal(secondCodex.lastTurn.threadId, "thread-1");
    secondCodex.emit("notification", {
      method: "turn/completed",
      params: { threadId: "thread-1", turn: { id: "turn-1", status: "completed" } },
    });

    const verified = secondService.verify(created.id, "Focused tests passed.");
    assert.equal(verified.status, "verified");
    const archived = await secondService.archive(created.id);
    assert.equal(archived.status, "archived");
    assert.deepEqual(secondCodex.archived, ["thread-1"]);
    secondStore.close();
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("an approval arriving immediately after turn start is retained instead of auto-declined", async () => {
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
    codex.beforeTurnResult = ({ threadId }) => codex.emit("serverRequest", {
      id: 7,
      method: "item/commandExecution/requestApproval",
      params: { threadId, reason: "Immediate approval" },
    });

    const started = await service.start(item.id);
    assert.equal(started.codex_thread_id, "thread-1");
    assert.equal(started.status, "waiting_approval");
    assert.equal(started.approval.requestId, 7);
    assert.deepEqual(codex.responses, []);
    const declined = service.approve(item.id, "decline");
    assert.equal(declined.status, "running");
    assert.equal(declined.failure_summary, null);
  } finally {
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
    assert.match(await page.text(), /LATTICE Control/u);

    const projectResponse = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "Demo", rootPath: directory }),
    });
    assert.equal(projectResponse.status, 201);
    const project = await projectResponse.json();

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

    const state = await (await fetch(`${origin}/api/state`)).json();
    assert.equal(state.codexConnected, false);
    assert.equal(state.projects.length, 1);
    assert.equal(state.workItems.length, 1);
    assert.equal(state.workItems[0].status, "draft");
  } finally {
    await new Promise((resolve) => application.server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});
