import { domainFailure } from "./errors.js";

export const TASK_STATES = Object.freeze([
  "DRAFT",
  "AWAITING_EXECUTION_APPROVAL",
  "PREPARING",
  "EXECUTING",
  "VERIFYING",
  "REVIEWING",
  "AWAITING_MERGE_APPROVAL",
  "MERGING",
  "COMPLETED",
  "REJECTED",
  "BLOCKED",
  "FAILED",
  "STOPPING",
  "CANCELLED",
]);

export const ALLOWED_TRANSITIONS = Object.freeze({
  DRAFT: Object.freeze(["AWAITING_EXECUTION_APPROVAL", "CANCELLED"]),
  AWAITING_EXECUTION_APPROVAL: Object.freeze([
    "PREPARING",
    "REJECTED",
    "CANCELLED",
  ]),
  PREPARING: Object.freeze(["EXECUTING", "BLOCKED", "FAILED", "STOPPING"]),
  EXECUTING: Object.freeze(["VERIFYING", "BLOCKED", "FAILED", "STOPPING"]),
  VERIFYING: Object.freeze(["REVIEWING", "BLOCKED", "FAILED", "STOPPING"]),
  REVIEWING: Object.freeze([
    "AWAITING_MERGE_APPROVAL",
    "BLOCKED",
    "FAILED",
    "STOPPING",
  ]),
  AWAITING_MERGE_APPROVAL: Object.freeze([
    "MERGING",
    "REJECTED",
    "CANCELLED",
  ]),
  MERGING: Object.freeze(["COMPLETED", "BLOCKED", "FAILED", "STOPPING"]),
  STOPPING: Object.freeze(["CANCELLED", "FAILED"]),
  COMPLETED: Object.freeze([]),
  REJECTED: Object.freeze([]),
  BLOCKED: Object.freeze([]),
  FAILED: Object.freeze([]),
  CANCELLED: Object.freeze([]),
});

const TASK_STATE_SET = new Set(TASK_STATES);

export function isTransitionAllowed(from, to) {
  if (!TASK_STATE_SET.has(from) || !TASK_STATE_SET.has(to)) {
    return false;
  }
  return ALLOWED_TRANSITIONS[from].includes(to);
}

export function transitionTaskState(from, to) {
  if (!TASK_STATE_SET.has(from) || !TASK_STATE_SET.has(to)) {
    domainFailure("UNKNOWN_TASK_STATE", "Task transition contains an unknown state.", {
      from,
      to,
    });
  }
  if (!isTransitionAllowed(from, to)) {
    domainFailure(
      "INVALID_STATE_TRANSITION",
      `Task state cannot transition from ${from} to ${to}.`,
      { from, to },
    );
  }
  return to;
}

