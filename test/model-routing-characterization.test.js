import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const testDirectory = path.dirname(fileURLToPath(import.meta.url));
const fixturePath = path.join(
  testDirectory,
  "fixtures",
  "issue-008-model-routing.characterization.json",
);

const expectedCapabilityFields = [
  "task_classes",
  "tools",
  "input_modalities",
  "output_modalities",
  "reasoning_efforts",
  "context_limits",
];

const expectedOperationalFields = [
  "quota",
  "cost",
  "concurrency",
  "health",
  "lifecycle",
  "primary_standby_compatibility",
];

const expectedProfiles = [
  {
    name: "守望",
    rank: 0,
    intent: "低風險觀察、分類、摘要與不寫入的狀態檢查",
    risk_floor: "R0",
    quality_floor: "basic",
    reasoning_profile: "lowest verified effort that satisfies the task",
    upgrade_triggers: [
      "required evidence is missing",
      "verification fails",
      "scope becomes writable",
    ],
    allowed_upgrade: "快速",
    deescalation: "future task only after evaluation evidence",
  },
  {
    name: "快速",
    rank: 1,
    intent: "邊界清楚、可逆且具聚焦驗證的單一工作切片",
    risk_floor: "R1",
    quality_floor: "bounded",
    reasoning_profile: "low latency within verified quality bounds",
    upgrade_triggers: [
      "one focused verification failure",
      "conflicting evidence",
      "scope crosses one module boundary",
    ],
    allowed_upgrade: "標準",
    deescalation: "future task only after evaluation evidence",
  },
  {
    name: "標準",
    rank: 2,
    intent: "一般開發、除錯與具明確驗收的單模組變更",
    risk_floor: "R2",
    quality_floor: "production-capable",
    reasoning_profile: "balanced verified effort",
    upgrade_triggers: [
      "repeated focused verification failure",
      "cross-module diagnosis is required",
      "evidence freshness is disputed",
    ],
    allowed_upgrade: "深度",
    deescalation: "future task only after evaluation evidence",
  },
  {
    name: "深度",
    rank: 3,
    intent: "跨模組根因分析、架構權衡與高不確定性工作",
    risk_floor: "R3",
    quality_floor: "high",
    reasoning_profile: "high verified effort with bounded retries",
    upgrade_triggers: [
      "security or durable authority boundary is reached",
      "independent review finds a major issue",
      "actual runtime setting conflicts with the resolved route",
    ],
    allowed_upgrade: "關鍵",
    deescalation: "future task only after evaluation evidence",
  },
  {
    name: "關鍵",
    rank: 4,
    intent: "安全、資料庫、租約、writer、公開契約與整合授權邊界",
    risk_floor: "R4",
    quality_floor: "critical",
    reasoning_profile:
      "highest verified level required by policy; no name-based inference",
    upgrade_triggers: [],
    allowed_upgrade: null,
    deescalation:
      "denied for the in-flight task; a later task needs fresh evaluation evidence",
  },
];

const expectedRoutingCrosswalk = [
  {
    profile: "守望",
    admit_when: [
      "read-only",
      "deterministic classification exists",
      "no protected action",
    ],
    deny_when: [
      "write authority is required",
      "capability evidence is missing",
    ],
  },
  {
    profile: "快速",
    admit_when: [
      "single reversible slice",
      "focused test exists",
      "risk is low",
    ],
    deny_when: [
      "shared public contract changes",
      "durable writer authority is involved",
    ],
  },
  {
    profile: "標準",
    admit_when: [
      "bounded module ownership",
      "normal code and review gates",
      "verified capability match",
    ],
    deny_when: [
      "cross-module conflict is unresolved",
      "actual route cannot be observed",
    ],
  },
  {
    profile: "深度",
    admit_when: [
      "cross-module reasoning is necessary",
      "retry and cost ceilings are explicit",
      "evidence is current",
    ],
    deny_when: [
      "critical authority is implied but not granted",
      "independent verification is unavailable",
    ],
  },
  {
    profile: "關鍵",
    admit_when: [
      "protected boundary is explicitly authorized",
      "strongest required capability is verified",
      "independent review and actual-route evidence are available",
    ],
    deny_when: [
      "cost or latency pressure would lower the risk floor",
      "actual setting mismatches the resolved route",
      "standby compatibility is unknown",
    ],
  },
];

const expectedReasonCodes = [
  {
    category: "selection",
    codes: [
      "ROUTE_REPRODUCIBILITY_PIN",
      "ROUTE_RISK_FLOOR",
      "ROUTE_PROFILE_REQUESTED",
      "ROUTE_CAPABILITY_MATCH",
      "ROUTE_PRIMARY_HEALTHY",
      "ROUTE_STANDBY_COMPATIBLE",
      "ROUTE_COST_LATENCY_TIEBREAK",
    ],
  },
  {
    category: "escalation",
    codes: [
      "ROUTE_VERIFICATION_FAILED",
      "ROUTE_SCOPE_EXPANDED",
      "ROUTE_RISK_BOUNDARY_CROSSED",
      "ROUTE_CONFLICTING_EVIDENCE",
      "ROUTE_MAJOR_REVIEW_FINDING",
      "ROUTE_SESSION_ABNORMAL_EXIT",
    ],
  },
  {
    category: "block",
    codes: [
      "ROUTE_CAPABILITY_EVIDENCE_MISSING",
      "ROUTE_CAPABILITY_EVIDENCE_STALE",
      "ROUTE_PROVIDER_UNAVAILABLE",
      "ROUTE_NO_COMPATIBLE_STANDBY",
      "ROUTE_BUDGET_EXHAUSTED",
      "ROUTE_QUALITY_FLOOR_UNMET",
      "ROUTE_ACTUAL_SETTING_MISMATCH",
      "ROUTE_ESCALATION_LIMIT_REACHED",
      "ROUTE_PROVIDER_SCOPE_DENIED",
      "ROUTE_INDEPENDENT_REVIEW_REQUIRED",
    ],
  },
  {
    category: "deescalation",
    codes: [
      "ROUTE_FUTURE_TASK_EVAL_PROVEN",
      "ROUTE_FUTURE_TASK_LOW_RISK_RETRY",
    ],
  },
];

const expectedEvidenceFields = {
  requested: [
    "task_classes",
    "required_tools",
    "input_modalities",
    "output_modalities",
    "governance_profile",
    "risk_class",
    "quality_floor",
    "maximum_reasoning_effort",
    "max_model_calls",
    "latency_ceiling",
    "cost_ceiling",
    "allowed_provider_scope",
    "reproducibility_pin",
    "escalation_allowed",
    "max_escalations",
  ],
  resolved: [
    "request_digest",
    "provider_id",
    "provider_surface",
    "model_id",
    "model_version",
    "reasoning_effort",
    "capability_snapshot_ref",
    "policy_version",
    "reason_codes",
    "primary_or_standby",
    "model_call_budget",
    "escalation_budget",
    "boundary_conformance",
    "resolved_at",
  ],
  actual: [
    "request_digest",
    "resolution_digest",
    "provider_id",
    "provider_surface",
    "model_id",
    "model_version",
    "reasoning_effort",
    "service_tier",
    "model_calls",
    "escalations",
    "usage",
    "rate_limit_snapshot",
    "run_thread_binding",
    "evidence_state",
    "source_ref",
    "observed_at",
    "conformance",
  ],
};

const resolvedActualIdentityFields = [
  "provider_id",
  "provider_surface",
  "model_id",
  "model_version",
  "reasoning_effort",
];

const evidenceEvaluationTime = Date.parse("2026-08-10T02:00:00+08:00");

function evidenceDecision(observation) {
  if (observation.evidence_state !== "VERIFIED") {
    return {
      disposition: "BLOCK",
      reason_code: "ROUTE_CAPABILITY_EVIDENCE_MISSING",
    };
  }
  if (observation.value === null || observation.value === undefined) {
    return {
      disposition: "BLOCK",
      reason_code: "ROUTE_CAPABILITY_EVIDENCE_MISSING",
    };
  }
  if (observation.freshness.state === "PINNED") {
    return { disposition: "ALLOW", reason_code: "ROUTE_CAPABILITY_MATCH" };
  }
  const validUntil = Date.parse(observation.freshness.valid_until);
  if (
    observation.freshness.state !== "CURRENT" ||
    Number.isNaN(validUntil) ||
    validUntil <= evidenceEvaluationTime
  ) {
    return {
      disposition: "BLOCK",
      reason_code: "ROUTE_CAPABILITY_EVIDENCE_STALE",
    };
  }
  return { disposition: "ALLOW", reason_code: "ROUTE_CAPABILITY_MATCH" };
}

function actualEvidenceDecision({ resolved, actual }) {
  const hasAllRequiredFields =
    actual &&
    expectedEvidenceFields.actual.every((field) =>
      Object.prototype.hasOwnProperty.call(actual, field),
    );
  const allRequiredValuesPresent =
    hasAllRequiredFields &&
    expectedEvidenceFields.actual.every(
      (field) => actual[field] !== null && actual[field] !== undefined,
    );
  const nonEmptyStringFields = [
    "request_digest",
    "resolution_digest",
    "provider_id",
    "provider_surface",
    "model_id",
    "model_version",
    "reasoning_effort",
    "service_tier",
    "source_ref",
    "conformance",
  ];
  const hasTypedValues =
    allRequiredValuesPresent &&
    nonEmptyStringFields.every(
      (field) => typeof actual[field] === "string" && actual[field].length > 0,
    ) &&
    ["model_calls", "escalations"].every(
      (field) => Number.isInteger(actual[field]) && actual[field] >= 0,
    ) &&
    ["usage", "rate_limit_snapshot", "run_thread_binding"].every(
      (field) =>
        typeof actual[field] === "object" && !Array.isArray(actual[field]),
    ) &&
    typeof actual.observed_at === "string" &&
    Number.isNaN(Date.parse(actual.observed_at)) === false;
  const hasCompleteResolvedIdentity =
    resolved &&
    resolvedActualIdentityFields.every(
      (field) =>
        Object.prototype.hasOwnProperty.call(resolved, field) &&
        typeof resolved[field] === "string" &&
        resolved[field].length > 0,
    );
  const resolvedMatchesActual =
    hasTypedValues &&
    hasCompleteResolvedIdentity &&
    resolvedActualIdentityFields.every(
      (field) => resolved[field] === actual[field],
    );
  if (
    !hasTypedValues ||
    !resolvedMatchesActual ||
    actual.evidence_state !== "VERIFIED" ||
    actual.conformance !== "MATCH" ||
    typeof actual.model_id !== "string" ||
    actual.model_id.length === 0 ||
    !/^(?:provider|runtime):/u.test(actual.source_ref ?? "")
  ) {
    return {
      disposition: "BLOCK",
      reason_code: "ROUTE_ACTUAL_SETTING_MISMATCH",
    };
  }
  return {
    disposition: "ALLOW",
    reason_code: "ROUTE_CAPABILITY_MATCH",
  };
}

function assertTypedObservationValue(observation) {
  const { value, value_type: valueType } = observation;
  if (value === null) {
    return;
  }
  if (valueType === "string_set") {
    assert.ok(Array.isArray(value));
    value.forEach((item) => assert.equal(typeof item, "string"));
    return;
  }
  if (["catalog", "compatibility_matrix", "integer_bounds", "record"].includes(valueType)) {
    assert.equal(Array.isArray(value), false);
    assert.equal(typeof value, "object");
    return;
  }
  if (valueType === "status") {
    assert.equal(typeof value, "string");
    return;
  }
  assert.fail(`unhandled value_type: ${valueType}`);
}

const forbiddenNormalizedKeys = new Set([
  "apikey",
  "credential",
  "credentials",
  "secret",
  "secrets",
  "password",
  "passwd",
  "accesstoken",
  "refreshtoken",
  "authtoken",
  "bearertoken",
  "tokenvalue",
  "token",
  "tokens",
  "apitoken",
  "sessiontoken",
  "githubtoken",
  "idtoken",
  "personalaccesstoken",
  "authorization",
  "proxyauthorization",
  "cookie",
  "cookies",
  "sessionkey",
  "privatekey",
  "encryptionkey",
  "prompt",
  "prompts",
  "messages",
  "messagehistory",
  "conversation",
  "transcript",
  "analysis",
  "thoughts",
  "cot",
  "rawprompt",
  "fullprompt",
  "privateprompt",
  "systemprompt",
  "developerprompt",
  "userprompt",
  "promptcontent",
  "promptpayload",
  "prompttext",
  "hiddenreasoning",
  "privatereasoning",
  "rawreasoning",
  "reasoningcontent",
  "reasoningtrace",
  "reasoningpayload",
  "chainofthought",
  "cottrace",
]);

const sensitiveValuePatterns = [
  /\bsk-[A-Za-z0-9_-]{12,}\b/,
  /\bBearer\s+[A-Za-z0-9._~-]{12,}\b/i,
  /\bgh[pousr]_[A-Za-z0-9]{20,}\b/,
  /\bxox[baprs]-[A-Za-z0-9-]{20,}\b/,
  /\bAIza[0-9A-Za-z_-]{20,}\b/,
  /\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b/,
  /-----BEGIN [A-Z ]*PRIVATE KEY-----/,
  /\b(?:api[_-]?key|password|passwd|secret)\s*[:=]\s*[^\s,]{6,}/i,
];

function isForbiddenEvidenceKey(normalizedKey) {
  return (
    forbiddenNormalizedKeys.has(normalizedKey) ||
    /(?:credential|secret|password|passwd)/u.test(normalizedKey) ||
    /^(?:api)?key(?:value)?$/u.test(normalizedKey) ||
    /(?:access|refresh|auth|bearer)token/u.test(normalizedKey) ||
    /token(?:s|value|data)?$/u.test(normalizedKey) ||
    /(?:raw|full|private|system|developer|user)prompt/u.test(normalizedKey) ||
    /prompt(?:content|payload|text|body|messages)$/u.test(normalizedKey) ||
    /(?:hidden|private|raw)reasoning/u.test(normalizedKey) ||
    /reasoning(?:content|trace|payload|text|tokens)$/u.test(normalizedKey) ||
    /(?:chainofthought|cottrace)/u.test(normalizedKey)
  );
}

function assertNoSensitivePayload(value, location = "fixture") {
  if (Array.isArray(value)) {
    value.forEach((child, index) =>
      assertNoSensitivePayload(child, `${location}[${index}]`),
    );
    return;
  }
  if (value && typeof value === "object") {
    for (const [key, child] of Object.entries(value)) {
      const normalizedKey = key.replace(/[^a-z0-9]/giu, "").toLowerCase();
      assert.equal(
        isForbiddenEvidenceKey(normalizedKey),
        false,
        `forbidden evidence key at ${location}: ${key}`,
      );
      assertNoSensitivePayload(child, `${location}.${key}`);
    }
    return;
  }
  if (typeof value === "string") {
    for (const pattern of sensitiveValuePatterns) {
      assert.equal(
        pattern.test(value),
        false,
        `sensitive evidence value at ${location}`,
      );
    }
  }
}

test("Issue #8 routing characterization is provider-neutral and fail-closed", async () => {
  const fixture = JSON.parse(await readFile(fixturePath, "utf8"));

  assert.deepEqual(fixture.identity, {
    github_issue: 8,
    canonical_id: "model-routing",
    internal_task_041_reused: false,
  });
  assert.deepEqual(fixture.provenance, {
    stable_repo_commit: "8828d2b88faece6b399258744eea4ff8d46f0bea",
    task038_dirty_tree_authoritative: false,
  });

  assert.deepEqual(fixture.capability_model.capability_fields, expectedCapabilityFields);
  assert.deepEqual(fixture.capability_model.operational_fields, expectedOperationalFields);
  const expectedObservationFields = [
    ...expectedCapabilityFields,
    ...expectedOperationalFields,
  ].sort();
  const validEvidenceStates = new Set(["VERIFIED", "UNVERIFIED", "UNKNOWN"]);
  const validFreshnessStates = new Set(["CURRENT", "PINNED", "STALE", "UNKNOWN"]);
  const validValueTypes = new Set([
    "catalog",
    "compatibility_matrix",
    "integer_bounds",
    "record",
    "status",
    "string_set",
  ]);

  for (const surface of fixture.evidence_inventory) {
    assert.equal("evidence_state" in surface, false, `${surface.surface} is surface-scoped`);
    assert.deepEqual(Object.keys(surface.observations).sort(), expectedObservationFields);
    for (const [field, observation] of Object.entries(surface.observations)) {
      assert.deepEqual(Object.keys(observation).sort(), [
        "evidence_state",
        "freshness",
        "kind",
        "observed_at",
        "source_ref",
        "value",
        "value_type",
      ]);
      assert.equal(
        observation.kind,
        expectedCapabilityFields.includes(field) ? "capability" : "operational",
      );
      assert.ok(validValueTypes.has(observation.value_type));
      assertTypedObservationValue(observation);
      assert.ok(validEvidenceStates.has(observation.evidence_state));
      assert.match(observation.source_ref, /^(official|repo|runtime):/);
      assert.equal(Number.isNaN(Date.parse(observation.observed_at)), false);
      assert.deepEqual(Object.keys(observation.freshness).sort(), [
        "checked_at",
        "state",
        "valid_until",
      ]);
      assert.ok(validFreshnessStates.has(observation.freshness.state));
      assert.equal(Number.isNaN(Date.parse(observation.freshness.checked_at)), false);
      if (observation.freshness.valid_until !== null) {
        assert.equal(
          Number.isNaN(Date.parse(observation.freshness.valid_until)),
          false,
        );
      }
      if (observation.evidence_state === "VERIFIED") {
        assert.notEqual(observation.value, null);
        assert.ok(["CURRENT", "PINNED"].includes(observation.freshness.state));
      }
    }
  }

  const codexReasoning = fixture.evidence_inventory.find(
    ({ surface }) => surface === "Codex App and app-server",
  ).observations.reasoning_efforts;
  assert.deepEqual(codexReasoning.value, {
    "gpt-5.6-sol": ["low", "medium", "high", "xhigh", "max", "ultra"],
    "gpt-5.6-terra": ["low", "medium", "high", "xhigh", "max", "ultra"],
    "gpt-5.6-luna": ["low", "medium", "high", "xhigh", "max"],
    "gpt-5.5": ["low", "medium", "high", "xhigh"],
    "gpt-5.4": ["low", "medium", "high", "xhigh"],
    "gpt-5.4-mini": ["low", "medium", "high", "xhigh"],
    "gpt-5.3-codex-spark": ["low", "medium", "high", "xhigh"],
  });

  assert.deepEqual(
    fixture.evidence_admission_cases.map(({ observation }) =>
      evidenceDecision(observation),
    ),
    fixture.evidence_admission_cases.map(({ expected }) => expected),
  );
  assert.deepEqual(
    fixture.evidence_admission_cases.map(({ id }) => id),
    [
      "verified-current-allows",
      "verified-pinned-allows",
      "unknown-blocks",
      "unverified-blocks",
      "stale-blocks",
      "unknown-freshness-blocks",
      "current-expired-blocks",
      "current-missing-expiry-blocks",
      "verified-null-blocks",
    ],
  );
  const completeActualCase = fixture.actual_evidence_cases.find(
    ({ id }) => id === "runtime-actual-allows",
  );
  const completeActual = completeActualCase.actual;
  for (const field of expectedEvidenceFields.actual) {
    const partialActual = structuredClone(completeActual);
    delete partialActual[field];
    assert.deepEqual(
      actualEvidenceDecision({
        resolved: completeActualCase.resolved,
        actual: partialActual,
      }),
      {
        disposition: "BLOCK",
        reason_code: "ROUTE_ACTUAL_SETTING_MISMATCH",
      },
      `missing actual field should block: ${field}`,
    );
    const nullActual = structuredClone(completeActual);
    nullActual[field] = null;
    assert.deepEqual(
      actualEvidenceDecision({
        resolved: completeActualCase.resolved,
        actual: nullActual,
      }),
      {
        disposition: "BLOCK",
        reason_code: "ROUTE_ACTUAL_SETTING_MISMATCH",
      },
      `null actual field should block: ${field}`,
    );
  }
  for (const field of resolvedActualIdentityFields) {
    const mismatchedActual = structuredClone(completeActual);
    mismatchedActual[field] = `${completeActual[field]}-mismatch`;
    assert.deepEqual(
      actualEvidenceDecision({
        resolved: completeActualCase.resolved,
        actual: mismatchedActual,
      }),
      {
        disposition: "BLOCK",
        reason_code: "ROUTE_ACTUAL_SETTING_MISMATCH",
      },
      `resolved/actual identity mismatch should block: ${field}`,
    );

    const partialResolved = structuredClone(completeActualCase.resolved);
    delete partialResolved[field];
    assert.deepEqual(
      actualEvidenceDecision({ resolved: partialResolved, actual: completeActual }),
      {
        disposition: "BLOCK",
        reason_code: "ROUTE_ACTUAL_SETTING_MISMATCH",
      },
      `missing resolved identity should block: ${field}`,
    );
  }
  const mismatchedConformance = structuredClone(completeActual);
  mismatchedConformance.conformance = "MISMATCH";
  assert.deepEqual(
    actualEvidenceDecision({
      resolved: completeActualCase.resolved,
      actual: mismatchedConformance,
    }),
    {
      disposition: "BLOCK",
      reason_code: "ROUTE_ACTUAL_SETTING_MISMATCH",
    },
  );
  const numericObservedAt = structuredClone(completeActual);
  numericObservedAt.observed_at = 0;
  assert.deepEqual(
    actualEvidenceDecision({
      resolved: completeActualCase.resolved,
      actual: numericObservedAt,
    }),
    {
      disposition: "BLOCK",
      reason_code: "ROUTE_ACTUAL_SETTING_MISMATCH",
    },
  );
  assert.deepEqual(
    evidenceDecision(
      fixture.evidence_inventory.find(
        ({ surface }) => surface === "Local model providers",
      ).observations.task_classes,
    ),
    {
      disposition: "BLOCK",
      reason_code: "ROUTE_CAPABILITY_EVIDENCE_MISSING",
    },
  );

  assert.deepEqual(fixture.governance_profiles, expectedProfiles);
  assert.deepEqual(fixture.routing_crosswalk, expectedRoutingCrosswalk);
  const serializedProfiles = JSON.stringify(fixture.governance_profiles).toLowerCase();
  for (const providerOrModelName of ["codex", "gpt-", "hermes", "ollama", "openai"]) {
    assert.equal(
      serializedProfiles.includes(providerOrModelName),
      false,
      `governance profile leaked provider/model identity: ${providerOrModelName}`,
    );
  }

  assert.deepEqual(Object.keys(fixture.evidence_shape), ["requested", "resolved", "actual"]);
  for (const [stage, fields] of Object.entries(expectedEvidenceFields)) {
    assert.deepEqual(fixture.evidence_shape[stage].required_fields, fields);
  }
  assert.deepEqual(fixture.evidence_shape.requested.immutable_boundary_fields, [
    "task_classes",
    "required_tools",
    "input_modalities",
    "output_modalities",
    "governance_profile",
    "risk_class",
    "quality_floor",
    "maximum_reasoning_effort",
    "max_model_calls",
    "latency_ceiling",
    "cost_ceiling",
    "allowed_provider_scope",
    "reproducibility_pin",
    "escalation_allowed",
    "max_escalations",
  ]);
  assert.equal(
    fixture.evidence_shape.actual.rule,
    "Only runtime or provider evidence populates actual values. Missing fields stay UNKNOWN and never inherit requested or resolved values.",
  );
  assert.deepEqual(fixture.actual_origin_policy, {
    allowed_source_prefixes: ["provider:", "runtime:"],
    forbidden_source_prefixes: ["requested:", "resolved:"],
    resolved_actual_identity_fields: resolvedActualIdentityFields,
    required_conformance: "MATCH",
    missing_actual_disposition: "BLOCK",
    substitution_reason_code: "ROUTE_ACTUAL_SETTING_MISMATCH",
  });
  assert.deepEqual(
    fixture.actual_evidence_cases.map((characterizationCase) =>
      actualEvidenceDecision(characterizationCase),
    ),
    fixture.actual_evidence_cases.map(({ expected }) => expected),
  );
  assert.deepEqual(
    fixture.actual_evidence_cases.map(({ id }) => id),
    [
      "runtime-actual-allows",
      "missing-actual-blocks-with-requested-resolved-present",
      "requested-substitution-blocks",
      "resolved-substitution-blocks",
      "unknown-actual-blocks",
    ],
  );
  for (const stage of ["requested", "resolved"]) {
    for (const field of expectedEvidenceFields.actual) {
      assert.deepEqual(
        actualEvidenceDecision({
          resolved: completeActualCase.resolved,
          actual: {
            model_id: "copied-value",
            evidence_state: "VERIFIED",
            source_ref: `${stage}:${field}`,
          },
        }),
        {
          disposition: "BLOCK",
          reason_code: "ROUTE_ACTUAL_SETTING_MISMATCH",
        },
      );
    }
  }

  assert.deepEqual(fixture.reason_codes, expectedReasonCodes);
  assert.equal(
    new Set(fixture.reason_codes.flatMap(({ codes }) => codes)).size,
    25,
  );

  assert.deepEqual(fixture.transition_policy, {
    deterministic_rules_first: true,
    model_may_self_authorize: false,
    missing_capability_evidence: "BLOCK",
    stale_capability_evidence: "BLOCK",
    critical_profile_downgrade: "DENY",
    in_flight_deescalation: "DENY",
    max_same_route_retries: 1,
    max_automatic_escalations: 2,
    escalation_step:
      "one profile at a time unless a deterministic risk floor requires a higher profile",
    budget_rule:
      "cost and latency may break ties but cannot lower the risk or quality floor",
    actual_mismatch_rule:
      "fail closed for reproducibility pins and critical work; otherwise record mismatch and require policy disposition",
    deescalation_rule:
      "only a later task may start lower after current evaluation evidence proves the lower profile meets quality and risk floors",
  });

  const allowedClassifications = new Set([
    "acceptance_reproducibility_pin",
    "temporary_implementation_constraint",
    "general_route_candidate",
  ]);
  assert.ok(fixture.fixed_setting_inventory.length > 0);
  for (const setting of fixture.fixed_setting_inventory) {
    assert.ok(allowedClassifications.has(setting.classification));
    assert.match(setting.source_ref, /^(repo|official|runtime):/);
    assert.doesNotMatch(setting.source_ref, /worktree:feature\/task-038|run-task038/i);
  }
  assert.deepEqual(fixture.pending_evidence, [
    {
      scope: "TASK-038 fixed model and reasoning settings",
      evidence_state: "UNVERIFIED",
      disposition: "WAIT_TASK038_STABLE",
      source_ref: "runtime:task-038-dirty-worktree-observation-2026-08-10",
      observed_at: "2026-08-10T02:00:00+08:00",
      reason:
        "The dirty worktree has no stable blob or line address and is not authoritative evidence.",
    },
  ]);

  const fixedSettingRefs = new Set(
    fixture.fixed_setting_inventory.map(({ source_ref }) => source_ref),
  );
  for (const requiredRef of [
    "repo:crates/lattice-codex-adapter/tests/process.rs:212",
    "repo:crates/lattice-hermes-adapter/tests/reflection_api.rs:183",
    "repo:crates/lattice-hermes-adapter/tests/hermes_sandbox_runner_fixture.py:399",
    "repo:apps/lattice-runtime/src/composition.rs:98",
    "repo:apps/lattice-runtime/src/composition.rs:100",
  ]) {
    assert.ok(fixedSettingRefs.has(requiredRef), `missing fixed-setting evidence: ${requiredRef}`);
  }
  const hermesCompatibilityIdentity = fixture.fixed_setting_inventory.find(
    ({ source_ref }) => source_ref === "repo:apps/lattice-runtime/src/composition.rs:98",
  );
  assert.equal(
    hermesCompatibilityIdentity.classification,
    "acceptance_reproducibility_pin",
  );
  assert.match(hermesCompatibilityIdentity.setting, /not an underlying LLM route/);
  assert.equal(
    fixture.fixed_setting_inventory.find(
      ({ source_ref }) => source_ref === "repo:apps/lattice-runtime/src/composition.rs:100",
    ).classification,
    "temporary_implementation_constraint",
  );

  assertNoSensitivePayload(fixture);
  for (const key of [
    "apiKey",
    "credentials",
    "access_token",
    "token",
    "apiToken",
    "sessionToken",
    "githubToken",
    "authorization",
    "privateKey",
    "prompt",
    "rawPrompt",
    "systemPromptText",
    "analysis",
    "hiddenReasoningTrace",
    "chainOfThought",
    "reasoning_content",
  ]) {
    assert.throws(
      () => assertNoSensitivePayload({ [key]: "redacted" }),
      /forbidden evidence key/,
    );
  }
  for (const value of [
    "sk-" + "examplecredential123456",
    "Bearer " + "examplecredential123456",
    "-----BEGIN " + "PRIVATE KEY-----",
    "password" + "=examplecredential",
    "ghp_" + "examplecredential1234567890",
    "xoxb-" + "examplecredential1234567890",
    "AIza" + "examplecredential1234567890",
    "eyJ" + "abcdefghijk" + "." + "abcdefghijk" + "." + "abcdefghijk",
  ]) {
    assert.throws(
      () => assertNoSensitivePayload({ note: value }),
      /sensitive evidence value/,
    );
  }
});
