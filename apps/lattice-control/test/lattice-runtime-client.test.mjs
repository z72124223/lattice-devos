import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough, Writable } from "node:stream";
import test from "node:test";
import { LatticeRuntimeClient } from "../src/lattice-runtime-client.mjs";

function fixture({ onCall, callsPerSession = 48, tools, delayedExit = false } = {}) {
  const sessions = [];
  const client = new LatticeRuntimeClient({
    configurationLoader: async () => ({ executablePath: "/fixture/latticed", environment: {} }),
    requestTimeoutMs: 40, startupTimeoutMs: 100, cleanupTimeoutMs: 20, callsPerSession,
    spawnProcess: () => {
      const child = new EventEmitter();
      child.stdout = new PassThrough();
      child.exitCode = null;
      child.signalCode = null;
      const session = { child, calls: [], kills: 0 };
      sessions.push(session);
      function reply(id, result) {
        const bytes = Buffer.from(JSON.stringify({ jsonrpc: "2.0", id, result }) + "\n");
        // Include byte splits inside Unicode messages.
        for (const byte of bytes) child.stdout.write(Buffer.from([byte]));
      }
      child.kill = () => {
        session.kills += 1;
        child.signalCode = "SIGTERM";
        if (!delayedExit) queueMicrotask(() => child.emit("close", null, "SIGTERM"));
      };
      child.stdin = new Writable({
        write(bytes, _encoding, callback) {
          const message = JSON.parse(bytes.toString());
          queueMicrotask(() => {
            if (message.method === "initialize") reply(message.id, {
              protocolVersion: "2025-11-25", serverInfo: { name: "latticed" },
            });
            else if (message.method === "tools/list") reply(message.id, {
              tools: (tools ?? ["lattice_task_submit", "lattice_task_status",
                "lattice_control_snapshot", "lattice_control_update"]).map((name) => ({ name })),
            });
            else if (message.method === "tools/call") {
              session.calls.push(message.params);
              if (onCall) onCall({ message, session, reply });
              else reply(message.id, { content: [{ type: "text", text: JSON.stringify({
                status: "SUBMITTED", title: "繁體中文結果",
              }) }] });
            }
          });
          callback();
        },
        final(callback) {
          child.exitCode = 0;
          queueMicrotask(() => child.emit("close", 0, null));
          callback();
        },
      });
      return child;
    },
  });
  return { client, sessions };
}

test("Runtime keeps request bytes stable and rotates before its bounded session budget", async () => {
  const { client, sessions } = fixture({ callsPerSession: 1 });
  try {
    const args = { objective: "建立小工具", client_request_id: "one" };
    const first = client.call("lattice_task_submit", args);
    args.objective = "changed after submission";
    assert.equal((await first).title, "繁體中文結果");
    await client.call("lattice_task_status", { task_ref: "a".repeat(64) });
    assert.equal(sessions.length, 2);
    assert.equal(sessions[0].calls[0].arguments.objective, "建立小工具");
    assert.equal(sessions[0].calls.length, 1);
    assert.equal(sessions[1].calls.length, 1);
    assert.equal(sessions[0].child.exitCode, 0);
  } finally { await client.close(); }
});

test("Runtime never resends a mutation after an uncertain response", async () => {
  let invocation = 0;
  const { client, sessions } = fixture({ onCall: ({ message, reply }) => {
    invocation += 1;
    if (invocation > 1) reply(message.id, {
      content: [], structuredContent: { status: "SUBMITTED" },
    });
  } });
  try {
    await assert.rejects(client.call("lattice_task_submit", { client_request_id: "uncertain" }),
      { code: "LATTICE_RUNTIME_OUTCOME_UNKNOWN" });
    assert.equal(invocation, 1);
    assert.equal(sessions[0].kills, 1);
    await client.call("lattice_task_status", { task_ref: "a".repeat(64) });
    assert.equal(sessions.length, 2);
    assert.deepEqual(sessions.map((session) => session.calls.map((call) => call.name)),
      [["lattice_task_submit"], ["lattice_task_status"]]);
  } finally { await client.close(); }
});

test("Runtime reports an older installed tool surface without making a write", async () => {
  const { client, sessions } = fixture({ tools: ["lattice_task_status"] });
  try {
    await assert.rejects(client.call("lattice_control_update", { action: "METADATA" }),
      { code: "LATTICE_RUNTIME_UPGRADE_REQUIRED" });
    assert.equal(sessions[0].calls.length, 0);
  } finally { await client.close(); }
});

test("closing while Runtime configuration loads prevents a late process spawn", async () => {
  let resolveConfiguration;
  let spawns = 0;
  const client = new LatticeRuntimeClient({
    configurationLoader: () => new Promise((resolve) => { resolveConfiguration = resolve; }),
    spawnProcess: () => { spawns += 1; throw new Error("unexpected spawn"); },
  });
  const pending = client.call("lattice_task_status", { task_ref: "a".repeat(64) });
  const rejected = assert.rejects(pending, { code: "LATTICE_RUNTIME_CLIENT_CLOSED" });
  await Promise.resolve();
  await client.close();
  await rejected;
  resolveConfiguration({ executablePath: "/fixture/latticed", environment: {} });
  await Promise.resolve();
  assert.equal(spawns, 0);
});

test("an unconfirmed Runtime exit prevents a subsequent process and operation", async () => {
  const { client, sessions } = fixture({ onCall: () => {}, delayedExit: true });
  try {
    await assert.rejects(client.call("lattice_task_submit", { client_request_id: "one" }),
      { code: "LATTICE_RUNTIME_OUTCOME_UNKNOWN" });
    await assert.rejects(client.call("lattice_task_submit", { client_request_id: "two" }),
      { code: "LATTICE_RUNTIME_PROCESS_STILL_CLOSING" });
    assert.equal(sessions.length, 1);
    assert.equal(sessions[0].calls.length, 1);
  } finally {
    sessions[0].child.emit("close", null, "SIGTERM");
    await client.close();
  }
});
