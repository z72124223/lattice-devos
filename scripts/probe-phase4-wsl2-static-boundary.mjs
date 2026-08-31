import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import path from "node:path";
import { Script } from "node:vm";

const WSL = String.raw`C:\Windows\System32\wsl.exe`;
const SCHEMA = "lattice.phase4-wsl2-static-boundary-probe/1.0";
const MAX_GATEWAY_OUTPUT_BYTES = 512 * 1024;

const canonical = (value) => Array.isArray(value)
  ? value.map(canonical)
  : value !== null && typeof value === "object"
    ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]))
    : value;

const canonicalJson = (value) => JSON.stringify(canonical(value));
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

function fail(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
}

function ensure(condition, code) {
  if (!condition) fail(code);
}

function exactKeys(value, expected, code) {
  ensure(value !== null && typeof value === "object" && !Array.isArray(value), code);
  const actual = Object.keys(value).sort();
  const sortedExpected = [...expected].sort();
  ensure(actual.length === sortedExpected.length
    && actual.every((key, index) => key === sortedExpected[index]), code);
}

function safeLinuxHomePath(value) {
  return typeof value === "string" && value.startsWith("/home/")
    && path.posix.normalize(value) === value && !value.includes("\\")
    && !value.includes("\0") && !/[\u0000-\u001f\u007f]/u.test(value);
}

function parseArguments(argv) {
  const keys = [
    "--distribution",
    "--task-root",
    "--codex-root",
    "--supervisor-runtime-root",
    "--node-root",
    "--rust-root",
    "--keyring-root",
    "--regular-target",
    "--mutable-isolation-dir",
    "--output",
    "--expected-uid",
  ];
  const allowed = new Set(keys);
  ensure(argv.length === keys.length * 2, "PHASE4_STATIC_BOUNDARY_ARGUMENT_REJECTED");
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    ensure(allowed.has(key) && !values.has(key) && typeof value === "string"
      && value.length > 0, "PHASE4_STATIC_BOUNDARY_ARGUMENT_REJECTED");
    values.set(key, value);
  }
  ensure(values.size === keys.length, "PHASE4_STATIC_BOUNDARY_ARGUMENT_REJECTED");

  const taskRoot = values.get("--task-root");
  const roots = {
    codex: values.get("--codex-root"),
    supervisor_runtime: values.get("--supervisor-runtime-root"),
    node: values.get("--node-root"),
    rust: values.get("--rust-root"),
    keyring: values.get("--keyring-root"),
  };
  const regularTarget = values.get("--regular-target");
  const mutableIsolationDir = values.get("--mutable-isolation-dir");
  const expectedUid = Number(values.get("--expected-uid"));
  const distribution = values.get("--distribution");
  const output = values.get("--output");

  ensure(typeof distribution === "string" && /^[A-Za-z0-9._-]{1,64}$/u.test(distribution)
    && safeLinuxHomePath(taskRoot)
    && taskRoot.split("/").filter(Boolean).length >= 3
    && Object.values(roots).every((root) => safeLinuxHomePath(root)
      && path.posix.dirname(root) === taskRoot)
    && new Set(Object.values(roots)).size === Object.keys(roots).length
    && safeLinuxHomePath(regularTarget)
    && Object.values(roots).some((root) => regularTarget.startsWith(`${root}/`))
    && safeLinuxHomePath(mutableIsolationDir)
    && mutableIsolationDir.startsWith(`${taskRoot}/`)
    && !Object.values(roots).some((root) => mutableIsolationDir === root
      || mutableIsolationDir.startsWith(`${root}/`))
    && safeLinuxHomePath(output)
    && path.posix.dirname(output) === `${mutableIsolationDir}/evidence`
    && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}\.json$/u.test(path.posix.basename(output))
    && Number.isSafeInteger(expectedUid) && expectedUid > 0,
  "PHASE4_STATIC_BOUNDARY_ARGUMENT_REJECTED");

  const home = `/${taskRoot.split("/").filter(Boolean).slice(0, 2).join("/")}`;
  return {
    output_windows_path: `\\\\wsl.localhost\\${distribution}${output.replaceAll("/", "\\")}`,
    probe: {
    distribution,
    expected_uid: expectedUid,
    home,
    mutable_isolation_dir: mutableIsolationDir,
    regular_target: regularTarget,
    roots,
    task_root: taskRoot,
    },
  };
}

function boundedBuffer(value) {
  if (Buffer.isBuffer(value)) return value;
  if (typeof value === "string") return Buffer.from(value, "utf8");
  return Buffer.alloc(0);
}

function failureReceipt(code, result = null) {
  const stdout = boundedBuffer(result?.stdout);
  const stderr = boundedBuffer(result?.stderr);
  return {
    schema: SCHEMA,
    status: "FAIL",
    code,
    gateway_exit: {
      exit_code: Number.isSafeInteger(result?.status) ? result.status : null,
      error_code: typeof result?.error?.code === "string" ? result.error.code : null,
      signal: typeof result?.signal === "string" ? result.signal : null,
      stderr_bytes: stderr.length,
      stderr_sha256: sha256(stderr),
      stdout_bytes: stdout.length,
      stdout_sha256: sha256(stdout),
    },
    effect_counters: {
      account_read: 0,
      provider_effect_count: 0,
      thread_start: 0,
      turn_start: 0,
    },
    provider_effect_count: 0,
  };
}

const WSL_OPERATION_SOURCE = String.raw`const fs = require("node:fs");
const config = JSON.parse(Buffer.from(process.argv[1], "base64").toString("utf8"));
const exact = ["cleanup_on_success", "destination", "kind", "mode", "restore_mode", "target"];
const actual = Object.keys(config).sort();
if (actual.length !== exact.length || !actual.every((key, index) => key === exact[index])) {
  process.exit(74);
}
let succeeded = false;
let cleanupSucceeded = null;
let caught = null;
let descriptor = null;
try {
  if (config.kind === "CREATE") {
    descriptor = fs.openSync(config.target,
      fs.constants.O_WRONLY | fs.constants.O_CREAT | fs.constants.O_EXCL | fs.constants.O_NOFOLLOW,
      0o600);
    fs.closeSync(descriptor);
    descriptor = null;
  } else if (config.kind === "OPEN_WRITE_NOFOLLOW") {
    descriptor = fs.openSync(config.target, fs.constants.O_WRONLY | fs.constants.O_NOFOLLOW);
    fs.closeSync(descriptor);
    descriptor = null;
  } else if (config.kind === "CHMOD") {
    fs.chmodSync(config.target, config.mode);
  } else if (config.kind === "RENAME") {
    fs.renameSync(config.target, config.destination);
  } else if (config.kind === "UNLINK") {
    fs.unlinkSync(config.target);
  } else {
    throw Object.assign(new Error("OPERATION_KIND_REJECTED"), { code: "OPERATION_KIND_REJECTED" });
  }
  succeeded = true;
} catch (error) {
  caught = error;
} finally {
  if (descriptor !== null) { try { fs.closeSync(descriptor); } catch {} }
}
if (succeeded && config.cleanup_on_success) {
  try {
    if (config.kind === "CREATE") fs.unlinkSync(config.target);
    else if (config.kind === "CHMOD") fs.chmodSync(config.target, config.restore_mode);
    else if (config.kind === "RENAME") fs.renameSync(config.destination, config.target);
    cleanupSucceeded = true;
  } catch { cleanupSucceeded = false; }
}
process.stdout.write(JSON.stringify({
  cleanup_succeeded: cleanupSucceeded,
  error_code: caught && typeof caught.code === "string" ? caught.code : null,
  error_errno: caught && Number.isSafeInteger(caught.errno) ? caught.errno : null,
  error_syscall: caught && typeof caught.syscall === "string" ? caught.syscall : null,
  operation_succeeded: succeeded,
  schema: "lattice.phase4-static-boundary-operation/1.0",
}) + "\n");
process.exitCode = succeeded ? 0 : 73;`;
const WSL_OPERATION_SOURCE_BASE64 = Buffer.from(WSL_OPERATION_SOURCE, "utf8").toString("base64");
new Script(WSL_OPERATION_SOURCE, { filename: "phase4-static-boundary-operation.cjs" });

const WSL_PROBE_SOURCE = String.raw`
const fs = require("node:fs");
const cp = require("node:child_process");
const crypto = require("node:crypto");
const path = require("node:path");

const SCHEMA = "lattice.phase4-wsl2-static-boundary-probe/1.0";
const OPERATION_SCHEMA = "lattice.phase4-static-boundary-operation/1.0";
const MAX_OPERATION_OUTPUT_BYTES = 4096;
const MAX_DIRECTORY_ENTRIES = 200000;
const MAX_REGULAR_TARGET_BYTES = 536870912;
const OPERATION_TIMEOUT_MS = 5000;
const DENIAL_EXIT_CODE = 73;
const EXPECTED_DENIAL_CODES = new Set(["EACCES", "EPERM", "EROFS"]);

const canonical = (value) => Array.isArray(value)
  ? value.map(canonical)
  : value !== null && typeof value === "object"
    ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]))
    : value;
const canonicalJson = (value) => JSON.stringify(canonical(value));
const sha256 = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");
const typedDigest = (domain, value) => domain + ":sha256:"
  + sha256(Buffer.from(canonicalJson(value), "utf8"));
const safeCode = (error, fallback) => typeof error?.code === "string"
  && /^[A-Z0-9_]{1,96}$/.test(error.code) ? error.code : fallback;
const fail = (code) => { const error = new Error(code); error.code = code; throw error; };
const ensure = (condition, code) => { if (!condition) fail(code); };
const exactKeys = (value, expected, code) => {
  ensure(value !== null && typeof value === "object" && !Array.isArray(value), code);
  const actual = Object.keys(value).sort();
  const sortedExpected = [...expected].sort();
  ensure(actual.length === sortedExpected.length
    && actual.every((key, index) => key === sortedExpected[index]), code);
};
const safePath = (value) => typeof value === "string" && value.startsWith("/home/")
  && path.posix.normalize(value) === value && !value.includes("\\")
  && !value.includes("\0") && !/[\u0000-\u001f\u007f]/.test(value);
const modeString = (metadata) => (Number(metadata.mode) & 0o7777).toString(8).padStart(4, "0");
const fileKind = (metadata) => metadata.isDirectory() && !metadata.isSymbolicLink()
  ? "DIRECTORY" : metadata.isFile() ? "REGULAR_FILE"
    : metadata.isSymbolicLink() ? "SYMLINK" : "OTHER";
const absent = (target) => {
  try { fs.lstatSync(target); return false; }
  catch (error) { if (error?.code === "ENOENT") return true; throw error; }
};
const OPERATION_SOURCE = Buffer.from("${WSL_OPERATION_SOURCE_BASE64}", "base64").toString("utf8");

function readFileDigest(target, expectedMetadata) {
  ensure(Number(expectedMetadata.size) <= MAX_REGULAR_TARGET_BYTES,
    "STATIC_BOUNDARY_REGULAR_TARGET_TOO_LARGE");
  const descriptor = fs.openSync(target, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    const observed = fs.fstatSync(descriptor, { bigint: true });
    ensure(observed.isFile() && observed.dev === expectedMetadata.dev
      && observed.ino === expectedMetadata.ino && observed.size === expectedMetadata.size,
    "STATIC_BOUNDARY_REGULAR_TARGET_CHANGED_DURING_READ");
    const hash = crypto.createHash("sha256");
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0n;
    while (offset < observed.size) {
      const remaining = observed.size - offset;
      const requested = Number(remaining < BigInt(buffer.length) ? remaining : BigInt(buffer.length));
      const count = fs.readSync(descriptor, buffer, 0, requested, Number(offset));
      ensure(count > 0, "STATIC_BOUNDARY_REGULAR_TARGET_SHORT_READ");
      hash.update(buffer.subarray(0, count));
      offset += BigInt(count);
    }
    return hash.digest("hex");
  } finally { fs.closeSync(descriptor); }
}

function directoryEntries(target) {
  const entries = fs.readdirSync(target, { withFileTypes: true });
  ensure(entries.length <= MAX_DIRECTORY_ENTRIES, "STATIC_BOUNDARY_DIRECTORY_BOUND_EXCEEDED");
  entries.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0);
  const records = entries.map((entry) => {
    ensure(entry.name.length > 0 && entry.name !== "." && entry.name !== ".."
      && !entry.name.includes("/") && !entry.name.includes("\0"),
    "STATIC_BOUNDARY_DIRECTORY_ENTRY_REJECTED");
    const metadata = fs.lstatSync(target + "/" + entry.name, { bigint: true });
    return {
      device: String(metadata.dev),
      inode: String(metadata.ino),
      kind: fileKind(metadata),
      name: entry.name,
    };
  });
  return {
    entries_digest: typedDigest("directory-entries", records),
    entry_count: records.length,
  };
}

function observe(target, requiredKind) {
  ensure(safePath(target) && fs.realpathSync(target) === target,
    "STATIC_BOUNDARY_TARGET_REALPATH_REJECTED");
  const metadata = fs.lstatSync(target, { bigint: true });
  const kind = fileKind(metadata);
  ensure(kind === requiredKind && !metadata.isSymbolicLink(),
    "STATIC_BOUNDARY_TARGET_KIND_REJECTED");
  const identity = {
    ctime_ns: String(metadata.ctimeNs),
    device: String(metadata.dev),
    inode: String(metadata.ino),
    kind,
    mode: modeString(metadata),
    mtime_ns: String(metadata.mtimeNs),
    nlink: Number(metadata.nlink),
    owner_gid: Number(metadata.gid),
    owner_uid: Number(metadata.uid),
    size: Number(metadata.size),
  };
  const contentSha256 = kind === "REGULAR_FILE" ? readFileDigest(target, metadata) : null;
  const entries = kind === "DIRECTORY" ? directoryEntries(target) : null;
  const result = {
    content_sha256: contentSha256,
    directory_entries: entries,
    identity,
    identity_digest: typedDigest("filesystem-identity", identity),
    path: target,
  };
  result.observation_digest = typedDigest("filesystem-observation", result);
  return result;
}

function parseOperationOutput(stdout) {
  const lines = stdout.toString("utf8").replaceAll("\r", "").split("\n").filter(Boolean);
  if (lines.length !== 1) return null;
  try {
    const value = JSON.parse(lines[0]);
    exactKeys(value, [
      "cleanup_succeeded", "error_code", "error_errno", "error_syscall",
      "operation_succeeded", "schema",
    ], "STATIC_BOUNDARY_OPERATION_OUTPUT_REJECTED");
    ensure(value.schema === OPERATION_SCHEMA && typeof value.operation_succeeded === "boolean"
      && (value.cleanup_succeeded === null || typeof value.cleanup_succeeded === "boolean")
      && (value.error_code === null || typeof value.error_code === "string")
      && (value.error_errno === null || Number.isSafeInteger(value.error_errno))
      && (value.error_syscall === null || typeof value.error_syscall === "string"),
    "STATIC_BOUNDARY_OPERATION_OUTPUT_REJECTED");
    return value;
  } catch { return null; }
}

function runOperation(sequence, label, operation, expected) {
  exactKeys(operation, [
    "cleanup_on_success", "destination", "kind", "mode", "restore_mode", "target",
  ], "STATIC_BOUNDARY_OPERATION_REQUEST_REJECTED");
  const encoded = Buffer.from(canonicalJson(operation), "utf8").toString("base64");
  const result = cp.spawnSync(process.execPath, ["-e", OPERATION_SOURCE, encoded], {
    encoding: "buffer",
    env: {
      HOME: config.home,
      LANG: "C.UTF-8",
      LC_ALL: "C.UTF-8",
      PATH: "/usr/bin:/bin",
      TMPDIR: config.mutable_isolation_dir,
    },
    maxBuffer: MAX_OPERATION_OUTPUT_BYTES,
    timeout: OPERATION_TIMEOUT_MS,
  });
  const stdout = Buffer.isBuffer(result.stdout) ? result.stdout : Buffer.alloc(0);
  const stderr = Buffer.isBuffer(result.stderr) ? result.stderr : Buffer.alloc(0);
  const parsed = stdout.length <= MAX_OPERATION_OUTPUT_BYTES
    && stderr.length <= MAX_OPERATION_OUTPUT_BYTES ? parseOperationOutput(stdout) : null;
  const common = {
    error_code: parsed?.error_code
      ?? (typeof result.error?.code === "string" ? result.error.code : null),
    error_errno: parsed?.error_errno ?? null,
    error_syscall: parsed?.error_syscall ?? null,
    exit_code: Number.isSafeInteger(result.status) ? result.status : null,
    label,
    sequence,
    signal: typeof result.signal === "string" ? result.signal : null,
    stderr_bytes: stderr.length,
    stderr_sha256: sha256(stderr),
    stdout_bytes: stdout.length,
    stdout_sha256: sha256(stdout),
  };
  let passed = false;
  if (expected === "DENIED") {
    passed = !result.error && result.signal === null && result.status === DENIAL_EXIT_CODE
      && stderr.length === 0 && parsed?.operation_succeeded === false
      && parsed.cleanup_succeeded === null && EXPECTED_DENIAL_CODES.has(parsed.error_code);
  } else {
    passed = !result.error && result.signal === null && result.status === 0
      && stderr.length === 0 && parsed?.operation_succeeded === true
      && parsed.error_code === null && parsed.error_errno === null
      && parsed.error_syscall === null && parsed.cleanup_succeeded === null;
  }
  return { ...common, expected, passed };
}

function staticScopeObservations() {
  return {
    regular_target: observe(config.regular_target, "REGULAR_FILE"),
    roots: Object.fromEntries(Object.entries(config.roots)
      .map(([name, target]) => [name, observe(target, "DIRECTORY")])),
    task_root: observe(config.task_root, "DIRECTORY"),
  };
}

function immutableAttributeEvidence(target) {
  const result = cp.spawnSync("/usr/bin/lsattr", ["-d", target], {
    encoding: "buffer",
    env: { HOME: config.home, LANG: "C.UTF-8", LC_ALL: "C.UTF-8", PATH: "/usr/bin:/bin" },
    maxBuffer: 16384,
    timeout: OPERATION_TIMEOUT_MS,
  });
  const stdout = Buffer.isBuffer(result.stdout) ? result.stdout : Buffer.alloc(0);
  const stderr = Buffer.isBuffer(result.stderr) ? result.stderr : Buffer.alloc(0);
  const lines = stdout.toString("utf8").replaceAll("\r", "").split("\n").filter(Boolean);
  const attributes = lines.length === 1 ? /^([A-Za-z-]+)\s/.exec(lines[0])?.[1] : null;
  const immutable = !result.error && result.signal === null && result.status === 0
    && stderr.length === 0 && lines.length === 1 && lines[0].endsWith(" " + target)
    && typeof attributes === "string" && attributes.includes("i");
  return {
    error_code: typeof result.error?.code === "string" ? result.error.code : null,
    exit_code: Number.isSafeInteger(result.status) ? result.status : null,
    immutable,
    signal: typeof result.signal === "string" ? result.signal : null,
    stderr_bytes: stderr.length,
    stderr_sha256: sha256(stderr),
    stdout_bytes: stdout.length,
    stdout_sha256: sha256(stdout),
  };
}

function sameObservation(left, right) {
  return canonicalJson(left) === canonicalJson(right);
}

function scopeUnchanged(before, after) {
  return sameObservation(before.task_root, after.task_root)
    && sameObservation(before.regular_target, after.regular_target)
    && Object.keys(before.roots).every((name) => sameObservation(before.roots[name], after.roots[name]));
}

function validateConfig(value) {
  exactKeys(value, [
    "distribution", "expected_uid", "home", "mutable_isolation_dir", "regular_target",
    "roots", "task_root",
  ], "STATIC_BOUNDARY_CONFIG_REJECTED");
  exactKeys(value.roots, ["codex", "keyring", "node", "rust", "supervisor_runtime"],
    "STATIC_BOUNDARY_CONFIG_REJECTED");
  ensure(typeof value.distribution === "string" && /^[A-Za-z0-9._-]{1,64}$/.test(value.distribution)
    && Number.isSafeInteger(value.expected_uid) && value.expected_uid > 0
    && safePath(value.task_root) && safePath(value.home)
    && value.task_root.startsWith(value.home + "/")
    && Object.values(value.roots).every((root) => safePath(root)
      && path.posix.dirname(root) === value.task_root)
    && new Set(Object.values(value.roots)).size === Object.keys(value.roots).length
    && safePath(value.regular_target)
    && Object.values(value.roots).some((root) => value.regular_target.startsWith(root + "/"))
    && safePath(value.mutable_isolation_dir)
    && value.mutable_isolation_dir.startsWith(value.task_root + "/")
    && !Object.values(value.roots).some((root) => value.mutable_isolation_dir === root
      || value.mutable_isolation_dir.startsWith(root + "/")),
  "STATIC_BOUNDARY_CONFIG_REJECTED");
  return value;
}

const config = validateConfig(JSON.parse(process.argv[1]));

function main() {
  ensure(process.platform === "linux" && process.getuid() === config.expected_uid
    && process.getuid() !== 0 && process.geteuid() === config.expected_uid,
  "STATIC_BOUNDARY_TASK_USER_REJECTED");
  ensure(Number.isInteger(fs.constants.O_NOFOLLOW) && fs.constants.O_NOFOLLOW > 0,
    "STATIC_BOUNDARY_NOFOLLOW_UNAVAILABLE");
  const groups = process.getgroups();
  ensure(!groups.includes(0), "STATIC_BOUNDARY_ROOT_GROUP_REJECTED");
  const taskMetadata = fs.lstatSync(config.task_root, { bigint: true });
  ensure(taskMetadata.isDirectory() && !taskMetadata.isSymbolicLink()
    && Number(taskMetadata.uid) === 0 && Number(taskMetadata.gid) === 0
    && (Number(taskMetadata.mode) & 0o022) === 0
    && fs.realpathSync(config.task_root) === config.task_root,
  "STATIC_BOUNDARY_TASK_ROOT_REJECTED");
  for (const root of Object.values(config.roots)) {
    const metadata = fs.lstatSync(root, { bigint: true });
    ensure(metadata.isDirectory() && !metadata.isSymbolicLink()
      && Number(metadata.uid) === 0 && Number(metadata.gid) === 0
      && (Number(metadata.mode) & 0o022) === 0 && fs.realpathSync(root) === root,
    "STATIC_BOUNDARY_TREE_ROOT_REJECTED");
  }
  const targetMetadata = fs.lstatSync(config.regular_target, { bigint: true });
  const parentMetadata = fs.lstatSync(path.posix.dirname(config.regular_target), { bigint: true });
  ensure(targetMetadata.isFile() && !targetMetadata.isSymbolicLink()
    && Number(targetMetadata.uid) === 0 && Number(targetMetadata.gid) === 0
    && (Number(targetMetadata.mode) & 0o022) === 0
    && parentMetadata.isDirectory() && !parentMetadata.isSymbolicLink()
    && Number(parentMetadata.uid) === 0 && Number(parentMetadata.gid) === 0
    && (Number(parentMetadata.mode) & 0o022) === 0
    && fs.realpathSync(config.regular_target) === config.regular_target
    && fs.realpathSync(path.posix.dirname(config.regular_target))
      === path.posix.dirname(config.regular_target),
  "STATIC_BOUNDARY_REGULAR_TARGET_REJECTED");
  const mutableMetadata = fs.lstatSync(config.mutable_isolation_dir, { bigint: true });
  ensure(mutableMetadata.isDirectory() && !mutableMetadata.isSymbolicLink()
    && Number(mutableMetadata.uid) === config.expected_uid
    && (Number(mutableMetadata.mode) & 0o300) === 0o300
    && fs.realpathSync(config.mutable_isolation_dir) === config.mutable_isolation_dir,
  "STATIC_BOUNDARY_MUTABLE_DIRECTORY_REJECTED");

  const status = fs.readFileSync("/proc/self/status", "utf8");
  const capMatch = /(?:^|\n)CapEff:\s*([A-Fa-f0-9]+)(?:\n|$)/.exec(status);
  ensure(capMatch && /^0+$/.test(capMatch[1]), "STATIC_BOUNDARY_CAPABILITIES_REJECTED");
  const capEff = capMatch[1].toLowerCase();
  const sudo = cp.spawnSync("/usr/bin/sudo", ["-n", "true"], {
    encoding: "buffer",
    env: { HOME: config.home, LANG: "C.UTF-8", LC_ALL: "C.UTF-8", PATH: "/usr/bin:/bin" },
    maxBuffer: 16384,
    timeout: OPERATION_TIMEOUT_MS,
  });
  const sudoStdout = Buffer.isBuffer(sudo.stdout) ? sudo.stdout : Buffer.alloc(0);
  const sudoStderr = Buffer.isBuffer(sudo.stderr) ? sudo.stderr : Buffer.alloc(0);
  const sudoEvidence = {
    error_code: typeof sudo.error?.code === "string" ? sudo.error.code : null,
    exit_code: Number.isSafeInteger(sudo.status) ? sudo.status : null,
    noninteractive_root_unavailable: !sudo.error && sudo.signal === null
      && Number.isSafeInteger(sudo.status) && sudo.status !== 0,
    signal: typeof sudo.signal === "string" ? sudo.signal : null,
    stderr_bytes: sudoStderr.length,
    stderr_sha256: sha256(sudoStderr),
    stdout_bytes: sudoStdout.length,
    stdout_sha256: sha256(sudoStdout),
  };
  ensure(sudoEvidence.noninteractive_root_unavailable
    && sudoStdout.length <= 16384 && sudoStderr.length <= 16384,
  "STATIC_BOUNDARY_SUDO_REJECTED");

  const immutableAttributes = {
    regular_parent: immutableAttributeEvidence(path.posix.dirname(config.regular_target)),
    regular_target: immutableAttributeEvidence(config.regular_target),
    roots: Object.fromEntries(Object.entries(config.roots)
      .map(([name, root]) => [name, immutableAttributeEvidence(root)])),
    task_root: immutableAttributeEvidence(config.task_root),
  };
  ensure(immutableAttributes.task_root.immutable
    && Object.values(immutableAttributes.roots).every((evidence) => evidence.immutable)
    && (immutableAttributes.regular_parent.immutable
      || immutableAttributes.regular_target.immutable),
  "STATIC_BOUNDARY_IMMUTABLE_ATTRIBUTE_REJECTED");

  const nonce = crypto.randomBytes(16).toString("hex");
  const nonceDigest = typedDigest("probe-nonce", nonce);
  const staticBefore = staticScopeObservations();
  let sequence = 0;
  const negativeProbes = [];
  const negativeCandidates = [];
  const createLocations = {
    task_root: config.task_root,
    ...Object.fromEntries(Object.entries(config.roots)
      .map(([name, root]) => ["tree_" + name, root])),
  };
  for (const [label, directory] of Object.entries(createLocations)) {
    const candidate = directory + "/.lattice-static-boundary-" + nonce + "-" + sequence;
    ensure(absent(candidate), "STATIC_BOUNDARY_SENTINEL_COLLISION");
    negativeCandidates.push({ label: "create_" + label, path: candidate });
    negativeProbes.push(runOperation(++sequence, "create_" + label, {
      cleanup_on_success: true,
      destination: null,
      kind: "CREATE",
      mode: null,
      restore_mode: null,
      target: candidate,
    }, "DENIED"));
  }
  negativeProbes.push(runOperation(++sequence, "regular_open_write_nofollow", {
    cleanup_on_success: false,
    destination: null,
    kind: "OPEN_WRITE_NOFOLLOW",
    mode: null,
    restore_mode: null,
    target: config.regular_target,
  }, "DENIED"));
  negativeProbes.push(runOperation(++sequence, "regular_chmod", {
    cleanup_on_success: true,
    destination: null,
    kind: "CHMOD",
    mode: 0o600,
    restore_mode: Number(targetMetadata.mode) & 0o7777,
    target: config.regular_target,
  }, "DENIED"));
  const renameDestination = path.posix.dirname(config.regular_target)
    + "/.lattice-static-boundary-rename-" + nonce;
  ensure(absent(renameDestination), "STATIC_BOUNDARY_SENTINEL_COLLISION");
  negativeCandidates.push({ label: "regular_rename_destination", path: renameDestination });
  negativeProbes.push(runOperation(++sequence, "regular_rename", {
    cleanup_on_success: true,
    destination: renameDestination,
    kind: "RENAME",
    mode: null,
    restore_mode: null,
    target: config.regular_target,
  }, "DENIED"));
  negativeProbes.push(runOperation(++sequence, "regular_unlink", {
    cleanup_on_success: false,
    destination: null,
    kind: "UNLINK",
    mode: null,
    restore_mode: null,
    target: config.regular_target,
  }, "DENIED"));

  const mutableBefore = directoryEntries(config.mutable_isolation_dir);
  const mutableA = config.mutable_isolation_dir + "/.lattice-mutable-boundary-" + nonce + "-a";
  const mutableB = config.mutable_isolation_dir + "/.lattice-mutable-boundary-" + nonce + "-b";
  ensure(absent(mutableA) && absent(mutableB), "STATIC_BOUNDARY_SENTINEL_COLLISION");
  const mutableOperations = [];
  let createdState = null;
  let renamedState = null;
  try {
    mutableOperations.push(runOperation(++sequence, "mutable_create", {
      cleanup_on_success: false,
      destination: null,
      kind: "CREATE",
      mode: null,
      restore_mode: null,
      target: mutableA,
    }, "SUCCEEDED"));
    if (!absent(mutableA)) createdState = observe(mutableA, "REGULAR_FILE");
    mutableOperations.push(runOperation(++sequence, "mutable_rename", {
      cleanup_on_success: false,
      destination: mutableB,
      kind: "RENAME",
      mode: null,
      restore_mode: null,
      target: mutableA,
    }, "SUCCEEDED"));
    if (!absent(mutableB)) renamedState = observe(mutableB, "REGULAR_FILE");
    mutableOperations.push(runOperation(++sequence, "mutable_unlink", {
      cleanup_on_success: false,
      destination: null,
      kind: "UNLINK",
      mode: null,
      restore_mode: null,
      target: mutableB,
    }, "SUCCEEDED"));
  } finally {
    for (const candidate of [mutableA, mutableB]) {
      try { fs.unlinkSync(candidate); } catch (error) { if (error?.code !== "ENOENT") throw error; }
    }
  }
  const mutableFinalAbsence = { source: absent(mutableA), destination: absent(mutableB) };
  const mutableAfter = directoryEntries(config.mutable_isolation_dir);
  const mutablePassed = mutableOperations.length === 3
    && mutableOperations.every((probe) => probe.passed)
    && createdState !== null && renamedState !== null
    && createdState.content_sha256 === renamedState.content_sha256
    && createdState.identity.inode === renamedState.identity.inode
    && mutableFinalAbsence.source && mutableFinalAbsence.destination
    && canonicalJson(mutableBefore) === canonicalJson(mutableAfter);

  const staticAfter = staticScopeObservations();
  const negativeFinalAbsence = Object.fromEntries(negativeCandidates
    .map(({ label, path: candidate }) => [label, absent(candidate)]));
  const staticUnchanged = scopeUnchanged(staticBefore, staticAfter);
  const negativePassed = negativeProbes.every((probe) => probe.passed)
    && Object.values(negativeFinalAbsence).every(Boolean);
  const passed = negativePassed && staticUnchanged && mutablePassed
    && sudoEvidence.noninteractive_root_unavailable && /^0+$/.test(capEff);

  return {
    schema: SCHEMA,
    status: passed ? "PASS" : "FAIL",
    code: passed ? null : "STATIC_BOUNDARY_PROBE_REJECTED",
    bounds: {
      max_directory_entries: MAX_DIRECTORY_ENTRIES,
      max_operation_output_bytes: MAX_OPERATION_OUTPUT_BYTES,
      max_regular_target_bytes: MAX_REGULAR_TARGET_BYTES,
      operation_timeout_ms: OPERATION_TIMEOUT_MS,
    },
    distribution: config.distribution,
    effect_counters: {
      account_read: 0,
      provider_effect_count: 0,
      thread_start: 0,
      turn_start: 0,
    },
    mutable_probe: {
      created_state: createdState,
      directory_entries_after: mutableAfter,
      directory_entries_before: mutableBefore,
      final_absence: mutableFinalAbsence,
      operations: mutableOperations,
      passed: mutablePassed,
      renamed_state: renamedState,
      sentinel_identity: nonceDigest,
    },
    immutable_attributes: immutableAttributes,
    negative_probe_candidates_absent: negativeFinalAbsence,
    negative_probes: negativeProbes,
    privilege: {
      capabilities_empty: /^0+$/.test(capEff),
      effective_gid: process.getegid(),
      effective_groups: groups,
      effective_uid: process.geteuid(),
      proc_status_cap_eff: capEff,
      sudo_noninteractive: sudoEvidence,
    },
    provider_effect_count: 0,
    scope: {
      mutable_isolation_dir: config.mutable_isolation_dir,
      regular_target: config.regular_target,
      roots: config.roots,
      task_root: config.task_root,
    },
    static_post_state: staticAfter,
    static_pre_state: staticBefore,
    static_state_unchanged: staticUnchanged,
  };
}

let receipt;
try {
  receipt = main();
} catch (error) {
  receipt = {
    schema: SCHEMA,
    status: "FAIL",
    code: safeCode(error, "STATIC_BOUNDARY_PROBE_FAILED"),
    effect_counters: {
      account_read: 0,
      provider_effect_count: 0,
      thread_start: 0,
      turn_start: 0,
    },
    provider_effect_count: 0,
  };
}
process.stdout.write(canonicalJson(receipt) + "\n");
process.exitCode = receipt.status === "PASS" ? 0 : 1;
`;
new Script(WSL_PROBE_SOURCE, { filename: "phase4-static-boundary-probe.cjs" });

function main() {
  let config;
  try {
    config = parseArguments(process.argv.slice(2));
  } catch (error) {
    const receipt = failureReceipt(
      typeof error?.code === "string" ? error.code : "PHASE4_STATIC_BOUNDARY_ARGUMENT_REJECTED",
    );
    process.stdout.write(`${canonicalJson(receipt)}\n`);
    process.exitCode = 1;
    return;
  }

  const result = spawnSync(WSL, [
    "-d", config.probe.distribution, "--exec", "/usr/bin/env", "-i",
    `HOME=${config.probe.home}`, `TMPDIR=${config.probe.mutable_isolation_dir}`,
    "PATH=/usr/bin:/bin", "LANG=C.UTF-8", "LC_ALL=C.UTF-8",
    "/usr/bin/node", "-e", WSL_PROBE_SOURCE, canonicalJson(config.probe),
  ], {
    encoding: "buffer",
    windowsHide: true,
    timeout: 120_000,
    maxBuffer: MAX_GATEWAY_OUTPUT_BYTES,
  });
  const stdout = boundedBuffer(result.stdout);
  const stderr = boundedBuffer(result.stderr);
  let receipt;
  try {
    ensure(!result.error && result.signal === null
      && stdout.length <= MAX_GATEWAY_OUTPUT_BYTES && stderr.length <= MAX_GATEWAY_OUTPUT_BYTES
      && stderr.length === 0, "PHASE4_STATIC_BOUNDARY_GATEWAY_REJECTED");
    const lines = stdout.toString("utf8").replaceAll("\r", "").split("\n").filter(Boolean);
    ensure(lines.length === 1, "PHASE4_STATIC_BOUNDARY_OUTPUT_REJECTED");
    receipt = JSON.parse(lines[0]);
    ensure(receipt !== null && typeof receipt === "object" && !Array.isArray(receipt)
      && receipt.schema === SCHEMA && ["PASS", "FAIL"].includes(receipt.status)
      && receipt.provider_effect_count === 0
      && receipt.effect_counters?.provider_effect_count === 0
      && ((receipt.status === "PASS" && result.status === 0)
        || (receipt.status === "FAIL" && result.status !== 0)),
    "PHASE4_STATIC_BOUNDARY_OUTPUT_REJECTED");
  } catch (error) {
    receipt = failureReceipt(
      typeof error?.code === "string" ? error.code : "PHASE4_STATIC_BOUNDARY_OUTPUT_REJECTED",
      result,
    );
  }
  const rendered = `${canonicalJson(receipt)}\n`;
  try {
    writeFileSync(config.output_windows_path, rendered, { encoding: "utf8", flag: "wx", mode: 0o600 });
  } catch {
    receipt = failureReceipt("PHASE4_STATIC_BOUNDARY_EVIDENCE_WRITE_REJECTED");
  }
  process.stdout.write(`${canonicalJson(receipt)}\n`);
  process.exitCode = receipt.status === "PASS" ? 0 : 1;
}

main();
