import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { ControlDecisionService } from "../src/decision-core-service.mjs";
import { LatticeStore } from "../src/store.mjs";

const source = Object.freeze({
  kind: "user_confirmation",
  reference: "thread:decision-core/turn:confirmed-1",
});

test("Control schema v5 migrates durable decisions without losing existing product data", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-decision-migration-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  try {
    store = new LatticeStore(databasePath);
    const project = store.createProject({ name: "Existing product", rootPath: directory });
    const item = store.createWorkItem({
      projectId: project.id,
      title: "Existing work",
      objective: "Survive the decision schema migration.",
    });
    store.close();
    store = null;

    const legacy = new DatabaseSync(databasePath);
    legacy.exec(`
      DROP TRIGGER IF EXISTS decisions_no_delete;
      DROP TRIGGER IF EXISTS decisions_immutable_update;
      DROP TRIGGER IF EXISTS decision_state_no_delete;
      DROP TRIGGER IF EXISTS decision_state_revision_guard;
      DROP INDEX IF EXISTS decisions_current_scope_subject;
      DROP INDEX IF EXISTS decisions_unique_successor;
      DROP INDEX IF EXISTS decisions_scope_created_at;
      DROP TABLE IF EXISTS decision_state;
      DROP TABLE IF EXISTS decisions;
      PRAGMA user_version = 5;
    `);
    legacy.close();

    store = new LatticeStore(databasePath);
    const service = new ControlDecisionService({ store });
    const current = service.current({ scope: "project:migration", limit: 10 });
    assert.equal(store.database.prepare("PRAGMA user_version").get().user_version, 7);
    assert.equal(store.getWorkItem(item.id).objective, "Survive the decision schema migration.");
    assert.equal(current.revision, 0);
    assert.match(current.digest, /^[a-f0-9]{64}$/u);
    assert.deepEqual(current.decisions, []);
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("schema v5 migration rejects pre-existing decision-owned objects without adopting them", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-decision-polluted-migration-"));
  const databasePath = path.join(directory, "control.db");
  try {
    const current = new LatticeStore(databasePath);
    current.close();
    const legacy = new DatabaseSync(databasePath);
    legacy.exec(`
      DROP TRIGGER IF EXISTS decisions_no_delete;
      DROP TRIGGER IF EXISTS decisions_immutable_update;
      DROP TRIGGER IF EXISTS decision_state_no_delete;
      DROP TRIGGER IF EXISTS decision_state_revision_guard;
      DROP INDEX IF EXISTS decisions_current_scope_subject;
      DROP INDEX IF EXISTS decisions_unique_successor;
      DROP INDEX IF EXISTS decisions_scope_created_at;
      DROP TABLE IF EXISTS decision_state;
      DROP TABLE IF EXISTS decisions;
      CREATE TABLE decisions (id TEXT PRIMARY KEY, untrusted_payload TEXT);
      INSERT INTO decisions (id, untrusted_payload) VALUES ('polluted', 'must-not-be-adopted');
      PRAGMA user_version = 5;
    `);
    legacy.close();

    assert.throws(
      () => new LatticeStore(databasePath),
      /Control database schema profile mismatch: legacy decision core objects: decisions/u,
    );
    const unchanged = new DatabaseSync(databasePath);
    try {
      assert.equal(unchanged.prepare("PRAGMA user_version").get().user_version, 5);
      assert.deepEqual(
        unchanged.prepare("PRAGMA table_info(decisions)").all().map(({ name }) => name),
        ["id", "untrusted_payload"],
      );
      assert.equal(unchanged.prepare("SELECT COUNT(*) AS count FROM decisions").get().count, 1);
    } finally {
      unchanged.close();
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("schema v0 initialization rejects decision rows without trusted mutation provenance", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-decision-polluted-v0-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  try {
    store = new LatticeStore(databasePath);
    const service = new ControlDecisionService({ store });
    const initial = service.current({ scope: "product:lattice", limit: 10 });
    service.record({
      scope: "product:lattice",
      subject: "untrusted.bootstrap",
      content: "This row must not be adopted through schema initialization.",
      rationale: "Only the bounded mutation path may mint durable decision state.",
      source,
      clientRequestId: "decision-polluted-v0",
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    });
    store.close();
    store = null;

    const polluted = new DatabaseSync(databasePath);
    polluted.exec(`
      DROP TRIGGER decisions_no_delete;
      DROP TRIGGER decisions_immutable_update;
      DROP TRIGGER decision_state_no_delete;
      DROP TRIGGER decision_state_revision_guard;
      DROP INDEX decisions_current_scope_subject;
      DROP INDEX decisions_unique_successor;
      DROP INDEX decisions_scope_created_at;
      DROP TABLE decision_state;
      PRAGMA user_version = 0;
    `);
    polluted.close();

    assert.throws(
      () => new LatticeStore(databasePath),
      /Control database schema profile mismatch: legacy decision core objects: decisions/u,
    );
    const unchanged = new DatabaseSync(databasePath);
    try {
      assert.equal(unchanged.prepare("PRAGMA user_version").get().user_version, 0);
      assert.equal(unchanged.prepare("SELECT COUNT(*) AS count FROM decisions").get().count, 1);
      assert.equal(
        unchanged.prepare("SELECT COUNT(*) AS count FROM sqlite_master WHERE name = 'decision_state'")
          .get().count,
        0,
      );
    } finally {
      unchanged.close();
    }
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("an explicit decision is bounded, current, revision-bound, and request-idempotent", () => {
  const store = new LatticeStore();
  const service = new ControlDecisionService({ store });
  try {
    const initial = service.current({ scope: "product:lattice", limit: 10 });
    const input = {
      scope: "product:lattice",
      subject: "execution.adapter",
      content: "Ordinary work uses disposable codex exec workers.",
      rationale: "Interactive App Server sessions are reserved for lifecycle control.",
      source,
      clientRequestId: "decision-record-1",
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    };
    const recorded = service.record(input);
    assert.equal(recorded.changed, true);
    assert.equal(recorded.revision, 1);
    assert.match(recorded.digest, /^[a-f0-9]{64}$/u);
    assert.match(recorded.decision.id, /^[a-f0-9-]{36}$/u);
    assert.equal(recorded.decision.status, "current");
    assert.equal(recorded.decision.supersedes_decision_id, null);
    assert.equal(Object.hasOwn(recorded.decision, "client_request_id"), false);
    assert.deepEqual(recorded.decision.source, source);
    assert.match(recorded.decision.created_at, /^\d{4}-\d{2}-\d{2}T/u);

    const replay = service.record(input);
    assert.equal(replay.changed, false);
    assert.equal(replay.decision.id, recorded.decision.id);
    assert.equal(replay.revision, recorded.revision);
    assert.equal(replay.digest, recorded.digest);

    const current = service.current({ scope: "product:lattice", limit: 10 });
    assert.equal(current.schema_version, "lattice.control.current-decisions-packet.v1");
    assert.deepEqual(current.source, {
      kind: "CONTROL_SQLITE_DECISIONS",
      authority: "CONTROL_LOCAL_PRODUCT_STATE",
    });
    assert.equal(current.revision, recorded.revision);
    assert.equal(current.digest, recorded.digest);
    assert.deepEqual(current.decisions.map(({ id }) => id), [recorded.decision.id]);

    assert.throws(
      () => service.record({ ...input, content: "A conflicting replay." }),
      (error) => error.code === "DECISION_IDEMPOTENCY_CONFLICT",
    );
    assert.throws(
      () => service.record({
        ...input,
        expectedRevision: Number.MAX_SAFE_INTEGER,
        expectedDigest: "f".repeat(64),
      }),
      (error) => error.code === "DECISION_IDEMPOTENCY_CONFLICT",
    );
    assert.throws(
      () => service.record({
        ...input,
        clientRequestId: "decision-record-2",
        expectedRevision: recorded.revision,
        expectedDigest: recorded.digest,
      }),
      (error) => error.code === "DECISION_CURRENT_EXISTS",
    );
    assert.throws(
      () => service.record({
        ...input,
        subject: "another.subject",
        clientRequestId: "decision-record-3",
        expectedRevision: 0,
      }),
      (error) => error.code === "DECISION_REVISION_MISMATCH",
    );
  } finally {
    store.close();
  }
});

test("supersession retains history and moves the only current decision atomically", () => {
  const store = new LatticeStore();
  const service = new ControlDecisionService({ store });
  try {
    const initial = service.current({ scope: "product:lattice", limit: 10 });
    const first = service.record({
      scope: "product:lattice",
      subject: "worker.model",
      content: "Engineering workers use gpt-5.6-terra by default.",
      rationale: "Use the balanced model for routine bounded implementation.",
      source,
      clientRequestId: "decision-lineage-1",
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    });
    const replacementInput = {
      scope: "product:lattice",
      subject: "worker.model",
      content: "Engineering workers use the currently configured balanced model.",
      rationale: "Keep the durable decision independent from a replaceable model release name.",
      source: {
        kind: "approved_document",
        reference: "file:AGENTS.md#authorized-models",
      },
      supersedesDecisionId: first.decision.id,
      clientRequestId: "decision-lineage-2",
      expectedRevision: first.revision,
      expectedDigest: first.digest,
    };
    const replacement = service.record(replacementInput);
    assert.equal(replacement.changed, true);
    assert.equal(replacement.revision, 2);
    assert.equal(replacement.decision.status, "current");
    assert.equal(replacement.decision.supersedes_decision_id, first.decision.id);
    assert.deepEqual(
      service.current({ scope: "product:lattice", limit: 10 }).decisions.map(({ id }) => id),
      [replacement.decision.id],
    );
    assert.deepEqual(
      store.database.prepare(`
        SELECT id, status FROM decisions
        WHERE scope = ? AND subject = ? ORDER BY created_at, id
      `).all("product:lattice", "worker.model").map(({ id, status }) => ({ id, status })),
      [
        { id: first.decision.id, status: "superseded" },
        { id: replacement.decision.id, status: "current" },
      ],
    );

    const replay = service.record(replacementInput);
    assert.equal(replay.changed, false);
    assert.equal(replay.decision.id, replacement.decision.id);

    assert.throws(
      () => service.record({
        ...replacementInput,
        clientRequestId: "decision-lineage-3",
        expectedRevision: replacement.revision,
        expectedDigest: replacement.digest,
      }),
      (error) => error.code === "DECISION_SUPERSESSION_TARGET_NOT_CURRENT",
    );
    assert.throws(
      () => service.record({
        ...replacementInput,
        scope: "product:other",
        clientRequestId: "decision-lineage-4",
        expectedRevision: replacement.revision,
        expectedDigest: replacement.digest,
      }),
      (error) => error.code === "DECISION_CROSS_SCOPE_SUPERSESSION_REJECTED",
    );
    assert.throws(
      () => service.record({
        ...replacementInput,
        supersedesDecisionId: "00000000-0000-4000-8000-000000000000",
        clientRequestId: "decision-lineage-5",
        expectedRevision: replacement.revision,
        expectedDigest: replacement.digest,
      }),
      (error) => error.code === "DECISION_SUPERSESSION_TARGET_NOT_FOUND",
    );
  } finally {
    store.close();
  }
});

test("read and search are bounded and bound to the same verifiable decision state", () => {
  const store = new LatticeStore();
  const service = new ControlDecisionService({ store });
  try {
    const initial = service.current({ scope: "product:lattice", limit: 10 });
    const first = service.record({
      scope: "product:lattice",
      subject: "worker.lifecycle",
      content: "Ordinary workers are disposable after they return bounded evidence.",
      rationale: "LATTICE retains durable state while execution adapters remain replaceable.",
      source,
      clientRequestId: "decision-query-1",
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    });
    const second = service.record({
      scope: "product:lattice",
      subject: "worker.lifecycle",
      content: "Ordinary workers are disposable and the foreman owns continuation.",
      rationale: "One long-lived front end can replace short-lived execution workers.",
      source,
      supersedesDecisionId: first.decision.id,
      clientRequestId: "decision-query-2",
      expectedRevision: first.revision,
      expectedDigest: first.digest,
    });
    const third = service.record({
      scope: "product:lattice",
      subject: "privacy.boundary",
      content: "Decision memory stores explicit decisions, not complete chat transcripts.",
      rationale: "Durable product memory must stay bounded and intentional.",
      source,
      clientRequestId: "decision-query-3",
      expectedRevision: second.revision,
      expectedDigest: second.digest,
    });
    const current = service.current({ scope: "product:lattice", limit: 10 });

    const history = service.read({
      decisionId: first.decision.id,
      maxDepth: 10,
      expectedRevision: current.revision,
      expectedDigest: current.digest,
    });
    assert.equal(history.schema_version, "lattice.control.decision-read.v1");
    assert.equal(history.revision, current.revision);
    assert.equal(history.digest, current.digest);
    assert.equal(history.decision.id, first.decision.id);
    assert.deepEqual(
      history.lineage.map(({ id, status }) => ({ id, status })),
      [
        { id: first.decision.id, status: "superseded" },
        { id: second.decision.id, status: "current" },
      ],
    );
    assert.equal(history.truncated_before, false);
    assert.equal(history.truncated_after, false);

    const one = service.read({
      decisionId: second.decision.id,
      maxDepth: 1,
      expectedRevision: current.revision,
      expectedDigest: current.digest,
    });
    assert.deepEqual(one.lineage.map(({ id }) => id), [second.decision.id]);
    assert.equal(one.truncated_before, true);
    assert.equal(one.truncated_after, false);

    const search = service.search({
      scope: "product:lattice",
      query: "disposable",
      limit: 5,
      expectedRevision: current.revision,
      expectedDigest: current.digest,
    });
    assert.equal(search.schema_version, "lattice.control.decision-search.v1");
    assert.equal(search.revision, current.revision);
    assert.equal(search.digest, current.digest);
    assert.deepEqual(
      new Set(search.decisions.map(({ id }) => id)),
      new Set([first.decision.id, second.decision.id]),
    );
    assert.ok(search.decisions.every(({ scope }) => scope === "product:lattice"));
    assert.equal(search.decisions.some(({ id }) => id === third.decision.id), false);

    assert.throws(
      () => service.search({
        scope: "product:lattice",
        query: "worker",
        expectedRevision: current.revision,
        expectedDigest: current.digest,
      }),
      (error) => error.code === "DECISION_QUERY_REJECTED",
    );
    assert.throws(
      () => service.read({
        decisionId: first.decision.id,
        maxDepth: 65,
        expectedRevision: current.revision,
        expectedDigest: current.digest,
      }),
      (error) => error.code === "DECISION_QUERY_REJECTED",
    );
    assert.throws(
      () => service.read({
        decisionId: first.decision.id,
        maxDepth: 10,
        expectedRevision: current.revision - 1,
        expectedDigest: current.digest,
      }),
      (error) => error.code === "DECISION_REVISION_MISMATCH",
    );
  } finally {
    store.close();
  }
});

test("decision inputs reject oversized, secret-like, transcript, reasoning, and environment material", () => {
  const store = new LatticeStore();
  const service = new ControlDecisionService({ store });
  try {
    const initial = service.current({ scope: "product:lattice", limit: 10 });
    const syntheticSecret = "z".repeat(24);
    const base = {
      scope: "product:lattice",
      subject: "privacy.boundary",
      content: "Only explicit confirmed decisions are durable.",
      rationale: "Conversation and execution context remain outside decision memory.",
      source,
      clientRequestId: "decision-private-1",
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    };
    for (const mutation of [
      { content: "x".repeat(4_097) },
      { content: "password=hunter2 must never be retained" },
      { rationale: "Persist hidden reasoning for later analysis." },
      { source: { ...source, reference: `thread:${"x".repeat(600)}` } },
      { source: { kind: "verified_evidence", reference: "evidence:unconfirmed" } },
      { source: { kind: "user_confirmation", reference: "file:not-a-thread" } },
      { source: { kind: "approved_document", reference: "file:AGENTS.md" } },
      { source: { ...source, transcript: "complete chat" } },
      { messages: [{ role: "user", content: "complete chat" }] },
      { environment: { SERVICE_TOKEN: "redacted" } },
    ]) {
      assert.throws(
        () => service.record({ ...base, ...mutation }),
        (error) => [
          "DECISION_INPUT_REJECTED",
          "DECISION_SOURCE_REJECTED",
          "DECISION_SENSITIVE_CONTENT_REJECTED",
        ].includes(error.code),
      );
    }
    for (const mutation of [
      { content: `sk-proj-${"a".repeat(24)}` },
      { content: `access_token=${syntheticSecret}` },
      { content: `service_token=${syntheticSecret}` },
      { content: `client_secret=${syntheticSecret}` },
      { content: "OTP 123456" },
      { content: "User: retain everything Assistant: including this transcript" },
      { rationale: "Model internal deliberation should become durable." },
      {
        source: {
          ...source,
          reference: `thread:decision-core/turn:github_pat_${"a".repeat(24)}`,
        },
      },
      {
        source: {
          kind: "approved_document",
          reference: `file:AGENTS.md#access_token:${syntheticSecret}`,
        },
      },
      {
        source: {
          kind: "approved_document",
          reference: `file:AGENTS.md#sk_live_${syntheticSecret}`,
        },
      },
      { clientRequestId: `sk-${"a".repeat(24)}` },
      { clientRequestId: `access_token:${syntheticSecret}` },
    ]) {
      assert.throws(
        () => service.record({ ...base, ...mutation }),
        (error) => error.code === "DECISION_SENSITIVE_CONTENT_REJECTED",
      );
    }
    const after = service.current({ scope: "product:lattice", limit: 10 });
    assert.equal(after.revision, 0);
    assert.equal(after.digest, initial.digest);
    assert.deepEqual(after.decisions, []);
  } finally {
    store.close();
  }
});

test("database constraints and transactions fail closed on double-current, dangling, cycle, and races", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-decision-race-"));
  const databasePath = path.join(directory, "control.db");
  let firstStore;
  let secondStore;
  try {
    firstStore = new LatticeStore(databasePath);
    secondStore = new LatticeStore(databasePath);
    const firstService = new ControlDecisionService({ store: firstStore });
    const secondService = new ControlDecisionService({ store: secondStore });
    const initial = firstService.current({ scope: "product:lattice", limit: 10 });
    const base = {
      scope: "product:lattice",
      subject: "single.current",
      content: "Only one current decision may exist for a subject.",
      rationale: "Competing writers must serialize through the Control store.",
      source,
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    };
    const winner = firstService.record({ ...base, clientRequestId: "decision-race-1" });
    assert.throws(
      () => secondService.record({ ...base, clientRequestId: "decision-race-2" }),
      (error) => error.code === "DECISION_REVISION_MISMATCH",
    );
    assert.equal(firstService.current({ scope: "product:lattice", limit: 10 }).decisions.length, 1);

    assert.throws(
      () => firstStore.database.prepare(`
        INSERT INTO decisions (
          id, scope, subject, content, rationale, source_kind, source_reference,
          status, supersedes_decision_id, client_request_id, request_digest, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, 'current', NULL, ?, ?, ?)
      `).run(
        "00000000-0000-4000-8000-000000000001",
        "product:lattice",
        "single.current",
        "Conflicting current decision.",
        "This must be rejected.",
        source.kind,
        source.reference,
        "decision-race-direct",
        "0".repeat(64),
        new Date().toISOString(),
      ),
      /UNIQUE constraint failed/iu,
    );
    assert.throws(
      () => firstStore.database.prepare(`
        INSERT INTO decisions (
          id, scope, subject, content, rationale, source_kind, source_reference,
          status, supersedes_decision_id, client_request_id, request_digest, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, 'current', ?, ?, ?, ?)
      `).run(
        "00000000-0000-4000-8000-000000000002",
        "product:lattice",
        "dangling.subject",
        "Dangling decision.",
        "This must be rejected.",
        source.kind,
        source.reference,
        "00000000-0000-4000-8000-000000000099",
        "decision-dangling-direct",
        "1".repeat(64),
        new Date().toISOString(),
      ),
      /FOREIGN KEY constraint failed/iu,
    );

    const current = firstService.current({ scope: "product:lattice", limit: 10 });
    const replacement = firstService.record({
      ...base,
      content: "One current decision remains after explicit replacement.",
      supersedesDecisionId: winner.decision.id,
      clientRequestId: "decision-race-3",
      expectedRevision: current.revision,
      expectedDigest: current.digest,
    });
    assert.throws(
      () => firstStore.database.prepare(
        "UPDATE decisions SET supersedes_decision_id = ? WHERE id = ?",
      ).run(replacement.decision.id, winner.decision.id),
      /decision history is immutable/iu,
    );
  } finally {
    secondStore?.close();
    firstStore?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("corrupt decision time and digest are rejected before reads", () => {
  const timeStore = new LatticeStore();
  const timeService = new ControlDecisionService({ store: timeStore });
  try {
    timeStore.database.exec("DROP TRIGGER decision_state_revision_guard;");
    timeStore.database.prepare(
      "UPDATE decision_state SET updated_at = '9999-99-99T99:99:99.999Z' WHERE slot = 'current'",
    ).run();
    assert.throws(
      () => timeService.current({ scope: "product:lattice", limit: 10 }),
      (error) => error.code === "DECISION_STATE_CORRUPT",
    );
  } finally {
    timeStore.close();
  }

  const digestStore = new LatticeStore();
  const digestService = new ControlDecisionService({ store: digestStore });
  try {
    digestStore.database.exec("DROP TRIGGER decision_state_revision_guard;");
    digestStore.database.prepare(
      "UPDATE decision_state SET digest = ? WHERE slot = 'current'",
    ).run("f".repeat(64));
    assert.throws(
      () => digestService.current({ scope: "product:lattice", limit: 10 }),
      (error) => error.code === "DECISION_STATE_DIGEST_MISMATCH",
    );
  } finally {
    digestStore.close();
  }
});
