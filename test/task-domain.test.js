import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  DomainError,
  TASK_STATES,
  ALLOWED_TRANSITIONS,
  assertAcyclicTaskGraph,
  createTaskPacket,
  createTaskSpec,
  isTransitionAllowed,
  transitionTaskState,
} from "../src/domain/task-spec.js";

function validTaskSpec(overrides = {}) {
  return {
    schema_version: "1.0",
    task_id: "TASK-2026-0001",
    revision: 1,
    created_at: "2026-07-29T00:00:00.000Z",
    created_by: "owner",
    project_id: "lattice-devos",
    base_ref: "main",
    base_commit_sha: "a".repeat(40),
    goal: "Prove the offline controlled workflow.",
    non_goals: ["Do not deploy."],
    risk_class: "R1",
    depends_on: [],
    scope: {
      allowed_paths: ["src/domain/**", "test/task-domain.test.js"],
      forbidden_paths: [".git/**"],
      allowed_operations: ["create", "modify"],
    },
    acceptance_criteria: [
      {
        id: "AC-01",
        description: "A safe task spec is accepted.",
        evidence_type: "test",
        expected_result: "Focused test exits with code 0.",
      },
    ],
    verification_commands: ["node --test test/task-domain.test.js"],
    required_checks: ["test", "scope"],
    requested_capabilities: [
      "READ_REPOSITORY",
      "WRITE_PRODUCT_CODE",
      "RUN_TESTS",
    ],
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
    ...overrides,
  };
}

test("accepts and deeply freezes a safe Phase 1 task spec", () => {
  const spec = createTaskSpec(validTaskSpec());

  assert.equal(spec.task_id, "TASK-2026-0001");
  assert.match(spec.spec_hash, /^[a-f0-9]{64}$/);
  assert.equal(Object.isFrozen(spec), true);
  assert.equal(Object.isFrozen(spec.scope), true);
  assert.equal(Object.isFrozen(spec.acceptance_criteria), true);
});

test("rejects Phase 1 envelopes that could call a model, network, or deployment", () => {
  const unsafeCases = [
    { budget: { ...validTaskSpec().budget, max_model_calls: 1 } },
    { budget: { ...validTaskSpec().budget, max_external_cost: 0.01 } },
    { network_policy: "allow" },
    { deployment_policy: "allow" },
    { runtime_profile: "codex" },
  ];

  for (const override of unsafeCases) {
    assert.throws(
      () => createTaskSpec(validTaskSpec(override)),
      (error) =>
        error instanceof DomainError &&
        error.code === "PHASE1_SAFETY_ENVELOPE_REQUIRED",
    );
  }
});

test("hashes only normalized immutable spec fields deterministically", () => {
  const firstInput = validTaskSpec();
  const secondInput = validTaskSpec();
  const first = createTaskSpec(firstInput);
  const second = createTaskSpec(secondInput);
  const revised = createTaskSpec(
    validTaskSpec({
      revision: 2,
      goal: "Prove a revised offline controlled workflow.",
    }),
  );

  assert.equal(first.spec_hash, second.spec_hash);
  assert.notEqual(first.spec_hash, revised.spec_hash);

  firstInput.goal = "Caller mutation after creation.";
  assert.equal(first.goal, "Prove the offline controlled workflow.");
});

test("accepts an acyclic task graph and rejects a dependency cycle", () => {
  assert.doesNotThrow(() =>
    assertAcyclicTaskGraph({
      "TASK-2026-0001": [],
      "TASK-2026-0002": ["TASK-2026-0001"],
      "TASK-2026-0003": ["TASK-2026-0002"],
    }),
  );

  assert.throws(
    () =>
      assertAcyclicTaskGraph({
        "TASK-2026-0001": ["TASK-2026-0003"],
        "TASK-2026-0002": ["TASK-2026-0001"],
        "TASK-2026-0003": ["TASK-2026-0002"],
      }),
    (error) =>
      error instanceof DomainError &&
      error.code === "TASK_DEPENDENCY_CYCLE" &&
      error.details.cycle[0] === error.details.cycle.at(-1),
  );
});

test("enforces the complete task transition graph", () => {
  const mainPath = [
    "DRAFT",
    "AWAITING_EXECUTION_APPROVAL",
    "PREPARING",
    "EXECUTING",
    "VERIFYING",
    "REVIEWING",
    "AWAITING_MERGE_APPROVAL",
    "MERGING",
    "COMPLETED",
  ];
  let current = mainPath[0];
  for (const next of mainPath.slice(1)) {
    current = transitionTaskState(current, next);
  }
  assert.equal(current, "COMPLETED");

  for (const from of TASK_STATES) {
    for (const to of TASK_STATES) {
      assert.equal(
        isTransitionAllowed(from, to),
        ALLOWED_TRANSITIONS[from].includes(to),
        `${from} -> ${to}`,
      );
    }
  }

  assert.throws(
    () => transitionTaskState("COMPLETED", "EXECUTING"),
    (error) =>
      error instanceof DomainError &&
      error.code === "INVALID_STATE_TRANSITION" &&
      error.details.from === "COMPLETED" &&
      error.details.to === "EXECUTING",
  );
  assert.throws(
    () => transitionTaskState("UNKNOWN", "DRAFT"),
    (error) => error instanceof DomainError && error.code === "UNKNOWN_TASK_STATE",
  );
});

test("projects an immutable initial Task Packet separate from immutable spec", async () => {
  const spec = createTaskSpec(validTaskSpec());
  const packet = createTaskPacket(spec);
  const schema = JSON.parse(
    await readFile(
      new URL("../schemas/task-packet.schema.json", import.meta.url),
      "utf8",
    ),
  );

  assert.equal(packet.schema_version, "1.0");
  assert.equal(packet.task_id, spec.task_id);
  assert.equal(packet.status, "AWAITING_EXECUTION_APPROVAL");
  assert.equal(packet.record.sequence, 0);
  assert.equal(packet.record.current_revision, 1);
  assert.equal(packet.spec.spec_hash, spec.spec_hash);
  assert.deepEqual(packet.approvals, []);
  assert.deepEqual(packet.evidence, []);
  assert.equal(Object.isFrozen(packet), true);
  assert.equal(Object.isFrozen(packet.record), true);
  assert.equal(schema.$schema, "https://json-schema.org/draft/2020-12/schema");
  assert.equal(schema.additionalProperties, false);
  assert.deepEqual(
    new Set(schema.required),
    new Set([
      "schema_version",
      "task_id",
      "project_id",
      "risk_class",
      "status",
      "spec",
      "record",
      "approvals",
      "evidence",
    ]),
  );
});
