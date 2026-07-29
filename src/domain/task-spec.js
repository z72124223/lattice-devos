import { deepFreeze, sha256Canonical } from "./canonical-json.js";
import { DomainError, domainFailure } from "./errors.js";
export { assertAcyclicTaskGraph } from "./task-graph.js";
export {
  ALLOWED_TRANSITIONS,
  TASK_STATES,
  isTransitionAllowed,
  transitionTaskState,
} from "./task-state.js";
export { createTaskPacket } from "./task-packet.js";

const TASK_SPEC_FIELDS = [
  "schema_version",
  "task_id",
  "revision",
  "created_at",
  "created_by",
  "project_id",
  "base_ref",
  "base_commit_sha",
  "goal",
  "non_goals",
  "risk_class",
  "depends_on",
  "scope",
  "acceptance_criteria",
  "verification_commands",
  "required_checks",
  "requested_capabilities",
  "budget",
  "runtime_profile",
  "network_policy",
  "deployment_policy",
  "execution_approval_required",
  "merge_approval_required",
];

const RISK_CLASSES = new Set(["R0", "R1", "R2", "R3"]);
const OPERATIONS = new Set(["create", "modify", "delete", "rename", "typechange"]);
const EVIDENCE_TYPES = new Set(["test", "command", "artifact", "manual"]);
const CHECKS = new Set([
  "build",
  "test",
  "scope",
  "security",
  "architecture",
  "lint",
  "typecheck",
]);
const CAPABILITIES = new Set([
  "READ_REPOSITORY",
  "MAP_CODE",
  "PLAN_TASK",
  "WRITE_PRODUCT_CODE",
  "RUN_TESTS",
  "GIT_WORKTREE",
  "GIT_INTEGRATE",
  "READ_REVIEW",
  "STOP_RUNTIME",
]);

function assertPlainObject(value, field) {
  if (
    value === null ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.getPrototypeOf(value) !== Object.prototype
  ) {
    domainFailure("INVALID_TASK_SPEC", `${field} must be a plain object.`, {
      field,
    });
  }
}

function assertExactFields(value, fields, field) {
  assertPlainObject(value, field);
  const expected = new Set(fields);
  const missing = fields.filter((name) => !(name in value));
  const unexpected = Object.keys(value).filter((name) => !expected.has(name));
  if (missing.length > 0 || unexpected.length > 0) {
    domainFailure("INVALID_TASK_SPEC", `${field} fields do not match the schema.`, {
      field,
      missing,
      unexpected,
    });
  }
}

function nonEmptyString(value, field) {
  if (typeof value !== "string" || value.trim().length === 0 || value.includes("\0")) {
    domainFailure("INVALID_TASK_SPEC", `${field} must be a non-empty string.`, {
      field,
    });
  }
  return value.trim();
}

function stringArray(value, field, { min = 0 } = {}) {
  if (!Array.isArray(value) || value.length < min) {
    domainFailure("INVALID_TASK_SPEC", `${field} must contain at least ${min} item(s).`, {
      field,
    });
  }
  const normalized = value.map((entry, index) =>
    nonEmptyString(entry, `${field}[${index}]`),
  );
  if (new Set(normalized).size !== normalized.length) {
    domainFailure("INVALID_TASK_SPEC", `${field} must not contain duplicates.`, {
      field,
    });
  }
  return normalized;
}

function normalizeScopePath(value, field) {
  const candidate = nonEmptyString(value, field);
  if (
    candidate.includes("\\") ||
    candidate.startsWith("/") ||
    candidate.startsWith("//") ||
    /^[A-Za-z]:/.test(candidate)
  ) {
    domainFailure("INVALID_SCOPE_PATH", `${field} must be repository-relative.`, {
      field,
      path: candidate,
    });
  }
  const segments = candidate.split("/");
  if (
    segments.some(
      (segment) =>
        segment === "" ||
        segment === "." ||
        segment === ".." ||
        segment.includes("\0"),
    )
  ) {
    domainFailure("INVALID_SCOPE_PATH", `${field} contains an unsafe segment.`, {
      field,
      path: candidate,
    });
  }
  return candidate;
}

function normalizeBaseRef(value) {
  const ref = nonEmptyString(value, "base_ref");
  if (
    ref.startsWith("-") ||
    ref.startsWith("/") ||
    ref.endsWith("/") ||
    ref.endsWith(".") ||
    ref.includes("..") ||
    ref.includes("@{") ||
    ref.includes("\\") ||
    ref.includes("//") ||
    /[\s~^:?*\[]/.test(ref)
  ) {
    domainFailure("INVALID_TASK_SPEC", "base_ref is not a safe Git ref.", {
      field: "base_ref",
    });
  }
  return ref;
}

function normalizeAcceptanceCriteria(value) {
  if (!Array.isArray(value) || value.length === 0) {
    domainFailure(
      "INVALID_TASK_SPEC",
      "acceptance_criteria must contain at least one criterion.",
    );
  }
  const normalized = value.map((criterion, index) => {
    const field = `acceptance_criteria[${index}]`;
    assertExactFields(
      criterion,
      ["id", "description", "evidence_type", "expected_result"],
      field,
    );
    const evidenceType = nonEmptyString(
      criterion.evidence_type,
      `${field}.evidence_type`,
    );
    if (!EVIDENCE_TYPES.has(evidenceType)) {
      domainFailure("INVALID_TASK_SPEC", `${field}.evidence_type is unknown.`);
    }
    return {
      id: nonEmptyString(criterion.id, `${field}.id`),
      description: nonEmptyString(criterion.description, `${field}.description`),
      evidence_type: evidenceType,
      expected_result: nonEmptyString(
        criterion.expected_result,
        `${field}.expected_result`,
      ),
    };
  });
  const ids = normalized.map((criterion) => criterion.id);
  if (new Set(ids).size !== ids.length) {
    domainFailure("INVALID_TASK_SPEC", "acceptance_criteria IDs must be unique.");
  }
  return normalized;
}

function normalizeScope(value) {
  assertExactFields(
    value,
    ["allowed_paths", "forbidden_paths", "allowed_operations"],
    "scope",
  );
  const allowedPaths = stringArray(value.allowed_paths, "scope.allowed_paths", {
    min: 1,
  }).map((entry, index) =>
    normalizeScopePath(entry, `scope.allowed_paths[${index}]`),
  );
  if (allowedPaths.some((entry) => entry === ".git" || entry.startsWith(".git/"))) {
    domainFailure("INVALID_SCOPE_PATH", ".git cannot be an allowed path.");
  }
  const forbiddenPaths = stringArray(
    value.forbidden_paths,
    "scope.forbidden_paths",
    { min: 1 },
  ).map((entry, index) =>
    normalizeScopePath(entry, `scope.forbidden_paths[${index}]`),
  );
  if (!forbiddenPaths.includes(".git/**")) {
    domainFailure(
      "PHASE1_SAFETY_ENVELOPE_REQUIRED",
      "Phase 1 scope must explicitly forbid .git/**.",
    );
  }
  const allowedOperations = stringArray(
    value.allowed_operations,
    "scope.allowed_operations",
    { min: 1 },
  );
  for (const operation of allowedOperations) {
    if (!OPERATIONS.has(operation)) {
      domainFailure("INVALID_TASK_SPEC", `Unknown scope operation: ${operation}.`);
    }
  }
  return {
    allowed_paths: allowedPaths,
    forbidden_paths: forbiddenPaths,
    allowed_operations: allowedOperations,
  };
}

function normalizeBudget(value) {
  assertExactFields(
    value,
    [
      "max_agents",
      "max_duration_seconds",
      "max_attempts",
      "max_model_calls",
      "max_external_cost",
    ],
    "budget",
  );
  const integerFields = [
    "max_agents",
    "max_duration_seconds",
    "max_attempts",
    "max_model_calls",
  ];
  for (const field of integerFields) {
    if (!Number.isInteger(value[field]) || value[field] < 0) {
      domainFailure("INVALID_TASK_SPEC", `budget.${field} must be a non-negative integer.`);
    }
  }
  if (
    value.max_agents < 1 ||
    value.max_agents > 4 ||
    value.max_duration_seconds < 1 ||
    value.max_attempts < 1 ||
    value.max_model_calls !== 0 ||
    value.max_external_cost !== 0
  ) {
    domainFailure(
      "PHASE1_SAFETY_ENVELOPE_REQUIRED",
      "Phase 1 budget must be local, bounded, and zero-cost.",
    );
  }
  return {
    max_agents: value.max_agents,
    max_duration_seconds: value.max_duration_seconds,
    max_attempts: value.max_attempts,
    max_model_calls: value.max_model_calls,
    max_external_cost: value.max_external_cost,
  };
}

function normalizeEnumArray(value, field, allowed, { min = 0 } = {}) {
  const normalized = stringArray(value, field, { min });
  for (const entry of normalized) {
    if (!allowed.has(entry)) {
      domainFailure("INVALID_TASK_SPEC", `${field} contains unknown value '${entry}'.`);
    }
  }
  return normalized;
}

function validatePhase1Envelope(spec) {
  if (
    spec.runtime_profile !== "fake" ||
    spec.network_policy !== "deny" ||
    spec.deployment_policy !== "deny" ||
    spec.execution_approval_required !== true ||
    spec.merge_approval_required !== true
  ) {
    domainFailure(
      "PHASE1_SAFETY_ENVELOPE_REQUIRED",
      "Phase 1 requires fake runtime, denied network/deployment, and both approvals.",
    );
  }
}

export function createTaskSpec(input) {
  assertExactFields(input, TASK_SPEC_FIELDS, "task_spec");

  if (input.schema_version !== "1.0") {
    domainFailure("UNSUPPORTED_SCHEMA_VERSION", "Only Task Spec schema 1.0 is supported.");
  }
  const taskId = nonEmptyString(input.task_id, "task_id");
  if (!/^TASK-[A-Z0-9][A-Z0-9_-]{2,63}$/.test(taskId)) {
    domainFailure("INVALID_TASK_SPEC", "task_id has an invalid format.");
  }
  if (!Number.isInteger(input.revision) || input.revision < 1) {
    domainFailure("INVALID_TASK_SPEC", "revision must be a positive integer.");
  }
  const createdAt = nonEmptyString(input.created_at, "created_at");
  if (!Number.isFinite(Date.parse(createdAt))) {
    domainFailure("INVALID_TASK_SPEC", "created_at must be an ISO timestamp.");
  }
  const projectId = nonEmptyString(input.project_id, "project_id");
  if (!/^[a-z0-9][a-z0-9._-]{1,63}$/.test(projectId)) {
    domainFailure("INVALID_TASK_SPEC", "project_id has an invalid format.");
  }
  const commit = nonEmptyString(input.base_commit_sha, "base_commit_sha").toLowerCase();
  if (!/^[a-f0-9]{40,64}$/.test(commit)) {
    domainFailure("INVALID_TASK_SPEC", "base_commit_sha must be a Git object hash.");
  }
  if (!RISK_CLASSES.has(input.risk_class)) {
    domainFailure("INVALID_TASK_SPEC", "risk_class is unknown.");
  }
  const dependsOn = stringArray(input.depends_on, "depends_on");
  if (dependsOn.includes(taskId)) {
    domainFailure("TASK_DEPENDENCY_CYCLE", "A task cannot depend on itself.");
  }
  const requestedCapabilities = normalizeEnumArray(
    input.requested_capabilities,
    "requested_capabilities",
    CAPABILITIES,
    { min: 1 },
  );
  const requiredChecks = normalizeEnumArray(
    input.required_checks,
    "required_checks",
    CHECKS,
    { min: 1 },
  );
  const normalized = {
    schema_version: "1.0",
    task_id: taskId,
    revision: input.revision,
    created_at: new Date(createdAt).toISOString(),
    created_by: nonEmptyString(input.created_by, "created_by"),
    project_id: projectId,
    base_ref: normalizeBaseRef(input.base_ref),
    base_commit_sha: commit,
    goal: nonEmptyString(input.goal, "goal"),
    non_goals: stringArray(input.non_goals, "non_goals", { min: 1 }),
    risk_class: input.risk_class,
    depends_on: dependsOn,
    scope: normalizeScope(input.scope),
    acceptance_criteria: normalizeAcceptanceCriteria(input.acceptance_criteria),
    verification_commands: stringArray(
      input.verification_commands,
      "verification_commands",
      { min: 1 },
    ),
    required_checks: requiredChecks,
    requested_capabilities: requestedCapabilities,
    budget: normalizeBudget(input.budget),
    runtime_profile: input.runtime_profile,
    network_policy: input.network_policy,
    deployment_policy: input.deployment_policy,
    execution_approval_required: input.execution_approval_required,
    merge_approval_required: input.merge_approval_required,
  };
  validatePhase1Envelope(normalized);
  const specHash = sha256Canonical(normalized);
  return deepFreeze({ ...normalized, spec_hash: specHash });
}

export { DomainError };
