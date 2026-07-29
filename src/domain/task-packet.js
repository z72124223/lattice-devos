import { deepFreeze } from "./canonical-json.js";
import { domainFailure } from "./errors.js";
import { TASK_STATES } from "./task-state.js";

const TASK_STATE_SET = new Set(TASK_STATES);

function cloneArray(value, field) {
  if (!Array.isArray(value)) {
    domainFailure("INVALID_TASK_PACKET", `${field} must be an array.`);
  }
  try {
    return structuredClone(value);
  } catch (error) {
    domainFailure("INVALID_TASK_PACKET", `${field} must be cloneable JSON data.`, {
      cause: error.message,
    });
  }
}

export function createTaskPacket(
  spec,
  {
    status = "AWAITING_EXECUTION_APPROVAL",
    sequence = 0,
    attempt_id = null,
    active_lease_id = null,
    worktree_id = null,
    result_artifact_hash = null,
    last_error = null,
    approvals = [],
    evidence = [],
  } = {},
) {
  if (
    spec === null ||
    typeof spec !== "object" ||
    typeof spec.spec_hash !== "string" ||
    typeof spec.task_id !== "string"
  ) {
    domainFailure(
      "INVALID_TASK_PACKET",
      "createTaskPacket requires a validated Task Spec.",
    );
  }
  if (!TASK_STATE_SET.has(status)) {
    domainFailure("UNKNOWN_TASK_STATE", "Task Packet status is unknown.", {
      status,
    });
  }
  if (!Number.isInteger(sequence) || sequence < 0) {
    domainFailure(
      "INVALID_TASK_PACKET",
      "Task Packet sequence must be a non-negative integer.",
    );
  }
  const packet = {
    schema_version: "1.0",
    task_id: spec.task_id,
    project_id: spec.project_id,
    risk_class: spec.risk_class,
    status,
    spec,
    record: {
      sequence,
      current_revision: spec.revision,
      attempt_id,
      active_lease_id,
      worktree_id,
      result_artifact_hash,
      last_error,
    },
    approvals: cloneArray(approvals, "approvals"),
    evidence: cloneArray(evidence, "evidence"),
  };
  return deepFreeze(packet);
}

