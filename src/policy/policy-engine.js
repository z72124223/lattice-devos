import { deepFreeze } from "../domain/canonical-json.js";
import { TASK_STATES } from "../domain/task-spec.js";
import {
  ACTIONS,
  AGENT_ROLES,
  PHASE1_PROTECTED_ACTIONS,
  ROLE_ACTIONS,
} from "./roles.js";
import { createMergeApprovalSubject, verifyApproval } from "./approval.js";

const KNOWN_ROLES = new Set(Object.values(AGENT_ROLES));
const KNOWN_ACTIONS = new Set(Object.values(ACTIONS));
const KNOWN_STATES = new Set(TASK_STATES);

const ACTION_STATES = Object.freeze({
  [ACTIONS.SUBMIT_PLAN]: Object.freeze(["DRAFT"]),
  [ACTIONS.PLAN_TASK]: Object.freeze(["DRAFT"]),
  [ACTIONS.MAP_CODE]: Object.freeze(["DRAFT", "AWAITING_EXECUTION_APPROVAL"]),
  [ACTIONS.WRITE_PRODUCT_CODE]: Object.freeze(["EXECUTING"]),
  [ACTIONS.RUN_TESTS]: Object.freeze(["EXECUTING", "VERIFYING"]),
  [ACTIONS.REVIEW_CORRECTNESS]: Object.freeze(["REVIEWING"]),
  [ACTIONS.REVIEW_SECURITY]: Object.freeze(["REVIEWING"]),
  [ACTIONS.REVIEW_ARCHITECTURE]: Object.freeze(["REVIEWING"]),
  [ACTIONS.PREPARE_WORKTREE]: Object.freeze(["PREPARING"]),
  [ACTIONS.INTEGRATE_GIT]: Object.freeze(["MERGING"]),
  [ACTIONS.STOP_RUNTIME]: Object.freeze([
    "PREPARING",
    "EXECUTING",
    "VERIFYING",
    "REVIEWING",
    "MERGING",
    "STOPPING",
  ]),
});

function decision(allowed, reasonCode, evidence) {
  return deepFreeze({
    allowed,
    reason_code: reasonCode,
    evidence,
  });
}

function validateWriterLease({ lease, spec, now }) {
  if (lease === null || typeof lease !== "object" || lease.active !== true) {
    return decision(false, "WRITER_LEASE_REQUIRED", {
      task_id: spec?.task_id,
    });
  }
  const expiresAt = Date.parse(lease.expires_at);
  if (!Number.isFinite(expiresAt) || expiresAt <= now.getTime()) {
    return decision(false, "WRITER_LEASE_EXPIRED", {
      task_id: spec?.task_id,
    });
  }
  if (
    lease.role !== AGENT_ROLES.IMPLEMENTER ||
    lease.project_id !== spec?.project_id ||
    lease.task_id !== spec?.task_id ||
    lease.task_revision !== spec?.revision ||
    lease.spec_hash !== spec?.spec_hash ||
    !Number.isInteger(lease.fencing_token) ||
    lease.fencing_token !== lease.current_fencing_token
  ) {
    return decision(false, "WRITER_LEASE_INVALID", {
      task_id: spec?.task_id,
    });
  }
  return null;
}

export class PolicyEngine {
  #clock;
  #approvalVerifier;

  constructor({
    clock = () => new Date(),
    approvalVerifier = async () => ({ verified: false }),
  } = {}) {
    this.#clock = clock;
    this.#approvalVerifier = approvalVerifier;
  }

  authorizeAgentAction({ role, action, state, spec, lease = null }) {
    if (!KNOWN_ROLES.has(role)) {
      return decision(false, "UNKNOWN_ROLE", { role });
    }
    if (!KNOWN_ACTIONS.has(action)) {
      return decision(false, "UNKNOWN_ACTION", { action });
    }
    if (!KNOWN_STATES.has(state)) {
      return decision(false, "UNKNOWN_STATE", { state });
    }
    if (PHASE1_PROTECTED_ACTIONS.has(action)) {
      return decision(false, "PHASE1_PROTECTED_ACTION", { action });
    }
    if (!ROLE_ACTIONS[role].includes(action)) {
      return decision(false, "ROLE_ACTION_DENIED", { role, action });
    }
    const allowedStates = ACTION_STATES[action];
    if (allowedStates && !allowedStates.includes(state)) {
      return decision(false, "ACTION_STATE_DENIED", { action, state });
    }
    if (action === ACTIONS.WRITE_PRODUCT_CODE) {
      const leaseDenial = validateWriterLease({
        lease,
        spec,
        now: this.#clock(),
      });
      if (leaseDenial) {
        return leaseDenial;
      }
      return decision(true, "AGENT_ACTION_ALLOWED", {
        role,
        action,
        fencing_token: lease.fencing_token,
      });
    }
    return decision(true, "AGENT_ACTION_ALLOWED", { role, action });
  }

  async verifyExecutionApproval({
    spec,
    state,
    approval,
    usedNonces = new Set(),
  }) {
    return verifyApproval({
      kind: "execution",
      expectedState: "AWAITING_EXECUTION_APPROVAL",
      state,
      spec,
      approval,
      subjectHash: spec?.spec_hash,
      usedNonces,
      approvalVerifier: this.#approvalVerifier,
      now: this.#clock(),
    });
  }

  async verifyMergeApproval({
    spec,
    state,
    approval,
    reviewed_commit,
    diff_hash,
    usedNonces = new Set(),
  }) {
    if (
      typeof reviewed_commit !== "string" ||
      !/^[a-f0-9]{40,64}$/.test(reviewed_commit) ||
      typeof diff_hash !== "string" ||
      !/^[a-f0-9]{64}$/.test(diff_hash)
    ) {
      return decision(false, "MERGE_EVIDENCE_INVALID", {});
    }
    const subjectHash = createMergeApprovalSubject({
      task_id: spec?.task_id,
      task_revision: spec?.revision,
      reviewed_commit,
      diff_hash,
    });
    return verifyApproval({
      kind: "merge",
      expectedState: "AWAITING_MERGE_APPROVAL",
      state,
      spec,
      approval,
      subjectHash,
      usedNonces,
      approvalVerifier: this.#approvalVerifier,
      now: this.#clock(),
    });
  }

  admitWorkers({
    spec,
    active_workers = [],
    requested_roles = [],
  }) {
    if (
      spec?.runtime_profile !== "fake" ||
      spec?.network_policy !== "deny" ||
      spec?.deployment_policy !== "deny" ||
      !Number.isInteger(spec?.budget?.max_agents) ||
      spec.budget.max_agents < 1 ||
      spec.budget.max_agents > 4 ||
      spec.budget.max_model_calls !== 0 ||
      spec.budget.max_external_cost !== 0
    ) {
      return decision(false, "PHASE1_ENVELOPE_DENIED", {});
    }
    if (!Array.isArray(active_workers) || !Array.isArray(requested_roles)) {
      return decision(false, "WORKER_ADMISSION_INVALID", {});
    }
    for (const role of [...active_workers, ...requested_roles]) {
      if (!KNOWN_ROLES.has(role)) {
        return decision(false, "UNKNOWN_ROLE", { role });
      }
    }
    const resultingWorkers = active_workers.length + requested_roles.length;
    const limit = Math.min(spec.budget.max_agents, 4);
    if (resultingWorkers > limit) {
      return decision(false, "AGENT_LIMIT_EXCEEDED", {
        active_workers: active_workers.length,
        requested_workers: requested_roles.length,
        resulting_workers: resultingWorkers,
        limit,
      });
    }
    return decision(true, "WORKER_ADMISSION_ALLOWED", {
      active_workers: active_workers.length,
      requested_workers: requested_roles.length,
      resulting_workers: resultingWorkers,
      limit,
    });
  }
}

export { ACTIONS, AGENT_ROLES };
