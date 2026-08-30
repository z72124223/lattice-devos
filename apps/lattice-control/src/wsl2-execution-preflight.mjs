import { execFile as execFileCallback } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile as readFileDefault } from "node:fs/promises";

import {
  buildWsl2SandboxState,
  canonicalJson,
  codexHomeIdentity,
  executionEnvironmentIdentity,
  immutableSnapshotIdentity,
  MAX_WSL2_ATTEMPTS,
  privilegeBoundaryIdentity,
  validateWsl2SubtreeExitReceipt,
  validateWsl2ImmutableObservation,
  WSL2_PROCESS_MARKER_SCHEMA,
  WSL2_SUBTREE_EXIT_SCHEMA,
  WSL2_SUPERVISOR_BOOTSTRAP_SHA256,
  WSL2_SUPERVISOR_BOOTSTRAP_SOURCE,
  validateWsl2ExecutionEnvironment,
  windowsWslPathToLinux,
} from "./wsl2-execution-domain.mjs";

function execFileClosedStdin(program, args, options) {
  return new Promise((resolve, reject) => {
    const child = execFileCallback(program, args, options, (error, stdout, stderr) => {
      if (error) {
        if (error.stdout === undefined) error.stdout = stdout;
        if (error.stderr === undefined) error.stderr = stderr;
        reject(error);
      } else {
        resolve({ stdout, stderr });
      }
    });
    child.stdin?.end();
  });
}

const execFileDefault = execFileClosedStdin;
const HEX_40 = /^[a-f0-9]{40}$/u;
const HEX_64 = /^[a-f0-9]{64}$/u;
const TYPED = /^[a-z0-9][a-z0-9-]*:sha256:[a-f0-9]{64}$/u;
const MAX_OUTPUT = 1_048_576;
const IMMUTABLE_TREE_NAMES = Object.freeze([
  "codex", "supervisor_runtime", "node", "rust", "keyring",
]);
const IMMUTABLE_LIMITS = Object.freeze({
  max_entries_per_tree: 200_000,
  max_file_bytes_per_tree: 8 * 1_073_741_824,
  max_single_file_bytes: 1_073_741_824,
});

function failure(code) {
  const error = new Error(code);
  error.code = code;
  return error;
}

function ensure(condition, code) {
  if (!condition) throw failure(code);
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

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function digest(domain, subject) {
  return `${domain}:sha256:${sha256(Buffer.from(canonicalJson(subject), "utf8"))}`;
}

function firstLine(value) {
  return value.replaceAll("\r", "").split("\n")[0].trimEnd();
}

function canonicalLinuxHomePath(value) {
  return typeof value === "string" && value.startsWith("/home/")
    && !value.includes("\\") && !value.includes("\0") && !value.includes("/../")
    && !value.endsWith("/..") && !value.includes("/./") && !value.endsWith("/.");
}

function firstDigest(output) {
  const observed = output.trim().split(/\s+/u)[0];
  ensure(HEX_64.test(observed), "WSL2_PREFLIGHT_INVALID_DIGEST_OUTPUT");
  return observed;
}

function onlyJsonLine(output, code) {
  const lines = output.replaceAll("\r", "").split("\n").filter((line) => line.trim().length > 0);
  ensure(lines.length === 1 && Buffer.byteLength(lines[0], "utf8") <= 256 * 1024, code);
  try {
    return JSON.parse(lines[0]);
  } catch {
    throw failure(code);
  }
}

function framedJson(stderr, schema, code) {
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

function immutableProbeInput(input, enforceExpected) {
  const code = "WSL2_PREFLIGHT_IMMUTABLE_REQUEST_REJECTED";
  ensure(object(input?.verification_toolchain) && object(input?.process_fence)
    && object(input?.immutable_snapshot), code);
  const taskRoot = input.verification_toolchain.task_root;
  const ownerUid = input.verification_toolchain.owner_uid;
  ensure(canonicalLinuxHomePath(taskRoot) && Number.isSafeInteger(ownerUid) && ownerUid > 0, code);
  const tools = {
    controller: input.process_fence.supervisor_bootstrap_node,
    lsattr: input.process_fence.immutable_probe_lsattr,
    sudo: input.process_fence.noninteractive_root_probe,
  };
  for (const [name, tool] of Object.entries(tools)) {
    exactKeys(tool, ["path", "version", "sha256"], code);
    ensure(typeof tool.path === "string" && tool.path.startsWith("/usr/bin/")
      && typeof tool.version === "string" && tool.version.length > 0 && HEX_64.test(tool.sha256), code);
    if (name === "controller") ensure(tool.path === "/usr/bin/node", code);
    if (name === "lsattr") ensure(tool.path === "/usr/bin/lsattr", code);
    if (name === "sudo") ensure(tool.path === "/usr/bin/sudo", code);
  }
  exactKeys(input.immutable_snapshot.trees, IMMUTABLE_TREE_NAMES, code);
  const trees = {};
  for (const name of IMMUTABLE_TREE_NAMES) {
    const tree = input.immutable_snapshot.trees[name];
    exactKeys(tree, ["root", "manifest_digest"], code);
    ensure(canonicalLinuxHomePath(tree.root) && tree.root.startsWith(`${taskRoot}/`)
      && !tree.root.slice(taskRoot.length + 1).includes("/"), code);
    ensure(!enforceExpected || (typeof tree.manifest_digest === "string"
      && /^immutable-tree-manifest:sha256:[a-f0-9]{64}$/u.test(tree.manifest_digest)), code);
    trees[name] = {
      root: tree.root,
      expected_manifest_digest: enforceExpected ? tree.manifest_digest : null,
    };
  }
  ensure(new Set(Object.values(trees).map((tree) => tree.root)).size === IMMUTABLE_TREE_NAMES.length,
    code);
  return { taskRoot, ownerUid, tools, trees };
}

function validateImmutableSourceFacts(facts, request) {
  const snapshotCode = "WSL2_PREFLIGHT_IMMUTABLE_SNAPSHOT_REJECTED";
  const privilegeCode = "WSL2_PREFLIGHT_IMMUTABLE_PRIVILEGE_REJECTED";
  exactKeys(facts, ["schema", "task_root", "trees", "privilege", "bounds"], snapshotCode);
  ensure(facts.schema === "lattice.wsl2-immutable-observation-source/1.0", snapshotCode);
  exactKeys(facts.task_root, [
    "path", "device", "inode", "owner_uid", "owner_gid", "mode", "immutable",
  ], snapshotCode);
  ensure(facts.task_root.path === request.taskRoot
    && /^[1-9][0-9]*$/u.test(facts.task_root.device)
    && /^[1-9][0-9]*$/u.test(facts.task_root.inode)
    && facts.task_root.owner_uid === 0 && facts.task_root.owner_gid === 0
    && facts.task_root.mode === "0555" && facts.task_root.immutable === true, snapshotCode);
  exactKeys(facts.trees, IMMUTABLE_TREE_NAMES, snapshotCode);
  for (const name of IMMUTABLE_TREE_NAMES) {
    const tree = facts.trees[name];
    exactKeys(tree, ["root", "manifest_digest", "entry_count", "file_bytes"], snapshotCode);
    ensure(tree.root === request.trees[name].root
      && /^immutable-tree-manifest:sha256:[a-f0-9]{64}$/u.test(tree.manifest_digest)
      && Number.isSafeInteger(tree.entry_count) && tree.entry_count >= 1
      && tree.entry_count <= IMMUTABLE_LIMITS.max_entries_per_tree
      && Number.isSafeInteger(tree.file_bytes) && tree.file_bytes >= 0
      && tree.file_bytes <= IMMUTABLE_LIMITS.max_file_bytes_per_tree, snapshotCode);
    if (request.trees[name].expected_manifest_digest !== null) {
      ensure(tree.manifest_digest === request.trees[name].expected_manifest_digest, snapshotCode);
    }
  }
  exactKeys(facts.bounds, [
    "max_entries_per_tree", "max_file_bytes_per_tree", "max_single_file_bytes",
  ], snapshotCode);
  ensure(canonicalJson(facts.bounds) === canonicalJson(IMMUTABLE_LIMITS), snapshotCode);
  exactKeys(facts.privilege, [
    "effective_uid", "effective_gid", "effective_capabilities_digest", "capabilities_empty",
    "noninteractive_root_unavailable", "sudo_denial_recognized", "sudo_exit_code",
    "sudo_stdout_bytes", "sudo_stderr_bytes", "sudo_stdout_sha256", "sudo_stderr_sha256",
  ], privilegeCode);
  ensure(facts.privilege.effective_uid === request.ownerUid
    && Number.isSafeInteger(facts.privilege.effective_gid) && facts.privilege.effective_gid > 0
    && /^linux-capabilities:sha256:[a-f0-9]{64}$/u.test(facts.privilege.effective_capabilities_digest)
    && facts.privilege.capabilities_empty === true
    && facts.privilege.noninteractive_root_unavailable === true
    && facts.privilege.sudo_denial_recognized === true && facts.privilege.sudo_exit_code === 1
    && facts.privilege.sudo_stdout_bytes === 0
    && Number.isSafeInteger(facts.privilege.sudo_stderr_bytes)
    && facts.privilege.sudo_stderr_bytes >= 1 && facts.privilege.sudo_stderr_bytes <= 16_384
    && HEX_64.test(facts.privilege.sudo_stdout_sha256)
    && facts.privilege.sudo_stdout_sha256 === sha256(Buffer.alloc(0))
    && HEX_64.test(facts.privilege.sudo_stderr_sha256), privilegeCode);
  return facts;
}

async function runImmutableProbe(input, dependencies, enforceExpected) {
  const request = immutableProbeInput(input, enforceExpected);
  const run = dependencies?.run;
  ensure(typeof run === "function", "WSL2_PREFLIGHT_IMMUTABLE_RUNNER_REQUIRED");
  for (const [label, tool] of Object.entries(request.tools)) {
    const observed = firstDigest((await run("/usr/bin/sha256sum", [tool.path], {
      timeout: 10_000, maxBuffer: 64 * 1024,
    })).stdout);
    ensure(observed === tool.sha256,
      `WSL2_PREFLIGHT_IMMUTABLE_${label.toUpperCase()}_DIGEST_MISMATCH`);
  }
  const controllerVersion = firstLine((await run(request.tools.controller.path,
    ["--version"], { timeout: 10_000, maxBuffer: 64 * 1024 })).stdout);
  ensure(controllerVersion === request.tools.controller.version,
    "WSL2_PREFLIGHT_IMMUTABLE_CONTROLLER_VERSION_MISMATCH");
  const lsattrResult = await run(request.tools.lsattr.path,
    ["-V", "-d", request.taskRoot], { timeout: 10_000, maxBuffer: 64 * 1024 });
  const lsattrStdout = (lsattrResult.stdout ?? "").replaceAll("\r", "").split("\n").filter(Boolean);
  const lsattrStderr = (lsattrResult.stderr ?? "").replaceAll("\r", "").split("\n").filter(Boolean);
  ensure(lsattrStderr.length === 1 && lsattrStderr[0] === request.tools.lsattr.version
    && lsattrStdout.length === 1 && /^([A-Za-z-]+)\s+(.+)$/u.test(lsattrStdout[0])
    && lsattrStdout[0].endsWith(` ${request.taskRoot}`),
    "WSL2_PREFLIGHT_IMMUTABLE_LSATTR_VERSION_MISMATCH");
  const sudoVersion = firstLine((await run(request.tools.sudo.path,
    ["-V"], { timeout: 10_000, maxBuffer: 64 * 1024 })).stdout);
  ensure(sudoVersion === request.tools.sudo.version,
    "WSL2_PREFLIGHT_IMMUTABLE_SUDO_VERSION_MISMATCH");
  const sourceConfig = {
    schema: "lattice.wsl2-immutable-observation-request/1.0",
    enforce_expected: enforceExpected,
    task_root: request.taskRoot,
    owner_uid: request.ownerUid,
    trees: request.trees,
    tools: { lsattr: request.tools.lsattr, sudo: request.tools.sudo },
    limits: IMMUTABLE_LIMITS,
  };
  const result = await run(request.tools.controller.path, [
    "-e", IMMUTABLE_OBSERVATION_SOURCE, canonicalJson(sourceConfig),
  ], { timeout: 120_000, maxBuffer: 256 * 1024 });
  ensure(Buffer.byteLength(result.stderr ?? "", "utf8") === 0,
    "WSL2_PREFLIGHT_IMMUTABLE_SOURCE_STDERR_REJECTED");
  const facts = onlyJsonLine(result.stdout ?? "", "WSL2_PREFLIGHT_IMMUTABLE_SNAPSHOT_REJECTED");
  return {
    request,
    facts: validateImmutableSourceFacts(facts, request),
    probeTools: {
      controller: structuredClone(request.tools.controller),
      lsattr: structuredClone(request.tools.lsattr),
      noninteractive_root: structuredClone(request.tools.sudo),
      source_sha256: sha256(Buffer.from(IMMUTABLE_OBSERVATION_SOURCE, "utf8")),
    },
  };
}

/**
 * Establishes the expected immutable tree and privilege facts before the
 * descriptor is sealed. The input is a descriptor draft whose five tree roots
 * are present; supplied manifest digests are deliberately ignored.
 */
export async function materializeWsl2ImmutableExecutionFacts(descriptorDraft, dependencies = {}) {
  const { request, facts, probeTools } = await runImmutableProbe(descriptorDraft, dependencies, false);
  const immutableSnapshot = {
    schema: "lattice.wsl2-immutable-snapshot/1.0",
    task_root_path: facts.task_root.path,
    task_root_device: facts.task_root.device,
    task_root_inode: facts.task_root.inode,
    task_root_owner_uid: facts.task_root.owner_uid,
    task_root_owner_gid: facts.task_root.owner_gid,
    task_root_mode: facts.task_root.mode,
    task_root_immutable: facts.task_root.immutable,
    trees: Object.fromEntries(IMMUTABLE_TREE_NAMES.map((name) => [name, {
      root: facts.trees[name].root,
      manifest_digest: facts.trees[name].manifest_digest,
    }])),
    snapshot_digest: null,
  };
  immutableSnapshot.snapshot_digest = immutableSnapshotIdentity({ immutable_snapshot: immutableSnapshot });
  const privilegeBoundary = {
    schema: "lattice.wsl2-privilege-boundary/1.0",
    effective_uid: facts.privilege.effective_uid,
    effective_gid: facts.privilege.effective_gid,
    effective_capabilities_digest: facts.privilege.effective_capabilities_digest,
    noninteractive_root_unavailable: facts.privilege.noninteractive_root_unavailable,
    boundary_digest: null,
  };
  privilegeBoundary.boundary_digest = privilegeBoundaryIdentity({ privilege_boundary: privilegeBoundary });
  const evidence = {
    schema: "lattice.wsl2-immutable-materialization/1.0",
    immutable_snapshot_ref: immutableSnapshot.snapshot_digest,
    privilege_boundary_ref: privilegeBoundary.boundary_digest,
    task_root: structuredClone(facts.task_root),
    trees: structuredClone(facts.trees),
    privilege: structuredClone(facts.privilege),
    probe_tools: probeTools,
    bounds: structuredClone(facts.bounds),
    materialization_digest: null,
  };
  evidence.materialization_digest = digest("wsl2-immutable-materialization", Object.fromEntries(
    Object.entries(evidence).filter(([key]) => key !== "materialization_digest"),
  ));
  ensure(request.ownerUid === privilegeBoundary.effective_uid,
    "WSL2_PREFLIGHT_IMMUTABLE_PRIVILEGE_REJECTED");
  return Object.freeze({ immutable_snapshot: immutableSnapshot,
    privilege_boundary: privilegeBoundary, evidence });
}

/** Re-observes a sealed descriptor; no supplied manifest or privilege fact is trusted. */
export async function observeWsl2ImmutableExecutionState(untrusted, dependencies = {}) {
  const descriptor = validateWsl2ExecutionEnvironment(untrusted);
  const { facts, probeTools } = await runImmutableProbe(descriptor, dependencies, true);
  const snapshot = descriptor.immutable_snapshot;
  ensure(facts.task_root.path === snapshot.task_root_path
    && facts.task_root.device === snapshot.task_root_device
    && facts.task_root.inode === snapshot.task_root_inode
    && facts.task_root.owner_uid === snapshot.task_root_owner_uid
    && facts.task_root.owner_gid === snapshot.task_root_owner_gid
    && facts.task_root.mode === snapshot.task_root_mode
    && facts.task_root.immutable === snapshot.task_root_immutable,
  "WSL2_PREFLIGHT_IMMUTABLE_SNAPSHOT_REJECTED");
  for (const name of IMMUTABLE_TREE_NAMES) {
    ensure(facts.trees[name].root === snapshot.trees[name].root
      && facts.trees[name].manifest_digest === snapshot.trees[name].manifest_digest,
    "WSL2_PREFLIGHT_IMMUTABLE_SNAPSHOT_REJECTED");
  }
  const boundary = descriptor.privilege_boundary;
  ensure(facts.privilege.effective_uid === boundary.effective_uid
    && facts.privilege.effective_gid === boundary.effective_gid
    && facts.privilege.effective_capabilities_digest === boundary.effective_capabilities_digest
    && facts.privilege.noninteractive_root_unavailable === boundary.noninteractive_root_unavailable,
  "WSL2_PREFLIGHT_IMMUTABLE_PRIVILEGE_REJECTED");
  const observation = {
    schema: "lattice.wsl2-immutable-observation/1.0",
    execution_environment_ref: descriptor.identity_digest,
    immutable_snapshot_ref: snapshot.snapshot_digest,
    sandbox_policy_ref: descriptor.sandbox_policy.policy_digest,
    privilege_boundary_ref: boundary.boundary_digest,
    task_root: structuredClone(facts.task_root),
    trees: structuredClone(facts.trees),
    privilege: structuredClone(facts.privilege),
    probe_tools: probeTools,
    bounds: structuredClone(facts.bounds),
    observation_digest: null,
  };
  observation.observation_digest = digest("wsl2-immutable-observation", Object.fromEntries(
    Object.entries(observation).filter(([key]) => key !== "observation_digest"),
  ));
  return Object.freeze(validateWsl2ImmutableObservation(observation, descriptor,
    "WSL2_PREFLIGHT_IMMUTABLE_OBSERVATION_REJECTED"));
}

async function defaultGatewayVersion({ gatewayPath, execFile }) {
  const escapedPath = gatewayPath.replaceAll("\\", "\\\\").replaceAll("'", "''");
  const { stdout } = await execFile("wmic.exe", [
    "datafile", "where", `name='${escapedPath}'`, "get", "Version", "/value",
  ], { encoding: "utf8", windowsHide: true, timeout: 30_000, maxBuffer: 64 * 1024 });
  const match = stdout.replaceAll("\r", "").match(/(?:^|\n)Version=(\d+\.\d+(?:\.\d+){0,2})(?:\n|$)/u);
  ensure(match, "WSL2_PREFLIGHT_GATEWAY_VERSION_UNAVAILABLE");
  return match[1];
}

const CREDENTIAL_PROBE_SOURCE = String.raw`
const fs = require("node:fs");
const crypto = require("node:crypto");
const path = require("node:path");
const home = process.argv[1];
const expectedUid = Number(process.argv[2]);
const configPath = path.join(home, "config.toml");
const authPath = path.join(home, "auth.json");
const nofollow = fs.constants.O_NOFOLLOW || 0;
const fd = fs.openSync(configPath, fs.constants.O_RDONLY | nofollow);
try {
  const stat = fs.fstatSync(fd, { bigint: true });
  const bytes = fs.readFileSync(fd);
  const text = bytes.toString("utf8");
  let authAbsent = false;
  try { fs.lstatSync(authPath); } catch (error) { authAbsent = error && error.code === "ENOENT"; }
  const keyringOnly = /^\s*cli_auth_credentials_store\s*=\s*["']keyring["']\s*(?:#.*)?$/mu.test(text)
    && !/^\s*cli_auth_credentials_store\s*=\s*["'](?:file|auto)["']/mu.test(text);
  const lines = text.replaceAll("\r", "").split("\n");
  const section = lines.indexOf("[shell_environment_policy]");
  if (section < 0 || lines.indexOf("[shell_environment_policy]", section + 1) !== -1) {
    throw new Error("SHELL_ENVIRONMENT_POLICY_INVALID");
  }
  const assignments = new Map();
  for (const line of lines.slice(section + 1)) {
    if (line.startsWith("[")) break;
    if (line.trim() === "") continue;
    const match = line.match(/^([a-z_]+)\s*=\s*(.+)$/u);
    if (!match || assignments.has(match[1])) throw new Error("SHELL_ENVIRONMENT_POLICY_INVALID");
    assignments.set(match[1], match[2]);
  }
  let includeOnly;
  try { includeOnly = JSON.parse(assignments.get("include_only")); } catch {}
  const expected = ["HOME", "PATH", "LANG", "LC_ALL", "TERM", "COLORTERM"];
  if (assignments.size !== 4 || assignments.get("inherit") !== '"all"'
      || assignments.get("ignore_default_excludes") !== "false"
      || assignments.get("experimental_use_profile") !== "false"
      || !Array.isArray(includeOnly) || JSON.stringify(includeOnly) !== JSON.stringify(expected)) {
    throw new Error("SHELL_ENVIRONMENT_POLICY_INVALID");
  }
  const probeInput = { HOME: "/home/lattice/probe", CODEX_HOME: home,
    PATH: "/usr/bin:/bin", LANG: "C.UTF-8", LC_ALL: "C.UTF-8", TERM: "dumb",
    COLORTERM: "false", LATTICE_FAKE_API_TOKEN: "must-not-cross" };
  const effectiveKeys = includeOnly.filter((name) => Object.hasOwn(probeInput, name));
  const requiredKeysPresent = ["HOME", "PATH"].every((name) => effectiveKeys.includes(name));
  const forbiddenKeysAbsent = !effectiveKeys.includes("CODEX_HOME")
    && effectiveKeys.every((name) => !/(?:KEY|SECRET|TOKEN)/iu.test(name));
  if (!requiredKeysPresent || !forbiddenKeysAbsent) throw new Error("SHELL_ENVIRONMENT_POLICY_INVALID");
  process.stdout.write(JSON.stringify({
    config_regular_file: stat.isFile(),
    config_sha256: crypto.createHash("sha256").update(bytes).digest("hex"),
    config_identity: {
      device: String(stat.dev), inode: String(stat.ino), owner_uid: Number(stat.uid),
      mode: stat.mode.toString(8), size: Number(stat.size),
    },
    config_owner_matches: Number(stat.uid) === expectedUid,
    keyring_only: keyringOnly,
    auth_json_absent: authAbsent,
    shell_environment_policy: {
      inherit: "all", ignore_default_excludes: false, include_only: includeOnly,
      experimental_use_profile: false, set_keys: [], probe_effective_keys: effectiveKeys,
      required_keys_present: requiredKeysPresent, forbidden_keys_absent: forbiddenKeysAbsent,
    },
  }) + "\n");
} finally { fs.closeSync(fd); }
`;

const ISOLATION_PROBE_SOURCE = String.raw`
const fs = require("node:fs");
const paths = JSON.parse(process.argv[1]);
const expectedUid = Number(process.argv[2]);
const observations = {};
for (const candidate of paths) {
  const lstat = fs.lstatSync(candidate, { bigint: true });
  const realpath = fs.realpathSync(candidate);
  observations[candidate] = {
    realpath, directory: lstat.isDirectory(), symlink: lstat.isSymbolicLink(),
    owner_uid: Number(lstat.uid), owner_matches: Number(lstat.uid) === expectedUid,
    mode: lstat.mode.toString(8), device: String(lstat.dev), inode: String(lstat.ino),
  };
}
process.stdout.write(JSON.stringify({ observations }) + "\n");
`;

const KEYRING_LIBRARY_MANIFEST_SOURCE = String.raw`
const fs = require("node:fs");
const crypto = require("node:crypto");
const root = process.argv[1];
const expectedUid = Number(process.argv[2]);
const canonical = (value) => Array.isArray(value) ? value.map(canonical)
  : value && typeof value === "object"
    ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]))
    : value;
if (!Number.isSafeInteger(expectedUid) || expectedUid < 0 || fs.realpathSync(root) !== root) {
  throw new Error("KEYRING_LIBRARY_REJECTED");
}
const records = [];
const walk = (current, relative) => {
  const metadata = fs.lstatSync(current);
  if (metadata.uid !== expectedUid) throw new Error("KEYRING_LIBRARY_REJECTED");
  const mode = metadata.mode & 0o7777;
  if (metadata.isDirectory()) {
    if (metadata.isSymbolicLink() || (mode & 0o022) !== 0) throw new Error("KEYRING_LIBRARY_REJECTED");
    records.push({ path: relative, kind: "DIRECTORY", mode, owner_uid: metadata.uid });
    const entries = fs.readdirSync(current, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name, "en"));
    for (const entry of entries) {
      if (entry.name.includes("/") || entry.name === "." || entry.name === "..") {
        throw new Error("KEYRING_LIBRARY_REJECTED");
      }
      walk(current + "/" + entry.name, relative === "." ? entry.name : relative + "/" + entry.name);
    }
    return;
  }
  if (metadata.isFile()) {
    if ((mode & 0o022) !== 0) throw new Error("KEYRING_LIBRARY_REJECTED");
    const bytes = fs.readFileSync(current);
    records.push({ path: relative, kind: "FILE", mode, owner_uid: metadata.uid,
      byte_len: metadata.size, sha256: crypto.createHash("sha256").update(bytes).digest("hex") });
    return;
  }
  if (metadata.isSymbolicLink()) {
    const target = fs.readlinkSync(current);
    if (!/^[A-Za-z0-9._+-]{1,255}$/.test(target)) throw new Error("KEYRING_LIBRARY_REJECTED");
    const resolved = fs.realpathSync(current);
    const targetMetadata = fs.statSync(resolved);
    if (!resolved.startsWith(root + "/") || !targetMetadata.isFile() || targetMetadata.uid !== expectedUid) {
      throw new Error("KEYRING_LIBRARY_REJECTED");
    }
    records.push({ path: relative, kind: "SYMLINK", owner_uid: metadata.uid, target });
    return;
  }
  throw new Error("KEYRING_LIBRARY_REJECTED");
};
walk(root, ".");
const digest = "keyring-library-manifest:sha256:" + crypto.createHash("sha256")
  .update(JSON.stringify(canonical(records)), "utf8").digest("hex");
process.stdout.write(JSON.stringify({ schema: "lattice.wsl2-keyring-library-manifest/1.0", digest }) + "\n");
`;

const IMMUTABLE_OBSERVATION_SOURCE = String.raw`
const fs = require("node:fs");
const cp = require("node:child_process");
const crypto = require("node:crypto");
const path = require("node:path");
const config = JSON.parse(process.argv[1]);
const canonical = (value) => Array.isArray(value) ? value.map(canonical)
  : value && typeof value === "object"
    ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]))
    : value;
const hashJson = (value) => crypto.createHash("sha256")
  .update(JSON.stringify(canonical(value)), "utf8").digest("hex");
const fail = () => { throw new Error("IMMUTABLE_OBSERVATION_REJECTED"); };
const exactKeys = (value, keys) => {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail();
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (actual.length !== expected.length || !actual.every((key, index) => key === expected[index])) fail();
};
const safePath = (value) => typeof value === "string" && value.startsWith("/home/")
  && path.posix.normalize(value) === value && !value.includes("\\") && !value.includes("\0");
exactKeys(config, ["schema", "enforce_expected", "task_root", "owner_uid", "trees", "tools", "limits"]);
if (config.schema !== "lattice.wsl2-immutable-observation-request/1.0"
    || typeof config.enforce_expected !== "boolean" || !safePath(config.task_root)
    || !Number.isSafeInteger(config.owner_uid) || config.owner_uid <= 0) fail();
exactKeys(config.limits, ["max_entries_per_tree", "max_file_bytes_per_tree", "max_single_file_bytes"]);
if (config.limits.max_entries_per_tree !== 200000
    || config.limits.max_file_bytes_per_tree !== 8589934592
    || config.limits.max_single_file_bytes !== 1073741824) fail();
exactKeys(config.tools, ["lsattr", "sudo"]);
for (const key of ["lsattr", "sudo"]) {
  exactKeys(config.tools[key], ["path", "version", "sha256"]);
  if (!/^\/usr\/bin\/[a-z]+$/.test(config.tools[key].path)
      || !/^[a-f0-9]{64}$/.test(config.tools[key].sha256)
      || typeof config.tools[key].version !== "string") fail();
}
const modeString = (metadata) => (Number(metadata.mode) & 0o7777).toString(8).padStart(4, "0");
const taskMetadata = fs.lstatSync(config.task_root, { bigint: true });
if (!taskMetadata.isDirectory() || taskMetadata.isSymbolicLink()
    || fs.realpathSync(config.task_root) !== config.task_root
    || Number(taskMetadata.uid) !== 0 || Number(taskMetadata.gid) !== 0
    || modeString(taskMetadata) !== "0555") fail();
const lsattr = cp.spawnSync(config.tools.lsattr.path, ["-V", "-d", config.task_root], {
  encoding: "utf8", timeout: 5000, maxBuffer: 16384,
  env: { HOME: path.posix.dirname(config.task_root), PATH: "/usr/bin:/bin", LANG: "C.UTF-8", LC_ALL: "C.UTF-8" },
});
if (lsattr.error || lsattr.signal !== null || lsattr.status !== 0
    || Buffer.byteLength(lsattr.stdout || "", "utf8") > 16384
    || Buffer.byteLength(lsattr.stderr || "", "utf8") > 16384) fail();
const lsattrLines = (lsattr.stdout || "").replaceAll("\r", "").split("\n").filter(Boolean);
const lsattrErrors = (lsattr.stderr || "").replaceAll("\r", "").split("\n").filter(Boolean);
if (lsattrErrors.length !== 1 || lsattrErrors[0] !== config.tools.lsattr.version || lsattrLines.length !== 1) fail();
const attributeMatch = /^([A-Za-z-]+)\s+(.+)$/.exec(lsattrLines[0]);
if (!attributeMatch || !attributeMatch[1].includes("i") || attributeMatch[2] !== config.task_root) fail();

const treeNames = ["codex", "supervisor_runtime", "node", "rust", "keyring"];
exactKeys(config.trees, treeNames);
const treeResults = {};
const hashFile = (filename, expected) => {
  const nofollow = fs.constants.O_NOFOLLOW || 0;
  const fd = fs.openSync(filename, fs.constants.O_RDONLY | nofollow);
  try {
    const observed = fs.fstatSync(fd, { bigint: true });
    if (!observed.isFile() || observed.dev !== expected.dev || observed.ino !== expected.ino
        || observed.size !== expected.size) fail();
    const hash = crypto.createHash("sha256");
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0;
    while (offset < Number(observed.size)) {
      const length = fs.readSync(fd, buffer, 0, Math.min(buffer.length, Number(observed.size) - offset), offset);
      if (length <= 0) fail();
      hash.update(buffer.subarray(0, length));
      offset += length;
    }
    return hash.digest("hex");
  } finally { fs.closeSync(fd); }
};
for (const treeName of treeNames) {
  exactKeys(config.trees[treeName], ["root", "expected_manifest_digest"]);
  const root = config.trees[treeName].root;
  if (!safePath(root) || !root.startsWith(config.task_root + "/") || fs.realpathSync(root) !== root) fail();
  const manifest = crypto.createHash("sha256");
  let entryCount = 0;
  let fileBytes = 0;
  const addRecord = (record) => manifest.update(JSON.stringify(canonical(record)) + "\n", "utf8");
  const walk = (current, relative, depth) => {
    if (depth > 64 || ++entryCount > config.limits.max_entries_per_tree) fail();
    const metadata = fs.lstatSync(current, { bigint: true });
    const ownerUid = Number(metadata.uid);
    const ownerGid = Number(metadata.gid);
    const mode = modeString(metadata);
    if (ownerUid !== 0 || ownerGid !== 0) fail();
    const base = { path: relative, device: String(metadata.dev), inode: String(metadata.ino),
      owner_uid: ownerUid, owner_gid: ownerGid, mode, size: Number(metadata.size) };
    if (metadata.isDirectory() && !metadata.isSymbolicLink()) {
      if ((Number(metadata.mode) & 0o022) !== 0) fail();
      addRecord({ ...base, kind: "DIRECTORY" });
      const names = fs.readdirSync(current);
      names.sort((left, right) => left < right ? -1 : left > right ? 1 : 0);
      for (const name of names) {
        if (name === "." || name === ".." || name.length === 0 || name.length > 255
            || !/^[\x20-\x7e]+$/.test(name) || name.includes("/") || name.includes("\\")) fail();
        walk(current + "/" + name, relative === "." ? name : relative + "/" + name, depth + 1);
      }
      return;
    }
    if (metadata.isFile()) {
      if ((Number(metadata.mode) & 0o022) !== 0 || Number(metadata.size) > config.limits.max_single_file_bytes) fail();
      fileBytes += Number(metadata.size);
      if (!Number.isSafeInteger(fileBytes) || fileBytes > config.limits.max_file_bytes_per_tree) fail();
      addRecord({ ...base, kind: "FILE", sha256: hashFile(current, metadata) });
      return;
    }
    if (metadata.isSymbolicLink()) {
      const target = fs.readlinkSync(current);
      if (target.length === 0 || target.length > 4096 || /[^\x20-\x7e]/.test(target)) fail();
      const resolved = fs.realpathSync(current);
      if (resolved !== root && !resolved.startsWith(root + "/")) fail();
      addRecord({ ...base, kind: "SYMLINK", link_target: target });
      return;
    }
    fail();
  };
  walk(root, ".", 0);
  const manifestDigest = "immutable-tree-manifest:sha256:" + manifest.digest("hex");
  if (config.enforce_expected && manifestDigest !== config.trees[treeName].expected_manifest_digest) fail();
  treeResults[treeName] = { root, manifest_digest: manifestDigest, entry_count: entryCount, file_bytes: fileBytes };
}

const status = fs.readFileSync("/proc/self/status", "utf8");
const capMatch = /(?:^|\n)CapEff:\s*([A-Fa-f0-9]{16})(?:\n|$)/.exec(status);
if (!capMatch) fail();
const capEff = capMatch[1].toLowerCase();
const effectiveUid = process.getuid();
const effectiveGid = process.getgid();
const capabilitiesDigest = "linux-capabilities:sha256:" + hashJson({
  effective_uid: effectiveUid, effective_gid: effectiveGid, proc_status_cap_eff: capEff,
});
const sudo = cp.spawnSync(config.tools.sudo.path, ["-n", "true"], {
  encoding: "utf8", timeout: 5000, maxBuffer: 16384,
  env: { HOME: path.posix.dirname(config.task_root), PATH: "/usr/bin:/bin", LANG: "C.UTF-8", LC_ALL: "C.UTF-8" },
});
const sudoOut = sudo.stdout || "";
const sudoErr = sudo.stderr || "";
const sudoDenial = /(?:authentication|password).*(?:required|unavailable)|not allowed|not in the sudoers/i.test(sudoErr);
if (effectiveUid !== config.owner_uid || effectiveUid === 0 || BigInt("0x" + capEff) !== 0n
    || sudo.error || sudo.signal !== null || sudo.status !== 1 || sudoOut.length !== 0
    || !sudoDenial || Buffer.byteLength(sudoErr, "utf8") < 1 || Buffer.byteLength(sudoErr, "utf8") > 16384) fail();
fs.writeSync(1, JSON.stringify({
  schema: "lattice.wsl2-immutable-observation-source/1.0",
  task_root: { path: config.task_root, device: String(taskMetadata.dev), inode: String(taskMetadata.ino),
    owner_uid: Number(taskMetadata.uid), owner_gid: Number(taskMetadata.gid), mode: modeString(taskMetadata), immutable: true },
  trees: treeResults,
  privilege: { effective_uid: effectiveUid, effective_gid: effectiveGid,
    effective_capabilities_digest: capabilitiesDigest, capabilities_empty: true,
    noninteractive_root_unavailable: true, sudo_denial_recognized: sudoDenial,
    sudo_exit_code: sudo.status, sudo_stdout_bytes: Buffer.byteLength(sudoOut, "utf8"),
    sudo_stderr_bytes: Buffer.byteLength(sudoErr, "utf8"),
    sudo_stdout_sha256: crypto.createHash("sha256").update(sudoOut, "utf8").digest("hex"),
    sudo_stderr_sha256: crypto.createHash("sha256").update(sudoErr, "utf8").digest("hex") },
  bounds: config.limits,
}) + "\n");
`;

const TECHNICAL_PROBE_SOURCE = String.raw`
const fs = require("node:fs");
const cp = require("node:child_process");
const crypto = require("node:crypto");
const config = JSON.parse(process.argv[1]);
const childEnvironment = { ...process.env, HOME: config.home, TMPDIR: config.temp,
  npm_config_cache: config.npm_cache, CARGO_HOME: config.cargo_home,
  CARGO_TARGET_DIR: config.cargo_target_dir };
let commandSequence = 0;
const commandEvidence = [];
const openedTools = [];
const exactKeys = (value, expected, rejection = "TOOL_IDENTITY_REJECTED") => {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(rejection);
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    throw new Error(rejection);
  }
};
const readNamespaceMap = (name, expectedInsideId) => {
  const bytes = fs.readFileSync("/proc/self/" + name, "utf8");
  if (Buffer.byteLength(bytes, "utf8") > 4096) throw new Error("SANDBOX_NAMESPACE_REJECTED:" + name + ":BOUND");
  const lines = bytes.trim().split("\n").map((line) => line.trim().split(/\s+/u).map(Number));
  if (lines.length !== 1 || lines[0].length !== 3 || lines[0].some((value) => !Number.isSafeInteger(value))
      || lines[0][0] !== expectedInsideId || lines[0][2] !== 1) {
    throw new Error("SANDBOX_NAMESPACE_REJECTED:" + name + ":MAP");
  }
  return { bytes, digest: crypto.createHash("sha256").update(bytes, "utf8").digest("hex") };
};
exactKeys(config, ["cwd", "fence", "home", "temp", "npm_cache", "cargo_home", "cargo_target_dir",
  "git_config_global", "git_hooks", "git", "npm", "cargo", "rustc", "rustdoc", "setsid", "node",
  "execution_uid", "execution_gid", "tool_identities", "command_timeout_ms", "command_output_limit_bytes"]);
if (!Number.isSafeInteger(config.execution_uid) || config.execution_uid <= 0
    || !Number.isSafeInteger(config.execution_gid) || config.execution_gid <= 0
    || process.getuid() !== config.execution_uid || process.getgid() !== config.execution_gid) {
  throw new Error("SANDBOX_NAMESPACE_REJECTED:PROCESS_IDENTITY");
}
const uidMap = readNamespaceMap("uid_map", config.execution_uid);
const gidMap = readNamespaceMap("gid_map", config.execution_gid);
const overflowUid = Number(fs.readFileSync("/proc/sys/kernel/overflowuid", "utf8").trim());
const overflowGid = Number(fs.readFileSync("/proc/sys/kernel/overflowgid", "utf8").trim());
if (!Number.isSafeInteger(overflowUid) || overflowUid <= config.execution_uid
    || !Number.isSafeInteger(overflowGid) || overflowGid <= config.execution_gid) {
  throw new Error("SANDBOX_NAMESPACE_REJECTED:OVERFLOW_IDENTITY");
}
const sandboxNamespace = {
  schema: "lattice.wsl2-user-namespace/1.0",
  process_uid: process.getuid(), process_gid: process.getgid(),
  uid_map_sha256: uidMap.digest, gid_map_sha256: gidMap.digest,
  root_owner_sandbox_uid: overflowUid, root_owner_sandbox_gid: overflowGid,
};
const openTool = (name, identity) => {
  const reject = (reason) => new Error("TOOL_IDENTITY_REJECTED:" + name + ":" + reason);
  exactKeys(identity, ["path", "version", "sha256", "owner_uid"], reject("SHAPE").message);
  if (typeof identity.path !== "string" || !identity.path.startsWith("/")
      || !/^[a-f0-9]{64}$/u.test(identity.sha256)
      || !Number.isSafeInteger(identity.owner_uid) || identity.owner_uid < 0
      || !(identity.version === null || (typeof identity.version === "string" && identity.version.length > 0))) {
    throw reject("INPUT");
  }
  let fd;
  try {
    fd = fs.openSync(identity.path, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW || 0));
  } catch {
    throw reject("OPEN");
  }
  try {
    const metadata = fs.fstatSync(fd, { bigint: true });
    const mode = Number(metadata.mode & 0o7777n);
    const sandboxOwnerUid = Number(metadata.uid);
    const sandboxOwnerGid = Number(metadata.gid);
    const ownerMatches = identity.owner_uid === 0
      ? sandboxOwnerUid === sandboxNamespace.root_owner_sandbox_uid
        && sandboxOwnerGid === sandboxNamespace.root_owner_sandbox_gid
      : sandboxOwnerUid === identity.owner_uid;
    if (!metadata.isFile() || !ownerMatches || (mode & 0o022) !== 0
        || metadata.size <= 0n || metadata.size > 536870912n) throw reject("METADATA");
    const hash = crypto.createHash("sha256");
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let position = 0;
    while (position < Number(metadata.size)) {
      const length = Math.min(buffer.length, Number(metadata.size) - position);
      const read = fs.readSync(fd, buffer, 0, length, position);
      if (read <= 0) throw reject("READ");
      hash.update(buffer.subarray(0, read));
      position += read;
    }
    const observedSha256 = hash.digest("hex");
    if (position !== Number(metadata.size) || observedSha256 !== identity.sha256) {
      throw reject("DIGEST");
    }
    const sealed = {
      name, fd, fd_path: "/proc/self/fd/" + String(fd), path: identity.path,
      version: identity.version, sha256: observedSha256, owner_uid: identity.owner_uid,
      sandbox_owner_uid: sandboxOwnerUid, sandbox_owner_gid: sandboxOwnerGid,
      mode: metadata.mode.toString(8), device: String(metadata.dev), inode: String(metadata.ino),
      size: Number(metadata.size),
    };
    openedTools.push(sealed);
    return sealed;
  } catch (error) {
    fs.closeSync(fd);
    throw error;
  }
};
const bounded = (label, executable, args, environment = childEnvironment, inheritedFds = []) => new Promise((resolve, reject) => {
  commandSequence += 1;
  let child;
  let timer;
  let settled = false;
  const stdoutChunks = [];
  const stderrChunks = [];
  let stdoutBytes = 0;
  let stderrBytes = 0;
  let timedOut = false;
  let boundExceeded = false;
  const finish = (error, value) => {
    if (settled) return;
    settled = true;
    clearTimeout(timer);
    if (error) reject(error); else resolve(value);
  };
  try {
    if (!executable || !Number.isSafeInteger(executable.fd) || executable.fd < 0) {
      throw new Error("TOOL_IDENTITY_REJECTED");
    }
    child = cp.spawn("/proc/self/fd/3", args, {
      cwd: config.cwd, env: environment,
      stdio: ["ignore", "pipe", "pipe", executable.fd, ...inheritedFds],
    });
  } catch (error) { finish(error); return; }
  timer = setTimeout(() => { timedOut = true; child.kill("SIGKILL"); }, config.command_timeout_ms);
  const capture = (chunks, streamName) => (chunk) => {
    const bytes = Buffer.from(chunk);
    if (streamName === "stdout") stdoutBytes += bytes.length; else stderrBytes += bytes.length;
    const total = streamName === "stdout" ? stdoutBytes : stderrBytes;
    if (total > config.command_output_limit_bytes) {
      boundExceeded = true;
      child.kill("SIGKILL");
      return;
    }
    chunks.push(bytes);
  };
  child.stdout.on("data", capture(stdoutChunks, "stdout"));
  child.stderr.on("data", capture(stderrChunks, "stderr"));
  child.once("error", (error) => finish(error));
  child.once("close", (code, signal) => {
    if (settled) return;
    const stdout = Buffer.concat(stdoutChunks);
    const stderr = Buffer.concat(stderrChunks);
    commandEvidence.push({
      sequence: commandSequence, label, stdout_bytes: stdoutBytes, stderr_bytes: stderrBytes,
      stdout_sha256: crypto.createHash("sha256").update(stdout).digest("hex"),
      stderr_sha256: crypto.createHash("sha256").update(stderr).digest("hex"),
      exit_code: code, signal, timed_out: timedOut, output_bound_exceeded: boundExceeded,
    });
    if (timedOut) finish(new Error("PROBE_COMMAND_TIMEOUT"));
    else if (boundExceeded) finish(new Error("PROBE_OUTPUT_BOUND_EXCEEDED"));
    else if (code !== 0 || signal !== null) finish(new Error("PROBE_COMMAND_FAILED"));
    else finish(null, stdout.toString("utf8").replaceAll("\r", "").trimEnd());
  });
});
const boundedRegularFile = (label, executable, args, environment = childEnvironment, inheritedFds = []) => new Promise((resolve, reject) => {
  commandSequence += 1;
  const nonce = crypto.randomBytes(16).toString("hex");
  const prefix = config.temp + "/.lattice-wsl2-probe-" + nonce;
  const stdoutPath = prefix + ".stdout";
  const stderrPath = prefix + ".stderr";
  let stdoutFd;
  let stderrFd;
  let child;
  let timer;
  let poll;
  let settled = false;
  let timedOut = false;
  let boundExceeded = false;
  const closeAndUnlink = () => {
    for (const fd of [stdoutFd, stderrFd]) {
      if (Number.isSafeInteger(fd)) {
        try { fs.closeSync(fd); } catch (error) { if (error?.code !== "EBADF") throw error; }
      }
    }
    for (const file of [stdoutPath, stderrPath]) {
      try { fs.unlinkSync(file); } catch (error) { if (error?.code !== "ENOENT") throw error; }
    }
  };
  const finish = (error, value) => {
    if (settled) return;
    settled = true;
    clearTimeout(timer);
    clearInterval(poll);
    try { closeAndUnlink(); } catch (cleanupError) { error ||= cleanupError; }
    if (error) reject(error); else resolve(value);
  };
  const readBounded = (fd, size) => {
    const bytes = Buffer.alloc(size);
    let offset = 0;
    while (offset < size) {
      const read = fs.readSync(fd, bytes, offset, size - offset, offset);
      if (read <= 0) throw new Error("PROBE_OUTPUT_READ_REJECTED");
      offset += read;
    }
    return bytes;
  };
  try {
    if (!executable || !Number.isSafeInteger(executable.fd) || executable.fd < 0) {
      throw new Error("TOOL_IDENTITY_REJECTED");
    }
    const flags = fs.constants.O_CREAT | fs.constants.O_EXCL | fs.constants.O_RDWR
      | (fs.constants.O_NOFOLLOW || 0);
    stdoutFd = fs.openSync(stdoutPath, flags, 0o600);
    stderrFd = fs.openSync(stderrPath, flags, 0o600);
    child = cp.spawn("/proc/self/fd/3", args, {
      cwd: config.cwd, env: environment,
      stdio: ["ignore", stdoutFd, stderrFd, executable.fd, ...inheritedFds],
    });
  } catch (error) { finish(error); return; }
  timer = setTimeout(() => { timedOut = true; child.kill("SIGKILL"); }, config.command_timeout_ms);
  poll = setInterval(() => {
    try {
      if (fs.fstatSync(stdoutFd).size > config.command_output_limit_bytes
          || fs.fstatSync(stderrFd).size > config.command_output_limit_bytes) {
        boundExceeded = true;
        child.kill("SIGKILL");
      }
    } catch (error) { child.kill("SIGKILL"); finish(error); }
  }, 25);
  poll.unref();
  child.once("error", (error) => finish(error));
  child.once("close", (code, signal) => {
    if (settled) return;
    try {
      const stdoutSize = fs.fstatSync(stdoutFd).size;
      const stderrSize = fs.fstatSync(stderrFd).size;
      if (stdoutSize > config.command_output_limit_bytes || stderrSize > config.command_output_limit_bytes) {
        boundExceeded = true;
      }
      const stdout = boundExceeded ? Buffer.alloc(0) : readBounded(stdoutFd, stdoutSize);
      const stderr = boundExceeded ? Buffer.alloc(0) : readBounded(stderrFd, stderrSize);
      commandEvidence.push({
        sequence: commandSequence, label, stdout_bytes: stdoutSize, stderr_bytes: stderrSize,
        stdout_sha256: crypto.createHash("sha256").update(stdout).digest("hex"),
        stderr_sha256: crypto.createHash("sha256").update(stderr).digest("hex"),
        exit_code: code, signal, timed_out: timedOut, output_bound_exceeded: boundExceeded,
      });
      if (timedOut) finish(new Error("PROBE_COMMAND_TIMEOUT"));
      else if (boundExceeded) finish(new Error("PROBE_OUTPUT_BOUND_EXCEEDED"));
      else if (code !== 0 || signal !== null) finish(new Error("PROBE_COMMAND_FAILED"));
      else finish(null, stdout.toString("utf8").replaceAll("\r", "").trimEnd());
    } catch (error) { finish(error); }
  });
});
(async () => {
exactKeys(config.tool_identities, ["controller", "node", "npm", "cargo", "rustc", "rustdoc", "git", "setsid"]);
const tools = Object.fromEntries(Object.entries(config.tool_identities).map(([name, identity]) => (
  [name, openTool(name, identity)]
)));
if (process.version !== tools.node.version) throw new Error("NODE_RUNTIME_VERSION_MISMATCH");
const sentinel = config.cwd + "/.lattice-wsl2-preflight-" + config.fence.slice(0, 16);
const payload = Buffer.from(config.fence + "\n", "utf8");
let writeDigest;
try {
  const fd = fs.openSync(sentinel, "wx", 0o600);
  try { fs.writeFileSync(fd, payload); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
  const readBack = fs.readFileSync(sentinel);
  if (!readBack.equals(payload)) throw new Error("PROBE_WRITE_MISMATCH");
  writeDigest = crypto.createHash("sha256").update(readBack).digest("hex");
} finally { try { fs.unlinkSync(sentinel); } catch (error) { if (!error || error.code !== "ENOENT") throw error; } }
const gitEnvironment = { HOME: config.home, TMPDIR: config.temp,
  GIT_CONFIG_GLOBAL: config.git_config_global, NO_COLOR: "1", CI: "1",
  GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_COUNT: "0", GIT_TERMINAL_PROMPT: "0",
  GIT_OPTIONAL_LOCKS: "0", GIT_ATTR_NOSYSTEM: "1", PATH: "/usr/bin:/bin",
  LANG: "C.UTF-8", LC_ALL: "C.UTF-8" };
const gitPrefix = ["--no-pager", "--no-replace-objects", "--literal-pathspecs", "-c",
  "core.hooksPath=" + config.git_hooks, "-c", "core.fsmonitor=false", "-c",
  "protocol.allow=never", "-c", "commit.gpgSign=false", "-C", config.cwd];
const closedGit = (label, args) => bounded(label, tools.git, [...gitPrefix, ...args], gitEnvironment);
const gitTop = await closedGit("git-top", ["rev-parse", "--show-toplevel"]);
const gitDirectory = await closedGit("git-dir", ["rev-parse", "--absolute-git-dir"]);
const gitCommon = await closedGit("git-common", ["rev-parse", "--path-format=absolute", "--git-common-dir"]);
const gitHead = await closedGit("git-head", ["rev-parse", "--verify", "HEAD^{commit}"]);
const gitStatus = await closedGit("git-status", ["status", "--porcelain=v1"]);
const nodeVersion = await bounded("node-version", tools.node, ["--version"]);
const npmVersion = await boundedRegularFile("npm-version", tools.node,
  ["/proc/self/fd/4", "--version"], childEnvironment, [tools.npm.fd]);
const npmPackage = await boundedRegularFile("npm-package", tools.node,
  ["/proc/self/fd/4", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "pkg", "get", "name"],
  childEnvironment, [tools.npm.fd]);
const cargoEnvironment = { ...childEnvironment, CARGO_NET_OFFLINE: "true",
  RUSTC: "/proc/self/fd/4", RUSTDOC: "/proc/self/fd/5" };
const cargoVersion = await bounded("cargo-version", tools.cargo, ["-Vv"], cargoEnvironment,
  [tools.rustc.fd, tools.rustdoc.fd]);
const cargoMetadata = await bounded("cargo-metadata", tools.cargo,
  ["metadata", "--locked", "--offline", "--no-deps", "--format-version", "1"], cargoEnvironment,
  [tools.rustc.fd, tools.rustdoc.fd]);
const rustcVersion = await bounded("rustc-version", tools.rustc, ["-Vv"]);
const rustdocVersion = await bounded("rustdoc-version", tools.rustdoc, ["--version"]);
await new Promise((resolve, reject) => {
  const daemon = cp.spawn("/proc/self/fd/3",
    ["-f", "/proc/self/fd/4", "-e", "setTimeout(() => {}, 60000)"],
    { cwd: config.cwd, env: childEnvironment,
      stdio: ["ignore", "ignore", "ignore", tools.setsid.fd, tools.node.fd] });
  const timer = setTimeout(() => { daemon.kill("SIGKILL"); reject(new Error("DAEMON_ESCAPE_PROBE_TIMEOUT")); }, 10000);
  daemon.once("error", (error) => { clearTimeout(timer); reject(error); });
  daemon.once("close", (code, signal) => {
    clearTimeout(timer);
    if (code === 0 && signal === null) resolve();
    else reject(new Error("DAEMON_ESCAPE_PROBE_FAILED"));
  });
});
fs.writeSync(1, JSON.stringify({
  schema: "lattice.wsl2-technical-probe/1.1", status: "PASS", cwd: config.cwd,
  sandbox_namespace: sandboxNamespace,
  write_probe_sha256: writeDigest, git: { top_level: gitTop, git_dir: gitDirectory, common_dir: gitCommon,
    head: gitHead, status: gitStatus }, node: { path: config.node, version: nodeVersion },
  npm: { version: npmVersion, package_name_json: npmPackage },
  cargo: { version_verbose: cargoVersion, metadata_sha256: crypto.createHash("sha256").update(cargoMetadata).digest("hex") },
  rustc: { version_verbose: rustcVersion }, rustdoc: { version: rustdocVersion },
  daemon_escape_probe: { attempted: true, mechanism: "setsid-fork" },
  tool_input_identities: Object.fromEntries(openedTools.map((tool) => [tool.name, {
    path: tool.path, version: tool.version, sha256: tool.sha256, owner_uid: tool.owner_uid,
    sandbox_owner_uid: tool.sandbox_owner_uid, sandbox_owner_gid: tool.sandbox_owner_gid,
    mode: tool.mode, device: tool.device, inode: tool.inode, size: tool.size,
  }])),
  command_evidence: commandEvidence,
  effect_counters: { account_read: 0, thread_start: 0, turn_start: 0, provider_effect_count: 0 },
}) + "\n");
})().finally(() => {
  for (const tool of openedTools.reverse()) {
    try { fs.closeSync(tool.fd); } catch (error) { if (error?.code !== "EBADF") throw error; }
  }
}).catch((error) => {
  fs.writeSync(2, String(error?.stack || error) + "\n");
  process.exitCode = 1;
});
`;

export const WSL2_TECHNICAL_PROBE_SOURCE = TECHNICAL_PROBE_SOURCE;

const CGROUP_EXIT_PROBE_SOURCE = String.raw`
const fs = require("node:fs");
const cp = require("node:child_process");
const config = JSON.parse(process.argv[1]);
const show = cp.spawnSync(config.systemctl, ["--user", "show", config.unit,
  "--property=ActiveState", "--property=SubState", "--property=Result", "--property=ControlGroup",
  "--property=Delegate"],
  { encoding: "utf8", timeout: 10000, maxBuffer: 65536,
    env: { ...process.env, XDG_RUNTIME_DIR: config.runtime_dir } });
if (show.error || show.status !== 0) throw new Error("SYSTEMCTL_SHOW_FAILED");
const values = Object.fromEntries(show.stdout.replaceAll("\r", "").trim().split("\n").map((line) => {
  const index = line.indexOf("="); return [line.slice(0, index), line.slice(index + 1)];
}));
const cgroupPath = values.ControlGroup || config.cgroup_path;
const eventsPath = config.cgroup_mount + cgroupPath + "/cgroup.events";
let exists = true; let populated = null;
try {
  const events = fs.readFileSync(eventsPath, "utf8");
  const match = events.match(/(?:^|\n)populated\s+(\d+)(?:\n|$)/u);
  if (!match) throw new Error("CGROUP_EVENTS_INVALID");
  populated = Number(match[1]);
} catch (error) { if (error && error.code === "ENOENT") exists = false; else throw error; }
process.stdout.write(JSON.stringify({ unit: config.unit, active_state: values.ActiveState,
  sub_state: values.SubState, result: values.Result, cgroup_path: cgroupPath,
  delegate: values.Delegate, cgroup_exists: exists, populated }) + "\n");
`;

export const WSL2_CGROUP_EXIT_PROBE_SOURCE = CGROUP_EXIT_PROBE_SOURCE;

function validateCredentialFacts(facts, descriptor) {
  const code = "WSL2_PREFLIGHT_CREDENTIAL_SEAL_REJECTED";
  exactKeys(facts, [
    "config_regular_file", "config_sha256", "config_identity", "config_owner_matches",
    "keyring_only", "auth_json_absent", "shell_environment_policy",
  ], code);
  exactKeys(facts.config_identity, ["device", "inode", "owner_uid", "mode", "size"], code);
  exactKeys(facts.shell_environment_policy, [
    "inherit", "ignore_default_excludes", "include_only", "experimental_use_profile",
    "set_keys", "probe_effective_keys", "required_keys_present", "forbidden_keys_absent",
  ], code);
  ensure(facts.config_regular_file === true && facts.config_owner_matches === true, code);
  ensure(facts.keyring_only === true && facts.auth_json_absent === true, code);
  ensure(facts.shell_environment_policy.inherit === "all"
    && facts.shell_environment_policy.ignore_default_excludes === false
    && facts.shell_environment_policy.experimental_use_profile === false
    && canonicalJson(facts.shell_environment_policy.include_only)
      === canonicalJson(["HOME", "PATH", "LANG", "LC_ALL", "TERM", "COLORTERM"])
    && canonicalJson(facts.shell_environment_policy.set_keys) === "[]"
    && canonicalJson(facts.shell_environment_policy.probe_effective_keys)
      === canonicalJson(["HOME", "PATH", "LANG", "LC_ALL", "TERM", "COLORTERM"])
    && facts.shell_environment_policy.required_keys_present === true
    && facts.shell_environment_policy.forbidden_keys_absent === true, code);
  ensure(facts.config_sha256 === descriptor.linux.config_digest.slice(-64), code);
  ensure(facts.config_identity.owner_uid === descriptor.verification_toolchain.owner_uid, code);
  ensure(/^\d+$/u.test(facts.config_identity.device) && /^\d+$/u.test(facts.config_identity.inode), code);
  ensure(/^100[46]00$/u.test(facts.config_identity.mode), code);
  ensure(Number.isSafeInteger(facts.config_identity.size) && facts.config_identity.size > 0, code);
  return digest("credential-seal", {
    authority_ref: descriptor.credential_authority.authority_digest,
    config_sha256: facts.config_sha256,
    config_identity: facts.config_identity,
    keyring_only: true,
    auth_json_absent: true,
    shell_environment_policy: facts.shell_environment_policy,
  });
}

function validateIsolationFacts(facts, descriptor) {
  const code = "WSL2_PREFLIGHT_ISOLATION_REJECTED";
  exactKeys(facts, ["observations"], code);
  const toolchain = descriptor.verification_toolchain;
  const paths = [toolchain.isolation_root, toolchain.home_dir, toolchain.temp_dir,
    toolchain.npm_cache, toolchain.cargo_home, toolchain.cargo_target_dir];
  exactKeys(facts.observations, paths, code);
  for (const candidate of paths) {
    const observation = facts.observations[candidate];
    exactKeys(observation, ["realpath", "directory", "symlink", "owner_uid", "owner_matches", "mode", "device", "inode"], code);
    ensure(observation.realpath === candidate && observation.directory === true
      && observation.symlink === false && observation.owner_matches === true
      && observation.owner_uid === toolchain.owner_uid && observation.mode === "40700", code);
  }
  return { root: toolchain.isolation_root, owner_uid: toolchain.owner_uid, observations: facts.observations };
}

function validateTechnicalProbe(probe, descriptor) {
  const code = "WSL2_PREFLIGHT_TECHNICAL_PROBE_REJECTED";
  exactKeys(probe, [
    "schema", "status", "cwd", "write_probe_sha256", "git", "node", "npm", "cargo",
    "rustc", "rustdoc", "daemon_escape_probe", "tool_input_identities", "command_evidence",
    "sandbox_namespace", "effect_counters",
  ], code);
  ensure(probe.schema === "lattice.wsl2-technical-probe/1.1" && probe.status === "PASS", code);
  ensure(probe.cwd === descriptor.linux.cwd && HEX_64.test(probe.write_probe_sha256), code);
  exactKeys(probe.sandbox_namespace, [
    "schema", "process_uid", "process_gid", "uid_map_sha256", "gid_map_sha256",
    "root_owner_sandbox_uid", "root_owner_sandbox_gid",
  ], code);
  ensure(probe.sandbox_namespace.schema === "lattice.wsl2-user-namespace/1.0"
    && probe.sandbox_namespace.process_uid === descriptor.verification_toolchain.owner_uid
    && probe.sandbox_namespace.process_gid === descriptor.verification_toolchain.owner_uid
    && HEX_64.test(probe.sandbox_namespace.uid_map_sha256)
    && HEX_64.test(probe.sandbox_namespace.gid_map_sha256)
    && Number.isSafeInteger(probe.sandbox_namespace.root_owner_sandbox_uid)
    && probe.sandbox_namespace.root_owner_sandbox_uid > probe.sandbox_namespace.process_uid
    && Number.isSafeInteger(probe.sandbox_namespace.root_owner_sandbox_gid)
    && probe.sandbox_namespace.root_owner_sandbox_gid > probe.sandbox_namespace.process_gid, code);
  exactKeys(probe.git, ["top_level", "git_dir", "common_dir", "head", "status"], code);
  ensure(probe.git.top_level === descriptor.linux.cwd && probe.git.head === descriptor.linux.repository_head
    && canonicalLinuxHomePath(probe.git.git_dir) && canonicalLinuxHomePath(probe.git.common_dir)
    && probe.git.common_dir.startsWith(`${descriptor.verification_toolchain.task_root}/`)
    && probe.git.git_dir.startsWith(`${probe.git.common_dir}/worktrees/`)
    && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/u.test(
      probe.git.git_dir.slice(`${probe.git.common_dir}/worktrees/`.length),
    )
    && repositoryIdentity(descriptor, probe.git) === descriptor.linux.repository_identity, code);
  exactKeys(probe.node, ["path", "version"], code);
  ensure(probe.node.path === descriptor.linux.node_path && probe.node.version === descriptor.linux.node_version, code);
  exactKeys(probe.npm, ["version", "package_name_json"], code);
  ensure(probe.npm.version === descriptor.verification_toolchain.npm.version, code);
  ensure(typeof probe.npm.package_name_json === "string" && probe.npm.package_name_json.length > 0, code);
  exactKeys(probe.cargo, ["version_verbose", "metadata_sha256"], code);
  ensure(firstLine(probe.cargo.version_verbose) === descriptor.verification_toolchain.cargo.version, code);
  ensure(HEX_64.test(probe.cargo.metadata_sha256), code);
  exactKeys(probe.rustc, ["version_verbose"], code);
  ensure(firstLine(probe.rustc.version_verbose) === descriptor.verification_toolchain.rustc.version, code);
  exactKeys(probe.rustdoc, ["version"], code);
  ensure(probe.rustdoc.version === descriptor.verification_toolchain.rustdoc.version, code);
  exactKeys(probe.daemon_escape_probe, ["attempted", "mechanism"], code);
  ensure(probe.daemon_escape_probe.attempted === true
    && probe.daemon_escape_probe.mechanism === "setsid-fork", code);
  const toolchain = descriptor.verification_toolchain;
  const expectedTools = {
    controller: { ...descriptor.process_fence.supervisor_bootstrap_node, owner_uid: 0 },
    node: { path: descriptor.linux.node_path, version: descriptor.linux.node_version,
      sha256: descriptor.linux.node_sha256, owner_uid: 0 },
    npm: { ...toolchain.npm, owner_uid: 0 },
    cargo: { ...toolchain.cargo, owner_uid: 0 },
    rustc: { ...toolchain.rustc, owner_uid: 0 },
    rustdoc: { ...toolchain.rustdoc, owner_uid: 0 },
    git: { path: descriptor.linux.git_path, version: descriptor.linux.git_version,
      sha256: descriptor.linux.git_sha256, owner_uid: 0 },
    setsid: { path: descriptor.linux.setsid_path, version: null,
      sha256: descriptor.linux.setsid_sha256, owner_uid: 0 },
  };
  exactKeys(probe.tool_input_identities, Object.keys(expectedTools), code);
  for (const [name, expected] of Object.entries(expectedTools)) {
    const observed = probe.tool_input_identities[name];
    exactKeys(observed, [
      "path", "version", "sha256", "owner_uid", "sandbox_owner_uid", "sandbox_owner_gid",
      "mode", "device", "inode", "size",
    ], code);
    const mode = typeof observed.mode === "string" && /^[0-7]+$/u.test(observed.mode)
      ? Number.parseInt(observed.mode, 8) : 0;
    ensure(observed.path === expected.path && observed.version === expected.version
      && observed.sha256 === expected.sha256 && observed.owner_uid === expected.owner_uid
      && observed.sandbox_owner_uid === probe.sandbox_namespace.root_owner_sandbox_uid
      && observed.sandbox_owner_gid === probe.sandbox_namespace.root_owner_sandbox_gid
      && (mode & 0o170000) === 0o100000 && (mode & 0o022) === 0
      && typeof observed.device === "string" && /^[0-9]+$/u.test(observed.device)
      && typeof observed.inode === "string" && /^[0-9]+$/u.test(observed.inode)
      && Number.isSafeInteger(observed.size) && observed.size > 0 && observed.size <= 536_870_912,
    code);
  }
  const commandLabels = [
    "git-top", "git-dir", "git-common", "git-head", "git-status", "node-version",
    "npm-version", "npm-package", "cargo-version", "cargo-metadata", "rustc-version", "rustdoc-version",
  ];
  ensure(Array.isArray(probe.command_evidence) && probe.command_evidence.length === commandLabels.length, code);
  for (const [index, evidence] of probe.command_evidence.entries()) {
    exactKeys(evidence, [
      "sequence", "label", "stdout_bytes", "stderr_bytes", "stdout_sha256", "stderr_sha256",
      "exit_code", "signal", "timed_out", "output_bound_exceeded",
    ], code);
    ensure(evidence.sequence === index + 1 && evidence.label === commandLabels[index]
      && Number.isSafeInteger(evidence.stdout_bytes) && evidence.stdout_bytes >= 0
      && evidence.stdout_bytes <= 262_144 && Number.isSafeInteger(evidence.stderr_bytes)
      && evidence.stderr_bytes >= 0 && evidence.stderr_bytes <= 262_144
      && HEX_64.test(evidence.stdout_sha256) && HEX_64.test(evidence.stderr_sha256)
      && evidence.exit_code === 0 && evidence.signal === null
      && evidence.timed_out === false && evidence.output_bound_exceeded === false, code);
  }
  exactKeys(probe.effect_counters, ["account_read", "thread_start", "turn_start", "provider_effect_count"], code);
  for (const value of Object.values(probe.effect_counters)) ensure(Number.isSafeInteger(value) && value >= 0, code);
  ensure(probe.effect_counters.account_read === 0 && probe.effect_counters.thread_start === 0
    && probe.effect_counters.turn_start === 0 && probe.effect_counters.provider_effect_count === 0,
  "WSL2_PREFLIGHT_PROVIDER_EFFECT_DETECTED");
  return probe;
}

function validateSupervisorRecords(stderr, descriptor, context, unit, credentialSeal) {
  const marker = framedJson(stderr, WSL2_PROCESS_MARKER_SCHEMA, "WSL2_PREFLIGHT_SUPERVISOR_MARKER_REJECTED");
  const receipt = validateWsl2SubtreeExitReceipt(
    framedJson(stderr, WSL2_SUBTREE_EXIT_SCHEMA, "WSL2_PREFLIGHT_SUPERVISOR_RECEIPT_REJECTED"),
    descriptor,
    "PREFLIGHT",
  );
  const ownerUid = descriptor.verification_toolchain.owner_uid;
  const canonicalCgroup = `/user.slice/user-${ownerUid}.slice/user@${ownerUid}.service/app.slice/${unit}`;
  for (const record of [marker, receipt]) {
    ensure(record.fence === context.processFence && record.unit === unit, "WSL2_PREFLIGHT_CGROUP_V2_FENCE_REJECTED");
    ensure(record.execution_environment_ref === descriptor.identity_digest, "WSL2_PREFLIGHT_CGROUP_V2_FENCE_REJECTED");
    ensure(record.credential_seal_digest === credentialSeal, "WSL2_PREFLIGHT_CREDENTIAL_SEAL_REJECTED");
    ensure(record.cgroup_path === canonicalCgroup, "WSL2_PREFLIGHT_CGROUP_V2_FENCE_REJECTED");
  }
  ensure(marker.cgroup_version === 2 && marker.delegated === false, "WSL2_PREFLIGHT_CGROUP_V2_FENCE_REJECTED");
  ensure(receipt.zero_descendants === true && receipt.credential_seal_intact === true,
    "WSL2_PREFLIGHT_CGROUP_V2_FENCE_REJECTED");
  return { marker, receipt };
}

function validateOuterExit(facts, unit, cgroupPath) {
  const code = "WSL2_PREFLIGHT_CGROUP_V2_FENCE_REJECTED";
  exactKeys(facts, ["unit", "active_state", "sub_state", "result", "cgroup_path", "delegate", "cgroup_exists", "populated"], code);
  ensure(facts.unit === unit && facts.active_state === "inactive" && facts.sub_state === "dead", code);
  ensure(facts.result === "success" && facts.cgroup_path === cgroupPath && facts.delegate === "no", code);
  ensure((facts.cgroup_exists === false && facts.populated === null)
    || (facts.cgroup_exists === true && facts.populated === 0), code);
  return facts;
}

function repositoryIdentity(descriptor, git) {
  return digest("repository", {
    distribution_identity_ref: descriptor.distribution_identity.identity_digest,
    cwd: descriptor.linux.cwd,
    top_level: git.top_level,
    git_dir: git.git_dir,
    common_git_dir: git.common_dir,
    head: git.head,
    status: git.status,
    git_path: descriptor.linux.git_path,
    git_version: descriptor.linux.git_version,
    git_sha256: descriptor.linux.git_sha256,
  });
}

function mappingIdentity(descriptor) {
  return digest("path-mapping", {
    distribution: descriptor.distribution,
    windows_path: descriptor.path_mapping.windows_path,
    linux_path: descriptor.path_mapping.linux_path,
    repository_identity: descriptor.linux.repository_identity,
    repository_head: descriptor.linux.repository_head,
  });
}

function preflightUnit(descriptor, fence) {
  return `${descriptor.process_fence.unit_prefix}-preflight-${fence.slice(0, 12)}.service`;
}

function technicalProbeConfig(descriptor, context) {
  const toolchain = descriptor.verification_toolchain;
  const immutable = (tool) => ({ ...tool, owner_uid: 0 });
  return {
    cwd: descriptor.linux.cwd,
    fence: context.processFence,
    home: toolchain.home_dir,
    temp: toolchain.temp_dir,
    npm_cache: toolchain.npm_cache,
    cargo_home: toolchain.cargo_home,
    cargo_target_dir: toolchain.cargo_target_dir,
    git_config_global: `${toolchain.home_dir}/.gitconfig`,
    git_hooks: `${toolchain.temp_dir}/git-hooks`,
    git: descriptor.linux.git_path,
    npm: toolchain.npm.path,
    cargo: toolchain.cargo.path,
    rustc: toolchain.rustc.path,
    rustdoc: toolchain.rustdoc.path,
    setsid: descriptor.linux.setsid_path,
    node: descriptor.linux.node_path,
    execution_uid: toolchain.owner_uid,
    execution_gid: toolchain.owner_uid,
    tool_identities: {
      controller: { ...descriptor.process_fence.supervisor_bootstrap_node, owner_uid: 0 },
      node: immutable({ path: descriptor.linux.node_path, version: descriptor.linux.node_version,
        sha256: descriptor.linux.node_sha256 }),
      npm: immutable(toolchain.npm),
      cargo: immutable(toolchain.cargo),
      rustc: immutable(toolchain.rustc),
      rustdoc: immutable(toolchain.rustdoc),
      git: { path: descriptor.linux.git_path, version: descriptor.linux.git_version,
        sha256: descriptor.linux.git_sha256, owner_uid: 0 },
      setsid: { path: descriptor.linux.setsid_path, version: null,
        sha256: descriptor.linux.setsid_sha256, owner_uid: 0 },
    },
    command_timeout_ms: 120_000,
    command_output_limit_bytes: 262_144,
  };
}

function technicalLaunch(descriptor, context, credentialSeal) {
  const unit = preflightUnit(descriptor, context.processFence);
  const toolchain = descriptor.verification_toolchain;
  const fence = descriptor.process_fence;
  const fixedPath = "/usr/bin:/bin";
  const explicitEnvironment = [
    `HOME=${toolchain.home_dir}`,
    `TMPDIR=${toolchain.temp_dir}`, `npm_config_cache=${toolchain.npm_cache}`,
    `CARGO_HOME=${toolchain.cargo_home}`, `CARGO_TARGET_DIR=${toolchain.cargo_target_dir}`,
    `XDG_RUNTIME_DIR=${fence.user_runtime_dir}`,
    `PATH=${fixedPath}`, "LANG=C.UTF-8", "LC_ALL=C.UTF-8",
  ];
  const sandboxState = buildWsl2SandboxState(descriptor, {
    role: "PREFLIGHT",
    cwd: descriptor.linux.cwd,
    writableRoots: [
      descriptor.linux.cwd,
      toolchain.home_dir,
      toolchain.temp_dir,
      toolchain.npm_cache,
      toolchain.cargo_home,
      toolchain.cargo_target_dir,
    ],
    deniedRoots: [descriptor.linux.codex_home],
  });
  return {
    unit,
    program: fence.systemd_run_path,
    args: [
      "--user", "--wait", "--pipe", "--quiet", `--unit=${unit}`,
      "--property=Type=exec", "--property=KillMode=control-group", "--property=Delegate=no",
      "--property=RuntimeMaxSec=180", "--property=TimeoutStopSec=5s",
      `--setenv=HOME=${toolchain.home_dir}`,
      `--setenv=TMPDIR=${toolchain.temp_dir}`, `--setenv=npm_config_cache=${toolchain.npm_cache}`,
      `--setenv=CARGO_HOME=${toolchain.cargo_home}`, `--setenv=CARGO_TARGET_DIR=${toolchain.cargo_target_dir}`,
      `--setenv=XDG_RUNTIME_DIR=${fence.user_runtime_dir}`,
      `--setenv=PATH=${fixedPath}`, "--setenv=LANG=C.UTF-8", "--setenv=LC_ALL=C.UTF-8",
      "/usr/bin/env", "-i", ...explicitEnvironment,
      descriptor.linux.dbus_run_session_path, "--", fence.supervisor_bootstrap_node.path,
      "-e", WSL2_SUPERVISOR_BOOTSTRAP_SOURCE,
      descriptor.linux.supervisor_path, descriptor.linux.supervisor_sha256,
      "--role", "PREFLIGHT", "--fence", context.processFence, "--unit", unit,
      "--execution-environment-ref", descriptor.identity_digest,
      "--credential-authority-ref", descriptor.credential_authority.authority_digest,
      "--credential-seal-digest", credentialSeal,
      "--config-digest", descriptor.linux.config_digest,
      "--codex-home", descriptor.linux.codex_home,
      "--cwd", descriptor.linux.cwd,
      "--executable", descriptor.linux.launcher_path,
      "--executable-version", descriptor.linux.launcher_version,
      "--executable-sha256", descriptor.linux.launcher_sha256,
      "--verifier-tool", "NONE", "--verifier-tool-version", "NONE",
      "--verifier-tool-sha256", "NONE",
      "--node-runtime", descriptor.linux.node_path,
      "--node-runtime-version", descriptor.linux.node_version,
      "--node-runtime-sha256", descriptor.linux.node_sha256,
      "--rustc", "NONE", "--rustc-version", "NONE", "--rustc-sha256", "NONE",
      "--rustdoc", "NONE", "--rustdoc-version", "NONE", "--rustdoc-sha256", "NONE",
      "--keyring-daemon", descriptor.linux.keyring_daemon_path,
      "--keyring-daemon-sha256", descriptor.linux.keyring_daemon_sha256,
      "--keyring-library-path", descriptor.linux.keyring_library_path,
      "--keyring-library-manifest-digest", descriptor.linux.keyring_library_manifest_digest,
      "--sandbox-helper", toolchain.sandbox_helper.path,
      "--sandbox-helper-version", toolchain.sandbox_helper.version,
      "--sandbox-helper-sha256", toolchain.sandbox_helper.sha256,
      "--timeout-ms", "150000", "--stdout-limit-bytes", "262144", "--stderr-limit-bytes", "262144",
      "--attempt", String(context.attempt), "--retry-of", context.retryOf ?? "NONE",
      "--reconnect-of", context.reconnectOf ?? "NONE", "--",
      "sandbox", "--sandbox-state-json", canonicalJson(sandboxState),
      "--sandbox-state-disable-network", "--", descriptor.linux.node_path, "-e", TECHNICAL_PROBE_SOURCE,
      canonicalJson(technicalProbeConfig(descriptor, context)),
    ],
  };
}

/**
 * Runs the local, zero-model production preflight. This never starts an
 * app-server, thread, or turn. Connector account/read is completed separately
 * by completeWsl2ConnectorPreflight before provider effects are authorized.
 */
export async function preflightWsl2ExecutionEnvironment(untrusted, context = {}, dependencies = {}) {
  const descriptor = validateWsl2ExecutionEnvironment(structuredClone(untrusted));
  exactKeys(context, [
    "processFence", "taskRef", "attempt", "worktreeRef", "retryOf", "reconnectOf",
  ], "WSL2_PREFLIGHT_CONTEXT_REJECTED");
  ensure(HEX_64.test(context.processFence), "WSL2_PREFLIGHT_CONTEXT_REJECTED");
  ensure(context.taskRef === descriptor.verification_toolchain.task_ref, "WSL2_PREFLIGHT_CONTEXT_REJECTED");
  ensure(Number.isSafeInteger(context.attempt) && context.attempt >= 1
    && context.attempt <= MAX_WSL2_ATTEMPTS,
    "WSL2_PREFLIGHT_CONTEXT_REJECTED");
  ensure(typeof context.worktreeRef === "string" && /^worktree:sha256:[a-f0-9]{64}$/u.test(context.worktreeRef),
    "WSL2_PREFLIGHT_CONTEXT_REJECTED");
  ensure(context.retryOf === null || (typeof context.retryOf === "string" && TYPED.test(context.retryOf)),
    "WSL2_PREFLIGHT_CONTEXT_REJECTED");
  ensure(context.reconnectOf === null || (typeof context.reconnectOf === "string" && TYPED.test(context.reconnectOf)),
    "WSL2_PREFLIGHT_CONTEXT_REJECTED");
  ensure(context.retryOf === null || context.reconnectOf === null,
  "WSL2_PREFLIGHT_CONTEXT_REJECTED");
  const execFile = dependencies.execFile ?? execFileDefault;
  const readFile = dependencies.readFile ?? readFileDefault;
  const observeGatewayVersion = dependencies.observeGatewayVersion ?? defaultGatewayVersion;
  const run = async (program, args = [], options = {}) => {
    const toolchain = descriptor.verification_toolchain;
    const passiveRootTools = new Set([
      "/usr/bin/cat", "/usr/bin/realpath", "/usr/bin/sha256sum", "/usr/bin/stat", "/usr/bin/uname",
      descriptor.process_fence.supervisor_bootstrap_node.path,
      descriptor.process_fence.systemd_run_path, descriptor.process_fence.systemctl_path,
      descriptor.process_fence.immutable_probe_lsattr.path,
      descriptor.process_fence.noninteractive_root_probe.path,
      descriptor.verification_toolchain.sandbox_helper.path, descriptor.linux.git_path,
    ]);
    ensure(passiveRootTools.has(program), "WSL2_PREFLIGHT_UNSUPERVISED_TOOL_REJECTED");
    const result = await execFile(descriptor.gateway.windows_path, [
      "-d", descriptor.distribution, "--exec", "/usr/bin/env", "-i",
      `HOME=${toolchain.home_dir}`, `TMPDIR=${toolchain.temp_dir}`,
      `XDG_RUNTIME_DIR=${descriptor.process_fence.user_runtime_dir}`,
      "PATH=/usr/bin:/bin",
      "LANG=C.UTF-8", "LC_ALL=C.UTF-8", program, ...args,
    ], {
      encoding: "utf8", windowsHide: true, timeout: options.timeout ?? 30_000,
      maxBuffer: options.maxBuffer ?? 256 * 1024,
    });
    return { stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
  };
  const runClosedGit = async (args) => {
    const gitGlobal = `${descriptor.verification_toolchain.home_dir}/.gitconfig`;
    const gitHooks = `${descriptor.verification_toolchain.temp_dir}/git-hooks`;
    const gitEnvironment = [
      `HOME=${descriptor.verification_toolchain.home_dir}`,
      `TMPDIR=${descriptor.verification_toolchain.temp_dir}`,
      `GIT_CONFIG_GLOBAL=${gitGlobal}`,
      "NO_COLOR=1", "CI=1", "GIT_CONFIG_NOSYSTEM=1", "GIT_CONFIG_COUNT=0",
      "GIT_TERMINAL_PROMPT=0", "GIT_OPTIONAL_LOCKS=0", "GIT_ATTR_NOSYSTEM=1",
      "PATH=/usr/bin:/bin", "LANG=C.UTF-8", "LC_ALL=C.UTF-8",
    ];
    const fixed = [
      "--no-pager", "--no-replace-objects", "--literal-pathspecs", "-c",
      `core.hooksPath=${gitHooks}`, "-c", "core.fsmonitor=false", "-c",
      "protocol.allow=never", "-c", "commit.gpgSign=false", "-C", descriptor.linux.cwd,
    ];
    const result = await execFile(descriptor.gateway.windows_path, [
      "-d", descriptor.distribution, "--exec", "/usr/bin/env", "-i",
      ...gitEnvironment, descriptor.linux.git_path, ...fixed, ...args,
    ], {
      encoding: "utf8", windowsHide: true, timeout: 30_000, maxBuffer: 256 * 1024,
    });
    return { stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
  };

  const gatewayBytes = await readFile(descriptor.gateway.windows_path);
  ensure(sha256(gatewayBytes) === descriptor.gateway.sha256, "WSL2_PREFLIGHT_GATEWAY_DIGEST_MISMATCH");
  ensure(await observeGatewayVersion({ gatewayPath: descriptor.gateway.windows_path, execFile })
    === descriptor.gateway.version, "WSL2_PREFLIGHT_GATEWAY_VERSION_MISMATCH");

  const linux = descriptor.linux;
  const toolchain = descriptor.verification_toolchain;
  const fence = descriptor.process_fence;
  const pinnedFiles = [
    [linux.launcher_path, linux.launcher_sha256, "LAUNCHER"],
    [linux.node_path, linux.node_sha256, "NODE"], [linux.git_path, linux.git_sha256, "GIT"],
    [linux.supervisor_path, linux.supervisor_sha256, "SUPERVISOR"],
    [linux.dbus_run_session_path, linux.dbus_run_session_sha256, "DBUS"],
    [linux.setsid_path, linux.setsid_sha256, "SETSID"],
    [linux.keyring_daemon_path, linux.keyring_daemon_sha256, "KEYRING"],
    [fence.systemd_run_path, fence.systemd_run_sha256, "SYSTEMD_RUN"],
    [fence.systemctl_path, fence.systemctl_sha256, "SYSTEMCTL"],
    [fence.supervisor_bootstrap_node.path, fence.supervisor_bootstrap_node.sha256, "SUPERVISOR_BOOTSTRAP_NODE"],
    [toolchain.npm.path, toolchain.npm.sha256, "NPM"], [toolchain.cargo.path, toolchain.cargo.sha256, "CARGO"],
    [toolchain.rustc.path, toolchain.rustc.sha256, "RUSTC"],
    [toolchain.rustdoc.path, toolchain.rustdoc.sha256, "RUSTDOC"],
    [toolchain.sandbox_helper.path, toolchain.sandbox_helper.sha256, "SANDBOX_HELPER"],
  ];
  for (const [file, expected, label] of pinnedFiles) {
    const observed = firstDigest((await run("/usr/bin/sha256sum", [file])).stdout);
    ensure(observed === expected, `WSL2_PREFLIGHT_${label}_DIGEST_MISMATCH`);
  }

  const versionChecks = [
    [linux.git_path, ["--version"], linux.git_version, "GIT"],
    [fence.systemd_run_path, ["--version"], fence.systemd_run_version, "SYSTEMD_RUN"],
    [fence.systemctl_path, ["--version"], fence.systemctl_version, "SYSTEMCTL"],
    [fence.supervisor_bootstrap_node.path, ["--version"], fence.supervisor_bootstrap_node.version,
      "SUPERVISOR_BOOTSTRAP_NODE"],
    [toolchain.sandbox_helper.path, ["--version"], toolchain.sandbox_helper.version, "SANDBOX_HELPER"],
  ];
  for (const [program, args, expected, label] of versionChecks) {
    ensure(firstLine((await run(program, args)).stdout) === expected, `WSL2_PREFLIGHT_${label}_VERSION_MISMATCH`);
  }

  const keyringLibrary = onlyJsonLine((await run(fence.supervisor_bootstrap_node.path, [
    "-e", KEYRING_LIBRARY_MANIFEST_SOURCE, linux.keyring_library_path, "0",
  ])).stdout, "WSL2_PREFLIGHT_KEYRING_LIBRARY_REJECTED");
  exactKeys(keyringLibrary, ["schema", "digest"], "WSL2_PREFLIGHT_KEYRING_LIBRARY_REJECTED");
  ensure(keyringLibrary.schema === "lattice.wsl2-keyring-library-manifest/1.0"
    && keyringLibrary.digest === linux.keyring_library_manifest_digest,
  "WSL2_PREFLIGHT_KEYRING_LIBRARY_REJECTED");

  const [osReleaseOutput, osReleaseDigestOutput, kernelOutput, bootOutput] = await Promise.all([
    run("/usr/bin/cat", ["/etc/os-release"]), run("/usr/bin/sha256sum", ["/etc/os-release"]),
    run("/usr/bin/uname", ["-r"]), run("/usr/bin/cat", ["/proc/sys/kernel/random/boot_id"]),
  ]);
  const osRelease = osReleaseOutput.stdout;
  ensure(firstDigest(osReleaseDigestOutput.stdout) === descriptor.distribution_identity.os_release_sha256,
    "WSL2_PREFLIGHT_DISTRIBUTION_DIGEST_MISMATCH");
  const osFields = Object.fromEntries(osRelease.replaceAll("\r", "").split("\n").filter((line) => line.includes("="))
    .map((line) => { const index = line.indexOf("="); return [line.slice(0, index), line.slice(index + 1).replace(/^"|"$/gu, "")]; }));
  ensure(osFields.ID === descriptor.distribution_identity.os_id
    && osFields.VERSION_ID === descriptor.distribution_identity.os_version_id
    && osFields.VERSION_CODENAME === descriptor.distribution_identity.os_version_codename,
  "WSL2_PREFLIGHT_DISTRIBUTION_IDENTITY_MISMATCH");
  ensure(kernelOutput.stdout.trim() === descriptor.distribution_identity.kernel_release,
    "WSL2_PREFLIGHT_KERNEL_RELEASE_MISMATCH");
  const bootId = bootOutput.stdout.trim();
  ensure(/^[a-f0-9-]{36}$/u.test(bootId), "WSL2_PREFLIGHT_BOOT_ID_REJECTED");
  const bootIdDigest = `wsl-boot:sha256:${sha256(Buffer.from(bootId, "utf8"))}`;

  ensure(windowsWslPathToLinux(descriptor.path_mapping.windows_path, descriptor.distribution) === linux.cwd,
    "WSL2_PREFLIGHT_PATH_MAPPING_MISMATCH");
  const git = {
    top_level: (await runClosedGit(["rev-parse", "--show-toplevel"])).stdout.trimEnd(),
    git_dir: (await runClosedGit(["rev-parse", "--absolute-git-dir"])).stdout.trimEnd(),
    common_dir: (await runClosedGit(["rev-parse", "--path-format=absolute", "--git-common-dir"])).stdout.trimEnd(),
    head: (await runClosedGit(["rev-parse", "--verify", "HEAD^{commit}"])).stdout.trimEnd(),
    status: (await runClosedGit(["status", "--porcelain=v1"])).stdout.trimEnd(),
  };
  ensure(git.top_level === linux.cwd && HEX_40.test(git.head), "WSL2_PREFLIGHT_REPOSITORY_ROOT_MISMATCH");
  ensure(git.head === linux.repository_head, "WSL2_PREFLIGHT_REPOSITORY_HEAD_MISMATCH");
  linux.repository_identity = repositoryIdentity(descriptor, git);
  descriptor.path_mapping.digest = mappingIdentity(descriptor);
  descriptor.identity_digest = executionEnvironmentIdentity(descriptor);
  const environment = validateWsl2ExecutionEnvironment(descriptor);
  const immutable = await observeWsl2ImmutableExecutionState(environment, { run });

  const credentialFacts = onlyJsonLine((await run(fence.supervisor_bootstrap_node.path, [
    "-e", CREDENTIAL_PROBE_SOURCE, linux.codex_home, String(toolchain.owner_uid),
  ])).stdout, "WSL2_PREFLIGHT_CREDENTIAL_SEAL_REJECTED");
  const credentialSeal = validateCredentialFacts(credentialFacts, environment);

  const isolationPaths = [toolchain.isolation_root, toolchain.home_dir, toolchain.temp_dir,
    toolchain.npm_cache, toolchain.cargo_home, toolchain.cargo_target_dir];
  const isolationFacts = onlyJsonLine((await run(fence.supervisor_bootstrap_node.path, [
    "-e", ISOLATION_PROBE_SOURCE, canonicalJson(isolationPaths), String(toolchain.owner_uid),
  ])).stdout, "WSL2_PREFLIGHT_ISOLATION_REJECTED");
  const isolation = validateIsolationFacts(isolationFacts, environment);

  const launch = technicalLaunch(environment, context, credentialSeal);
  let technicalResult;
  try {
    technicalResult = await run(launch.program, launch.args, { timeout: 170_000, maxBuffer: MAX_OUTPUT });
  } catch (error) {
    try {
      await run(fence.systemctl_path, ["--user", "--no-block", "stop", launch.unit], {
        timeout: 15_000,
        maxBuffer: 65_536,
      });
    } catch {
      // RuntimeMaxSec and TimeoutStopSec remain the bounded systemd backstop.
    }
    throw error;
  }
  let technical;
  let supervisor;
  try {
    technical = validateTechnicalProbe(onlyJsonLine(technicalResult.stdout,
      "WSL2_PREFLIGHT_TECHNICAL_PROBE_REJECTED"), environment);
    supervisor = validateSupervisorRecords(
      technicalResult.stderr, environment, context, launch.unit, credentialSeal,
    );
  } catch (error) {
    if (error.stdout === undefined) error.stdout = technicalResult.stdout;
    if (error.stderr === undefined) error.stderr = technicalResult.stderr;
    throw error;
  }

  const exitConfig = {
    systemctl: fence.systemctl_path, unit: launch.unit, runtime_dir: fence.user_runtime_dir,
    cgroup_mount: fence.cgroup_mount, cgroup_path: supervisor.marker.cgroup_path,
  };
  const outerFacts = onlyJsonLine((await run(fence.supervisor_bootstrap_node.path, [
    "-e", CGROUP_EXIT_PROBE_SOURCE, canonicalJson(exitConfig),
  ])).stdout, "WSL2_PREFLIGHT_CGROUP_V2_FENCE_REJECTED");
  const outerExit = validateOuterExit(outerFacts, launch.unit, supervisor.marker.cgroup_path);

  const effectCounters = technical.effect_counters;
  const receipt = {
    schema: "lattice.wsl2-zero-model-preflight/1.0",
    status: "PASS",
    task_ref: context.taskRef,
    attempt: context.attempt,
    worktree_ref: context.worktreeRef,
    execution_environment_ref: environment.identity_digest,
    descriptor_digest: environment.identity_digest,
    distribution_identity_ref: environment.distribution_identity.identity_digest,
    linux_cwd: environment.linux.cwd,
    repository_head: environment.linux.repository_head,
    repository_identity: environment.linux.repository_identity,
    codex_home_digest: codexHomeIdentity(environment),
    credential_authority_ref: environment.credential_authority.authority_digest,
    credential_seal_digest: credentialSeal,
    verification_toolchain_ref: environment.verification_toolchain.identity_digest,
    immutable_snapshot_ref: environment.immutable_snapshot.snapshot_digest,
    sandbox_policy_ref: environment.sandbox_policy.policy_digest,
    privilege_boundary_ref: environment.privilege_boundary.boundary_digest,
    process_fence: {
      fence: context.processFence,
      authority_ref: environment.process_fence.identity_digest,
      service_unit: launch.unit,
      cgroup_path: supervisor.marker.cgroup_path,
      cgroup_version: 2,
      delegated: false,
      boot_id_digest: bootIdDigest,
      supervisor_zero_descendants: supervisor.receipt.zero_descendants,
      outer_post_exit: outerExit,
    },
    isolation,
    probes: {
      gateway: { path: environment.gateway.windows_path, version: environment.gateway.version, sha256: environment.gateway.sha256 },
      distribution: environment.distribution_identity,
      credential: credentialFacts,
      keyring_library: keyringLibrary,
      immutable,
      repository: git,
      toolchain: {
        node: { path: linux.node_path, version: linux.node_version, sha256: linux.node_sha256 },
        supervisor_bootstrap_node: fence.supervisor_bootstrap_node,
        npm: toolchain.npm, cargo: toolchain.cargo, rustc: toolchain.rustc,
        rustdoc: toolchain.rustdoc, sandbox: toolchain.sandbox,
      },
      technical: { ...technical, source_sha256: sha256(Buffer.from(TECHNICAL_PROBE_SOURCE, "utf8")) },
      supervisor: {
        bootstrap_sha256: WSL2_SUPERVISOR_BOOTSTRAP_SHA256,
        marker: supervisor.marker,
        receipt: supervisor.receipt,
      },
    },
    effect_counters: effectCounters,
    provider_effect_count: effectCounters.provider_effect_count,
    bounds: {
      stdout_limit_bytes: MAX_OUTPUT,
      stderr_limit_bytes: MAX_OUTPUT,
      stdout_observed_bytes: Buffer.byteLength(technicalResult.stdout, "utf8"),
      stderr_observed_bytes: Buffer.byteLength(technicalResult.stderr, "utf8"),
    },
    timeout: {
      timeout_ms: 170_000,
      timed_out: false,
      interrupted: supervisor.receipt.interrupted,
    },
    continuation: {
      attempt: context.attempt,
      retry_of: context.retryOf,
      reconnect_of: context.reconnectOf,
    },
    connector_auth_ready: false,
    receipt_digest: null,
  };
  receipt.receipt_digest = digest("wsl2-preflight", Object.fromEntries(
    Object.entries(receipt).filter(([key]) => key !== "receipt_digest"),
  ));
  return { environment, receipt };
}

/**
 * Converts the instrumented connector account/read result into the final
 * provider-effect gate. The observation must be captured before thread/start
 * or turn/start; this function never performs connector I/O itself.
 */
export function completeWsl2ConnectorPreflight(environmentInput, technicalReceipt, observation) {
  const environment = validateWsl2ExecutionEnvironment(environmentInput);
  const code = "WSL2_CONNECTOR_PREFLIGHT_REJECTED";
  ensure(object(technicalReceipt) && technicalReceipt.schema === "lattice.wsl2-zero-model-preflight/1.0"
    && technicalReceipt.status === "PASS" && technicalReceipt.execution_environment_ref === environment.identity_digest
    && technicalReceipt.connector_auth_ready === false && technicalReceipt.provider_effect_count === 0, code);
  ensure(technicalReceipt.receipt_digest === digest("wsl2-preflight", Object.fromEntries(
    Object.entries(technicalReceipt).filter(([key]) => key !== "receipt_digest"),
  )), code);
  exactKeys(observation, [
    "schema", "execution_environment_ref", "credential_authority_ref", "credential_seal_digest",
    "process_fence", "linux_cwd", "process_identity_digest", "account_response_digest",
    "account_read_count", "refresh_token", "auth_mode", "auth_ready", "thread_start_count",
    "turn_start_count", "provider_effect_count", "stdout_bytes", "stderr_bytes", "timeout_ms",
  ], code);
  ensure(observation.schema === "lattice.wsl2-connector-auth-observation/1.0", code);
  ensure(observation.execution_environment_ref === environment.identity_digest, code);
  ensure(observation.credential_authority_ref === environment.credential_authority.authority_digest, code);
  ensure(observation.credential_seal_digest === technicalReceipt.credential_seal_digest, code);
  ensure(observation.process_fence === technicalReceipt.process_fence.fence, code);
  ensure(observation.linux_cwd === environment.linux.cwd, code);
  ensure(/^wsl2-process:sha256:[a-f0-9]{64}$/u.test(observation.process_identity_digest), code);
  ensure(/^codex-account-read:sha256:[a-f0-9]{64}$/u.test(observation.account_response_digest), code);
  ensure(observation.account_read_count === 1 && observation.refresh_token === false, code);
  ensure(observation.auth_mode === "CHATGPT" && observation.auth_ready === true, code);
  ensure(observation.thread_start_count === 0 && observation.turn_start_count === 0
    && observation.provider_effect_count === 0, code);
  ensure(Number.isSafeInteger(observation.stdout_bytes) && observation.stdout_bytes >= 0
    && observation.stdout_bytes <= MAX_OUTPUT, code);
  ensure(Number.isSafeInteger(observation.stderr_bytes) && observation.stderr_bytes >= 0
    && observation.stderr_bytes <= MAX_OUTPUT, code);
  ensure(Number.isSafeInteger(observation.timeout_ms) && observation.timeout_ms >= 1_000
    && observation.timeout_ms <= 30_000, code);
  const receipt = {
    schema: "lattice.wsl2-production-preflight/1.0",
    status: "PASS",
    technical_preflight_ref: technicalReceipt.receipt_digest,
    execution_environment_ref: environment.identity_digest,
    process_fence: technicalReceipt.process_fence,
    credential_authority_ref: environment.credential_authority.authority_digest,
    credential_seal_digest: technicalReceipt.credential_seal_digest,
    connector_auth: structuredClone(observation),
    effect_counters: {
      account_read: technicalReceipt.effect_counters.account_read + observation.account_read_count,
      thread_start: technicalReceipt.effect_counters.thread_start + observation.thread_start_count,
      turn_start: technicalReceipt.effect_counters.turn_start + observation.turn_start_count,
      provider_effect_count: technicalReceipt.effect_counters.provider_effect_count + observation.provider_effect_count,
    },
    provider_effect_count: technicalReceipt.provider_effect_count + observation.provider_effect_count,
    receipt_digest: null,
  };
  ensure(receipt.effect_counters.account_read === 1 && receipt.effect_counters.thread_start === 0
    && receipt.effect_counters.turn_start === 0 && receipt.provider_effect_count === 0, code);
  receipt.receipt_digest = digest("wsl2-production-preflight", Object.fromEntries(
    Object.entries(receipt).filter(([key]) => key !== "receipt_digest"),
  ));
  return Object.freeze(receipt);
}

export const WSL2_CREDENTIAL_PROBE_SOURCE_SHA256 = sha256(Buffer.from(CREDENTIAL_PROBE_SOURCE, "utf8"));
export const WSL2_TECHNICAL_PROBE_SOURCE_SHA256 = sha256(Buffer.from(TECHNICAL_PROBE_SOURCE, "utf8"));
export const WSL2_IMMUTABLE_OBSERVATION_SOURCE_SHA256 = sha256(
  Buffer.from(IMMUTABLE_OBSERVATION_SOURCE, "utf8"),
);
export const WSL2_KEYRING_LIBRARY_MANIFEST_SOURCE = KEYRING_LIBRARY_MANIFEST_SOURCE;
