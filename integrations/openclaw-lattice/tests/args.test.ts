import assert from "node:assert/strict";
import { test } from "node:test";

import {
  deriveCommandIdentities,
  parseLatticeArguments,
} from "../src/commands.js";

test("accepts only the closed lattice command grammar", () => {
  assert.deepEqual(parseLatticeArguments("status project project-a"), {
    action: "status",
    projectId: "project-a",
    targetKind: "project",
  });
  assert.deepEqual(parseLatticeArguments("status command project-a command-a"), {
    action: "status",
    projectId: "project-a",
    targetCommandId: "command-a",
    targetKind: "command",
  });
  const taskTail = `project-a snapshot-a task-a 2 ${"a".repeat(64)} ${"b".repeat(64)}`;
  const target = {
    expectedLedgerHeadDigest: "b".repeat(64),
    projectId: "project-a",
    projectSnapshotId: "snapshot-a",
    taskId: "task-a",
    taskRevision: "2",
    taskSpecDigest: "a".repeat(64),
  };
  assert.deepEqual(parseLatticeArguments(`status task ${taskTail}`), {
    action: "status",
    target,
    targetKind: "task",
  });
  assert.deepEqual(parseLatticeArguments(`stop ${taskTail} attempt-a SAFETY_CONCERN`), {
    action: "stop",
    attemptId: "attempt-a",
    reason: "SAFETY_CONCERN",
    target,
  });
  assert.deepEqual(
    parseLatticeArguments(`submit ${"a".repeat(64)}`),
    { action: "submit", taskSpecDigest: "a".repeat(64) },
  );
});

test("rejects arbitrary text and dangerous or unknown schema", () => {
  const forbidden = [
    "",
    "submit write the feature",
    `submit ${"A".repeat(64)}`,
    `submit ${"0".repeat(64)}`,
    "status project project-a extra",
    "status command command-a",
    "status task task-a",
    "status memory project-a",
    "stop",
    "stop command-a extra",
    `stop project-a snapshot-a task-a 2 ${"a".repeat(64)} ${"b".repeat(64)} attempt-a arbitrary text`,
    "shell whoami",
    "sql SELECT",
    "path C:/repo",
    "credential token",
    "provider openai",
    "unknown anything",
  ];
  for (const value of forbidden) {
    assert.throws(() => parseLatticeArguments(value), { name: "LatticeInputError" });
  }
});

test("derives domain-separated stable ids from session key and canonical args", () => {
  const command = parseLatticeArguments("  status   project   project-a  ");
  const first = deriveCommandIdentities("session-a", command);
  const same = deriveCommandIdentities(
    "session-a",
    parseLatticeArguments("status project project-a"),
  );
  const otherSession = deriveCommandIdentities("session-b", command);

  assert.deepEqual(first, same);
  assert.match(first.commandId, /^[0-9a-f]{64}$/u);
  assert.match(first.correlationId, /^[0-9a-f]{64}$/u);
  assert.notEqual(first.commandId, first.correlationId);
  assert.notDeepEqual(first, otherSession);
});
