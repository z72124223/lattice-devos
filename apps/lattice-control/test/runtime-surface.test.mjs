import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import { controlDataScopeDescriptor } from "../src/database-path.mjs";
import { ControlMcpHealthMonitor, probeBundledControlMcps } from "../src/mcp-health.mjs";
import { createLatticeServer } from "../src/server.mjs";

class QuietCodex extends EventEmitter {
  connected = false;

  async close() {
    this.connected = false;
  }
}

async function listen(application) {
  await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
  const { port } = application.server.address();
  return `http://127.0.0.1:${port}`;
}

async function close(application) {
  await new Promise((resolve, reject) => {
    application.server.close((error) => (error ? reject(error) : resolve()));
  });
}

test("the Control data scope is path-bound without exposing its absolute path", () => {
  const firstPath = path.resolve("C:/scope-a/LATTICE/control/lattice-control.db");
  const samePath = path.resolve("C:/scope-a/LATTICE/control/./lattice-control.db");
  const otherPath = path.resolve("C:/scope-b/LATTICE/control/lattice-control.db");
  const first = controlDataScopeDescriptor(firstPath);
  const same = controlDataScopeDescriptor(samePath);
  const other = controlDataScopeDescriptor(otherPath);

  assert.deepEqual(Object.keys(first), [
    "schema_version",
    "store",
    "store_schema_version",
    "authority_class",
    "registry_authority",
    "digest",
  ]);
  assert.equal(first.schema_version, "lattice.control.data-scope.v1");
  assert.equal(first.store, "CONTROL_SQLITE");
  assert.equal(first.store_schema_version, 7);
  assert.equal(first.authority_class, "CONTROL_LOCAL_PRODUCT_STATE");
  assert.equal(first.registry_authority, "NONE");
  assert.match(first.digest, /^[a-f0-9]{64}$/u);
  assert.equal(first.digest, same.digest);
  assert.notEqual(first.digest, other.digest);
  assert.doesNotMatch(JSON.stringify(first), /scope-a|lattice-control\.db|[A-Z]:[\\/]/iu);
  if (process.platform === "win32") {
    assert.equal(
      controlDataScopeDescriptor("C:\\LATTICE\\資料\\控制.db").digest,
      "6abca5698e2c85cf0e0de89a8bc7b1adfafb502b61999164c897da31346e4976",
      "Node data-scope digest drifted from the shared non-ASCII .NET fixture",
    );
  }
});

test("the runtime probe exposes one versioned, content-free capability surface", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-runtime-surface-"));
  const databasePath = path.join(directory, "control.db");
  const application = createLatticeServer({
    databasePath,
    codex: new QuietCodex(),
  });
  try {
    const origin = await listen(application);
    const response = await fetch(`${origin}/api/runtime`);
    assert.equal(response.status, 200);
    assert.match(response.headers.get("content-type") ?? "", /^application\/json\b/u);

    const surface = await response.json();
    assert.deepEqual(Object.keys(surface), [
      "schema_version",
      "identity",
      "data_scope",
      "reconciliation_required",
      "health",
      "capabilities",
    ]);
    assert.equal(surface.schema_version, "lattice.control.runtime-surface.v2");
    assert.deepEqual(surface.identity, {
      schema_version: "lattice.control.runtime-identity.v1",
      product: "LATTICE_CONTROL",
      version: "1.0.0",
    });
    assert.deepEqual(surface.data_scope, controlDataScopeDescriptor(databasePath));
    assert.equal(surface.reconciliation_required, false);
    assert.equal(surface.health, "HEALTHY");
    assert.deepEqual(surface.capabilities, [
      { id: "control_sqlite", label: "Control／SQLite", status: "HEALTHY", has_data: null },
      { id: "codex_app_server", label: "Codex App Server", status: "STOPPED", has_data: null },
      { id: "work_mcp", label: "Work MCP", status: "HEALTHY", has_data: false },
      { id: "decision_mcp", label: "Decision MCP", status: "HEALTHY", has_data: false },
      { id: "postgresql", label: "正式 PostgreSQL", status: "NOT_IMPLEMENTED", has_data: null },
    ]);

    const serialized = JSON.stringify(surface);
    assert.doesNotMatch(serialized, /chat|message|prompt|secret|token|reasoning/iu);
    assert.doesNotMatch(serialized, new RegExp(directory.replace(/[\\^$.*+?()[\]{}|]/gu, "\\$&"), "iu"));
  } finally {
    await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});

test("MCP availability is independent from SQLite data presence", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-runtime-mcp-health-"));
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex: new QuietCodex(),
    mcpHealth: {
      current: async () => ({
        work_mcp: "UNREACHABLE",
        decision_mcp: "INCOMPATIBLE",
      }),
    },
  });
  try {
    const origin = await listen(application);
    const surface = await (await fetch(`${origin}/api/runtime`)).json();
    const work = surface.capabilities.find(({ id }) => id === "work_mcp");
    const decision = surface.capabilities.find(({ id }) => id === "decision_mcp");
    assert.deepEqual(work, {
      id: "work_mcp",
      label: "Work MCP",
      status: "UNREACHABLE",
      has_data: false,
    });
    assert.deepEqual(decision, {
      id: "decision_mcp",
      label: "Decision MCP",
      status: "INCOMPATIBLE",
      has_data: false,
    });
  } finally {
    await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});

test("runtime data presence covers the whole SQLite scope when project context is ambiguous", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-runtime-scope-data-"));
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex: new QuietCodex(),
    mcpHealth: {
      current: async () => ({ work_mcp: "HEALTHY", decision_mcp: "HEALTHY" }),
    },
  });
  try {
    const first = application.service.createProject({
      name: "First project",
      rootPath: path.join(directory, "first"),
    });
    application.service.createProject({
      name: "Second project",
      rootPath: path.join(directory, "second"),
    });
    application.service.createWorkItem({
      projectId: first.id,
      title: "Existing scoped work",
      objective: "Data presence must not depend on the selected four-core project.",
    });
    const decisionState = application.store.getCurrentDecisionsPacket({
      scope: first.id,
      limit: 1,
    });
    application.store.recordDecision({
      scope: first.id,
      subject: "runtime.data-presence",
      content: "Use the complete Control SQLite scope for existence checks.",
      rationale: "Project selection is a view concern, not data ownership.",
      source: {
        kind: "user_confirmation",
        reference: "thread:runtime-surface/turn:scope-data",
      },
      clientRequestId: "runtime-scope-data-001",
      expectedRevision: decisionState.revision,
      expectedDigest: decisionState.digest,
    });

    const origin = await listen(application);
    const surface = await (await fetch(`${origin}/api/runtime`)).json();
    assert.equal(application.service.fourCoreSurface().context.status, "not_ready");
    assert.equal(surface.capabilities.find(({ id }) => id === "work_mcp").has_data, true);
    assert.equal(surface.capabilities.find(({ id }) => id === "decision_mcp").has_data, true);
  } finally {
    await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});

test("the synthetic primary conversation is not Work MCP data", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-runtime-primary-only-"));
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex: new QuietCodex(),
    mcpHealth: {
      current: async () => ({ work_mcp: "HEALTHY", decision_mcp: "HEALTHY" }),
    },
  });
  try {
    const project = application.service.createProject({
      name: "Conversation only",
      rootPath: directory,
    });
    application.store.ensurePrimaryConversation(project.id);
    const origin = await listen(application);
    const surface = await (await fetch(`${origin}/api/runtime`)).json();
    assert.equal(application.store.listWorkItems().length, 1);
    assert.equal(application.store.listWorkItems()[0].id, "primary");
    assert.equal(surface.capabilities.find(({ id }) => id === "work_mcp").has_data, false);
    assert.equal(surface.capabilities.find(({ id }) => id === "decision_mcp").has_data, false);
  } finally {
    await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});

test("a cold hanging MCP probe degrades only that capability within the desktop deadline", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-runtime-hung-mcp-"));
  const databasePath = path.join(directory, "control.db");
  const monitor = new ControlMcpHealthMonitor({
    databasePath,
    ttlMs: 0,
    probe: ({ databasePath: probedPath }) => probeBundledControlMcps({
      databasePath: probedPath,
      timeoutMs: 300,
      probeEndpoint: async ({ expectedName, timeoutMs }) => {
        if (expectedName.includes("work")) {
          await new Promise((resolve) => setTimeout(resolve, timeoutMs));
          return "UNREACHABLE";
        }
        return "HEALTHY";
      },
    }),
  });
  const application = createLatticeServer({
    databasePath,
    codex: new QuietCodex(),
    mcpHealth: monitor,
  });
  try {
    const origin = await listen(application);
    const startedAt = performance.now();
    const surface = await (await fetch(`${origin}/api/runtime`)).json();
    const elapsedMs = performance.now() - startedAt;
    assert.ok(elapsedMs < 1_500, `cold runtime probe exceeded desktop budget: ${elapsedMs}ms`);
    assert.equal(surface.health, "HEALTHY");
    assert.equal(surface.capabilities.find(({ id }) => id === "work_mcp").status, "UNREACHABLE");
    assert.equal(surface.capabilities.find(({ id }) => id === "decision_mcp").status, "HEALTHY");
  } finally {
    await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});

test("the four-core page renders the runtime capability list without adding a fifth core", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-runtime-page-"));
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex: new QuietCodex(),
  });
  try {
    const origin = await listen(application);
    const page = await (await fetch(`${origin}/`)).text();
    assert.equal(page.match(/data-core-target=/gu)?.length, 4);
    assert.match(page, /id="runtime-capabilities"/u);
    assert.match(page, /api\("\/api\/runtime"\)/u);
    for (const status of [
      "HEALTHY",
      "NOT_IMPLEMENTED",
      "STOPPED",
      "UNREACHABLE",
      "INCOMPATIBLE",
      "NO_DATA",
    ]) {
      assert.match(page, new RegExp(`\\["${status}"`, "u"));
      assert.match(page, new RegExp(`data-runtime-status="${status}"`, "u"));
    }
    assert.match(page, /capability\.has_data===false/u);
    assert.match(page, /runtimeDataStatus="NO_DATA"/u);
    assert.match(page, /element\("em","NO_DATA","runtime-data"\)/u);
    assert.match(page, /reconciliation_required!==true/u);
    assert.match(page, /reconciliation required/u);
  } finally {
    await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});
