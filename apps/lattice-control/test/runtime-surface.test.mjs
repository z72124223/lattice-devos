import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

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

test("the runtime probe exposes one versioned, content-free capability surface", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-runtime-surface-"));
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
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
      "health",
      "capabilities",
    ]);
    assert.equal(surface.schema_version, "lattice.control.runtime-surface.v1");
    assert.deepEqual(surface.identity, {
      schema_version: "lattice.control.runtime-identity.v1",
      product: "LATTICE_CONTROL",
      version: "1.0.0-rc.1",
    });
    assert.equal(surface.health, "HEALTHY");
    assert.deepEqual(surface.capabilities, [
      { id: "control_sqlite", label: "Control／SQLite", status: "HEALTHY" },
      { id: "codex_app_server", label: "Codex App Server", status: "STOPPED" },
      { id: "work_mcp", label: "Work MCP", status: "NO_DATA" },
      { id: "decision_mcp", label: "Decision MCP", status: "NO_DATA" },
      { id: "postgresql", label: "正式 PostgreSQL", status: "NOT_IMPLEMENTED" },
    ]);

    const serialized = JSON.stringify(surface);
    assert.doesNotMatch(serialized, /chat|message|prompt|secret|token|reasoning/iu);
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
  } finally {
    await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});
