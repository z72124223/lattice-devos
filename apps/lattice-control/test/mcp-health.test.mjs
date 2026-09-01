import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { PassThrough } from "node:stream";
import test from "node:test";

import {
  ControlMcpHealthMonitor,
  probeBundledControlMcps,
  probeControlMcpEndpoint,
} from "../src/mcp-health.mjs";
import { LatticeStore } from "../src/store.mjs";
import { controlWorkMcpTools } from "../src/work-core-mcp.mjs";

function fakeMcpSpawn({
  protocolVersion = "2025-11-25",
  tools = controlWorkMcpTools,
  hang = false,
} = {}) {
  return () => {
    const child = new EventEmitter();
    child.stdin = new PassThrough();
    child.stdout = new PassThrough();
    child.exitCode = null;
    child.signalCode = null;
    let buffer = "";
    child.stdin.on("data", (chunk) => {
      if (hang) return;
      buffer += chunk.toString("utf8");
      while (buffer.includes("\n")) {
        const newline = buffer.indexOf("\n");
        const frame = JSON.parse(buffer.slice(0, newline));
        buffer = buffer.slice(newline + 1);
        if (frame.method === "initialize") {
          child.stdout.write(`${JSON.stringify({
            jsonrpc: "2.0",
            id: 1,
            result: {
              protocolVersion,
              capabilities: { tools: { listChanged: false } },
              serverInfo: { name: "lattice-control-work-core", version: "1.0.0" },
            },
          })}\n`);
        } else if (frame.method === "tools/list") {
          child.stdout.write(`${JSON.stringify({
            jsonrpc: "2.0",
            id: 2,
            result: { tools },
          })}\n`);
        }
      }
    });
    child.stdin.once("finish", () => {
      if (child.exitCode !== null || child.signalCode !== null) return;
      child.exitCode = 0;
      child.stdout.end();
      child.emit("exit", 0, null);
    });
    child.kill = () => {
      if (child.exitCode !== null || child.signalCode !== null) return false;
      child.signalCode = "SIGTERM";
      child.stdout.end();
      child.emit("exit", null, "SIGTERM");
      return true;
    };
    return child;
  };
}

async function bounded(promise, timeoutMs = 500) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("test guard timed out")), timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

test("the runtime health probe performs real stdio initialize and tools/list handshakes", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-mcp-health-"));
  const databasePath = path.join(directory, "control.db");
  const store = new LatticeStore(databasePath);
  try {
    const health = await probeBundledControlMcps({
      databasePath,
      timeoutMs: 5_000,
    });
    assert.deepEqual(health, {
      work_mcp: "HEALTHY",
      decision_mcp: "HEALTHY",
    });
  } finally {
    store.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("the monitor shares concurrent probes and caches only for a bounded TTL", async () => {
  let calls = 0;
  let now = 100;
  let release;
  const gate = new Promise((resolve) => { release = resolve; });
  const probe = async () => {
    calls += 1;
    await gate;
    return { work_mcp: "HEALTHY", decision_mcp: "UNREACHABLE" };
  };
  const monitor = new ControlMcpHealthMonitor({
    databasePath: path.resolve("control.db"),
    ttlMs: 2_000,
    probe,
    now: () => now,
  });
  const first = monitor.current();
  const concurrent = monitor.current();
  release();
  assert.deepEqual(await first, await concurrent);
  assert.equal(calls, 1);
  await monitor.current();
  assert.equal(calls, 1);
  now += 2_001;
  await monitor.current();
  assert.equal(calls, 2);
});

test("bundled MCP probes start concurrently instead of consuming two serial deadlines", async () => {
  let calls = 0;
  let release;
  let bothStarted;
  const releaseGate = new Promise((resolve) => { release = resolve; });
  const bothStartedGate = new Promise((resolve) => { bothStarted = resolve; });
  const probing = probeBundledControlMcps({
    databasePath: path.resolve("control.db"),
    timeoutMs: 250,
    probeEndpoint: async ({ expectedName }) => {
      calls += 1;
      if (calls === 2) bothStarted();
      await releaseGate;
      return expectedName.includes("work") ? "UNREACHABLE" : "HEALTHY";
    },
  });
  await bounded(bothStartedGate);
  release();
  assert.deepEqual(await probing, {
    work_mcp: "UNREACHABLE",
    decision_mcp: "HEALTHY",
  });
});

test("MCP health rejects wrong protocol, widened schemas, and duplicate or extra tools", async () => {
  const databasePath = path.resolve("control.db");
  const expected = {
    scriptPath: path.resolve("fake-work-mcp.mjs"),
    databasePath,
    expectedName: "lattice-control-work-core",
    expectedTools: controlWorkMcpTools,
    timeoutMs: 250,
  };
  assert.equal(await probeControlMcpEndpoint({
    ...expected,
    spawnProcess: fakeMcpSpawn({ protocolVersion: "2025-06-18" }),
  }), "INCOMPATIBLE");

  const widened = structuredClone(controlWorkMcpTools);
  widened[0].inputSchema.properties.project_id.maxLength = 4_096;
  assert.equal(await probeControlMcpEndpoint({
    ...expected,
    spawnProcess: fakeMcpSpawn({ tools: widened }),
  }), "INCOMPATIBLE");

  assert.equal(await probeControlMcpEndpoint({
    ...expected,
    spawnProcess: fakeMcpSpawn({ tools: [controlWorkMcpTools[0], controlWorkMcpTools[0]] }),
  }), "INCOMPATIBLE");
  assert.equal(await probeControlMcpEndpoint({
    ...expected,
    spawnProcess: fakeMcpSpawn({ tools: [
      ...controlWorkMcpTools,
      { ...controlWorkMcpTools[0], name: "unexpected_tool" },
    ] }),
  }), "INCOMPATIBLE");
});

test("a hanging MCP endpoint becomes UNREACHABLE within the bounded probe deadline", async () => {
  const status = await probeControlMcpEndpoint({
    scriptPath: path.resolve("hung-work-mcp.mjs"),
    databasePath: path.resolve("control.db"),
    expectedName: "lattice-control-work-core",
    expectedTools: controlWorkMcpTools,
    timeoutMs: 100,
    spawnProcess: fakeMcpSpawn({ hang: true }),
  });
  assert.equal(status, "UNREACHABLE");
});
