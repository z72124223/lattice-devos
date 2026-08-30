import { createHash } from "node:crypto";
import path from "node:path";

const PRODUCTION_SCHEMA = "lattice.execution-environment.wsl2-linux/1.1";
const LEGACY_SCHEMA = "lattice.execution-environment.wsl2-linux/1.0";
const PREFLIGHT_SCHEMA = "lattice.wsl2-zero-model-preflight/1.0";
const HEX_40 = /^[a-f0-9]{40}$/u;
const HEX_64 = /^[a-f0-9]{64}$/u;
const TYPED_DIGEST = /^[a-z0-9][a-z0-9-]*:sha256:[a-f0-9]{64}$/u;
export const WSL2_PROCESS_MARKER_SCHEMA = "lattice.wsl2-process-fence/1.1";
export const WSL2_SUBTREE_EXIT_SCHEMA = "lattice.wsl2-subtree-exit/1.2";
export const MAX_WSL2_ATTEMPTS = 3;
export const WSL2_IMMUTABLE_SNAPSHOT_SCHEMA = "lattice.wsl2-immutable-snapshot/1.0";
export const WSL2_SANDBOX_POLICY_SCHEMA = "lattice.wsl2-sandbox-policy/1.0";
export const WSL2_PRIVILEGE_BOUNDARY_SCHEMA = "lattice.wsl2-privilege-boundary/1.0";

const WSL2_IMMUTABLE_TREE_NAMES = Object.freeze([
  "codex", "supervisor_runtime", "node", "rust", "keyring",
]);

const WSL2_SUBTREE_EXIT_KEYS = Object.freeze([
  "schema", "fence", "unit", "execution_environment_ref", "credential_seal_digest",
  "cgroup_path", "zero_descendants", "credential_seal_intact", "credential_watch_intact",
  "keyring_daemon_sha256", "keyring_library_manifest_digest", "tool_input_identities",
  "stdout_bytes", "stderr_bytes", "stdout_limit_bytes", "stderr_limit_bytes",
  "output_bound_exceeded", "timeout_ms", "timed_out", "interrupted", "stdin_bytes",
  "stdin_sha256", "stdin_complete", "attempt", "retry_of", "reconnect_of", "exit_code",
  "exit_signal",
]);
const WSL2_TOOL_INPUT_KEYS = Object.freeze([
  "executable", "verifier_tool", "sandbox_helper", "node_runtime", "rustc", "rustdoc",
  "keyring_daemon", "keyring_libraries",
]);
const WSL2_SEAL_KEYS = Object.freeze([
  "path", "resolved_path", "sha256", "device", "inode", "owner_uid", "mode", "size",
]);
const WSL2_KEYRING_LIBRARY_FILES = Object.freeze([
  "libgck-1.so.0.0.0", "libgcr-base-3.so.1.0.0",
]);

const SUPERVISOR_BOOTSTRAP_SOURCE = String.raw`
const fs = require("node:fs");
const crypto = require("node:crypto");
const process = require("node:process");
(async () => {
  const [supervisorPath, expectedSha256, ...supervisorArgs] = process.argv.slice(1);
  if (typeof supervisorPath !== "string" || !supervisorPath.startsWith("/home/")
      || !/^[a-f0-9]{64}$/u.test(expectedSha256 || "")) throw new Error("BOOTSTRAP_INPUT");
  const fd = fs.openSync(supervisorPath, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  let source;
  try {
    const identity = fs.fstatSync(fd);
    if ((identity.mode & fs.constants.S_IFMT) !== fs.constants.S_IFREG
        || (identity.uid !== 0 && identity.uid !== process.getuid()) || (identity.mode & 0o022) !== 0
        || identity.size <= 0 || identity.size > 1_048_576) throw new Error("BOOTSTRAP_IDENTITY");
    source = fs.readFileSync(fd);
    if (source.length !== identity.size
        || crypto.createHash("sha256").update(source).digest("hex") !== expectedSha256) {
      throw new Error("BOOTSTRAP_DIGEST");
    }
  } finally {
    fs.closeSync(fd);
  }
  process.argv = [process.argv[0], supervisorPath, ...supervisorArgs];
  const loaded = await import("data:text/javascript;base64," + source.toString("base64"));
  if (typeof loaded.runWsl2CodexSupervisor !== "function") throw new Error("BOOTSTRAP_EXPORT");
  await loaded.runWsl2CodexSupervisor();
})().catch(() => {
  process.stderr.write(JSON.stringify({
    schema: "lattice.wsl2-supervisor-bootstrap-rejection/1.0",
    status: "REJECTED",
    code: "WSL2_SUPERVISOR_BOOTSTRAP_REJECTED",
  }) + "\n");
  process.exitCode = 70;
});
`;

export const WSL2_SUPERVISOR_BOOTSTRAP_SOURCE = SUPERVISOR_BOOTSTRAP_SOURCE;
export const WSL2_SUPERVISOR_BOOTSTRAP_SHA256 = createHash("sha256")
  .update(SUPERVISOR_BOOTSTRAP_SOURCE, "utf8").digest("hex");

function rejected(code = "WSL2_EXECUTION_ENVIRONMENT_REJECTED") {
  const error = new Error(code);
  error.code = code;
  return error;
}

function check(condition, code = "WSL2_EXECUTION_ENVIRONMENT_REJECTED") {
  if (!condition) throw rejected(code);
}

function object(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function exactKeys(value, keys) {
  check(object(value));
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  check(actual.length === expected.length && actual.every((key, index) => key === expected[index]));
}

function validateReceiptSeal(value, expected, { library = false } = {}) {
  exactKeys(value, library ? ["manifest_path", ...WSL2_SEAL_KEYS] : WSL2_SEAL_KEYS);
  check(value.path === expected.path && value.sha256 === expected.sha256
    && canonicalLinuxPath(value.resolved_path)
    && value.owner_uid === expected.owner_uid
    && Number.isSafeInteger(value.mode) && value.mode > 0 && (value.mode & 0o022) === 0
    && Number.isSafeInteger(value.size) && value.size > 0
    && typeof value.device === "string" && /^[0-9]+$/u.test(value.device)
    && typeof value.inode === "string" && /^[0-9]+$/u.test(value.inode));
  if (library) check(value.manifest_path === expected.manifest_path);
}

/**
 * Validates the complete supervisor terminal receipt before any caller may
 * treat a WSL verifier/preflight result as fenced. Role selects the exact
 * executable closure that must have been held open by the supervisor.
 */
export function validateWsl2SubtreeExitReceipt(receipt, environment, role) {
  const descriptor = validateWsl2ExecutionEnvironment(environment);
  check(["PROVIDER", "PREFLIGHT", "NODE", "CARGO", "GIT"].includes(role));
  exactKeys(receipt, WSL2_SUBTREE_EXIT_KEYS);
  const receiptFacts = [
    ["SCHEMA", receipt.schema === WSL2_SUBTREE_EXIT_SCHEMA],
    ["ENVIRONMENT", receipt.execution_environment_ref === descriptor.identity_digest],
    ["CREDENTIAL", typed(receipt.credential_seal_digest, "credential-seal")],
    ["CGROUP", canonicalLinuxPath(receipt.cgroup_path)],
    ["INTEGRITY", typeof receipt.zero_descendants === "boolean"
      && typeof receipt.credential_seal_intact === "boolean"
      && typeof receipt.credential_watch_intact === "boolean"],
    ["KEYRING", receipt.keyring_daemon_sha256 === descriptor.linux.keyring_daemon_sha256
      && receipt.keyring_library_manifest_digest === descriptor.linux.keyring_library_manifest_digest],
    ["OUTPUT", Number.isSafeInteger(receipt.stdout_bytes) && receipt.stdout_bytes >= 0
      && Number.isSafeInteger(receipt.stderr_bytes) && receipt.stderr_bytes >= 0
      && Number.isSafeInteger(receipt.stdout_limit_bytes) && receipt.stdout_limit_bytes >= 1_024
      && Number.isSafeInteger(receipt.stderr_limit_bytes) && receipt.stderr_limit_bytes >= 1_024
      && receipt.stdout_bytes <= receipt.stdout_limit_bytes + 1
      && receipt.stderr_bytes <= receipt.stderr_limit_bytes + 1
      && typeof receipt.output_bound_exceeded === "boolean"
      && (receipt.output_bound_exceeded
        ? receipt.stdout_bytes === receipt.stdout_limit_bytes + 1
          || receipt.stderr_bytes === receipt.stderr_limit_bytes + 1
        : receipt.stdout_bytes <= receipt.stdout_limit_bytes
          && receipt.stderr_bytes <= receipt.stderr_limit_bytes)],
    ["TIMEOUT", Number.isSafeInteger(receipt.timeout_ms) && receipt.timeout_ms >= 1_000
      && typeof receipt.timed_out === "boolean" && typeof receipt.interrupted === "boolean"],
    ["STDIN", Number.isSafeInteger(receipt.stdin_bytes) && receipt.stdin_bytes >= 0
      && HEX_64.test(receipt.stdin_sha256) && typeof receipt.stdin_complete === "boolean"],
    ["CONTINUATION", Number.isSafeInteger(receipt.attempt) && receipt.attempt >= 1
      && (receipt.retry_of === null || TYPED_DIGEST.test(receipt.retry_of))
      && (receipt.reconnect_of === null || TYPED_DIGEST.test(receipt.reconnect_of))
      && (receipt.retry_of === null || receipt.reconnect_of === null)],
    ["TERMINAL", receipt.exit_code === null
      || (Number.isSafeInteger(receipt.exit_code) && receipt.exit_code >= 0 && receipt.exit_code <= 255)],
    ["SIGNAL", receipt.exit_signal === null
      || (typeof receipt.exit_signal === "string" && /^[A-Z0-9]{1,32}$/u.test(receipt.exit_signal))],
  ];
  const invalidFact = receiptFacts.find(([, accepted]) => !accepted);
  check(invalidFact === undefined,
    `WSL2_SUBTREE_EXIT_${invalidFact?.[0] ?? "RECEIPT"}_REJECTED`);
  const toolchain = descriptor.verification_toolchain;
  const expectedExecutable = role === "PROVIDER"
    ? { path: descriptor.linux.launcher_path, sha256: descriptor.linux.launcher_sha256 }
    : toolchain.sandbox;
  const expectedVerifier = role === "NODE" ? toolchain.npm
    : role === "CARGO" ? toolchain.cargo
      : role === "GIT" ? { path: descriptor.linux.git_path, sha256: descriptor.linux.git_sha256 }
        : null;
  exactKeys(receipt.tool_input_identities, WSL2_TOOL_INPUT_KEYS);
  const inputs = receipt.tool_input_identities;
  validateReceiptSeal(inputs.executable, { ...expectedExecutable, owner_uid: 0 });
  validateReceiptSeal(inputs.sandbox_helper, { ...toolchain.sandbox_helper, owner_uid: 0 });
  validateReceiptSeal(inputs.keyring_daemon, {
    path: descriptor.linux.keyring_daemon_path,
    sha256: descriptor.linux.keyring_daemon_sha256,
    owner_uid: 0,
  });
  if (expectedVerifier === null) check(inputs.verifier_tool === null);
  else validateReceiptSeal(inputs.verifier_tool, { ...expectedVerifier, owner_uid: 0 });
  const expectedNode = ["PREFLIGHT", "NODE"].includes(role)
    ? { path: descriptor.linux.node_path, sha256: descriptor.linux.node_sha256, owner_uid: 0 }
    : null;
  if (expectedNode === null) check(inputs.node_runtime === null);
  else validateReceiptSeal(inputs.node_runtime, expectedNode);
  for (const [name, expected] of [["rustc", role === "CARGO" ? toolchain.rustc : null],
    ["rustdoc", role === "CARGO" ? toolchain.rustdoc : null]]) {
    if (expected === null) check(inputs[name] === null);
    else validateReceiptSeal(inputs[name], { ...expected, owner_uid: 0 });
  }
  check(Array.isArray(inputs.keyring_libraries)
    && inputs.keyring_libraries.length === WSL2_KEYRING_LIBRARY_FILES.length);
  for (const [index, manifestPath] of WSL2_KEYRING_LIBRARY_FILES.entries()) {
    const observed = inputs.keyring_libraries[index];
    validateReceiptSeal(observed, {
      manifest_path: manifestPath,
      path: `${descriptor.linux.keyring_library_path}/${manifestPath}`,
      sha256: observed?.sha256,
      owner_uid: 0,
    }, { library: true });
  }
  return receipt;
}

export function canonicalJson(value) {
  const visit = (entry) => {
    if (Array.isArray(entry)) return entry.map(visit);
    if (object(entry)) {
      return Object.fromEntries(Object.keys(entry).sort().map((key) => [key, visit(entry[key])]));
    }
    return entry;
  };
  return JSON.stringify(visit(value));
}

function typedDigest(domain, subject) {
  const digest = createHash("sha256").update(canonicalJson(subject), "utf8").digest("hex");
  return `${domain}:sha256:${digest}`;
}

function without(value, ...keys) {
  return Object.fromEntries(Object.entries(value).filter(([key]) => !keys.includes(key)));
}

function canonicalLinuxPath(value, { home = false } = {}) {
  return typeof value === "string"
    && value.startsWith(home ? "/home/" : "/")
    && !value.includes("\\")
    && !value.includes("\0")
    && path.posix.normalize(value) === value;
}

function linuxFileUri(value) {
  check(canonicalLinuxPath(value));
  return `file://${value.split("/").map((part) => encodeURIComponent(part)).join("/")}`;
}

/**
 * Materializes one exact, network-restricted Codex Linux sandbox state. The
 * state is passed as data instead of resolved from ambient config so every
 * writable and denied root is part of the supervised command digest.
 */
export function buildWsl2SandboxState(environment, {
  role,
  cwd,
  writableRoots = [],
  deniedRoots = [],
} = {}) {
  const descriptor = validateWsl2ExecutionEnvironment(environment);
  check(["PREFLIGHT", "NODE", "CARGO", "GIT"].includes(role));
  check(canonicalLinuxPath(cwd, { home: true }));
  const unique = (values) => [...new Set(values)];
  const linuxHome = descriptor.verification_toolchain.task_root.split("/").slice(0, 3).join("/");
  const writes = unique(writableRoots);
  const denies = unique([
    ...deniedRoots,
    `${linuxHome}/.codex`,
    "/mnt",
    descriptor.process_fence.user_runtime_dir,
  ]);
  for (const candidate of [...writes, ...denies]) check(canonicalLinuxPath(candidate));
  check(!writes.some((candidate) => denies.includes(candidate)));
  check(deniedRoots.every((candidate) => candidate === descriptor.linux.codex_home));
  const roleTemplate = buildWsl2SandboxPolicyTemplate(descriptor).role_writes[role];
  if (role !== "GIT") check(canonicalJson(writes) === canonicalJson(roleTemplate));
  else check(writes.length >= 2 && writes.length <= 3
    && writes.every((candidate) => descendant(descriptor.verification_toolchain.task_root, candidate)));
  const entries = [
    { path: { type: "special", value: { kind: "minimal" } }, access: "read" },
    { path: { type: "path", path: descriptor.verification_toolchain.task_root }, access: "read" },
    ...writes.map((candidate) => ({ path: { type: "path", path: candidate }, access: "write" })),
    ...denies.map((candidate) => ({
      path: { type: "path", path: candidate },
      access: "deny",
      missing_path_behavior: "skip",
    })),
  ];
  return Object.freeze({
    permissionProfile: Object.freeze({
      type: "managed",
      file_system: Object.freeze({ type: "restricted", entries: Object.freeze(entries) }),
      network: "restricted",
    }),
    // The CLI derives its sealed multicall Linux sandbox stage from the
    // already-running executable. The separately pinned helper is the exact
    // root-owned system bwrap selected by the closed /usr/bin:/bin PATH.
    codexLinuxSandboxExe: null,
    sandboxCwd: linuxFileUri(cwd),
    useLegacyLandlock: false,
  });
}

/**
 * The durable sandbox policy is a digest of this complete role template, not
 * a digest of the two-field policy reference. Dynamic Git roots are named as
 * constrained request slots; every concrete launch remains independently
 * bound by its verifier-command digest.
 */
export function buildWsl2SandboxPolicyTemplate(descriptor) {
  check(object(descriptor?.linux) && object(descriptor?.process_fence)
    && object(descriptor?.verification_toolchain));
  const linux = descriptor.linux;
  const toolchain = descriptor.verification_toolchain;
  const linuxHome = toolchain.task_root.split("/").slice(0, 3).join("/");
  const denyPaths = [...new Set([
    linux.codex_home, `${linuxHome}/.codex`, "/mnt", descriptor.process_fence.user_runtime_dir,
  ])];
  return Object.freeze({
    schema: "lattice.wsl2-sandbox-template/1.0",
    permission_profile_type: "managed",
    filesystem_type: "restricted",
    network: "restricted",
    base_entries: Object.freeze([
      Object.freeze({ path: Object.freeze({ type: "special", value: Object.freeze({ kind: "minimal" }) }), access: "read" }),
      Object.freeze({ path: Object.freeze({ type: "path", path: toolchain.task_root }), access: "read" }),
    ]),
    role_writes: Object.freeze({
      PREFLIGHT: Object.freeze([
        linux.cwd, toolchain.home_dir, toolchain.temp_dir, toolchain.npm_cache,
        toolchain.cargo_home, toolchain.cargo_target_dir,
      ]),
      NODE: Object.freeze([toolchain.home_dir, toolchain.temp_dir, toolchain.npm_cache]),
      CARGO: Object.freeze([
        toolchain.home_dir, toolchain.temp_dir, toolchain.cargo_home, toolchain.cargo_target_dir,
      ]),
      GIT: Object.freeze({
        bootstrap: Object.freeze(["$GIT_CONTROL_HOME", "$GIT_CONTROL_TMPDIR"]),
        guarded_object_write: Object.freeze([
          "$GIT_CONTROL_HOME", "$GIT_CONTROL_TMPDIR", "$GIT_COMMON_DIR/objects",
        ]),
        guarded_index_write: Object.freeze([
          "$GIT_CONTROL_HOME", "$GIT_CONTROL_TMPDIR", "$GIT_CONTROL_ROOT/candidate-index",
        ]),
      }),
    }),
    deny_entries: Object.freeze(denyPaths.map((candidate) => Object.freeze({
      path: candidate,
      missing_path_behavior: "skip",
    }))),
    codex_linux_sandbox_exe: null,
    sandbox_cwd: linuxFileUri(linux.cwd),
    use_legacy_landlock: false,
  });
}

function descendant(root, candidate) {
  return canonicalLinuxPath(root, { home: true })
    && canonicalLinuxPath(candidate, { home: true })
    && candidate.startsWith(`${root}/`);
}

function sha(value) {
  return typeof value === "string" && HEX_64.test(value);
}

function nodeVersionAtLeast(value, minimum) {
  const match = typeof value === "string"
    ? /^v([0-9]{1,6})\.([0-9]{1,6})\.([0-9]{1,6})$/u.exec(value)
    : null;
  if (match === null) return false;
  const actual = match.slice(1).map(Number);
  for (let index = 0; index < minimum.length; index += 1) {
    if (actual[index] !== minimum[index]) return actual[index] > minimum[index];
  }
  return true;
}

const WSL2_TOOL_VERSION_GRAMMARS = Object.freeze({
  WSL_GATEWAY: /^[0-9]{1,6}(?:\.[0-9]{1,6}){2,3}$/u,
  CODEX_CLI: /^codex-cli [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$/u,
  NODE: /^v[0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$/u,
  GIT: /^git version [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$/u,
  SYSTEMD: /^systemd [0-9]{2,4}(?: \([0-9A-Za-z.+:~_-]+\))?$/u,
  NPM: /^[0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$/u,
  CARGO: /^cargo [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6} \([a-f0-9]{7,40} [0-9]{4}-[0-9]{2}-[0-9]{2}\)$/u,
  RUSTC: /^rustc [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6} \([a-f0-9]{7,40} [0-9]{4}-[0-9]{2}-[0-9]{2}\)$/u,
  RUSTDOC: /^rustdoc [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6} \([a-f0-9]{7,40} [0-9]{4}-[0-9]{2}-[0-9]{2}\)$/u,
  BWRAP: /^bubblewrap [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}$/u,
  SUDO: /^(?:Sudo version [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}(?:p[0-9]{1,64})?|sudo-rs [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}-[0-9A-Za-z.+~_-]{1,64})$/u,
  LSATTR: /^lsattr [0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6} \([0-9]{1,2}-[A-Za-z]{3}-[0-9]{4}\)$/u,
});

const CREDENTIAL_SHAPED_VERSION_PAYLOAD = /(?:^|[\s,;])(?:token|password|secret|api[_-]?key|authorization)\s*=/iu;
const EXECUTION_ENVIRONMENT_STRING_SCAN_MAX_DEPTH = 16;
const EXECUTION_ENVIRONMENT_STRING_SCAN_MAX_NODES = 512;
const EXECUTION_ENVIRONMENT_STRING_SCAN_MAX_BYTES = 4096;
const EXECUTION_ENVIRONMENT_SECRET_PREFIXES = Object.freeze([
  "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "glpat-", "npm_", "pypi-",
  "xoxa-", "xoxb-", "xoxp-", "xoxr-", "xoxs-",
]);
const EXECUTION_ENVIRONMENT_SENSITIVE_ASSIGNMENT =
  /(?:^|[^a-z0-9_-])(?:password|passphrase|passwd|pwd|token|access[ _-]token|refresh[ _-]token|id[ _-]token|session[ _-]token|api[ _-]?key|apikey|client[ _-]secret|secret|credential|credentials|cookie|set-cookie|authorization)\s*["']?\s*[:=]/iu;
const EXECUTION_ENVIRONMENT_AWS_ACCESS_KEY =
  /(?:^|[^A-Za-z0-9])(?:AKIA|ASIA)[A-Z0-9]{16}(?:[^A-Za-z0-9]|$)/u;

function containsBoundedSecretPrefix(value, prefix) {
  for (let index = value.indexOf(prefix); index >= 0;
    index = value.indexOf(prefix, index + prefix.length)) {
    if (index === 0 || !/[a-z0-9]/u.test(value[index - 1])) return true;
  }
  return false;
}

function executionEnvironmentStringContainsRecognizedSecret(value) {
  const lower = value.toLowerCase();
  return lower.includes("bearer ")
    || (lower.includes("-----begin ") && lower.includes("private key-----"))
    || /:\/\/[^/?#\s"'<>}{]+@/u.test(value)
    || EXECUTION_ENVIRONMENT_SECRET_PREFIXES.some((prefix) => lower.includes(prefix))
    || containsBoundedSecretPrefix(lower, "sk-")
    || EXECUTION_ENVIRONMENT_SENSITIVE_ASSIGNMENT.test(value)
    || EXECUTION_ENVIRONMENT_AWS_ACCESS_KEY.test(value);
}

function executionEnvironmentStringLeavesAreSecretFree(root) {
  const pending = [{ value: root, depth: 0 }];
  let nodes = 0;
  while (pending.length > 0) {
    const { value, depth } = pending.pop();
    nodes += 1;
    if (nodes > EXECUTION_ENVIRONMENT_STRING_SCAN_MAX_NODES) return false;
    if (typeof value === "string") {
      if (Buffer.byteLength(value, "utf8") > EXECUTION_ENVIRONMENT_STRING_SCAN_MAX_BYTES
        || executionEnvironmentStringContainsRecognizedSecret(value)) return false;
      continue;
    }
    if (value === null || typeof value !== "object") continue;
    if (depth >= EXECUTION_ENVIRONMENT_STRING_SCAN_MAX_DEPTH) return false;
    const children = Array.isArray(value) ? value : Object.values(value);
    if (nodes + pending.length + children.length > EXECUTION_ENVIRONMENT_STRING_SCAN_MAX_NODES) {
      return false;
    }
    for (const child of children) pending.push({ value: child, depth: depth + 1 });
  }
  return true;
}

export function isClosedWsl2ToolVersion(kind, value) {
  const grammar = WSL2_TOOL_VERSION_GRAMMARS[kind];
  return grammar instanceof RegExp
    && typeof value === "string" && value.length >= 1 && value.length <= 128
    && !CREDENTIAL_SHAPED_VERSION_PAYLOAD.test(value) && grammar.test(value);
}

function typed(value, domain) {
  return typeof value === "string"
    && (domain ? new RegExp(`^${domain}:sha256:[a-f0-9]{64}$`, "u").test(value) : TYPED_DIGEST.test(value));
}

function validateTool(tool, versionKind) {
  exactKeys(tool, ["path", "version", "sha256"]);
  check(canonicalLinuxPath(tool.path) && isClosedWsl2ToolVersion(versionKind, tool.version)
    && sha(tool.sha256));
}

export function distributionIdentity(descriptor) {
  const identity = descriptor?.distribution_identity;
  check(object(identity));
  return typedDigest("wsl2-distribution", {
    distribution: descriptor.distribution,
    ...without(identity, "identity_digest"),
  });
}

export function credentialAuthorityIdentity(descriptor) {
  const authority = descriptor?.credential_authority;
  const linux = descriptor?.linux;
  check(object(authority) && object(linux));
  return typedDigest("wsl2-credential-authority", {
    kind: authority.kind,
    distribution_identity_ref: descriptor.distribution_identity?.identity_digest,
    codex_home: linux.codex_home,
    config_digest: linux.config_digest,
    keyring_daemon_path: linux.keyring_daemon_path,
    keyring_daemon_sha256: linux.keyring_daemon_sha256,
    keyring_library_path: linux.keyring_library_path,
    keyring_library_manifest_digest: linux.keyring_library_manifest_digest,
    xdg_runtime_dir: linux.xdg_runtime_dir,
  });
}

export function codexHomeIdentity(descriptor) {
  check(object(descriptor?.linux) && object(descriptor?.credential_authority));
  return typedDigest("codex-home", {
    distribution_identity_ref: descriptor.distribution_identity?.identity_digest,
    linux_codex_home: descriptor.linux.codex_home,
    config_digest: descriptor.linux.config_digest,
    credential_authority_ref: descriptor.credential_authority.authority_digest,
  });
}

export function processFenceAuthorityIdentity(descriptor) {
  const fence = descriptor?.process_fence;
  check(object(fence));
  return typedDigest("wsl2-process-fence-authority", {
    distribution_identity_ref: descriptor.distribution_identity?.identity_digest,
    ...without(fence, "identity_digest"),
  });
}

export function verificationToolchainIdentity(descriptor) {
  const toolchain = descriptor?.verification_toolchain;
  check(object(toolchain));
  return typedDigest("wsl2-verification-toolchain", without(toolchain, "identity_digest"));
}

export function immutableSnapshotIdentity(descriptor) {
  const snapshot = descriptor?.immutable_snapshot;
  check(object(snapshot));
  return typedDigest("wsl2-immutable-snapshot", without(snapshot, "snapshot_digest"));
}

export function sandboxPolicyIdentity(descriptor) {
  check(object(descriptor?.sandbox_policy));
  return typedDigest("wsl2-sandbox-policy", buildWsl2SandboxPolicyTemplate(descriptor));
}

export function privilegeBoundaryIdentity(descriptor) {
  const boundary = descriptor?.privilege_boundary;
  check(object(boundary));
  return typedDigest("wsl2-privilege-boundary", without(boundary, "boundary_digest"));
}

export function linuxCapabilitiesIdentity({ effective_uid: effectiveUid,
  effective_gid: effectiveGid, proc_status_cap_eff: capEff }) {
  check(Number.isSafeInteger(effectiveUid) && effectiveUid >= 0
    && Number.isSafeInteger(effectiveGid) && effectiveGid >= 0
    && typeof capEff === "string" && /^[a-f0-9]{16}$/u.test(capEff));
  return typedDigest("linux-capabilities", {
    effective_uid: effectiveUid,
    effective_gid: effectiveGid,
    proc_status_cap_eff: capEff,
  });
}

export function validateWsl2ImmutableObservation(observation, environmentInput,
  code = "WSL2_IMMUTABLE_OBSERVATION_REJECTED") {
  try {
    const environment = validateWsl2ExecutionEnvironment(environmentInput);
    exactKeys(observation, [
      "schema", "execution_environment_ref", "immutable_snapshot_ref", "sandbox_policy_ref",
      "privilege_boundary_ref", "task_root", "trees", "privilege", "probe_tools", "bounds",
      "observation_digest",
    ]);
    check(observation.schema === "lattice.wsl2-immutable-observation/1.0"
      && observation.execution_environment_ref === environment.identity_digest
      && observation.immutable_snapshot_ref === environment.immutable_snapshot.snapshot_digest
      && observation.sandbox_policy_ref === environment.sandbox_policy.policy_digest
      && observation.privilege_boundary_ref === environment.privilege_boundary.boundary_digest, code);
    const expectedRoot = environment.immutable_snapshot;
    exactKeys(observation.task_root, [
      "path", "device", "inode", "owner_uid", "owner_gid", "mode", "immutable",
    ]);
    check(observation.task_root.path === expectedRoot.task_root_path
      && observation.task_root.device === expectedRoot.task_root_device
      && observation.task_root.inode === expectedRoot.task_root_inode
      && observation.task_root.owner_uid === expectedRoot.task_root_owner_uid
      && observation.task_root.owner_gid === expectedRoot.task_root_owner_gid
      && observation.task_root.mode === expectedRoot.task_root_mode
      && observation.task_root.immutable === expectedRoot.task_root_immutable, code);
    exactKeys(observation.trees, WSL2_IMMUTABLE_TREE_NAMES);
    for (const name of WSL2_IMMUTABLE_TREE_NAMES) {
      const tree = observation.trees[name];
      exactKeys(tree, ["root", "manifest_digest", "entry_count", "file_bytes"]);
      check(tree.root === expectedRoot.trees[name].root
        && tree.manifest_digest === expectedRoot.trees[name].manifest_digest
        && Number.isSafeInteger(tree.entry_count) && tree.entry_count >= 1 && tree.entry_count <= 200_000
        && Number.isSafeInteger(tree.file_bytes) && tree.file_bytes >= 0
        && tree.file_bytes <= 8 * 1_073_741_824, code);
    }
    const expectedBoundary = environment.privilege_boundary;
    exactKeys(observation.privilege, [
      "effective_uid", "effective_gid", "effective_capabilities_digest", "capabilities_empty",
      "noninteractive_root_unavailable", "sudo_denial_recognized", "sudo_exit_code",
      "sudo_stdout_bytes", "sudo_stderr_bytes", "sudo_stdout_sha256", "sudo_stderr_sha256",
    ]);
    check(observation.privilege.effective_uid === expectedBoundary.effective_uid
      && observation.privilege.effective_gid === expectedBoundary.effective_gid
      && observation.privilege.effective_capabilities_digest
        === expectedBoundary.effective_capabilities_digest
      && observation.privilege.capabilities_empty === true
      && observation.privilege.noninteractive_root_unavailable
        === expectedBoundary.noninteractive_root_unavailable
      && observation.privilege.sudo_denial_recognized === true
      && observation.privilege.sudo_exit_code === 1
      && observation.privilege.sudo_stdout_bytes === 0
      && Number.isSafeInteger(observation.privilege.sudo_stderr_bytes)
      && observation.privilege.sudo_stderr_bytes >= 1
      && observation.privilege.sudo_stderr_bytes <= 16_384
      && observation.privilege.sudo_stdout_sha256
        === createHash("sha256").update(Buffer.alloc(0)).digest("hex")
      && sha(observation.privilege.sudo_stderr_sha256), code);
    exactKeys(observation.probe_tools, [
      "controller", "lsattr", "noninteractive_root", "source_sha256",
    ]);
    for (const [name, expected, versionKind] of [
      ["controller", environment.process_fence.supervisor_bootstrap_node, "NODE"],
      ["lsattr", environment.process_fence.immutable_probe_lsattr, "LSATTR"],
      ["noninteractive_root", environment.process_fence.noninteractive_root_probe, "SUDO"],
    ]) {
      validateTool(observation.probe_tools[name], versionKind);
      check(canonicalJson(observation.probe_tools[name]) === canonicalJson(expected), code);
    }
    check(sha(observation.probe_tools.source_sha256), code);
    exactKeys(observation.bounds, [
      "max_entries_per_tree", "max_file_bytes_per_tree", "max_single_file_bytes",
    ]);
    check(observation.bounds.max_entries_per_tree === 200_000
      && observation.bounds.max_file_bytes_per_tree === 8 * 1_073_741_824
      && observation.bounds.max_single_file_bytes === 1_073_741_824, code);
    check(observation.observation_digest === typedDigest("wsl2-immutable-observation",
      without(observation, "observation_digest")), code);
    return structuredClone(observation);
  } catch (error) {
    if (error?.code === code) throw error;
    throw rejected(code);
  }
}

/**
 * Identity framing is UTF-8 SHA-256 over canonical JSON (recursively sorted
 * object keys, arrays kept in order). Only the top-level identity_digest is
 * omitted; all nested identity and mapping digests remain part of the subject.
 */
export function executionEnvironmentIdentity(descriptor) {
  check(object(descriptor));
  return typedDigest("execution-environment", without(descriptor, "identity_digest"));
}

export function pathMappingIdentity(descriptor) {
  check(object(descriptor?.path_mapping) && object(descriptor?.linux));
  return typedDigest("path-mapping", {
    distribution: descriptor.distribution,
    windows_path: descriptor.path_mapping.windows_path,
    linux_path: descriptor.path_mapping.linux_path,
    repository_identity: descriptor.linux.repository_identity,
    repository_head: descriptor.linux.repository_head,
  });
}

export function windowsWslPathToLinux(windowsPath, distribution) {
  check(typeof windowsPath === "string" && typeof distribution === "string");
  const prefix = `\\\\wsl.localhost\\${distribution}\\`;
  check(windowsPath.toLowerCase().startsWith(prefix.toLowerCase()));
  const rest = windowsPath.slice(prefix.length).replaceAll("\\", "/");
  const linux = `/${rest}`;
  check(canonicalLinuxPath(linux, { home: true }));
  return linux;
}

function validateProduction(untrusted) {
  exactKeys(untrusted, [
    "schema", "kind", "distribution", "distribution_identity", "gateway", "linux",
    "credential_authority", "process_fence", "verification_toolchain", "path_mapping",
    "immutable_snapshot", "sandbox_policy", "privilege_boundary", "identity_digest",
  ]);
  check(untrusted.schema === PRODUCTION_SCHEMA && untrusted.kind === "WSL2_LINUX");
  check(/^[A-Za-z0-9._-]{1,64}$/u.test(untrusted.distribution));

  const distribution = untrusted.distribution_identity;
  exactKeys(distribution, [
    "os_id", "os_version_id", "os_version_codename", "os_release_sha256",
    "kernel_release", "identity_digest",
  ]);
  check(/^[a-z0-9._-]+$/u.test(distribution.os_id));
  check(/^[0-9]+(?:\.[0-9]+)*$/u.test(distribution.os_version_id));
  check(/^[a-z0-9._-]+$/u.test(distribution.os_version_codename));
  check(sha(distribution.os_release_sha256));
  check(typeof distribution.kernel_release === "string" && /microsoft-standard-WSL2$/u.test(distribution.kernel_release));
  check(distribution.identity_digest === distributionIdentity(untrusted));

  const gateway = untrusted.gateway;
  exactKeys(gateway, ["windows_path", "version", "sha256"]);
  check(typeof gateway.windows_path === "string" && /^[A-Za-z]:\\/u.test(gateway.windows_path));
  check(gateway.windows_path.toLowerCase().endsWith("\\wsl.exe"));
  check(isClosedWsl2ToolVersion("WSL_GATEWAY", gateway.version) && sha(gateway.sha256));

  const linux = untrusted.linux;
  exactKeys(linux, [
    "launcher_path", "launcher_version", "launcher_sha256", "node_path", "node_version",
    "node_sha256", "git_path", "git_version", "git_sha256", "supervisor_path",
    "supervisor_sha256", "codex_home", "config_digest", "cwd", "repository_head",
    "repository_identity", "dbus_run_session_path", "dbus_run_session_sha256",
    "setsid_path", "setsid_sha256", "keyring_daemon_path", "keyring_daemon_sha256",
    "keyring_library_path", "keyring_library_manifest_digest", "xdg_runtime_dir",
  ]);
  for (const [file, digest] of [
    [linux.launcher_path, linux.launcher_sha256], [linux.node_path, linux.node_sha256],
    [linux.git_path, linux.git_sha256], [linux.supervisor_path, linux.supervisor_sha256],
    [linux.dbus_run_session_path, linux.dbus_run_session_sha256],
    [linux.setsid_path, linux.setsid_sha256], [linux.keyring_daemon_path, linux.keyring_daemon_sha256],
  ]) check(canonicalLinuxPath(file) && sha(digest));
  check(isClosedWsl2ToolVersion("CODEX_CLI", linux.launcher_version));
  check(isClosedWsl2ToolVersion("NODE", linux.node_version));
  check(isClosedWsl2ToolVersion("GIT", linux.git_version));
  check(canonicalLinuxPath(linux.codex_home, { home: true }));
  check(typed(linux.config_digest, "codex-config"));
  check(canonicalLinuxPath(linux.cwd, { home: true }) && HEX_40.test(linux.repository_head));
  check(typed(linux.repository_identity, "repository"));
  check(canonicalLinuxPath(linux.keyring_library_path, { home: true }));
  check(typed(linux.keyring_library_manifest_digest, "keyring-library-manifest"));
  check(/^\/run\/user\/[0-9]+$/u.test(linux.xdg_runtime_dir));

  const credential = untrusted.credential_authority;
  exactKeys(credential, ["kind", "authority_digest"]);
  check(credential.kind === "LINUX_KEYRING");
  check(credential.authority_digest === credentialAuthorityIdentity(untrusted));

  const processFence = untrusted.process_fence;
  exactKeys(processFence, [
    "schema", "kind", "systemd_run_path", "systemd_run_version", "systemd_run_sha256",
    "systemctl_path", "systemctl_version", "systemctl_sha256", "cgroup_mount",
    "user_runtime_dir", "unit_prefix", "supervisor_bootstrap_node", "immutable_probe_lsattr",
    "noninteractive_root_probe", "identity_digest",
  ]);
  check(processFence.schema === "lattice.wsl2-cgroup-v2-fence/1.0");
  check(processFence.kind === "SYSTEMD_USER_SERVICE_CGROUP_V2");
  check(canonicalLinuxPath(processFence.systemd_run_path));
  check(canonicalLinuxPath(processFence.systemctl_path));
  check(isClosedWsl2ToolVersion("SYSTEMD", processFence.systemd_run_version));
  check(isClosedWsl2ToolVersion("SYSTEMD", processFence.systemctl_version));
  check(sha(processFence.systemd_run_sha256));
  check(sha(processFence.systemctl_sha256));
  check(processFence.cgroup_mount === "/sys/fs/cgroup");
  check(/^\/run\/user\/[0-9]+$/u.test(processFence.user_runtime_dir));
  check(linux.xdg_runtime_dir === processFence.user_runtime_dir);
  check(/^lattice-wsl2-[a-f0-9]{16}$/u.test(processFence.unit_prefix));
  validateTool(processFence.supervisor_bootstrap_node, "NODE");
  check(processFence.supervisor_bootstrap_node.path === "/usr/bin/node");
  validateTool(processFence.immutable_probe_lsattr, "LSATTR");
  check(processFence.immutable_probe_lsattr.path === "/usr/bin/lsattr");
  validateTool(processFence.noninteractive_root_probe, "SUDO");
  check(processFence.noninteractive_root_probe.path === "/usr/bin/sudo");
  check(processFence.identity_digest === processFenceAuthorityIdentity(untrusted));

  const toolchain = untrusted.verification_toolchain;
  exactKeys(toolchain, [
    "schema", "task_ref", "task_root", "isolation_root", "owner_uid", "home_dir", "temp_dir",
    "npm_cache", "cargo_home", "cargo_target_dir", "cargo_host", "npm", "cargo", "rustc",
    "rustdoc", "sandbox", "sandbox_helper", "identity_digest",
  ]);
  check(toolchain.schema === "lattice.wsl2-verification-toolchain/1.0");
  check(HEX_64.test(toolchain.task_ref));
  check(canonicalLinuxPath(toolchain.task_root, { home: true }));
  check(descendant(toolchain.task_root, toolchain.isolation_root));
  check(Number.isSafeInteger(toolchain.owner_uid) && toolchain.owner_uid > 0);
  for (const isolated of [
    toolchain.home_dir, toolchain.temp_dir, toolchain.npm_cache, toolchain.cargo_home,
    toolchain.cargo_target_dir,
  ]) check(descendant(toolchain.isolation_root, isolated));
  check(/^[A-Za-z0-9._-]+$/u.test(toolchain.cargo_host));
  for (const [tool, versionKind] of [
    [toolchain.npm, "NPM"], [toolchain.cargo, "CARGO"], [toolchain.rustc, "RUSTC"],
    [toolchain.rustdoc, "RUSTDOC"], [toolchain.sandbox, "CODEX_CLI"],
  ]) {
    validateTool(tool, versionKind);
    check(descendant(toolchain.task_root, tool.path));
  }
  validateTool(toolchain.sandbox_helper, "BWRAP");
  check(descendant(toolchain.task_root, linux.node_path));
  check(nodeVersionAtLeast(linux.node_version, [24, 15, 0]));
  check(descendant(toolchain.task_root, linux.launcher_path));
  check(descendant(toolchain.task_root, linux.supervisor_path));
  check(toolchain.sandbox.path === linux.launcher_path
    && toolchain.sandbox.version === linux.launcher_version
    && toolchain.sandbox.sha256 === linux.launcher_sha256);
  check(toolchain.sandbox_helper.path === "/usr/bin/bwrap");
  check(descendant(toolchain.task_root, linux.cwd));
  check(linux.cwd.startsWith(`${toolchain.task_root}/managed-worktrees/`));
  check(linux.codex_home === `${toolchain.task_root}/codex-home`);
  check(toolchain.identity_digest === verificationToolchainIdentity(untrusted));

  const snapshot = untrusted.immutable_snapshot;
  exactKeys(snapshot, [
    "schema", "task_root_path", "task_root_device", "task_root_inode",
    "task_root_owner_uid", "task_root_owner_gid", "task_root_mode", "task_root_immutable",
    "trees", "snapshot_digest",
  ]);
  check(snapshot.schema === WSL2_IMMUTABLE_SNAPSHOT_SCHEMA
    && snapshot.task_root_path === toolchain.task_root
    && /^[1-9][0-9]*$/u.test(snapshot.task_root_device)
    && /^[1-9][0-9]*$/u.test(snapshot.task_root_inode)
    && snapshot.task_root_owner_uid === 0 && snapshot.task_root_owner_gid === 0
    && snapshot.task_root_mode === "0555" && snapshot.task_root_immutable === true);
  exactKeys(snapshot.trees, WSL2_IMMUTABLE_TREE_NAMES);
  for (const treeName of WSL2_IMMUTABLE_TREE_NAMES) {
    const tree = snapshot.trees[treeName];
    exactKeys(tree, ["root", "manifest_digest"]);
    check(descendant(toolchain.task_root, tree.root)
      && path.posix.dirname(tree.root) === toolchain.task_root
      && typed(tree.manifest_digest, "immutable-tree-manifest"));
  }
  check(new Set(WSL2_IMMUTABLE_TREE_NAMES.map((name) => snapshot.trees[name].root)).size
    === WSL2_IMMUTABLE_TREE_NAMES.length);
  const contains = (root, candidate) => candidate === root || candidate.startsWith(`${root}/`);
  check(WSL2_IMMUTABLE_TREE_NAMES.every((name, index) => WSL2_IMMUTABLE_TREE_NAMES
    .every((other, otherIndex) => index === otherIndex
      || (!contains(snapshot.trees[name].root, snapshot.trees[other].root)
        && !contains(snapshot.trees[other].root, snapshot.trees[name].root)))));
  check(linux.launcher_path === `${snapshot.trees.codex.root}/bin/codex`
    && contains(snapshot.trees.codex.root, toolchain.sandbox.path)
    && contains(snapshot.trees.supervisor_runtime.root, linux.supervisor_path)
    && contains(snapshot.trees.node.root, linux.node_path)
    && contains(snapshot.trees.node.root, toolchain.npm.path)
    && contains(snapshot.trees.rust.root, toolchain.cargo.path)
    && contains(snapshot.trees.rust.root, toolchain.rustc.path)
    && contains(snapshot.trees.rust.root, toolchain.rustdoc.path)
    && linux.keyring_daemon_path === `${snapshot.trees.keyring.root}/root/usr/bin/gnome-keyring-daemon`
    && linux.keyring_library_path === `${snapshot.trees.keyring.root}/packages`);
  check(snapshot.snapshot_digest === immutableSnapshotIdentity(untrusted));

  const sandboxPolicy = untrusted.sandbox_policy;
  exactKeys(sandboxPolicy, ["schema", "policy_digest"]);
  check(sandboxPolicy.schema === WSL2_SANDBOX_POLICY_SCHEMA
    && sandboxPolicy.policy_digest === sandboxPolicyIdentity(untrusted));

  const privilegeBoundary = untrusted.privilege_boundary;
  exactKeys(privilegeBoundary, [
    "schema", "effective_uid", "effective_gid", "effective_capabilities_digest",
    "noninteractive_root_unavailable", "boundary_digest",
  ]);
  check(privilegeBoundary.schema === WSL2_PRIVILEGE_BOUNDARY_SCHEMA
    && privilegeBoundary.effective_uid === toolchain.owner_uid
    && Number.isSafeInteger(privilegeBoundary.effective_gid) && privilegeBoundary.effective_gid > 0
    && typed(privilegeBoundary.effective_capabilities_digest, "linux-capabilities")
    && privilegeBoundary.noninteractive_root_unavailable === true
    && privilegeBoundary.boundary_digest === privilegeBoundaryIdentity(untrusted));

  const mapping = untrusted.path_mapping;
  exactKeys(mapping, ["windows_path", "linux_path", "digest"]);
  check(canonicalLinuxPath(mapping.linux_path, { home: true }) && mapping.linux_path === linux.cwd);
  check(windowsWslPathToLinux(mapping.windows_path, untrusted.distribution) === linux.cwd);
  check(mapping.digest === pathMappingIdentity(untrusted));
  check(untrusted.identity_digest === executionEnvironmentIdentity(untrusted));
  return structuredClone(untrusted);
}

export function validateWsl2ExecutionEnvironment(untrusted) {
  try {
    check(executionEnvironmentStringLeavesAreSecretFree(untrusted));
    return validateProduction(untrusted);
  } catch (error) {
    if (error?.code === "WSL2_EXECUTION_ENVIRONMENT_REJECTED") throw error;
    throw rejected();
  }
}

export function bindWsl2ExecutionWorktree(untrusted, windowsWorktreePath, observedRepoIdentity) {
  const descriptor = validateWsl2ExecutionEnvironment(untrusted);
  exactKeys(observedRepoIdentity, ["repository_identity", "head"]);
  check(typed(observedRepoIdentity.repository_identity, "repository") && HEX_40.test(observedRepoIdentity.head));
  const linuxPath = windowsWslPathToLinux(windowsWorktreePath, descriptor.distribution);
  check(descendant(descriptor.verification_toolchain.task_root, linuxPath));
  check(linuxPath.startsWith(`${descriptor.verification_toolchain.task_root}/managed-worktrees/`));
  descriptor.linux.cwd = linuxPath;
  descriptor.linux.repository_head = observedRepoIdentity.head;
  descriptor.linux.repository_identity = observedRepoIdentity.repository_identity;
  descriptor.path_mapping.windows_path = windowsWorktreePath;
  descriptor.path_mapping.linux_path = linuxPath;
  descriptor.path_mapping.digest = pathMappingIdentity(descriptor);
  descriptor.sandbox_policy.policy_digest = sandboxPolicyIdentity(descriptor);
  descriptor.identity_digest = executionEnvironmentIdentity(descriptor);
  return validateWsl2ExecutionEnvironment(descriptor);
}

function validatePreflightReceipt(environment, fence, receipt, code) {
  check(object(receipt), code);
  exactKeys(receipt, [
    "schema", "status", "task_ref", "attempt", "worktree_ref", "execution_environment_ref",
    "descriptor_digest", "distribution_identity_ref", "linux_cwd", "repository_head",
    "repository_identity", "codex_home_digest", "credential_authority_ref",
    "credential_seal_digest", "verification_toolchain_ref", "immutable_snapshot_ref",
    "sandbox_policy_ref", "privilege_boundary_ref", "process_fence", "isolation", "probes",
    "effect_counters", "provider_effect_count", "bounds", "timeout", "continuation",
    "connector_auth_ready", "receipt_digest",
  ]);
  check(receipt.schema === PREFLIGHT_SCHEMA && receipt.status === "PASS", code);
  check(receipt.task_ref === environment.verification_toolchain.task_ref, code);
  check(Number.isSafeInteger(receipt.attempt) && receipt.attempt >= 1
    && receipt.attempt <= MAX_WSL2_ATTEMPTS, code);
  check(/^worktree:sha256:[a-f0-9]{64}$/u.test(receipt.worktree_ref), code);
  check(object(receipt.process_fence) && receipt.process_fence.fence === fence && HEX_64.test(fence), code);
  check(receipt.process_fence.authority_ref === environment.process_fence.identity_digest, code);
  check(receipt.process_fence.cgroup_version === 2 && receipt.process_fence.delegated === false, code);
  check(receipt.process_fence.supervisor_zero_descendants === true, code);
  exactKeys(receipt.process_fence, [
    "fence", "authority_ref", "service_unit", "cgroup_path", "cgroup_version", "delegated",
    "boot_id_digest", "supervisor_zero_descendants", "outer_post_exit",
  ]);
  check(object(receipt.process_fence.outer_post_exit)
    && receipt.process_fence.outer_post_exit.active_state === "inactive"
    && receipt.process_fence.outer_post_exit.sub_state === "dead"
    && receipt.process_fence.outer_post_exit.delegate === "no"
    && ((receipt.process_fence.outer_post_exit.cgroup_exists === false
      && receipt.process_fence.outer_post_exit.populated === null)
      || (receipt.process_fence.outer_post_exit.cgroup_exists === true
        && receipt.process_fence.outer_post_exit.populated === 0)), code);
  check(receipt.execution_environment_ref === environment.identity_digest, code);
  check(receipt.descriptor_digest === environment.identity_digest, code);
  check(receipt.linux_cwd === environment.linux.cwd, code);
  check(receipt.repository_head === environment.linux.repository_head, code);
  check(receipt.repository_identity === environment.linux.repository_identity, code);
  check(receipt.credential_authority_ref === environment.credential_authority.authority_digest, code);
  check(receipt.codex_home_digest === codexHomeIdentity(environment), code);
  check(typed(receipt.credential_seal_digest, "credential-seal"), code);
  check(receipt.verification_toolchain_ref === environment.verification_toolchain.identity_digest, code);
  check(receipt.immutable_snapshot_ref === environment.immutable_snapshot.snapshot_digest
    && receipt.sandbox_policy_ref === environment.sandbox_policy.policy_digest
    && receipt.privilege_boundary_ref === environment.privilege_boundary.boundary_digest, code);
  validateWsl2ImmutableObservation(receipt.probes?.immutable, environment, code);
  check(receipt.provider_effect_count === 0, code);
  check(object(receipt.effect_counters)
    && receipt.effect_counters.thread_start === 0
    && receipt.effect_counters.turn_start === 0
    && receipt.effect_counters.provider_effect_count === 0, code);
  check(typed(receipt.receipt_digest, "wsl2-preflight"), code);
  exactKeys(receipt.bounds, [
    "stdout_limit_bytes", "stderr_limit_bytes", "stdout_observed_bytes", "stderr_observed_bytes",
  ]);
  for (const key of ["stdout_limit_bytes", "stderr_limit_bytes"]) {
    check(Number.isSafeInteger(receipt.bounds[key]) && receipt.bounds[key] >= 1_024
      && receipt.bounds[key] <= 1_048_576, code);
  }
  check(Number.isSafeInteger(receipt.bounds.stdout_observed_bytes)
    && receipt.bounds.stdout_observed_bytes >= 0
    && receipt.bounds.stdout_observed_bytes <= receipt.bounds.stdout_limit_bytes, code);
  check(Number.isSafeInteger(receipt.bounds.stderr_observed_bytes)
    && receipt.bounds.stderr_observed_bytes >= 0
    && receipt.bounds.stderr_observed_bytes <= receipt.bounds.stderr_limit_bytes, code);
  exactKeys(receipt.timeout, ["timeout_ms", "timed_out", "interrupted"]);
  check(Number.isSafeInteger(receipt.timeout.timeout_ms) && receipt.timeout.timeout_ms >= 1_000
    && receipt.timeout.timeout_ms <= 300_000 && receipt.timeout.timed_out === false
    && receipt.timeout.interrupted === false, code);
  exactKeys(receipt.continuation, ["attempt", "retry_of", "reconnect_of"]);
  check(receipt.continuation.attempt === receipt.attempt
    && (receipt.continuation.retry_of === null || typed(receipt.continuation.retry_of))
    && (receipt.continuation.reconnect_of === null || typed(receipt.continuation.reconnect_of))
    && (receipt.continuation.retry_of === null
      || receipt.continuation.reconnect_of === null), code);
  check(receipt.connector_auth_ready === false, code);
  const subject = without(receipt, "receipt_digest");
  check(receipt.receipt_digest === typedDigest("wsl2-preflight", subject), code);
  return receipt;
}

function wslPrefix(environment) {
  return ["-d", environment.distribution, "--exec", "/usr/bin/env", "-i"];
}

function serviceUnit(environment, fence, role) {
  return `${environment.process_fence.unit_prefix}-${role.toLowerCase()}-${fence.slice(0, 12)}.service`;
}

function commonSupervisorArgs(environment, options, role, executable, version, digest, verifierTool = null) {
  const unit = serviceUnit(environment, options.fence, role);
  const supervisor = environment.linux.supervisor_path;
  const fixedPath = "/usr/bin:/bin";
  const childHome = role === "PROVIDER"
    ? environment.linux.codex_home
    : environment.verification_toolchain.home_dir;
  const verifierEnvironment = role === "CARGO"
    ? [
      `RUSTC=${environment.verification_toolchain.rustc.path}`,
      `RUSTDOC=${environment.verification_toolchain.rustdoc.path}`,
      "CARGO_NET_OFFLINE=true",
    ]
    : role === "NODE"
      ? ["npm_config_offline=true", "npm_config_audit=false", "npm_config_fund=false"]
      : [];
  const credentialEnvironment = role === "PROVIDER"
    ? [`CODEX_HOME=${environment.linux.codex_home}`]
    : [];
  const explicitEnvironment = [
    `HOME=${childHome}`,
    ...credentialEnvironment,
    `TMPDIR=${environment.verification_toolchain.temp_dir}`,
    `npm_config_cache=${environment.verification_toolchain.npm_cache}`,
    `CARGO_HOME=${environment.verification_toolchain.cargo_home}`,
    `CARGO_TARGET_DIR=${environment.verification_toolchain.cargo_target_dir}`,
    `XDG_RUNTIME_DIR=${environment.process_fence.user_runtime_dir}`,
    `PATH=${fixedPath}`,
    "LANG=C.UTF-8",
    "LC_ALL=C.UTF-8",
    ...verifierEnvironment,
  ];
  const nodeRuntime = ["PREFLIGHT", "NODE"].includes(role)
    ? {
      path: environment.linux.node_path,
      version: environment.linux.node_version,
      sha256: environment.linux.node_sha256,
    }
    : { path: "NONE", version: "NONE", sha256: "NONE" };
  const rustc = role === "CARGO"
    ? environment.verification_toolchain.rustc
    : { path: "NONE", version: "NONE", sha256: "NONE" };
  const rustdoc = role === "CARGO"
    ? environment.verification_toolchain.rustdoc
    : { path: "NONE", version: "NONE", sha256: "NONE" };
  const stdinArgs = role === "GIT"
    ? [
      "--stdin-byte-len", String(options.gitInvocation?.stdin?.byte_len ?? 0),
      "--stdin-sha256", options.gitInvocation?.stdin?.sha256
        ?? createHash("sha256").update(Buffer.alloc(0)).digest("hex"),
    ]
    : [];
  return {
    unit,
    args: [
      ...wslPrefix(environment),
      ...explicitEnvironment,
      environment.process_fence.systemd_run_path,
      "--user", "--wait", "--pipe", "--quiet",
      `--unit=${unit}`, "--property=Type=exec", "--property=KillMode=control-group",
      "--property=Delegate=no", "--property=TimeoutStopSec=5s",
      `--property=RuntimeMaxSec=${Math.ceil((options.timeoutMs + 30_000) / 1000)}`,
      `--setenv=HOME=${childHome}`,
      ...credentialEnvironment.map((entry) => `--setenv=${entry}`),
      `--setenv=TMPDIR=${environment.verification_toolchain.temp_dir}`,
      `--setenv=npm_config_cache=${environment.verification_toolchain.npm_cache}`,
      `--setenv=CARGO_HOME=${environment.verification_toolchain.cargo_home}`,
      `--setenv=CARGO_TARGET_DIR=${environment.verification_toolchain.cargo_target_dir}`,
      `--setenv=XDG_RUNTIME_DIR=${environment.process_fence.user_runtime_dir}`,
      `--setenv=PATH=${fixedPath}`,
      "--setenv=LANG=C.UTF-8",
      "--setenv=LC_ALL=C.UTF-8",
      ...verifierEnvironment.map((entry) => `--setenv=${entry}`),
      "/usr/bin/env", "-i", ...explicitEnvironment,
      environment.linux.dbus_run_session_path, "--", environment.process_fence.supervisor_bootstrap_node.path,
      "-e", SUPERVISOR_BOOTSTRAP_SOURCE, supervisor, environment.linux.supervisor_sha256,
      "--role", role,
      "--fence", options.fence,
      "--unit", unit,
      "--execution-environment-ref", environment.identity_digest,
      "--credential-authority-ref", environment.credential_authority.authority_digest,
      "--credential-seal-digest", options.preflightReceipt.credential_seal_digest,
      "--config-digest", environment.linux.config_digest,
      "--codex-home", environment.linux.codex_home,
      "--cwd", options.cwd,
      "--executable", executable,
      "--executable-version", version,
      "--executable-sha256", digest,
      "--verifier-tool", verifierTool?.path ?? "NONE",
      "--verifier-tool-version", verifierTool?.version ?? "NONE",
      "--verifier-tool-sha256", verifierTool?.sha256 ?? "NONE",
      "--node-runtime", nodeRuntime.path,
      "--node-runtime-version", nodeRuntime.version,
      "--node-runtime-sha256", nodeRuntime.sha256,
      "--rustc", rustc.path,
      "--rustc-version", rustc.version,
      "--rustc-sha256", rustc.sha256,
      "--rustdoc", rustdoc.path,
      "--rustdoc-version", rustdoc.version,
      "--rustdoc-sha256", rustdoc.sha256,
      "--keyring-daemon", environment.linux.keyring_daemon_path,
      "--keyring-daemon-sha256", environment.linux.keyring_daemon_sha256,
      "--keyring-library-path", environment.linux.keyring_library_path,
      "--keyring-library-manifest-digest", environment.linux.keyring_library_manifest_digest,
      "--sandbox-helper", environment.verification_toolchain.sandbox_helper.path,
      "--sandbox-helper-version", environment.verification_toolchain.sandbox_helper.version,
      "--sandbox-helper-sha256", environment.verification_toolchain.sandbox_helper.sha256,
      "--timeout-ms", String(options.timeoutMs),
      "--stdout-limit-bytes", String(options.stdoutLimitBytes),
      "--stderr-limit-bytes", String(options.stderrLimitBytes),
      "--attempt", String(options.attempt),
      "--retry-of", options.retryOf ?? "NONE",
      "--reconnect-of", options.reconnectOf ?? "NONE",
      ...stdinArgs,
      "--",
    ],
  };
}

function validateAttemptOptions(environment, options, code, preflightFence = options.fence) {
  check(object(options), code);
  validatePreflightReceipt(environment, preflightFence, options.preflightReceipt, code);
  check(options.cwd === environment.linux.cwd, code);
  check(Number.isSafeInteger(options.timeoutMs) && options.timeoutMs >= 1_000 && options.timeoutMs <= 300_000, code);
  for (const bound of [options.stdoutLimitBytes, options.stderrLimitBytes]) {
    check(Number.isSafeInteger(bound) && bound >= 1_024 && bound <= 1_048_576, code);
  }
  check(Number.isSafeInteger(options.attempt) && options.attempt >= 1
    && options.attempt <= MAX_WSL2_ATTEMPTS, code);
  check(options.retryOf === null || typed(options.retryOf), code);
  check(options.reconnectOf === null || typed(options.reconnectOf), code);
  check(options.retryOf === null || options.reconnectOf === null, code);
}

export function buildWsl2CodexLaunch(untrusted, options = {}) {
  let environment;
  try {
    environment = validateWsl2ExecutionEnvironment(untrusted);
    const normalized = {
      ...options,
      cwd: environment.linux.cwd,
      timeoutMs: options.timeoutMs ?? 300_000,
      stdoutLimitBytes: options.stdoutLimitBytes ?? 1_048_576,
      stderrLimitBytes: options.stderrLimitBytes ?? 1_048_576,
      attempt: options.attempt ?? 1,
      retryOf: options.retryOf ?? null,
      reconnectOf: options.reconnectOf ?? null,
    };
    validateAttemptOptions(environment, normalized, "WSL2_PRODUCTION_PREFLIGHT_REQUIRED");
    check(normalized.attempt === normalized.preflightReceipt.attempt
      && normalized.retryOf === normalized.preflightReceipt.continuation.retry_of
      && normalized.reconnectOf === normalized.preflightReceipt.continuation.reconnect_of,
    "WSL2_PRODUCTION_PREFLIGHT_REQUIRED");
    check((normalized.retryOf === null || typed(normalized.retryOf, "attempt-receipt"))
      && (normalized.reconnectOf === null || typed(normalized.reconnectOf, "attempt-receipt")),
    "WSL2_PRODUCTION_PREFLIGHT_REQUIRED");
    check((normalized.attempt === 1 && normalized.retryOf === null)
      || (normalized.attempt > 1
        && (normalized.retryOf !== null) !== (normalized.reconnectOf !== null)),
    "WSL2_PRODUCTION_PREFLIGHT_REQUIRED");
    const common = commonSupervisorArgs(
      environment, normalized, "PROVIDER", environment.linux.launcher_path,
      environment.linux.launcher_version, environment.linux.launcher_sha256,
    );
    const args = [...common.args, "app-server"];
    return Object.freeze({
      command: environment.gateway.windows_path,
      args: Object.freeze(args),
      processFence: normalized.fence,
      serviceUnit: common.unit,
      gracefulClose: true,
      postExitProbe: Object.freeze({
        distribution: environment.distribution,
        unit: common.unit,
        process_fence: normalized.fence,
        authority_ref: environment.process_fence.identity_digest,
        systemctl_path: environment.process_fence.systemctl_path,
        cgroup_mount: environment.process_fence.cgroup_mount,
        user_runtime_dir: environment.process_fence.user_runtime_dir,
      }),
      codexIdentity: Object.freeze({
        schema: "lattice.wsl2-codex-launch/1.1",
        execution_environment_ref: environment.identity_digest,
        credential_authority_ref: environment.credential_authority.authority_digest,
        codex_home_digest: codexHomeIdentity(environment),
        credential_seal_digest: normalized.preflightReceipt.credential_seal_digest,
        process_fence_authority_ref: environment.process_fence.identity_digest,
        process_fence: normalized.fence,
        linux_cwd: environment.linux.cwd,
        repository_head: environment.linux.repository_head,
        provider_effects_authorized: false,
      }),
    });
  } catch (error) {
    if (error?.code === "WSL2_PRODUCTION_PREFLIGHT_REQUIRED") throw error;
    throw rejected("WSL2_PRODUCTION_PREFLIGHT_REQUIRED");
  }
}

const CLOSED_VERIFIER_ARGS = Object.freeze({
  NODE: Object.freeze(["run", "verify", "--offline", "--no-audit", "--no-fund"]),
  CARGO: Object.freeze(["test", "--locked", "--offline"]),
});

export function buildWsl2VerifierLaunch(untrusted, options = {}) {
  try {
    const environment = validateWsl2ExecutionEnvironment(untrusted);
    check(["NODE", "CARGO", "GIT"].includes(options.role), "WSL2_VERIFIER_LAUNCH_REJECTED");
    check(HEX_64.test(options.fence) && HEX_64.test(options.preflightFence), "WSL2_VERIFIER_LAUNCH_REJECTED");
    check(Array.isArray(options.args) && (options.role === "GIT"
      ? options.args.length >= 12 && options.args.length <= 256
      : canonicalJson(options.args) === canonicalJson(CLOSED_VERIFIER_ARGS[options.role])),
    "WSL2_VERIFIER_LAUNCH_REJECTED");
    validateAttemptOptions(environment, options, "WSL2_VERIFIER_LAUNCH_REJECTED", options.preflightFence);
    check(options.attempt === options.preflightReceipt.attempt
      && options.retryOf === options.preflightReceipt.continuation.retry_of
      && options.reconnectOf === options.preflightReceipt.continuation.reconnect_of,
    "WSL2_VERIFIER_LAUNCH_REJECTED");
    check((options.retryOf === null || options.reconnectOf === null)
      && (options.retryOf === null || typed(options.retryOf, "verifier-receipt"))
      && (options.reconnectOf === null || typed(options.reconnectOf, "verifier-receipt")),
    "WSL2_VERIFIER_LAUNCH_REJECTED");
    let gitInvocation = null;
    if (options.role === "GIT") {
      exactKeys(options.gitInvocation, [
        "schema", "sequence", "environment", "args", "stdin", "invocation_digest", "process_fence",
      ]);
      const invocation = options.gitInvocation;
      check(invocation.schema === "lattice.wsl2-git-invocation/1.0"
        && Number.isSafeInteger(invocation.sequence) && invocation.sequence >= 1 && invocation.sequence <= 10_000
        && object(invocation.environment) && canonicalJson(invocation.args) === canonicalJson(options.args)
        && typed(invocation.invocation_digest, "wsl2-git-invocation")
        && invocation.process_fence === options.fence,
      "WSL2_VERIFIER_LAUNCH_REJECTED");
      if (invocation.stdin !== null) {
        exactKeys(invocation.stdin, ["byte_len", "sha256", "base64"]);
        const decoded = Buffer.from(invocation.stdin.base64, "base64");
        check(Number.isSafeInteger(invocation.stdin.byte_len) && invocation.stdin.byte_len >= 0
          && invocation.stdin.byte_len <= 32 * 1_048_576 && decoded.length === invocation.stdin.byte_len
          && decoded.toString("base64") === invocation.stdin.base64 && HEX_64.test(invocation.stdin.sha256)
          && createHash("sha256").update(decoded).digest("hex") === invocation.stdin.sha256,
        "WSL2_VERIFIER_LAUNCH_REJECTED");
      }
      const subject = {
        schema: invocation.schema,
        sequence: invocation.sequence,
        environment: invocation.environment,
        args: invocation.args,
        stdin: invocation.stdin,
      };
      check(typedDigest("wsl2-git-invocation", subject) === invocation.invocation_digest,
        "WSL2_VERIFIER_LAUNCH_REJECTED");
      gitInvocation = invocation;
    } else {
      check(options.gitInvocation === undefined, "WSL2_VERIFIER_LAUNCH_REJECTED");
    }
    const tool = options.role === "NODE"
      ? environment.verification_toolchain.npm
      : options.role === "CARGO"
        ? environment.verification_toolchain.cargo
        : {
          path: environment.linux.git_path,
          version: environment.linux.git_version,
          sha256: environment.linux.git_sha256,
        };
    const sandbox = environment.verification_toolchain.sandbox;
    const common = commonSupervisorArgs(
      environment, options, options.role, sandbox.path, sandbox.version, sandbox.sha256, tool,
    );
    const gitEnvironment = gitInvocation?.environment ?? null;
    const guardedGitEnvironment = options.role === "GIT"
      && gitEnvironment.GIT_OBJECT_DIRECTORY !== undefined
      && gitEnvironment.GIT_INDEX_FILE !== undefined;
    const gitCommand = options.role === "GIT" ? options.args[11] : null;
    const gitWritableRoots = options.role !== "GIT" ? null : [
      gitEnvironment.HOME,
      gitEnvironment.TMPDIR,
      ...(["hash-object", "commit-tree"].includes(gitCommand)
        ? [gitEnvironment.GIT_OBJECT_DIRECTORY] : []),
      ...(["read-tree", "update-index", "write-tree"].includes(gitCommand)
        ? [path.posix.dirname(gitEnvironment.GIT_INDEX_FILE)] : []),
    ].filter((value, index, values) => value !== undefined && values.indexOf(value) === index);
    const sandboxState = buildWsl2SandboxState(environment, {
      role: options.role,
      cwd: options.cwd,
      writableRoots: options.role === "NODE"
        ? [environment.verification_toolchain.home_dir, environment.verification_toolchain.temp_dir,
          environment.verification_toolchain.npm_cache]
        : options.role === "CARGO"
          ? [environment.verification_toolchain.home_dir, environment.verification_toolchain.temp_dir,
            environment.verification_toolchain.cargo_home, environment.verification_toolchain.cargo_target_dir]
          : guardedGitEnvironment ? gitWritableRoots : [gitEnvironment.HOME, gitEnvironment.TMPDIR],
      deniedRoots: [environment.linux.codex_home],
    });
    const fixedPath = [
      environment.linux.node_path.slice(0, environment.linux.node_path.lastIndexOf("/")),
      environment.verification_toolchain.cargo.path.slice(0,
        environment.verification_toolchain.cargo.path.lastIndexOf("/")),
      "/usr/bin", "/bin",
    ].join(":");
    const sandboxEnvironment = options.role === "GIT" ? [
      ...Object.entries(gitEnvironment).sort(([left], [right]) => left.localeCompare(right, "en"))
        .map(([key, value]) => `${key}=${value}`),
      "PATH=/usr/bin:/bin", "LANG=C.UTF-8", "LC_ALL=C.UTF-8",
    ] : [
      `HOME=${environment.verification_toolchain.home_dir}`,
      `TMPDIR=${environment.verification_toolchain.temp_dir}`,
      `npm_config_cache=${environment.verification_toolchain.npm_cache}`,
      `CARGO_HOME=${environment.verification_toolchain.cargo_home}`,
      `CARGO_TARGET_DIR=${environment.verification_toolchain.cargo_target_dir}`,
      `PATH=${fixedPath}`,
      "LANG=C.UTF-8", "LC_ALL=C.UTF-8",
      ...(options.role === "NODE"
        ? ["npm_config_offline=true", "npm_config_audit=false", "npm_config_fund=false"]
        : [
          `RUSTC=${environment.verification_toolchain.rustc.path}`,
          `RUSTDOC=${environment.verification_toolchain.rustdoc.path}`,
          "CARGO_NET_OFFLINE=true",
        ]),
    ];
    const sandboxArgs = [
      "sandbox", "--sandbox-state-json", canonicalJson(sandboxState),
      "--sandbox-state-disable-network", "--", "/usr/bin/env", "-i",
      ...sandboxEnvironment, tool.path, ...options.args,
    ];
    const args = [...common.args, ...sandboxArgs];
    const commandSubject = {
      role: options.role,
      executable: tool,
      sandbox: environment.verification_toolchain.sandbox,
      sandbox_state: sandboxState,
      sandbox_environment: sandboxEnvironment,
      args: options.args,
      cwd: options.cwd,
      process_fence: options.fence,
      service_unit: common.unit,
      execution_environment_ref: environment.identity_digest,
      credential_seal_digest: options.preflightReceipt.credential_seal_digest,
      supervisor_bootstrap_sha256: WSL2_SUPERVISOR_BOOTSTRAP_SHA256,
      timeout_ms: options.timeoutMs,
      stdout_limit_bytes: options.stdoutLimitBytes,
      stderr_limit_bytes: options.stderrLimitBytes,
      attempt: options.attempt,
      retry_of: options.retryOf,
      reconnect_of: options.reconnectOf,
      ...(gitInvocation === null ? {} : { git_invocation_digest: gitInvocation.invocation_digest }),
    };
    return Object.freeze({
      command: environment.gateway.windows_path,
      args: Object.freeze(args),
      processFence: options.fence,
      serviceUnit: common.unit,
      postExitProbe: Object.freeze({
        distribution: environment.distribution,
        unit: common.unit,
        process_fence: options.fence,
        authority_ref: environment.process_fence.identity_digest,
        systemctl_path: environment.process_fence.systemctl_path,
        cgroup_mount: environment.process_fence.cgroup_mount,
      }),
      verifierIdentity: Object.freeze({
        schema: "lattice.wsl2-verifier-launch/1.0",
        command_digest: typedDigest("wsl2-verifier-command", commandSubject),
        execution_environment_ref: environment.identity_digest,
        verification_toolchain_ref: environment.verification_toolchain.identity_digest,
        credential_seal_digest: options.preflightReceipt.credential_seal_digest,
        process_fence: options.fence,
        linux_cwd: options.cwd,
        repository_head: environment.linux.repository_head,
        provider_effect_count: 0,
      }),
    });
  } catch (error) {
    if (error?.code === "WSL2_VERIFIER_LAUNCH_REJECTED") throw error;
    throw rejected("WSL2_VERIFIER_LAUNCH_REJECTED");
  }
}

function validateLegacyFixture(untrusted) {
  check(object(untrusted) && untrusted.schema === LEGACY_SCHEMA && untrusted.kind === "WSL2_LINUX");
  check(object(untrusted.gateway) && typeof untrusted.gateway.windows_path === "string");
  check(object(untrusted.linux) && canonicalLinuxPath(untrusted.linux.cwd, { home: true }));
  check(untrusted.identity_digest === executionEnvironmentIdentity(untrusted));
  return structuredClone(untrusted);
}

/** Explicitly fixture-only compatibility path. Never call from production. */
export function buildLegacyWsl2CodexLaunchFixture(untrusted, options = {}) {
  const environment = validateLegacyFixture(untrusted);
  check(HEX_64.test(options.fence));
  return Object.freeze({
    command: environment.gateway.windows_path,
    args: Object.freeze([
      "-d", environment.distribution, "--exec", environment.linux.launcher_path, "app-server",
    ]),
    processFence: options.fence,
    fixtureOnly: true,
  });
}

export const WSL2_PRODUCTION_SCHEMA = PRODUCTION_SCHEMA;
export const WSL2_PREFLIGHT_SCHEMA = PREFLIGHT_SCHEMA;
