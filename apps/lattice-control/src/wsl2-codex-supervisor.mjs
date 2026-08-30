import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { createWriteStream, watch as watchFs } from "node:fs";
import {
  constants as fsConstants,
  lstat,
  open,
  readFile,
  readlink,
  readdir,
  realpath,
  stat,
  unlink,
} from "node:fs/promises";
import process from "node:process";
import { Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const MARKER_SCHEMA = "lattice.wsl2-process-fence/1.1";
const RECEIPT_SCHEMA = "lattice.wsl2-subtree-exit/1.2";
const CGROUP_MOUNT = "/sys/fs/cgroup";
const MAX_GIT_STDIN_BYTES = 32 * 1_048_576;
const MAX_WSL2_ATTEMPTS = 3;
const KEYRING_UNLOCK_TIMEOUT_MS = 10_000;
const KEYRING_PRIVATE_LIBRARY_FILES = Object.freeze([
  "libgck-1.so.0.0.0",
  "libgcr-base-3.so.1.0.0",
]);
const KEYRING_PRIVATE_LIBRARY_LINKS = Object.freeze(new Map([
  ["libgck-1.so.0", "libgck-1.so.0.0.0"],
  ["libgcr-base-3.so.1", "libgcr-base-3.so.1.0.0"],
]));
const HEX_64 = /^[a-f0-9]{64}$/u;
const TYPED = /^[a-z0-9][a-z0-9-]*:sha256:[a-f0-9]{64}$/u;
const MANAGED_SHELL_ENVIRONMENT_KEYS = Object.freeze([
  "HOME", "PATH", "LANG", "LC_ALL", "TERM", "COLORTERM",
]);

function supervisorError(code) {
  const error = new Error(code);
  error.code = code;
  return error;
}

function ensure(condition, code) {
  if (!condition) throw supervisorError(code);
}

function object(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (object(value)) {
    return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]));
  }
  return value;
}

export function parseManagedShellEnvironmentPolicy(text) {
  ensure(typeof text === "string" && Buffer.byteLength(text, "utf8") <= 65_536,
    "WSL2_SHELL_ENVIRONMENT_POLICY_REJECTED");
  const lines = text.replaceAll("\r", "").split("\n");
  const start = lines.indexOf("[shell_environment_policy]");
  ensure(start >= 0 && lines.indexOf("[shell_environment_policy]", start + 1) === -1,
    "WSL2_SHELL_ENVIRONMENT_POLICY_REJECTED");
  const assignments = new Map();
  for (const line of lines.slice(start + 1)) {
    if (line.startsWith("[")) break;
    if (line.trim() === "") continue;
    const match = line.match(/^([a-z_]+)\s*=\s*(.+)$/u);
    ensure(match && !assignments.has(match[1]), "WSL2_SHELL_ENVIRONMENT_POLICY_REJECTED");
    assignments.set(match[1], match[2]);
  }
  ensure(assignments.size === 4
    && assignments.get("inherit") === '"all"'
    && assignments.get("ignore_default_excludes") === "false"
    && assignments.get("experimental_use_profile") === "false",
  "WSL2_SHELL_ENVIRONMENT_POLICY_REJECTED");
  let includeOnly;
  try { includeOnly = JSON.parse(assignments.get("include_only")); } catch { /* rejected below */ }
  ensure(Array.isArray(includeOnly)
    && JSON.stringify(includeOnly) === JSON.stringify(MANAGED_SHELL_ENVIRONMENT_KEYS),
  "WSL2_SHELL_ENVIRONMENT_POLICY_REJECTED");
  const probeInput = {
    HOME: "/home/lattice/probe", CODEX_HOME: "/home/lattice/codex-home",
    PATH: "/usr/bin:/bin", LANG: "C.UTF-8", LC_ALL: "C.UTF-8",
    TERM: "dumb", COLORTERM: "false", LATTICE_FAKE_API_TOKEN: "must-not-cross",
  };
  const effectiveKeys = includeOnly.filter((name) => Object.hasOwn(probeInput, name));
  const requiredKeysPresent = ["HOME", "PATH"].every((name) => effectiveKeys.includes(name));
  const forbiddenKeysAbsent = !effectiveKeys.includes("CODEX_HOME")
    && effectiveKeys.every((name) => !/(?:KEY|SECRET|TOKEN)/iu.test(name));
  ensure(requiredKeysPresent && forbiddenKeysAbsent,
    "WSL2_SHELL_ENVIRONMENT_POLICY_REJECTED");
  return Object.freeze({
    inherit: "all",
    ignore_default_excludes: false,
    include_only: Object.freeze([...includeOnly]),
    experimental_use_profile: false,
    set_keys: Object.freeze([]),
    probe_effective_keys: Object.freeze(effectiveKeys),
    required_keys_present: requiredKeysPresent,
    forbidden_keys_absent: forbiddenKeysAbsent,
  });
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

export function createExactStdinTracker(expectedBytes, expectedSha256) {
  ensure((expectedBytes === null && expectedSha256 === null)
    || (Number.isSafeInteger(expectedBytes) && expectedBytes >= 0
      && expectedBytes <= MAX_GIT_STDIN_BYTES && HEX_64.test(expectedSha256)),
  "WSL2_STDIN_IDENTITY_REJECTED");
  const hash = createHash("sha256");
  let observedBytes = 0;
  let writeError = false;
  let result = null;
  return Object.freeze({
    observe(chunk) {
      ensure(result === null && (Buffer.isBuffer(chunk) || chunk instanceof Uint8Array),
        "WSL2_STDIN_IDENTITY_REJECTED");
      const bytes = Buffer.from(chunk);
      observedBytes += bytes.length;
      hash.update(bytes);
      ensure(expectedBytes === null || observedBytes <= expectedBytes,
        "WSL2_STDIN_IDENTITY_REJECTED");
    },
    markWriteError() {
      ensure(result === null, "WSL2_STDIN_IDENTITY_REJECTED");
      writeError = true;
    },
    finish() {
      if (result !== null) return result;
      const observedSha256 = hash.digest("hex");
      result = Object.freeze({
        stdin_bytes: observedBytes,
        stdin_sha256: observedSha256,
        stdin_complete: !writeError && (expectedBytes === null
          || (observedBytes === expectedBytes && observedSha256 === expectedSha256)),
      });
      return result;
    },
  });
}

function typedDigest(domain, subject) {
  return `${domain}:sha256:${sha256(Buffer.from(JSON.stringify(canonical(subject)), "utf8"))}`;
}

function canonicalLinuxPath(value, home = false) {
  return typeof value === "string" && value.startsWith(home ? "/home/" : "/")
    && !value.includes("\\") && !value.includes("\0") && !value.includes("/../")
    && !value.endsWith("/..") && !value.includes("/./") && !value.endsWith("/.");
}

function parseProcStat(statLine) {
  ensure(typeof statLine === "string", "WSL2_PROC_STAT_REJECTED");
  const close = statLine.lastIndexOf(") ");
  const open = statLine.indexOf(" (");
  ensure(open >= 1 && close > open, "WSL2_PROC_STAT_REJECTED");
  const pid = statLine.slice(0, open);
  const tail = statLine.slice(close + 2).trim().split(/\s+/u);
  ensure(/^\d+$/u.test(pid) && /^\d+$/u.test(tail[2] ?? "") && /^\d+$/u.test(tail[19] ?? ""),
    "WSL2_PROC_STAT_REJECTED");
  return Object.freeze({ pid: Number(pid), processGroupId: Number(tail[2]), startTime: tail[19] });
}

const CHILD_ENVIRONMENT_KEYS = Object.freeze([
  "HOME", "CODEX_HOME", "TMPDIR", "npm_config_cache", "CARGO_HOME", "CARGO_TARGET_DIR",
  "XDG_RUNTIME_DIR", "PATH", "LANG", "LC_ALL", "RUSTC", "RUSTDOC",
  "CARGO_NET_OFFLINE", "npm_config_offline", "npm_config_audit", "npm_config_fund",
  "DBUS_SESSION_BUS_ADDRESS", "DBUS_SESSION_BUS_PID", "DBUS_SESSION_BUS_WINDOWID",
]);

export function explicitChildEnvironment(role) {
  return Object.fromEntries(CHILD_ENVIRONMENT_KEYS.flatMap((key) => (
    process.env[key] === undefined
      || (role !== "PROVIDER" && (key === "CODEX_HOME" || key.startsWith("DBUS_SESSION_BUS_")))
      ? [] : [[key, process.env[key]]]
  )));
}

export function parseSupervisorArgs(argv) {
  ensure(Array.isArray(argv), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  const separator = argv.indexOf("--");
  ensure(separator >= 0 && separator % 2 === 0 && separator < argv.length - 1,
    "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  const allowed = new Set([
    "role", "fence", "unit", "execution-environment-ref", "credential-authority-ref",
    "credential-seal-digest", "config-digest", "codex-home", "cwd", "executable",
    "executable-version", "executable-sha256", "verifier-tool", "verifier-tool-version",
    "verifier-tool-sha256", "node-runtime", "node-runtime-version", "node-runtime-sha256",
    "rustc", "rustc-version", "rustc-sha256", "rustdoc", "rustdoc-version", "rustdoc-sha256",
    "keyring-daemon", "keyring-daemon-sha256", "keyring-library-path",
    "keyring-library-manifest-digest", "sandbox-helper", "sandbox-helper-version",
    "sandbox-helper-sha256", "timeout-ms", "stdout-limit-bytes",
    "stderr-limit-bytes", "attempt", "retry-of", "reconnect-of",
    "stdin-byte-len", "stdin-sha256",
  ]);
  const options = Object.create(null);
  for (let index = 0; index < separator; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    ensure(typeof key === "string" && key.startsWith("--") && value !== undefined,
      "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
    const normalized = key.slice(2);
    ensure(allowed.has(normalized) && options[normalized] === undefined,
      "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
    options[normalized] = value;
  }
  const stdinFieldCount = ["stdin-byte-len", "stdin-sha256"]
    .filter((name) => options[name] !== undefined).length;
  ensure((stdinFieldCount === 0 || stdinFieldCount === 2)
    && Object.keys(options).length === allowed.size - 2 + stdinFieldCount,
  "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  ensure(["PROVIDER", "PREFLIGHT", "NODE", "CARGO", "GIT"].includes(options.role),
    "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  ensure(HEX_64.test(options.fence), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  ensure(new RegExp(`^lattice-wsl2-[a-f0-9]{16}-${options.role.toLowerCase()}-${options.fence.slice(0, 12)}\\.service$`, "u")
    .test(options.unit), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  ensure(/^execution-environment:sha256:[a-f0-9]{64}$/u.test(options["execution-environment-ref"]),
    "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  ensure(/^wsl2-credential-authority:sha256:[a-f0-9]{64}$/u.test(options["credential-authority-ref"]),
    "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  ensure(/^credential-seal:sha256:[a-f0-9]{64}$/u.test(options["credential-seal-digest"]),
    "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  ensure(/^codex-config:sha256:[a-f0-9]{64}$/u.test(options["config-digest"]),
    "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  for (const path of [options["codex-home"], options.cwd]) {
    ensure(canonicalLinuxPath(path, true), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  }
  for (const path of [
    options.executable, options["keyring-daemon"], options["keyring-library-path"],
    options["sandbox-helper"],
  ]) {
    ensure(canonicalLinuxPath(path), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  }
  ensure(/^keyring-library-manifest:sha256:[a-f0-9]{64}$/u.test(options["keyring-library-manifest-digest"]),
    "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  ensure(HEX_64.test(options["keyring-daemon-sha256"]), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  ensure(typeof options["sandbox-helper-version"] === "string"
    && options["sandbox-helper-version"].length > 0 && HEX_64.test(options["sandbox-helper-sha256"]),
  "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  ensure(HEX_64.test(options["executable-sha256"]), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  const verifierRole = options.role === "NODE" || options.role === "CARGO" || options.role === "GIT";
  if (verifierRole) {
    ensure(canonicalLinuxPath(options["verifier-tool"]) && options["verifier-tool-version"] !== "NONE"
      && HEX_64.test(options["verifier-tool-sha256"]), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  } else {
    ensure(options["verifier-tool"] === "NONE" && options["verifier-tool-version"] === "NONE"
      && options["verifier-tool-sha256"] === "NONE", "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  }
  const roleSpecificTools = [
    [["PREFLIGHT", "NODE"], "node-runtime", "node-runtime-version", "node-runtime-sha256"],
    [["CARGO"], "rustc", "rustc-version", "rustc-sha256"],
    [["CARGO"], "rustdoc", "rustdoc-version", "rustdoc-sha256"],
  ];
  for (const [roles, pathName, versionName, digestName] of roleSpecificTools) {
    if (roles.includes(options.role)) {
      ensure(canonicalLinuxPath(options[pathName]) && typeof options[versionName] === "string"
        && options[versionName].length > 0 && options[versionName] !== "NONE"
        && HEX_64.test(options[digestName]), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
    } else {
      ensure(options[pathName] === "NONE" && options[versionName] === "NONE"
        && options[digestName] === "NONE", "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
    }
  }
  if (options.role === "GIT") {
    ensure(stdinFieldCount === 2 && /^\d+$/u.test(options["stdin-byte-len"])
      && HEX_64.test(options["stdin-sha256"]), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
    options["stdin-byte-len"] = Number(options["stdin-byte-len"]);
    ensure(Number.isSafeInteger(options["stdin-byte-len"])
      && options["stdin-byte-len"] >= 0 && options["stdin-byte-len"] <= MAX_GIT_STDIN_BYTES,
    "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  } else {
    ensure(stdinFieldCount === 0, "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
    options["stdin-byte-len"] = options.role === "PROVIDER" ? null : 0;
    options["stdin-sha256"] = options.role === "PROVIDER" ? null : sha256(Buffer.alloc(0));
  }
  for (const [name, minimum, maximum] of [
    ["timeout-ms", 1_000, 300_000], ["stdout-limit-bytes", 1_024, 1_048_576],
    ["stderr-limit-bytes", 1_024, 1_048_576], ["attempt", 1, MAX_WSL2_ATTEMPTS],
  ]) {
    ensure(/^\d+$/u.test(options[name]), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
    options[name] = Number(options[name]);
    ensure(Number.isSafeInteger(options[name]) && options[name] >= minimum && options[name] <= maximum,
      "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  }
  for (const name of ["retry-of", "reconnect-of"]) {
    ensure(options[name] === "NONE" || TYPED.test(options[name]), "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
    if (options[name] === "NONE") options[name] = null;
  }
  ensure(options["retry-of"] === null || options["reconnect-of"] === null,
  "WSL2_SUPERVISOR_ARGUMENTS_REJECTED");
  return Object.freeze({ options: Object.freeze(options), commandArgs: Object.freeze(argv.slice(separator + 1)) });
}

export function parseUnifiedCgroup(content, expectedUnit, expectedUid) {
  ensure(typeof content === "string" && typeof expectedUnit === "string"
    && /^[A-Za-z0-9_.@:-]+\.service$/u.test(expectedUnit)
    && Number.isSafeInteger(expectedUid) && expectedUid > 0,
  "WSL2_CGROUP_V2_FENCE_REJECTED");
  const lines = content.trim().split("\n").filter(Boolean);
  ensure(lines.length === 1 && lines[0].startsWith("0::/"), "WSL2_CGROUP_V2_FENCE_REJECTED");
  const cgroupPath = lines[0].slice(3);
  const expectedPath = `/user.slice/user-${expectedUid}.slice/user@${expectedUid}.service/app.slice/${expectedUnit}`;
  ensure(cgroupPath === expectedPath, "WSL2_CGROUP_V2_FENCE_REJECTED");
  return cgroupPath;
}

export function credentialSealIdentity(authorityRef, facts) {
  ensure(/^wsl2-credential-authority:sha256:[a-f0-9]{64}$/u.test(authorityRef)
    && object(facts) && HEX_64.test(facts.config_sha256) && object(facts.config_identity)
    && object(facts.shell_environment_policy),
  "WSL2_CREDENTIAL_SEAL_REJECTED");
  return typedDigest("credential-seal", {
    authority_ref: authorityRef,
    config_sha256: facts.config_sha256,
    config_identity: facts.config_identity,
    keyring_only: facts.keyring_only,
    auth_json_absent: facts.auth_json_absent,
    shell_environment_policy: facts.shell_environment_policy,
  });
}

async function fileSha256(file) {
  return sha256(await readFile(file));
}

function executableIdentity(metadata) {
  return Object.freeze({
    device: String(metadata.dev),
    inode: String(metadata.ino),
    owner_uid: Number(metadata.uid),
    mode: Number(metadata.mode & 0o7777n),
    size: Number(metadata.size),
  });
}

function sameExecutableIdentity(left, right) {
  return left.device === right.device && left.inode === right.inode
    && left.owner_uid === right.owner_uid && left.mode === right.mode && left.size === right.size;
}

async function regularFileHandleSha256(handle) {
  return sha256(await readFile(`/proc/self/fd/${handle.fd}`));
}

/**
 * Opens one Linux regular file without following its final target and binds
 * the exact identity plus bytes to a handle that remains live across every
 * child using it. Symlinked inputs are resolved once, then the resolved file is
 * opened with O_NOFOLLOW.
 */
export async function openRegularFileIdentitySeal(
  file,
  expectedSha256,
  code = "WSL2_REGULAR_FILE_IDENTITY_REJECTED",
  { requireExecutable = false } = {},
) {
  let handle;
  try {
    ensure(process.platform === "linux" && canonicalLinuxPath(file)
      && HEX_64.test(expectedSha256), code);
    const resolvedPath = await realpath(file);
    ensure(canonicalLinuxPath(resolvedPath), code);
    handle = await open(resolvedPath, fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0));
    const [handleStat, pathStat, digest] = await Promise.all([
      handle.stat({ bigint: true }),
      lstat(resolvedPath, { bigint: true }),
      regularFileHandleSha256(handle),
    ]);
    const identity = executableIdentity(handleStat);
    const observedPathIdentity = executableIdentity(pathStat);
    const ownerUid = process.getuid?.();
    ensure(handleStat.isFile() && pathStat.isFile() && !pathStat.isSymbolicLink()
      && sameExecutableIdentity(identity, observedPathIdentity)
      && Number.isSafeInteger(ownerUid) && ownerUid > 0
      && (identity.owner_uid === 0 || identity.owner_uid === ownerUid)
      && (!requireExecutable || (identity.mode & 0o111) !== 0)
      && (identity.mode & 0o022) === 0
      && identity.size > 0 && digest === expectedSha256, code);
    return Object.freeze({
      handle,
      path: file,
      resolvedPath,
      identity,
      sha256: digest,
      code,
      requireExecutable,
    });
  } catch (error) {
    await handle?.close().catch(() => {});
    if (error?.code === code) throw error;
    throw supervisorError(code);
  }
}

export async function verifyRegularFileIdentitySeal(seal, {
  requirePath = false,
  verifyDigest = true,
} = {}) {
  try {
    ensure(object(seal) && typeof seal.handle?.fd === "number" && HEX_64.test(seal.sha256), seal.code);
    const metadata = await seal.handle.stat({ bigint: true });
    ensure(metadata.isFile() && sameExecutableIdentity(executableIdentity(metadata), seal.identity), seal.code);
    ensure(!seal.requireExecutable || (seal.identity.mode & 0o111) !== 0, seal.code);
    if (verifyDigest) ensure(await regularFileHandleSha256(seal.handle) === seal.sha256, seal.code);
    if (requirePath) {
      const resolvedPath = await realpath(seal.path);
      const pathStat = await lstat(resolvedPath, { bigint: true });
      ensure(resolvedPath === seal.resolvedPath && pathStat.isFile() && !pathStat.isSymbolicLink()
        && sameExecutableIdentity(executableIdentity(pathStat), seal.identity), seal.code);
    }
  } catch (error) {
    if (error?.code === seal?.code) throw error;
    throw supervisorError(seal?.code ?? "WSL2_EXECUTABLE_IDENTITY_REJECTED");
  }
}

export async function openExecutableIdentitySeal(
  executable,
  expectedSha256,
  code = "WSL2_EXECUTABLE_IDENTITY_REJECTED",
) {
  return openRegularFileIdentitySeal(executable, expectedSha256, code, { requireExecutable: true });
}

const verifyExecutableIdentitySeal = verifyRegularFileIdentitySeal;

export async function closeExecutableIdentitySeal(seal) {
  await seal?.handle?.close();
}

export async function runSealedExecutableOutput(
  seal,
  args,
  env,
  code = "WSL2_EXECUTABLE_VERSION_REJECTED",
) {
  ensure(Array.isArray(args) && args.every((argument) => typeof argument === "string")
    && object(env), code);
  await verifyExecutableIdentitySeal(seal);
  const output = await new Promise((resolve, reject) => {
    const child = spawn("/proc/self/fd/3", args, {
      env,
      stdio: ["ignore", "pipe", "ignore", seal.handle.fd],
    });
    const chunks = [];
    let length = 0;
    let settled = false;
    const finish = (operation) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      operation();
    };
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      finish(() => reject(supervisorError(code)));
    }, 10_000);
    child.stdout.on("data", (chunk) => {
      length += chunk.length;
      if (length <= 8_192) chunks.push(chunk);
    });
    child.once("error", () => finish(() => reject(supervisorError(code))));
    child.once("exit", (exitCode) => finish(() => {
      if (exitCode !== 0 || length > 8_192) reject(supervisorError(code));
      else resolve(Buffer.concat(chunks).toString("utf8"));
    }));
  });
  await verifyExecutableIdentitySeal(seal);
  return output;
}

export async function runSealedNodeScriptOutput(
  nodeSeal,
  scriptSeal,
  args,
  env,
  code = "WSL2_NODE_SCRIPT_VERSION_REJECTED",
) {
  ensure(Array.isArray(args) && args.every((argument) => typeof argument === "string")
    && object(env), code);
  await Promise.all([
    verifyExecutableIdentitySeal(nodeSeal),
    verifyRegularFileIdentitySeal(scriptSeal),
  ]);
  const output = await new Promise((resolve, reject) => {
    const child = spawn("/proc/self/fd/3", ["/proc/self/fd/4", ...args], {
      env,
      stdio: ["ignore", "pipe", "ignore", nodeSeal.handle.fd, scriptSeal.handle.fd],
    });
    const chunks = [];
    let length = 0;
    let settled = false;
    const finish = (operation) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      operation();
    };
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      finish(() => reject(supervisorError(code)));
    }, 10_000);
    child.stdout.on("data", (chunk) => {
      length += chunk.length;
      if (length <= 8_192) chunks.push(chunk);
    });
    child.once("error", () => finish(() => reject(supervisorError(code))));
    child.once("exit", (exitCode) => finish(() => {
      if (exitCode !== 0 || length > 8_192) reject(supervisorError(code));
      else resolve(Buffer.concat(chunks).toString("utf8"));
    }));
  });
  await Promise.all([
    verifyExecutableIdentitySeal(nodeSeal),
    verifyRegularFileIdentitySeal(scriptSeal),
  ]);
  return output;
}

async function boundedVersion(seal, args, env, code) {
  const output = await runSealedExecutableOutput(seal, args, env, code);
  return output.replaceAll("\r", "").split("\n")[0].trimEnd();
}

async function observeKeyringLibraryManifest(root) {
  const ownerUid = process.getuid?.();
  ensure(Number.isSafeInteger(ownerUid) && ownerUid > 0, "WSL2_KEYRING_LIBRARY_REJECTED");
  ensure(await realpath(root) === root, "WSL2_KEYRING_LIBRARY_REJECTED");
  const records = [];
  const walk = async (current, relative) => {
    const metadata = await lstat(current);
    // Credential runtime binaries and libraries are part of the immutable
    // task snapshot. They must be root-owned; the unprivileged execution UID
    // may consume their already-open descriptors but cannot replace them.
    ensure(metadata.uid === 0, "WSL2_KEYRING_LIBRARY_REJECTED");
    const mode = metadata.mode & 0o7777;
    if (metadata.isDirectory()) {
      ensure(!metadata.isSymbolicLink() && (mode & 0o022) === 0, "WSL2_KEYRING_LIBRARY_REJECTED");
      records.push({ path: relative, kind: "DIRECTORY", mode, owner_uid: metadata.uid });
      const entries = await readdir(current, { withFileTypes: true });
      entries.sort((left, right) => left.name.localeCompare(right.name, "en"));
      for (const entry of entries) {
        ensure(!entry.name.includes("/") && entry.name !== "." && entry.name !== "..",
          "WSL2_KEYRING_LIBRARY_REJECTED");
        await walk(`${current}/${entry.name}`, relative === "." ? entry.name : `${relative}/${entry.name}`);
      }
      return;
    }
    if (metadata.isFile()) {
      ensure((mode & 0o022) === 0, "WSL2_KEYRING_LIBRARY_REJECTED");
      records.push({
        path: relative,
        kind: "FILE",
        mode,
        owner_uid: metadata.uid,
        byte_len: metadata.size,
        sha256: await fileSha256(current),
      });
      return;
    }
    if (metadata.isSymbolicLink()) {
      const target = await readlink(current);
      ensure(/^[A-Za-z0-9._+-]{1,255}$/u.test(target), "WSL2_KEYRING_LIBRARY_REJECTED");
      const resolved = await realpath(current);
      ensure(resolved.startsWith(`${root}/`), "WSL2_KEYRING_LIBRARY_REJECTED");
      const targetMetadata = await stat(resolved);
      ensure(targetMetadata.isFile() && targetMetadata.uid === 0, "WSL2_KEYRING_LIBRARY_REJECTED");
      records.push({ path: relative, kind: "SYMLINK", owner_uid: metadata.uid, target });
      return;
    }
    throw supervisorError("WSL2_KEYRING_LIBRARY_REJECTED");
  };
  await walk(root, ".");
  const frozenRecords = Object.freeze(records.map((record) => Object.freeze(record)));
  return Object.freeze({
    digest: typedDigest("keyring-library-manifest", frozenRecords),
    records: frozenRecords,
  });
}

export function selectKeyringPrivateLibraryRecords(records) {
  ensure(Array.isArray(records), "WSL2_KEYRING_LIBRARY_REJECTED");
  const files = records.filter((record) => record?.kind === "FILE");
  ensure(files.length === KEYRING_PRIVATE_LIBRARY_FILES.length
    && new Set(records.map((record) => record?.path)).size === records.length,
  "WSL2_KEYRING_LIBRARY_REJECTED");
  const byPath = new Map(records.map((record) => [record?.path, record]));
  const selected = KEYRING_PRIVATE_LIBRARY_FILES.map((path) => {
    const record = byPath.get(path);
    ensure(record?.kind === "FILE" && Number.isSafeInteger(record.byte_len) && record.byte_len > 0
      && HEX_64.test(record.sha256), "WSL2_KEYRING_LIBRARY_REJECTED");
    return record;
  });
  ensure(files.every((record) => KEYRING_PRIVATE_LIBRARY_FILES.includes(record.path)),
    "WSL2_KEYRING_LIBRARY_REJECTED");
  for (const [path, target] of KEYRING_PRIVATE_LIBRARY_LINKS) {
    const record = byPath.get(path);
    ensure(record?.kind === "SYMLINK" && record.target === target,
      "WSL2_KEYRING_LIBRARY_REJECTED");
  }
  const symlinks = records.filter((record) => record?.kind === "SYMLINK");
  ensure(symlinks.length === KEYRING_PRIVATE_LIBRARY_LINKS.size,
    "WSL2_KEYRING_LIBRARY_REJECTED");
  return Object.freeze(selected);
}

async function observeCredential(options, handle) {
  const configPath = `${options["codex-home"]}/config.toml`;
  const authPath = `${options["codex-home"]}/auth.json`;
  const [fdStat, pathStat, bytes] = await Promise.all([
    handle.stat({ bigint: true }), lstat(configPath, { bigint: true }), readFile(`/proc/self/fd/${handle.fd}`),
  ]);
  let authAbsent = false;
  try { await lstat(authPath); } catch (error) { authAbsent = error?.code === "ENOENT"; }
  const text = bytes.toString("utf8");
  const facts = {
    config_sha256: sha256(bytes),
    config_identity: {
      device: String(fdStat.dev), inode: String(fdStat.ino), owner_uid: Number(fdStat.uid),
      mode: fdStat.mode.toString(8), size: Number(fdStat.size),
    },
    keyring_only: /^\s*cli_auth_credentials_store\s*=\s*["']keyring["']\s*(?:#.*)?$/mu.test(text)
      && !/^\s*cli_auth_credentials_store\s*=\s*["'](?:file|auto)["']/mu.test(text),
    auth_json_absent: authAbsent,
    shell_environment_policy: parseManagedShellEnvironmentPolicy(text),
  };
  ensure(fdStat.isFile() && pathStat.isFile() && !pathStat.isSymbolicLink(), "WSL2_CREDENTIAL_SEAL_REJECTED");
  ensure(fdStat.dev === pathStat.dev && fdStat.ino === pathStat.ino, "WSL2_CREDENTIAL_SEAL_REJECTED");
  ensure(Number(fdStat.uid) === process.getuid(), "WSL2_CREDENTIAL_SEAL_REJECTED");
  ensure((Number(fdStat.mode) & 0o077) === 0, "WSL2_CREDENTIAL_SEAL_REJECTED");
  ensure(facts.config_sha256 === options["config-digest"].slice(-64)
    && facts.keyring_only && facts.auth_json_absent, "WSL2_CREDENTIAL_SEAL_REJECTED");
  const seal = credentialSealIdentity(options["credential-authority-ref"], facts);
  ensure(seal === options["credential-seal-digest"], "WSL2_CREDENTIAL_SEAL_REJECTED");
  return { facts, seal };
}

export async function openCredentialSeal(options, {
  onDrift = () => {},
  afterInitialObservation = null,
} = {}) {
  const mutationWatch = startCredentialMutationWatch(options["codex-home"], onDrift);
  let handle;
  try {
    const homeRealpath = await realpath(options["codex-home"]);
    ensure(homeRealpath === options["codex-home"], "WSL2_CREDENTIAL_SEAL_REJECTED");
    const flags = fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0);
    handle = await open(`${options["codex-home"]}/config.toml`, flags);
    const initial = await observeCredential(options, handle);
    ensure(!mutationWatch.drifted, "WSL2_CREDENTIAL_SEAL_REJECTED");
    if (afterInitialObservation !== null) {
      ensure(typeof afterInitialObservation === "function", "WSL2_CREDENTIAL_SEAL_REJECTED");
      await afterInitialObservation();
    }
    await observeCredential(options, handle);
    ensure(!mutationWatch.drifted, "WSL2_CREDENTIAL_SEAL_REJECTED");
    return { handle, initial, mutationWatch };
  } catch (error) {
    mutationWatch.close();
    await handle?.close();
    throw error;
  }
}

/**
 * Node's Linux fs.watch backend is inotify. Watching both the containing
 * directory and the already-open config inode closes the create/delete and
 * rename windows that a periodic stat/hash check cannot observe. A missing
 * filename or watcher error is treated as a possible queue overflow and is
 * permanently fail-closed.
 */
export function startCredentialMutationWatch(codexHome, onDrift) {
  ensure(process.platform === "linux" && canonicalLinuxPath(codexHome, true)
    && typeof onDrift === "function", "WSL2_CREDENTIAL_WATCH_REJECTED");
  let drifted = false;
  let closed = false;
  const drift = (reason) => {
    if (closed || drifted) return;
    drifted = true;
    try {
      Promise.resolve(onDrift(reason)).catch(() => {});
    } catch {
      // The watch has already permanently recorded drift. The supervisor's
      // terminal validation remains fail-closed even if termination throws.
    }
  };
  const watchers = [];
  try {
    const separator = codexHome.lastIndexOf("/");
    ensure(separator > 0 && separator < codexHome.length - 1, "WSL2_CREDENTIAL_WATCH_REJECTED");
    const parent = codexHome.slice(0, separator);
    const homeName = codexHome.slice(separator + 1);
    const parentWatcher = watchFs(parent, { persistent: false }, (_eventType, filename) => {
      const name = filename === null ? null : filename.toString();
      if (name === null) drift("INOTIFY_PARENT_OVERFLOW");
      else if (name === homeName) drift("CODEX_HOME_REPLACED");
    });
    parentWatcher.on("error", () => drift("INOTIFY_PARENT_ERROR"));
    watchers.push(parentWatcher);
    const directoryWatcher = watchFs(codexHome, { persistent: false }, (_eventType, filename) => {
      const name = filename === null ? null : filename.toString();
      if (name === null) drift("INOTIFY_OVERFLOW");
      else if (name === "config.toml" || name === "auth.json") drift(`CODEX_HOME_${name}`);
    });
    directoryWatcher.on("error", () => drift("INOTIFY_DIRECTORY_ERROR"));
    watchers.push(directoryWatcher);
    const configWatcher = watchFs(`${codexHome}/config.toml`, { persistent: false }, () => {
      drift("CONFIG_INODE_MUTATION");
    });
    configWatcher.on("error", () => drift("INOTIFY_CONFIG_ERROR"));
    watchers.push(configWatcher);
  } catch (error) {
    for (const watcher of watchers) watcher.close();
    throw supervisorError("WSL2_CREDENTIAL_WATCH_REJECTED");
  }
  return Object.freeze({
    get drifted() { return drifted; },
    close() {
      if (closed) return;
      closed = true;
      for (const watcher of watchers) watcher.close();
    },
  });
}

async function procIdentity(pid, expectedUnit) {
  const [bootId, statContent, cgroupContent] = await Promise.all([
    readFile("/proc/sys/kernel/random/boot_id", "utf8"),
    readFile(`/proc/${pid}/stat`, "utf8"), readFile(`/proc/${pid}/cgroup`, "utf8"),
  ]);
  const fields = parseProcStat(statContent);
  return {
    boot_id_digest: `wsl-boot:sha256:${sha256(Buffer.from(bootId.trim(), "utf8"))}`,
    pid,
    process_start_ticks: fields.startTime,
    process_group_id: fields.processGroupId,
    cgroup_path: parseUnifiedCgroup(cgroupContent, expectedUnit, process.getuid()),
  };
}

async function ownCgroupAuthority(unit) {
  const [mountInfo, content] = await Promise.all([
    readFile("/proc/self/mountinfo", "utf8"), readFile("/proc/self/cgroup", "utf8"),
  ]);
  ensure(mountInfo.split("\n").some((line) => line.includes(` ${CGROUP_MOUNT} `) && line.includes(" - cgroup2 ")),
    "WSL2_CGROUP_V2_FENCE_REJECTED");
  const ownerUid = process.getuid();
  const cgroupPath = parseUnifiedCgroup(content, unit, ownerUid);
  const directory = `${CGROUP_MOUNT}${cgroupPath}`;
  const [type, subtree, directoryStat] = await Promise.all([
    readFile(`${directory}/cgroup.type`, "utf8"),
    readFile(`${directory}/cgroup.subtree_control`, "utf8"), stat(directory),
  ]);
  ensure(type.trim() === "domain" && subtree.trim() === "" && directoryStat.isDirectory()
    && Number(directoryStat.uid) === ownerUid,
    "WSL2_CGROUP_V2_FENCE_REJECTED");
  return { cgroup_path: cgroupPath, directory, cgroup_version: 2, delegated: false,
    owner_uid: Number(directoryStat.uid) };
}

async function cgroupPids(directory) {
  const result = new Set();
  const walk = async (current) => {
    const content = await readFile(`${current}/cgroup.procs`, "utf8");
    for (const line of content.trim().split("\n")) if (/^\d+$/u.test(line)) result.add(Number(line));
    for (const entry of await readdir(current, { withFileTypes: true })) {
      if (entry.isDirectory()) await walk(`${current}/${entry.name}`);
    }
  };
  await walk(directory);
  return [...result].sort((left, right) => left - right);
}

async function terminateDescendants(directory, timeoutMs = 10_000) {
  const signal = async (name) => {
    for (const pid of await cgroupPids(directory)) {
      if (pid === process.pid) continue;
      try { process.kill(pid, name); } catch (error) { if (error?.code !== "ESRCH") throw error; }
    }
  };
  await signal("SIGTERM");
  let deadline = Date.now() + timeoutMs;
  while ((await cgroupPids(directory)).some((pid) => pid !== process.pid) && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  if ((await cgroupPids(directory)).some((pid) => pid !== process.pid)) {
    await signal("SIGKILL");
    deadline = Date.now() + 2_000;
    while ((await cgroupPids(directory)).some((pid) => pid !== process.pid) && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  }
  return !(await cgroupPids(directory)).some((pid) => pid !== process.pid);
}

export async function runSealedKeyringUnlock(daemonSeal, librarySeals, environment) {
  ensure(Boolean(process.env.DBUS_SESSION_BUS_ADDRESS) && /^\/run\/user\/[0-9]+$/u.test(process.env.XDG_RUNTIME_DIR ?? ""),
    "WSL2_KEYRING_SESSION_REJECTED");
  ensure(Array.isArray(librarySeals)
    && librarySeals.length === KEYRING_PRIVATE_LIBRARY_FILES.length,
  "WSL2_KEYRING_LIBRARY_REJECTED");
  await Promise.all([
    verifyExecutableIdentitySeal(daemonSeal),
    ...librarySeals.map((seal) => verifyRegularFileIdentitySeal(seal)),
  ]);
  const preload = librarySeals.map((_seal, index) => `/proc/self/fd/${index + 4}`).join(":");
  const stdio = ["pipe", "ignore", "ignore", daemonSeal.handle.fd,
    ...librarySeals.map((seal) => seal.handle.fd)];
  await new Promise((resolve, reject) => {
    const child = spawn("/proc/self/fd/3", ["--unlock", "--components=secrets"], {
      env: {
        ...environment,
        DBUS_SESSION_BUS_ADDRESS: process.env.DBUS_SESSION_BUS_ADDRESS,
        LD_PRELOAD: preload,
      },
      stdio,
    });
    let settled = false;
    let timer = null;
    const finish = (operation) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      operation();
    };
    child.stdin.once("error", () => finish(() => reject(supervisorError("WSL2_KEYRING_START_REJECTED"))));
    child.once("error", () => finish(() => reject(supervisorError("WSL2_KEYRING_START_REJECTED"))));
    child.once("exit", (code) => finish(() => (
      code === 0 ? resolve() : reject(supervisorError("WSL2_KEYRING_START_REJECTED"))
    )));
    timer = setTimeout(() => {
      child.kill("SIGKILL");
      finish(() => reject(supervisorError("WSL2_KEYRING_START_REJECTED")));
    }, KEYRING_UNLOCK_TIMEOUT_MS);
    child.stdin.end("\n");
  });
  await Promise.all([
    verifyExecutableIdentitySeal(daemonSeal),
    ...librarySeals.map((seal) => verifyRegularFileIdentitySeal(seal)),
  ]);
}

export function observeBoundedOutput(state, key, chunk, limit) {
  const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
  const alreadyExceeded = state.outputBoundExceeded;
  const observed = state[key] + bytes.length;
  // Keep the receipt itself bounded while preserving an unambiguous overflow
  // sentinel. The retained spool never contains more than `limit` bytes; a
  // counter of `limit + 1` means at least one additional byte was observed.
  state[key] = Math.min(observed, limit + 1);
  if (observed > limit) {
    state.outputBoundExceeded = true;
    return !alreadyExceeded;
  }
  return false;
}

export function observeChildTerminal(child) {
  ensure(object(child) && typeof child.once === "function", "WSL2_CHILD_TERMINAL_REJECTED");
  return new Promise((resolve, reject) => {
    let settled = false;
    const finish = (operation) => {
      if (settled) return;
      settled = true;
      child.off("error", onError);
      child.off("exit", onExit);
      operation();
    };
    const onError = (error) => finish(() => reject(error));
    const onExit = (code, signal) => finish(() => resolve({ code, signal }));
    child.once("error", onError);
    child.once("exit", onExit);
    if (child.exitCode !== null || child.signalCode !== null) {
      queueMicrotask(() => onExit(child.exitCode, child.signalCode));
    }
  });
}

function writeBounded(target, chunk, state, key, limit, prefix = "") {
  const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
  const priorBytes = state[key];
  const exceeded = observeBoundedOutput(state, key, bytes, limit);
  if (state[key] > limit) {
    const remaining = Math.max(0, limit - Math.min(priorBytes, limit));
    if (remaining > 0) target.write(prefix ? `${prefix}${bytes.subarray(0, remaining).toString("utf8")}` : bytes.subarray(0, remaining));
    return exceeded;
  }
  target.write(prefix ? `${prefix}${bytes.toString("utf8").slice(0, 2048)}\n` : bytes);
  return false;
}

async function openVerifierSpools(options) {
  if (options.role === "PROVIDER") return null;
  const root = process.env.TMPDIR;
  ensure(canonicalLinuxPath(root, true), "WSL2_OUTPUT_SPOOL_REJECTED");
  const [resolvedRoot, rootStat] = await Promise.all([realpath(root), lstat(root)]);
  ensure(resolvedRoot === root && rootStat.isDirectory() && !rootStat.isSymbolicLink()
    && rootStat.uid === process.getuid() && (rootStat.mode & 0o077) === 0,
  "WSL2_OUTPUT_SPOOL_REJECTED");
  const base = `${root}/lattice-supervisor-${options.role.toLowerCase()}-${options.fence}-${process.pid}`;
  const flags = fsConstants.O_RDWR | fsConstants.O_CREAT | fsConstants.O_EXCL
    | (fsConstants.O_NOFOLLOW ?? 0);
  const opened = [];
  try {
    for (const channel of ["stdout", "stderr"]) {
      const path = `${base}.${channel}`;
      const handle = await open(path, flags, 0o600);
      const metadata = await handle.stat({ bigint: true });
      ensure(metadata.isFile() && Number(metadata.uid) === process.getuid()
        && (Number(metadata.mode) & 0o077) === 0, "WSL2_OUTPUT_SPOOL_REJECTED");
      const entry = {
        channel,
        path,
        handle,
        device: metadata.dev,
        inode: metadata.ino,
      };
      opened.push(entry);
      // Retain only the already-open descriptor. The verifier receives the
      // same TMPDIR and must never be able to discover or mutate its evidence
      // spool by pathname.
      await unlink(path);
      entry.path = null;
    }
    return Object.freeze(Object.fromEntries(opened.map((entry) => [entry.channel, entry])));
  } catch (error) {
    for (const entry of opened) {
      await entry.handle.close().catch(() => {});
      if (entry.path !== null) await unlink(entry.path).catch(() => {});
    }
    throw error;
  }
}

function streamVerifierOutput(source, entry, state, key, limit, terminate) {
  const counter = new Transform({
    transform(chunk, _encoding, callback) {
      if (observeBoundedOutput(state, key, chunk, limit)) {
        void terminate("OUTPUT_BOUND");
        callback(supervisorError("WSL2_OUTPUT_BOUND_EXCEEDED"));
        return;
      }
      if (state.outputBoundExceeded) {
        callback(supervisorError("WSL2_OUTPUT_BOUND_EXCEEDED"));
        return;
      }
      callback(null, chunk);
    },
  });
  const target = createWriteStream("/proc/self/fd/0", {
    fd: entry.handle.fd,
    autoClose: false,
  });
  return pipeline(source, counter, target).catch((error) => {
    if (error?.code === "WSL2_OUTPUT_BOUND_EXCEEDED") return;
    state.spoolIntegrity = false;
    return terminate("SPOOL_DRIFT");
  });
}

async function observeSpool(entry) {
  const metadata = await entry.handle.stat({ bigint: true });
  ensure(metadata.isFile() && metadata.dev === entry.device && metadata.ino === entry.inode
    && Number(metadata.uid) === process.getuid() && (Number(metadata.mode) & 0o077) === 0,
  "WSL2_OUTPUT_SPOOL_REJECTED");
  return Number(metadata.size);
}

async function readVerifierSpool(entry, limit) {
  const size = await observeSpool(entry);
  const length = Math.min(size, limit);
  const metadata = await entry.handle.stat({ bigint: true });
  ensure(metadata.dev === entry.device && metadata.ino === entry.inode
    && Number(metadata.size) === size, "WSL2_OUTPUT_SPOOL_REJECTED");
  const buffer = Buffer.alloc(length);
  let offset = 0;
  while (offset < length) {
    const { bytesRead } = await entry.handle.read(buffer, offset, length - offset, offset);
    ensure(bytesRead > 0, "WSL2_OUTPUT_SPOOL_REJECTED");
    offset += bytesRead;
  }
  return { buffer, size };
}

async function closeVerifierSpools(spools) {
  if (spools === null) return;
  for (const entry of [spools.stdout, spools.stderr]) {
    await entry.handle.close().catch(() => {});
  }
}

async function removeVerifierSpools(spools) {
  // Spools are unlinked immediately after open; closing their retained FDs is
  // the only cleanup operation.
  void spools;
}

async function openSupervisorToolSeals(options, keyringManifest) {
  const opened = [];
  const capture = async (name, file, digest, code, requireExecutable = true) => {
    const seal = requireExecutable
      ? await openExecutableIdentitySeal(file, digest, code)
      : await openRegularFileIdentitySeal(file, digest, code);
    opened.push(seal);
    return [name, seal];
  };
  try {
    const entries = [
      await capture("executable", options.executable, options["executable-sha256"],
        "WSL2_EXECUTABLE_DIGEST_MISMATCH"),
      await capture("sandboxHelper", options["sandbox-helper"], options["sandbox-helper-sha256"],
        "WSL2_SANDBOX_HELPER_DIGEST_MISMATCH"),
      await capture("keyringDaemon", options["keyring-daemon"], options["keyring-daemon-sha256"],
        "WSL2_KEYRING_DAEMON_DIGEST_MISMATCH"),
    ];
    if (options["verifier-tool"] !== "NONE") {
      entries.push(await capture("verifierTool", options["verifier-tool"],
        options["verifier-tool-sha256"], "WSL2_VERIFIER_TOOL_DIGEST_MISMATCH",
        options.role !== "NODE"));
    }
    if (options.role === "NODE" || options.role === "PREFLIGHT") {
      entries.push(await capture("nodeRuntime", options["node-runtime"],
        options["node-runtime-sha256"], "WSL2_NODE_RUNTIME_DIGEST_MISMATCH"));
    }
    if (options.role === "CARGO") {
      entries.push(await capture("rustc", options.rustc, options["rustc-sha256"],
        "WSL2_RUSTC_DIGEST_MISMATCH"));
      entries.push(await capture("rustdoc", options.rustdoc, options["rustdoc-sha256"],
        "WSL2_RUSTDOC_DIGEST_MISMATCH"));
    }
    const keyringLibraries = [];
    for (const record of selectKeyringPrivateLibraryRecords(keyringManifest.records)) {
      const [, seal] = await capture("keyringLibrary", `${options["keyring-library-path"]}/${record.path}`,
        record.sha256, "WSL2_KEYRING_LIBRARY_REJECTED", false);
      keyringLibraries.push(Object.freeze({ ...seal, manifestPath: record.path }));
    }
    entries.push(["keyringLibraries", Object.freeze(keyringLibraries)]);
    return Object.freeze(Object.fromEntries(entries));
  } catch (error) {
    await Promise.allSettled(opened.map(closeExecutableIdentitySeal));
    throw error;
  }
}

async function closeSupervisorToolSeals(seals) {
  if (!seals) return;
  const flat = Object.values(seals).flatMap((seal) => Array.isArray(seal) ? seal : [seal]);
  await Promise.allSettled(flat.map(closeExecutableIdentitySeal));
}

async function verifySupervisorToolSeals(seals, {
  requireSandboxPath = false,
  verifyDigest = true,
} = {}) {
  await Promise.all([
    verifyExecutableIdentitySeal(seals.executable, { verifyDigest }),
    verifyExecutableIdentitySeal(seals.sandboxHelper, {
      requirePath: requireSandboxPath,
      verifyDigest,
    }),
    verifyExecutableIdentitySeal(seals.keyringDaemon, { verifyDigest }),
    ...seals.keyringLibraries.map((seal) => verifyRegularFileIdentitySeal(seal, { verifyDigest })),
    ...(seals.verifierTool
      ? [verifyRegularFileIdentitySeal(seals.verifierTool, { verifyDigest })]
      : []),
    ...(seals.nodeRuntime
      ? [verifyExecutableIdentitySeal(seals.nodeRuntime, { verifyDigest })]
      : []),
    ...(seals.rustc ? [verifyExecutableIdentitySeal(seals.rustc, { verifyDigest })] : []),
    ...(seals.rustdoc ? [verifyExecutableIdentitySeal(seals.rustdoc, { verifyDigest })] : []),
  ]);
}

export function rewriteSealedVerifierCommandArgs(commandArgs, options) {
  ensure(Array.isArray(commandArgs) && commandArgs.every((argument) => typeof argument === "string")
    && object(options), "WSL2_VERIFIER_TOOL_IDENTITY_REJECTED");
  if (!["PREFLIGHT", "NODE", "CARGO", "GIT"].includes(options.role)) {
    return Object.freeze([...commandArgs]);
  }
  const counts = { verifier: 0, nodeRuntime: 0, rustc: 0, rustdoc: 0 };
  const rewritten = [];
  for (const argument of commandArgs) {
    if (options.role === "PREFLIGHT" && argument === options["node-runtime"]) {
      counts.nodeRuntime += 1;
      rewritten.push("/proc/self/fd/4");
    } else if (argument === options["verifier-tool"]) {
      counts.verifier += 1;
      if (options.role === "NODE") rewritten.push("/proc/self/fd/5", "/proc/self/fd/4");
      else rewritten.push("/proc/self/fd/4");
    } else if (options.role === "CARGO" && argument === `RUSTC=${options.rustc}`) {
      counts.rustc += 1;
      rewritten.push("RUSTC=/proc/self/fd/5");
    } else if (options.role === "CARGO" && argument === `RUSTDOC=${options.rustdoc}`) {
      counts.rustdoc += 1;
      rewritten.push("RUSTDOC=/proc/self/fd/6");
    } else {
      rewritten.push(argument);
    }
  }
  ensure((options.role === "PREFLIGHT" ? counts.nodeRuntime === 1 : counts.verifier === 1)
    && (options.role !== "CARGO" || (counts.rustc === 1 && counts.rustdoc === 1)),
  "WSL2_VERIFIER_TOOL_IDENTITY_REJECTED");
  return Object.freeze(rewritten);
}

function sealReceiptIdentity(seal) {
  return Object.freeze({
    path: seal.path,
    resolved_path: seal.resolvedPath,
    sha256: seal.sha256,
    ...seal.identity,
  });
}

function supervisorToolInputIdentities(seals) {
  return Object.freeze({
    executable: sealReceiptIdentity(seals.executable),
    verifier_tool: seals.verifierTool ? sealReceiptIdentity(seals.verifierTool) : null,
    sandbox_helper: sealReceiptIdentity(seals.sandboxHelper),
    node_runtime: seals.nodeRuntime ? sealReceiptIdentity(seals.nodeRuntime) : null,
    rustc: seals.rustc ? sealReceiptIdentity(seals.rustc) : null,
    rustdoc: seals.rustdoc ? sealReceiptIdentity(seals.rustdoc) : null,
    keyring_daemon: sealReceiptIdentity(seals.keyringDaemon),
    keyring_libraries: Object.freeze(seals.keyringLibraries.map((seal) => Object.freeze({
      manifest_path: seal.manifestPath,
      ...sealReceiptIdentity(seal),
    }))),
  });
}

async function main(argv = process.argv.slice(2)) {
  const { options, commandArgs } = parseSupervisorArgs(argv);
  ensure(options.role === "PROVIDER"
    ? process.env.CODEX_HOME === options["codex-home"]
    : process.env.CODEX_HOME === undefined,
  "WSL2_SUPERVISOR_IDENTITY_REJECTED");
  const childEnvironment = explicitChildEnvironment(options.role);
  const cgroup = await ownCgroupAuthority(options.unit);
  const keyringManifest = await observeKeyringLibraryManifest(options["keyring-library-path"]);
  ensure(keyringManifest.digest === options["keyring-library-manifest-digest"],
    "WSL2_KEYRING_LIBRARY_REJECTED");
  const toolSeals = await openSupervisorToolSeals(options, keyringManifest);
  try {
    const credentialDrift = { detected: false, terminate: null };
    const credential = await openCredentialSeal(options, {
      onDrift: () => {
        credentialDrift.detected = true;
        if (credentialDrift.terminate !== null) {
          return credentialDrift.terminate("CREDENTIAL_WATCH_DRIFT");
        }
        return undefined;
      },
    });
    try {
    ensure(!credentialDrift.detected, "WSL2_CREDENTIAL_SEAL_REJECTED");
    ensure(await boundedVersion(toolSeals.executable, ["--version"], childEnvironment,
      "WSL2_EXECUTABLE_VERSION_REJECTED") === options["executable-version"],
      "WSL2_EXECUTABLE_VERSION_MISMATCH");
    ensure(await boundedVersion(toolSeals.sandboxHelper, ["--version"], childEnvironment,
      "WSL2_SANDBOX_HELPER_VERSION_REJECTED")
      === options["sandbox-helper-version"], "WSL2_SANDBOX_HELPER_VERSION_MISMATCH");
    if (options.role === "NODE" || options.role === "PREFLIGHT") {
      ensure(await boundedVersion(toolSeals.nodeRuntime, ["--version"], childEnvironment,
        "WSL2_NODE_RUNTIME_VERSION_REJECTED") === options["node-runtime-version"],
      "WSL2_NODE_RUNTIME_VERSION_MISMATCH");
    }
    if (options.role === "NODE") {
      ensure((await runSealedNodeScriptOutput(toolSeals.nodeRuntime, toolSeals.verifierTool,
        ["--version"], childEnvironment, "WSL2_VERIFIER_TOOL_VERSION_REJECTED"))
        .replaceAll("\r", "").split("\n")[0].trimEnd() === options["verifier-tool-version"],
      "WSL2_VERIFIER_TOOL_VERSION_MISMATCH");
    } else if (toolSeals.verifierTool) {
      const versionArgs = options.role === "CARGO" ? ["-Vv"] : ["--version"];
      ensure(await boundedVersion(toolSeals.verifierTool, versionArgs, childEnvironment,
        "WSL2_VERIFIER_TOOL_VERSION_REJECTED")
        === options["verifier-tool-version"], "WSL2_VERIFIER_TOOL_VERSION_MISMATCH");
    }
    if (options.role === "CARGO") {
      ensure(await boundedVersion(toolSeals.rustc, ["-Vv"], childEnvironment,
        "WSL2_RUSTC_VERSION_REJECTED") === options["rustc-version"],
      "WSL2_RUSTC_VERSION_MISMATCH");
      ensure(await boundedVersion(toolSeals.rustdoc, ["--version"], childEnvironment,
        "WSL2_RUSTDOC_VERSION_REJECTED") === options["rustdoc-version"],
      "WSL2_RUSTDOC_VERSION_MISMATCH");
    }
    if (options.role === "PROVIDER" || options.role === "PREFLIGHT") {
      await runSealedKeyringUnlock(toolSeals.keyringDaemon, toolSeals.keyringLibraries, childEnvironment);
    }
    ensure(!credentialDrift.detected, "WSL2_CREDENTIAL_SEAL_REJECTED");
    await verifySupervisorToolSeals(toolSeals, { requireSandboxPath: true });
    process.chdir(options.cwd);
    const spools = await openVerifierSpools(options);
    try {
      const sealedCommandArgs = rewriteSealedVerifierCommandArgs(commandArgs, options);
      const childStdio = ["pipe", "pipe", "pipe"];
      childStdio.push(toolSeals.executable.handle.fd);
      if (toolSeals.verifierTool) childStdio.push(toolSeals.verifierTool.handle.fd);
      if (toolSeals.nodeRuntime) childStdio.push(toolSeals.nodeRuntime.handle.fd);
      if (toolSeals.rustc) childStdio.push(toolSeals.rustc.handle.fd);
      if (toolSeals.rustdoc) childStdio.push(toolSeals.rustdoc.handle.fd);
      const sealedChildEnvironment = options.role === "CARGO"
        ? { ...childEnvironment, RUSTC: "/proc/self/fd/5", RUSTDOC: "/proc/self/fd/6" }
        : childEnvironment;
      ensure(!credentialDrift.detected, "WSL2_CREDENTIAL_SEAL_REJECTED");
      const child = spawn("/proc/self/fd/3", sealedCommandArgs, {
        cwd: options.cwd,
        detached: false,
        env: sealedChildEnvironment,
        stdio: childStdio,
      });
      const terminalPromise = observeChildTerminal(child);
      const state = {
        stdoutBytes: 0, stderrBytes: 0, outputBoundExceeded: false, timedOut: false,
        interrupted: false, credentialSealIntact: true, toolIdentityIntact: true,
        spoolIntegrity: true, stdinIntegrity: true,
      };
      if (credentialDrift.detected) state.credentialSealIntact = false;
      const childStat = parseProcStat(await readFile(`/proc/${child.pid}/stat`, "utf8"));
      const marker = {
      schema: MARKER_SCHEMA, fence: options.fence, unit: options.unit,
      execution_environment_ref: options["execution-environment-ref"],
      credential_seal_digest: credential.initial.seal,
      boot_id_digest: `wsl-boot:sha256:${sha256(Buffer.from((await readFile("/proc/sys/kernel/random/boot_id", "utf8")).trim(), "utf8"))}`,
      pid: child.pid, process_start_ticks: childStat.startTime, process_group_id: childStat.processGroupId,
      cgroup_path: cgroup.cgroup_path, cgroup_version: 2, delegated: false,
      attempt: options.attempt, retry_of: options["retry-of"], reconnect_of: options["reconnect-of"],
      };
      process.stderr.write(`${JSON.stringify(marker)}\n`);
      let terminating = false;
      const stdinTracker = createExactStdinTracker(
        options["stdin-byte-len"],
        options["stdin-sha256"],
      );
      let stdinAttached = true;
      let stdinEnded = false;
      const pendingStdinWrites = new Set();
      const stdinFailure = () => {
        try { stdinTracker.markWriteError(); } catch { /* receipt is already permanently incomplete */ }
        state.stdinIntegrity = false;
        void terminate("STDIN_ERROR");
      };
      const trackStdinOperation = (operation) => {
        const token = {};
        token.promise = new Promise((resolve) => { token.resolve = resolve; });
        pendingStdinWrites.add(token);
        const done = (error) => {
          if (!pendingStdinWrites.delete(token)) return;
          if (error) stdinFailure();
          token.resolve();
        };
        try {
          return { accepted: operation(done), done };
        } catch (error) {
          done(error);
          return { accepted: false, done };
        }
      };
      const endChildStdin = () => {
        if (stdinEnded) return;
        stdinEnded = true;
        trackStdinOperation((done) => {
          child.stdin.end(done);
          return true;
        });
      };
      const onStdinData = (chunk) => {
        try {
          stdinTracker.observe(chunk);
        } catch {
          stdinFailure();
          return;
        }
        const { accepted } = trackStdinOperation((done) => child.stdin.write(chunk, done));
        if (!accepted) {
          process.stdin.pause();
          child.stdin.once("drain", () => { if (stdinAttached) process.stdin.resume(); });
        }
      };
      const onStdinEnd = () => {
        endChildStdin();
        if (options.role === "PROVIDER") void terminate("STDIN_END");
      };
      const detachStdin = () => {
        if (!stdinAttached) return;
        stdinAttached = false;
        process.stdin.off("data", onStdinData);
        process.stdin.off("end", onStdinEnd);
      };
      const terminate = async (reason) => {
        if (terminating) return;
        terminating = true;
        if (reason === "TIMEOUT") state.timedOut = true;
        if (reason === "SIGTERM" || reason === "SIGINT") state.interrupted = true;
        detachStdin();
        endChildStdin();
        await terminateDescendants(cgroup.directory);
      };
      credentialDrift.terminate = terminate;
      if (credentialDrift.detected) {
        state.credentialSealIntact = false;
        void terminate("CREDENTIAL_WATCH_DRIFT");
      }
      const spoolPipelines = spools === null ? [] : [
        streamVerifierOutput(child.stdout, spools.stdout, state, "stdoutBytes",
          options["stdout-limit-bytes"], terminate),
        streamVerifierOutput(child.stderr, spools.stderr, state, "stderrBytes",
          options["stderr-limit-bytes"], terminate),
      ];
      if (spools === null) {
        child.stdout.on("data", (chunk) => {
          if (writeBounded(process.stdout, chunk, state, "stdoutBytes",
            options["stdout-limit-bytes"])) void terminate("OUTPUT_BOUND");
        });
        child.stderr.on("data", (chunk) => {
          if (writeBounded(process.stderr, chunk, state, "stderrBytes",
            options["stderr-limit-bytes"], "CODEX_DIAGNOSTIC ")) void terminate("OUTPUT_BOUND");
        });
      }
      try { await observeCredential(options, credential.handle); } catch {
        state.credentialSealIntact = false;
        void terminate("CREDENTIAL_DRIFT");
      }
      process.stdin.on("data", onStdinData);
      process.stdin.once("end", onStdinEnd);
      child.stdin.on("error", stdinFailure);
      process.stdin.resume();
      const timeout = setTimeout(() => { void terminate("TIMEOUT"); }, options["timeout-ms"]);
      const credentialPoll = setInterval(() => {
      void observeCredential(options, credential.handle).catch(() => {
        state.credentialSealIntact = false;
        void terminate("CREDENTIAL_DRIFT");
      });
      }, 250);
      let toolCheck = null;
      const toolPoll = setInterval(() => {
      if (toolCheck !== null) return;
      toolCheck = verifySupervisorToolSeals(toolSeals, {
        requireSandboxPath: true,
        verifyDigest: false,
      }).catch(() => {
        state.toolIdentityIntact = false;
        return terminate("TOOL_DRIFT");
      }).finally(() => { toolCheck = null; });
      }, 250);
      let spoolCheck = null;
      const spoolPoll = spools === null ? null : setInterval(() => {
        if (spoolCheck !== null) return;
        spoolCheck = Promise.all([
          observeSpool(spools.stdout), observeSpool(spools.stderr),
        ]).then(([stdoutSize, stderrSize]) => {
          if (stdoutSize > options["stdout-limit-bytes"] || stderrSize > options["stderr-limit-bytes"]) {
            state.outputBoundExceeded = true;
            return terminate("OUTPUT_BOUND");
          }
          return undefined;
        }).catch(() => {
          state.spoolIntegrity = false;
          return terminate("SPOOL_DRIFT");
        }).finally(() => { spoolCheck = null; });
      }, 50);
      process.once("SIGTERM", () => { void terminate("SIGTERM"); });
      process.once("SIGINT", () => { void terminate("SIGINT"); });
      const terminal = await terminalPromise;
      if (spoolPipelines.length > 0) await Promise.all(spoolPipelines);
      clearTimeout(timeout);
      clearInterval(credentialPoll);
      clearInterval(toolPoll);
      if (toolCheck !== null) await toolCheck;
      if (spoolPoll !== null) clearInterval(spoolPoll);
      if (spoolCheck !== null) await spoolCheck;
      await terminate("CHILD_EXIT");
      detachStdin();
      endChildStdin();
      const stdinWritesSettled = await Promise.race([
        Promise.allSettled([...pendingStdinWrites].map((token) => token.promise)).then(() => true),
        new Promise((resolve) => setTimeout(() => resolve(false), 1_000)),
      ]);
      if (!stdinWritesSettled) {
        stdinTracker.markWriteError();
        state.stdinIntegrity = false;
        child.stdin.destroy();
      }
      const stdinReceipt = stdinTracker.finish();
      state.stdinIntegrity &&= stdinReceipt.stdin_complete;
      if (spools !== null) {
        const [stdout, stderr] = await Promise.all([
          readVerifierSpool(spools.stdout, options["stdout-limit-bytes"]),
          readVerifierSpool(spools.stderr, options["stderr-limit-bytes"]),
        ]);
        state.outputBoundExceeded ||= stdout.size > options["stdout-limit-bytes"]
          || stderr.size > options["stderr-limit-bytes"];
        if (!state.outputBoundExceeded
          && (stdout.size !== state.stdoutBytes || stderr.size !== state.stderrBytes)) {
          state.spoolIntegrity = false;
        }
        if (stdout.buffer.length > 0) process.stdout.write(stdout.buffer);
        if (stderr.buffer.length > 0) {
          process.stderr.write(`CODEX_DIAGNOSTIC ${stderr.buffer.toString("utf8")}\n`);
        }
        await closeVerifierSpools(spools);
      }
      try { await observeCredential(options, credential.handle); } catch { state.credentialSealIntact = false; }
      state.credentialSealIntact &&= !credentialDrift.detected;
      try { await verifySupervisorToolSeals(toolSeals, { requireSandboxPath: true }); } catch {
        state.toolIdentityIntact = false;
      }
      const zeroDescendants = !(await cgroupPids(cgroup.directory)).some((pid) => pid !== process.pid);
      const receipt = {
      schema: RECEIPT_SCHEMA, fence: options.fence, unit: options.unit,
      execution_environment_ref: options["execution-environment-ref"],
      credential_seal_digest: credential.initial.seal, cgroup_path: cgroup.cgroup_path,
      zero_descendants: zeroDescendants, credential_seal_intact: state.credentialSealIntact,
      credential_watch_intact: !credential.mutationWatch.drifted,
      keyring_daemon_sha256: toolSeals.keyringDaemon.sha256,
      keyring_library_manifest_digest: keyringManifest.digest,
      tool_input_identities: supervisorToolInputIdentities(toolSeals),
      stdout_bytes: state.stdoutBytes, stderr_bytes: state.stderrBytes,
      stdout_limit_bytes: options["stdout-limit-bytes"], stderr_limit_bytes: options["stderr-limit-bytes"],
      output_bound_exceeded: state.outputBoundExceeded, timeout_ms: options["timeout-ms"],
      timed_out: state.timedOut, interrupted: state.interrupted,
      ...stdinReceipt,
      attempt: options.attempt, retry_of: options["retry-of"], reconnect_of: options["reconnect-of"],
      exit_code: terminal.code, exit_signal: terminal.signal,
      };
      process.stderr.write(`${JSON.stringify(receipt)}\n`);
      if (!zeroDescendants || !state.credentialSealIntact || !state.toolIdentityIntact
        || !state.spoolIntegrity || !state.stdinIntegrity
        || state.outputBoundExceeded || state.timedOut) {
      process.exitCode = 71;
      } else if (terminal.code !== 0) {
      process.exitCode = terminal.code ?? 1;
      }
    } finally {
      credentialDrift.terminate = null;
      await closeVerifierSpools(spools);
      await removeVerifierSpools(spools);
    }
    } finally {
      credential.mutationWatch.close();
      await credential.handle.close();
    }
  } finally {
    await closeSupervisorToolSeals(toolSeals);
  }
}

function fail(error) {
  const code = typeof error?.code === "string" && /^WSL2_[A-Z0-9_]+$/u.test(error.code)
    ? error.code
    : "WSL2_SUPERVISOR_FAILED";
  process.stderr.write(`${JSON.stringify({ schema: RECEIPT_SCHEMA, status: "REJECTED", code })}\n`);
  process.exitCode = 70;
}

const isEntryPoint = import.meta.url.startsWith("file:")
  && process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];
if (isEntryPoint) main().catch(fail);

export { main as runWsl2CodexSupervisor };
