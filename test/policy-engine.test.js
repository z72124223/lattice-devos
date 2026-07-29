import assert from "node:assert/strict";
import test from "node:test";

import { createTaskSpec } from "../src/domain/task-spec.js";
import {
  ACTIONS,
  AGENT_ROLES,
  PolicyEngine,
} from "../src/policy/policy-engine.js";
import { createMergeApprovalSubject } from "../src/policy/approval.js";

function taskSpec() {
  return createTaskSpec({
    schema_version: "1.0",
    task_id: "TASK-2026-0001",
    revision: 1,
    created_at: "2026-07-29T00:00:00.000Z",
    created_by: "owner",
    project_id: "lattice-devos",
    base_ref: "main",
    base_commit_sha: "a".repeat(40),
    goal: "Prove fail-closed policy.",
    non_goals: ["Do not deploy."],
    risk_class: "R1",
    depends_on: [],
    scope: {
      allowed_paths: ["src/policy/**"],
      forbidden_paths: [".git/**"],
      allowed_operations: ["create", "modify"],
    },
    acceptance_criteria: [
      {
        id: "AC-03",
        description: "Only Implementer can write product code.",
        evidence_type: "test",
        expected_result: "Policy matrix test passes.",
      },
    ],
    verification_commands: ["node --test test/policy-engine.test.js"],
    required_checks: ["test", "scope", "security"],
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

function activeLease(spec) {
  return {
    active: true,
    project_id: spec.project_id,
    task_id: spec.task_id,
    task_revision: spec.revision,
    spec_hash: spec.spec_hash,
    role: "IMPLEMENTER",
    fencing_token: 7,
    current_fencing_token: 7,
    expires_at: "2026-07-29T00:30:00.000Z",
  };
}

function engine() {
  return new PolicyEngine({
    clock: () => new Date("2026-07-29T00:10:00.000Z"),
    approvalVerifier: async (approval) => ({
      verified: true,
      owner_id: approval.approver_id,
    }),
  });
}

function executionApproval(spec, overrides = {}) {
  return {
    approval_id: "APPROVAL-EXEC-0001",
    kind: "execution",
    task_id: spec.task_id,
    task_revision: spec.revision,
    subject_hash: spec.spec_hash,
    approver_id: "owner",
    authority: "HUMAN_OWNER",
    issued_at: "2026-07-29T00:00:00.000Z",
    expires_at: "2026-07-29T00:20:00.000Z",
    nonce: "nonce-exec-0001",
    channel: "fake-owner-channel",
    ...overrides,
  };
}

function mergeApproval(spec, mergeEvidence, overrides = {}) {
  return {
    approval_id: "APPROVAL-MERGE-0001",
    kind: "merge",
    task_id: spec.task_id,
    task_revision: spec.revision,
    subject_hash: createMergeApprovalSubject({
      task_id: spec.task_id,
      task_revision: spec.revision,
      reviewed_commit: mergeEvidence.reviewed_commit,
      diff_hash: mergeEvidence.diff_hash,
    }),
    approver_id: "owner",
    authority: "HUMAN_OWNER",
    issued_at: "2026-07-29T00:00:00.000Z",
    expires_at: "2026-07-29T00:20:00.000Z",
    nonce: "nonce-merge-0001",
    channel: "fake-owner-channel",
    ...overrides,
  };
}

test("defaults unknowns to deny and grants code write only to current Implementer lease", () => {
  const policy = engine();
  const spec = taskSpec();
  const lease = activeLease(spec);

  assert.deepEqual(
    policy.authorizeAgentAction({
      role: "UNKNOWN",
      action: ACTIONS.READ_REPOSITORY,
      state: "EXECUTING",
      spec,
      lease,
    }),
    {
      allowed: false,
      reason_code: "UNKNOWN_ROLE",
      evidence: { role: "UNKNOWN" },
    },
  );
  assert.equal(
    policy.authorizeAgentAction({
      role: AGENT_ROLES.IMPLEMENTER,
      action: "UNKNOWN_ACTION",
      state: "EXECUTING",
      spec,
      lease,
    }).reason_code,
    "UNKNOWN_ACTION",
  );
  assert.equal(
    policy.authorizeAgentAction({
      role: AGENT_ROLES.IMPLEMENTER,
      action: ACTIONS.WRITE_PRODUCT_CODE,
      state: "UNKNOWN_STATE",
      spec,
      lease,
    }).reason_code,
    "UNKNOWN_STATE",
  );
  assert.equal(
    policy.authorizeAgentAction({
      role: AGENT_ROLES.PLANNER,
      action: ACTIONS.WRITE_PRODUCT_CODE,
      state: "EXECUTING",
      spec,
      lease,
    }).reason_code,
    "ROLE_ACTION_DENIED",
  );
  assert.equal(
    policy.authorizeAgentAction({
      role: AGENT_ROLES.IMPLEMENTER,
      action: ACTIONS.WRITE_PRODUCT_CODE,
      state: "EXECUTING",
      spec,
      lease: null,
    }).reason_code,
    "WRITER_LEASE_REQUIRED",
  );
  assert.deepEqual(
    policy.authorizeAgentAction({
      role: AGENT_ROLES.IMPLEMENTER,
      action: ACTIONS.WRITE_PRODUCT_CODE,
      state: "EXECUTING",
      spec,
      lease,
    }),
    {
      allowed: true,
      reason_code: "AGENT_ACTION_ALLOWED",
      evidence: {
        role: "IMPLEMENTER",
        action: "WRITE_PRODUCT_CODE",
        fencing_token: 7,
      },
    },
  );
  assert.equal(
    policy.authorizeAgentAction({
      role: AGENT_ROLES.INTEGRATOR,
      action: ACTIONS.WRITE_PRODUCT_CODE,
      state: "MERGING",
      spec,
      lease,
    }).allowed,
    false,
  );
});

test("denies every Phase 1 protected action for every agent role", () => {
  const policy = engine();
  const spec = taskSpec();
  const protectedActions = [
    ACTIONS.RESOLVE_MERGE_CONFLICT,
    ACTIONS.CALL_REAL_MODEL,
    ACTIONS.NETWORK_ACCESS,
    ACTIONS.DEPLOY_PRODUCTION,
    ACTIONS.PURCHASE_SERVICE,
    ACTIONS.MANAGE_CREDENTIALS,
    ACTIONS.PUBLIC_PUBLISH,
    ACTIONS.PERMANENT_DELETE,
    ACTIONS.ACCESS_PLAYMATE,
    ACTIONS.DISABLE_SECURITY,
  ];

  for (const role of Object.values(AGENT_ROLES)) {
    for (const action of protectedActions) {
      assert.deepEqual(
        policy.authorizeAgentAction({
          role,
          action,
          state: "EXECUTING",
          spec,
          lease: activeLease(spec),
        }),
        {
          allowed: false,
          reason_code: "PHASE1_PROTECTED_ACTION",
          evidence: { action },
        },
        `${role} ${action}`,
      );
    }
  }
});

test("accepts only current, unused, owner-verified execution approval", async () => {
  const policy = engine();
  const spec = taskSpec();
  const approval = executionApproval(spec);

  assert.deepEqual(
    await policy.verifyExecutionApproval({
      spec,
      state: "AWAITING_EXECUTION_APPROVAL",
      approval,
      usedNonces: new Set(),
    }),
    {
      allowed: true,
      reason_code: "EXECUTION_APPROVAL_VALID",
      evidence: {
        approval_id: approval.approval_id,
        kind: "execution",
        approver_id: "owner",
        subject_hash: spec.spec_hash,
      },
    },
  );

  const deniedCases = [
    {
      override: { kind: "merge" },
      reason: "APPROVAL_KIND_MISMATCH",
    },
    {
      override: { task_id: "TASK-2026-9999" },
      reason: "APPROVAL_TASK_MISMATCH",
    },
    {
      override: { task_revision: 2 },
      reason: "APPROVAL_REVISION_MISMATCH",
    },
    {
      override: { subject_hash: "b".repeat(64) },
      reason: "APPROVAL_SUBJECT_MISMATCH",
    },
    {
      override: { authority: "AGENT" },
      reason: "APPROVAL_AUTHORITY_DENIED",
    },
    {
      override: { expires_at: "2026-07-29T00:09:59.000Z" },
      reason: "APPROVAL_EXPIRED",
    },
  ];

  for (const denied of deniedCases) {
    assert.equal(
      (
        await policy.verifyExecutionApproval({
          spec,
          state: "AWAITING_EXECUTION_APPROVAL",
          approval: executionApproval(spec, denied.override),
          usedNonces: new Set(),
        })
      ).reason_code,
      denied.reason,
    );
  }

  assert.equal(
    (
      await policy.verifyExecutionApproval({
        spec,
        state: "AWAITING_EXECUTION_APPROVAL",
        approval,
        usedNonces: new Set([approval.nonce]),
      })
    ).reason_code,
    "APPROVAL_REPLAYED",
  );
  assert.equal(
    (
      await policy.verifyExecutionApproval({
        spec,
        state: "EXECUTING",
        approval,
        usedNonces: new Set(),
      })
    ).reason_code,
    "APPROVAL_STATE_DENIED",
  );

  const unverified = new PolicyEngine({
    clock: () => new Date("2026-07-29T00:10:00.000Z"),
    approvalVerifier: async () => ({ verified: false }),
  });
  assert.equal(
    (
      await unverified.verifyExecutionApproval({
        spec,
        state: "AWAITING_EXECUTION_APPROVAL",
        approval,
        usedNonces: new Set(),
      })
    ).reason_code,
    "APPROVAL_IDENTITY_UNVERIFIED",
  );
});

test("binds merge approval to the exact reviewed commit and diff hash", async () => {
  const policy = engine();
  const spec = taskSpec();
  const reviewed = {
    reviewed_commit: "b".repeat(40),
    diff_hash: "c".repeat(64),
  };
  const approval = mergeApproval(spec, reviewed);

  assert.deepEqual(
    await policy.verifyMergeApproval({
      spec,
      state: "AWAITING_MERGE_APPROVAL",
      approval,
      ...reviewed,
      usedNonces: new Set(),
    }),
    {
      allowed: true,
      reason_code: "MERGE_APPROVAL_VALID",
      evidence: {
        approval_id: approval.approval_id,
        kind: "merge",
        approver_id: "owner",
        subject_hash: approval.subject_hash,
      },
    },
  );

  assert.equal(
    (
      await policy.verifyMergeApproval({
        spec,
        state: "AWAITING_MERGE_APPROVAL",
        approval,
        reviewed_commit: "d".repeat(40),
        diff_hash: reviewed.diff_hash,
        usedNonces: new Set(),
      })
    ).reason_code,
    "APPROVAL_SUBJECT_MISMATCH",
  );
  assert.equal(
    (
      await policy.verifyMergeApproval({
        spec,
        state: "REVIEWING",
        approval,
        ...reviewed,
        usedNonces: new Set(),
      })
    ).reason_code,
    "APPROVAL_STATE_DENIED",
  );
});

test("admits at most the task budget and global limit of four worker agents", () => {
  const policy = engine();
  const spec = taskSpec();

  assert.deepEqual(
    policy.admitWorkers({
      spec,
      active_workers: [AGENT_ROLES.PLANNER, AGENT_ROLES.CODE_MAPPER],
      requested_roles: [
        AGENT_ROLES.IMPLEMENTER,
        AGENT_ROLES.SECURITY_REVIEWER,
      ],
    }),
    {
      allowed: true,
      reason_code: "WORKER_ADMISSION_ALLOWED",
      evidence: {
        active_workers: 2,
        requested_workers: 2,
        resulting_workers: 4,
        limit: 4,
      },
    },
  );
  assert.equal(
    policy.admitWorkers({
      spec,
      active_workers: [
        AGENT_ROLES.PLANNER,
        AGENT_ROLES.CODE_MAPPER,
        AGENT_ROLES.CORRECTNESS_REVIEWER,
      ],
      requested_roles: [
        AGENT_ROLES.IMPLEMENTER,
        AGENT_ROLES.SECURITY_REVIEWER,
      ],
    }).reason_code,
    "AGENT_LIMIT_EXCEEDED",
  );
  assert.equal(
    policy.admitWorkers({
      spec,
      active_workers: [],
      requested_roles: ["UNKNOWN"],
    }).reason_code,
    "UNKNOWN_ROLE",
  );

  const unsafeBudgetSpec = {
    ...spec,
    budget: {
      ...spec.budget,
      max_agents: 5,
    },
  };
  assert.equal(
    policy.admitWorkers({
      spec: unsafeBudgetSpec,
      active_workers: [],
      requested_roles: [AGENT_ROLES.PLANNER],
    }).reason_code,
    "PHASE1_ENVELOPE_DENIED",
  );
});

test("matches the complete documented role/action matrix", () => {
  const policy = engine();
  const spec = taskSpec();
  const expected = {
    LATTICE_PM: ["READ_REPOSITORY", "SUBMIT_PLAN", "STOP_RUNTIME"],
    PLANNER: ["READ_REPOSITORY", "PLAN_TASK"],
    CODE_MAPPER: ["READ_REPOSITORY", "MAP_CODE"],
    GRAPHIFY: ["READ_REPOSITORY", "MAP_CODE"],
    IMPLEMENTER: ["READ_REPOSITORY", "WRITE_PRODUCT_CODE", "RUN_TESTS"],
    CORRECTNESS_REVIEWER: ["READ_REPOSITORY", "REVIEW_CORRECTNESS"],
    SECURITY_REVIEWER: ["READ_REPOSITORY", "REVIEW_SECURITY"],
    ARCHITECTURE_REVIEWER: ["READ_REPOSITORY", "REVIEW_ARCHITECTURE"],
    INTEGRATOR: ["READ_REPOSITORY", "PREPARE_WORKTREE", "INTEGRATE_GIT"],
  };
  const stateForAction = {
    READ_REPOSITORY: "DRAFT",
    SUBMIT_PLAN: "DRAFT",
    STOP_RUNTIME: "EXECUTING",
    PLAN_TASK: "DRAFT",
    MAP_CODE: "DRAFT",
    WRITE_PRODUCT_CODE: "EXECUTING",
    RUN_TESTS: "EXECUTING",
    REVIEW_CORRECTNESS: "REVIEWING",
    REVIEW_SECURITY: "REVIEWING",
    REVIEW_ARCHITECTURE: "REVIEWING",
    PREPARE_WORKTREE: "PREPARING",
    INTEGRATE_GIT: "MERGING",
  };
  const nonProtectedActions = Object.values(ACTIONS).filter(
    (action) => action in stateForAction,
  );

  for (const [role, allowedActions] of Object.entries(expected)) {
    for (const action of nonProtectedActions) {
      const result = policy.authorizeAgentAction({
        role,
        action,
        state: stateForAction[action],
        spec,
        lease: activeLease(spec),
      });
      assert.equal(
        result.allowed,
        allowedActions.includes(action),
        `${role} ${action}: ${result.reason_code}`,
      );
    }
  }
});

test("rejects expired and stale fencing evidence for Implementer writes", () => {
  const policy = engine();
  const spec = taskSpec();
  const lease = activeLease(spec);

  assert.equal(
    policy.authorizeAgentAction({
      role: AGENT_ROLES.IMPLEMENTER,
      action: ACTIONS.WRITE_PRODUCT_CODE,
      state: "EXECUTING",
      spec,
      lease: { ...lease, expires_at: "2026-07-29T00:09:59.000Z" },
    }).reason_code,
    "WRITER_LEASE_EXPIRED",
  );
  assert.equal(
    policy.authorizeAgentAction({
      role: AGENT_ROLES.IMPLEMENTER,
      action: ACTIONS.WRITE_PRODUCT_CODE,
      state: "EXECUTING",
      spec,
      lease: { ...lease, current_fencing_token: 8 },
    }).reason_code,
    "WRITER_LEASE_INVALID",
  );
});
