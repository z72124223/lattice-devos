import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { PassThrough, Writable } from "node:stream";
import test from "node:test";
import { setImmediate as waitImmediate, setTimeout as delay } from "node:timers/promises";
import { LatticeStore } from "../src/store.mjs";
import { ControlWorkService } from "../src/work-core-service.mjs";
import { runControlWorkMcp } from "../src/work-core-mcp.mjs";

function startMcp(databasePath) {
  const child = spawn(process.execPath, [
    path.resolve(import.meta.dirname, "../src/work-core-mcp.mjs"),
  ], {
    env: { ...process.env, LATTICE_CONTROL_DATABASE_PATH: databasePath },
    stdio: ["pipe", "pipe", "pipe"],
    windowsHide: true,
  });
  const pending = new Map();
  const stderr = [];
  createInterface({ input: child.stdout, crlfDelay: Infinity }).on("line", (line) => {
    const message = JSON.parse(line);
    const waiter = pending.get(message.id);
    if (!waiter) throw new Error(`unexpected MCP response ${line}`);
    pending.delete(message.id);
    waiter.resolve(message);
  });
  child.stderr.on("data", (chunk) => stderr.push(chunk));
  const rejectPending = (error) => {
    for (const waiter of pending.values()) waiter.reject(error);
    pending.clear();
  };
  child.once("error", rejectPending);
  child.once("exit", (code) => {
    if (pending.size > 0) rejectPending(new Error(`MCP child exited before response (${code})`));
  });
  return {
    child,
    stderr,
    request(message) {
      const response = new Promise((resolve, reject) => pending.set(message.id, { resolve, reject }));
      child.stdin.write(`${JSON.stringify(message)}\n`);
      return response;
    },
    rawRequest(id, serialized) {
      const response = new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
      child.stdin.write(`${serialized}\n`);
      return response;
    },
    notify(message) {
      child.stdin.write(`${JSON.stringify(message)}\n`);
    },
  };
}

test("Control work MCP serves two bounded read tools over a real STDIO child", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-work-mcp-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  let session;
  try {
    store = new LatticeStore(databasePath);
    const service = new ControlWorkService({ store });
    const project = store.createProject({ name: "MCP", rootPath: directory });
    const prerequisite = store.createWorkItem({
      projectId: project.id,
      title: "Migration",
      objective: "Prove the migration.",
    });
    const dependent = store.createWorkItem({
      projectId: project.id,
      title: "MCP read",
      objective: "Expose the real Control work state.",
    });
    const initial = service.workSnapshot({ projectId: project.id });
    service.setWorkRelations({
      projectId: project.id,
      workItemId: dependent.id,
      dependsOn: [prerequisite.id],
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    });
    store.close();
    store = null;

    session = startMcp(databasePath);
    const nestedMeta = `${"[".repeat(10_000)}0${"]".repeat(10_000)}`;
    const rejectedNestedMeta = await session.rawRequest(
      9,
      `{"jsonrpc":"2.0","id":9,"method":"ping","params":{"_meta":{"nested":${nestedMeta}}}}`,
    );
    assert.equal(rejectedNestedMeta.error.code, -32602);
    const malformedInitialize = await session.request({
      jsonrpc: "2.0",
      id: 10,
      method: "initialize",
      params: { protocolVersion: "2025-11-25" },
    });
    assert.equal(malformedInitialize.error.code, -32602);
    const preInitializePing = await session.request({
      jsonrpc: "2.0",
      id: 0,
      method: "ping",
      params: { _meta: { progressToken: "must-not-be-echoed" } },
    });
    assert.deepEqual(preInitializePing.result, {});
    assert.doesNotMatch(JSON.stringify(preInitializePing), /must-not-be-echoed/u);
    const initialize = await session.request({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2099-01-01",
        capabilities: {},
        clientInfo: { name: "work-core-test", version: "1" },
        _meta: { progressToken: "initialize-token" },
      },
    });
    assert.equal(initialize.result.protocolVersion, "2025-11-25");
    assert.equal(initialize.result.serverInfo.name, "lattice-control-work-core");
    assert.doesNotMatch(JSON.stringify(initialize), /initialize-token/u);
    session.notify({ jsonrpc: "2.0", method: "notifications/initialized", params: null });
    const stillUninitialized = await session.request({
      jsonrpc: "2.0",
      id: 11,
      method: "tools/list",
      params: {},
    });
    assert.equal(stillUninitialized.error.code, -32002);
    session.notify({
      jsonrpc: "2.0",
      method: "notifications/initialized",
      params: { _meta: { progressToken: 1 } },
    });
    session.notify({ jsonrpc: "2.0", method: "ping", params: {} });
    session.notify({ jsonrpc: "2.0", method: "tools/list", params: {} });
    session.notify({
      jsonrpc: "2.0",
      method: "tools/call",
      params: { name: "lattice_control_work_snapshot", arguments: {} },
    });

    const postInitializePing = await session.request({
      jsonrpc: "2.0",
      id: 7,
      method: "ping",
      params: {},
    });
    assert.deepEqual(postInitializePing.result, {});

    const listed = await session.request({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/list",
      params: { _meta: { progressToken: 2 } },
    });
    assert.deepEqual(
      listed.result.tools.map(({ name }) => name),
      ["lattice_control_work_snapshot", "lattice_control_work_node"],
    );

    const snapshotResponse = await session.request({
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: {
        name: "lattice_control_work_snapshot",
        arguments: { project_id: project.id, max_nodes: 10, max_edges: 20 },
        _meta: { progressToken: "snapshot-token" },
      },
    });
    assert.equal(snapshotResponse.result.isError, false);
    assert.doesNotMatch(JSON.stringify(snapshotResponse), /snapshot-token/u);
    const snapshot = snapshotResponse.result.structuredContent;
    assert.equal(snapshot.tree.revision, snapshot.graph.revision);
    assert.equal(snapshot.tree.digest, snapshot.graph.digest);

    const nodeResponse = await session.request({
      jsonrpc: "2.0",
      id: 4,
      method: "tools/call",
      params: {
        name: "lattice_control_work_node",
        arguments: {
          project_id: project.id,
          work_item_id: dependent.id,
          revision: snapshot.revision,
          digest: snapshot.digest,
          max_nodes: 10,
          max_edges: 20,
        },
      },
    });
    assert.deepEqual(nodeResponse.result.structuredContent.graph_node.depends_on, [prerequisite.id]);
    assert.deepEqual(
      nodeResponse.result.structuredContent.graph_node.blocker.reasons,
      [{ kind: "dependency", work_item_id: prerequisite.id, status: "draft" }],
    );

    const invalid = await session.request({
      jsonrpc: "2.0",
      id: 5,
      method: "tools/call",
      params: {
        name: "lattice_control_work_snapshot",
        arguments: { project_id: project.id, unexpected: true },
      },
    });
    assert.equal(invalid.result.isError, true);
    assert.equal(
      invalid.result.structuredContent.error.code,
      "CONTROL_WORK_TOOL_ARGUMENTS_REJECTED",
    );

    const missingArguments = await session.request({
      jsonrpc: "2.0",
      id: 8,
      method: "tools/call",
      params: {
        name: "lattice_control_work_snapshot",
        _meta: { progressToken: 8 },
      },
    });
    assert.equal(missingArguments.result.isError, true);
    assert.equal(
      missingArguments.result.structuredContent.error.code,
      "CONTROL_WORK_TOOL_ARGUMENTS_REJECTED",
    );

    const stale = await session.request({
      jsonrpc: "2.0",
      id: 6,
      method: "tools/call",
      params: {
        name: "lattice_control_work_node",
        arguments: {
          project_id: project.id,
          work_item_id: dependent.id,
          revision: "0".repeat(64),
          digest: snapshot.digest,
        },
      },
    });
    assert.equal(stale.result.isError, true);
    assert.equal(
      stale.result.structuredContent.error.code,
      "CONTROL_WORK_REVISION_MISMATCH",
    );

    session.child.stdin.end();
    const [exitCode] = await once(session.child, "exit");
    assert.equal(exitCode, 0);
    assert.equal(Buffer.concat(session.stderr).toString("utf8"), "");
  } finally {
    if (session?.child.exitCode == null) session.child.kill();
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("Control work MCP rejects an oversized STDIO frame and closes non-zero", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-work-mcp-frame-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  let child;
  try {
    store = new LatticeStore(databasePath);
    store.close();
    store = null;
    child = spawn(process.execPath, [
      path.resolve(import.meta.dirname, "../src/work-core-mcp.mjs"),
    ], {
      env: { ...process.env, LATTICE_CONTROL_DATABASE_PATH: databasePath },
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    child.stdin.end(`${"x".repeat(65_537)}\n`);
    const [exitCode] = await once(child, "exit");
    assert.equal(exitCode, 1);
    const response = JSON.parse(Buffer.concat(stdout).toString("utf8").trim());
    assert.equal(response.error.code, -32600);
    assert.equal(response.error.message, "MCP frame size rejected");
    assert.equal(Buffer.concat(stderr).toString("utf8"), "");
  } finally {
    if (child?.exitCode == null) child.kill();
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("Control work MCP returns a large valid snapshot without duplicating it in text content", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-work-mcp-large-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  let session;
  try {
    store = new LatticeStore(databasePath);
    const project = store.createProject({ name: "Large MCP", rootPath: directory });
    for (let index = 0; index < 64; index += 1) {
      const item = store.createWorkItem({
        projectId: project.id,
        title: `Bounded work ${index}`,
        objective: `${index}:`.padEnd(4_096, "x"),
      });
      store.updateWorkItem(item.id, { progress: `${index}:`.padEnd(4_096, "y") });
    }
    store.close();
    store = null;

    session = startMcp(databasePath);
    await session.request({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        capabilities: {},
        clientInfo: { name: "work-core-large-test", version: "1" },
      },
    });
    session.notify({ jsonrpc: "2.0", method: "notifications/initialized", params: {} });
    const response = await session.request({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: {
        name: "lattice_control_work_snapshot",
        arguments: { project_id: project.id, max_nodes: 64, max_edges: 64 },
      },
    });
    assert.equal(response.result.isError, false);
    assert.equal(response.result.structuredContent.graph.nodes.length, 64);
    assert.ok(Buffer.byteLength(JSON.stringify(response.result.structuredContent), "utf8") > 500_000);
    assert.ok(Buffer.byteLength(response.result.content[0].text, "utf8") < 1_024);

    session.child.stdin.end();
    const [exitCode] = await once(session.child, "exit");
    assert.equal(exitCode, 0);
    assert.equal(Buffer.concat(session.stderr).toString("utf8"), "");
  } finally {
    if (session?.child.exitCode == null) session.child.kill();
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("Control work MCP keeps a long-lived bounded session usable beyond 64 calls", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-work-mcp-long-session-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  let session;
  try {
    store = new LatticeStore(databasePath);
    store.close();
    store = null;
    session = startMcp(databasePath);
    const initialize = await session.request({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        capabilities: {},
        clientInfo: { name: "work-core-long-session-test", version: "1" },
      },
    });
    assert.equal(initialize.result.protocolVersion, "2025-11-25");
    session.notify({ jsonrpc: "2.0", method: "notifications/initialized", params: {} });
    let last;
    for (let index = 0; index < 70; index += 1) {
      last = await session.request({
        jsonrpc: "2.0",
        id: index + 2,
        method: "tools/list",
        params: {},
      });
    }
    assert.deepEqual(
      last.result.tools.map(({ name }) => name),
      ["lattice_control_work_snapshot", "lattice_control_work_node"],
    );

    session.child.stdin.end();
    const [exitCode] = await once(session.child, "exit");
    assert.equal(exitCode, 0);
    assert.equal(Buffer.concat(session.stderr).toString("utf8"), "");
  } finally {
    if (session?.child.exitCode == null) session.child.kill();
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("Control work MCP rate-limits only valid tool execution and recovers automatically", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-work-mcp-rate-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  let session;
  try {
    store = new LatticeStore(databasePath);
    const project = store.createProject({ name: "Rate", rootPath: directory });
    store.close();
    store = null;
    session = startMcp(databasePath);
    await session.request({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        capabilities: {},
        clientInfo: { name: "work-core-rate-test", version: "1" },
      },
    });
    session.notify({ jsonrpc: "2.0", method: "notifications/initialized", params: {} });
    const burst = await Promise.all(Array.from({ length: 17 }, (_value, index) => session.request({
      jsonrpc: "2.0",
      id: index + 2,
      method: "tools/call",
      params: {
        name: "lattice_control_work_snapshot",
        arguments: { project_id: project.id, max_nodes: 1, max_edges: 1 },
      },
    })));
    assert.equal(burst.filter(({ result }) => result?.isError === false).length, 16);
    assert.equal(
      burst.filter(({ result }) => (
        result?.structuredContent?.error?.code === "CONTROL_WORK_TOOL_RATE_LIMITED"
      )).length,
      1,
    );

    await delay(1_050);
    const recovered = await session.request({
      jsonrpc: "2.0",
      id: 100,
      method: "tools/call",
      params: {
        name: "lattice_control_work_snapshot",
        arguments: { project_id: project.id, max_nodes: 1, max_edges: 1 },
      },
    });
    assert.equal(recovered.result.isError, false);

    session.child.stdin.end();
    const [exitCode] = await once(session.child, "exit");
    assert.equal(exitCode, 0);
    assert.equal(Buffer.concat(session.stderr).toString("utf8"), "");
  } finally {
    if (session?.child.exitCode == null) session.child.kill();
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("Control work MCP pauses input on output backpressure and resumes after drain", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-work-mcp-backpressure-"));
  const databasePath = path.join(directory, "control.db");
  const input = new PassThrough();
  const chunks = [];
  const releases = [];
  const output = new Writable({
    highWaterMark: 1,
    write(chunk, _encoding, callback) {
      chunks.push(Buffer.from(chunk));
      releases.push(callback);
    },
  });
  let session;
  let store;
  try {
    store = new LatticeStore(databasePath);
    store.close();
    store = null;
    session = runControlWorkMcp({ input, output, databasePath });

    input.write(`${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping", params: {} })}\n`);
    await waitImmediate();
    assert.equal(input.isPaused(), true);
    assert.equal(output.writableNeedDrain, true);
    const firstDrain = once(output, "drain");
    releases.shift()();
    await firstDrain;
    await waitImmediate();
    assert.equal(input.isPaused(), false);

    input.write(`${JSON.stringify({ jsonrpc: "2.0", id: 2, method: "ping", params: {} })}\n`);
    await waitImmediate();
    assert.equal(input.isPaused(), true);
    const secondDrain = once(output, "drain");
    releases.shift()();
    await secondDrain;
    await waitImmediate();
    assert.equal(input.isPaused(), false);
    assert.deepEqual(
      Buffer.concat(chunks).toString("utf8").trim().split("\n").map((line) => JSON.parse(line).id),
      [1, 2],
    );

    input.write(`${JSON.stringify({ jsonrpc: "2.0", id: 3, method: "ping", params: {} })}\n`);
    await waitImmediate();
    assert.equal(input.isPaused(), true);
    session.close();
    assert.equal(output.listenerCount("drain"), 0);
    assert.equal(input.listenerCount("data"), 0);
    assert.equal(input.listenerCount("end"), 0);
    assert.equal(input.listenerCount("error"), 0);
    releases.shift()();
    input.end();
    output.end();
  } finally {
    session?.close();
    store?.close();
    input.destroy();
    output.destroy();
    await rm(directory, { recursive: true, force: true });
  }
});
