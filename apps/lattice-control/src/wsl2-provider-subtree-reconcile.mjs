import { execFile as execFileCallback } from "node:child_process";
import { createHash } from "node:crypto";
import process from "node:process";
import { promisify } from "node:util";

import {
  buildWsl2CodexLaunch,
  canonicalJson,
  validateWsl2ExecutionEnvironment,
} from "./wsl2-execution-domain.mjs";

export const WSL2_PROVIDER_SUBTREE_MARKER_SCHEMA =
  "lattice.wsl2-provider-subtree-marker/1.0";
export const WSL2_PROVIDER_SUBTREE_RECEIPT_SCHEMA =
  "lattice.wsl2-provider-subtree-receipt/1.0";
export const WSL2_PROVIDER_SUBTREE_RECONCILIATION_SCHEMA =
  "lattice.wsl2-provider-subtree-reconciliation/1.0";
export const WSL2_PROVIDER_SUBTREE_RECONCILE_REQUEST_SCHEMA =
  "lattice.wsl2-provider-subtree-reconcile-request/1.0";
export const WSL2_REVIEWER_SUBTREE_RECONCILE_REQUEST_SCHEMA =
  "lattice.wsl2-reviewer-subtree-reconcile-request/1.0";

const PROCESS_MARKER_SCHEMA = "lattice.wsl2-process-fence/1.1";
const SUBTREE_EXIT_SCHEMA = "lattice.wsl2-subtree-exit/1.2";
const OUTER_POST_EXIT_SCHEMA = "lattice.wsl2-provider-outer-post-exit/1.0";
const CLEANUP_SCHEMA = "lattice.wsl2-provider-subtree-cleanup/1.0";
const HEX_40 = /^[a-f0-9]{40}$/u;
const HEX_64 = /^[a-f0-9]{64}$/u;
const TYPED = /^[a-z0-9][a-z0-9-]*:sha256:[a-f0-9]{64}$/u;
const MAX_RECORD_BYTES = 16_384;
const MAX_INPUT_BYTES = 131_072;
const MAX_SCAN_DEPTH = 16;
const MAX_SCAN_NODES = 512;
const MAX_STRING_BYTES = 4_096;
const MAX_PROVIDER_EFFECT_COUNT = 16;
const execFileDefault = promisify(execFileCallback);

const PROCESS_MARKER_KEYS = Object.freeze([
  "schema", "fence", "unit", "execution_environment_ref", "credential_seal_digest",
  "boot_id_digest", "pid", "process_start_ticks", "process_group_id", "cgroup_path",
  "cgroup_version", "delegated", "attempt", "retry_of", "reconnect_of",
]);
const SUBTREE_EXIT_KEYS = Object.freeze([
  "schema", "fence", "unit", "execution_environment_ref", "credential_seal_digest",
  "cgroup_path", "zero_descendants", "credential_seal_intact", "credential_watch_intact",
  "keyring_daemon_sha256", "keyring_library_manifest_digest", "tool_input_identities",
  "stdout_bytes", "stderr_bytes", "stdout_limit_bytes", "stderr_limit_bytes",
  "output_bound_exceeded", "timeout_ms", "timed_out", "interrupted", "stdin_bytes",
  "stdin_sha256", "stdin_complete", "attempt", "retry_of", "reconnect_of", "exit_code",
  "exit_signal",
]);
const OUTER_POST_EXIT_KEYS = Object.freeze([
  "schema", "unit", "fence", "cgroup_path", "boot_id_digest", "active_state",
  "sub_state", "result", "delegate", "cgroup_exists", "populated",
]);
const MARKER_CONTEXT_KEYS = Object.freeze([
  "task_ref", "attempt", "packet_digest", "worktree_ref", "repository_head",
  "execution_environment_ref", "descriptor_digest", "source_preflight_descriptor_digest",
  "source_preflight_content_digest", "source_preflight_receipt_digest",
]);
const MARKER_KEYS = Object.freeze([
  "schema", "status", ...MARKER_CONTEXT_KEYS, "role", "provider_subtree_segment_ref",
  "process_marker", "boot_id_digest",
  "credential_seal_digest", "continuation", "provider_effect_count", "marker_digest",
]);
const RECEIPT_KEYS = Object.freeze([
  "schema", "status", ...MARKER_CONTEXT_KEYS, "role", "provider_subtree_segment_ref",
  "source_marker_digest",
  "process_marker", "subtree_exit", "outer_post_exit", "boot_id_digest",
  "credential_seal_digest", "continuation", "provider_effect_count", "receipt_digest",
]);
const RECONCILIATION_KEYS = Object.freeze([
  "schema", "status", "task_ref", "attempt", "worktree_ref", "repository_head",
  "execution_environment_ref", "descriptor_digest", "source_preflight_descriptor_digest",
  "source_preflight_content_digest", "source_preflight_receipt_digest", "role",
  "provider_subtree_segment_ref", "marker_observation", "source_marker_digest", "packet_digest",
  "process_marker", "fence",
  "unit", "cgroup_path", "boot_id_digest", "credential_seal_digest", "continuation",
  "cleanup", "outer_post_exit", "provider_effect_count_before",
  "provider_effect_count_after", "reconciliation_digest",
]);
const REVIEWER_RECONCILIATION_KEYS = Object.freeze([
  ...RECONCILIATION_KEYS, "subject_digest", "model_call_identity",
]);

function rejected(code) {
  const error = new Error(code);
  error.code = code;
  return error;
}

function ensure(condition, code) {
  if (!condition) throw rejected(code);
}

function object(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function exactKeys(value, expected, code) {
  ensure(object(value), code);
  const actual = Object.keys(value).sort();
  const sorted = [...expected].sort();
  ensure(actual.length === sorted.length
    && actual.every((key, index) => key === sorted[index]), code);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function canonicalWslBootIdDigest(value) {
  ensure(typeof value === "string"
    && /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\s*$/u
      .test(value),
    "WSL2_PROVIDER_BOOT_ID_REJECTED");
  return `wsl-boot:sha256:${sha256(Buffer.from(value.trim(), "utf8"))}`;
}

export function canonicalProviderSubtreeDigest(domain, value, digestKey) {
  ensure(object(value) && typeof digestKey === "string",
    "WSL2_PROVIDER_SUBTREE_DIGEST_REJECTED");
  const subject = Object.fromEntries(
    Object.entries(value).filter(([key]) => key !== digestKey),
  );
  return `${domain}:sha256:${sha256(Buffer.from(canonicalJson(subject), "utf8"))}`;
}

function recognizedSecret(value) {
  const lower = value.toLowerCase();
  return lower.includes("bearer ")
    || (lower.includes("-----begin ") && lower.includes("private key-----"))
    || /:\/\/[^/?#\s"'<>}{]+@/u.test(value)
    || /(?:^|[^a-z0-9_-])(?:password|passphrase|passwd|pwd|token|access[ _-]token|refresh[ _-]token|id[ _-]token|session[ _-]token|api[ _-]?key|apikey|client[ _-]secret|secret|credential|credentials|cookie|set-cookie|authorization)\s*["']?\s*[:=]/iu.test(value)
    || /(?:^|[^A-Za-z0-9])(?:AKIA|ASIA)[A-Z0-9]{16}(?:[^A-Za-z0-9]|$)/u.test(value)
    || ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "glpat-", "npm_",
      "pypi-", "xoxa-", "xoxb-", "xoxp-", "xoxr-", "xoxs-", "sk-"]
      .some((prefix) => lower.includes(prefix));
}

function boundedSecretFree(root) {
  const pending = [{ value: root, depth: 0 }];
  let nodes = 0;
  while (pending.length > 0) {
    const { value, depth } = pending.pop();
    nodes += 1;
    if (nodes > MAX_SCAN_NODES) return false;
    if (typeof value === "string") {
      if (Buffer.byteLength(value, "utf8") > MAX_STRING_BYTES || recognizedSecret(value)) {
        return false;
      }
      continue;
    }
    if (value === null || typeof value !== "object") continue;
    if (depth >= MAX_SCAN_DEPTH) return false;
    const children = Array.isArray(value) ? value : Object.values(value);
    if (nodes + pending.length + children.length > MAX_SCAN_NODES) return false;
    for (const child of children) pending.push({ value: child, depth: depth + 1 });
  }
  return true;
}

function boundedRecord(value, code) {
  ensure(boundedSecretFree(value)
    && Buffer.byteLength(canonicalJson(value), "utf8") <= MAX_RECORD_BYTES, code);
  return value;
}

function validTyped(value, domain = null) {
  return typeof value === "string" && TYPED.test(value)
    && (domain === null || value.startsWith(`${domain}:sha256:`));
}

function validContinuation(value, attempt, code) {
  exactKeys(value, ["retry_of", "reconnect_of"], code);
  ensure((value.retry_of === null || validTyped(value.retry_of, "attempt-receipt"))
    && (value.reconnect_of === null || validTyped(value.reconnect_of, "attempt-receipt"))
    && (value.retry_of === null || value.reconnect_of === null)
    && ((attempt === 1 && value.retry_of === null)
      || (attempt > 1 && (value.retry_of !== null || value.reconnect_of !== null))), code);
}

function validateProcessMarker(value, context, code) {
  exactKeys(value, PROCESS_MARKER_KEYS, code);
  ensure(value.schema === PROCESS_MARKER_SCHEMA
    && HEX_64.test(value.fence)
    && value.unit
      === `lattice-wsl2-${context.task_ref.slice(0, 16)}-provider-${value.fence.slice(0, 12)}.service`
    && value.execution_environment_ref === context.execution_environment_ref
    && value.credential_seal_digest === context.credential_seal_digest
    && validTyped(value.boot_id_digest, "wsl-boot")
    && Number.isSafeInteger(value.pid) && value.pid > 0
    && typeof value.process_start_ticks === "string" && /^[1-9]\d*$/u.test(value.process_start_ticks)
    && Number.isSafeInteger(value.process_group_id) && value.process_group_id > 0
    && typeof value.cgroup_path === "string" && value.cgroup_path.length <= 1_024
    && value.cgroup_path.startsWith("/user.slice/")
    && value.cgroup_path.endsWith(`/${value.unit}`)
    && !value.cgroup_path.includes("..") && !value.cgroup_path.includes("\\")
    && value.cgroup_version === 2 && value.delegated === false
    && value.attempt === context.attempt
    && value.retry_of === context.continuation.retry_of
    && value.reconnect_of === context.continuation.reconnect_of, code);
  return value;
}

function validSeal(value, { library = false } = {}) {
  const keys = library
    ? ["manifest_path", "path", "resolved_path", "sha256", "device", "inode", "owner_uid",
      "mode", "size"]
    : ["path", "resolved_path", "sha256", "device", "inode", "owner_uid", "mode", "size"];
  return object(value) && Object.keys(value).sort().join("\0") === keys.sort().join("\0")
    && typeof value.path === "string" && value.path.startsWith("/")
    && typeof value.resolved_path === "string" && value.resolved_path.startsWith("/")
    && HEX_64.test(value.sha256)
    && typeof value.device === "string" && /^\d+$/u.test(value.device)
    && typeof value.inode === "string" && /^\d+$/u.test(value.inode)
    && Number.isSafeInteger(value.owner_uid) && value.owner_uid >= 0
    && Number.isSafeInteger(value.mode) && value.mode > 0 && (value.mode & 0o022) === 0
    && Number.isSafeInteger(value.size) && value.size > 0
    && (!library || (typeof value.manifest_path === "string"
      && /^[A-Za-z0-9._-]{1,128}$/u.test(value.manifest_path)));
}

function validateSubtreeExit(value, marker, code) {
  exactKeys(value, SUBTREE_EXIT_KEYS, code);
  const tools = value.tool_input_identities;
  exactKeys(tools, [
    "executable", "verifier_tool", "sandbox_helper", "node_runtime", "rustc", "rustdoc",
    "keyring_daemon", "keyring_libraries",
  ], code);
  ensure(value.schema === SUBTREE_EXIT_SCHEMA
    && value.fence === marker.fence && value.unit === marker.unit
    && value.execution_environment_ref === marker.execution_environment_ref
    && value.credential_seal_digest === marker.credential_seal_digest
    && value.cgroup_path === marker.cgroup_path
    && value.zero_descendants === true && value.credential_seal_intact === true
    && value.credential_watch_intact === true
    && HEX_64.test(value.keyring_daemon_sha256)
    && validTyped(value.keyring_library_manifest_digest, "keyring-library-manifest")
    && validSeal(tools.executable) && tools.verifier_tool === null
    && validSeal(tools.sandbox_helper) && tools.node_runtime === null
    && tools.rustc === null && tools.rustdoc === null && validSeal(tools.keyring_daemon)
    && Array.isArray(tools.keyring_libraries) && tools.keyring_libraries.length === 2
    && tools.keyring_libraries.every((entry) => validSeal(entry, { library: true }))
    && Number.isSafeInteger(value.stdout_bytes) && value.stdout_bytes >= 0
    && Number.isSafeInteger(value.stderr_bytes) && value.stderr_bytes >= 0
    && Number.isSafeInteger(value.stdout_limit_bytes) && value.stdout_limit_bytes >= 1_024
    && Number.isSafeInteger(value.stderr_limit_bytes) && value.stderr_limit_bytes >= 1_024
    && value.stdout_bytes <= value.stdout_limit_bytes
    && value.stderr_bytes <= value.stderr_limit_bytes
    && value.output_bound_exceeded === false
    && Number.isSafeInteger(value.timeout_ms) && value.timeout_ms >= 1_000
    && value.timed_out === false && value.interrupted === false
    && Number.isSafeInteger(value.stdin_bytes) && value.stdin_bytes >= 0
    && HEX_64.test(value.stdin_sha256) && value.stdin_complete === true
    && value.attempt === marker.attempt && value.retry_of === marker.retry_of
    && value.reconnect_of === marker.reconnect_of
    && (value.exit_code === null || (Number.isSafeInteger(value.exit_code)
      && value.exit_code >= 0 && value.exit_code <= 255))
    && (value.exit_signal === null || (typeof value.exit_signal === "string"
      && /^SIG[A-Z0-9]{1,24}$/u.test(value.exit_signal))), code);
  return value;
}

export function validateWsl2ProviderOuterPostExit(value, expected) {
  const code = "WSL2_PROVIDER_OUTER_POST_EXIT_REJECTED";
  exactKeys(value, OUTER_POST_EXIT_KEYS, code);
  ensure(value.schema === OUTER_POST_EXIT_SCHEMA
    && value.unit === expected.unit && value.fence === expected.fence
    && value.cgroup_path === expected.cgroup_path
    && value.boot_id_digest === expected.boot_id_digest
    && value.active_state === "inactive" && value.sub_state === "dead"
    && typeof value.result === "string" && /^[a-z0-9-]{1,32}$/u.test(value.result)
    && value.delegate === "no"
    && ((value.cgroup_exists === false && value.populated === null)
      || (value.cgroup_exists === true && value.populated === 0)), code);
  return boundedRecord(value, code);
}

function validateContext(context, code) {
  exactKeys(context, MARKER_CONTEXT_KEYS, code);
  ensure(HEX_64.test(context.task_ref)
    && Number.isSafeInteger(context.attempt) && context.attempt >= 1 && context.attempt <= 3
    && validTyped(context.packet_digest, "attempt-packet")
    && validTyped(context.worktree_ref, "worktree") && HEX_40.test(context.repository_head)
    && validTyped(context.execution_environment_ref, "execution-environment")
    && HEX_64.test(context.descriptor_digest)
    && HEX_64.test(context.source_preflight_descriptor_digest)
    && HEX_64.test(context.source_preflight_content_digest)
    && validTyped(context.source_preflight_receipt_digest, "wsl2-preflight"), code);
  return context;
}

function providerSubtreeSegmentRef(context, fence, continuation) {
  return `provider-subtree-segment:sha256:${sha256(Buffer.from(canonicalJson({
    task_ref: context.task_ref,
    attempt: context.attempt,
    source_preflight_descriptor_digest: context.source_preflight_descriptor_digest,
    source_preflight_content_digest: context.source_preflight_content_digest,
    source_preflight_receipt_digest: context.source_preflight_receipt_digest,
    fence,
    role: "PROVIDER",
    continuation,
  }), "utf8"))}`;
}

function reviewerSubtreeSegmentRef(
  context, fence, continuation, subjectDigest, modelCallIdentity,
) {
  return `provider-subtree-segment:sha256:${sha256(Buffer.from(canonicalJson({
    task_ref: context.task_ref,
    attempt: context.attempt,
    source_preflight_descriptor_digest: context.source_preflight_descriptor_digest,
    source_preflight_content_digest: context.source_preflight_content_digest,
    source_preflight_receipt_digest: context.source_preflight_receipt_digest,
    fence,
    role: "REVIEWER",
    subject_digest: subjectDigest,
    model_call_identity: modelCallIdentity,
    continuation,
  }), "utf8"))}`;
}

function reviewerProcessFence(context, descriptor, continuation) {
  return sha256(Buffer.from(canonicalJson({
    schema: "lattice.managed-review-process-fence/1.0",
    task_ref: context.task_ref,
    attempt: context.attempt,
    subject_digest: context.subject_digest,
    model_call_identity: context.model_call_identity,
    worktree_ref: context.worktree_ref,
    repository_head: context.repository_head,
    execution_environment_ref: context.execution_environment_ref,
    process_fence_authority_ref: descriptor.process_fence.identity_digest,
    continuation,
  }), "utf8"));
}

export function buildWsl2ProviderSubtreeMarker(untrustedContext, processMarker) {
  const code = "WSL2_PROVIDER_SUBTREE_MARKER_REJECTED";
  const context = validateContext(untrustedContext, code);
  const continuation = {
    retry_of: processMarker?.retry_of ?? null,
    reconnect_of: processMarker?.reconnect_of ?? null,
  };
  validContinuation(continuation, context.attempt, code);
  const markerContext = {
    ...context,
    credential_seal_digest: processMarker?.credential_seal_digest,
    continuation,
  };
  validateProcessMarker(processMarker, markerContext, code);
  const value = {
    schema: WSL2_PROVIDER_SUBTREE_MARKER_SCHEMA,
    status: "OPEN",
    ...context,
    role: "PROVIDER",
    provider_subtree_segment_ref: providerSubtreeSegmentRef(
      context, processMarker.fence, continuation,
    ),
    process_marker: structuredClone(processMarker),
    boot_id_digest: processMarker.boot_id_digest,
    credential_seal_digest: processMarker.credential_seal_digest,
    continuation,
    provider_effect_count: 0,
    marker_digest: null,
  };
  value.marker_digest = canonicalProviderSubtreeDigest(
    "provider-subtree-marker", value, "marker_digest",
  );
  return Object.freeze(boundedRecord(value, code));
}

export function validateWsl2ProviderSubtreeMarker(value) {
  const code = "WSL2_PROVIDER_SUBTREE_MARKER_REJECTED";
  exactKeys(value, MARKER_KEYS, code);
  const context = Object.fromEntries(MARKER_CONTEXT_KEYS.map((key) => [key, value[key]]));
  validateContext(context, code);
  validContinuation(value.continuation, value.attempt, code);
  ensure(value.schema === WSL2_PROVIDER_SUBTREE_MARKER_SCHEMA && value.status === "OPEN"
    && value.role === "PROVIDER" && value.provider_effect_count === 0
    && value.provider_subtree_segment_ref === providerSubtreeSegmentRef(
      value, value.process_marker?.fence, value.continuation,
    )
    && value.boot_id_digest === value.process_marker?.boot_id_digest
    && value.credential_seal_digest === value.process_marker?.credential_seal_digest, code);
  validateProcessMarker(value.process_marker, value, code);
  ensure(value.marker_digest === canonicalProviderSubtreeDigest(
    "provider-subtree-marker", value, "marker_digest",
  ), code);
  return boundedRecord(value, code);
}

export function buildWsl2ProviderSubtreeReceipt(
  untrustedMarker,
  subtreeExit,
  outerPostExit,
  providerEffectCount,
) {
  const code = "WSL2_PROVIDER_SUBTREE_RECEIPT_REJECTED";
  const markerRecord = validateWsl2ProviderSubtreeMarker(untrustedMarker);
  validateSubtreeExit(subtreeExit, markerRecord.process_marker, code);
  validateWsl2ProviderOuterPostExit(outerPostExit, markerRecord.process_marker);
  ensure(Number.isSafeInteger(providerEffectCount) && providerEffectCount >= 0
    && providerEffectCount <= MAX_PROVIDER_EFFECT_COUNT, code);
  const context = Object.fromEntries(MARKER_CONTEXT_KEYS.map((key) => [key, markerRecord[key]]));
  const value = {
    schema: WSL2_PROVIDER_SUBTREE_RECEIPT_SCHEMA,
    status: "CLOSED",
    ...context,
    role: "PROVIDER",
    provider_subtree_segment_ref: markerRecord.provider_subtree_segment_ref,
    source_marker_digest: markerRecord.marker_digest,
    process_marker: structuredClone(markerRecord.process_marker),
    subtree_exit: structuredClone(subtreeExit),
    outer_post_exit: structuredClone(outerPostExit),
    boot_id_digest: markerRecord.boot_id_digest,
    credential_seal_digest: markerRecord.credential_seal_digest,
    continuation: structuredClone(markerRecord.continuation),
    provider_effect_count: providerEffectCount,
    receipt_digest: null,
  };
  value.receipt_digest = canonicalProviderSubtreeDigest(
    "provider-subtree-receipt", value, "receipt_digest",
  );
  return Object.freeze(boundedRecord(value, code));
}

export function validateWsl2ProviderSubtreeReceipt(value, untrustedMarker) {
  const code = "WSL2_PROVIDER_SUBTREE_RECEIPT_REJECTED";
  const markerRecord = validateWsl2ProviderSubtreeMarker(untrustedMarker);
  exactKeys(value, RECEIPT_KEYS, code);
  const context = Object.fromEntries(MARKER_CONTEXT_KEYS.map((key) => [key, value[key]]));
  validateContext(context, code);
  validContinuation(value.continuation, value.attempt, code);
  ensure(value.schema === WSL2_PROVIDER_SUBTREE_RECEIPT_SCHEMA && value.status === "CLOSED"
    && value.role === "PROVIDER" && value.source_marker_digest === markerRecord.marker_digest
    && value.provider_subtree_segment_ref === markerRecord.provider_subtree_segment_ref
    && MARKER_CONTEXT_KEYS.every((key) => value[key] === markerRecord[key])
    && canonicalJson(value.process_marker) === canonicalJson(markerRecord.process_marker)
    && value.boot_id_digest === markerRecord.boot_id_digest
    && value.credential_seal_digest === markerRecord.credential_seal_digest
    && canonicalJson(value.continuation) === canonicalJson(markerRecord.continuation)
    && Number.isSafeInteger(value.provider_effect_count) && value.provider_effect_count >= 0
    && value.provider_effect_count <= MAX_PROVIDER_EFFECT_COUNT, code);
  validateSubtreeExit(value.subtree_exit, markerRecord.process_marker, code);
  try {
    validateWsl2ProviderOuterPostExit(value.outer_post_exit, markerRecord.process_marker);
  } catch {
    throw rejected(code);
  }
  ensure(value.receipt_digest === canonicalProviderSubtreeDigest(
    "provider-subtree-receipt", value, "receipt_digest",
  ), code);
  return boundedRecord(value, code);
}

export function buildWsl2ReviewerSubtreeMarker(
  untrustedContext,
  processMarker,
  subjectDigest,
  modelCallIdentity,
) {
  const code = "WSL2_REVIEWER_SUBTREE_MARKER_REJECTED";
  const context = validateContext(untrustedContext, code);
  ensure(HEX_64.test(subjectDigest)
    && typeof modelCallIdentity === "string"
    && /^managed-review-[a-f0-9]{64}-[1-3]$/u.test(modelCallIdentity), code);
  const continuation = {
    retry_of: processMarker?.retry_of ?? null,
    reconnect_of: processMarker?.reconnect_of ?? null,
  };
  validContinuation(continuation, context.attempt, code);
  validateProcessMarker(processMarker, {
    ...context,
    credential_seal_digest: processMarker?.credential_seal_digest,
    continuation,
  }, code);
  const value = {
    schema: WSL2_PROVIDER_SUBTREE_MARKER_SCHEMA,
    status: "OPEN",
    ...context,
    role: "REVIEWER",
    subject_digest: subjectDigest,
    model_call_identity: modelCallIdentity,
    provider_subtree_segment_ref: reviewerSubtreeSegmentRef(
      context, processMarker.fence, continuation, subjectDigest, modelCallIdentity,
    ),
    process_marker: structuredClone(processMarker),
    boot_id_digest: processMarker.boot_id_digest,
    credential_seal_digest: processMarker.credential_seal_digest,
    continuation,
    provider_effect_count: 0,
    marker_digest: null,
  };
  value.marker_digest = canonicalProviderSubtreeDigest(
    "provider-subtree-marker", value, "marker_digest",
  );
  return Object.freeze(boundedRecord(value, code));
}

export function validateWsl2ReviewerSubtreeMarker(value) {
  const code = "WSL2_REVIEWER_SUBTREE_MARKER_REJECTED";
  exactKeys(value, [...MARKER_KEYS, "subject_digest", "model_call_identity"], code);
  const context = Object.fromEntries(MARKER_CONTEXT_KEYS.map((key) => [key, value[key]]));
  validateContext(context, code);
  validContinuation(value.continuation, value.attempt, code);
  ensure(value.schema === WSL2_PROVIDER_SUBTREE_MARKER_SCHEMA && value.status === "OPEN"
    && value.role === "REVIEWER" && value.provider_effect_count === 0
    && HEX_64.test(value.subject_digest)
    && typeof value.model_call_identity === "string"
    && /^managed-review-[a-f0-9]{64}-[1-3]$/u.test(value.model_call_identity)
    && value.provider_subtree_segment_ref === reviewerSubtreeSegmentRef(
      value, value.process_marker?.fence, value.continuation, value.subject_digest,
      value.model_call_identity,
    )
    && value.boot_id_digest === value.process_marker?.boot_id_digest
    && value.credential_seal_digest === value.process_marker?.credential_seal_digest, code);
  validateProcessMarker(value.process_marker, value, code);
  ensure(value.marker_digest === canonicalProviderSubtreeDigest(
    "provider-subtree-marker", value, "marker_digest",
  ), code);
  return boundedRecord(value, code);
}

export function buildWsl2ReviewerSubtreeReceipt(
  untrustedMarker,
  subtreeExit,
  outerPostExit,
  providerEffectCount,
) {
  const code = "WSL2_REVIEWER_SUBTREE_RECEIPT_REJECTED";
  const markerRecord = validateWsl2ReviewerSubtreeMarker(untrustedMarker);
  validateSubtreeExit(subtreeExit, markerRecord.process_marker, code);
  validateWsl2ProviderOuterPostExit(outerPostExit, markerRecord.process_marker);
  ensure(Number.isSafeInteger(providerEffectCount) && providerEffectCount >= 0
    && providerEffectCount <= MAX_PROVIDER_EFFECT_COUNT, code);
  const context = Object.fromEntries(MARKER_CONTEXT_KEYS.map((key) => [key, markerRecord[key]]));
  const value = {
    schema: WSL2_PROVIDER_SUBTREE_RECEIPT_SCHEMA,
    status: "CLOSED",
    ...context,
    role: "REVIEWER",
    subject_digest: markerRecord.subject_digest,
    model_call_identity: markerRecord.model_call_identity,
    provider_subtree_segment_ref: markerRecord.provider_subtree_segment_ref,
    source_marker_digest: markerRecord.marker_digest,
    process_marker: structuredClone(markerRecord.process_marker),
    subtree_exit: structuredClone(subtreeExit),
    outer_post_exit: structuredClone(outerPostExit),
    boot_id_digest: markerRecord.boot_id_digest,
    credential_seal_digest: markerRecord.credential_seal_digest,
    continuation: structuredClone(markerRecord.continuation),
    provider_effect_count: providerEffectCount,
    receipt_digest: null,
  };
  value.receipt_digest = canonicalProviderSubtreeDigest(
    "provider-subtree-receipt", value, "receipt_digest",
  );
  return Object.freeze(boundedRecord(value, code));
}

export function validateWsl2ReviewerSubtreeReceipt(value, untrustedMarker) {
  const code = "WSL2_REVIEWER_SUBTREE_RECEIPT_REJECTED";
  const markerRecord = validateWsl2ReviewerSubtreeMarker(untrustedMarker);
  exactKeys(value, [...RECEIPT_KEYS, "subject_digest", "model_call_identity"], code);
  const context = Object.fromEntries(MARKER_CONTEXT_KEYS.map((key) => [key, value[key]]));
  validateContext(context, code);
  validContinuation(value.continuation, value.attempt, code);
  ensure(value.schema === WSL2_PROVIDER_SUBTREE_RECEIPT_SCHEMA && value.status === "CLOSED"
    && value.role === "REVIEWER"
    && value.subject_digest === markerRecord.subject_digest
    && value.model_call_identity === markerRecord.model_call_identity
    && value.source_marker_digest === markerRecord.marker_digest
    && value.provider_subtree_segment_ref === markerRecord.provider_subtree_segment_ref
    && MARKER_CONTEXT_KEYS.every((key) => value[key] === markerRecord[key])
    && canonicalJson(value.process_marker) === canonicalJson(markerRecord.process_marker)
    && value.boot_id_digest === markerRecord.boot_id_digest
    && value.credential_seal_digest === markerRecord.credential_seal_digest
    && canonicalJson(value.continuation) === canonicalJson(markerRecord.continuation)
    && Number.isSafeInteger(value.provider_effect_count) && value.provider_effect_count >= 0
    && value.provider_effect_count <= MAX_PROVIDER_EFFECT_COUNT, code);
  validateSubtreeExit(value.subtree_exit, markerRecord.process_marker, code);
  validateWsl2ProviderOuterPostExit(value.outer_post_exit, markerRecord.process_marker);
  ensure(value.receipt_digest === canonicalProviderSubtreeDigest(
    "provider-subtree-receipt", value, "receipt_digest",
  ), code);
  return boundedRecord(value, code);
}

const WSL2_PROVIDER_SUBTREE_PROBE_SOURCE = String.raw`
const cp = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const config = JSON.parse(process.argv[1]);
const hash = (value) => crypto.createHash("sha256").update(value).digest("hex");
const boot = () => { const id = fs.readFileSync("/proc/sys/kernel/random/boot_id", "utf8").trim();
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(id)) {
    throw new Error("BOOT_ID_INVALID");
  }
  return "wsl-boot:sha256:" + hash(Buffer.from(id, "utf8")); };
if (boot() !== config.boot_id_digest) throw new Error("BOOT_ID_MISMATCH");
const inspect = () => {
  const show = cp.spawnSync(config.systemctl, ["--user", "show", config.unit,
    "--property=ActiveState", "--property=SubState", "--property=Result",
    "--property=ControlGroup", "--property=Delegate"],
  { encoding: "utf8", timeout: 10000, maxBuffer: 65536,
    env: { ...process.env, XDG_RUNTIME_DIR: config.runtime_dir } });
  if (show.error || show.status !== 0 || show.stderr.length > 65536) {
    throw new Error("SYSTEMCTL_SHOW_FAILED");
  }
  const values = Object.fromEntries(show.stdout.replaceAll("\r", "").trim().split("\n")
    .map((line) => { const index = line.indexOf("=");
      if (index <= 0) throw new Error("SYSTEMCTL_SHOW_INVALID");
      return [line.slice(0, index), line.slice(index + 1)]; }));
  const cgroupPath = values.ControlGroup || config.cgroup_path;
  if (cgroupPath !== config.cgroup_path) throw new Error("CGROUP_PATH_MISMATCH");
  let exists = true; let populated = null;
  try {
    const events = fs.readFileSync(config.cgroup_mount + cgroupPath + "/cgroup.events", "utf8");
    const match = events.match(/(?:^|\n)populated\s+(\d+)(?:\n|$)/u);
    if (!match) throw new Error("CGROUP_EVENTS_INVALID");
    populated = Number(match[1]);
  } catch (error) { if (error && error.code === "ENOENT") exists = false; else throw error; }
  return { schema: "lattice.wsl2-provider-outer-post-exit/1.0", unit: config.unit,
    fence: config.fence, cgroup_path: cgroupPath, boot_id_digest: boot(),
    active_state: values.ActiveState, sub_state: values.SubState, result: values.Result,
    delegate: values.Delegate, cgroup_exists: exists, populated };
};
const actions = [];
const run = (action, args) => {
  const result = cp.spawnSync(config.systemctl, ["--user", ...args, config.unit],
    { encoding: "utf8", timeout: 10000, maxBuffer: 65536,
      env: { ...process.env, XDG_RUNTIME_DIR: config.runtime_dir } });
  const stdout = Buffer.from(result.stdout || "", "utf8");
  const stderr = Buffer.from(result.stderr || "", "utf8");
  if (stdout.length > 65536 || stderr.length > 65536) throw new Error("CLEANUP_OUTPUT_BOUND");
  actions.push({ sequence: actions.length + 1, action,
    result: result.error ? "TRANSPORT_ERROR" : result.status === 0 ? "SUCCESS" : "EXIT_NONZERO",
    exit_code: Number.isInteger(result.status) ? result.status : null,
    signal: typeof result.signal === "string" ? result.signal : null,
    stdout_bytes: stdout.length, stderr_bytes: stderr.length,
    stdout_sha256: hash(stdout), stderr_sha256: hash(stderr) });
};
let outer = inspect();
const closed = (value) => value.active_state === "inactive" && value.sub_state === "dead"
  && value.delegate === "no" && ((!value.cgroup_exists && value.populated === null)
    || (value.cgroup_exists && value.populated === 0));
if (config.cleanup && !closed(outer)) {
  run("TERM", ["kill", "--kill-who=all", "--signal=SIGTERM"]);
  run("STOP", ["stop"]);
  outer = inspect();
  if (!closed(outer)) {
    run("KILL", ["kill", "--kill-who=all", "--signal=SIGKILL"]);
    run("FORCE_STOP", ["stop", "--force"]);
    outer = inspect();
  }
}
if (!closed(outer) || boot() !== config.boot_id_digest) throw new Error("SUBTREE_NOT_CLOSED");
process.stdout.write(JSON.stringify({ cleanup: {
  schema: "lattice.wsl2-provider-subtree-cleanup/1.0", actions }, outer_post_exit: outer }) + "\n");
`;

function closedHostEnvironment() {
  return Object.fromEntries(["SystemRoot", "WINDIR"].flatMap((key) => (
    process.env[key] === undefined ? [] : [[key, process.env[key]]]
  )));
}

function onlyJsonLine(stdout, code) {
  ensure(typeof stdout === "string", code);
  const lines = stdout.replaceAll("\r", "").split("\n").filter(Boolean);
  ensure(lines.length === 1 && Buffer.byteLength(lines[0], "utf8") <= MAX_RECORD_BYTES, code);
  try {
    return JSON.parse(lines[0]);
  } catch {
    throw rejected(code);
  }
}

async function defaultRunProbe({ gateway, distribution, node, expected, runtimeDir, systemctl,
  cgroupMount, cleanup }) {
  const config = {
    systemctl,
    unit: expected.unit,
    runtime_dir: runtimeDir,
    cgroup_mount: cgroupMount,
    cgroup_path: expected.cgroup_path,
    fence: expected.fence,
    boot_id_digest: expected.boot_id_digest,
    cleanup,
  };
  const result = await execFileDefault(gateway, [
    "-d", distribution, "--exec", "/usr/bin/env", "-i",
    `XDG_RUNTIME_DIR=${runtimeDir}`, "LANG=C.UTF-8", "LC_ALL=C.UTF-8",
    node, "-e", WSL2_PROVIDER_SUBTREE_PROBE_SOURCE, canonicalJson(config),
  ], {
    encoding: "utf8",
    windowsHide: true,
    timeout: 30_000,
    maxBuffer: 65_536,
    env: closedHostEnvironment(),
  });
  ensure(result.stderr === "", "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED");
  return onlyJsonLine(result.stdout, "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED");
}

function validateCleanup(value, code) {
  exactKeys(value, ["schema", "actions"], code);
  ensure(value.schema === CLEANUP_SCHEMA && Array.isArray(value.actions)
    && value.actions.length <= 4, code);
  const sequence = ["TERM", "STOP", "KILL", "FORCE_STOP"];
  for (const [index, action] of value.actions.entries()) {
    exactKeys(action, [
      "sequence", "action", "result", "exit_code", "signal", "stdout_bytes", "stderr_bytes",
      "stdout_sha256", "stderr_sha256",
    ], code);
    ensure(action.sequence === index + 1 && action.action === sequence[index]
      && ["SUCCESS", "EXIT_NONZERO", "TRANSPORT_ERROR"].includes(action.result)
      && (action.exit_code === null || (Number.isSafeInteger(action.exit_code)
        && action.exit_code >= 0 && action.exit_code <= 255))
      && (action.signal === null || (typeof action.signal === "string"
        && /^[A-Z0-9]{1,32}$/u.test(action.signal)))
      && Number.isSafeInteger(action.stdout_bytes) && action.stdout_bytes >= 0
      && action.stdout_bytes <= 65_536
      && Number.isSafeInteger(action.stderr_bytes) && action.stderr_bytes >= 0
      && action.stderr_bytes <= 65_536
      && HEX_64.test(action.stdout_sha256) && HEX_64.test(action.stderr_sha256), code);
  }
  ensure(value.actions.length === 0 || value.actions.length === 2 || value.actions.length === 4,
    code);
  return value;
}

function preflightAnchor(input, dependencies) {
  const code = "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED";
  exactKeys(input, [
    "schema", "descriptor_json", "descriptor_digest", "source_preflight", "open_marker",
    "packet_digest", "provider_effect_count_before", "provider_effect_count_after",
  ], code);
  exactKeys(input.source_preflight, ["descriptor_digest", "content_digest", "receipt_json"], code);
  ensure(input.schema === WSL2_PROVIDER_SUBTREE_RECONCILE_REQUEST_SCHEMA
    && typeof input.descriptor_json === "string" && input.descriptor_json.length <= 65_536
    && validTyped(input.packet_digest, "attempt-packet")
    && HEX_64.test(input.descriptor_digest)
    && sha256(Buffer.from(input.descriptor_json, "utf8")) === input.descriptor_digest
    && HEX_64.test(input.source_preflight.descriptor_digest)
    && HEX_64.test(input.source_preflight.content_digest)
    && typeof input.source_preflight.receipt_json === "string"
    && input.source_preflight.receipt_json.length <= 65_536
    && sha256(Buffer.from(input.source_preflight.receipt_json, "utf8"))
      === input.source_preflight.content_digest
    && Number.isSafeInteger(input.provider_effect_count_before)
    && input.provider_effect_count_before >= 0
    && input.provider_effect_count_before <= MAX_PROVIDER_EFFECT_COUNT
    && input.provider_effect_count_after === input.provider_effect_count_before, code);
  let descriptor;
  let receipt;
  try {
    descriptor = (dependencies.validateEnvironment ?? validateWsl2ExecutionEnvironment)(
      JSON.parse(input.descriptor_json),
    );
    receipt = JSON.parse(input.source_preflight.receipt_json);
  } catch {
    throw rejected(code);
  }
  ensure(receipt?.schema === "lattice.wsl2-zero-model-preflight/1.0"
    && receipt.status === "PASS" && HEX_64.test(receipt.task_ref)
    && Number.isSafeInteger(receipt.attempt) && receipt.attempt >= 1 && receipt.attempt <= 3
    && validTyped(receipt.worktree_ref, "worktree") && HEX_40.test(receipt.repository_head)
    && receipt.execution_environment_ref === descriptor.identity_digest
    && validTyped(receipt.credential_seal_digest, "credential-seal")
    && HEX_64.test(receipt.process_fence?.fence)
    && validTyped(receipt.process_fence?.boot_id_digest, "wsl-boot")
    && Number.isSafeInteger(receipt.bounds?.stdout_limit_bytes)
    && Number.isSafeInteger(receipt.bounds?.stderr_limit_bytes)
    && Number.isSafeInteger(receipt.timeout?.timeout_ms)
    && receipt.provider_effect_count === 0
    && validTyped(receipt.receipt_digest, "wsl2-preflight"), code);
  const continuation = receipt.continuation;
  validContinuation(continuation, receipt.attempt, code);
  const launch = (dependencies.buildLaunch ?? buildWsl2CodexLaunch)(descriptor, {
    fence: receipt.process_fence.fence,
    preflightReceipt: receipt,
    attempt: receipt.attempt,
    retryOf: continuation.retry_of,
    reconnectOf: continuation.reconnect_of,
    timeoutMs: receipt.timeout.timeout_ms,
    stdoutLimitBytes: receipt.bounds.stdout_limit_bytes,
    stderrLimitBytes: receipt.bounds.stderr_limit_bytes,
  });
  ensure(launch.processFence === receipt.process_fence.fence
    && typeof launch.serviceUnit === "string"
    && launch.postExitProbe?.distribution === descriptor.distribution
    && launch.postExitProbe.unit === launch.serviceUnit
    && launch.postExitProbe.process_fence === launch.processFence
    && launch.postExitProbe.authority_ref === descriptor.process_fence.identity_digest
    && launch.postExitProbe.systemctl_path === descriptor.process_fence.systemctl_path
    && launch.postExitProbe.cgroup_mount === descriptor.process_fence.cgroup_mount, code);
  return { descriptor, receipt, continuation, launch };
}

export async function probeWsl2ProviderPostExit(launch, processMarker, dependencies = {}) {
  const code = "WSL2_PROVIDER_OUTER_POST_EXIT_REJECTED";
  ensure(object(launch?.postExitProbe) && object(processMarker), code);
  const expected = {
    unit: launch.serviceUnit,
    fence: launch.processFence,
    cgroup_path: processMarker.cgroup_path,
    boot_id_digest: processMarker.boot_id_digest,
  };
  ensure(processMarker.unit === expected.unit && processMarker.fence === expected.fence
    && launch.postExitProbe.unit === expected.unit
    && launch.postExitProbe.process_fence === expected.fence, code);
  const runtimeDirArguments = Array.isArray(launch.args)
    ? launch.args.filter((value) => typeof value === "string"
      && /^XDG_RUNTIME_DIR=\/run\/user\/\d+$/u.test(value))
    : [];
  const runtimeDir = launch.postExitProbe.user_runtime_dir
    ?? (runtimeDirArguments.length === 1
      ? runtimeDirArguments[0].slice("XDG_RUNTIME_DIR=".length)
      : null);
  ensure(typeof runtimeDir === "string" && /^\/run\/user\/\d+$/u.test(runtimeDir), code);
  const observed = await (dependencies.runProbe ?? defaultRunProbe)({
    gateway: launch.command,
    distribution: launch.postExitProbe.distribution,
    node: "/usr/bin/node",
    expected,
    runtimeDir,
    systemctl: launch.postExitProbe.systemctl_path,
    cgroupMount: launch.postExitProbe.cgroup_mount,
    cleanup: false,
  });
  validateCleanup(observed.cleanup, code);
  ensure(observed.cleanup.actions.length === 0, code);
  return validateWsl2ProviderOuterPostExit(observed.outer_post_exit, expected);
}

export async function reconcileWsl2ProviderSubtree(untrusted, dependencies = {}) {
  const code = "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED";
  try {
    const input = structuredClone(untrusted);
    const { descriptor, receipt, continuation, launch } = preflightAnchor(input, dependencies);
    let open = null;
    if (input.open_marker !== null) {
      open = validateWsl2ProviderSubtreeMarker(input.open_marker);
      ensure(open.task_ref === receipt.task_ref && open.attempt === receipt.attempt
        && open.worktree_ref === receipt.worktree_ref
        && open.repository_head === receipt.repository_head
        && open.execution_environment_ref === descriptor.identity_digest
        && open.descriptor_digest === input.descriptor_digest
        && open.source_preflight_descriptor_digest === input.source_preflight.descriptor_digest
        && open.source_preflight_content_digest === input.source_preflight.content_digest
        && open.source_preflight_receipt_digest === receipt.receipt_digest
        && open.packet_digest === input.packet_digest
        && open.process_marker.fence === launch.processFence
        && open.process_marker.unit === launch.serviceUnit
        && open.process_marker.boot_id_digest === receipt.process_fence.boot_id_digest
        && open.process_marker.credential_seal_digest === receipt.credential_seal_digest
        && canonicalJson(open.continuation) === canonicalJson(continuation), code);
    }
    const unit = launch.serviceUnit;
    const cgroupPath = open?.process_marker.cgroup_path
      ?? `/user.slice/user-${descriptor.verification_toolchain.owner_uid}.slice/`
        + `user@${descriptor.verification_toolchain.owner_uid}.service/app.slice/${unit}`;
    const expected = {
      unit,
      fence: launch.processFence,
      cgroup_path: cgroupPath,
      boot_id_digest: receipt.process_fence.boot_id_digest,
    };
    const observed = await (dependencies.runProbe ?? defaultRunProbe)({
      gateway: descriptor.gateway.windows_path,
      distribution: descriptor.distribution,
      node: descriptor.process_fence.supervisor_bootstrap_node.path,
      expected,
      runtimeDir: descriptor.process_fence.user_runtime_dir,
      systemctl: descriptor.process_fence.systemctl_path,
      cgroupMount: descriptor.process_fence.cgroup_mount,
      cleanup: true,
    });
    const cleanup = validateCleanup(observed.cleanup, code);
    const outer = validateWsl2ProviderOuterPostExit(observed.outer_post_exit, expected);
    const value = {
      schema: WSL2_PROVIDER_SUBTREE_RECONCILIATION_SCHEMA,
      status: "RECONCILED",
      task_ref: receipt.task_ref,
      attempt: receipt.attempt,
      worktree_ref: receipt.worktree_ref,
      repository_head: receipt.repository_head,
      execution_environment_ref: descriptor.identity_digest,
      descriptor_digest: input.descriptor_digest,
      source_preflight_descriptor_digest: input.source_preflight.descriptor_digest,
      source_preflight_content_digest: input.source_preflight.content_digest,
      source_preflight_receipt_digest: receipt.receipt_digest,
      role: "PROVIDER",
      provider_subtree_segment_ref: providerSubtreeSegmentRef(
        {
          task_ref: receipt.task_ref,
          attempt: receipt.attempt,
          source_preflight_descriptor_digest: input.source_preflight.descriptor_digest,
          source_preflight_content_digest: input.source_preflight.content_digest,
          source_preflight_receipt_digest: receipt.receipt_digest,
        },
        expected.fence,
        continuation,
      ),
      marker_observation: open === null ? "ABSENT_AFTER_TRANSPORT_LOSS" : "PRESENT",
      source_marker_digest: open?.marker_digest ?? null,
      packet_digest: input.packet_digest,
      process_marker: open === null ? null : structuredClone(open.process_marker),
      fence: expected.fence,
      unit: expected.unit,
      cgroup_path: expected.cgroup_path,
      boot_id_digest: expected.boot_id_digest,
      credential_seal_digest: receipt.credential_seal_digest,
      continuation: structuredClone(continuation),
      cleanup: structuredClone(cleanup),
      outer_post_exit: structuredClone(outer),
      provider_effect_count_before: input.provider_effect_count_before,
      provider_effect_count_after: input.provider_effect_count_after,
      reconciliation_digest: null,
    };
    value.reconciliation_digest = canonicalProviderSubtreeDigest(
      "provider-subtree-reconciliation", value, "reconciliation_digest",
    );
    exactKeys(value, RECONCILIATION_KEYS, code);
    return Object.freeze(boundedRecord(value, code));
  } catch (error) {
    if (error?.code === code) throw error;
    throw rejected(code);
  }
}

export async function reconcileWsl2ReviewerSubtree(untrusted, dependencies = {}) {
  const code = "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED";
  try {
    const input = structuredClone(untrusted);
    exactKeys(input, [
      "schema", "descriptor_json", "descriptor_digest", "source_preflight", "open_marker",
      "packet_digest", "provider_effect_count_before", "provider_effect_count_after",
      "reviewer_context",
    ], code);
    exactKeys(input.reviewer_context, [
      "task_ref", "attempt", "subject_digest", "model_call_identity", "worktree_ref",
      "repository_head", "execution_environment_ref", "packet_digest",
    ], code);
    ensure(input.schema === WSL2_REVIEWER_SUBTREE_RECONCILE_REQUEST_SCHEMA
      && HEX_64.test(input.reviewer_context.task_ref)
      && Number.isSafeInteger(input.reviewer_context.attempt)
      && input.reviewer_context.attempt >= 1 && input.reviewer_context.attempt <= 3
      && HEX_64.test(input.reviewer_context.subject_digest)
      && input.reviewer_context.model_call_identity
        === `managed-review-${input.reviewer_context.task_ref}-${input.reviewer_context.attempt}`
      && validTyped(input.reviewer_context.worktree_ref, "worktree")
      && HEX_40.test(input.reviewer_context.repository_head)
      && validTyped(input.reviewer_context.execution_environment_ref, "execution-environment")
      && validTyped(input.reviewer_context.packet_digest, "attempt-packet")
      && input.reviewer_context.packet_digest === input.packet_digest, code);
    const providerInput = {
      schema: WSL2_PROVIDER_SUBTREE_RECONCILE_REQUEST_SCHEMA,
      descriptor_json: input.descriptor_json,
      descriptor_digest: input.descriptor_digest,
      source_preflight: input.source_preflight,
      open_marker: input.open_marker,
      packet_digest: input.packet_digest,
      provider_effect_count_before: input.provider_effect_count_before,
      provider_effect_count_after: input.provider_effect_count_after,
    };
    const { descriptor, receipt, continuation, launch } = preflightAnchor(
      providerInput, dependencies,
    );
    const context = input.reviewer_context;
    ensure(context.task_ref === receipt.task_ref && context.attempt === receipt.attempt
      && context.worktree_ref === receipt.worktree_ref
      && context.repository_head === receipt.repository_head
      && context.execution_environment_ref === descriptor.identity_digest, code);
    ensure(launch.processFence === reviewerProcessFence(context, descriptor, continuation), code);
    let open = null;
    if (input.open_marker !== null) {
      open = validateWsl2ReviewerSubtreeMarker(input.open_marker);
      ensure(open.task_ref === context.task_ref && open.attempt === context.attempt
        && open.worktree_ref === context.worktree_ref
        && open.repository_head === context.repository_head
        && open.execution_environment_ref === context.execution_environment_ref
        && open.descriptor_digest === input.descriptor_digest
        && open.source_preflight_descriptor_digest === input.source_preflight.descriptor_digest
        && open.source_preflight_content_digest === input.source_preflight.content_digest
        && open.source_preflight_receipt_digest === receipt.receipt_digest
        && open.packet_digest === input.packet_digest
        && open.model_call_identity === context.model_call_identity
        && open.process_marker.fence === launch.processFence
        && open.process_marker.unit === launch.serviceUnit
        && canonicalJson(open.continuation) === canonicalJson(continuation), code);
    }
    const unit = launch.serviceUnit;
    const cgroupPath = open?.process_marker.cgroup_path
      ?? `/user.slice/user-${descriptor.verification_toolchain.owner_uid}.slice/`
        + `user@${descriptor.verification_toolchain.owner_uid}.service/app.slice/${unit}`;
    const expected = {
      unit,
      fence: launch.processFence,
      cgroup_path: cgroupPath,
      boot_id_digest: receipt.process_fence.boot_id_digest,
    };
    const observed = await (dependencies.runProbe ?? defaultRunProbe)({
      gateway: descriptor.gateway.windows_path,
      distribution: descriptor.distribution,
      node: descriptor.process_fence.supervisor_bootstrap_node.path,
      expected,
      runtimeDir: descriptor.process_fence.user_runtime_dir,
      systemctl: descriptor.process_fence.systemctl_path,
      cgroupMount: descriptor.process_fence.cgroup_mount,
      cleanup: true,
    });
    const cleanup = validateCleanup(observed.cleanup, code);
    const outer = validateWsl2ProviderOuterPostExit(observed.outer_post_exit, expected);
    const value = {
      schema: WSL2_PROVIDER_SUBTREE_RECONCILIATION_SCHEMA,
      status: "RECONCILED",
      task_ref: context.task_ref,
      attempt: context.attempt,
      worktree_ref: context.worktree_ref,
      repository_head: context.repository_head,
      execution_environment_ref: context.execution_environment_ref,
      descriptor_digest: input.descriptor_digest,
      source_preflight_descriptor_digest: input.source_preflight.descriptor_digest,
      source_preflight_content_digest: input.source_preflight.content_digest,
      source_preflight_receipt_digest: receipt.receipt_digest,
      role: "REVIEWER",
      subject_digest: context.subject_digest,
      model_call_identity: context.model_call_identity,
      provider_subtree_segment_ref: reviewerSubtreeSegmentRef(
        {
          task_ref: context.task_ref,
          attempt: context.attempt,
          source_preflight_descriptor_digest: input.source_preflight.descriptor_digest,
          source_preflight_content_digest: input.source_preflight.content_digest,
          source_preflight_receipt_digest: receipt.receipt_digest,
        },
        expected.fence,
        continuation,
        context.subject_digest,
        context.model_call_identity,
      ),
      marker_observation: open === null ? "ABSENT_AFTER_TRANSPORT_LOSS" : "PRESENT",
      source_marker_digest: open?.marker_digest ?? null,
      packet_digest: input.packet_digest,
      process_marker: open === null ? null : structuredClone(open.process_marker),
      fence: expected.fence,
      unit: expected.unit,
      cgroup_path: expected.cgroup_path,
      boot_id_digest: expected.boot_id_digest,
      credential_seal_digest: receipt.credential_seal_digest,
      continuation: structuredClone(continuation),
      cleanup: structuredClone(cleanup),
      outer_post_exit: structuredClone(outer),
      provider_effect_count_before: input.provider_effect_count_before,
      provider_effect_count_after: input.provider_effect_count_after,
      reconciliation_digest: null,
    };
    value.reconciliation_digest = canonicalProviderSubtreeDigest(
      "provider-subtree-reconciliation", value, "reconciliation_digest",
    );
    exactKeys(value, REVIEWER_RECONCILIATION_KEYS, code);
    return Object.freeze(boundedRecord(value, code));
  } catch (error) {
    if (error?.code === code) throw error;
    throw rejected(code);
  }
}

async function readBoundedStdin() {
  const chunks = [];
  let bytes = 0;
  for await (const chunk of process.stdin) {
    bytes += chunk.length;
    ensure(bytes <= MAX_INPUT_BYTES, "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED");
    chunks.push(chunk);
  }
  const lines = Buffer.concat(chunks).toString("utf8").replaceAll("\r", "")
    .split("\n").filter(Boolean);
  ensure(lines.length === 1, "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED");
  return JSON.parse(lines[0]);
}

if (process.argv[1] && import.meta.filename === process.argv[1]) {
  try {
    const request = await readBoundedStdin();
    const result = request?.schema === WSL2_REVIEWER_SUBTREE_RECONCILE_REQUEST_SCHEMA
      ? await reconcileWsl2ReviewerSubtree(request)
      : await reconcileWsl2ProviderSubtree(request);
    process.stdout.write(`${JSON.stringify(result)}\n`);
  } catch {
    process.stdout.write(`${JSON.stringify({
      schema: WSL2_PROVIDER_SUBTREE_RECONCILIATION_SCHEMA,
      status: "REJECTED",
      code: "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED",
    })}\n`);
    process.exitCode = 2;
  }
}
