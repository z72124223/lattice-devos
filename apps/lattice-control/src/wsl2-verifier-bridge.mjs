import { execFile as execFileCallback, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

import {
  buildWsl2VerifierLaunch,
  canonicalJson,
  MAX_WSL2_ATTEMPTS,
  validateWsl2SubtreeExitReceipt,
  validateWsl2ExecutionEnvironment,
  WSL2_PROCESS_MARKER_SCHEMA,
  WSL2_SUBTREE_EXIT_SCHEMA,
} from "./wsl2-execution-domain.mjs";
import { WSL2_CGROUP_EXIT_PROBE_SOURCE } from "./wsl2-execution-preflight.mjs";

const execFileDefault = promisify(execFileCallback);
const REQUEST_SCHEMA = "lattice.wsl2-verifier-request/1.0";
const RESULT_SCHEMA = "lattice.wsl2-verifier-result/1.0";
const TRANSPORT_FAILURE_SCHEMA = "lattice.wsl2-verifier-transport-failure/1.0";
const MARKER_SCHEMA = WSL2_PROCESS_MARKER_SCHEMA;
const EXIT_SCHEMA = WSL2_SUBTREE_EXIT_SCHEMA;
const MAX_INPUT_BYTES = 262_144;
const MAX_GIT_INPUT_BYTES = 48 * 1_048_576;
const MAX_GIT_STDIN_BYTES = 32 * 1_048_576;
const MAX_OUTPUT_BYTES = 1_048_576;
const HEX_40 = /^[a-f0-9]{40}$/u;
const HEX_64 = /^[a-f0-9]{64}$/u;
const TYPED = /^[a-z0-9][a-z0-9-]*:sha256:[a-f0-9]{64}$/u;
const CLOSED_ARGS = Object.freeze({
  NODE: Object.freeze(["run", "verify", "--offline", "--no-audit", "--no-fund"]),
  CARGO: Object.freeze(["test", "--locked", "--offline"]),
});
const GIT_PATH_KEYS = Object.freeze([
  "HOME", "TMPDIR", "GIT_CONFIG_GLOBAL", "GIT_WORK_TREE", "GIT_DIR", "GIT_COMMON_DIR",
  "GIT_OBJECT_DIRECTORY", "GIT_INDEX_FILE",
]);
const GIT_LITERAL_ENVIRONMENT = Object.freeze({
  NO_COLOR: "1",
  CI: "1",
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_CONFIG_COUNT: "0",
  GIT_TERMINAL_PROMPT: "0",
  GIT_OPTIONAL_LOCKS: "0",
  GIT_ATTR_NOSYSTEM: "1",
});
const GIT_IDENTITY_KEYS = Object.freeze([
  "GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GIT_AUTHOR_DATE",
  "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL", "GIT_COMMITTER_DATE",
]);
const GIT_BOOTSTRAP_PATH_KEYS = Object.freeze(["HOME", "TMPDIR", "GIT_CONFIG_GLOBAL"]);
const GIT_BOOTSTRAP_FORMS = Object.freeze([
  Object.freeze(["rev-parse", "--show-toplevel"]),
  Object.freeze(["rev-parse", "--verify", "HEAD^{commit}"]),
  Object.freeze(["rev-parse", "--absolute-git-dir"]),
  Object.freeze(["rev-parse", "--path-format=absolute", "--git-common-dir"]),
]);
const OID = /^[a-f0-9]{40}$/u;
const OUTER_WATCHDOGS = Object.freeze([
  "TIMED_OUT", "INTERRUPTED", "OUTPUT_BOUND_EXCEEDED",
]);
const CLEANUP_REASONS = Object.freeze([...OUTER_WATCHDOGS, "TRANSPORT_ERROR"]);
const OUTER_PROCESS_LIMIT_BYTES = MAX_OUTPUT_BYTES + 262_144;
const OUTER_CLEANUP_TIMEOUT_MS = 15_000;
const OUTER_CLEANUP_OUTPUT_LIMIT_BYTES = 65_536;
const CLEANUP_RESULTS = Object.freeze([
  "SUCCESS", "EXIT_NONZERO", "TIMED_OUT", "OUTPUT_BOUND_EXCEEDED", "TRANSPORT_ERROR",
]);
const GIT_CONTROL_ROOT_SCHEMA = "lattice.wsl2-git-control-root/1.0";

function rejected(code = "WSL2_VERIFIER_BRIDGE_REQUEST_REJECTED") {
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

function exactKeys(value, keys, code) {
  ensure(object(value), code);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  ensure(actual.length === expected.length && actual.every((key, index) => key === expected[index]), code);
}

function digest(domain, value) {
  return `${domain}:sha256:${createHash("sha256").update(canonicalJson(value), "utf8").digest("hex")}`;
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function canonicalLinuxHomePath(value) {
  return typeof value === "string" && value.startsWith("/home/")
    && !value.includes("\\") && !value.includes("\0") && !value.includes("/../")
    && !value.endsWith("/..") && !value.includes("/./") && !value.endsWith("/.");
}

export function deriveWsl2GitControlRootIdentity(untrusted) {
  const code = "WSL2_VERIFIER_GIT_CONTROL_ROOT_REJECTED";
  exactKeys(untrusted, [
    "task_ref", "attempt", "worktree_ref", "execution_environment_ref",
    "preflight_receipt_ref", "repository_head", "isolation_root",
  ], code);
  ensure(HEX_64.test(untrusted.task_ref)
    && Number.isSafeInteger(untrusted.attempt)
    && untrusted.attempt >= 1 && untrusted.attempt <= MAX_WSL2_ATTEMPTS
    && /^worktree:sha256:[a-f0-9]{64}$/u.test(untrusted.worktree_ref)
    && /^execution-environment:sha256:[a-f0-9]{64}$/u.test(
      untrusted.execution_environment_ref,
    )
    && /^wsl2-preflight:sha256:[a-f0-9]{64}$/u.test(untrusted.preflight_receipt_ref)
    && HEX_40.test(untrusted.repository_head)
    && canonicalLinuxHomePath(untrusted.isolation_root), code);
  const binding = {
    schema: GIT_CONTROL_ROOT_SCHEMA,
    task_ref: untrusted.task_ref,
    attempt: untrusted.attempt,
    worktree_ref: untrusted.worktree_ref,
    execution_environment_ref: untrusted.execution_environment_ref,
    preflight_receipt_ref: untrusted.preflight_receipt_ref,
    repository_head: untrusted.repository_head,
    isolation_root: untrusted.isolation_root,
  };
  const locatorKey = sha256(Buffer.from(canonicalJson(binding), "utf8"));
  const subject = {
    ...binding,
    locator: `${untrusted.isolation_root}/git-control/attempt-${untrusted.attempt}-${locatorKey}`,
  };
  return Object.freeze({
    ...subject,
    identity_ref: digest("wsl2-git-control-root", subject),
  });
}

function decodeCanonicalBase64(value, code) {
  ensure(typeof value === "string"
    && /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/u.test(value), code);
  const bytes = Buffer.from(value, "base64");
  ensure(bytes.toString("base64") === value, code);
  return bytes;
}

function safeRepositoryPath(value) {
  return typeof value === "string" && value.length > 0 && value.length <= 4_096
    && !value.startsWith("/") && !value.startsWith("-") && !value.includes("\\")
    && !/[\0\r\n]/u.test(value)
    && value.split("/").every((component) => component.length > 0
      && component !== "." && component !== ".." && component !== ".git");
}

function sameArgs(actual, expected) {
  return actual.length === expected.length && actual.every((value, index) => value === expected[index]);
}

function validateGitOperation(args, phase, stdin, code) {
  const operation = args.slice(11);
  if (phase === "BOOTSTRAP") {
    ensure(stdin === null && GIT_BOOTSTRAP_FORMS.some((form) => sameArgs(operation, form)), code);
    return;
  }
  const [command, ...rest] = operation;
  let accepted = false;
  switch (command) {
    case "rev-parse":
      accepted = sameArgs(rest, ["--verify", "HEAD^{commit}"])
        || (rest.length === 2 && rest[0] === "--verify"
          && (/^[a-f0-9]{40}\^\{tree\}$/u.test(rest[1]) || /^[a-f0-9]{40}\^1$/u.test(rest[1])));
      break;
    case "for-each-ref":
      accepted = sameArgs(rest, ["--format=%(refname)%00%(objectname)%00"]);
      break;
    case "ls-files":
      accepted = sameArgs(rest, ["--others", "--ignored", "--exclude-standard", "-z", "--"])
        || sameArgs(rest, ["--others", "--exclude-standard", "-z", "--"])
        || sameArgs(rest, ["--cached", "--others", "--exclude-standard", "-z", "--"]);
      break;
    case "ls-tree":
      accepted = sameArgs(rest.slice(0, 5), ["-r", "-z", "--name-only", rest[3], "--"])
          && rest.length === 5 && OID.test(rest[3])
        || (rest.length === 4 && rest[0] === "-z" && OID.test(rest[1])
          && rest[2] === "--" && safeRepositoryPath(rest[3]))
        || (rest.length === 5 && rest[0] === "-z" && rest[1] === "--name-only"
          && OID.test(rest[2]) && rest[3] === "--" && safeRepositoryPath(rest[4]))
        || (rest.length === 3 && OID.test(rest[0]) && rest[1] === "--"
          && safeRepositoryPath(rest[2]));
      break;
    case "show": {
      const separator = rest.length === 1 ? rest[0].indexOf(":") : -1;
      accepted = separator === 40 && OID.test(rest[0].slice(0, separator))
        && safeRepositoryPath(rest[0].slice(separator + 1));
      break;
    }
    case "read-tree":
      accepted = rest.length === 1 && OID.test(rest[0]);
      break;
    case "hash-object":
      accepted = sameArgs(rest, ["-w", "--stdin"]);
      break;
    case "update-index": {
      const cache = rest.length === 3 && rest[0] === "--add" && rest[1] === "--cacheinfo"
        ? rest[2] : "";
      const firstSeparator = cache.indexOf(",");
      const secondSeparator = firstSeparator < 0 ? -1 : cache.indexOf(",", firstSeparator + 1);
      const fields = firstSeparator > 0 && secondSeparator > firstSeparator + 1
        ? [cache.slice(0, firstSeparator), cache.slice(firstSeparator + 1, secondSeparator),
          cache.slice(secondSeparator + 1)] : [];
      accepted = fields.length === 3 && /^(?:100644|100755)$/u.test(fields[0])
        && OID.test(fields[1]) && safeRepositoryPath(fields[2]);
      break;
    }
    case "write-tree":
      accepted = rest.length === 0;
      break;
    case "diff":
      accepted = rest.length === 5 && sameArgs(rest.slice(0, 3), ["--name-status", "-z", "--no-renames"])
          && OID.test(rest[3]) && rest[4] === "--"
        || (rest.length >= 5 && sameArgs(rest.slice(0, 3), ["--binary", "--no-ext-diff", "--no-textconv"])
          && OID.test(rest[3]) && rest[4] === "--" && rest.slice(5).every(safeRepositoryPath))
        || (rest.length === 6 && sameArgs(rest.slice(0, 3), ["--binary", "--no-ext-diff", "--no-textconv"])
          && OID.test(rest[3]) && OID.test(rest[4]) && rest[5] === "--")
        || (rest.length === 4 && rest[0] === "--check" && OID.test(rest[1])
          && OID.test(rest[2]) && rest[3] === "--");
      break;
    case "commit-tree":
      accepted = rest.length === 3 && OID.test(rest[0]) && rest[1] === "-p" && OID.test(rest[2]);
      break;
    case "cat-file":
      accepted = rest.length === 2 && rest[0] === "-t" && OID.test(rest[1]);
      break;
    default:
      accepted = false;
  }
  ensure(accepted, code);
  const takesStdin = command === "hash-object" || command === "commit-tree";
  ensure(takesStdin ? stdin !== null : stdin === null, code);
}

function validateGitArgs(args, environmentFacts, stdin, code) {
  ensure(Array.isArray(args) && args.length >= 12 && args.length <= 256, code);
  let total = 0;
  for (const value of args) {
    ensure(typeof value === "string" && value.length > 0 && value.length <= 8_192
      && !/[\0\r\n]/u.test(value), code);
    total += value.length;
  }
  ensure(total <= 65_536
    && args[0] === "--no-pager" && args[1] === "--no-replace-objects"
    && args[2] === "--literal-pathspecs" && args[3] === "-c"
    && args[4] === `core.hooksPath=${environmentFacts.controlRoot}/empty-hooks` && args[5] === "-c"
    && args[6] === "core.fsmonitor=false" && args[7] === "-c"
    && args[8] === "protocol.allow=never" && args[9] === "-c"
    && args[10] === "commit.gpgSign=false", code);
  validateGitOperation(args, environmentFacts.phase, stdin, code);
}

function validateGitEnvironment(environment, request, code) {
  ensure(object(environment), code);
  const actualKeys = Object.keys(environment).sort();
  const bootstrap = [...GIT_BOOTSTRAP_PATH_KEYS, ...Object.keys(GIT_LITERAL_ENVIRONMENT)].sort();
  const guarded = [...GIT_PATH_KEYS, ...Object.keys(GIT_LITERAL_ENVIRONMENT)].sort();
  const guardedWithIdentity = [...guarded, ...GIT_IDENTITY_KEYS].sort();
  const phase = actualKeys.length === bootstrap.length
      && actualKeys.every((key, index) => key === bootstrap[index]) ? "BOOTSTRAP"
    : (actualKeys.length === guarded.length
      && actualKeys.every((key, index) => key === guarded[index]))
      || (actualKeys.length === guardedWithIdentity.length
        && actualKeys.every((key, index) => key === guardedWithIdentity[index])) ? "GUARDED" : null;
  ensure(phase !== null, code);
  const executionEnvironment = request.environment;
  for (const key of phase === "BOOTSTRAP" ? GIT_BOOTSTRAP_PATH_KEYS : GIT_PATH_KEYS) {
    ensure(canonicalLinuxHomePath(environment[key]), code);
  }
  const controlRoot = path.posix.dirname(environment.HOME);
  const expectedControl = deriveWsl2GitControlRootIdentity({
    task_ref: request.task_ref,
    attempt: request.attempt,
    worktree_ref: request.worktree_ref,
    execution_environment_ref: executionEnvironment.identity_digest,
    preflight_receipt_ref: request.preflight_receipt.receipt_digest,
    repository_head: executionEnvironment.linux.repository_head,
    isolation_root: executionEnvironment.verification_toolchain.isolation_root,
  });
  ensure(path.posix.basename(environment.HOME) === "git-home"
    && environment.TMPDIR === `${controlRoot}/git-temp`
    && environment.GIT_CONFIG_GLOBAL === `${controlRoot}/empty-global.gitconfig`
    && controlRoot === expectedControl.locator, code);
  if (phase === "GUARDED") {
    const commonDirectory = request.preflight_receipt?.probes?.technical?.git?.common_dir;
    const gitDirectory = request.preflight_receipt?.probes?.technical?.git?.git_dir;
    const command = request.args[11];
    const writesCandidateIndex = ["read-tree", "update-index", "write-tree"].includes(command);
    const expectedIndex = writesCandidateIndex
      ? `${controlRoot}/candidate-index`
      : `${gitDirectory}/index`;
    ensure(canonicalLinuxHomePath(commonDirectory)
      && canonicalLinuxHomePath(gitDirectory)
      && commonDirectory.startsWith(`${executionEnvironment.verification_toolchain.task_root}/`)
      && gitDirectory.startsWith(`${commonDirectory}/worktrees/`)
      && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/u.test(
        gitDirectory.slice(`${commonDirectory}/worktrees/`.length),
      )
      && environment.GIT_WORK_TREE === executionEnvironment.linux.cwd
      && environment.GIT_COMMON_DIR === commonDirectory
      && environment.GIT_OBJECT_DIRECTORY === `${commonDirectory}/objects`
      && environment.GIT_DIR === gitDirectory
      && environment.GIT_INDEX_FILE === expectedIndex, code);
  }
  for (const [key, expected] of Object.entries(GIT_LITERAL_ENVIRONMENT)) {
    ensure(environment[key] === expected, code);
  }
  for (const key of GIT_IDENTITY_KEYS) {
    if (environment[key] !== undefined) {
      ensure(typeof environment[key] === "string" && environment[key].length > 0
        && environment[key].length <= 8_192 && !/[\0\r\n]/u.test(environment[key]), code);
    }
  }
  return Object.freeze({ phase, controlRoot });
}

function validateGitInvocation(invocation, request, code) {
  exactKeys(invocation, [
    "schema", "sequence", "environment", "args", "stdin", "invocation_digest", "process_fence",
  ], code);
  ensure(invocation.schema === "lattice.wsl2-git-invocation/1.0"
    && Number.isSafeInteger(invocation.sequence) && invocation.sequence >= 1 && invocation.sequence <= 10_000
    && canonicalJson(invocation.args) === canonicalJson(request.args), code);
  const environmentFacts = validateGitEnvironment(invocation.environment, request, code);
  let stdin = null;
  if (invocation.stdin !== null) {
    exactKeys(invocation.stdin, ["byte_len", "sha256", "base64"], code);
    stdin = decodeCanonicalBase64(invocation.stdin.base64, code);
    ensure(Number.isSafeInteger(invocation.stdin.byte_len) && invocation.stdin.byte_len >= 0
      && invocation.stdin.byte_len <= MAX_GIT_STDIN_BYTES && stdin.length === invocation.stdin.byte_len
      && HEX_64.test(invocation.stdin.sha256) && sha256(stdin) === invocation.stdin.sha256, code);
  }
  validateGitArgs(invocation.args, environmentFacts, invocation.stdin, code);
  const identityPresent = GIT_IDENTITY_KEYS.every((key) => invocation.environment[key] !== undefined);
  const command = invocation.args[11];
  ensure(command === "commit-tree" ? identityPresent : !identityPresent, code);
  if (identityPresent) {
    ensure(invocation.environment.GIT_AUTHOR_NAME === "LATTICE Foreman"
      && invocation.environment.GIT_AUTHOR_EMAIL === "lattice@invalid.local"
      && invocation.environment.GIT_COMMITTER_NAME === "LATTICE Foreman"
      && invocation.environment.GIT_COMMITTER_EMAIL === "lattice@invalid.local"
      && invocation.environment.GIT_AUTHOR_DATE === invocation.environment.GIT_COMMITTER_DATE
      && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/u.test(invocation.environment.GIT_AUTHOR_DATE)
      && stdin !== null && stdin.equals(Buffer.from("LATTICE managed verification candidate\n", "utf8")), code);
  }
  const subject = {
    schema: invocation.schema,
    sequence: invocation.sequence,
    environment: invocation.environment,
    args: invocation.args,
    stdin: invocation.stdin,
  };
  const invocationDigest = digest("wsl2-git-invocation", subject);
  const processFence = sha256(Buffer.from(
    `${request.preflight_receipt.process_fence.fence}\n${invocationDigest}\n${invocation.sequence}`,
    "utf8",
  ));
  ensure(invocation.invocation_digest === invocationDigest && invocation.process_fence === processFence, code);
  return stdin ?? Buffer.alloc(0);
}

function verifierFence(request) {
  return sha256(Buffer.from(canonicalJson({
    schema: "lattice.wsl2-verifier-fence/1.0",
    task_ref: request.task_ref,
    worktree_ref: request.worktree_ref,
    execution_environment_ref: request.environment.identity_digest,
    preflight_receipt_ref: request.preflight_receipt.receipt_digest,
    preflight_fence: request.preflight_receipt.process_fence.fence,
    role: request.role,
    args: request.args,
    attempt: request.attempt,
    retry_of: request.preflight_receipt.continuation.retry_of,
    reconnect_of: request.preflight_receipt.continuation.reconnect_of,
  }), "utf8"));
}

export function validateWsl2VerifierBridgeRequest(untrusted) {
  const code = "WSL2_VERIFIER_BRIDGE_REQUEST_REJECTED";
  ensure(object(untrusted) && untrusted.schema === REQUEST_SCHEMA
    && ["NODE", "CARGO", "GIT"].includes(untrusted.role), code);
  exactKeys(untrusted, [
    "schema", "environment", "preflight_receipt", "task_ref", "attempt", "worktree_ref",
    "role", "args", ...(untrusted.role === "GIT" ? ["git_invocation"] : []),
  ], code);
  const environment = validateWsl2ExecutionEnvironment(untrusted.environment);
  ensure(typeof untrusted.task_ref === "string" && HEX_64.test(untrusted.task_ref), code);
  ensure(Number.isSafeInteger(untrusted.attempt) && untrusted.attempt >= 1
    && untrusted.attempt <= MAX_WSL2_ATTEMPTS, code);
  ensure(typeof untrusted.worktree_ref === "string"
    && /^worktree:sha256:[a-f0-9]{64}$/u.test(untrusted.worktree_ref), code);
  ensure(Array.isArray(untrusted.args) && (untrusted.role === "GIT"
    ? true : canonicalJson(untrusted.args) === canonicalJson(CLOSED_ARGS[untrusted.role])), code);
  const receipt = untrusted.preflight_receipt;
  ensure(object(receipt)
    && receipt.schema === "lattice.wsl2-zero-model-preflight/1.0"
    && receipt.status === "PASS"
    && receipt.task_ref === untrusted.task_ref
    && receipt.attempt === untrusted.attempt
    && receipt.worktree_ref === untrusted.worktree_ref
    && receipt.execution_environment_ref === environment.identity_digest
    && receipt.linux_cwd === environment.linux.cwd
    && receipt.repository_head === environment.linux.repository_head
    && receipt.provider_effect_count === 0
    && object(receipt.process_fence)
    && typeof receipt.process_fence.fence === "string"
    && HEX_64.test(receipt.process_fence.fence)
    && object(receipt.continuation)
    && receipt.continuation.attempt === untrusted.attempt
    && (receipt.continuation.retry_of === null
      || /^verifier-receipt:sha256:[a-f0-9]{64}$/u.test(receipt.continuation.retry_of))
    && (receipt.continuation.reconnect_of === null
      || /^verifier-receipt:sha256:[a-f0-9]{64}$/u.test(receipt.continuation.reconnect_of))
    && (receipt.continuation.retry_of === null
      || receipt.continuation.reconnect_of === null)
    && typeof receipt.receipt_digest === "string"
    && /^wsl2-preflight:sha256:[a-f0-9]{64}$/u.test(receipt.receipt_digest), code);
  ensure(environment.verification_toolchain.task_ref === untrusted.task_ref
    && HEX_40.test(environment.linux.repository_head), code);
  const normalized = { ...structuredClone(untrusted), environment };
  if (untrusted.role === "GIT") normalized.gitStdin = validateGitInvocation(untrusted.git_invocation, normalized, code);
  return Object.freeze(normalized);
}

function framed(stderr, schema, code) {
  const records = stderr.replaceAll("\r", "").split("\n").filter(Boolean).flatMap((line) => {
    try {
      const value = JSON.parse(line);
      return value?.schema === schema ? [value] : [];
    } catch {
      return [];
    }
  });
  ensure(records.length === 1, code);
  return records[0];
}

function windowsHostEnvironment() {
  return Object.fromEntries(["SystemRoot", "WINDIR"].flatMap((key) => (
    process.env[key] === undefined ? [] : [[key, process.env[key]]]
  )));
}

function expectedServiceUnit(request, processFence) {
  return `${request.environment.process_fence.unit_prefix}-${request.role.toLowerCase()}-${processFence.slice(0, 12)}.service`;
}

function expectedCgroupPath(request, launch) {
  const uid = request.environment.verification_toolchain.owner_uid;
  return `/user.slice/user-${uid}.slice/user@${uid}.service/app.slice/${launch.serviceUnit}`;
}

function validateLaunchAuthority(request, launch, processFence) {
  const code = "WSL2_VERIFIER_BRIDGE_FENCE_REJECTED";
  ensure(object(launch) && launch.command === request.environment.gateway.windows_path
    && Array.isArray(launch.args) && launch.processFence === processFence
    && launch.serviceUnit === expectedServiceUnit(request, processFence), code);
  exactKeys(launch.postExitProbe, [
    "distribution", "unit", "process_fence", "authority_ref", "systemctl_path", "cgroup_mount",
  ], code);
  ensure(launch.postExitProbe.distribution === request.environment.distribution
    && launch.postExitProbe.unit === launch.serviceUnit
    && launch.postExitProbe.process_fence === processFence
    && launch.postExitProbe.authority_ref === request.environment.process_fence.identity_digest
    && launch.postExitProbe.systemctl_path === request.environment.process_fence.systemctl_path
    && launch.postExitProbe.cgroup_mount === request.environment.process_fence.cgroup_mount, code);
}

async function defaultRunUnitCleanup(request, launch, reason, execFile = execFileDefault) {
  ensure(CLEANUP_REASONS.includes(reason), "WSL2_VERIFIER_BRIDGE_CLEANUP_REJECTED");
  const environment = request.environment;
  const prefix = [
    "-d", environment.distribution, "--exec", "/usr/bin/env", "-i",
    `XDG_RUNTIME_DIR=${environment.process_fence.user_runtime_dir}`,
    "LANG=C.UTF-8", "LC_ALL=C.UTF-8", environment.process_fence.systemctl_path,
  ];
  const options = {
    encoding: "buffer",
    windowsHide: true,
    timeout: OUTER_CLEANUP_TIMEOUT_MS,
    maxBuffer: OUTER_CLEANUP_OUTPUT_LIMIT_BYTES,
    env: windowsHostEnvironment(),
  };
  const attempts = [];
  const runSystemctl = async (action, args) => {
    let value;
    let error = null;
    try {
      value = await execFile(
        environment.gateway.windows_path, [...prefix, "--user", ...args], options,
      );
    } catch (caught) {
      error = caught;
    }
    const rawStdout = Buffer.isBuffer((error ?? value)?.stdout)
      ? (error ?? value).stdout : Buffer.alloc(0);
    const rawStderr = Buffer.isBuffer((error ?? value)?.stderr)
      ? (error ?? value).stderr : Buffer.alloc(0);
    const stdout = rawStdout.subarray(0, OUTER_CLEANUP_OUTPUT_LIMIT_BYTES);
    const stderr = rawStderr.subarray(0, OUTER_CLEANUP_OUTPUT_LIMIT_BYTES);
    const outputBoundExceeded = rawStdout.length > OUTER_CLEANUP_OUTPUT_LIMIT_BYTES
      || rawStderr.length > OUTER_CLEANUP_OUTPUT_LIMIT_BYTES
      || error?.code === "ERR_CHILD_PROCESS_STDIO_MAXBUFFER";
    const timedOut = error?.killed === true && !outputBoundExceeded;
    const exitCode = error === null ? 0
      : Number.isSafeInteger(error?.code) ? error.code : null;
    const signal = typeof error?.signal === "string" && /^[A-Z0-9]{1,32}$/u.test(error.signal)
      ? error.signal : null;
    const result = error === null ? "SUCCESS"
      : outputBoundExceeded ? "OUTPUT_BOUND_EXCEEDED"
        : timedOut ? "TIMED_OUT"
          : exitCode !== null ? "EXIT_NONZERO" : "TRANSPORT_ERROR";
    const attempt = {
      sequence: attempts.length + 1,
      action,
      result,
      exit_code: exitCode,
      signal,
      timed_out: timedOut,
      output_bound_exceeded: outputBoundExceeded,
      stdout_captured_bytes: stdout.length,
      stderr_captured_bytes: stderr.length,
      stdout_sha256: sha256(stdout),
      stderr_sha256: sha256(stderr),
    };
    attempts.push(attempt);
    return attempt;
  };
  const term = await runSystemctl("TERM_KILL", [
    "kill", "--kill-whom=all", "--signal=SIGTERM", launch.serviceUnit,
  ]);
  const stop = await runSystemctl("STOP", ["stop", launch.serviceUnit]);
  if (term.result !== "SUCCESS" || stop.result !== "SUCCESS") {
    await runSystemctl("KILL", [
      "kill", "--kill-whom=all", "--signal=SIGKILL", launch.serviceUnit,
    ]);
    await runSystemctl("FORCE_STOP", ["stop", launch.serviceUnit]);
  }
  const evidence = {
    schema: "lattice.wsl2-verifier-cleanup/1.0",
    reason,
    unit: launch.serviceUnit,
    process_fence: launch.processFence,
    systemctl_identity: {
      path: environment.process_fence.systemctl_path,
      version: environment.process_fence.systemctl_version,
      sha256: environment.process_fence.systemctl_sha256,
    },
    attempt: request.attempt,
    retry_of: request.preflight_receipt.continuation.retry_of,
    reconnect_of: request.preflight_receipt.continuation.reconnect_of,
    attempts,
    cleanup_digest: null,
  };
  evidence.cleanup_digest = digest("wsl2-verifier-cleanup", Object.fromEntries(
    Object.entries(evidence).filter(([key]) => key !== "cleanup_digest"),
  ));
  return evidence;
}

function validateCleanupEvidence(evidence, request, launch, reason) {
  const code = "WSL2_VERIFIER_BRIDGE_CLEANUP_REJECTED";
  ensure(CLEANUP_REASONS.includes(reason), code);
  exactKeys(evidence, [
    "schema", "reason", "unit", "process_fence", "systemctl_identity", "attempt",
    "retry_of", "reconnect_of", "attempts", "cleanup_digest",
  ], code);
  exactKeys(evidence.systemctl_identity, ["path", "version", "sha256"], code);
  ensure(evidence.schema === "lattice.wsl2-verifier-cleanup/1.0"
    && evidence.reason === reason && evidence.unit === launch.serviceUnit
    && evidence.process_fence === launch.processFence
    && evidence.systemctl_identity.path === request.environment.process_fence.systemctl_path
    && evidence.systemctl_identity.version === request.environment.process_fence.systemctl_version
    && evidence.systemctl_identity.sha256 === request.environment.process_fence.systemctl_sha256
    && evidence.attempt === request.attempt
    && evidence.retry_of === request.preflight_receipt.continuation.retry_of
    && evidence.reconnect_of === request.preflight_receipt.continuation.reconnect_of
    && Array.isArray(evidence.attempts)
    && (evidence.attempts.length === 2 || evidence.attempts.length === 4), code);
  const expectedActions = evidence.attempts.length === 2
    ? ["TERM_KILL", "STOP"] : ["TERM_KILL", "STOP", "KILL", "FORCE_STOP"];
  for (const [index, attempt] of evidence.attempts.entries()) {
    exactKeys(attempt, [
      "sequence", "action", "result", "exit_code", "signal", "timed_out",
      "output_bound_exceeded", "stdout_captured_bytes", "stderr_captured_bytes",
      "stdout_sha256", "stderr_sha256",
    ], code);
    ensure(attempt.sequence === index + 1 && attempt.action === expectedActions[index]
      && CLEANUP_RESULTS.includes(attempt.result)
      && (attempt.exit_code === null
        || (Number.isSafeInteger(attempt.exit_code) && attempt.exit_code >= 0))
      && (attempt.signal === null
        || (typeof attempt.signal === "string" && /^[A-Z0-9]{1,32}$/u.test(attempt.signal)))
      && typeof attempt.timed_out === "boolean"
      && typeof attempt.output_bound_exceeded === "boolean"
      && Number.isSafeInteger(attempt.stdout_captured_bytes)
      && attempt.stdout_captured_bytes >= 0
      && attempt.stdout_captured_bytes <= OUTER_CLEANUP_OUTPUT_LIMIT_BYTES
      && Number.isSafeInteger(attempt.stderr_captured_bytes)
      && attempt.stderr_captured_bytes >= 0
      && attempt.stderr_captured_bytes <= OUTER_CLEANUP_OUTPUT_LIMIT_BYTES
      && HEX_64.test(attempt.stdout_sha256) && HEX_64.test(attempt.stderr_sha256), code);
    const resultMatches = attempt.result === "SUCCESS"
      ? attempt.exit_code === 0 && attempt.signal === null
        && !attempt.timed_out && !attempt.output_bound_exceeded
      : attempt.result === "EXIT_NONZERO"
        ? Number.isSafeInteger(attempt.exit_code) && attempt.exit_code !== 0
          && attempt.signal === null && !attempt.timed_out && !attempt.output_bound_exceeded
        : attempt.result === "TIMED_OUT"
          ? attempt.timed_out && !attempt.output_bound_exceeded
          : attempt.result === "OUTPUT_BOUND_EXCEEDED"
            ? attempt.output_bound_exceeded
            : attempt.exit_code === null && !attempt.timed_out && !attempt.output_bound_exceeded;
    ensure(resultMatches, code);
  }
  ensure((evidence.attempts.length === 4)
    === (evidence.attempts[0].result !== "SUCCESS"
      || evidence.attempts[1].result !== "SUCCESS"), code);
  const subject = Object.fromEntries(
    Object.entries(evidence).filter(([key]) => key !== "cleanup_digest"),
  );
  ensure(evidence.cleanup_digest === digest("wsl2-verifier-cleanup", subject), code);
  return evidence;
}

function normalizeTransportError(error, source) {
  const errorName = typeof error?.name === "string" && /^[A-Za-z][A-Za-z0-9_.-]{0,127}$/u.test(error.name)
    ? error.name : "Error";
  const rawCode = typeof error?.code === "string" || Number.isSafeInteger(error?.code)
    ? String(error.code) : null;
  const errorCode = rawCode !== null && /^[A-Za-z0-9_.-]{1,128}$/u.test(rawCode) ? rawCode : null;
  const message = typeof error?.message === "string" ? error.message : String(error ?? "");
  const errorType = { source, error_name: errorName, error_code: errorCode };
  return Object.freeze({
    ...errorType,
    message_sha256: sha256(Buffer.from(message, "utf8")),
    error_type_digest: digest("wsl2-verifier-transport-error", errorType),
  });
}

function preSpawnTransportObservation(error) {
  return {
    code: null,
    signal: null,
    watchdog: null,
    stdout: Buffer.alloc(0),
    stderr: Buffer.alloc(0),
    spawn_observed: false,
    close_observed: false,
    stdout_seen_bytes: 0,
    stderr_seen_bytes: 0,
    stdout_bound_exceeded: false,
    stderr_bound_exceeded: false,
    transport_error: normalizeTransportError(error, "SPAWN"),
  };
}

async function defaultRunSpawn(launch, timeoutMs, stdin, onWatchdog, spawnProcess = spawn) {
  let child;
  try {
    child = spawnProcess(launch.command, launch.args, {
      windowsHide: true,
      stdio: ["pipe", "pipe", "pipe"],
      env: windowsHostEnvironment(),
    });
  } catch (error) {
    try { await onWatchdog("TRANSPORT_ERROR"); } catch { /* validated by the caller */ }
    return preSpawnTransportObservation(error);
  }
  return await new Promise((resolve) => {
    const stdout = [];
    const stderr = [];
    let stdoutBytes = 0;
    let stderrBytes = 0;
    let stdoutCapturedBytes = 0;
    let stderrCapturedBytes = 0;
    let settled = false;
    let spawned = false;
    let closed = false;
    let closeCode = null;
    let closeSignal = null;
    let watchdog = null;
    let transportError = null;
    let cleanupReason = null;
    let cleanupPromise = null;
    let wrapperKillTimer = null;
    let transportSettleTimer = null;
    const snapshot = () => ({
      code: closeCode,
      signal: closeSignal,
      watchdog,
      stdout: Buffer.concat(stdout),
      stderr: Buffer.concat(stderr),
      ...(transportError === null ? {} : {
        spawn_observed: spawned,
        close_observed: closed,
        stdout_seen_bytes: stdoutBytes,
        stderr_seen_bytes: stderrBytes,
        stdout_bound_exceeded: stdoutBytes > OUTER_PROCESS_LIMIT_BYTES,
        stderr_bound_exceeded: stderrBytes > OUTER_PROCESS_LIMIT_BYTES,
        transport_error: transportError,
      }),
    });
    const finish = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      clearTimeout(wrapperKillTimer);
      clearTimeout(transportSettleTimer);
      process.off("SIGINT", onInterrupt);
      process.off("SIGTERM", onInterrupt);
      resolve(snapshot());
    };
    const beginCleanup = (reason) => {
      if (cleanupReason !== null) return cleanupPromise;
      cleanupReason = reason;
      cleanupPromise = Promise.resolve().then(() => onWatchdog(reason));
      return cleanupPromise;
    };
    const forceWrapperClosed = () => {
      if (closed) return finish();
      try { child.kill("SIGKILL"); } catch { /* outer unit proof remains authoritative */ }
      transportSettleTimer = setTimeout(finish, 5_000);
    };
    const beginTransportError = (source, error) => {
      if (transportError !== null || watchdog !== null) return cleanupPromise;
      clearTimeout(timer);
      transportError = normalizeTransportError(error, source);
      try { child.stdin?.destroy(); } catch { /* transport failure already captured */ }
      const cleanup = beginCleanup("TRANSPORT_ERROR");
      Promise.resolve(cleanup).then(forceWrapperClosed, forceWrapperClosed);
      return cleanup;
    };
    const beginWatchdog = (reason) => {
      if (transportError !== null) return cleanupPromise;
      if (watchdog !== null) return cleanupPromise;
      watchdog = reason;
      try { child.stdin?.destroy(); } catch { /* cleanup is authoritative */ }
      const cleanup = beginCleanup(reason);
      Promise.resolve(cleanup).then(() => {
        if (!closed) wrapperKillTimer = setTimeout(() => {
          try { child.kill("SIGKILL"); } catch { /* close/cleanup path validates later */ }
        }, 5_000);
      }, () => {
        try { child.kill("SIGKILL"); } catch { /* caller validates cleanup rejection */ }
      });
      return cleanup;
    };
    const appendBounded = (chunks, chunk, capturedBytes) => {
      const remaining = Math.max(0, OUTER_PROCESS_LIMIT_BYTES - capturedBytes);
      const captured = Math.min(remaining, chunk.length);
      if (captured > 0) chunks.push(chunk.subarray(0, captured));
      return capturedBytes + captured;
    };
    const onInterrupt = () => { void beginWatchdog("INTERRUPTED"); };
    const timer = setTimeout(() => {
      void beginWatchdog("TIMED_OUT");
    }, timeoutMs + 30_000);
    process.once("SIGINT", onInterrupt);
    process.once("SIGTERM", onInterrupt);
    if (typeof child?.once !== "function" || child.stdin === null || child.stdin === undefined
      || child.stdout === null || child.stdout === undefined
      || child.stderr === null || child.stderr === undefined) {
      queueMicrotask(() => beginTransportError(
        "CHILD", Object.assign(new Error("CHILD_STDIO_UNAVAILABLE"), { code: "CHILD_STDIO_UNAVAILABLE" }),
      ));
      return;
    }
    child.once("spawn", () => { spawned = true; });
    child.stdout.on("data", (chunk) => {
      const bytes = Buffer.from(chunk);
      stdoutCapturedBytes = appendBounded(stdout, bytes, stdoutCapturedBytes);
      stdoutBytes = Math.min(OUTER_PROCESS_LIMIT_BYTES + 1, stdoutBytes + bytes.length);
      if (stdoutBytes > OUTER_PROCESS_LIMIT_BYTES) void beginWatchdog("OUTPUT_BOUND_EXCEEDED");
    });
    child.stderr.on("data", (chunk) => {
      const bytes = Buffer.from(chunk);
      stderrCapturedBytes = appendBounded(stderr, bytes, stderrCapturedBytes);
      stderrBytes = Math.min(OUTER_PROCESS_LIMIT_BYTES + 1, stderrBytes + bytes.length);
      if (stderrBytes > OUTER_PROCESS_LIMIT_BYTES) void beginWatchdog("OUTPUT_BOUND_EXCEEDED");
    });
    child.stdout.on("error", (error) => { void beginTransportError("STDOUT", error); });
    child.stderr.on("error", (error) => { void beginTransportError("STDERR", error); });
    child.stdin.on("error", (error) => {
      if (error?.code !== "EPIPE") void beginTransportError("STDIN", error);
    });
    child.once("error", (error) => { void beginTransportError(spawned ? "CHILD" : "SPAWN", error); });
    child.once("close", (code, signal) => {
      closed = true;
      closeCode = Number.isSafeInteger(code) ? code : null;
      closeSignal = typeof signal === "string" ? signal : null;
      Promise.resolve(cleanupPromise).then(finish, finish);
    });
    try {
      child.stdin.end(stdin);
    } catch (error) {
      void beginTransportError("STDIN", error);
    }
  });
}

async function defaultRunLaunch(launch, timeoutMs, _stdin, onWatchdog, spawnProcess) {
  return await defaultRunSpawn(launch, timeoutMs, Buffer.alloc(0), onWatchdog, spawnProcess);
}

async function defaultRunGitLaunch(launch, timeoutMs, stdin, onWatchdog, spawnProcess) {
  return await defaultRunSpawn(launch, timeoutMs, stdin, onWatchdog, spawnProcess);
}

function onlyJsonLine(stdout, code) {
  ensure(typeof stdout === "string", code);
  const lines = stdout.replaceAll("\r", "").split("\n").filter(Boolean);
  ensure(lines.length === 1, code);
  try { return JSON.parse(lines[0]); } catch { throw rejected(code); }
}

async function defaultRunOuterProbe(request, launch, marker) {
  const environment = request.environment;
  const config = {
    systemctl: environment.process_fence.systemctl_path,
    unit: launch.serviceUnit,
    runtime_dir: environment.process_fence.user_runtime_dir,
    cgroup_mount: environment.process_fence.cgroup_mount,
    cgroup_path: marker.cgroup_path,
  };
  const value = await execFileDefault(environment.gateway.windows_path, [
    "-d", environment.distribution, "--exec", "/usr/bin/env", "-i",
    `XDG_RUNTIME_DIR=${environment.process_fence.user_runtime_dir}`,
    "LANG=C.UTF-8", "LC_ALL=C.UTF-8", environment.process_fence.supervisor_bootstrap_node.path,
    "-e", WSL2_CGROUP_EXIT_PROBE_SOURCE, canonicalJson(config),
  ], {
    encoding: "utf8", windowsHide: true, timeout: 30_000, maxBuffer: 65_536,
    env: Object.fromEntries(["SystemRoot", "WINDIR"].flatMap((key) => (
      process.env[key] === undefined ? [] : [[key, process.env[key]]]
    ))),
  });
  return onlyJsonLine(value.stdout, "WSL2_VERIFIER_BRIDGE_OUTER_EXIT_REJECTED");
}

function verifierOutcome(exit) {
  const terminal = (Number.isSafeInteger(exit.exit_code) && exit.exit_signal === null)
    || (exit.exit_code === null && typeof exit.exit_signal === "string");
  ensure(terminal, "WSL2_VERIFIER_BRIDGE_FENCE_REJECTED");
  if (exit.interrupted) return "INTERRUPTED";
  if (exit.timed_out) return "TIMED_OUT";
  if (exit.output_bound_exceeded) return "OUTPUT_BOUND_EXCEEDED";
  return exit.exit_code === 0 && exit.exit_signal === null ? "PASS" : "FAILED";
}

function validateObservedProcess(observed, exit, outcome) {
  const code = "WSL2_VERIFIER_BRIDGE_FENCE_REJECTED";
  ensure(object(observed)
    && (Number.isSafeInteger(observed.code) || observed.code === null)
    && (observed.signal === null
      || (typeof observed.signal === "string" && /^[A-Z0-9]{1,32}$/u.test(observed.signal)))
    && (observed.watchdog === undefined || observed.watchdog === null
      || OUTER_WATCHDOGS.includes(observed.watchdog)), code);
  const outerSucceeded = observed.code === 0 && observed.signal === null;
  if (observed.watchdog === undefined || observed.watchdog === null) {
    ensure((outcome === "PASS") === outerSucceeded, code);
  }
  if (observed.watchdog === "INTERRUPTED") ensure(exit.interrupted === true, code);
  if (observed.watchdog === "TIMED_OUT") ensure(exit.timed_out === true, code);
  if (observed.watchdog === "OUTPUT_BOUND_EXCEEDED") {
    ensure(exit.output_bound_exceeded === true, code);
  }
}

function validateOuterPostExit(facts, request, launch, marker, outcome, watchdog) {
  const code = "WSL2_VERIFIER_BRIDGE_OUTER_EXIT_REJECTED";
  exactKeys(facts, [
    "unit", "active_state", "sub_state", "result", "cgroup_path", "delegate",
    "cgroup_exists", "populated",
  ], code);
  const expectedResults = watchdog === null
    ? [outcome === "PASS" ? "success" : "exit-code"]
    : ["success", "exit-code", "signal"];
  ensure(facts.unit === launch.serviceUnit && facts.active_state === "inactive"
    && facts.sub_state === "dead"
    && expectedResults.includes(facts.result)
    && facts.cgroup_path === expectedCgroupPath(request, launch)
    && facts.cgroup_path === marker.cgroup_path && facts.delegate === "no"
    && ((facts.cgroup_exists === false && facts.populated === null)
      || (facts.cgroup_exists === true && facts.populated === 0)), code);
  return facts;
}

function validateTransportObservation(observed) {
  const code = "WSL2_VERIFIER_BRIDGE_TRANSPORT_REJECTED";
  exactKeys(observed, [
    "code", "signal", "watchdog", "stdout", "stderr", "spawn_observed", "close_observed",
    "stdout_seen_bytes", "stderr_seen_bytes", "stdout_bound_exceeded",
    "stderr_bound_exceeded", "transport_error",
  ], code);
  exactKeys(observed.transport_error, [
    "source", "error_name", "error_code", "message_sha256", "error_type_digest",
  ], code);
  ensure(observed.watchdog === null && Buffer.isBuffer(observed.stdout)
    && Buffer.isBuffer(observed.stderr)
    && observed.stdout.length <= OUTER_PROCESS_LIMIT_BYTES
    && observed.stderr.length <= OUTER_PROCESS_LIMIT_BYTES
    && typeof observed.spawn_observed === "boolean"
    && typeof observed.close_observed === "boolean"
    && (observed.close_observed || (observed.code === null && observed.signal === null))
    && (Number.isSafeInteger(observed.code) || observed.code === null)
    && (observed.signal === null
      || (typeof observed.signal === "string" && /^[A-Z0-9]{1,32}$/u.test(observed.signal)))
    && Number.isSafeInteger(observed.stdout_seen_bytes)
    && Number.isSafeInteger(observed.stderr_seen_bytes)
    && typeof observed.stdout_bound_exceeded === "boolean"
    && typeof observed.stderr_bound_exceeded === "boolean", code);
  for (const [seenBytes, boundExceeded, capturedBytes] of [
    [observed.stdout_seen_bytes, observed.stdout_bound_exceeded, observed.stdout.length],
    [observed.stderr_seen_bytes, observed.stderr_bound_exceeded, observed.stderr.length],
  ]) {
    ensure(boundExceeded
      ? seenBytes === OUTER_PROCESS_LIMIT_BYTES + 1 && capturedBytes === OUTER_PROCESS_LIMIT_BYTES
      : seenBytes === capturedBytes && capturedBytes <= OUTER_PROCESS_LIMIT_BYTES, code);
  }
  const error = observed.transport_error;
  ensure(["SPAWN", "STDIN", "STDOUT", "STDERR", "CHILD"].includes(error.source)
    && typeof error.error_name === "string"
    && /^[A-Za-z][A-Za-z0-9_.-]{0,127}$/u.test(error.error_name)
    && (error.error_code === null
      || (typeof error.error_code === "string" && /^[A-Za-z0-9_.-]{1,128}$/u.test(error.error_code)))
    && HEX_64.test(error.message_sha256), code);
  const errorType = {
    source: error.source,
    error_name: error.error_name,
    error_code: error.error_code,
  };
  ensure(error.error_type_digest === digest("wsl2-verifier-transport-error", errorType), code);
  const evidence = {
    schema: "lattice.wsl2-verifier-transport-evidence/1.0",
    error,
    process: {
      spawn_observed: observed.spawn_observed,
      close_observed: observed.close_observed,
      exit_code: Number.isSafeInteger(observed.code) && observed.code >= 0 && observed.code <= 255
        ? observed.code : null,
      signal: observed.signal,
    },
    output: {
      stdout_captured_bytes: observed.stdout.length,
      stderr_captured_bytes: observed.stderr.length,
      stdout_seen_bytes: observed.stdout_seen_bytes,
      stderr_seen_bytes: observed.stderr_seen_bytes,
      stdout_bound_exceeded: observed.stdout_bound_exceeded,
      stderr_bound_exceeded: observed.stderr_bound_exceeded,
      stdout_sha256: sha256(observed.stdout),
      stderr_sha256: sha256(observed.stderr),
    },
    evidence_digest: null,
  };
  evidence.evidence_digest = digest("wsl2-verifier-transport-evidence", Object.fromEntries(
    Object.entries(evidence).filter(([key]) => key !== "evidence_digest"),
  ));
  return evidence;
}

function validateTransportOuterPostExit(facts, request, launch) {
  const code = "WSL2_VERIFIER_BRIDGE_OUTER_EXIT_REJECTED";
  exactKeys(facts, [
    "unit", "active_state", "sub_state", "result", "cgroup_path", "delegate",
    "cgroup_exists", "populated",
  ], code);
  ensure(facts.unit === launch.serviceUnit && facts.active_state === "inactive"
    && facts.sub_state === "dead" && ["success", "exit-code", "signal"].includes(facts.result)
    && facts.cgroup_path === expectedCgroupPath(request, launch) && facts.delegate === "no"
    && ((facts.cgroup_exists === false && facts.populated === null)
      || (facts.cgroup_exists === true && facts.populated === 0)), code);
  return facts;
}

async function transportFailureResult(request, launch, observed, cleanupUnit, runOuterProbe) {
  const evidence = validateTransportObservation(observed);
  const cleanup = validateCleanupEvidence(
    await cleanupUnit("TRANSPORT_ERROR"), request, launch, "TRANSPORT_ERROR",
  );
  let outerFacts;
  try {
    outerFacts = await runOuterProbe(request, launch, {
      cgroup_path: expectedCgroupPath(request, launch),
    });
  } catch {
    throw rejected("WSL2_VERIFIER_BRIDGE_OUTER_EXIT_REJECTED");
  }
  const outer = validateTransportOuterPostExit(outerFacts, request, launch);
  const result = {
    schema: TRANSPORT_FAILURE_SCHEMA,
    status: "FAILED",
    outcome: "TRANSPORT_ERROR",
    retryable: true,
    task_ref: request.task_ref,
    attempt: request.attempt,
    worktree_ref: request.worktree_ref,
    role: request.role,
    execution_environment_ref: request.environment.identity_digest,
    repository_head: request.environment.linux.repository_head,
    credential_seal_digest: request.preflight_receipt.credential_seal_digest,
    verifier_identity: launch.verifierIdentity,
    unit: launch.serviceUnit,
    process_fence: launch.processFence,
    continuation: {
      retry_of: request.preflight_receipt.continuation.retry_of,
      reconnect_of: request.preflight_receipt.continuation.reconnect_of,
    },
    transport_evidence: evidence,
    outer_cleanup: cleanup,
    outer_post_exit: outer,
    provider_effect_count: 0,
    ...(request.role === "GIT" ? {
      invocation_digest: request.git_invocation.invocation_digest,
    } : {}),
    result_digest: null,
  };
  result.result_digest = digest("wsl2-verifier-transport-failure", Object.fromEntries(
    Object.entries(result).filter(([key]) => key !== "result_digest"),
  ));
  ensure(Buffer.byteLength(canonicalJson(result), "utf8") <= MAX_OUTPUT_BYTES,
    "WSL2_VERIFIER_BRIDGE_OUTPUT_BOUND_EXCEEDED");
  return result;
}

export async function runWsl2VerifierBridge(untrusted, dependencies = {}) {
  const request = validateWsl2VerifierBridgeRequest(untrusted);
  const receipt = request.preflight_receipt;
  const processFence = request.role === "GIT"
    ? request.git_invocation.process_fence
    : verifierFence(request);
  const launch = (dependencies.buildLaunch ?? buildWsl2VerifierLaunch)(request.environment, {
    role: request.role,
    args: request.args,
    fence: processFence,
    preflightFence: receipt.process_fence.fence,
    preflightReceipt: receipt,
    cwd: request.environment.linux.cwd,
    timeoutMs: receipt.timeout.timeout_ms,
    stdoutLimitBytes: receipt.bounds.stdout_limit_bytes,
    stderrLimitBytes: receipt.bounds.stderr_limit_bytes,
    attempt: request.attempt,
    retryOf: receipt.continuation.retry_of,
    reconnectOf: receipt.continuation.reconnect_of,
    ...(request.role === "GIT" ? { gitInvocation: request.git_invocation } : {}),
  });
  validateLaunchAuthority(request, launch, processFence);
  let cleanupReason = null;
  let cleanupPromise = null;
  const cleanupUnit = (reason) => {
    ensure(CLEANUP_REASONS.includes(reason), "WSL2_VERIFIER_BRIDGE_CLEANUP_REJECTED");
    if (cleanupReason !== null) {
      ensure(cleanupReason === reason, "WSL2_VERIFIER_BRIDGE_CLEANUP_REJECTED");
      return cleanupPromise;
    }
    cleanupReason = reason;
    cleanupPromise = (dependencies.runUnitCleanup
      ? dependencies.runUnitCleanup(request, launch, reason)
      : defaultRunUnitCleanup(request, launch, reason, dependencies.execFile ?? execFileDefault));
    return cleanupPromise;
  };
  const runner = dependencies.runLaunch
    ?? (request.role === "GIT" ? defaultRunGitLaunch : defaultRunLaunch);
  const observed = await runner(
    launch,
    receipt.timeout.timeout_ms,
    request.role === "GIT" ? request.gitStdin : undefined,
    cleanupUnit,
    dependencies.spawnProcess ?? spawn,
  );
  if (object(observed) && observed.transport_error !== undefined) {
    return await transportFailureResult(
      request,
      launch,
      observed,
      cleanupUnit,
      dependencies.runOuterProbe ?? defaultRunOuterProbe,
    );
  }
  ensure(Buffer.isBuffer(observed.stdout) && Buffer.isBuffer(observed.stderr)
    && observed.stdout.length <= OUTER_PROCESS_LIMIT_BYTES
    && observed.stderr.length <= OUTER_PROCESS_LIMIT_BYTES,
  "WSL2_VERIFIER_BRIDGE_OUTPUT_BOUND_EXCEEDED");
  const watchdog = observed.watchdog ?? null;
  ensure(watchdog === null || OUTER_WATCHDOGS.includes(watchdog),
    "WSL2_VERIFIER_BRIDGE_PROCESS_REJECTED");
  const cleanupEvidence = watchdog === null ? null
    : validateCleanupEvidence(await cleanupUnit(watchdog), request, launch, watchdog);
  const postExitFacts = watchdog === null ? null
    : await (dependencies.runOuterProbe ?? defaultRunOuterProbe)(request, launch, {
      cgroup_path: expectedCgroupPath(request, launch),
    });
  const stderr = observed.stderr.toString("utf8");
  const marker = framed(stderr, MARKER_SCHEMA, "WSL2_VERIFIER_BRIDGE_FENCE_REJECTED");
  const exit = validateWsl2SubtreeExitReceipt(
    framed(stderr, EXIT_SCHEMA, "WSL2_VERIFIER_BRIDGE_FENCE_REJECTED"),
    request.environment,
    request.role,
  );
  const expectedStdin = request.role === "GIT" ? request.gitStdin : Buffer.alloc(0);
  ensure(marker.fence === launch.processFence && marker.unit === launch.serviceUnit
    && marker.execution_environment_ref === request.environment.identity_digest
    && marker.credential_seal_digest === receipt.credential_seal_digest
    && marker.attempt === request.attempt
    && marker.retry_of === receipt.continuation.retry_of
    && marker.reconnect_of === receipt.continuation.reconnect_of
    && exit.fence === launch.processFence && exit.unit === launch.serviceUnit
    && exit.execution_environment_ref === request.environment.identity_digest
    && exit.credential_seal_digest === receipt.credential_seal_digest
    && marker.cgroup_path === expectedCgroupPath(request, launch)
    && exit.cgroup_path === marker.cgroup_path
    && exit.zero_descendants === true && exit.credential_seal_intact === true
    && exit.credential_watch_intact === true && exit.attempt === request.attempt
    && exit.retry_of === receipt.continuation.retry_of
    && exit.reconnect_of === receipt.continuation.reconnect_of
    && exit.stdout_limit_bytes === receipt.bounds.stdout_limit_bytes
    && exit.stderr_limit_bytes === receipt.bounds.stderr_limit_bytes
    && exit.timeout_ms === receipt.timeout.timeout_ms
    && exit.stdin_bytes === expectedStdin.length
    && exit.stdin_sha256 === sha256(expectedStdin)
    && exit.stdin_complete === true,
  "WSL2_VERIFIER_BRIDGE_FENCE_REJECTED");
  if (exit.output_bound_exceeded === false) {
    ensure(exit.stdout_bytes <= exit.stdout_limit_bytes
      && exit.stderr_bytes <= exit.stderr_limit_bytes
      && exit.stdout_bytes === observed.stdout.length,
    "WSL2_VERIFIER_BRIDGE_FENCE_REJECTED");
  } else {
    ensure(exit.stdout_bytes <= exit.stdout_limit_bytes + 1
      && exit.stderr_bytes <= exit.stderr_limit_bytes + 1
      && (exit.stdout_bytes === exit.stdout_limit_bytes + 1
        || exit.stderr_bytes === exit.stderr_limit_bytes + 1)
      && observed.stdout.length <= exit.stdout_limit_bytes,
    "WSL2_VERIFIER_BRIDGE_FENCE_REJECTED");
  }
  const outcome = verifierOutcome(exit);
  validateObservedProcess(observed, exit, outcome);
  const outerPostExit = validateOuterPostExit(
    postExitFacts ?? await (dependencies.runOuterProbe ?? defaultRunOuterProbe)(request, launch, marker, exit),
    request,
    launch,
    marker,
    outcome,
    watchdog,
  );
  const status = outcome === "PASS" ? "PASS" : "FAILED";
  const result = {
    schema: RESULT_SCHEMA,
    status,
    outcome,
    task_ref: request.task_ref,
    attempt: request.attempt,
    worktree_ref: request.worktree_ref,
    role: request.role,
    repository_head: request.environment.linux.repository_head,
    verifier_identity: launch.verifierIdentity,
    process_marker: marker,
    exit_receipt: exit,
    outer_cleanup: cleanupEvidence,
    outer_post_exit: outerPostExit,
    output: {
      stdout_observed_bytes: observed.stdout.length,
      stderr_observed_bytes: observed.stderr.length,
      stdout_sha256: sha256(observed.stdout),
      stderr_sha256: sha256(observed.stderr),
      ...(request.role === "GIT" ? { stdout_base64: observed.stdout.toString("base64") } : {}),
    },
    provider_effect_count: 0,
    ...(request.role === "GIT" ? { invocation_digest: request.git_invocation.invocation_digest } : {}),
    result_digest: null,
  };
  result.result_digest = digest("wsl2-verifier-result", Object.fromEntries(
    Object.entries(result).filter(([key]) => key !== "result_digest"),
  ));
  ensure(Buffer.byteLength(canonicalJson(result), "utf8") <= MAX_OUTPUT_BYTES,
    "WSL2_VERIFIER_BRIDGE_OUTPUT_BOUND_EXCEEDED");
  return result;
}

async function readBoundedStdin() {
  const chunks = [];
  let length = 0;
  for await (const chunk of process.stdin) {
    length += chunk.length;
    ensure(length <= MAX_GIT_INPUT_BYTES, "WSL2_VERIFIER_BRIDGE_INPUT_BOUND_EXCEEDED");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

export function parseWsl2VerifierBridgeInput(bytes) {
  ensure(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length <= MAX_GIT_INPUT_BYTES,
    "WSL2_VERIFIER_BRIDGE_INPUT_BOUND_EXCEEDED");
  const lines = bytes.toString("utf8").replaceAll("\r", "").split("\n").filter(Boolean);
  ensure(lines.length === 1, "WSL2_VERIFIER_BRIDGE_REQUEST_REJECTED");
  try {
    const value = JSON.parse(lines[0]);
    ensure(value?.role === "GIT" || bytes.length <= MAX_INPUT_BYTES,
      "WSL2_VERIFIER_BRIDGE_INPUT_BOUND_EXCEEDED");
    return value;
  } catch (error) {
    if (error?.code === "WSL2_VERIFIER_BRIDGE_INPUT_BOUND_EXCEEDED") throw error;
    throw rejected();
  }
}

async function main() {
  const result = await runWsl2VerifierBridge(parseWsl2VerifierBridgeInput(await readBoundedStdin()));
  process.stdout.write(`${canonicalJson(result)}\n`);
}

function fail(error) {
  const code = typeof error?.code === "string" && /^WSL2_[A-Z0-9_]+$/u.test(error.code)
    ? error.code : "WSL2_VERIFIER_BRIDGE_FAILED";
  process.stderr.write(`${JSON.stringify({ schema: RESULT_SCHEMA, status: "REJECTED", code })}\n`);
  process.exitCode = 70;
}

const isEntryPoint = process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];
if (isEntryPoint) main().catch(fail);

export const WSL2_VERIFIER_BRIDGE_MAX_INPUT_BYTES = MAX_INPUT_BYTES;
export const WSL2_VERIFIER_BRIDGE_MAX_GIT_INPUT_BYTES = MAX_GIT_INPUT_BYTES;
export const WSL2_VERIFIER_BRIDGE_MAX_OUTPUT_BYTES = MAX_OUTPUT_BYTES;
