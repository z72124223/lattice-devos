import { execFile as execFileCallback } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

import {
  canonicalJson,
  credentialAuthorityIdentity,
  distributionIdentity,
  executionEnvironmentIdentity,
  isClosedWsl2ToolVersion,
  processFenceAuthorityIdentity,
  sandboxPolicyIdentity,
  validateWsl2ExecutionEnvironment,
  verificationToolchainIdentity,
} from "../apps/lattice-control/src/wsl2-execution-domain.mjs";
import {
  materializeWsl2ImmutableExecutionFacts,
  preflightWsl2ExecutionEnvironment,
  WSL2_KEYRING_LIBRARY_MANIFEST_SOURCE,
} from "../apps/lattice-control/src/wsl2-execution-preflight.mjs";

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

const execFile = execFileClosedStdin;
const WSL = String.raw`C:\Windows\System32\wsl.exe`;
const DISTRIBUTION = "Ubuntu";
const HEX_64 = /^[a-f0-9]{64}$/u;
const WORKTREE_REF = /^worktree:sha256:[a-f0-9]{64}$/u;

const MATERIALIZER_VERSION_KINDS = Object.freeze({
  gateway: "WSL_GATEWAY",
  launcher: "CODEX_CLI",
  node: "NODE",
  git: "GIT",
  systemd_run: "SYSTEMD",
  systemctl: "SYSTEMD",
  bootstrap_node: "NODE",
  lsattr: "LSATTR",
  sudo: "SUDO",
  bwrap: "BWRAP",
  npm: "NPM",
  cargo: "CARGO",
  rustc: "RUSTC",
  rustdoc: "RUSTDOC",
});

const MATERIALIZATION_FAILURE_STAGES = Object.freeze({
  PHASE4_WSL2_MATERIALIZE_ARGUMENT_REJECTED: "ARGUMENT_VALIDATION",
  PHASE4_WSL2_PATH_REJECTED: "PATH_VALIDATION",
  PHASE4_WSL2_GATEWAY_VERSION_REJECTED: "GATEWAY_VERSION_VALIDATION",
  PHASE4_WSL2_TOOL_VERSION_REJECTED: "TOOL_VERSION_VALIDATION",
  PHASE4_WSL2_FILE_DIGEST_REJECTED: "TOOL_DIGEST_VALIDATION",
  PHASE4_WSL2_REVIEWED_SUPERVISOR_REJECTED: "REVIEWED_SUPERVISOR_VALIDATION",
  PHASE4_WSL2_OWNER_REJECTED: "OWNER_VALIDATION",
  PHASE4_WSL2_KEYRING_MANIFEST_REJECTED: "KEYRING_MANIFEST_VALIDATION",
  PHASE4_WSL2_REPOSITORY_REJECTED: "REPOSITORY_VALIDATION",
  PHASE4_WSL2_CARGO_HOST_REJECTED: "CARGO_HOST_VALIDATION",
  PHASE4_WSL2_PROVIDER_EFFECT_REJECTED: "PROVIDER_EFFECT_VALIDATION",
  PHASE4_WSL2_MATERIALIZATION_FAILED: "MATERIALIZATION",
});

const MATERIALIZATION_FAILURE_CODES = new Set(Object.keys(MATERIALIZATION_FAILURE_STAGES));
const CLOSED_SIGNALS = new Set([
  "SIGABRT", "SIGBUS", "SIGFPE", "SIGHUP", "SIGILL", "SIGINT", "SIGKILL", "SIGPIPE",
  "SIGQUIT", "SIGSEGV", "SIGTERM", "SIGTRAP", "SIGUSR1", "SIGUSR2",
]);

function fail(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
}

function ensure(condition, code) {
  if (!condition) fail(code);
}

export async function hashPhase4Wsl2ToolsAfterVersionValidation(versions, hashTools) {
  const expectedKeys = Object.keys(MATERIALIZER_VERSION_KINDS);
  ensure(versions !== null && typeof versions === "object" && !Array.isArray(versions)
    && Object.keys(versions).length === expectedKeys.length
    && expectedKeys.every((key) => Object.hasOwn(versions, key)),
  "PHASE4_WSL2_TOOL_VERSION_REJECTED");
  for (const [name, kind] of Object.entries(MATERIALIZER_VERSION_KINDS)) {
    ensure(isClosedWsl2ToolVersion(kind, versions[name]), "PHASE4_WSL2_TOOL_VERSION_REJECTED");
  }
  ensure(typeof hashTools === "function", "PHASE4_WSL2_TOOL_VERSION_REJECTED");
  return hashTools();
}

export function assertReviewedSupervisorDigest(fileDigests, supervisorPath, expectedSha256) {
  ensure(fileDigests !== null && typeof fileDigests === "object" && !Array.isArray(fileDigests)
    && typeof supervisorPath === "string" && supervisorPath.startsWith("/home/")
    && path.posix.normalize(supervisorPath) === supervisorPath
    && supervisorPath.endsWith("/runtime-v4/wsl2-codex-supervisor.mjs")
    && HEX_64.test(expectedSha256)
    && Object.hasOwn(fileDigests, supervisorPath)
    && fileDigests[supervisorPath] === expectedSha256,
  "PHASE4_WSL2_REVIEWED_SUPERVISOR_REJECTED");
  return fileDigests[supervisorPath];
}

function summarizedOutput(value) {
  const bytes = Buffer.isBuffer(value) ? value
    : value instanceof Uint8Array ? Buffer.from(value)
      : Buffer.from(typeof value === "string" ? value : "", "utf8");
  return Object.freeze({ byte_len: bytes.length, sha256: sha256(bytes) });
}

export function phase4Wsl2MaterializationFailureEnvelope(error) {
  const observedCode = typeof error?.code === "string" ? error.code : null;
  const code = MATERIALIZATION_FAILURE_CODES.has(observedCode)
    ? observedCode : "PHASE4_WSL2_MATERIALIZATION_FAILED";
  const exitCode = Number.isSafeInteger(error?.exitCode) ? error.exitCode
    : Number.isSafeInteger(error?.code) ? error.code : null;
  return Object.freeze({
    schema: "lattice.phase4-wsl2-live-materialization/1.0",
    status: "FAIL",
    code,
    stage: MATERIALIZATION_FAILURE_STAGES[code],
    transport_exit_code: exitCode,
    transport_signal: CLOSED_SIGNALS.has(error?.signal) ? error.signal : null,
    transport_killed: error?.killed === true,
    stdout: summarizedOutput(error?.stdout),
    stderr: summarizedOutput(error?.stderr),
    provider_effect_count: 0,
  });
}

function parseArguments(argv) {
  const allowed = new Set([
    "--task-root", "--repository", "--task-ref", "--process-fence", "--worktree-ref",
    "--expected-repository-head", "--expected-supervisor-sha256",
  ]);
  ensure(argv.length === allowed.size * 2, "PHASE4_WSL2_MATERIALIZE_ARGUMENT_REJECTED");
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    ensure(allowed.has(key) && !values.has(key) && typeof value === "string" && value.length > 0,
      "PHASE4_WSL2_MATERIALIZE_ARGUMENT_REJECTED");
    values.set(key, value);
  }
  const taskRoot = values.get("--task-root");
  const repository = values.get("--repository");
  const taskRef = values.get("--task-ref");
  const processFence = values.get("--process-fence");
  const worktreeRef = values.get("--worktree-ref");
  const expectedRepositoryHead = values.get("--expected-repository-head");
  const expectedSupervisorSha256 = values.get("--expected-supervisor-sha256");
  ensure(taskRoot?.startsWith("/home/") && path.posix.normalize(taskRoot) === taskRoot
    && repository?.startsWith(`${taskRoot}/managed-worktrees/`)
    && path.posix.normalize(repository) === repository
    && HEX_64.test(taskRef) && HEX_64.test(processFence) && WORKTREE_REF.test(worktreeRef)
    && /^[a-f0-9]{40}$/u.test(expectedRepositoryHead) && HEX_64.test(expectedSupervisorSha256),
  "PHASE4_WSL2_MATERIALIZE_ARGUMENT_REJECTED");
  return {
    taskRoot, repository, taskRef, processFence, worktreeRef, expectedRepositoryHead,
    expectedSupervisorSha256,
  };
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const typedDigest = (domain, subject) => `${domain}:sha256:${sha256(Buffer.from(canonicalJson(subject), "utf8"))}`;
const firstLine = (value) => value.replaceAll("\r", "").split("\n")[0].trimEnd();
const firstDigest = (value) => value.trim().split(/\s+/u)[0];

function linuxToWindows(linuxPath) {
  ensure(linuxPath.startsWith("/home/") && path.posix.normalize(linuxPath) === linuxPath,
    "PHASE4_WSL2_PATH_REJECTED");
  return `\\\\wsl.localhost\\${DISTRIBUTION}${linuxPath.replaceAll("/", "\\")}`;
}

async function runLinux(program, args, home, options = {}) {
  const isolatedTemp = options.tempDir ?? `${path.posix.dirname(home)}/tmp`;
  const result = await execFile(WSL, [
    "-d", DISTRIBUTION, "--exec", "/usr/bin/env", "-i",
    `HOME=${home}`, `TMPDIR=${isolatedTemp}`,
    "XDG_RUNTIME_DIR=/run/user/1000", "PATH=/usr/bin:/bin",
    "LANG=C.UTF-8", "LC_ALL=C.UTF-8", program, ...args,
  ], {
    encoding: "utf8",
    windowsHide: true,
    timeout: options.timeout ?? 30_000,
    maxBuffer: options.maxBuffer ?? 1024 * 1024,
  });
  return { stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

async function runClosedGit(repository, home, tempDir, args) {
  const result = await execFile(WSL, [
    "-d", DISTRIBUTION, "--exec", "/usr/bin/env", "-i",
    `HOME=${home}`, `TMPDIR=${tempDir}`, `GIT_CONFIG_GLOBAL=${home}/.gitconfig`,
    "NO_COLOR=1", "CI=1", "GIT_CONFIG_NOSYSTEM=1", "GIT_CONFIG_COUNT=0",
    "GIT_TERMINAL_PROMPT=0", "GIT_OPTIONAL_LOCKS=0", "GIT_ATTR_NOSYSTEM=1",
    "PATH=/usr/bin:/bin", "LANG=C.UTF-8", "LC_ALL=C.UTF-8", "/usr/bin/git",
    "--no-pager", "--no-replace-objects", "--literal-pathspecs", "-c",
    `core.hooksPath=${tempDir}/git-hooks`, "-c", "core.fsmonitor=false", "-c",
    "protocol.allow=never", "-c", "commit.gpgSign=false", "-C", repository, ...args,
  ], {
    encoding: "utf8", windowsHide: true, timeout: 30_000, maxBuffer: 256 * 1024,
  });
  return (result.stdout ?? "").trimEnd();
}

async function gatewayVersion() {
  const escaped = WSL.replaceAll("\\", "\\\\").replaceAll("'", "''");
  const { stdout } = await execFile("wmic.exe", [
    "datafile", "where", `name='${escaped}'`, "get", "Version", "/value",
  ], { encoding: "utf8", windowsHide: true, timeout: 30_000, maxBuffer: 64 * 1024 });
  const match = stdout.replaceAll("\r", "").match(/(?:^|\n)Version=(\d+\.\d+(?:\.\d+){0,2})(?:\n|$)/u);
  ensure(match, "PHASE4_WSL2_GATEWAY_VERSION_REJECTED");
  return match[1];
}

async function main() {
  const {
    taskRoot, repository, taskRef, processFence, worktreeRef, expectedRepositoryHead,
    expectedSupervisorSha256,
  } = parseArguments(process.argv.slice(2));
  const isolationRoot = `${taskRoot}/verifier-state/acceptance-${taskRef.slice(0, 16)}`;
  const homeDir = `${isolationRoot}/home`;
  const tempDir = `${isolationRoot}/tmp`;
  const evidenceDir = `${isolationRoot}/evidence`;
  await execFile(WSL, ["-d", DISTRIBUTION, "--exec", "/usr/bin/install", "-d", "-m", "0700",
    isolationRoot, homeDir, tempDir, `${isolationRoot}/npm-cache`, `${isolationRoot}/cargo-home`,
    `${isolationRoot}/cargo-target`, evidenceDir],
  { encoding: "utf8", windowsHide: true, timeout: 30_000, maxBuffer: 64 * 1024 });

  const paths = {
    launcher: `${taskRoot}/codex/bin/codex`,
    node: `${taskRoot}/toolchain-node-24.15.0/root/bin/node`,
    bootstrapNode: "/usr/bin/node",
    git: "/usr/bin/git",
    supervisor: `${taskRoot}/runtime-v4/wsl2-codex-supervisor.mjs`,
    dbus: "/usr/bin/dbus-run-session",
    setsid: "/usr/bin/setsid",
    keyring: `${taskRoot}/keyring-static-v1/root/usr/bin/gnome-keyring-daemon`,
    keyringLibraries: `${taskRoot}/keyring-static-v1/packages`,
    npm: `${taskRoot}/toolchain-node-24.15.0/root/lib/node_modules/npm/bin/npm-cli.js`,
    cargo: `${taskRoot}/toolchain-rust-1.97.1/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/cargo`,
    rustc: `${taskRoot}/toolchain-rust-1.97.1/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustc`,
    rustdoc: `${taskRoot}/toolchain-rust-1.97.1/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustdoc`,
    sandboxHelper: "/usr/bin/bwrap",
    systemdRun: "/usr/bin/systemd-run",
    systemctl: "/usr/bin/systemctl",
    lsattr: "/usr/bin/lsattr",
    sudo: "/usr/bin/sudo",
  };
  const [gatewayVersionResult, osReleaseResult, kernelResult, uidResult, gidResult, launcherResult, nodeResult,
    gitVersionResult, systemdRunResult, systemctlResult, bootstrapNodeResult, lsattrResult,
    sudoResult, sandboxResult, npmResult, cargoResult, rustcResult, rustcVerboseResult,
    rustdocResult] = await Promise.all([
    gatewayVersion(),
    runLinux("/usr/bin/cat", ["/etc/os-release"], homeDir),
    runLinux("/usr/bin/uname", ["-r"], homeDir),
    runLinux("/usr/bin/id", ["-u"], homeDir),
    runLinux("/usr/bin/id", ["-g"], homeDir),
    runLinux(paths.launcher, ["--version"], homeDir),
    runLinux(paths.node, ["--version"], homeDir),
    runLinux(paths.git, ["--version"], homeDir),
    runLinux(paths.systemdRun, ["--version"], homeDir),
    runLinux(paths.systemctl, ["--version"], homeDir),
    runLinux(paths.bootstrapNode, ["--version"], homeDir),
    runLinux(paths.lsattr, ["-V"], homeDir),
    runLinux(paths.sudo, ["-V"], homeDir),
    runLinux(paths.sandboxHelper, ["--version"], homeDir),
    runLinux(paths.node, [paths.npm, "--version"], homeDir),
    runLinux(paths.cargo, ["--version"], homeDir),
    runLinux(paths.rustc, ["--version"], homeDir),
    runLinux(paths.rustc, ["-vV"], homeDir),
    runLinux(paths.rustdoc, ["--version"], homeDir),
  ]);
  const toolVersions = {
    gateway: gatewayVersionResult,
    launcher: firstLine(launcherResult.stdout),
    node: firstLine(nodeResult.stdout),
    git: firstLine(gitVersionResult.stdout),
    systemd_run: firstLine(systemdRunResult.stdout),
    systemctl: firstLine(systemctlResult.stdout),
    bootstrap_node: firstLine(bootstrapNodeResult.stdout),
    lsattr: firstLine(lsattrResult.stderr || lsattrResult.stdout),
    sudo: firstLine(sudoResult.stdout),
    bwrap: firstLine(sandboxResult.stdout),
    npm: firstLine(npmResult.stdout),
    cargo: firstLine(cargoResult.stdout),
    rustc: firstLine(rustcResult.stdout),
    rustdoc: firstLine(rustdocResult.stdout),
  };
  ensure(firstLine(rustcVerboseResult.stdout) === toolVersions.rustc,
    "PHASE4_WSL2_TOOL_VERSION_REJECTED");
  const fileDigests = await hashPhase4Wsl2ToolsAfterVersionValidation(toolVersions, async () => {
    const digests = {};
    for (const file of Object.values(paths).filter((value) => value.startsWith("/"))) {
      if (file === paths.keyringLibraries) continue;
      digests[file] = firstDigest((await runLinux("/usr/bin/sha256sum", [file], homeDir)).stdout);
      ensure(HEX_64.test(digests[file]), "PHASE4_WSL2_FILE_DIGEST_REJECTED");
    }
    return digests;
  });
  assertReviewedSupervisorDigest(fileDigests, paths.supervisor, expectedSupervisorSha256);
  const [configDigestResult, keyringManifestResult] = await Promise.all([
    runLinux("/usr/bin/sha256sum", [`${taskRoot}/codex-home/config.toml`], homeDir),
    runLinux(paths.bootstrapNode, ["-e", WSL2_KEYRING_LIBRARY_MANIFEST_SOURCE,
      paths.keyringLibraries, "0"], homeDir),
  ]);
  const ownerUid = Number(uidResult.stdout.trim());
  const ownerGid = Number(gidResult.stdout.trim());
  ensure(Number.isSafeInteger(ownerUid) && ownerUid > 0 && Number.isSafeInteger(ownerGid) && ownerGid > 0,
    "PHASE4_WSL2_OWNER_REJECTED");
  const osRelease = osReleaseResult.stdout;
  const osFields = Object.fromEntries(osRelease.replaceAll("\r", "").split("\n")
    .filter((line) => line.includes("="))
    .map((line) => { const index = line.indexOf("="); return [line.slice(0, index),
      line.slice(index + 1).replace(/^"|"$/gu, "")]; }));
  const keyringManifest = JSON.parse(keyringManifestResult.stdout.trim());
  ensure(keyringManifest.schema === "lattice.wsl2-keyring-library-manifest/1.0"
    && /^keyring-library-manifest:sha256:[a-f0-9]{64}$/u.test(keyringManifest.digest),
  "PHASE4_WSL2_KEYRING_MANIFEST_REJECTED");

  const git = {
    top_level: await runClosedGit(repository, homeDir, tempDir, ["rev-parse", "--show-toplevel"]),
    git_dir: await runClosedGit(repository, homeDir, tempDir, ["rev-parse", "--absolute-git-dir"]),
    common_dir: await runClosedGit(repository, homeDir, tempDir,
      ["rev-parse", "--path-format=absolute", "--git-common-dir"]),
    head: await runClosedGit(repository, homeDir, tempDir, ["rev-parse", "--verify", "HEAD^{commit}"]),
    status: await runClosedGit(repository, homeDir, tempDir, ["status", "--porcelain=v1"]),
  };
  ensure(git.top_level === repository && git.head === expectedRepositoryHead && git.status === "",
    "PHASE4_WSL2_REPOSITORY_REJECTED");
  const cargoHost = rustcVerboseResult.stdout.replaceAll("\r", "")
    .match(/(?:^|\n)host: ([A-Za-z0-9._-]+)(?:\n|$)/u)?.[1];
  ensure(cargoHost, "PHASE4_WSL2_CARGO_HOST_REJECTED");

  const descriptor = {
    schema: "lattice.execution-environment.wsl2-linux/1.1",
    kind: "WSL2_LINUX",
    distribution: DISTRIBUTION,
    distribution_identity: {
      os_id: osFields.ID,
      os_version_id: osFields.VERSION_ID,
      os_version_codename: osFields.VERSION_CODENAME,
      os_release_sha256: sha256(Buffer.from(osRelease, "utf8")),
      kernel_release: kernelResult.stdout.trim(),
      identity_digest: null,
    },
    gateway: {
      windows_path: WSL,
      version: toolVersions.gateway,
      sha256: sha256(await readFile(WSL)),
    },
    linux: {
      launcher_path: paths.launcher,
      launcher_version: toolVersions.launcher,
      launcher_sha256: fileDigests[paths.launcher],
      node_path: paths.node,
      node_version: toolVersions.node,
      node_sha256: fileDigests[paths.node],
      git_path: paths.git,
      git_version: toolVersions.git,
      git_sha256: fileDigests[paths.git],
      supervisor_path: paths.supervisor,
      supervisor_sha256: fileDigests[paths.supervisor],
      codex_home: `${taskRoot}/codex-home`,
      config_digest: `codex-config:sha256:${firstDigest(configDigestResult.stdout)}`,
      cwd: repository,
      repository_head: git.head,
      repository_identity: null,
      dbus_run_session_path: paths.dbus,
      dbus_run_session_sha256: fileDigests[paths.dbus],
      setsid_path: paths.setsid,
      setsid_sha256: fileDigests[paths.setsid],
      keyring_daemon_path: paths.keyring,
      keyring_daemon_sha256: fileDigests[paths.keyring],
      keyring_library_path: paths.keyringLibraries,
      keyring_library_manifest_digest: keyringManifest.digest,
      xdg_runtime_dir: `/run/user/${ownerUid}`,
    },
    credential_authority: { kind: "LINUX_KEYRING", authority_digest: null },
    process_fence: {
      schema: "lattice.wsl2-cgroup-v2-fence/1.0",
      kind: "SYSTEMD_USER_SERVICE_CGROUP_V2",
      systemd_run_path: paths.systemdRun,
      systemd_run_version: toolVersions.systemd_run,
      systemd_run_sha256: fileDigests[paths.systemdRun],
      systemctl_path: paths.systemctl,
      systemctl_version: toolVersions.systemctl,
      systemctl_sha256: fileDigests[paths.systemctl],
      cgroup_mount: "/sys/fs/cgroup",
      user_runtime_dir: `/run/user/${ownerUid}`,
      unit_prefix: `lattice-wsl2-${taskRef.slice(0, 16)}`,
      supervisor_bootstrap_node: {
        path: paths.bootstrapNode,
        version: toolVersions.bootstrap_node,
        sha256: fileDigests[paths.bootstrapNode],
      },
      immutable_probe_lsattr: {
        path: paths.lsattr,
        version: toolVersions.lsattr,
        sha256: fileDigests[paths.lsattr],
      },
      noninteractive_root_probe: {
        path: paths.sudo,
        version: toolVersions.sudo,
        sha256: fileDigests[paths.sudo],
      },
      identity_digest: null,
    },
    verification_toolchain: {
      schema: "lattice.wsl2-verification-toolchain/1.0",
      task_ref: taskRef,
      task_root: taskRoot,
      isolation_root: isolationRoot,
      owner_uid: ownerUid,
      home_dir: homeDir,
      temp_dir: tempDir,
      npm_cache: `${isolationRoot}/npm-cache`,
      cargo_home: `${isolationRoot}/cargo-home`,
      cargo_target_dir: `${isolationRoot}/cargo-target`,
      cargo_host: cargoHost,
      npm: { path: paths.npm, version: toolVersions.npm, sha256: fileDigests[paths.npm] },
      cargo: { path: paths.cargo, version: toolVersions.cargo, sha256: fileDigests[paths.cargo] },
      rustc: { path: paths.rustc, version: toolVersions.rustc, sha256: fileDigests[paths.rustc] },
      rustdoc: { path: paths.rustdoc, version: toolVersions.rustdoc, sha256: fileDigests[paths.rustdoc] },
      sandbox: { path: paths.launcher, version: toolVersions.launcher, sha256: fileDigests[paths.launcher] },
      sandbox_helper: { path: paths.sandboxHelper, version: toolVersions.bwrap,
        sha256: fileDigests[paths.sandboxHelper] },
      identity_digest: null,
    },
    path_mapping: {
      windows_path: linuxToWindows(repository),
      linux_path: repository,
      digest: null,
    },
    immutable_snapshot: {
      schema: "lattice.wsl2-immutable-snapshot/1.0",
      task_root_path: taskRoot,
      task_root_device: "1",
      task_root_inode: "1",
      task_root_owner_uid: 0,
      task_root_owner_gid: 0,
      task_root_mode: "0555",
      task_root_immutable: true,
      trees: {
        codex: { root: `${taskRoot}/codex`, manifest_digest: null },
        supervisor_runtime: { root: `${taskRoot}/runtime-v4`, manifest_digest: null },
        node: { root: `${taskRoot}/toolchain-node-24.15.0`, manifest_digest: null },
        rust: { root: `${taskRoot}/toolchain-rust-1.97.1`, manifest_digest: null },
        keyring: { root: `${taskRoot}/keyring-static-v1`, manifest_digest: null },
      },
      snapshot_digest: null,
    },
    sandbox_policy: { schema: "lattice.wsl2-sandbox-policy/1.0", policy_digest: null },
    privilege_boundary: {
      schema: "lattice.wsl2-privilege-boundary/1.0",
      effective_uid: ownerUid,
      effective_gid: ownerGid,
      effective_capabilities_digest: null,
      noninteractive_root_unavailable: true,
      boundary_digest: null,
    },
    identity_digest: null,
  };
  descriptor.distribution_identity.identity_digest = distributionIdentity(descriptor);
  const materialized = await materializeWsl2ImmutableExecutionFacts(descriptor, {
    run: (program, args = [], options = {}) => runLinux(program, args, homeDir,
      { ...options, tempDir }),
  });
  descriptor.immutable_snapshot = materialized.immutable_snapshot;
  descriptor.privilege_boundary = materialized.privilege_boundary;
  descriptor.credential_authority.authority_digest = credentialAuthorityIdentity(descriptor);
  descriptor.process_fence.identity_digest = processFenceAuthorityIdentity(descriptor);
  descriptor.verification_toolchain.identity_digest = verificationToolchainIdentity(descriptor);
  descriptor.linux.repository_identity = typedDigest("repository", {
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
  descriptor.path_mapping.digest = typedDigest("path-mapping", {
    distribution: descriptor.distribution,
    windows_path: descriptor.path_mapping.windows_path,
    linux_path: descriptor.path_mapping.linux_path,
    repository_identity: descriptor.linux.repository_identity,
    repository_head: descriptor.linux.repository_head,
  });
  descriptor.sandbox_policy.policy_digest = sandboxPolicyIdentity(descriptor);
  descriptor.identity_digest = executionEnvironmentIdentity(descriptor);
  const environment = validateWsl2ExecutionEnvironment(descriptor);
  const outputWindows = linuxToWindows(evidenceDir);
  await mkdir(outputWindows, { recursive: true });
  await Promise.all([
    writeFile(path.win32.join(outputWindows, "preflight-input-environment.json"),
      `${canonicalJson(environment)}\n`, { encoding: "utf8", mode: 0o600 }),
    writeFile(path.win32.join(outputWindows, "immutable-materialization.json"),
      `${canonicalJson(materialized.evidence)}\n`, { encoding: "utf8", mode: 0o600 }),
  ]);
  const context = {
    processFence, taskRef, attempt: 1, worktreeRef, retryOf: null, reconnectOf: null,
  };
  const preflight = await preflightWsl2ExecutionEnvironment(environment, context);
  ensure(preflight.receipt.provider_effect_count === 0
    && preflight.receipt.probes.technical.effect_counters.provider_effect_count === 0,
  "PHASE4_WSL2_PROVIDER_EFFECT_REJECTED");

  const artifacts = {
    descriptor: path.win32.join(outputWindows, "execution-environment.json"),
    preflight: path.win32.join(outputWindows, "zero-model-preflight.json"),
    materialization: path.win32.join(outputWindows, "immutable-materialization.json"),
    context: path.win32.join(outputWindows, "acceptance-context.json"),
  };
  await Promise.all([
    writeFile(artifacts.descriptor, `${canonicalJson(preflight.environment)}\n`, { encoding: "utf8", mode: 0o600 }),
    writeFile(artifacts.preflight, `${canonicalJson(preflight.receipt)}\n`, { encoding: "utf8", mode: 0o600 }),
    writeFile(artifacts.materialization, `${canonicalJson(materialized.evidence)}\n`, { encoding: "utf8", mode: 0o600 }),
    writeFile(artifacts.context, `${canonicalJson({ ...context,
      execution_environment_ref: preflight.environment.identity_digest,
      repository_head: preflight.environment.linux.repository_head,
      provider_effect_count: 0 })}\n`, { encoding: "utf8", mode: 0o600 }),
  ]);
  process.stdout.write(`${canonicalJson({
    schema: "lattice.phase4-wsl2-live-materialization/1.0",
    status: "PASS",
    task_ref: taskRef,
    attempt: 1,
    execution_environment_ref: preflight.environment.identity_digest,
    repository_head: preflight.environment.linux.repository_head,
    expected_repository_head: expectedRepositoryHead,
    credential_authority_kind: preflight.environment.credential_authority.kind,
    credential_seal_digest: preflight.receipt.credential_seal_digest,
    immutable_snapshot_ref: preflight.environment.immutable_snapshot.snapshot_digest,
    verification_toolchain_ref: preflight.environment.verification_toolchain.identity_digest,
    process_fence_authority_ref: preflight.environment.process_fence.identity_digest,
    provider_effect_count: 0,
    evidence_directory: linuxToWindows(evidenceDir),
  })}\n`);
}

if (typeof process.argv[1] === "string"
    && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    await main();
  } catch (error) {
    process.stdout.write(`${canonicalJson(phase4Wsl2MaterializationFailureEnvelope(error))}\n`);
    process.exitCode = 1;
  }
}
