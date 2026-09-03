import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import path from "node:path";
import { PassThrough } from "node:stream";
import test from "node:test";

import {
  LatticeRuntimeHealthMonitor,
  loadLatticeRuntimeConfiguration,
  probeConfiguredLatticeRuntime,
  probeLatticeRuntimeEndpoint,
} from "../src/lattice-runtime-health.mjs";

const expectedTools = [
  "lattice_delivery_reconcile",
  "lattice_delivery_run",
  "lattice_delivery_status",
  "lattice_foreman_checkpoint",
  "lattice_runtime_status",
  "lattice_task_status",
  "lattice_task_submit",
];

function validRuntimeStatus() {
  return {
    component: "delivery-receipt",
    status: "NOT_STARTED",
    scope: "receipt-only",
    runtime_integration: "GRAPHIFY_HERMES",
    graphify_runtime_status: "READY",
    hermes_runtime_status: "PREPARED",
    hermes_activation_status: "PREPARED",
    foreman: {
      schema: "lattice.foreman-runtime-projection/1.1",
      replay_status: "VERIFIED",
      checkpoint_status: "AVAILABLE",
      ledger_digest: "a".repeat(64),
      checkpoint_digest: "b".repeat(64),
      latest_generation: 1,
      active_count: 0,
      blocked_count: 0,
      completed_count: 1,
      next_action: "ALL_COMPLETED",
      degraded_code: null,
      dependency: null,
    },
  };
}

function fakeRuntimeSpawn({
  toolNames = expectedTools,
  runtimeStatus = validRuntimeStatus(),
  toolError = false,
  onSpawn,
} = {}) {
  return (executablePath, args, options) => {
    onSpawn?.(executablePath, args, options);
    const child = new EventEmitter();
    child.stdin = new PassThrough();
    child.stdout = new PassThrough();
    child.stderr = new PassThrough();
    child.exitCode = null;
    child.signalCode = null;
    let buffer = "";
    child.stdin.on("data", (chunk) => {
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
              protocolVersion: "2025-11-25",
              capabilities: { tools: {} },
              serverInfo: { name: "latticed", title: "LATTICE DevOS", version: "1.0.0" },
              instructions: "bounded LATTICE tools",
            },
          })}\n`);
        } else if (frame.method === "tools/list") {
          child.stdout.write(`${JSON.stringify({
            jsonrpc: "2.0",
            id: 2,
            result: { tools: toolNames.map((name) => ({ name })) },
          })}\n`);
        } else if (frame.method === "tools/call") {
          child.stdout.write(`${JSON.stringify({
            jsonrpc: "2.0",
            id: 3,
            result: {
              content: [{ type: "text", text: "bounded" }],
              structuredContent: runtimeStatus,
              isError: toolError,
            },
          })}\n`);
        }
      }
    });
    child.stdin.once("finish", () => {
      if (child.exitCode !== null || child.signalCode !== null) return;
      child.exitCode = 0;
      child.stdout.end();
      child.stderr.end();
      child.emit("exit", 0, null);
    });
    child.kill = () => {
      if (child.exitCode !== null || child.signalCode !== null) return false;
      child.signalCode = "SIGTERM";
      child.stdout.end();
      child.stderr.end();
      child.emit("exit", null, "SIGTERM");
      return true;
    };
    return child;
  };
}

function inertRuntimeSpawn({ onStart, onKill } = {}) {
  return () => {
    const child = new EventEmitter();
    child.stdin = new PassThrough();
    child.stdout = new PassThrough();
    child.stderr = new PassThrough();
    child.exitCode = null;
    child.signalCode = null;
    const keepAlive = setInterval(() => {}, 1_000);
    child.kill = () => {
      if (child.exitCode !== null || child.signalCode !== null) return false;
      clearInterval(keepAlive);
      child.signalCode = "SIGTERM";
      onKill?.();
      child.stdout.end();
      child.stderr.end();
      child.emit("exit", null, "SIGTERM");
      return true;
    };
    queueMicrotask(() => onStart?.(child));
    return child;
  };
}

test("the desktop Runtime probe verifies latticed and reads PostgreSQL health", async () => {
  const executablePath = path.resolve("runtime", "latticed.exe");
  let observedSpawn;
  const status = await probeLatticeRuntimeEndpoint({
    executablePath,
    environment: {},
    timeoutMs: 500,
    spawnProcess: fakeRuntimeSpawn({
      onSpawn: (file, args, options) => { observedSpawn = { file, args, options }; },
    }),
  });

  assert.deepEqual(status, {
    postgresql: "HEALTHY",
    detail: "LATTICE_RUNTIME_VERIFIED",
  });
  assert.equal(observedSpawn.file, executablePath);
  assert.deepEqual(observedSpawn.args, []);
  assert.equal(observedSpawn.options.cwd, path.dirname(executablePath));
  assert.equal(observedSpawn.options.shell, false);
});

test("the configuration loader derives only the two legacy Runtime aliases in memory", async () => {
  const executablePath = path.resolve("runtime", "latticed.exe");
  const launcherPath = path.resolve("bin", "codex.exe");
  const deliveryRoot = path.resolve("delivery");
  const secret = "not-browser-visible";
  const configText = [
    "[mcp_servers.lattice]",
    `command = ${JSON.stringify(executablePath)}`,
    "[mcp_servers.lattice.env]",
    `LATTICE_HERMES_CODEX_LAUNCHER = ${JSON.stringify(launcherPath)}`,
    `LATTICE_DELIVERY_ROOT = ${JSON.stringify(deliveryRoot)}`,
    `LATTICE_TASK019_PASSWORD = ${JSON.stringify(secret)}`,
  ].join("\n");

  const configuration = await loadLatticeRuntimeConfiguration({
    configPath: path.resolve("config.toml"),
    readText: async () => configText,
    verifyExecutable: async () => {},
  });

  assert.equal(configuration.executablePath, executablePath);
  assert.equal(configuration.environment.LATTICE_DELIVERY_LAUNCHER, launcherPath);
  assert.equal(
    configuration.environment.LATTICE_DELIVERY_SCHEMA_DIR,
    path.join(deliveryRoot, "schema"),
  );
  assert.equal(configuration.environment.LATTICE_TASK019_PASSWORD, secret);
});

test("a missing configured Runtime is reported as stopped", async () => {
  const status = await probeConfiguredLatticeRuntime({
    loadConfiguration: async () => {
      const error = new Error("missing");
      error.code = "ENOENT";
      throw error;
    },
  });
  assert.deepEqual(status, {
    postgresql: "STOPPED",
    detail: "LATTICE_RUNTIME_NOT_CONFIGURED",
  });
});

test("a substituted Runtime tool catalog fails closed", async () => {
  const status = await probeLatticeRuntimeEndpoint({
    executablePath: path.resolve("latticed.exe"),
    environment: {},
    timeoutMs: 500,
    spawnProcess: fakeRuntimeSpawn({ toolNames: [...expectedTools, "unexpected_tool"] }),
  });
  assert.deepEqual(status, {
    postgresql: "INCOMPATIBLE",
    detail: "LATTICE_RUNTIME_INCOMPATIBLE",
  });
});

test("a Runtime tool failure is unreachable without leaking raw output", async () => {
  const status = await probeLatticeRuntimeEndpoint({
    executablePath: path.resolve("latticed.exe"),
    environment: { LATTICE_TASK019_PASSWORD: "must-not-leak" },
    timeoutMs: 500,
    spawnProcess: fakeRuntimeSpawn({ toolError: true }),
  });
  assert.deepEqual(status, {
    postgresql: "UNREACHABLE",
    detail: "LATTICE_RUNTIME_UNREACHABLE",
  });
  assert.doesNotMatch(JSON.stringify(status), /must-not-leak/u);
});

test("a hanging Runtime probe times out and stops only its owned child", async () => {
  let killed = 0;
  const status = await probeLatticeRuntimeEndpoint({
    executablePath: path.resolve("latticed.exe"),
    environment: {},
    timeoutMs: 100,
    spawnProcess: inertRuntimeSpawn({ onKill: () => { killed += 1; } }),
  });

  assert.deepEqual(status, {
    postgresql: "UNREACHABLE",
    detail: "LATTICE_RUNTIME_UNREACHABLE",
  });
  assert.equal(killed, 1);
});

test("malformed Runtime output fails closed as incompatible", async () => {
  const status = await probeLatticeRuntimeEndpoint({
    executablePath: path.resolve("latticed.exe"),
    environment: {},
    timeoutMs: 500,
    spawnProcess: inertRuntimeSpawn({
      onStart: (child) => child.stdout.write("not-json\n"),
    }),
  });

  assert.deepEqual(status, {
    postgresql: "INCOMPATIBLE",
    detail: "LATTICE_RUNTIME_INCOMPATIBLE",
  });
});

test("cumulative Runtime output beyond the bounded limit fails closed", async () => {
  const status = await probeLatticeRuntimeEndpoint({
    executablePath: path.resolve("latticed.exe"),
    environment: {},
    timeoutMs: 500,
    spawnProcess: inertRuntimeSpawn({
      onStart: (child) => {
        child.stdout.write(Buffer.alloc(40_000, 0x20));
        child.stdout.write(Buffer.alloc(30_000, 0x20));
      },
    }),
  });

  assert.deepEqual(status, {
    postgresql: "INCOMPATIBLE",
    detail: "LATTICE_RUNTIME_INCOMPATIBLE",
  });
});

test("the Runtime monitor shares an in-flight probe and caches its safe projection", async () => {
  let calls = 0;
  let now = 100;
  const monitor = new LatticeRuntimeHealthMonitor({
    ttlMs: 50,
    now: () => now,
    probe: async () => {
      calls += 1;
      return { ...validRuntimeStatus(), postgresql: "HEALTHY", detail: "ignored" };
    },
  });

  const [first, second] = await Promise.all([monitor.current(), monitor.current()]);
  assert.equal(calls, 1);
  assert.deepEqual(first, {
    postgresql: "HEALTHY",
    detail: "LATTICE_RUNTIME_VERIFIED",
  });
  assert.deepEqual(second, first);

  now = 151;
  await monitor.current();
  assert.equal(calls, 2);
});

test("the Runtime monitor can warm in the background without blocking the UI", async () => {
  let finishProbe;
  const monitor = new LatticeRuntimeHealthMonitor({
    probe: () => new Promise((resolve) => { finishProbe = resolve; }),
  });

  assert.deepEqual(await monitor.current({ waitForProbe: false }), {
    postgresql: "NO_DATA",
    detail: "LATTICE_RUNTIME_CHECKING",
  });
  finishProbe({ postgresql: "HEALTHY", detail: "LATTICE_RUNTIME_VERIFIED" });
  assert.deepEqual(await monitor.current(), {
    postgresql: "HEALTHY",
    detail: "LATTICE_RUNTIME_VERIFIED",
  });
});

test("closing the Runtime monitor cancels only its owned probe", async () => {
  let observedSignal;
  let ownedChildExited = false;
  const monitor = new LatticeRuntimeHealthMonitor({
    probe: ({ signal, onOwnedChild }) => new Promise((resolve) => {
      observedSignal = signal;
      const child = new EventEmitter();
      child.exitCode = null;
      child.signalCode = null;
      child.kill = () => {
        setTimeout(() => {
          child.signalCode = "SIGTERM";
          ownedChildExited = true;
          child.emit("exit", null, "SIGTERM");
        }, 20);
        return true;
      };
      onOwnedChild(child);
      signal.addEventListener("abort", () => resolve({
        postgresql: "UNREACHABLE",
        detail: "LATTICE_RUNTIME_UNREACHABLE",
      }), { once: true });
    }),
  });

  await monitor.current({ waitForProbe: false });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(observedSignal.aborted, false);
  await monitor.close();
  assert.equal(observedSignal.aborted, true);
  assert.equal(ownedChildExited, true);
  assert.deepEqual(await monitor.current(), {
    postgresql: "STOPPED",
    detail: "LATTICE_RUNTIME_NOT_CONFIGURED",
  });
});
