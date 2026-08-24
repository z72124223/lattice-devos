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
