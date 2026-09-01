import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { createLatticeServer } from "../src/server.mjs";
import { LatticeStore } from "../src/store.mjs";

function removeDecisionCoreFromLegacyFixture(database) {
  database.exec(`
    DROP TRIGGER IF EXISTS decisions_no_delete;
    DROP TRIGGER IF EXISTS decisions_immutable_update;
    DROP TRIGGER IF EXISTS decision_state_no_delete;
    DROP TRIGGER IF EXISTS decision_state_revision_guard;
    DROP INDEX IF EXISTS decisions_current_scope_subject;
    DROP INDEX IF EXISTS decisions_unique_successor;
    DROP INDEX IF EXISTS decisions_scope_created_at;
    DROP TABLE IF EXISTS decision_state;
    DROP TABLE IF EXISTS decisions;
  `);
}

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

test("the upstream radar keeps one durable, replaceable, expiring advisory snapshot", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-radar-"));
  const databasePath = path.join(directory, "control.db");
  let application;
  try {
    application = createLatticeServer({ databasePath, codex: new QuietCodex() });
    let origin = await listen(application);
    const first = {
      observed_at: "2026-08-31T01:00:00.000Z",
      expires_at: "2099-01-01T00:00:00.000Z",
      summary: "OpenAI scheduled tasks already cover the daily scan.",
      decisions: [{
        action: "WRAP_OFFICIAL",
        subject: "Codex scheduled tasks",
        source_url: "https://learn.chatgpt.com/docs/automations",
        version_or_date: "2026-08-31",
        impact: "Keep scheduling outside LATTICE and store only this advisory snapshot.",
      }],
    };

    const recordedResponse = await fetch(`${origin}/api/development-radar`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(first),
    });
    assert.equal(recordedResponse.status, 200);
    const recorded = await recordedResponse.json();
    assert.equal(recorded.schema_version, "lattice.control.development-radar.v1");
    assert.equal(recorded.observation_kind, "UPSTREAM_DEVELOPMENT_RADAR");
    assert.equal(recorded.authority, "NON_AUTHORITATIVE");
    assert.equal(recorded.slot, "current");
    assert.equal(recorded.freshness, "CURRENT");
    assert.deepEqual(recorded.decisions, first.decisions);

    const state = await (await fetch(`${origin}/api/state`)).json();
    assert.deepEqual(state.developmentRadar, recorded);

    const replacement = {
      observed_at: "2026-09-01T01:00:00.000Z",
      expires_at: "2026-09-02T01:00:00.000Z",
      summary: "No upstream change requires LATTICE work.",
      decisions: [],
    };
    const replaced = await (await fetch(`${origin}/api/development-radar`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(replacement),
    })).json();
    assert.equal(replaced.slot, recorded.slot);
    assert.equal(replaced.summary, replacement.summary);
    assert.deepEqual(replaced.decisions, []);

    await close(application);
    application = null;
    application = createLatticeServer({ databasePath, codex: new QuietCodex() });
    origin = await listen(application);
    const replayed = await (await fetch(`${origin}/api/development-radar`)).json();
    assert.equal(replayed.observed_at, replacement.observed_at);
    assert.equal(replayed.summary, replacement.summary);

    const expired = await (await fetch(`${origin}/api/development-radar`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        observed_at: "2000-01-01T00:00:00.000Z",
        expires_at: "2000-01-02T00:00:00.000Z",
        summary: "This snapshot is stale and must not guide new work.",
        decisions: [],
      }),
    })).json();
    assert.equal(expired.freshness, "EXPIRED");
  } finally {
    if (application) await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});

test("the upstream radar rejects unsafe or unbounded advisory data", async () => {
  const application = createLatticeServer({ codex: new QuietCodex() });
  try {
    const origin = await listen(application);
    for (const payload of [
      { observed_at: "invalid", expires_at: "2099-01-01T00:00:00.000Z", summary: "x", decisions: [] },
      { observed_at: "2026-01-01T00:00:00.000Z", expires_at: "2025-01-01T00:00:00.000Z", summary: "x", decisions: [] },
      { observed_at: "2026-01-01T00:00:00.000Z", expires_at: "2099-01-01T00:00:00.000Z", summary: "x", decisions: [{ action: "AUTO_BUILD", subject: "unsafe", source_url: "https://github.com/openai/codex" }] },
      { observed_at: "2026-01-01T00:00:00.000Z", expires_at: "2099-01-01T00:00:00.000Z", summary: "x", decisions: [{ action: "ignore", subject: "unsafe", source_url: "https://github.com/openai/codex" }] },
      { observed_at: "2026-01-01T00:00:00.000Z", expires_at: "2099-01-01T00:00:00.000Z", summary: "x", decisions: [{ action: "WATCH", subject: "unsafe", source_url: "file:///secret" }] },
      { observed_at: "2026-01-01T00:00:00.000Z", expires_at: "2099-01-01T00:00:00.000Z", summary: "x", decisions: Array.from({ length: 33 }, () => ({ action: "IGNORE", subject: "bounded", source_url: "https://example.com/" })) },
    ]) {
      const response = await fetch(`${origin}/api/development-radar`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
      });
      assert.equal(response.status, 400);
    }
    const missing = await fetch(`${origin}/api/development-radar`);
    assert.equal(missing.status, 200);
    assert.equal(await missing.json(), null);
  } finally {
    await close(application);
  }
});

test("a version 1 Control database migrates in place without losing existing data", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-radar-migration-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  try {
    store = new LatticeStore(databasePath);
    store.createProject({ name: "Existing", rootPath: directory });
    store.close();
    store = null;

    const legacy = new DatabaseSync(databasePath);
    removeDecisionCoreFromLegacyFixture(legacy);
    legacy.exec("DROP TABLE development_radar; PRAGMA user_version = 1;");
    legacy.close();

    store = new LatticeStore(databasePath);
    assert.equal(store.database.prepare("PRAGMA user_version").get().user_version, 6);
    assert.equal(store.listProjects().length, 1);
    assert.equal(store.getDevelopmentRadar(), null);
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("a version 3 Control database gains a monotonic conversation fence generation", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-fence-migration-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  try {
    store = new LatticeStore(databasePath);
    store.close();
    store = null;

    const legacy = new DatabaseSync(databasePath);
    removeDecisionCoreFromLegacyFixture(legacy);
    legacy.exec(`
      DROP TABLE conversation_writer_leases;
      CREATE TABLE conversation_writer_leases (
        conversation_id TEXT PRIMARY KEY REFERENCES work_items(id) ON DELETE CASCADE
          CHECK (conversation_id = 'primary'),
        owner_id TEXT NOT NULL CHECK (length(owner_id) BETWEEN 1 AND 128),
        owner_pid INTEGER NOT NULL CHECK (owner_pid > 0),
        lease_expires_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
      PRAGMA user_version = 3;
    `);
    legacy.close();

    store = new LatticeStore(databasePath);
    assert.equal(store.database.prepare("PRAGMA user_version").get().user_version, 6);
    assert.deepEqual(
      store.database.prepare("PRAGMA table_info(conversation_writer_leases)").all()
        .map(({ name }) => name),
      ["conversation_id", "owner_id", "owner_pid", "lease_expires_at", "updated_at", "generation"],
    );
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});
