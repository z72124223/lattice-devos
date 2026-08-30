import { createHash } from "node:crypto";
import process from "node:process";
import { fileURLToPath } from "node:url";

import {
  bindWsl2ExecutionWorktree,
  canonicalJson,
  MAX_WSL2_ATTEMPTS,
  validateWsl2ExecutionEnvironment,
} from "./wsl2-execution-domain.mjs";
import { preflightWsl2ExecutionEnvironment } from "./wsl2-execution-preflight.mjs";

const REQUEST_SCHEMA = "lattice.wsl2-execution-preflight-request/1.0";
const RESULT_SCHEMA = "lattice.wsl2-execution-preflight-result/1.0";
const MAX_INPUT_BYTES = 262_144;
const MAX_OUTPUT_BYTES = 1_048_576;
const HEX_40 = /^[a-f0-9]{40}$/u;
const HEX_64 = /^[a-f0-9]{64}$/u;
const TYPED = /^[a-z0-9][a-z0-9-]*:sha256:[a-f0-9]{64}$/u;

function bridgeError(code) {
  const error = new Error(code);
  error.code = code;
  return error;
}

function ensure(condition, code = "WSL2_PREFLIGHT_BRIDGE_REQUEST_REJECTED") {
  if (!condition) throw bridgeError(code);
}

function object(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function exactKeys(value, keys) {
  ensure(object(value));
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  ensure(actual.length === expected.length && actual.every((key, index) => key === expected[index]));
}

function digest(domain, subject) {
  return `${domain}:sha256:${createHash("sha256").update(canonicalJson(subject), "utf8").digest("hex")}`;
}

export function validateWsl2PreflightBridgeRequest(untrusted) {
  exactKeys(untrusted, [
    "schema", "template_descriptor", "windows_worktree_path", "task_ref", "attempt",
    "worktree_ref", "expected_repository_head", "process_fence", "retry_of", "reconnect_of",
  ]);
  ensure(untrusted.schema === REQUEST_SCHEMA);
  ensure(object(untrusted.template_descriptor));
  ensure(typeof untrusted.windows_worktree_path === "string"
    && untrusted.windows_worktree_path.startsWith("\\\\wsl.localhost\\"));
  ensure(typeof untrusted.task_ref === "string" && HEX_64.test(untrusted.task_ref));
  ensure(Number.isSafeInteger(untrusted.attempt) && untrusted.attempt >= 1
    && untrusted.attempt <= MAX_WSL2_ATTEMPTS);
  ensure(/^worktree:sha256:[a-f0-9]{64}$/u.test(untrusted.worktree_ref));
  ensure(HEX_40.test(untrusted.expected_repository_head));
  ensure(HEX_64.test(untrusted.process_fence));
  ensure(untrusted.retry_of === null || (typeof untrusted.retry_of === "string" && TYPED.test(untrusted.retry_of)));
  ensure(untrusted.reconnect_of === null || (typeof untrusted.reconnect_of === "string" && TYPED.test(untrusted.reconnect_of)));
  ensure(untrusted.retry_of === null || untrusted.reconnect_of === null);
  return structuredClone(untrusted);
}

export async function runWsl2ExecutionPreflightBridge(untrusted, dependencies = {}) {
  const request = validateWsl2PreflightBridgeRequest(untrusted);
  const validateDescriptor = dependencies.validateDescriptor ?? validateWsl2ExecutionEnvironment;
  const bindWorktree = dependencies.bindWorktree ?? bindWsl2ExecutionWorktree;
  const preflight = dependencies.preflight ?? preflightWsl2ExecutionEnvironment;
  const template = validateDescriptor(request.template_descriptor);
  ensure(template.verification_toolchain.task_ref === request.task_ref);
  const provisionalRepositoryIdentity = digest("repository", {
    task_ref: request.task_ref,
    worktree_ref: request.worktree_ref,
    windows_worktree_path: request.windows_worktree_path,
    expected_repository_head: request.expected_repository_head,
  });
  const bound = bindWorktree(template, request.windows_worktree_path, {
    repository_identity: provisionalRepositoryIdentity,
    head: request.expected_repository_head,
  });
  const { environment, receipt } = await preflight(bound, {
    processFence: request.process_fence,
    taskRef: request.task_ref,
    attempt: request.attempt,
    worktreeRef: request.worktree_ref,
    retryOf: request.retry_of,
    reconnectOf: request.reconnect_of,
  }, dependencies.preflightDependencies ?? {});
  ensure(environment.schema === "lattice.execution-environment.wsl2-linux/1.1",
    "WSL2_PREFLIGHT_BRIDGE_RESULT_REJECTED");
  ensure(environment.linux.repository_head === request.expected_repository_head,
    "WSL2_PREFLIGHT_BRIDGE_REPOSITORY_HEAD_MISMATCH");
  ensure(environment.path_mapping.windows_path === request.windows_worktree_path
    && environment.linux.cwd === environment.path_mapping.linux_path
    && !environment.linux.cwd.startsWith("\\") && !environment.linux.cwd.startsWith("/mnt/c"),
  "WSL2_PREFLIGHT_BRIDGE_WORKTREE_MISMATCH");
  ensure(receipt.task_ref === request.task_ref && receipt.attempt === request.attempt
    && receipt.worktree_ref === request.worktree_ref
    && receipt.execution_environment_ref === environment.identity_digest
    && receipt.repository_head === request.expected_repository_head
    && receipt.process_fence.fence === request.process_fence
    && receipt.provider_effect_count === 0,
  "WSL2_PREFLIGHT_BRIDGE_RESULT_REJECTED");
  const result = {
    schema: RESULT_SCHEMA,
    status: "PASS",
    task_ref: request.task_ref,
    attempt: request.attempt,
    worktree_ref: request.worktree_ref,
    environment,
    receipt,
    result_digest: null,
  };
  result.result_digest = digest("wsl2-preflight-result", Object.fromEntries(
    Object.entries(result).filter(([key]) => key !== "result_digest"),
  ));
  ensure(Buffer.byteLength(canonicalJson(result), "utf8") <= MAX_OUTPUT_BYTES,
    "WSL2_PREFLIGHT_BRIDGE_OUTPUT_BOUND_EXCEEDED");
  return result;
}

export function parseWsl2PreflightBridgeInput(bytes) {
  ensure(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length <= MAX_INPUT_BYTES,
    "WSL2_PREFLIGHT_BRIDGE_INPUT_BOUND_EXCEEDED");
  const text = bytes.toString("utf8");
  const lines = text.replaceAll("\r", "").split("\n").filter((line) => line.length > 0);
  ensure(lines.length === 1, "WSL2_PREFLIGHT_BRIDGE_REQUEST_REJECTED");
  try {
    return JSON.parse(lines[0]);
  } catch {
    throw bridgeError("WSL2_PREFLIGHT_BRIDGE_REQUEST_REJECTED");
  }
}

async function readBoundedStdin() {
  const chunks = [];
  let length = 0;
  for await (const chunk of process.stdin) {
    length += chunk.length;
    ensure(length <= MAX_INPUT_BYTES, "WSL2_PREFLIGHT_BRIDGE_INPUT_BOUND_EXCEEDED");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

async function main() {
  const request = parseWsl2PreflightBridgeInput(await readBoundedStdin());
  const result = await runWsl2ExecutionPreflightBridge(request);
  const output = `${canonicalJson(result)}\n`;
  ensure(Buffer.byteLength(output, "utf8") <= MAX_OUTPUT_BYTES,
    "WSL2_PREFLIGHT_BRIDGE_OUTPUT_BOUND_EXCEEDED");
  process.stdout.write(output);
}

function fail(error) {
  const code = typeof error?.code === "string" && /^WSL2_[A-Z0-9_]+$/u.test(error.code)
    ? error.code
    : "WSL2_PREFLIGHT_BRIDGE_FAILED";
  process.stderr.write(`${JSON.stringify({ schema: RESULT_SCHEMA, status: "REJECTED", code })}\n`);
  process.exitCode = 70;
}

const isEntryPoint = process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];
if (isEntryPoint) main().catch(fail);

export const WSL2_PREFLIGHT_BRIDGE_MAX_INPUT_BYTES = MAX_INPUT_BYTES;
export const WSL2_PREFLIGHT_BRIDGE_MAX_OUTPUT_BYTES = MAX_OUTPUT_BYTES;
