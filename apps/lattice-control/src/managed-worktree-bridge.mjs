import readline from "node:readline";

import { WorkspaceError } from "../../../src/workspace/errors.js";
import { ManagedWorktreeOwner } from "./managed-worktree.mjs";

export const MANAGED_WORKTREE_COMMAND_SCHEMA =
  "lattice.managed-worktree-command/1.1";
export const MANAGED_WORKTREE_RESULT_SCHEMA =
  "lattice.managed-worktree-bridge-result/1.0";

const MAX_INPUT_BYTES = 32_768;
const MAX_OUTPUT_BYTES = 32_768;
const CLOSED_ERROR = /^MANAGED_WORKTREE_[A-Z0-9_]{1,80}$/u;
const CLOSED_OWNER_ERROR = /^[A-Z][A-Z0-9_]{0,80}$/u;
const EXECUTION_ENVIRONMENT_REF = /^execution-environment:sha256:[a-f0-9]{64}$/u;

function invalid(message) {
  const error = new TypeError(message);
  error.code = "MANAGED_WORKTREE_COMMAND_REJECTED";
  return error;
}

function validateCommand(value) {
  const common = [
    "schema",
    "operation",
    "repository_root",
    "worktree_root",
    "git_executable",
    "task_ref",
    "task_id",
    "base_commit",
    "expected_baseline_sha256",
    "expected_execution_environment_ref",
  ];
  const expected = new Set(value?.operation === "protect"
    ? [...common, "attempt", "writer_fence", "result_commit", "require_existing"]
    : common);
  if (
    !value
    || typeof value !== "object"
    || Array.isArray(value)
    || value.schema !== MANAGED_WORKTREE_COMMAND_SCHEMA
    || !["prepare", "verify", "protect"].includes(value.operation)
    || Object.keys(value).some((key) => !expected.has(key))
    || [...expected].some((key) => !Object.hasOwn(value, key))
    || (value.operation === "prepare" && value.expected_baseline_sha256 !== null)
    || (["verify", "protect"].includes(value.operation)
      && typeof value.expected_baseline_sha256 !== "string")
    || (value.expected_execution_environment_ref !== null
      && (
        typeof value.expected_execution_environment_ref !== "string"
        || !EXECUTION_ENVIRONMENT_REF.test(value.expected_execution_environment_ref)
      ))
    || (value.operation === "protect"
      && (
        !Number.isSafeInteger(value.attempt)
        || !Number.isSafeInteger(value.writer_fence)
        || value.writer_fence < 1
        || typeof value.result_commit !== "string"
        || typeof value.require_existing !== "boolean"
      ))
  ) {
    throw invalid("Managed worktree bridge command has an invalid closed shape.");
  }
  return value;
}

async function readOneCommand() {
  const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
  let retained = null;
  for await (const line of lines) {
    if (line.trim().length === 0) continue;
    if (retained !== null || Buffer.byteLength(line, "utf8") > MAX_INPUT_BYTES) {
      throw invalid("Managed worktree bridge accepts exactly one bounded command.");
    }
    retained = validateCommand(JSON.parse(line));
  }
  if (retained === null) throw invalid("Managed worktree bridge command is missing.");
  return retained;
}

function safeErrorCode(error) {
  return typeof error?.code === "string" && CLOSED_ERROR.test(error.code)
    ? error.code
    : error instanceof WorkspaceError && typeof error.code === "string"
      ? "MANAGED_WORKTREE_OWNER_REJECTED"
      : "MANAGED_WORKTREE_BRIDGE_FAILED";
}

function writeRecord(record) {
  const line = JSON.stringify(record);
  if (Buffer.byteLength(line, "utf8") > MAX_OUTPUT_BYTES) {
    throw invalid("Managed worktree bridge output exceeds its closed bound.");
  }
  process.stdout.write(`${line}\n`);
}

export async function runManagedWorktreeBridge() {
  let command = null;
  try {
    command = await readOneCommand();
    const owner = new ManagedWorktreeOwner({
      repositoryRoot: command.repository_root,
      worktreeRoot: command.worktree_root,
      gitExecutable: command.git_executable,
    });
    const result = command.operation === "protect"
      ? await owner.protectVerifiedResult({
        task_ref: command.task_ref,
        task_id: command.task_id,
        attempt: command.attempt,
        writer_fence: command.writer_fence,
        base_commit: command.base_commit,
        result_commit: command.result_commit,
        expected_baseline_sha256: command.expected_baseline_sha256,
        expected_execution_environment_ref: command.expected_execution_environment_ref,
        require_existing: command.require_existing,
      })
      : await owner.prepare({
        task_ref: command.task_ref,
        task_id: command.task_id,
        base_commit: command.base_commit,
        expected_baseline_sha256: command.expected_baseline_sha256,
        expected_execution_environment_ref: command.expected_execution_environment_ref,
        operation: command.operation,
      });
    if (command.operation === "protect") {
      writeRecord({
        schema: MANAGED_WORKTREE_RESULT_SCHEMA,
        kind: "result",
        operation: command.operation,
        task_ref: command.task_ref,
        task_id: command.task_id,
        attempt: command.attempt,
        writer_fence: command.writer_fence,
        base_commit: command.base_commit,
        result_commit: command.result_commit,
        worktree_path: result.worktree_path,
        protected_ref: result.protected_ref,
        baseline_sha256: result.baseline_sha256,
        replayed: result.replayed,
        protected_ref_digest: result.protected_ref_digest,
      });
      return 0;
    }
    writeRecord({
      schema: MANAGED_WORKTREE_RESULT_SCHEMA,
      kind: "result",
      operation: command.operation,
      task_ref: command.task_ref,
      task_id: command.task_id,
      base_commit: command.base_commit,
      worktree_id: result.worktree_id,
      worktree_path: result.worktree_path,
      branch: result.branch,
      replayed: result.replayed,
      baseline_json: result.baseline_json,
      baseline_sha256: result.baseline_sha256,
    });
    return 0;
  } catch (error) {
    writeRecord({
      schema: MANAGED_WORKTREE_RESULT_SCHEMA,
      kind: "error",
      operation: command?.operation ?? null,
      task_ref: typeof command?.task_ref === "string" ? command.task_ref : null,
      code: safeErrorCode(error),
      owner_code: error instanceof WorkspaceError
        && typeof error.code === "string"
        && CLOSED_OWNER_ERROR.test(error.code)
        ? error.code
        : null,
      message: "managed worktree bridge failed closed",
    });
    return error instanceof TypeError ? 2 : 3;
  }
}

if (process.argv[1] && import.meta.filename === process.argv[1]) {
  process.exitCode = await runManagedWorktreeBridge();
}
