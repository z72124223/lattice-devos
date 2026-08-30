import { execFile as execFileCallback } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { promisify } from "node:util";

import {
  canonicalJson,
  validateWsl2ExecutionEnvironment,
} from "../apps/lattice-control/src/wsl2-execution-domain.mjs";
import {
  deriveWsl2GitControlRootIdentity,
  runWsl2VerifierBridge,
} from "../apps/lattice-control/src/wsl2-verifier-bridge.mjs";

const execFile = promisify(execFileCallback);
const EVIDENCE_FILES = Object.freeze({
  context: "acceptance-context.json",
  environment: "execution-environment.json",
  preflight: "zero-model-preflight.json",
  NODE: "wsl2-verifier-node.json",
  CARGO: "wsl2-verifier-cargo.json",
  GIT: "wsl2-verifier-git.json",
  suite: "wsl2-verifier-suite.json",
});
const CLOSED_ARGS = Object.freeze({
  NODE: Object.freeze(["run", "verify", "--offline", "--no-audit", "--no-fund"]),
  CARGO: Object.freeze(["test", "--locked", "--offline"]),
});
const HEX_40 = /^[a-f0-9]{40}$/u;
const HEX_64 = /^[a-f0-9]{64}$/u;

function fail(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
}

function ensure(condition, code) {
  if (!condition) fail(code);
}

function exactKeys(value, keys, code) {
  ensure(value !== null && typeof value === "object" && !Array.isArray(value), code);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  ensure(actual.length === expected.length
    && actual.every((key, index) => key === expected[index]), code);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function digest(domain, value) {
  return `${domain}:sha256:${sha256(canonicalJson(value))}`;
}

function parseArgs(argv) {
  ensure(argv.length === 2 && argv[0] === "--evidence-dir" && argv[1].length > 0,
    "PHASE4_WSL2_VERIFIER_USAGE_REJECTED");
  return argv[1];
}

function readJson(evidenceDir, name) {
  const source = readFileSync(path.win32.join(evidenceDir, name));
  ensure(source.length > 0 && source.length <= 1_048_576,
    "PHASE4_WSL2_VERIFIER_EVIDENCE_BOUND_REJECTED");
  try {
    return JSON.parse(source.toString("utf8"));
  } catch {
    fail("PHASE4_WSL2_VERIFIER_EVIDENCE_REJECTED");
  }
}

function writeExclusive(evidenceDir, name, value) {
  writeFileSync(path.win32.join(evidenceDir, name), `${canonicalJson(value)}\n`, {
    encoding: "utf8",
    flag: "wx",
    mode: 0o600,
  });
}

function linuxToUnc(distribution, linuxPath) {
  ensure(/^[A-Za-z0-9._-]+$/u.test(distribution)
    && linuxPath.startsWith("/home/") && !linuxPath.includes("\\")
    && !linuxPath.includes("\0") && !linuxPath.includes("/../")
    && !linuxPath.endsWith("/.."), "PHASE4_WSL2_VERIFIER_PATH_REJECTED");
  return `\\\\wsl.localhost\\${distribution}${linuxPath.replaceAll("/", "\\")}`;
}

function windowsHostEnvironment() {
  return Object.fromEntries(["SystemRoot", "WINDIR"].flatMap((key) => (
    process.env[key] === undefined ? [] : [[key, process.env[key]]]
  )));
}

async function runWsl(environment, executable, args) {
  const result = await execFile(environment.gateway.windows_path, [
    "-d", environment.distribution, "--", executable, ...args,
  ], {
    encoding: "buffer",
    env: windowsHostEnvironment(),
    timeout: 15_000,
    windowsHide: true,
    maxBuffer: 65_536,
  });
  ensure(result.stdout.length <= 65_536 && result.stderr.length <= 65_536,
    "PHASE4_WSL2_VERIFIER_CONTROL_OUTPUT_BOUND_REJECTED");
  return result;
}

async function statGitControlEntry(environment, pathname, type, mode) {
  const typeProbe = await runWsl(environment, "/usr/bin/test", [
    type === "directory" ? "-d" : "-f", pathname,
  ]);
  ensure(typeProbe.stdout.length === 0 && typeProbe.stderr.length === 0,
    "PHASE4_WSL2_VERIFIER_CONTROL_ROOT_REJECTED");
  const stat = await runWsl(environment, "/usr/bin/stat", [
    "--printf=%u:%g:%a", pathname,
  ]);
  const match = /^(0|[1-9][0-9]*):(0|[1-9][0-9]*):([0-7]{3,4})$/u.exec(
    stat.stdout.toString("utf8"),
  );
  ensure(stat.stderr.length === 0 && match !== null
    && Number(match[1]) === environment.verification_toolchain.owner_uid
    && match[3] === mode, "PHASE4_WSL2_VERIFIER_CONTROL_ROOT_REJECTED");
  return Object.freeze({
    type,
    owner_uid: Number(match[1]),
    owner_gid: Number(match[2]),
    mode: `0${match[3]}`,
  });
}

async function prepareGitControlRoot(environment, identity) {
  const controlRoot = identity.locator;
  const controlParent = path.posix.dirname(controlRoot);
  let parentExists = true;
  try {
    await runWsl(environment, "/usr/bin/test", ["-e", controlParent]);
  } catch {
    parentExists = false;
  }
  if (parentExists) {
    await statGitControlEntry(environment, controlParent, "directory", "700");
  } else {
    await runWsl(environment, "/usr/bin/install", ["-d", "-m0700", controlParent]);
  }
  try {
    await runWsl(environment, "/usr/bin/test", ["!", "-e", controlRoot]);
  } catch {
    fail("PHASE4_WSL2_VERIFIER_CONTROL_ROOT_EXISTS");
  }
  await runWsl(environment, "/usr/bin/install", [
    "-d", "-m0700", controlRoot, `${controlRoot}/git-home`, `${controlRoot}/git-temp`,
    `${controlRoot}/empty-hooks`,
  ]);
  await runWsl(environment, "/usr/bin/install", [
    "-m0600", "/dev/null", `${controlRoot}/empty-global.gitconfig`,
  ]);
  const entries = {};
  for (const [name, pathname, type, mode] of [
    ["control_parent", controlParent, "directory", "700"],
    ["root", controlRoot, "directory", "700"],
    ["git_home", `${controlRoot}/git-home`, "directory", "700"],
    ["git_temp", `${controlRoot}/git-temp`, "directory", "700"],
    ["empty_hooks", `${controlRoot}/empty-hooks`, "directory", "700"],
    ["empty_global_gitconfig", `${controlRoot}/empty-global.gitconfig`, "file", "600"],
  ]) {
    entries[name] = await statGitControlEntry(environment, pathname, type, mode);
  }
  const ownerGids = new Set(Object.values(entries).map((entry) => entry.owner_gid));
  ensure(ownerGids.size === 1, "PHASE4_WSL2_VERIFIER_CONTROL_ROOT_REJECTED");
  return Object.freeze({
    schema: "lattice.wsl2-git-control-root-evidence/1.0",
    locator: controlRoot,
    control_root_ref: identity.identity_ref,
    owner_uid: environment.verification_toolchain.owner_uid,
    owner_gid: entries.root.owner_gid,
    entries,
    retained_for_audit: true,
  });
}

function baseRequest(environment, preflight, context, role, args) {
  return {
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: preflight,
    task_ref: context.taskRef,
    attempt: context.attempt,
    worktree_ref: context.worktreeRef,
    role,
    args,
  };
}

function assertPassingResult(result, role, context, environment) {
  ensure(result.schema === "lattice.wsl2-verifier-result/1.0"
    && result.status === "PASS" && result.outcome === "PASS"
    && result.role === role && result.task_ref === context.taskRef
    && result.attempt === context.attempt && result.worktree_ref === context.worktreeRef
    && result.repository_head === environment.linux.repository_head
    && result.provider_effect_count === 0
    && /^wsl2-verifier-result:sha256:[a-f0-9]{64}$/u.test(result.result_digest),
  `PHASE4_WSL2_${role}_VERIFIER_REJECTED`);
}

function gitRequest(environment, preflight, context, controlRoot) {
  const commonDirectory = preflight.probes.technical.git.common_dir;
  const gitDirectory = preflight.probes.technical.git.git_dir;
  const gitEnvironment = {
    HOME: `${controlRoot}/git-home`,
    TMPDIR: `${controlRoot}/git-temp`,
    GIT_CONFIG_GLOBAL: `${controlRoot}/empty-global.gitconfig`,
    GIT_WORK_TREE: environment.linux.cwd,
    GIT_DIR: gitDirectory,
    GIT_COMMON_DIR: commonDirectory,
    GIT_OBJECT_DIRECTORY: `${commonDirectory}/objects`,
    GIT_INDEX_FILE: `${gitDirectory}/index`,
    NO_COLOR: "1",
    CI: "1",
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_COUNT: "0",
    GIT_TERMINAL_PROMPT: "0",
    GIT_OPTIONAL_LOCKS: "0",
    GIT_ATTR_NOSYSTEM: "1",
  };
  const args = [
    "--no-pager", "--no-replace-objects", "--literal-pathspecs", "-c",
    `core.hooksPath=${controlRoot}/empty-hooks`,
    "-c", "core.fsmonitor=false", "-c", "protocol.allow=never", "-c",
    "commit.gpgSign=false", "rev-parse", "--verify", "HEAD^{commit}",
  ];
  const subject = {
    schema: "lattice.wsl2-git-invocation/1.0",
    sequence: 1,
    environment: gitEnvironment,
    args,
    stdin: null,
  };
  const invocationDigest = digest("wsl2-git-invocation", subject);
  const processFence = sha256(Buffer.from(
    `${preflight.process_fence.fence}\n${invocationDigest}\n${subject.sequence}`,
    "utf8",
  ));
  return {
    ...baseRequest(environment, preflight, context, "GIT", args),
    git_invocation: {
      ...subject,
      invocation_digest: invocationDigest,
      process_fence: processFence,
    },
  };
}

async function main() {
  const evidenceDir = parseArgs(process.argv.slice(2));
  const environment = validateWsl2ExecutionEnvironment(
    readJson(evidenceDir, EVIDENCE_FILES.environment),
  );
  const expectedEvidenceDir = linuxToUnc(
    environment.distribution,
    `${environment.verification_toolchain.isolation_root}/evidence`,
  );
  ensure(path.win32.normalize(evidenceDir) === path.win32.normalize(expectedEvidenceDir),
    "PHASE4_WSL2_VERIFIER_EVIDENCE_PATH_REJECTED");

  const preflight = readJson(evidenceDir, EVIDENCE_FILES.preflight);
  const context = readJson(evidenceDir, EVIDENCE_FILES.context);
  exactKeys(context, [
    "attempt", "execution_environment_ref", "processFence", "provider_effect_count",
    "reconnectOf", "repository_head", "retryOf", "taskRef", "worktreeRef",
  ], "PHASE4_WSL2_VERIFIER_CONTEXT_REJECTED");
  ensure(Number.isSafeInteger(context.attempt) && context.attempt === 1
    && HEX_64.test(context.taskRef) && context.retryOf === null && context.reconnectOf === null
    && context.execution_environment_ref === environment.identity_digest
    && context.repository_head === environment.linux.repository_head
    && HEX_40.test(context.repository_head) && context.provider_effect_count === 0
    && context.processFence === preflight.process_fence.fence,
  "PHASE4_WSL2_VERIFIER_CONTEXT_REJECTED");

  const results = {};
  for (const role of ["NODE", "CARGO"]) {
    const result = await runWsl2VerifierBridge(baseRequest(
      environment, preflight, context, role, CLOSED_ARGS[role],
    ));
    assertPassingResult(result, role, context, environment);
    writeExclusive(evidenceDir, EVIDENCE_FILES[role], result);
    results[role] = result;
  }

  const controlIdentity = deriveWsl2GitControlRootIdentity({
    task_ref: context.taskRef,
    attempt: context.attempt,
    worktree_ref: context.worktreeRef,
    execution_environment_ref: context.execution_environment_ref,
    preflight_receipt_ref: preflight.receipt_digest,
    repository_head: context.repository_head,
    isolation_root: environment.verification_toolchain.isolation_root,
  });
  const control = await prepareGitControlRoot(environment, controlIdentity);
  const git = await runWsl2VerifierBridge(gitRequest(
    environment, preflight, context, controlIdentity.locator,
  ));
  assertPassingResult(git, "GIT", context, environment);
  ensure(Buffer.from(git.output.stdout_base64, "base64").toString("utf8")
    === `${environment.linux.repository_head}\n`, "PHASE4_WSL2_GIT_OUTPUT_REJECTED");
  writeExclusive(evidenceDir, EVIDENCE_FILES.GIT, git);
  results.GIT = git;

  const subject = {
    schema: "lattice.phase4-wsl2-verifier-suite/1.0",
    status: "PASS",
    task_ref: context.taskRef,
    attempt: context.attempt,
    worktree_ref: context.worktreeRef,
    execution_environment_ref: environment.identity_digest,
    preflight_receipt_ref: preflight.receipt_digest,
    repository_head: environment.linux.repository_head,
    roles: Object.fromEntries(["NODE", "CARGO", "GIT"].map((role) => [
      role, results[role].result_digest,
    ])),
    git_control_root: control,
    provider_effect_count: 0,
  };
  const suite = {
    ...subject,
    suite_digest: digest("phase4-wsl2-verifier-suite", subject),
  };
  writeExclusive(evidenceDir, EVIDENCE_FILES.suite, suite);
  process.stdout.write(`${canonicalJson(suite)}\n`);
}

main().catch((error) => {
  const code = typeof error?.code === "string" && /^[A-Z0-9_]+$/u.test(error.code)
    ? error.code : "PHASE4_WSL2_VERIFIER_FAILED";
  process.stderr.write(`${JSON.stringify({
    schema: "lattice.phase4-wsl2-verifier-runner-error/1.0",
    status: "REJECTED",
    code,
  })}\n`);
  process.exitCode = 70;
});
