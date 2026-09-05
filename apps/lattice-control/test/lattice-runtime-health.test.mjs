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

function completedRuntimeStatus() {
  const runtimeStatus = {
    ...validRuntimeStatus(),
    component: "task-delivery-ledger",
    status: "COMPLETED",
    profile: "task032-codex-postgres-v1",
    request_id: "runtime-health-terminal",
    configuration_digest: "c".repeat(64),
    intent_digest: "d".repeat(64),
    outcome_digest: "e".repeat(64),
    receipt_digest: "f".repeat(64),
    launcher_path: "redacted-by-Control",
    version: "codex-cli-test",
    launcher_sha256: "1".repeat(64),
    schema_bundle_sha256: "2".repeat(64),
    schema_file_count: 1,
    repository_path: "redacted-by-Control",
    changed_paths: ["answer.txt"],
    test: "FIXED_TEST_PASSED",
    test_command_id: "git-diff-no-index-exact-answer-v1",
    baseline_commit: "3".repeat(40),
    parent_sha: "4".repeat(40),
    commit_sha: "5".repeat(40),
    thread_id: "thread-runtime-health",
    turn_id: "turn-runtime-health",
    codex_runtime: "OFFICIAL_CODEX_APP_SERVER",
  };
  delete runtimeStatus.scope;
  return runtimeStatus;
}

function canonicalToolDescriptor(name) {
  const descriptor = {
    name,
    title: `LATTICE ${name}`,
    description: `Bounded ${name}`,
    inputSchema: name === "lattice_runtime_status"
      ? { type: "object", additionalProperties: false }
      : { type: "object" },
  };
  if ([
    "lattice_foreman_checkpoint",
    "lattice_task_status",
    "lattice_task_submit",
  ].includes(name)) descriptor.outputSchema = { type: "object" };
  return descriptor;
}

function fakeRuntimeSpawn({
  toolNames = expectedTools,
  toolDescriptors = toolNames.map(canonicalToolDescriptor),
  runtimeStatus = validRuntimeStatus(),
  toolError = false,
  protocolVersion = "2025-11-25",
  serverInfo = { name: "latticed", title: "LATTICE DevOS", version: "1.0.0" },
  killDelayMs = 0,
  onSpawn,
  onRequest,
  onExit,
} = {}) {
  return (executablePath, args, options) => {
    onSpawn?.(executablePath, args, options);
    const child = new EventEmitter();
    child.stdin = new PassThrough();
    child.stdout = new PassThrough();
    child.stderr = new PassThrough();
    child.pid = 42_001;
    child.exitCode = null;
    child.signalCode = null;
    let buffer = "";
    child.stdin.on("data", (chunk) => {
      buffer += chunk.toString("utf8");
      while (buffer.includes("\n")) {
        const newline = buffer.indexOf("\n");
        const frame = JSON.parse(buffer.slice(0, newline));
        buffer = buffer.slice(newline + 1);
        onRequest?.(frame);
        if (frame.method === "initialize") {
          child.stdout.write(`${JSON.stringify({
            jsonrpc: "2.0",
            id: 1,
            result: {
              protocolVersion,
              capabilities: { tools: {} },
              serverInfo,
              instructions: "bounded LATTICE tools",
            },
          })}\n`);
        } else if (frame.method === "tools/list") {
          child.stdout.write(`${JSON.stringify({
            jsonrpc: "2.0",
            id: 2,
            result: { tools: toolDescriptors },
          })}\n`);
        } else if (frame.method === "tools/call") {
          const structuredContent = toolError
            ? { status: "ERROR", code: "LATTICE_RUNTIME_STATUS_UNAVAILABLE" }
            : runtimeStatus;
          child.stdout.write(`${JSON.stringify({
            jsonrpc: "2.0",
            id: 3,
            result: {
              content: [{ type: "text", text: JSON.stringify(structuredContent) }],
              structuredContent,
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
      const exit = () => {
        child.signalCode = "SIGTERM";
        child.stdout.end();
        child.stderr.end();
        onExit?.();
        child.emit("exit", null, "SIGTERM");
      };
      if (killDelayMs > 0) setTimeout(exit, killDelayMs);
      else exit();
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
    child.pid = 42_002;
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
  const observedRequests = [];
  const status = await probeLatticeRuntimeEndpoint({
    executablePath,
    environment: {},
    timeoutMs: 500,
    spawnProcess: fakeRuntimeSpawn({
      onSpawn: (file, args, options) => { observedSpawn = { file, args, options }; },
      onRequest: (request) => { observedRequests.push(request); },
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
  assert.deepEqual(observedRequests.filter(({ method }) => method === "tools/call"), [{
    jsonrpc: "2.0",
    id: 3,
    method: "tools/call",
    params: { name: "lattice_runtime_status", arguments: {} },
  }]);
});

test("formal PostgreSQL health remains verified when delivery already has a terminal receipt", async () => {
  const status = await probeLatticeRuntimeEndpoint({
    executablePath: path.resolve("runtime", "latticed.exe"),
    environment: {},
    timeoutMs: 500,
    spawnProcess: fakeRuntimeSpawn({ runtimeStatus: completedRuntimeStatus() }),
  });

  assert.deepEqual(status, {
    postgresql: "HEALTHY",
    detail: "LATTICE_RUNTIME_VERIFIED",
  });
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

test("wrong Runtime protocol and server identity fail closed", async (context) => {
  for (const fixture of [
    { label: "protocol", options: { protocolVersion: "2024-11-05" } },
    {
      label: "server",
      options: {
        serverInfo: { name: "substituted-runtime", title: "LATTICE DevOS", version: "1.0.0" },
      },
    },
  ]) {
    await context.test(fixture.label, async () => {
      const status = await probeLatticeRuntimeEndpoint({
        executablePath: path.resolve("latticed.exe"),
        environment: {},
        timeoutMs: 500,
        spawnProcess: fakeRuntimeSpawn(fixture.options),
      });
      assert.deepEqual(status, {
        postgresql: "INCOMPATIBLE",
        detail: "LATTICE_RUNTIME_INCOMPATIBLE",
      });
    });
  }
});

test("a same-name catalog with a non-zero-argument Runtime status tool fails closed", async () => {
  const toolDescriptors = expectedTools.map(canonicalToolDescriptor);
  const runtimeStatus = toolDescriptors.find(({ name }) => name === "lattice_runtime_status");
  runtimeStatus.inputSchema = {
    type: "object",
    properties: { task_ref: { type: "string" } },
    required: ["task_ref"],
    additionalProperties: false,
  };

  const status = await probeLatticeRuntimeEndpoint({
    executablePath: path.resolve("latticed.exe"),
    environment: {},
    timeoutMs: 500,
    spawnProcess: fakeRuntimeSpawn({ toolDescriptors }),
  });
  assert.deepEqual(status, {
    postgresql: "INCOMPATIBLE",
    detail: "LATTICE_RUNTIME_INCOMPATIBLE",
  });
});

test("all seven tool descriptors require a plain input schema", async () => {
  const toolDescriptors = expectedTools.map(canonicalToolDescriptor);
  delete toolDescriptors[0].inputSchema;

  const status = await probeLatticeRuntimeEndpoint({
    executablePath: path.resolve("latticed.exe"),
    environment: {},
    timeoutMs: 500,
    spawnProcess: fakeRuntimeSpawn({ toolDescriptors }),
  });
  assert.equal(status.postgresql, "INCOMPATIBLE");
});

test("hostile direct Runtime and Foreman projection values fail closed", async (context) => {
  const fixtures = [
    ["integration", (value) => { value.runtime_integration = "UNKNOWN"; }],
    ["Graphify correlation", (value) => { value.graphify_runtime_status = "DEFERRED"; }],
    ["Hermes correlation", (value) => { value.hermes_runtime_status = "DEFERRED"; }],
    ["Hermes activation", (value) => { value.hermes_activation_status = "UNKNOWN"; }],
    ["schema", (value) => { value.foreman.schema = "lattice.foreman-runtime-projection/9.9"; }],
    ["replay", (value) => { value.foreman.replay_status = "UNKNOWN"; }],
    ["checkpoint correlation", (value) => { value.foreman.checkpoint_digest = null; }],
    ["next action", (value) => { value.foreman.next_action = "UNKNOWN"; }],
    ["degraded code", (value) => { value.foreman.degraded_code = "UNKNOWN"; }],
    ["negative count", (value) => { value.foreman.completed_count = -1; }],
    ["ledger digest", (value) => { value.foreman.ledger_digest = "not-a-digest"; }],
  ];

  for (const [label, mutate] of fixtures) {
    await context.test(label, async () => {
      const runtimeStatus = structuredClone(validRuntimeStatus());
      mutate(runtimeStatus);
      const status = await probeLatticeRuntimeEndpoint({
        executablePath: path.resolve("latticed.exe"),
        environment: {},
        timeoutMs: 500,
        spawnProcess: fakeRuntimeSpawn({ runtimeStatus }),
      });
      assert.deepEqual(status, {
        postgresql: "INCOMPATIBLE",
        detail: "LATTICE_RUNTIME_INCOMPATIBLE",
      });
    });
  }
});

test("optional Runtime module degradation does not mask verified PostgreSQL", async () => {
  const runtimeStatus = validRuntimeStatus();
  runtimeStatus.graphify_runtime_status = "DEGRADED";
  runtimeStatus.hermes_runtime_status = "DEGRADED";
  runtimeStatus.hermes_activation_status = "CONFIGURATION_REJECTED";
  const status = await probeLatticeRuntimeEndpoint({
    executablePath: path.resolve("latticed.exe"),
    environment: {},
    timeoutMs: 500,
    spawnProcess: fakeRuntimeSpawn({ runtimeStatus }),
  });
  assert.equal(status.postgresql, "HEALTHY");
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

test("the Runtime probe does not report completion until its owned child exits", async () => {
  let ownedChildExited = false;
  const startedAt = performance.now();
  const status = await probeLatticeRuntimeEndpoint({
    executablePath: path.resolve("latticed.exe"),
    environment: {},
    timeoutMs: 500,
    spawnProcess: fakeRuntimeSpawn({
      killDelayMs: 40,
      onExit: () => { ownedChildExited = true; },
    }),
  });

  assert.deepEqual(status, {
    postgresql: "HEALTHY",
    detail: "LATTICE_RUNTIME_VERIFIED",
  });
  assert.equal(ownedChildExited, true);
  assert.ok(performance.now() - startedAt >= 30);
});

test("the Runtime child receives configured values but not unrelated inherited secrets", async () => {
  const inheritedValues = {
    SPEC012_PARENT_API_TOKEN: "parent-only-test-value",
    HTTPS_PROXY: "http://private-proxy.invalid",
    PRIVATE_KEY: "private-key-test-value",
    PGSERVICE: "unrelated-postgres-service",
  };
  const configuredName = "LATTICE_SPEC012_TEST_PASSWORD";
  const previous = Object.fromEntries(
    Object.keys(inheritedValues).map((name) => [name, process.env[name]]),
  );
  Object.assign(process.env, inheritedValues);
  let observedEnvironment;
  try {
    await probeLatticeRuntimeEndpoint({
      executablePath: path.resolve("latticed.exe"),
      environment: { [configuredName]: "configured-test-value" },
      timeoutMs: 500,
      spawnProcess: fakeRuntimeSpawn({
        onSpawn: (_file, _args, options) => { observedEnvironment = options.env; },
      }),
    });
  } finally {
    for (const [name, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  }

  for (const name of Object.keys(inheritedValues)) {
    assert.equal(Object.hasOwn(observedEnvironment, name), false, `${name} must stay parent-only`);
  }
  assert.equal(observedEnvironment[configuredName], "configured-test-value");
  assert.equal(observedEnvironment.NO_COLOR, "1");
  if (process.env.SystemRoot) assert.equal(observedEnvironment.SystemRoot, process.env.SystemRoot);
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

test("a retained owned child blocks another probe after the cache expires", async () => {
  let calls = 0;
  let now = 100;
  let ownedChild;
  const monitor = new LatticeRuntimeHealthMonitor({
    ttlMs: 1,
    now: () => now,
    probe: async ({ onOwnedChild }) => {
      calls += 1;
      const child = new EventEmitter();
      child.exitCode = null;
      child.signalCode = null;
      child.kill = () => {
        child.signalCode = "SIGTERM";
        child.emit("exit", null, "SIGTERM");
        return true;
      };
      ownedChild = child;
      onOwnedChild(child);
      return { postgresql: "UNREACHABLE", detail: "LATTICE_RUNTIME_UNREACHABLE" };
    },
  });

  assert.equal((await monitor.current()).postgresql, "UNREACHABLE");
  now = 102;
  assert.equal((await monitor.current()).postgresql, "UNREACHABLE");
  assert.equal(calls, 1);
  await monitor.close();
  assert.notEqual(ownedChild.signalCode, null);
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

test("monitor shutdown fails closed when an owned child cannot prove exit", async () => {
  let ownedChild;
  const monitor = new LatticeRuntimeHealthMonitor({
    cleanupTimeoutMs: 50,
    probe: ({ signal, onOwnedChild }) => new Promise((resolve) => {
      const child = new EventEmitter();
      child.exitCode = null;
      child.signalCode = null;
      child.on("error", () => {});
      child.kill = () => {
        queueMicrotask(() => {
          child.emit("error", new Error("test kill failure"));
          child.emit("close", null, null);
        });
        return false;
      };
      ownedChild = child;
      onOwnedChild(child);
      signal.addEventListener("abort", () => resolve({
        postgresql: "UNREACHABLE",
        detail: "LATTICE_RUNTIME_UNREACHABLE",
      }), { once: true });
    }),
  });

  await monitor.current({ waitForProbe: false });
  await new Promise((resolve) => setImmediate(resolve));
  const keepAlive = setInterval(() => {}, 1_000);
  try {
    await assert.rejects(
      monitor.close(),
      (error) => error?.code === "LATTICE_RUNTIME_PROBE_CLEANUP_TIMEOUT",
    );
  } finally {
    clearInterval(keepAlive);
    ownedChild.exitCode = 1;
    ownedChild.emit("exit", 1, null);
  }
});
