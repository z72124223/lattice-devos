import {
  createTaskPacket,
  createTaskSpec,
  transitionTaskState,
} from "../domain/task-spec.js";
import { ledgerFailure } from "./errors.js";

function projectionFailure(message, event = undefined) {
  ledgerFailure("LEDGER_PROJECTION_INVALID", message, {
    sequence: event?.sequence,
    type: event?.type,
  });
}

function recreateSpec(storedSpec, event) {
  if (
    storedSpec === null ||
    typeof storedSpec !== "object" ||
    Array.isArray(storedSpec)
  ) {
    projectionFailure("TASK_CREATED event does not contain a Task Spec.", event);
  }
  const { spec_hash: storedHash, ...input } = storedSpec;
  let recreated;
  try {
    recreated = createTaskSpec(input);
  } catch (error) {
    projectionFailure(`Stored Task Spec is invalid: ${error.message}`, event);
  }
  if (recreated.spec_hash !== storedHash || event.subject_hash !== storedHash) {
    projectionFailure("Stored Task Spec hash does not match event evidence.", event);
  }
  return recreated;
}

export function projectTaskPacketFromEvents(events) {
  if (!Array.isArray(events) || events.length === 0) {
    ledgerFailure("TASK_NOT_FOUND", "No task events exist for projection.");
  }
  const created = events[0];
  if (created.type !== "TASK_CREATED" || created.sequence !== 1) {
    projectionFailure("The first task event must be TASK_CREATED.", created);
  }
  const spec = recreateSpec(created.payload?.spec, created);
  let status = created.payload?.initial_status;
  if (status !== "AWAITING_EXECUTION_APPROVAL") {
    projectionFailure(
      "TASK_CREATED must enter AWAITING_EXECUTION_APPROVAL.",
      created,
    );
  }

  const approvals = [];
  const evidence = [];
  const record = {
    sequence: events.length,
    attempt_id: null,
    active_lease_id: null,
    worktree_id: null,
    result_artifact_hash: null,
    last_error: null,
  };

  for (const event of events.slice(1)) {
    switch (event.type) {
      case "STATE_TRANSITION": {
        if (event.payload?.from !== status || typeof event.payload?.to !== "string") {
          projectionFailure("State transition does not start at current state.", event);
        }
        try {
          status = transitionTaskState(status, event.payload.to);
        } catch (error) {
          projectionFailure(`Illegal replayed transition: ${error.message}`, event);
        }
        break;
      }
      case "APPROVAL_RECORDED":
        if (
          event.payload?.approval === null ||
          typeof event.payload?.approval !== "object"
        ) {
          projectionFailure("Approval event has no approval record.", event);
        }
        approvals.push(event.payload.approval);
        break;
      case "EVIDENCE_RECORDED":
        if (
          event.payload?.evidence === null ||
          typeof event.payload?.evidence !== "object"
        ) {
          projectionFailure("Evidence event has no evidence record.", event);
        }
        evidence.push(event.payload.evidence);
        break;
      case "ATTEMPT_STARTED":
        record.attempt_id = event.payload?.attempt_id ?? null;
        break;
      case "LEASE_ACQUIRED":
        record.active_lease_id = event.payload?.lease_id ?? null;
        break;
      case "LEASE_RELEASED":
        record.active_lease_id = null;
        break;
      case "WORKTREE_PREPARED":
        record.worktree_id = event.payload?.worktree_id ?? null;
        break;
      case "ARTIFACT_RECORDED":
        record.result_artifact_hash = event.payload?.artifact_hash ?? null;
        break;
      case "ERROR_RECORDED":
        record.last_error = event.payload?.message ?? event.reason_code;
        break;
      default:
        break;
    }
  }

  return createTaskPacket(spec, {
    status,
    sequence: record.sequence,
    attempt_id: record.attempt_id,
    active_lease_id: record.active_lease_id,
    worktree_id: record.worktree_id,
    result_artifact_hash: record.result_artifact_hash,
    last_error: record.last_error,
    approvals,
    evidence,
  });
}

