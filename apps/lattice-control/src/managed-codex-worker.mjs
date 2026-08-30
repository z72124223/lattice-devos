import { createHash } from "node:crypto";
import path from "node:path";

const PACKET_SCHEMA = "lattice.foreman-attempt-packet/1.0";
const EVENT_SCHEMA = "lattice.managed-codex-worker-event/1.0";
const AUTH_CONTEXT_SCHEMA = "lattice.managed-codex-auth-context/1.0";
const AUTH_READINESS_SCHEMA = "lattice.managed-codex-auth-readiness/1.0";
const AUTH_CONTEXT_KEYS = new Set(["schema", "codex_home_digest", "config_digest"]);
const CONNECTOR_AUTH_READINESS_KEYS = new Set([
  "schema",
  "ready",
  "authMode",
  "appServerGeneration",
  "appServerSessionId",
]);
const ALLOWED_MODELS = new Set([
  "gpt-5.6-luna",
  "gpt-5.6-terra",
  "gpt-5.6-sol",
]);
const ALLOWED_REASONING = new Map([
  ["gpt-5.6-luna", new Set(["low", "medium", "high", "xhigh", "max"])],
  ["gpt-5.6-terra", new Set(["low", "medium", "high", "xhigh", "max", "ultra"])],
  ["gpt-5.6-sol", new Set(["low", "medium", "high", "xhigh", "max", "ultra"])],
]);
const TERMINAL_STATUSES = new Set(["completed", "interrupted", "failed"]);
const DISPATCH_RECONCILIATION_PASSES = 3;
const DISPATCH_RECONCILIATION_BACKOFF_MS = 100;
const DISPATCH_RECONCILIATION_MAX_PAGES = 4;
const DISPATCH_RECONCILIATION_PAGE_SIZE = 100;
const ACTIVE_TRANSPORT_RECONCILIATION_PASSES = 2;
const MANAGED_THREAD_SOURCE = ["appServer"];
const MEANINGFUL_PROGRESS_METHODS = new Map([
  ["item/started", "ITEM_STARTED"],
  ["item/completed", "ITEM_COMPLETED"],
  ["item/commandExecution/outputDelta", "COMMAND_EXECUTION_PROGRESS"],
  ["item/commandExecution/terminalInteraction", "COMMAND_EXECUTION_PROGRESS"],
  ["turn/diff/updated", "TURN_DIFF_UPDATED"],
  ["turn/plan/updated", "TURN_PLAN_UPDATED"],
]);
const DIGEST_FIELDS = new Map([
  ["project_ref", "project"],
  ["spec_ref", "spec"],
  ["approval_ref", "approval"],
  ["budget_digest", "budget"],
  ["verification_ref", "verification"],
  ["worktree_ref", "worktree"],
  ["execution_environment_ref", "execution-environment"],
  ["packet_digest", "attempt-packet"],
  ["model_reason_digest", "model-selection"],
]);
const PACKET_KEYS = new Set([
  "schema",
  "task_ref",
  "attempt",
  ...DIGEST_FIELDS.keys(),
  "global_active_limit",
  "per_task_active_limit",
  "repair_retry_limit",
  "max_duration_seconds",
  "max_total_tokens",
  "max_model_calls",
  "remaining_total_tokens",
  "remaining_model_calls",
  "external_cost_status",
  "external_cost_limit_micros",
  "non_model_external_spend_allowed",
  "base_commit",
  "model",
  "reasoning",
  "deadline_at",
  "heartbeat_timeout_ms",
  "writer_fence",
  "prior_terminal_evidence_ref",
  "continuation",
  "continuation_digest",
  "cwd",
  "prompt",
]);
const RETAINED_KEYS = new Set([
  "task_ref",
  "attempt",
  "packet_digest",
  "thread_id",
  "turn_id",
  "attempt_started_at",
  "attempt_deadline_at",
  "last_heartbeat_at",
  "last_meaningful_progress_at",
]);
const INTERRUPT_CONTROL_SCHEMA = "lattice.managed-codex-worker-control/1.0";
const INTERRUPT_CONTROL_KEYS = new Set([
  "schema",
  "operation",
  "task_ref",
  "attempt",
  "packet_digest",
  "thread_id",
  "turn_id",
]);
const RETAINED_EMPTY_THREAD_KEYS = new Set([
  "task_ref",
  "attempt",
  "packet_digest",
  "thread_id",
]);

function managedError(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

function plainRecord(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError(`${label} must be an object`);
  }
}

function exactKeys(value, expected, label) {
  for (const key of Object.keys(value)) {
    if (!expected.has(key)) throw new TypeError(`${label} contains unsupported field ${key}`);
  }
  for (const key of expected) {
    if (!Object.hasOwn(value, key)) throw new TypeError(`${label} is missing ${key}`);
  }
}

function boundedIdentifier(value, label, maximum = 128) {
  if (
    typeof value !== "string"
    || value.length < 1
    || value.length > maximum
    || !/^[a-z0-9][a-z0-9._:-]*$/u.test(value)
    || containsCredential(value)
  ) {
    throw new TypeError(`${label} must be a bounded secret-free lowercase identifier`);
  }
  return value;
}

function exactDigest(value, prefix, label) {
  if (typeof value !== "string" || !new RegExp(`^${prefix}:sha256:[a-f0-9]{64}$`, "u").test(value)) {
    throw new TypeError(`${label} must be an exact lowercase ${prefix} digest`);
  }
  return value;
}

function compactIsoTimestamp(value) {
  return value
    .replace(/\.000Z$/u, "Z")
    .replace(/(\.\d*[1-9])0+Z$/u, "$1Z");
}

function canonicalTime(value, label) {
  const match = typeof value === "string"
    ? /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.(\d{1,9}))?Z$/u.exec(value)
    : null;
  if (!match || !Number.isFinite(Date.parse(value))) {
    throw new TypeError(`${label} must be a canonical UTC timestamp`);
  }
  const normalized = new Date(value).toISOString();
  const fraction = match[1] ?? null;
  if (fraction !== null && fraction.length > 3) {
    const millisecondProjection = value.replace(
      `.${fraction}Z`,
      `.${fraction.slice(0, 3)}Z`,
    );
    if (fraction.endsWith("0") || normalized !== millisecondProjection) {
      throw new TypeError(`${label} must be a canonical UTC timestamp`);
    }
    return value;
  }
  const compact = compactIsoTimestamp(normalized);
  if (normalized !== value && compact !== value) {
    throw new TypeError(`${label} must be a canonical UTC timestamp`);
  }
  return value;
}

function normalizedCanonicalTime(value, label) {
  const canonical = canonicalTime(value, label);
  return compactIsoTimestamp(new Date(canonical).toISOString());
}

function derivedAttemptDeadline(startedAt, maxDurationSeconds, taskDeadlineAt) {
  const canonicalStartedAt = canonicalTime(startedAt, "attempt_started_at");
  const canonicalTaskDeadline = canonicalTime(taskDeadlineAt, "task deadline_at");
  if (!Number.isSafeInteger(maxDurationSeconds) || maxDurationSeconds < 1) {
    throw new TypeError("max_duration_seconds must be a positive bounded integer");
  }
  const deadlineMillis = Math.min(
    Date.parse(canonicalTaskDeadline),
    Date.parse(canonicalStartedAt) + (maxDurationSeconds * 1_000),
  );
  if (!Number.isSafeInteger(deadlineMillis)) {
    throw new TypeError("derived attempt deadline is outside the supported time range");
  }
  return normalizedCanonicalTime(
    new Date(deadlineMillis).toISOString(),
    "derived attempt_deadline_at",
  );
}

function exactWorktree(left, right) {
  if (typeof left !== "string" || typeof right !== "string") return false;
  const windows = (value) => /^[A-Za-z]:[\\/]/u.test(value);
  if (windows(left) || windows(right)) {
    return windows(left)
      && windows(right)
      && path.win32.normalize(left).toLowerCase() === path.win32.normalize(right).toLowerCase();
  }
  return path.resolve(left) === path.resolve(right);
}

function dispatchMarker(packet) {
  return `[LATTICE_MANAGED_ATTEMPT task_ref=${packet.task_ref} attempt=${packet.attempt} packet_digest=${packet.packet_digest}]`;
}

function markedPrompt(packet) {
  const value = `${dispatchMarker(packet)}\n${packet.prompt}`;
  if (Buffer.byteLength(value, "utf8") > 16_384) {
    throw new TypeError("marker-bound prompt exceeds the managed prompt limit");
  }
  return value;
}

function turnStartsWithMarker(turn, marker) {
  if (!turn || !Array.isArray(turn.items)) return false;
  return turn.items.some((item) => item?.type === "userMessage"
    && Array.isArray(item.content)
    && item.content.some((content) => content?.type === "text"
      && typeof content.text === "string"
      && (content.text === marker || content.text.startsWith(`${marker}\n`))));
}

function classifyMarkerThread(thread, marker) {
  if (!Array.isArray(thread?.turns)) {
    throw managedError(
      "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
      "Managed dispatch thread/read omitted the exact turn history",
    );
  }
  if (thread.turns.length === 0) return Object.freeze({ kind: "EMPTY" });
  const marked = thread.turns.filter((turn) => turnStartsWithMarker(turn, marker));
  if (marked.length !== 1 || marked[0] !== thread.turns.at(-1)) {
    throw managedError(
      "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
      "Managed dispatch thread contains ambiguous or substituted turns",
    );
  }
  const turn = marked[0];
  boundedIdentifier(turn.id, "reconciled turn id", 256);
  if (thread.turns.length !== 1 || !new Set(["inProgress", ...TERMINAL_STATUSES]).has(turn.status)) {
    throw managedError(
      "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
      "Managed dispatch marker turn is not the sole exact lifecycle turn",
    );
  }
  return Object.freeze({ kind: "MARKED_TURN", turn });
}

function containsCredential(value) {
  return /(?:\bBearer\s+[A-Za-z0-9._~+/=-]{12,}|\b(?:password|passwd|api[_-]?key|access[_-]?token|secret)\s*[:=]\s*["']?[A-Za-z0-9._~+/=-]{8,}|\bgh[pousr]_[A-Za-z0-9]{12,}|\bsk-[A-Za-z0-9_-]{16,}|[a-z][a-z0-9+.-]*:\/\/[^\s/:@]+:[^\s/@]+@)/iu.test(value);
}

function absoluteWorktree(value) {
  return typeof value === "string"
    && value.length > 0
    && value.length <= 1_024
    && !value.includes("\0")
    && (path.isAbsolute(value) || /^[A-Za-z]:[\\/]/u.test(value));
}

function exactThread(thread, expectedId, lifecycle) {
  if (!thread || thread.id !== expectedId) {
    throw managedError(
      "MANAGED_CODEX_EXACT_LIFECYCLE_MISMATCH",
      `Codex did not emit the exact ${lifecycle} identity`,
    );
  }
  return thread;
}

function exactTurn(turn, expectedId, statuses, lifecycle) {
  if (!turn || turn.id !== expectedId || !statuses.has(turn.status)) {
    throw managedError(
      "MANAGED_CODEX_EXACT_LIFECYCLE_MISMATCH",
      `Codex did not emit the exact ${lifecycle} identity and status`,
    );
  }
  return turn;
}

function validateRetainedAttempt(
  packet,
  retained,
  { requireLivenessTimes = false, requireExecutionWindow = requireLivenessTimes } = {},
) {
  plainRecord(retained, "retained attempt");
  const expected = new Set([...RETAINED_KEYS].filter((key) => (
    (requireLivenessTimes || !["last_heartbeat_at", "last_meaningful_progress_at"].includes(key))
    && (requireExecutionWindow || !["attempt_started_at", "attempt_deadline_at"].includes(key))
  )));
  exactKeys(retained, expected, "retained attempt");
  boundedIdentifier(retained.task_ref, "retained task_ref");
  if (!Number.isSafeInteger(retained.attempt) || retained.attempt < 1) {
    throw new TypeError("retained attempt must be a positive integer");
  }
  exactDigest(retained.packet_digest, "attempt-packet", "retained packet_digest");
  boundedIdentifier(retained.thread_id, "retained thread_id", 256);
  boundedIdentifier(retained.turn_id, "retained turn_id", 256);
  let attemptStartedAt = null;
  if (requireExecutionWindow) {
    attemptStartedAt = canonicalTime(retained.attempt_started_at, "retained attempt_started_at");
    const attemptDeadlineAt = canonicalTime(
      retained.attempt_deadline_at,
      "retained attempt_deadline_at",
    );
    if (normalizedCanonicalTime(attemptDeadlineAt, "retained attempt_deadline_at") !== derivedAttemptDeadline(
      attemptStartedAt,
      packet.max_duration_seconds,
      packet.deadline_at,
    )) {
      throw managedError(
        "MANAGED_CODEX_RETAINED_EXECUTION_WINDOW_MISMATCH",
        "Retained execution deadline is not bounded by the exact start and task deadline",
      );
    }
  }
  if (requireLivenessTimes) {
    const lastHeartbeatAt = canonicalTime(
      retained.last_heartbeat_at,
      "retained last heartbeat",
    );
    const lastMeaningfulAt = canonicalTime(
      retained.last_meaningful_progress_at,
      "retained last meaningful progress",
    );
    if (
      Date.parse(lastHeartbeatAt) < Date.parse(attemptStartedAt)
      || Date.parse(lastMeaningfulAt) < Date.parse(attemptStartedAt)
    ) {
      throw managedError(
        "MANAGED_CODEX_RETAINED_PROGRESS_PRECEDES_START",
        "Retained heartbeat or meaningful progress cannot precede the exact durable start",
      );
    }
  }
  if (
    retained.task_ref !== packet.task_ref
    || retained.attempt !== packet.attempt
    || retained.packet_digest !== packet.packet_digest
  ) {
    throw managedError(
      "MANAGED_CODEX_RETAINED_IDENTITY_MISMATCH",
      "Retained Codex identity does not match the exact worker packet",
    );
  }
  return Object.freeze({ ...retained });
}

function validateExactInterruptControl(packet, control) {
  plainRecord(control, "interrupt control");
  exactKeys(control, INTERRUPT_CONTROL_KEYS, "interrupt control");
  boundedIdentifier(control.task_ref, "interrupt task_ref");
  if (!Number.isSafeInteger(control.attempt) || control.attempt < 1) {
    throw new TypeError("interrupt attempt must be a positive integer");
  }
  exactDigest(control.packet_digest, "attempt-packet", "interrupt packet_digest");
  boundedIdentifier(control.thread_id, "interrupt thread_id", 256);
  boundedIdentifier(control.turn_id, "interrupt turn_id", 256);
  if (
    control.schema !== INTERRUPT_CONTROL_SCHEMA
    || control.operation !== "interrupt"
    || control.task_ref !== packet.task_ref
    || control.attempt !== packet.attempt
    || control.packet_digest !== packet.packet_digest
  ) {
    throw managedError(
      "MANAGED_CODEX_RETAINED_IDENTITY_MISMATCH",
      "Interrupt control does not match the exact worker packet",
    );
  }
  return Object.freeze({ ...control });
}

function validateRetainedEmptyThread(packet, retained) {
  plainRecord(retained, "retained empty thread");
  exactKeys(retained, RETAINED_EMPTY_THREAD_KEYS, "retained empty thread");
  boundedIdentifier(retained.task_ref, "retained task_ref");
  if (!Number.isSafeInteger(retained.attempt) || retained.attempt < 1) {
    throw new TypeError("retained attempt must be a positive integer");
  }
  exactDigest(retained.packet_digest, "attempt-packet", "retained packet_digest");
  boundedIdentifier(retained.thread_id, "retained thread_id", 256);
  if (
    retained.task_ref !== packet.task_ref
    || retained.attempt !== packet.attempt
    || retained.packet_digest !== packet.packet_digest
  ) {
    throw managedError(
      "MANAGED_CODEX_RETAINED_IDENTITY_MISMATCH",
      "Retained Codex empty-thread identity does not match the exact worker packet",
    );
  }
  return Object.freeze({ ...retained });
}

function validateRetainedPrestart(packet, retained) {
  plainRecord(retained, "retained prestart attempt");
  const allowed = new Set([...RETAINED_EMPTY_THREAD_KEYS, "turn_id"]);
  if (
    Object.keys(retained).some((key) => !allowed.has(key))
    || [...RETAINED_EMPTY_THREAD_KEYS].some((key) => !Object.hasOwn(retained, key))
  ) {
    throw new TypeError("retained prestart attempt shape is invalid");
  }
  boundedIdentifier(retained.task_ref, "retained task_ref");
  if (!Number.isSafeInteger(retained.attempt) || retained.attempt < 1) {
    throw new TypeError("retained attempt must be a positive integer");
  }
  exactDigest(retained.packet_digest, "attempt-packet", "retained packet_digest");
  boundedIdentifier(retained.thread_id, "retained thread_id", 256);
  if (Object.hasOwn(retained, "turn_id")) {
    boundedIdentifier(retained.turn_id, "retained turn_id", 256);
  }
  if (
    retained.task_ref !== packet.task_ref
    || retained.attempt !== packet.attempt
    || retained.packet_digest !== packet.packet_digest
  ) {
    throw managedError(
      "MANAGED_CODEX_RETAINED_IDENTITY_MISMATCH",
      "Retained Codex prestart identity does not match the exact worker packet",
    );
  }
  return Object.freeze({ ...retained });
}

function modelNames(value) {
  const source = Array.isArray(value) ? value : value?.data;
  if (!Array.isArray(source)) return new Set();
  return new Set(source.map((entry) => {
    if (typeof entry === "string") return entry;
    return entry?.id ?? entry?.model ?? entry?.slug ?? null;
  }).filter((entry) => typeof entry === "string"));
}

function digestEvent(event) {
  return `managed-worker-event:sha256:${createHash("sha256")
    .update(JSON.stringify(event), "utf8")
    .digest("hex")}`;
}

function boundedCounter(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

/** Returns only bounded counters correlated to the retained exact worker. */
export function normalizeCodexResourceObservation(message, { threadId, turnId }) {
  if (message?.method !== "thread/tokenUsage/updated") return null;
  const params = message.params;
  if (
    !params
    || params.threadId !== threadId
    || (params.turnId !== undefined && params.turnId !== null && params.turnId !== turnId)
  ) {
    return null;
  }
  const tokenUsage = params.tokenUsage ?? params.usage;
  const counters = tokenUsage?.total ?? tokenUsage;
  if (!counters || typeof counters !== "object" || Array.isArray(counters)) return null;
  const observation = {
    input_tokens: boundedCounter(counters.inputTokens),
    cached_input_tokens: boundedCounter(counters.cachedInputTokens),
    output_tokens: boundedCounter(counters.outputTokens),
    reasoning_output_tokens: boundedCounter(counters.reasoningOutputTokens),
    total_tokens: boundedCounter(counters.totalTokens),
    model_context_window: boundedCounter(tokenUsage?.modelContextWindow),
    external_cost_status: "UNAVAILABLE",
  };
  if (Object.values(observation).every((value) => value === null || value === "UNAVAILABLE")) {
    return null;
  }
  return Object.freeze(observation);
}

function retainedTerminalResourceObservation(thread, turn) {
  for (const tokenUsage of [turn?.tokenUsage, turn?.usage, thread?.tokenUsage, thread?.usage]) {
    const observed = normalizeCodexResourceObservation(
      {
        method: "thread/tokenUsage/updated",
        params: { threadId: thread?.id, turnId: turn?.id, tokenUsage },
      },
      { threadId: thread?.id, turnId: turn?.id },
    );
    if (observed) return observed;
  }
  return null;
}

/** Classifies only content-free provider activity for one exact thread/turn. */
export function normalizeCodexMeaningfulProgress(message, { threadId, turnId }) {
  const progressKind = MEANINGFUL_PROGRESS_METHODS.get(message?.method);
  if (!progressKind || message?.params?.threadId !== threadId || message?.params?.turnId !== turnId) {
    return null;
  }
  return Object.freeze({ progress_kind: progressKind });
}

/**
 * Validates the server-built runtime packet. This is a transport contract,
 * not an authority, routing, retry, or Task Domain policy decision.
 */
export function validateManagedCodexWorkerPacket(untrusted) {
  plainRecord(untrusted, "managed Codex worker packet");
  exactKeys(untrusted, PACKET_KEYS, "managed Codex worker packet");
  if (untrusted.schema !== PACKET_SCHEMA) throw new TypeError(`packet schema must be ${PACKET_SCHEMA}`);
  boundedIdentifier(untrusted.task_ref, "task_ref");
  if (!Number.isSafeInteger(untrusted.attempt) || untrusted.attempt < 1 || untrusted.attempt > 255) {
    throw new TypeError("attempt must be a positive bounded integer");
  }
  for (const [field, prefix] of DIGEST_FIELDS) exactDigest(untrusted[field], prefix, field);
  if (
    !Number.isSafeInteger(untrusted.global_active_limit)
    || untrusted.global_active_limit < 1
    || untrusted.global_active_limit > 4
    || !Number.isSafeInteger(untrusted.per_task_active_limit)
    || untrusted.per_task_active_limit < 1
    || untrusted.per_task_active_limit > untrusted.global_active_limit
    || !Number.isSafeInteger(untrusted.repair_retry_limit)
    || untrusted.repair_retry_limit < 0
    || untrusted.repair_retry_limit > 2
    || untrusted.attempt > untrusted.repair_retry_limit + 1
  ) {
    throw new TypeError("packet capacity and repair bounds are invalid");
  }
  if (
    !Number.isSafeInteger(untrusted.max_duration_seconds)
    || untrusted.max_duration_seconds < 1
    || untrusted.max_duration_seconds > 86_400
    || !Number.isSafeInteger(untrusted.max_total_tokens)
    || untrusted.max_total_tokens < 1
    || !Number.isSafeInteger(untrusted.max_model_calls)
    || untrusted.max_model_calls < 1
    || untrusted.max_model_calls > 255
    || untrusted.attempt > untrusted.max_model_calls
    || !Number.isSafeInteger(untrusted.remaining_total_tokens)
    || untrusted.remaining_total_tokens < 1
    || untrusted.remaining_total_tokens > untrusted.max_total_tokens
    || !Number.isSafeInteger(untrusted.remaining_model_calls)
    || untrusted.remaining_model_calls < 1
    || untrusted.remaining_model_calls > untrusted.max_model_calls
  ) {
    throw new TypeError("packet time, token, or model-call bounds are invalid");
  }
  if (
    !["UNAVAILABLE", "LIMIT_MICROS"].includes(untrusted.external_cost_status)
    || (untrusted.external_cost_status === "UNAVAILABLE"
      ? untrusted.external_cost_limit_micros !== null
      : !Number.isSafeInteger(untrusted.external_cost_limit_micros)
        || untrusted.external_cost_limit_micros < 0)
    || untrusted.non_model_external_spend_allowed !== false
  ) {
    throw new TypeError("packet external-cost policy must be closed and bounded");
  }
  if (typeof untrusted.base_commit !== "string" || !/^[a-f0-9]{40}$/u.test(untrusted.base_commit)) {
    throw new TypeError("base_commit must be an exact lowercase 40-character Git commit");
  }
  if (!ALLOWED_MODELS.has(untrusted.model)) {
    throw new TypeError("model is outside the managed Codex allowlist");
  }
  if (!ALLOWED_REASONING.get(untrusted.model).has(untrusted.reasoning)) {
    throw new TypeError("reasoning effort is not supported by the selected managed Codex model");
  }
  canonicalTime(untrusted.deadline_at, "deadline_at");
  if (
    !Number.isSafeInteger(untrusted.heartbeat_timeout_ms)
    || untrusted.heartbeat_timeout_ms < 1
    || untrusted.heartbeat_timeout_ms > 86_400_000
  ) {
    throw new TypeError("heartbeat_timeout_ms must be a positive bounded integer");
  }
  if (!Number.isSafeInteger(untrusted.writer_fence) || untrusted.writer_fence < 1) {
    throw new TypeError("writer_fence must be a positive integer");
  }
  if (untrusted.attempt === 1) {
    if (
      untrusted.prior_terminal_evidence_ref !== null
      || untrusted.continuation !== null
      || untrusted.continuation_digest !== null
    ) {
      throw new TypeError("initial attempt cannot carry repair continuation fields");
    }
  } else {
    exactDigest(untrusted.prior_terminal_evidence_ref, "evidence", "prior_terminal_evidence_ref");
    exactDigest(untrusted.continuation_digest, "continuation", "continuation_digest");
    if (
      typeof untrusted.continuation !== "string"
      || untrusted.continuation.length < 1
      || Buffer.byteLength(untrusted.continuation, "utf8") > 512
      || containsCredential(untrusted.continuation)
    ) {
      throw new TypeError("repair continuation must be bounded and secret-free");
    }
  }
  if (!absoluteWorktree(untrusted.cwd)) throw new TypeError("cwd must be an absolute bounded worktree path");
  if (
    typeof untrusted.prompt !== "string"
    || untrusted.prompt.length < 1
    || Buffer.byteLength(untrusted.prompt, "utf8") > 16_384
  ) {
    throw new TypeError("prompt must be non-empty and at most 16384 UTF-8 bytes");
  }
  if (containsCredential(untrusted.prompt) || containsCredential(untrusted.cwd)) {
    throw new TypeError("packet contains a secret or credential-bearing value");
  }
  return Object.freeze({ ...untrusted });
}

/** Validates only opaque identities captured by the server-owned runtime. */
export function validateManagedCodexAuthContext(untrusted) {
  plainRecord(untrusted, "managed Codex auth context");
  exactKeys(untrusted, AUTH_CONTEXT_KEYS, "managed Codex auth context");
  if (untrusted.schema !== AUTH_CONTEXT_SCHEMA) {
    throw new TypeError(`auth context schema must be ${AUTH_CONTEXT_SCHEMA}`);
  }
  exactDigest(untrusted.codex_home_digest, "codex-home", "codex_home_digest");
  exactDigest(untrusted.config_digest, "codex-config", "config_digest");
  return Object.freeze({ ...untrusted });
}

export class ManagedCodexWorkerTransport {
  constructor({
    codex,
    eventSink,
    authContext,
    availableModels = null,
    now = () => new Date().toISOString(),
    lifecycleTimeoutMs = 30_000,
    dispatchBackoffMs = DISPATCH_RECONCILIATION_BACKOFF_MS,
    turnStartAuthorizer = null,
  }) {
    if (!codex || typeof codex !== "object") throw new TypeError("codex connector is required");
    if (typeof eventSink !== "function") throw new TypeError("managed worker eventSink is required");
    const validatedAuthContext = validateManagedCodexAuthContext(authContext);
    if (availableModels !== null && !Array.isArray(availableModels) && typeof availableModels !== "function") {
      throw new TypeError("availableModels must be a list or async provider");
    }
    if (typeof now !== "function") throw new TypeError("now must be a function");
    if (!Number.isFinite(lifecycleTimeoutMs) || lifecycleTimeoutMs <= 0) {
      throw new TypeError("lifecycleTimeoutMs must be positive");
    }
    if (!Number.isSafeInteger(dispatchBackoffMs) || dispatchBackoffMs < 0 || dispatchBackoffMs > 500) {
      throw new TypeError("dispatchBackoffMs must be a bounded non-negative integer");
    }
    if (turnStartAuthorizer !== null && typeof turnStartAuthorizer !== "function") {
      throw new TypeError("turnStartAuthorizer must be a function when supplied");
    }
    this.codex = codex;
    this.eventSink = eventSink;
    this.authContext = validatedAuthContext;
    this.availableModels = availableModels;
    this.now = now;
    this.lifecycleTimeoutMs = lifecycleTimeoutMs;
    this.dispatchBackoffMs = dispatchBackoffMs;
    this.turnStartAuthorizer = turnStartAuthorizer;
    this.effectReadiness = null;
    this.eventSequence = 0;
  }

  async #assertModelAvailable(model) {
    const before = await this.#assertAuthReady();
    const listed = typeof this.availableModels === "function"
      ? await this.availableModels()
      : this.availableModels ?? await this.#providerEffect(
        (effectIdentity) => this.codex.listModels({ effectIdentity }),
      );
    if (!modelNames(listed).has(model)) {
      throw managedError(
        "MANAGED_CODEX_MODEL_UNAVAILABLE",
        `Selected managed Codex model ${model} is unavailable; substitution is forbidden`,
      );
    }
    const after = await this.#assertAuthReady();
    if (
      after.app_server_generation !== before.app_server_generation
      || after.app_server_session_id !== before.app_server_session_id
    ) {
      throw managedError(
        "MANAGED_CODEX_AUTH_READINESS_NOT_VERIFIED",
        "Managed Codex connector generation changed during readiness verification",
      );
    }
    return after;
  }

  async #assertAuthReady() {
    let readiness;
    try {
      readiness = await this.codex.readAuthReadiness?.();
    } catch {
      throw managedError(
        "MANAGED_CODEX_AUTH_READINESS_NOT_VERIFIED",
        "Managed Codex account readiness could not be verified",
      );
    }
    if (
      !readiness
      || typeof readiness !== "object"
      || Array.isArray(readiness)
      || Object.keys(readiness).some((key) => !CONNECTOR_AUTH_READINESS_KEYS.has(key))
      || [...CONNECTOR_AUTH_READINESS_KEYS].some((key) => !Object.hasOwn(readiness, key))
      || readiness.schema !== "lattice.codex-auth-readiness/1.0"
      || readiness.ready !== true
      || readiness.authMode !== "chatgpt"
      || !Number.isSafeInteger(readiness.appServerGeneration)
      || readiness.appServerGeneration < 1
      || !/^app-server-session:sha256:[a-f0-9]{64}$/u.test(readiness.appServerSessionId)
    ) {
      throw managedError(
        "MANAGED_CODEX_AUTH_READINESS_NOT_VERIFIED",
        "Managed Codex account readiness is not verified",
      );
    }
    const exact = Object.freeze({
      schema: AUTH_READINESS_SCHEMA,
      ready: true,
      auth_mode: "chatgpt",
      app_server_generation: readiness.appServerGeneration,
      app_server_session_id: readiness.appServerSessionId,
      codex_home_digest: this.authContext.codex_home_digest,
      config_digest: this.authContext.config_digest,
    });
    this.effectReadiness = exact;
    return exact;
  }

  async #providerEffect(effect) {
    const readiness = await this.#assertAuthReady();
    try {
      return await effect(Object.freeze({
        expectedGeneration: readiness.app_server_generation,
        expectedSessionId: readiness.app_server_session_id,
      }));
    } catch (error) {
      if (error?.code === "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED") {
        throw managedError(
          "MANAGED_CODEX_AUTH_EFFECT_IDENTITY_CHANGED",
          "Managed Codex App Server identity changed before the exact provider effect",
        );
      }
      throw error;
    }
  }

  async #authorizeTurnStart(packet, threadId) {
    if (!this.turnStartAuthorizer) return;
    await this.turnStartAuthorizer(Object.freeze({
      task_ref: packet.task_ref,
      attempt: packet.attempt,
      packet_digest: packet.packet_digest,
      thread_id: threadId,
    }));
  }

  /** Performs the same exact allowlist/provider check without starting a thread. */
  async probe(untrustedPacket) {
    const packet = validateManagedCodexWorkerPacket(untrustedPacket);
    const authReadiness = await this.#assertModelAvailable(packet.model);
    return Object.freeze({
      model: packet.model,
      available: true,
      auth_readiness: authReadiness,
    });
  }

  async #dispatchCandidates(packet, claimedAt) {
    const claimedSecond = Math.floor(Date.parse(claimedAt) / 1_000);
    let cursor = null;
    const candidates = [];
    for (let pageNumber = 0; pageNumber < DISPATCH_RECONCILIATION_MAX_PAGES; pageNumber += 1) {
      const page = await this.#providerEffect((effectIdentity) => this.codex.listThreads({
        cwd: packet.cwd,
        cursor,
        limit: DISPATCH_RECONCILIATION_PAGE_SIZE,
        sortKey: "created_at",
        sortDirection: "desc",
        archived: false,
        sourceKinds: MANAGED_THREAD_SOURCE,
        useStateDbOnly: true,
        effectIdentity,
      }));
      if (!page || !Array.isArray(page.data) || page.data.length > DISPATCH_RECONCILIATION_PAGE_SIZE) {
        throw managedError(
          "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
          "Managed dispatch thread/list page is invalid or unbounded",
        );
      }
      let crossedClaimBoundary = false;
      for (const thread of page.data) {
        if (
          !thread
          || typeof thread.id !== "string"
          || !Number.isSafeInteger(thread.createdAt)
          || !exactWorktree(thread.cwd, packet.cwd)
        ) {
          throw managedError(
            "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
            "Managed dispatch thread/list identity is malformed or substituted",
          );
        }
        boundedIdentifier(thread.id, "dispatch candidate thread id", 256);
        if (thread.createdAt < claimedSecond) {
          crossedClaimBoundary = true;
          continue;
        }
        candidates.push(thread);
        if (candidates.length > 1) {
          throw managedError(
            "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
            "Managed dispatch has more than one exact-cwd post-claim candidate",
          );
        }
      }
      if (crossedClaimBoundary || page.nextCursor === null) return candidates;
      if (typeof page.nextCursor !== "string" || page.nextCursor.length > 1_024) {
        throw managedError(
          "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
          "Managed dispatch thread/list cursor is invalid",
        );
      }
      cursor = page.nextCursor;
    }
    throw managedError(
      "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
      "Managed dispatch thread/list exceeded its bounded pagination window",
    );
  }

  async #reconcileDispatch(packet, claimedAt) {
    const marker = dispatchMarker(packet);
    let candidates = [];
    for (let pass = 0; pass < DISPATCH_RECONCILIATION_PASSES; pass += 1) {
      candidates = await this.#dispatchCandidates(packet, claimedAt);
      if (candidates.length !== 0) break;
      if (pass + 1 < DISPATCH_RECONCILIATION_PASSES) {
        await new Promise((resolve) => {
          if (this.dispatchBackoffMs === 0) setImmediate(resolve);
          else setTimeout(resolve, this.dispatchBackoffMs);
        });
      }
    }
    if (candidates.length === 0) return Object.freeze({ kind: "SAFE_FRESH", marker });

    const candidate = candidates[0];
    const read = await this.#providerEffect((effectIdentity) => this.codex.readThread(
      candidate.id,
      { includeTurns: true, allowEmpty: true, effectIdentity },
    ));
    if (
      read.id !== candidate.id
      || !exactWorktree(read.cwd, packet.cwd)
      || !Number.isSafeInteger(read.createdAt)
      || read.createdAt < Math.floor(Date.parse(claimedAt) / 1_000)
    ) {
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Managed dispatch thread/read changed the candidate identity",
      );
    }
    const classified = classifyMarkerThread(read, marker);
    if (classified.kind === "EMPTY") {
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Managed empty dispatch candidate is not exactly attributable to the durable claim",
      );
    }

    const resumed = await this.#providerEffect((effectIdentity) => this.codex.resumeThread(
      read.id,
      { expectedTurnId: classified.turn.id, effectIdentity },
    ));
    if (!exactWorktree(resumed.cwd, packet.cwd) || resumed.createdAt !== read.createdAt) {
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Managed marked dispatch thread changed during resume",
      );
    }
    const resumedMarker = classifyMarkerThread(resumed, marker);
    if (
      resumedMarker.kind !== "MARKED_TURN"
      || resumedMarker.turn.id !== classified.turn.id
      || resumedMarker.turn.status !== classified.turn.status
    ) {
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Managed marker turn changed during exact reconciliation",
      );
    }
    return Object.freeze({ kind: "RECOVER_MARKED_TURN", marker, thread: resumed, turn: resumedMarker.turn });
  }

  async #closePrestartTurn(packet, thread, turn, recoveredVia) {
    const threadId = boundedIdentifier(thread?.id, "recovered prestart thread id", 256);
    const exact = exactTurn(
      turn,
      boundedIdentifier(turn?.id, "recovered prestart turn id", 256),
      new Set(["inProgress", ...TERMINAL_STATUSES]),
      "prestart turn/read",
    );
    await this.#emit(packet, "THREAD_START_ACCEPTED", {
      thread_id: threadId,
      recovered_via: recoveredVia,
    });
    await this.#emit(packet, "TURN_START_ACCEPTED", {
      thread_id: threadId,
      turn_id: exact.id,
      recovered_via: recoveredVia,
    });

    let providerTerminal = exact;
    if (exact.status === "inProgress") {
      if (!this.codex.isTurnActive(threadId, exact.id)) {
        throw managedError(
          "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
          "Recovered prestart turn is reported in progress but is not the exact resumed active turn",
        );
      }
      await this.#emit(packet, "INTERRUPT_REQUESTED", {
        thread_id: threadId,
        turn_id: exact.id,
        recovered_via: recoveredVia,
        interrupt_reason: "EXACT_START_NOT_DURABLE",
      });
      providerTerminal = exactTurn(
        await this.#providerEffect((effectIdentity) => this.codex.interruptTurn(
          threadId,
          exact.id,
          { timeoutMs: this.lifecycleTimeoutMs, effectIdentity },
        )),
        exact.id,
        new Set(["interrupted", "failed"]),
        "prestart exact interrupt terminal",
      );
    }

    await this.#emit(packet, "PRESTART_TERMINAL", {
      thread_id: threadId,
      turn_id: exact.id,
      status: "failed",
      provider_terminal_status: providerTerminal.status,
      failure_reason: "EXACT_START_NOT_DURABLE",
      recovered_via: recoveredVia,
    });
    return Object.freeze({
      kind: "FAILED_START_TERMINAL",
      thread_id: threadId,
      turn_id: exact.id,
      status: "failed",
      provider_terminal_status: providerTerminal.status,
    });
  }

  async #emit(packet, eventType, details = {}) {
    const readiness = this.effectReadiness;
    if (
      !readiness
      || readiness.app_server_generation !== this.codex.connectionGeneration
      || readiness.app_server_session_id !== this.codex.appServerSessionId
    ) {
      throw managedError(
        "MANAGED_CODEX_AUTH_EFFECT_IDENTITY_CHANGED",
        "Managed Codex App Server identity changed before lifecycle evidence",
      );
    }
    const observedAt = canonicalTime(this.now(), "event observed_at");
    const event = {
      schema: EVENT_SCHEMA,
      sequence: this.eventSequence += 1,
      event_type: eventType,
      task_ref: packet.task_ref,
      attempt: packet.attempt,
      project_ref: packet.project_ref,
      spec_ref: packet.spec_ref,
      approval_ref: packet.approval_ref,
      budget_digest: packet.budget_digest,
      verification_ref: packet.verification_ref,
      worktree_ref: packet.worktree_ref,
      base_commit: packet.base_commit,
      packet_digest: packet.packet_digest,
      model_reason_digest: packet.model_reason_digest,
      writer_fence: packet.writer_fence,
      model: packet.model,
      reasoning: packet.reasoning,
      deadline_at: packet.deadline_at,
      app_server_generation: readiness.app_server_generation,
      app_server_session_id: readiness.app_server_session_id,
      codex_home_digest: readiness.codex_home_digest,
      config_digest: readiness.config_digest,
      observed_at: observedAt,
      ...details,
    };
    const evidence = Object.freeze({
      ...event,
      evidence_digest: digestEvent(event),
    });
    if (JSON.stringify(evidence).length > 4_096 || containsCredential(JSON.stringify(evidence))) {
      throw managedError("MANAGED_CODEX_UNSAFE_EVIDENCE", "managed worker evidence is unsafe or unbounded");
    }
    await this.eventSink(evidence);
    return evidence;
  }

  #resourceObserver(
    packet,
    threadId,
    turnId,
    {
      initialLastHeartbeatAt = this.now(),
      initialLastMeaningfulAt = this.now(),
    } = {},
  ) {
    let started = false;
    let closed = false;
    let previous = null;
    let latestResource = null;
    let lastHeartbeatAt = Date.parse(canonicalTime(
      initialLastHeartbeatAt,
      "initial last heartbeat",
    ));
    let lastMeaningfulAt = Date.parse(canonicalTime(
      initialLastMeaningfulAt,
      "initial last meaningful progress",
    ));
    let resourceError = null;
    let budgetInterruptStarted = false;
    let previousProgressKind = null;
    let lastProgressEvidenceAt = 0;
    let heartbeatTimer = null;
    let heartbeatPending = false;
    let exactTerminalProven = false;
    let chain = Promise.resolve();
    const ingest = (message) => {
      if (!started || closed || resourceError) return;
      const progress = normalizeCodexMeaningfulProgress(message, { threadId, turnId });
      if (progress) {
        const progressAt = Date.parse(this.now());
        lastMeaningfulAt = progressAt;
        const evidenceInterval = Math.min(30_000, Math.max(1, Math.floor(packet.heartbeat_timeout_ms / 2)));
        if (
          progress.progress_kind !== previousProgressKind
          || progressAt - lastProgressEvidenceAt >= evidenceInterval
        ) {
          previousProgressKind = progress.progress_kind;
          lastProgressEvidenceAt = progressAt;
          chain = chain
            .then(() => this.#emit(packet, "MEANINGFUL_PROGRESS", {
              thread_id: threadId,
              turn_id: turnId,
              progress_kind: progress.progress_kind,
            }))
            .catch((error) => { resourceError ??= error; });
        }
        return;
      }
      const observation = normalizeCodexResourceObservation(message, { threadId, turnId });
      if (!observation) return;
      const identity = JSON.stringify(observation);
      if (identity === previous) return;
      previous = identity;
      latestResource = observation;
      lastMeaningfulAt = Date.parse(this.now());
      const interruptForTokenBudget = observation.total_tokens !== null
        && observation.total_tokens >= packet.remaining_total_tokens
        && !budgetInterruptStarted;
      if (interruptForTokenBudget) budgetInterruptStarted = true;
      chain = chain
        .then(() => this.#emit(packet, "RESOURCE_OBSERVATION", {
          thread_id: threadId,
          turn_id: turnId,
          usage_scope: "CUMULATIVE_INTERMEDIATE",
          ...observation,
        }))
        .then(async () => {
          if (!interruptForTokenBudget) return;
          await this.#emit(packet, "STALL_CLASSIFIED", {
            thread_id: threadId,
            turn_id: turnId,
            stall_reason: "TOKEN_BUDGET_EXCEEDED",
          });
          await this.#emit(packet, "INTERRUPT_REQUESTED", {
            thread_id: threadId,
            turn_id: turnId,
            stall_reason: "TOKEN_BUDGET_EXCEEDED",
          });
          const terminal = exactTurn(
            await this.#providerEffect((effectIdentity) => this.codex.interruptTurn(
              threadId,
              turnId,
              { timeoutMs: this.lifecycleTimeoutMs, effectIdentity },
            )),
            turnId,
            new Set(["interrupted", "failed"]),
            "token budget interrupt terminal",
          );
          await this.#emit(packet, "INTERRUPT_TERMINAL", {
            thread_id: threadId,
            turn_id: turnId,
            status: terminal.status,
            stall_reason: "TOKEN_BUDGET_EXCEEDED",
          });
        })
        .catch((error) => { resourceError ??= error; });
    };
    const listener = (message) => ingest(message);
    this.codex.on?.("notification", listener);
    return {
      markStarted: () => {
        started = true;
        // Exact start/reconciliation is current liveness evidence, but it is
        // not meaningful task progress. Refresh only the heartbeat clock so a
        // slow foreman restart cannot immediately misclassify a live turn.
        lastHeartbeatAt = Math.max(lastHeartbeatAt, Date.parse(this.now()));
        const heartbeatInterval = Math.min(
          30_000,
          Math.max(1, Math.floor(packet.heartbeat_timeout_ms / 2)),
        );
        heartbeatTimer = setInterval(() => {
          if (closed || resourceError || heartbeatPending) return;
          heartbeatPending = true;
          chain = chain
            .then(async () => {
              const thread = exactThread(
                await this.#providerEffect((effectIdentity) => this.codex.readThread(
                  threadId,
                  { includeTurns: true, effectIdentity },
                )),
                threadId,
                "heartbeat thread/read",
              );
              if (!exactWorktree(thread.cwd, packet.cwd)) {
                throw managedError(
                  "MANAGED_CODEX_EXACT_LIFECYCLE_MISMATCH",
                  "Managed Codex heartbeat did not retain the exact worktree",
                );
              }
              const turn = exactTurn(
                thread.turns?.at(-1),
                turnId,
                new Set(["inProgress", ...TERMINAL_STATUSES]),
                "heartbeat exact provider read",
              );
              // A provider read can race the exact terminal notification.
              // Terminal is a valid observation, but it is not a heartbeat;
              // the main terminal/reconcile path remains its sole recorder.
              if (turn.status !== "inProgress") return;
              const heartbeat = await this.#emit(packet, "HEARTBEAT", {
                thread_id: threadId,
                turn_id: turnId,
                heartbeat_kind: "EXACT_PROVIDER_READ_ACTIVE",
              });
              lastHeartbeatAt = Date.parse(heartbeat.observed_at);
            })
            .catch((error) => { resourceError ??= error; })
            .finally(() => { heartbeatPending = false; });
        }, heartbeatInterval);
        for (const entry of this.codex.notificationSnapshot?.({ threadId, turnId }) ?? []) {
          ingest(entry.message);
        }
      },
      close: async () => {
        if (closed) {
          await chain;
          if (resourceError && !exactTerminalProven) throw resourceError;
          return;
        }
        closed = true;
        if (heartbeatTimer !== null) clearInterval(heartbeatTimer);
        this.codex.off?.("notification", listener);
        await chain;
        if (resourceError && !exactTerminalProven) throw resourceError;
        if (latestResource) {
          await this.#emit(packet, "RESOURCE_OBSERVATION", {
            thread_id: threadId,
            turn_id: turnId,
            usage_scope: "CUMULATIVE_TERMINAL",
            ...latestResource,
          });
        }
      },
      lastHeartbeatAt: () => lastHeartbeatAt,
      lastMeaningfulAt: () => lastMeaningfulAt,
      markExactTerminalProven: () => { exactTerminalProven = true; },
    };
  }

  async #reconcileActiveTransportFailure(
    packet,
    threadId,
    turnId,
    remainingPasses,
  ) {
    if (!Number.isSafeInteger(remainingPasses) || remainingPasses < 1) {
      throw managedError(
        "MANAGED_CODEX_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED",
        "Managed Codex exact-turn transport reconciliation exhausted its bounded attempts",
      );
    }
    for (let pass = 1; pass <= remainingPasses; pass += 1) {
      try {
        await this.codex.connect();
        await this.#assertAuthReady();
      } catch {
        if (pass < remainingPasses) await this.#activeReconciliationBackoff();
        continue;
      }
      await this.#emit(packet, "RECONCILE_STARTED", {
        thread_id: threadId,
        turn_id: turnId,
      });
      let reconciled;
      try {
        const resumed = exactThread(
          await this.#providerEffect((effectIdentity) => this.codex.resumeThread(
            threadId,
            { expectedTurnId: turnId, effectIdentity },
          )),
          threadId,
          "active transport thread/resume",
        );
        if (!exactWorktree(resumed.cwd, packet.cwd)) {
          throw managedError(
            "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
            "Managed active transport thread changed worktree during exact resume",
          );
        }
        const resumedTurn = exactTurn(
          resumed.turns?.at(-1),
          turnId,
          new Set(["inProgress", ...TERMINAL_STATUSES]),
          "active transport thread/resume turn",
        );
        const read = exactThread(
          await this.#providerEffect((effectIdentity) => this.codex.readThread(
            threadId,
            { includeTurns: true, effectIdentity },
          )),
          threadId,
          "active transport thread/read",
        );
        if (!exactWorktree(read.cwd, packet.cwd)) {
          throw managedError(
            "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
            "Managed active transport thread changed worktree during exact read",
          );
        }
        const readTurn = exactTurn(
          read.turns?.at(-1),
          turnId,
          new Set(["inProgress", ...TERMINAL_STATUSES]),
          "active transport thread/read turn",
        );
        if (
          TERMINAL_STATUSES.has(resumedTurn.status)
          && readTurn.status !== resumedTurn.status
        ) {
          throw managedError(
            "MANAGED_CODEX_EXACT_LIFECYCLE_MISMATCH",
            "Managed Codex terminal changed during exact transport reconciliation",
          );
        }
        if (TERMINAL_STATUSES.has(readTurn.status)) {
          reconciled = Object.freeze({
            kind: "TERMINAL",
            passes_used: pass,
            thread: read,
            turn: readTurn,
          });
        } else if (!this.codex.isTurnActive(threadId, turnId)) {
          throw managedError(
            "MANAGED_CODEX_EXACT_LIFECYCLE_MISMATCH",
            "Managed Codex reconciliation did not restore the exact active turn",
          );
        } else {
          reconciled = Object.freeze({ kind: "ACTIVE", passes_used: pass });
        }
      } catch {
        if (pass < remainingPasses) await this.#activeReconciliationBackoff();
        continue;
      }
      if (reconciled.kind === "ACTIVE") {
        await this.#emit(packet, "RECONCILED_ACTIVE", {
          thread_id: threadId,
          turn_id: turnId,
          status: "inProgress",
        });
      }
      return reconciled;
    }
    throw managedError(
      "MANAGED_CODEX_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED",
      "Managed Codex exact-turn transport reconciliation exhausted its bounded attempts",
    );
  }

  async #activeReconciliationBackoff() {
    await new Promise((resolve) => {
      if (this.dispatchBackoffMs === 0) setImmediate(resolve);
      else setTimeout(resolve, this.dispatchBackoffMs);
    });
  }

  async #terminal(
    packet,
    threadId,
    turnId,
    attemptStartedAt,
    attemptDeadlineAt,
    resourceObserver = null,
  ) {
    const deadlineAt = Date.parse(attemptDeadlineAt);
    const closeObserverAfterExactTerminal = async () => {
      resourceObserver?.markExactTerminalProven();
      try {
        await resourceObserver?.close();
      } catch {
        // Provider quiescence is the primary lifecycle receipt. A secondary
        // resource/heartbeat sink failure must not erase an exact terminal;
        // missing usage remains independently fail-closed during verification.
      }
    };
    const waitForTerminal = () => this.codex.waitForTurnCompleted(threadId, turnId, {
      timeoutMs: Math.max(1, deadlineAt - Date.parse(this.now())),
      statuses: [...TERMINAL_STATUSES],
    }).then(
      (value) => ({ kind: "TERMINAL", value }),
      (error) => ({ kind: "ERROR", error }),
    );
    let terminalWait = waitForTerminal();
    let transportReconciliationPasses = 0;
    while (true) {
      const now = Date.parse(this.now());
      const lastHeartbeat = resourceObserver?.lastHeartbeatAt() ?? now;
      const watchdogAt = Math.min(deadlineAt, lastHeartbeat + packet.heartbeat_timeout_ms);
      const delayMs = Math.max(1, watchdogAt - now);
      let watchdogTimer;
      const watchdog = new Promise((resolve) => {
        watchdogTimer = setTimeout(() => resolve({ kind: "WATCHDOG" }), delayMs);
      });
      const result = await Promise.race([terminalWait, watchdog]);
      clearTimeout(watchdogTimer);
      if (result.kind === "TERMINAL") {
        const terminal = exactTurn(
          result.value,
          turnId,
          TERMINAL_STATUSES,
          "turn/completed",
        );
        await closeObserverAfterExactTerminal();
        await this.#emit(packet, "TURN_TERMINAL", {
          thread_id: threadId,
          turn_id: turnId,
          status: terminal.status,
        });
        return Object.freeze({ thread_id: threadId, turn_id: turnId, status: terminal.status });
      }
      const observedAt = canonicalTime(this.now(), "watchdog observed_at");
      const observedMillis = Date.parse(observedAt);
      const heartbeatMillis = resourceObserver?.lastHeartbeatAt() ?? observedMillis;
      const meaningfulMillis = resourceObserver?.lastMeaningfulAt() ?? observedMillis;
      const expired = observedMillis >= deadlineAt
        || observedMillis - heartbeatMillis >= packet.heartbeat_timeout_ms;
      if (!expired && result.kind === "WATCHDOG") continue;
      if (!expired && result.kind === "ERROR") {
        const reconciled = await this.#reconcileActiveTransportFailure(
          packet,
          threadId,
          turnId,
          ACTIVE_TRANSPORT_RECONCILIATION_PASSES - transportReconciliationPasses,
        );
        transportReconciliationPasses += reconciled.passes_used;
        if (reconciled.kind === "TERMINAL") {
          await closeObserverAfterExactTerminal();
          const resource = retainedTerminalResourceObservation(reconciled.thread, reconciled.turn);
          if (resource) {
            try {
              await this.#emit(packet, "RESOURCE_OBSERVATION", {
                thread_id: threadId,
                turn_id: turnId,
                usage_scope: "CUMULATIVE_TERMINAL",
                ...resource,
              });
            } catch {
              // The exact reconciled terminal remains authoritative; a
              // missing usage sample is rejected by the later budget gate.
            }
          }
          await this.#emit(packet, "RECONCILED_TERMINAL", {
            thread_id: threadId,
            turn_id: turnId,
            status: reconciled.turn.status,
          });
          return Object.freeze({
            thread_id: threadId,
            turn_id: turnId,
            status: reconciled.turn.status,
          });
        }
        terminalWait = waitForTerminal();
        continue;
      }
      const lastMeaningful = canonicalTime(
        new Date(meaningfulMillis).toISOString(),
        "last meaningful progress",
      );
      const lastHeartbeatAt = canonicalTime(
        new Date(heartbeatMillis).toISOString(),
        "last heartbeat",
      );
      const recovered = await this.recoverTimedStall(
        packet,
        {
          task_ref: packet.task_ref,
          attempt: packet.attempt,
          packet_digest: packet.packet_digest,
          thread_id: threadId,
          turn_id: turnId,
          attempt_started_at: attemptStartedAt,
          attempt_deadline_at: attemptDeadlineAt,
        },
        {
          observed_at: observedAt,
          last_heartbeat_at: lastHeartbeatAt,
          last_meaningful_progress_at: lastMeaningful,
          interrupt: true,
        },
      );
      if (!recovered.terminal) {
        throw managedError(
          "MANAGED_CODEX_STALL_RECONCILIATION_FAILED",
          "Managed Codex stall did not reach an exact terminal",
        );
      }
      await closeObserverAfterExactTerminal();
      return Object.freeze({
        thread_id: threadId,
        turn_id: turnId,
        status: recovered.terminal.status,
      });
    }
  }

  /** Starts or exactly reconciles one already-claimed attempt. It never creates a retry. */
  async start(untrustedPacket, untrustedClaimedAt) {
    const packet = validateManagedCodexWorkerPacket(untrustedPacket);
    const claimedAt = canonicalTime(untrustedClaimedAt, "claimed_at");
    const dispatchObservedAt = canonicalTime(this.now(), "prestart observed_at");
    if (Date.parse(dispatchObservedAt) >= Date.parse(packet.deadline_at)) {
      throw managedError(
        "MANAGED_CODEX_PRESTART_DEADLINE_EXCEEDED",
        "Managed attempt expired before an exact provider turn could start",
      );
    }
    await this.#assertModelAvailable(packet.model);
    const prompt = markedPrompt(packet);
    let dispatch;
    try {
      dispatch = await this.#reconcileDispatch(packet, claimedAt);
    } catch (error) {
      if (error?.code === "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED") throw error;
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Managed dispatch could not complete its bounded provider reconciliation",
      );
    }

    let threadId;
    let recoveredTurn = null;
    if (dispatch.kind === "SAFE_FRESH") {
      const acceptedThread = await this.#providerEffect((effectIdentity) => this.codex.startThread({
        cwd: packet.cwd,
        model: packet.model,
        approvalPolicy: "never",
        sandbox: "workspace-write",
        ephemeral: false,
        serviceName: "lattice_managed_foreman",
        developerInstructions: [
          "Operate only inside the supplied worktree and bounded task packet.",
          "Do not push, merge, deploy, publish, pay, send external messages, or permanently delete data.",
        ].join(" "),
        config: {
          model_reasoning_effort: packet.reasoning,
          web_search: "disabled",
          sandbox_workspace_write: { network_access: false },
        },
        effectIdentity,
      }));
      threadId = boundedIdentifier(acceptedThread?.id, "accepted thread id", 256);
      await this.#emit(packet, "THREAD_START_ACCEPTED", { thread_id: threadId });
      exactThread(
        await this.codex.waitForThreadStarted(threadId, { timeoutMs: this.lifecycleTimeoutMs }),
        threadId,
        "thread/started",
      );
      await this.#emit(packet, "THREAD_STARTED", { thread_id: threadId });
    } else {
      threadId = boundedIdentifier(dispatch.thread?.id, "reconciled thread id", 256);
      await this.#emit(packet, "THREAD_START_ACCEPTED", {
        thread_id: threadId,
        recovered_via: "THREAD_LIST_READ",
      });
      await this.#emit(packet, "THREAD_STARTED", {
        thread_id: threadId,
        recovered_via: "THREAD_LIST_READ",
      });
      if (dispatch.kind === "RECOVER_MARKED_TURN") recoveredTurn = dispatch.turn;
    }

    let turnId;
    if (recoveredTurn) {
      turnId = boundedIdentifier(recoveredTurn.id, "reconciled turn id", 256);
      await this.#emit(packet, "TURN_START_ACCEPTED", {
        thread_id: threadId,
        turn_id: turnId,
        recovered_via: "EXACT_MARKER_THREAD_READ",
      });
      throw managedError(
        "MANAGED_CODEX_EXACT_START_EVIDENCE_LOST_AFTER_DISPATCH",
        "A marker-bound turn exists but this process did not observe its exact turn/started notification",
      );
    } else {
      await this.#authorizeTurnStart(packet, threadId);
      const acceptedTurn = await this.#providerEffect(
        (effectIdentity) => this.codex.startTurn(threadId, prompt, { effectIdentity }),
      );
      turnId = boundedIdentifier(acceptedTurn?.id, "accepted turn id", 256);
      await this.#emit(packet, "TURN_START_ACCEPTED", { thread_id: threadId, turn_id: turnId });
    }
    let resourceObserver = null;
    try {
      exactTurn(
        await this.codex.waitForTurnStarted(threadId, turnId, { timeoutMs: this.lifecycleTimeoutMs }),
        turnId,
        new Set(["inProgress"]),
        "turn/started",
      );
      const attemptStartedAt = normalizedCanonicalTime(
        this.now(),
        "exact turn/started observed_at",
      );
      const attemptDeadlineAt = derivedAttemptDeadline(
        attemptStartedAt,
        packet.max_duration_seconds,
        packet.deadline_at,
      );
      await this.#emit(packet, "TURN_STARTED", {
        thread_id: threadId,
        turn_id: turnId,
        status: "inProgress",
        observed_at: attemptStartedAt,
        attempt_deadline_at: attemptDeadlineAt,
      });
      resourceObserver = this.#resourceObserver(packet, threadId, turnId, {
        initialLastHeartbeatAt: attemptStartedAt,
        initialLastMeaningfulAt: attemptStartedAt,
      });
      resourceObserver.markStarted();
      return await this.#terminal(
        packet,
        threadId,
        turnId,
        attemptStartedAt,
        attemptDeadlineAt,
        resourceObserver,
      );
    } finally {
      await resourceObserver?.close();
    }
  }

  /**
   * Restart-only recovery for a durable WorkerThread dispatch claim. This
   * method can list/read/resume exact provider identities, but it can never
   * open a new thread or start a turn.
   */
  async recoverClaimedDispatch(untrustedPacket, untrustedClaimedAt) {
    const packet = validateManagedCodexWorkerPacket(untrustedPacket);
    const claimedAt = canonicalTime(untrustedClaimedAt, "claimed_at");
    await this.#assertAuthReady();
    let dispatch;
    try {
      dispatch = await this.#reconcileDispatch(packet, claimedAt);
    } catch (error) {
      if (error?.code === "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED") throw error;
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Managed restart dispatch could not complete bounded provider reconciliation",
      );
    }
    if (dispatch.kind === "SAFE_FRESH") {
      return Object.freeze({ kind: "PROVEN_NO_PROVIDER_CANDIDATE" });
    }
    return this.#closePrestartTurn(
      packet,
      dispatch.thread,
      dispatch.turn,
      "CLAIMED_DISPATCH_EXACT_MARKER",
    );
  }

  /**
   * Reconciles a durably accepted thread, and optionally its accepted turn,
   * before exact start. No execution window exists in this lifecycle phase.
   */
  async recoverPrestart(untrustedPacket, untrustedRetained) {
    const packet = validateManagedCodexWorkerPacket(untrustedPacket);
    const retained = validateRetainedPrestart(packet, untrustedRetained);
    await this.#assertAuthReady();
    await this.codex.connect();

    if (retained.turn_id) {
      const thread = exactThread(
        await this.#providerEffect((effectIdentity) => this.codex.resumeThread(
          retained.thread_id,
          { expectedTurnId: retained.turn_id, effectIdentity },
        )),
        retained.thread_id,
        "prestart retained thread/resume",
      );
      if (!exactWorktree(thread.cwd, packet.cwd)) {
        throw managedError(
          "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
          "Retained prestart thread changed worktree during exact resume",
        );
      }
      const turn = exactTurn(
        thread.turns?.at(-1),
        retained.turn_id,
        new Set(["inProgress", ...TERMINAL_STATUSES]),
        "prestart retained turn/read",
      );
      return this.#closePrestartTurn(packet, thread, turn, "RETAINED_PRESTART_EXACT_TURN");
    }

    const read = exactThread(
      await this.#providerEffect((effectIdentity) => this.codex.readThread(
        retained.thread_id,
        { includeTurns: true, allowEmpty: true, effectIdentity },
      )),
      retained.thread_id,
      "prestart retained thread/read",
    );
    if (!exactWorktree(read.cwd, packet.cwd)) {
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Retained prestart thread changed worktree during exact read",
      );
    }
    const classified = classifyMarkerThread(read, dispatchMarker(packet));
    if (classified.kind === "EMPTY") {
      const resumed = exactThread(
        await this.#providerEffect((effectIdentity) => this.codex.resumeEmptyThread(
          retained.thread_id,
          { effectIdentity },
        )),
        retained.thread_id,
        "prestart retained empty thread/resume",
      );
      if (!exactWorktree(resumed.cwd, packet.cwd)) {
        throw managedError(
          "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
          "Retained empty prestart thread changed worktree during exact resume",
        );
      }
      await this.#emit(packet, "THREAD_START_ACCEPTED", {
        thread_id: retained.thread_id,
        recovered_via: "RETAINED_PRESTART_EXACT_EMPTY",
      });
      return Object.freeze({ kind: "EXACT_EMPTY_THREAD", thread_id: retained.thread_id });
    }
    const resumed = exactThread(
      await this.#providerEffect((effectIdentity) => this.codex.resumeThread(
        retained.thread_id,
        { expectedTurnId: classified.turn.id, effectIdentity },
      )),
      retained.thread_id,
      "prestart marker thread/resume",
    );
    if (!exactWorktree(resumed.cwd, packet.cwd)) {
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Retained marker prestart thread changed worktree during exact resume",
      );
    }
    const resumedMarker = classifyMarkerThread(resumed, dispatchMarker(packet));
    if (resumedMarker.kind !== "MARKED_TURN" || resumedMarker.turn.id !== classified.turn.id) {
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Retained marker prestart turn changed during exact reconciliation",
      );
    }
    return this.#closePrestartTurn(
      packet,
      resumed,
      resumedMarker.turn,
      "RETAINED_PRESTART_EXACT_MARKER",
    );
  }

  /**
   * Continues only a durably retained, exact empty provider thread after a
   * process restart. It never discovers or starts a replacement thread.
   */
  async continueTurn(untrustedPacket, untrustedRetainedThread) {
    const packet = validateManagedCodexWorkerPacket(untrustedPacket);
    const retained = validateRetainedEmptyThread(packet, untrustedRetainedThread);
    await this.#assertAuthReady();
    await this.codex.connect();
    const thread = exactThread(
      await this.#providerEffect((effectIdentity) => this.codex.resumeEmptyThread(
        retained.thread_id,
        { effectIdentity },
      )),
      retained.thread_id,
      "retained empty thread/resume",
    );
    if (!exactWorktree(thread.cwd, packet.cwd) || !Array.isArray(thread.turns) || thread.turns.length !== 0) {
      throw managedError(
        "MANAGED_CODEX_RETAINED_EMPTY_THREAD_REQUIRED",
        "Retained provider thread is not the sole exact empty thread eligible to continue",
      );
    }
    await this.#emit(packet, "THREAD_RECONCILED_EMPTY", {
      thread_id: retained.thread_id,
      recovered_via: "EXACT_EMPTY_THREAD_RESUME",
    });
    await this.#authorizeTurnStart(packet, retained.thread_id);
    const acceptedTurn = await this.#providerEffect(
      (effectIdentity) => this.codex.startTurn(
        retained.thread_id,
        markedPrompt(packet),
        { effectIdentity },
      ),
    );
    const turnId = boundedIdentifier(acceptedTurn?.id, "accepted turn id", 256);
    await this.#emit(packet, "TURN_START_ACCEPTED", {
      thread_id: retained.thread_id,
      turn_id: turnId,
      recovered_via: "EXACT_EMPTY_THREAD_RESUME",
    });
    let resourceObserver = null;
    try {
      exactTurn(
        await this.codex.waitForTurnStarted(retained.thread_id, turnId, {
          timeoutMs: this.lifecycleTimeoutMs,
        }),
        turnId,
        new Set(["inProgress"]),
        "continued turn/started",
      );
      const attemptStartedAt = normalizedCanonicalTime(
        this.now(),
        "continued exact turn/started observed_at",
      );
      const attemptDeadlineAt = derivedAttemptDeadline(
        attemptStartedAt,
        packet.max_duration_seconds,
        packet.deadline_at,
      );
      await this.#emit(packet, "TURN_STARTED", {
        thread_id: retained.thread_id,
        turn_id: turnId,
        status: "inProgress",
        observed_at: attemptStartedAt,
        attempt_deadline_at: attemptDeadlineAt,
      });
      resourceObserver = this.#resourceObserver(packet, retained.thread_id, turnId, {
        initialLastHeartbeatAt: attemptStartedAt,
        initialLastMeaningfulAt: attemptStartedAt,
      });
      resourceObserver.markStarted();
      return await this.#terminal(
        packet,
        retained.thread_id,
        turnId,
        attemptStartedAt,
        attemptDeadlineAt,
        resourceObserver,
      );
    } finally {
      await resourceObserver?.close();
    }
  }

  /** Reconciles retained exact IDs on restart and never starts replacement work. */
  async resume(untrustedPacket, untrustedRetained) {
    const packet = validateManagedCodexWorkerPacket(untrustedPacket);
    const retained = validateRetainedAttempt(packet, untrustedRetained, {
      requireLivenessTimes: true,
    });
    await this.#assertAuthReady();
    await this.codex.connect();
    await this.#emit(packet, "RECONCILE_STARTED", {
      thread_id: retained.thread_id,
      turn_id: retained.turn_id,
    });
    const thread = exactThread(
      await this.#providerEffect((effectIdentity) => this.codex.resumeThread(
        retained.thread_id,
        { expectedTurnId: retained.turn_id, effectIdentity },
      )),
      retained.thread_id,
      "thread/resume",
    );
    if (!exactWorktree(thread.cwd, packet.cwd)) {
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Retained active thread changed worktree during exact resume",
      );
    }
    const latestTurn = thread.turns?.at(-1);
    const turn = exactTurn(
      latestTurn,
      retained.turn_id,
      new Set(["inProgress", ...TERMINAL_STATUSES]),
      "thread/read reconciliation",
    );
    if (TERMINAL_STATUSES.has(turn.status)) {
      const resource = retainedTerminalResourceObservation(thread, turn);
      if (resource) {
        await this.#emit(packet, "RESOURCE_OBSERVATION", {
          thread_id: retained.thread_id,
          turn_id: retained.turn_id,
          usage_scope: "CUMULATIVE_TERMINAL",
          ...resource,
        });
      }
      await this.#emit(packet, "RECONCILED_TERMINAL", {
        thread_id: retained.thread_id,
        turn_id: retained.turn_id,
        status: turn.status,
      });
      return Object.freeze({
        thread_id: retained.thread_id,
        turn_id: retained.turn_id,
        status: turn.status,
      });
    }
    if (!this.codex.isTurnActive(retained.thread_id, retained.turn_id)) {
      throw managedError(
        "MANAGED_CODEX_EXACT_LIFECYCLE_MISMATCH",
        "Reconciliation did not restore the retained exact active turn",
      );
    }
    await this.#emit(packet, "RECONCILED_ACTIVE", {
      thread_id: retained.thread_id,
      turn_id: retained.turn_id,
      status: "inProgress",
    });
    const resourceObserver = this.#resourceObserver(packet, retained.thread_id, retained.turn_id, {
      initialLastHeartbeatAt: retained.last_heartbeat_at,
      initialLastMeaningfulAt: retained.last_meaningful_progress_at,
    });
    resourceObserver.markStarted();
    try {
      return await this.#terminal(
        packet,
        retained.thread_id,
        retained.turn_id,
        retained.attempt_started_at,
        retained.attempt_deadline_at,
        resourceObserver,
      );
    } finally {
      await resourceObserver.close();
    }
  }

  /** Interrupts only the exact active turn retained by the current bridge. */
  async interruptActive(untrustedPacket, untrustedControl) {
    const packet = validateManagedCodexWorkerPacket(untrustedPacket);
    const control = validateExactInterruptControl(packet, untrustedControl);
    await this.#assertAuthReady();
    if (!this.codex.isTurnActive(control.thread_id, control.turn_id)) {
      throw managedError(
        "MANAGED_CODEX_RETAINED_IDENTITY_MISMATCH",
        "Interrupt control does not identify the exact active Codex turn",
      );
    }
    await this.#emit(packet, "INTERRUPT_REQUESTED", {
      thread_id: control.thread_id,
      turn_id: control.turn_id,
    });
    const terminal = exactTurn(
      await this.#providerEffect((effectIdentity) => this.codex.interruptTurn(
        control.thread_id,
        control.turn_id,
        { timeoutMs: this.lifecycleTimeoutMs, effectIdentity },
      )),
      control.turn_id,
      new Set(["interrupted", "failed"]),
      "graceful shutdown interrupt terminal",
    );
    await this.#emit(packet, "INTERRUPT_TERMINAL", {
      thread_id: control.thread_id,
      turn_id: control.turn_id,
      status: terminal.status,
      interrupt_reason: "GRACEFUL_SHUTDOWN",
    });
    return terminal;
  }

  /**
   * Reconciles a time-based stall candidate and optionally interrupts the
   * retained exact active turn. Retry selection remains outside this module.
   */
  async recoverTimedStall(untrustedPacket, untrustedRetained, observation) {
    const packet = validateManagedCodexWorkerPacket(untrustedPacket);
    const retained = validateRetainedAttempt(packet, untrustedRetained, {
      requireExecutionWindow: true,
    });
    plainRecord(observation, "stall observation");
    exactKeys(
      observation,
      new Set([
        "observed_at",
        "last_heartbeat_at",
        "last_meaningful_progress_at",
        "interrupt",
      ]),
      "stall observation",
    );
    const observedAt = canonicalTime(observation.observed_at, "observed_at");
    const lastHeartbeatAt = canonicalTime(
      observation.last_heartbeat_at,
      "last_heartbeat_at",
    );
    const lastProgressAt = canonicalTime(
      observation.last_meaningful_progress_at,
      "last_meaningful_progress_at",
    );
    if (typeof observation.interrupt !== "boolean") {
      throw new TypeError("stall observation interrupt must be boolean");
    }
    const heartbeatElapsed = Date.parse(observedAt) - Date.parse(lastHeartbeatAt);
    const meaningfulElapsed = Date.parse(observedAt) - Date.parse(lastProgressAt);
    if (
      heartbeatElapsed < 0
      || meaningfulElapsed < 0
      || Date.parse(lastHeartbeatAt) < Date.parse(retained.attempt_started_at)
      || Date.parse(lastProgressAt) < Date.parse(retained.attempt_started_at)
    ) {
      throw new TypeError("stall times must follow the retained exact start in order");
    }
    const deadlineExceeded = Date.parse(observedAt) >= Date.parse(retained.attempt_deadline_at);
    const heartbeatExceeded = heartbeatElapsed >= packet.heartbeat_timeout_ms;
    if (!deadlineExceeded && !heartbeatExceeded) return Object.freeze({ kind: "HEALTHY" });

    await this.#assertAuthReady();
    await this.codex.connect();
    await this.#emit(packet, "RECONCILE_STARTED", {
      thread_id: retained.thread_id,
      turn_id: retained.turn_id,
    });
    const thread = exactThread(
      await this.#providerEffect((effectIdentity) => this.codex.resumeThread(
        retained.thread_id,
        { expectedTurnId: retained.turn_id, effectIdentity },
      )),
      retained.thread_id,
      "thread/resume",
    );
    if (!exactWorktree(thread.cwd, packet.cwd)) {
      throw managedError(
        "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED",
        "Retained stalled thread changed worktree during exact resume",
      );
    }
    const turn = exactTurn(
      thread.turns?.at(-1),
      retained.turn_id,
      new Set(["inProgress", ...TERMINAL_STATUSES]),
      "thread/read reconciliation",
    );
    if (TERMINAL_STATUSES.has(turn.status)) {
      await this.#emit(packet, "RECONCILED_TERMINAL", {
        thread_id: retained.thread_id,
        turn_id: retained.turn_id,
        status: turn.status,
      });
      return Object.freeze({ kind: "TERMINAL", terminal: { id: turn.id, status: turn.status } });
    }
    if (!this.codex.isTurnActive(retained.thread_id, retained.turn_id)) {
      return Object.freeze({ kind: "NOT_EXACT_ACTIVE" });
    }

    await this.#emit(packet, "RECONCILED_ACTIVE", {
      thread_id: retained.thread_id,
      turn_id: retained.turn_id,
      status: "inProgress",
    });

    const stallReason = deadlineExceeded
      ? "DEADLINE_EXCEEDED"
      : "HEARTBEAT_TIMEOUT_ACTIVE_TURN";
    await this.#emit(packet, "STALL_CLASSIFIED", {
      thread_id: retained.thread_id,
      turn_id: retained.turn_id,
      stall_reason: stallReason,
      last_heartbeat_at: lastHeartbeatAt,
      last_meaningful_progress_at: lastProgressAt,
    });
    if (!observation.interrupt) {
      return Object.freeze({ kind: "STALL", stall_reason: stallReason, terminal: null });
    }
    await this.#emit(packet, "INTERRUPT_REQUESTED", {
      thread_id: retained.thread_id,
      turn_id: retained.turn_id,
      stall_reason: stallReason,
    });
    const terminal = exactTurn(
      await this.#providerEffect((effectIdentity) => this.codex.interruptTurn(
        retained.thread_id,
        retained.turn_id,
        { timeoutMs: this.lifecycleTimeoutMs, effectIdentity },
      )),
      retained.turn_id,
      new Set(["interrupted", "failed"]),
      "interrupt terminal",
    );
    await this.#emit(packet, "INTERRUPT_TERMINAL", {
      thread_id: retained.thread_id,
      turn_id: retained.turn_id,
      status: terminal.status,
      stall_reason: stallReason,
    });
    return Object.freeze({
      kind: "STALL",
      stall_reason: stallReason,
      terminal: Object.freeze({ id: terminal.id, status: terminal.status }),
    });
  }
}

export const MANAGED_CODEX_MODELS = Object.freeze([...ALLOWED_MODELS]);
