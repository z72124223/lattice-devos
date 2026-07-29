import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { createTaskSpec } from "../src/domain/task-spec.js";
import { LedgerError, TaskLedger } from "../src/ledger/task-ledger.js";

async function temporaryLedger(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-ledger-"));
  t.after(async () => {
    await rm(root, { force: true, recursive: true });
  });
  let id = 0;
  return {
    root,
    ledger: new TaskLedger({
      root,
      clock: () => new Date("2026-07-29T00:00:00.000Z"),
      idFactory: () => `EVENT-${String(++id).padStart(4, "0")}`,
    }),
  };
}

function firstEvent(overrides = {}) {
  return {
    task_id: "TASK-2026-0001",
    expected_sequence: 0,
    command_id: "CMD-0001",
    correlation_id: "CORR-0001",
    type: "TASK_CREATED",
    actor_id: "lattice-pm",
    role: "LATTICE_PM",
    action: "submit_plan",
    outcome: "recorded",
    reason_code: "TASK_SPEC_ACCEPTED",
    subject_hash: "a".repeat(64),
    payload: {
      goal: "Prove the append-only ledger.",
    },
    ...overrides,
  };
}

function validTaskSpec() {
  return createTaskSpec({
    schema_version: "1.0",
    task_id: "TASK-2026-0001",
    revision: 1,
    created_at: "2026-07-29T00:00:00.000Z",
    created_by: "owner",
    project_id: "lattice-devos",
    base_ref: "main",
    base_commit_sha: "a".repeat(40),
    goal: "Prove Task Ledger replay.",
    non_goals: ["Do not deploy."],
    risk_class: "R1",
    depends_on: [],
    scope: {
      allowed_paths: ["src/ledger/**"],
      forbidden_paths: [".git/**"],
      allowed_operations: ["create", "modify"],
    },
    acceptance_criteria: [
      {
        id: "AC-07",
        description: "Ledger replay is deterministic.",
        evidence_type: "test",
        expected_result: "Reopened packet matches.",
      },
    ],
    verification_commands: ["node --test test/task-ledger.test.js"],
    required_checks: ["test", "scope"],
    requested_capabilities: ["READ_REPOSITORY", "WRITE_PRODUCT_CODE", "RUN_TESTS"],
    budget: {
      max_agents: 4,
      max_duration_seconds: 1800,
      max_attempts: 2,
      max_model_calls: 0,
      max_external_cost: 0,
    },
    runtime_profile: "fake",
    network_policy: "deny",
    deployment_policy: "deny",
    execution_approval_required: true,
    merge_approval_required: true,
  });
}

test("appends and verifies the first hash-chained event", async (t) => {
  const { ledger } = await temporaryLedger(t);

  const receipt = await ledger.append(firstEvent());
  const events = await ledger.verify("TASK-2026-0001");
  const rawLog = await readFile(ledger.taskLogPath("TASK-2026-0001"), "utf8");
  const head = JSON.parse(
    await readFile(ledger.taskHeadPath("TASK-2026-0001"), "utf8"),
  );

  assert.equal(receipt.idempotent, false);
  assert.equal(receipt.event.sequence, 1);
  assert.equal(receipt.event.previous_hash, "0".repeat(64));
  assert.match(receipt.event.hash, /^[a-f0-9]{64}$/);
  assert.equal(events.length, 1);
  assert.deepEqual(events[0], receipt.event);
  assert.equal(rawLog.endsWith("\n"), true);
  assert.equal(head.sequence, 1);
  assert.equal(head.hash, receipt.event.hash);
});

test("is idempotent by command content and serializes sequence conflicts", async (t) => {
  const { ledger } = await temporaryLedger(t);
  const original = firstEvent();
  const first = await ledger.append(original);

  const duplicate = await ledger.append(original);
  assert.equal(duplicate.idempotent, true);
  assert.deepEqual(duplicate.event, first.event);

  await assert.rejects(
    ledger.append(
      firstEvent({
        payload: {
          goal: "Different content under a reused command ID.",
        },
      }),
    ),
    (error) => error instanceof LedgerError && error.code === "COMMAND_ID_REUSE",
  );

  await assert.rejects(
    ledger.append(
      firstEvent({
        command_id: "CMD-0002",
        expected_sequence: 0,
      }),
    ),
    (error) =>
      error instanceof LedgerError && error.code === "LEDGER_SEQUENCE_CONFLICT",
  );

  const contenders = [
    ledger.append(
      firstEvent({
        command_id: "CMD-0003",
        expected_sequence: 1,
        type: "POLICY_DECIDED",
      }),
    ),
    ledger.append(
      firstEvent({
        command_id: "CMD-0004",
        expected_sequence: 1,
        type: "POLICY_DECIDED",
      }),
    ),
  ];
  const settled = await Promise.allSettled(contenders);

  assert.equal(settled.filter((entry) => entry.status === "fulfilled").length, 1);
  assert.equal(settled.filter((entry) => entry.status === "rejected").length, 1);
  assert.equal(
    settled.find((entry) => entry.status === "rejected").reason.code,
    "LEDGER_SEQUENCE_CONFLICT",
  );
  assert.equal((await ledger.verify("TASK-2026-0001")).length, 2);
});

test("reopens and projects Task Packet state only from verified events", async (t) => {
  const { ledger, root } = await temporaryLedger(t);
  const spec = validTaskSpec();
  await ledger.append(
    firstEvent({
      subject_hash: spec.spec_hash,
      payload: {
        spec,
        initial_status: "AWAITING_EXECUTION_APPROVAL",
      },
    }),
  );
  await ledger.append(
    firstEvent({
      expected_sequence: 1,
      command_id: "CMD-0002",
      type: "STATE_TRANSITION",
      action: "approve_execution",
      reason_code: "EXECUTION_APPROVED",
      subject_hash: spec.spec_hash,
      payload: {
        from: "AWAITING_EXECUTION_APPROVAL",
        to: "PREPARING",
      },
    }),
  );

  const reopened = new TaskLedger({ root });
  const packet = await reopened.readTaskPacket("TASK-2026-0001");

  assert.equal(packet.status, "PREPARING");
  assert.equal(packet.record.sequence, 2);
  assert.equal(packet.spec.spec_hash, spec.spec_hash);
  assert.equal(Object.isFrozen(packet), true);
});

test("detects changed, reordered, and tail-truncated event streams", async (t) => {
  async function twoEventLedger() {
    const fixture = await temporaryLedger(t);
    await fixture.ledger.append(firstEvent());
    await fixture.ledger.append(
      firstEvent({
        expected_sequence: 1,
        command_id: "CMD-0002",
        type: "POLICY_DECIDED",
      }),
    );
    return fixture;
  }

  const changed = await twoEventLedger();
  const changedLines = (
    await readFile(changed.ledger.taskLogPath("TASK-2026-0001"), "utf8")
  )
    .trimEnd()
    .split("\n");
  const changedEvent = JSON.parse(changedLines[0]);
  changedEvent.payload.goal = "Tampered after append.";
  changedLines[0] = JSON.stringify(changedEvent);
  await writeFile(
    changed.ledger.taskLogPath("TASK-2026-0001"),
    `${changedLines.join("\n")}\n`,
  );
  await assert.rejects(
    changed.ledger.verify("TASK-2026-0001"),
    (error) => error instanceof LedgerError && error.code === "LEDGER_HASH_MISMATCH",
  );

  const reordered = await twoEventLedger();
  const reorderedLines = (
    await readFile(reordered.ledger.taskLogPath("TASK-2026-0001"), "utf8")
  )
    .trimEnd()
    .split("\n")
    .reverse();
  await writeFile(
    reordered.ledger.taskLogPath("TASK-2026-0001"),
    `${reorderedLines.join("\n")}\n`,
  );
  await assert.rejects(
    reordered.ledger.verify("TASK-2026-0001"),
    (error) => error instanceof LedgerError && error.code === "LEDGER_EVENT_INVALID",
  );

  const truncated = await twoEventLedger();
  const truncatedLines = (
    await readFile(truncated.ledger.taskLogPath("TASK-2026-0001"), "utf8")
  )
    .trimEnd()
    .split("\n");
  truncatedLines.pop();
  await writeFile(
    truncated.ledger.taskLogPath("TASK-2026-0001"),
    `${truncatedLines.join("\n")}\n`,
  );
  await assert.rejects(
    truncated.ledger.verify("TASK-2026-0001"),
    (error) => error instanceof LedgerError && error.code === "LEDGER_HEAD_MISMATCH",
  );
});

test("redacts nested secret keys and recognizable secret values before persistence", async (t) => {
  const { ledger } = await temporaryLedger(t);
  const secretValues = [
    "never-store-this-password",
    "secret-token-value",
    "sk-example123456789",
    "Bearer abcdefghijklmnop",
  ];

  const receipt = await ledger.append(
    firstEvent({
      payload: {
        password: secretValues[0],
        nested: {
          access_token: secretValues[1],
          note: `${secretValues[2]} and ${secretValues[3]}`,
        },
        safe: "visible audit detail",
      },
    }),
  );
  const raw = await readFile(ledger.taskLogPath("TASK-2026-0001"), "utf8");

  for (const secret of secretValues) {
    assert.equal(raw.includes(secret), false, secret);
  }
  assert.equal(receipt.event.payload.password, "[REDACTED]");
  assert.equal(receipt.event.payload.nested.access_token, "[REDACTED]");
  assert.equal(
    receipt.event.payload.nested.note,
    "[REDACTED] and [REDACTED]",
  );
  assert.equal(receipt.event.payload.safe, "visible audit detail");

  const cyclic = {};
  cyclic.self = cyclic;
  await assert.rejects(
    ledger.append(
      firstEvent({
        command_id: "CMD-CYCLIC",
        payload: cyclic,
      }),
    ),
    (error) =>
      error instanceof LedgerError && error.code === "AUDIT_PAYLOAD_NOT_JSON",
  );
});
