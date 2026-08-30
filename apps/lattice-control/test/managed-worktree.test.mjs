import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { access, appendFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import test from "node:test";

import { WorkspaceError } from "../../../src/workspace/errors.js";
import {
  MANAGED_WORKTREE_BASELINE_SCHEMA,
  ManagedWorktreeOwner,
} from "../src/managed-worktree.mjs";
import {
  credentialAuthorityIdentity,
  distributionIdentity,
  executionEnvironmentIdentity,
  immutableSnapshotIdentity,
  pathMappingIdentity,
  privilegeBoundaryIdentity,
  processFenceAuthorityIdentity,
  sandboxPolicyIdentity,
  validateWsl2ExecutionEnvironment,
  verificationToolchainIdentity,
} from "../src/wsl2-execution-domain.mjs";

const execFileAsync = promisify(execFile);
const BRIDGE_PATH = path.resolve(import.meta.dirname, "../src/managed-worktree-bridge.mjs");
const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const typed = (domain, marker) => `${domain}:sha256:${marker.repeat(64)}`;

async function git(cwd, args) {
  const { stdout } = await execFileAsync("git", args, {
    cwd,
    encoding: "utf8",
    windowsHide: true,
  });
  return stdout.trim();
}

async function exists(file) {
  try {
    await access(file);
    return true;
  } catch {
    return false;
  }
}

async function gitExecutable() {
  if (process.platform === "win32") {
    const { stdout } = await execFileAsync("where.exe", ["git.exe"], {
      encoding: "utf8",
      windowsHide: true,
    });
    return stdout.split(/\r?\n/u).find(Boolean);
  }
  const { stdout } = await execFileAsync("which", ["git"], { encoding: "utf8" });
  return stdout.trim();
}

function rehashWslDescriptor(descriptor) {
  descriptor.distribution_identity.identity_digest = distributionIdentity(descriptor);
  descriptor.credential_authority.authority_digest = credentialAuthorityIdentity(descriptor);
  descriptor.process_fence.identity_digest = processFenceAuthorityIdentity(descriptor);
  descriptor.verification_toolchain.identity_digest = verificationToolchainIdentity(descriptor);
  descriptor.immutable_snapshot.snapshot_digest = immutableSnapshotIdentity(descriptor);
  descriptor.sandbox_policy.policy_digest = sandboxPolicyIdentity(descriptor);
  descriptor.privilege_boundary.boundary_digest = privilegeBoundaryIdentity(descriptor);
  descriptor.path_mapping.digest = pathMappingIdentity(descriptor);
  descriptor.identity_digest = executionEnvironmentIdentity(descriptor);
  return descriptor;
}

function validWslDescriptor(taskRef, repositoryHead) {
  const taskRoot = "/home/zk/lattice-managed-worktree-binding";
  const repository = `${taskRoot}/managed-worktrees/work-${taskRef}`;
  const isolation = `${taskRoot}/verifier-state/${taskRef}`;
  const paths = {
    launcher: `${taskRoot}/codex/bin/codex`,
    node: `${taskRoot}/toolchain-node-24.15.0/root/bin/node`,
    git: "/usr/bin/git",
    supervisor: `${taskRoot}/runtime-v1/wsl2-codex-supervisor.mjs`,
    dbus: "/usr/bin/dbus-run-session",
    setsid: "/usr/bin/setsid",
    keyring: `${taskRoot}/keyring-static-v1/root/usr/bin/gnome-keyring-daemon`,
    npm: `${taskRoot}/toolchain-node-24.15.0/root/lib/node_modules/npm/bin/npm-cli.js`,
    cargo: `${taskRoot}/toolchain-rust-1.97.1/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/cargo`,
    rustc: `${taskRoot}/toolchain-rust-1.97.1/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustc`,
    rustdoc: `${taskRoot}/toolchain-rust-1.97.1/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustdoc`,
  };
  return rehashWslDescriptor({
    schema: "lattice.execution-environment.wsl2-linux/1.1",
    kind: "WSL2_LINUX",
    distribution: "Ubuntu",
    distribution_identity: {
      os_id: "ubuntu",
      os_version_id: "24.04",
      os_version_codename: "noble",
      os_release_sha256: sha256("os-release"),
      kernel_release: "6.6.87.2-microsoft-standard-WSL2",
      identity_digest: null,
    },
    gateway: {
      windows_path: String.raw`C:\Windows\System32\wsl.exe`,
      version: "2.6.1",
      sha256: sha256("wsl.exe"),
    },
    linux: {
      launcher_path: paths.launcher,
      launcher_version: "codex-cli 0.146.0",
      launcher_sha256: sha256(paths.launcher),
      node_path: paths.node,
      node_version: "v24.15.0",
      node_sha256: sha256(paths.node),
      git_path: paths.git,
      git_version: "git version 2.43.0",
      git_sha256: sha256(paths.git),
      supervisor_path: paths.supervisor,
      supervisor_sha256: sha256(paths.supervisor),
      codex_home: `${taskRoot}/codex-home`,
      config_digest: typed("codex-config", "1"),
      cwd: repository,
      repository_head: repositoryHead,
      repository_identity: typed("repository", "2"),
      dbus_run_session_path: paths.dbus,
      dbus_run_session_sha256: sha256(paths.dbus),
      setsid_path: paths.setsid,
      setsid_sha256: sha256(paths.setsid),
      keyring_daemon_path: paths.keyring,
      keyring_daemon_sha256: sha256(paths.keyring),
      keyring_library_path: `${taskRoot}/keyring-static-v1/packages`,
      keyring_library_manifest_digest: typed("keyring-library-manifest", "3"),
      xdg_runtime_dir: "/run/user/1000",
    },
    credential_authority: { kind: "LINUX_KEYRING", authority_digest: null },
    process_fence: {
      schema: "lattice.wsl2-cgroup-v2-fence/1.0",
      kind: "SYSTEMD_USER_SERVICE_CGROUP_V2",
      systemd_run_path: "/usr/bin/systemd-run",
      systemd_run_version: "systemd 255 (255.4-1ubuntu8.11)",
      systemd_run_sha256: sha256("/usr/bin/systemd-run"),
      systemctl_path: "/usr/bin/systemctl",
      systemctl_version: "systemd 255 (255.4-1ubuntu8.11)",
      systemctl_sha256: sha256("/usr/bin/systemctl"),
      cgroup_mount: "/sys/fs/cgroup",
      user_runtime_dir: "/run/user/1000",
      unit_prefix: `lattice-wsl2-${taskRef.slice(0, 16)}`,
      supervisor_bootstrap_node: {
        path: "/usr/bin/node",
        version: "v22.22.1",
        sha256: sha256("/usr/bin/node"),
      },
      immutable_probe_lsattr: {
        path: "/usr/bin/lsattr",
        version: "lsattr 1.47.0 (5-Feb-2023)",
        sha256: sha256("/usr/bin/lsattr"),
      },
      noninteractive_root_probe: {
        path: "/usr/bin/sudo",
        version: "Sudo version 1.9.15p5",
        sha256: sha256("/usr/bin/sudo"),
      },
      identity_digest: null,
    },
    verification_toolchain: {
      schema: "lattice.wsl2-verification-toolchain/1.0",
      task_ref: taskRef,
      task_root: taskRoot,
      isolation_root: isolation,
      owner_uid: 1000,
      home_dir: `${isolation}/home`,
      temp_dir: `${isolation}/tmp`,
      npm_cache: `${isolation}/npm-cache`,
      cargo_home: `${isolation}/cargo-home`,
      cargo_target_dir: `${isolation}/cargo-target`,
      cargo_host: "x86_64-unknown-linux-gnu",
      npm: { path: paths.npm, version: "11.12.1", sha256: sha256(paths.npm) },
      cargo: {
        path: paths.cargo,
        version: "cargo 1.97.1 (c980f4866 2026-03-10)",
        sha256: sha256(paths.cargo),
      },
      rustc: {
        path: paths.rustc,
        version: "rustc 1.97.1 (8bab26f4f 2026-03-10)",
        sha256: sha256(paths.rustc),
      },
      rustdoc: {
        path: paths.rustdoc,
        version: "rustdoc 1.97.1 (8bab26f4f 2026-03-10)",
        sha256: sha256(paths.rustdoc),
      },
      sandbox: {
        path: paths.launcher,
        version: "codex-cli 0.146.0",
        sha256: sha256(paths.launcher),
      },
      sandbox_helper: {
        path: "/usr/bin/bwrap",
        version: "bubblewrap 0.11.0",
        sha256: sha256("/usr/bin/bwrap"),
      },
      identity_digest: null,
    },
    immutable_snapshot: {
      schema: "lattice.wsl2-immutable-snapshot/1.0",
      task_root_path: taskRoot,
      task_root_device: "2049",
      task_root_inode: "40001",
      task_root_owner_uid: 0,
      task_root_owner_gid: 0,
      task_root_mode: "0555",
      task_root_immutable: true,
      trees: {
        codex: { root: `${taskRoot}/codex`, manifest_digest: typed("immutable-tree-manifest", "a") },
        supervisor_runtime: { root: `${taskRoot}/runtime-v1`, manifest_digest: typed("immutable-tree-manifest", "b") },
        node: { root: `${taskRoot}/toolchain-node-24.15.0`, manifest_digest: typed("immutable-tree-manifest", "c") },
        rust: { root: `${taskRoot}/toolchain-rust-1.97.1`, manifest_digest: typed("immutable-tree-manifest", "d") },
        keyring: { root: `${taskRoot}/keyring-static-v1`, manifest_digest: typed("immutable-tree-manifest", "e") },
      },
      snapshot_digest: null,
    },
    sandbox_policy: { schema: "lattice.wsl2-sandbox-policy/1.0", policy_digest: null },
    privilege_boundary: {
      schema: "lattice.wsl2-privilege-boundary/1.0",
      effective_uid: 1000,
      effective_gid: 1000,
      effective_capabilities_digest: typed("linux-capabilities", "0"),
      noninteractive_root_unavailable: true,
      boundary_digest: null,
    },
    path_mapping: {
      windows_path: `\\\\wsl.localhost\\Ubuntu${repository.replaceAll("/", "\\")}`,
      linux_path: repository,
      digest: null,
    },
    identity_digest: null,
  });
}

async function runBridge(command, executionEnvironmentJson = null) {
  const env = { ...process.env };
  delete env.LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON;
  if (executionEnvironmentJson !== null) {
    env.LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON = executionEnvironmentJson;
  }
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [BRIDGE_PATH], {
      cwd: path.dirname(BRIDGE_PATH),
      env,
      windowsHide: true,
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    child.once("error", reject);
    child.once("close", (exitCode) => resolve({
      exitCode,
      stdout,
      stderr,
      record: JSON.parse(stdout.trim()),
    }));
    child.stdin.end(`${JSON.stringify(command)}\n`);
  });
}

async function fixture(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-managed-worktree-"));
  const repositoryRoot = path.join(root, "registry-repository");
  const worktreeRoot = path.join(root, "managed-worktrees");
  await mkdir(repositoryRoot);
  await mkdir(worktreeRoot);
  t.after(async () => rm(root, { recursive: true, force: true }));
  await git(repositoryRoot, ["init", "-b", "main"]);
  await git(repositoryRoot, ["config", "user.name", "LATTICE Worktree Test"]);
  await git(repositoryRoot, ["config", "user.email", "worktree@invalid.example"]);
  await writeFile(
    path.join(repositoryRoot, ".gitignore"),
    "*.credentials\n*.worker-sentinel\n",
  );
  await writeFile(path.join(repositoryRoot, "proof.txt"), "base\n");
  await git(repositoryRoot, ["add", ".gitignore", "proof.txt"]);
  await git(repositoryRoot, ["commit", "-m", "managed baseline"]);
  const baseCommit = await git(repositoryRoot, ["rev-parse", "HEAD"]);
  await writeFile(
    path.join(repositoryRoot, "registry-only.credentials"),
    "source ignored sentinel only\n",
  );
  return {
    root,
    repositoryRoot,
    worktreeRoot,
    baseCommit,
    gitExecutable: await gitExecutable(),
  };
}

test("bridge 1.1 requires one exact expected execution-environment ref field", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-managed-worktree-schema-"));
  t.after(async () => rm(root, { recursive: true, force: true }));
  const command = {
    schema: "lattice.managed-worktree-command/1.1",
    operation: "prepare",
    repository_root: path.join(root, "missing-repository"),
    worktree_root: path.join(root, "missing-worktrees"),
    git_executable: path.join(root, "missing-git"),
    task_ref: "8".repeat(64),
    task_id: "TASK-MANAGED-SCHEMA",
    base_commit: "1".repeat(40),
    expected_baseline_sha256: null,
  };
  const missing = await runBridge(command);
  assert.equal(missing.exitCode, 2);
  assert.equal(missing.record.code, "MANAGED_WORKTREE_COMMAND_REJECTED");
  assert.equal(missing.record.owner_code, null);

  const stale = await runBridge({
    ...command,
    schema: "lattice.managed-worktree-command/1.0",
    expected_execution_environment_ref: null,
  });
  assert.equal(stale.exitCode, 2);
  assert.equal(stale.record.code, "MANAGED_WORKTREE_COMMAND_REJECTED");
});

test("native bridge commands require the expected execution-environment ref to be null", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-managed-worktree-native-ref-"));
  t.after(async () => rm(root, { recursive: true, force: true }));
  const result = await runBridge({
    schema: "lattice.managed-worktree-command/1.1",
    operation: "prepare",
    repository_root: path.join(root, "missing-repository"),
    worktree_root: path.join(root, "missing-worktrees"),
    git_executable: path.join(root, "missing-git"),
    task_ref: "6".repeat(64),
    task_id: "TASK-MANAGED-NATIVE-REF",
    base_commit: "2".repeat(40),
    expected_baseline_sha256: null,
    expected_execution_environment_ref: `execution-environment:sha256:${"6".repeat(64)}`,
  });
  assert.equal(result.exitCode, 3);
  assert.equal(
    result.record.owner_code,
    "MANAGED_WORKTREE_EXECUTION_ENVIRONMENT_BINDING_REJECTED",
  );
  assert.equal(await exists(path.join(root, "missing-repository")), false);
  assert.equal(await exists(path.join(root, "missing-worktrees")), false);
  assert.equal(await exists(path.join(root, "missing-git")), false);
});

test("re-sealed WSL tool identity fails prepare, verify, and protect before filesystem or Git effects", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-managed-worktree-wsl-ref-"));
  t.after(async () => rm(root, { recursive: true, force: true }));
  const taskRef = "7".repeat(64);
  const repositoryHead = "0123456789abcdef0123456789abcdef01234567";
  const retained = validWslDescriptor(taskRef, repositoryHead);
  const resealed = structuredClone(retained);
  resealed.verification_toolchain.rustc.sha256 = sha256("repacked-rustc");
  rehashWslDescriptor(resealed);
  assert.deepEqual(validateWsl2ExecutionEnvironment(resealed), resealed);
  assert.equal(resealed.linux.cwd, retained.linux.cwd);
  assert.equal(resealed.linux.repository_head, retained.linux.repository_head);
  assert.notEqual(resealed.identity_digest, retained.identity_digest);

  const common = {
    schema: "lattice.managed-worktree-command/1.1",
    repository_root: path.join(root, "missing-repository"),
    worktree_root: path.join(root, "missing-worktrees"),
    git_executable: path.join(root, "missing-git"),
    task_ref: taskRef,
    task_id: "TASK-MANAGED-WSL-REF",
    base_commit: repositoryHead,
    expected_execution_environment_ref: retained.identity_digest,
  };
  const commands = [
    { ...common, operation: "prepare", expected_baseline_sha256: null },
    { ...common, operation: "verify", expected_baseline_sha256: "a".repeat(64) },
    {
      ...common,
      operation: "protect",
      expected_baseline_sha256: "a".repeat(64),
      attempt: 1,
      writer_fence: 1,
      result_commit: "3".repeat(40),
      require_existing: false,
    },
  ];
  for (const command of commands) {
    const result = await runBridge(command, JSON.stringify(resealed));
    assert.equal(result.exitCode, 3, command.operation);
    assert.equal(
      result.record.owner_code,
      "MANAGED_WORKTREE_EXECUTION_ENVIRONMENT_BINDING_REJECTED",
      command.operation,
    );
  }
  for (const command of commands) {
    const result = await runBridge({
      ...command,
      expected_execution_environment_ref: null,
    }, JSON.stringify(resealed));
    assert.equal(result.exitCode, 3, `${command.operation}: null ref`);
    assert.equal(
      result.record.owner_code,
      "MANAGED_WORKTREE_EXECUTION_ENVIRONMENT_BINDING_REJECTED",
      `${command.operation}: null ref`,
    );
  }
  for (const command of commands) {
    const result = await runBridge({
      ...command,
      expected_execution_environment_ref: resealed.identity_digest,
    }, JSON.stringify(resealed));
    assert.equal(result.exitCode, 3, `${command.operation}: matching ref`);
    assert.equal(
      result.record.owner_code,
      "MANAGED_WORKTREE_GIT_EXECUTABLE_UNSAFE",
      `${command.operation}: matching ref`,
    );
  }
  assert.equal(await exists(path.join(root, "missing-repository")), false);
  assert.equal(await exists(path.join(root, "missing-worktrees")), false);
  assert.equal(await exists(path.join(root, "missing-git")), false);
});

test("managed owner creates an isolated worktree and exact-replays its durable baseline", async (t) => {
  const value = await fixture(t);
  const owner = new ManagedWorktreeOwner(value);
  const request = {
    task_ref: "a".repeat(64),
    task_id: "TASK-MANAGED-WORKTREE",
    base_commit: value.baseCommit,
    expected_execution_environment_ref: null,
  };
  const created = await owner.prepare(request);
  assert.equal(created.replayed, false);
  assert.equal(created.baseline.schema, MANAGED_WORKTREE_BASELINE_SCHEMA);
  assert.equal(created.baseline.task_ref, request.task_ref);
  assert.equal(created.baseline.base_commit, value.baseCommit);
  assert.equal(created.baseline.head_commit, value.baseCommit);
  assert.equal(created.baseline.base_tree, created.baseline.head_tree);
  assert.match(created.baseline_sha256, /^[a-f0-9]{64}$/u);
  assert.equal(
    await exists(path.join(created.worktree_path, "registry-only.credentials")),
    false,
    "source-only ignored credentials must not enter the worker cwd",
  );

  const replay = await owner.prepare({
    ...request,
    expected_baseline_sha256: created.baseline_sha256,
  });
  assert.equal(replay.replayed, true);
  assert.equal(replay.worktree_path, created.worktree_path);
  assert.equal(replay.baseline_json, created.baseline_json);
  assert.equal(replay.baseline_sha256, created.baseline_sha256);
});

test("worker ignored files remain isolated while index and Git-control drift fail closed", async (t) => {
  const value = await fixture(t);
  const owner = new ManagedWorktreeOwner(value);
  const request = {
    task_ref: "b".repeat(64),
    task_id: "TASK-MANAGED-DRIFT",
    base_commit: value.baseCommit,
    expected_execution_environment_ref: null,
  };
  const created = await owner.prepare(request);
  const workerSentinel = path.join(created.worktree_path, "attempt.worker-sentinel");
  await writeFile(workerSentinel, "worker-only ignored sentinel\n");
  const replay = await owner.prepare({
    ...request,
    expected_baseline_sha256: created.baseline_sha256,
  });
  assert.equal(replay.baseline_sha256, created.baseline_sha256);
  assert.equal(
    await exists(path.join(value.repositoryRoot, "attempt.worker-sentinel")),
    false,
    "worker ignored writes must not affect the registered source checkout",
  );
  assert.equal(
    await readFile(path.join(value.repositoryRoot, "proof.txt"), "utf8"),
    "base\n",
  );

  await appendFile(path.join(created.worktree_path, "proof.txt"), "staged drift\n");
  await git(created.worktree_path, ["add", "proof.txt"]);
  await assert.rejects(
    owner.prepare({
      ...request,
      expected_baseline_sha256: created.baseline_sha256,
    }),
    (error) =>
      error instanceof WorkspaceError
      && error.code === "MANAGED_WORKTREE_BASELINE_SUBSTITUTION",
  );
});

test("task, base, and expected baseline substitution never creates another worker checkout", async (t) => {
  const value = await fixture(t);
  const owner = new ManagedWorktreeOwner(value);
  const request = {
    task_ref: "c".repeat(64),
    task_id: "TASK-MANAGED-SUBSTITUTION",
    base_commit: value.baseCommit,
    expected_execution_environment_ref: null,
  };
  const created = await owner.prepare(request);
  await assert.rejects(
    owner.prepare({
      ...request,
      expected_baseline_sha256: "d".repeat(64),
    }),
    (error) =>
      error instanceof WorkspaceError
      && error.code === "MANAGED_WORKTREE_BASELINE_SUBSTITUTION",
  );
  await assert.rejects(
    owner.prepare({ ...request, base_commit: "e".repeat(40) }),
    (error) => error instanceof WorkspaceError,
  );
  assert.equal(
    (await git(value.repositoryRoot, ["worktree", "list", "--porcelain"]))
      .split(/\r?\n/u)
      .filter((line) => line.startsWith("worktree ")).length,
    2,
  );
  assert.equal(await exists(created.worktree_path), true);
});

test("verified commit receives one exact task-owned local ref without merge or checkout mutation", async (t) => {
  const value = await fixture(t);
  const owner = new ManagedWorktreeOwner(value);
  const request = {
    task_ref: "f".repeat(64),
    task_id: "TASK-MANAGED-PROTECTED-REF",
    base_commit: value.baseCommit,
    expected_execution_environment_ref: null,
  };
  const created = await owner.prepare(request);
  const tree = await git(created.worktree_path, ["rev-parse", `${value.baseCommit}^{tree}`]);
  const resultCommit = await git(created.worktree_path, [
    "commit-tree",
    tree,
    "-p",
    value.baseCommit,
    "-m",
    "verified managed result",
  ]);
  await assert.rejects(
    owner.protectVerifiedResult({
      ...request,
      attempt: 1,
      writer_fence: 41,
      result_commit: resultCommit,
      expected_baseline_sha256: created.baseline_sha256,
      require_existing: true,
    }),
    (error) =>
      error instanceof WorkspaceError
      && error.code === "MANAGED_WORKTREE_PROTECTED_REF_REQUIRED",
  );
  assert.equal(
    await git(created.worktree_path, [
      "for-each-ref",
      "--format=%(objectname)",
      "--",
      `refs/lattice/managed/${request.task_ref}/attempt-1`,
    ]),
    "",
    "exact-replay mode must not create a missing protected ref",
  );
  const protectedResult = await owner.protectVerifiedResult({
    ...request,
    attempt: 1,
    writer_fence: 41,
    result_commit: resultCommit,
    expected_baseline_sha256: created.baseline_sha256,
  });
  assert.equal(protectedResult.replayed, false);
  assert.equal(protectedResult.writer_fence, 41);
  assert.equal(
    protectedResult.protected_ref,
    `refs/lattice/managed/${request.task_ref}/attempt-1`,
  );
  assert.equal(
    await git(created.worktree_path, [
      "show-ref",
      "--verify",
      "--hash",
      protectedResult.protected_ref,
    ]),
    resultCommit,
  );
  assert.equal(await git(created.worktree_path, ["rev-parse", "HEAD"]), value.baseCommit);
  const replay = await owner.protectVerifiedResult({
    ...request,
    attempt: 1,
    writer_fence: 41,
    result_commit: resultCommit,
    expected_baseline_sha256: created.baseline_sha256,
  });
  assert.equal(replay.replayed, true);
  assert.equal(replay.protected_ref_digest, protectedResult.protected_ref_digest);
  const differentFence = await owner.protectVerifiedResult({
    ...request,
    attempt: 1,
    writer_fence: 42,
    result_commit: resultCommit,
    expected_baseline_sha256: created.baseline_sha256,
  });
  assert.notEqual(
    differentFence.protected_ref_digest,
    protectedResult.protected_ref_digest,
    "protected evidence digest must bind the exact writer fence",
  );
  const baselineReplay = await owner.prepare({
    ...request,
    expected_baseline_sha256: created.baseline_sha256,
  });
  assert.equal(baselineReplay.baseline_sha256, created.baseline_sha256);
  assert.equal(baselineReplay.replayed, true);
  await git(value.repositoryRoot, ["pack-refs", "--all"]);
  assert.equal(
    (
      await owner.prepare({
        ...request,
        expected_baseline_sha256: created.baseline_sha256,
      })
    ).baseline_sha256,
    created.baseline_sha256,
    "packing the exact task-owned protected ref must not alter its baseline",
  );

  const foreignRef = "refs/lattice/foreign/managed-result";
  await git(value.repositoryRoot, ["update-ref", foreignRef, resultCommit]);
  await assert.rejects(
    owner.prepare({
      ...request,
      expected_baseline_sha256: created.baseline_sha256,
    }),
    (error) =>
      error instanceof WorkspaceError
      && error.code === "MANAGED_WORKTREE_BASELINE_SUBSTITUTION",
  );
  await git(value.repositoryRoot, ["update-ref", "-d", foreignRef]);
  assert.equal(
    (
      await owner.prepare({
        ...request,
        expected_baseline_sha256: created.baseline_sha256,
      })
    ).baseline_sha256,
    created.baseline_sha256,
  );
  await git(value.repositoryRoot, ["config", "lattice.foreign-control", "true"]);
  await assert.rejects(
    owner.prepare({
      ...request,
      expected_baseline_sha256: created.baseline_sha256,
    }),
    (error) =>
      error instanceof WorkspaceError
      && error.code === "MANAGED_WORKTREE_BASELINE_SUBSTITUTION",
  );
  await git(value.repositoryRoot, ["config", "--unset", "lattice.foreign-control"]);

  const substitutedCommit = await git(created.worktree_path, [
    "commit-tree",
    tree,
    "-p",
    value.baseCommit,
    "-m",
    "substituted managed result",
  ]);
  await assert.rejects(
    owner.protectVerifiedResult({
      ...request,
      attempt: 1,
      writer_fence: 41,
      result_commit: substitutedCommit,
      expected_baseline_sha256: created.baseline_sha256,
    }),
    (error) =>
      error instanceof WorkspaceError
      && error.code === "MANAGED_WORKTREE_PROTECTED_REF_SUBSTITUTION",
  );
});

test("protected ref CAS disables repository hooks and external Git helpers", async (t) => {
  const value = await fixture(t);
  const hookDirectory = path.join(value.root, "attacker-hooks");
  const marker = path.join(value.root, "reference-transaction-executed.txt");
  await mkdir(hookDirectory);
  const markerForShell = marker.replaceAll("\\", "/").replaceAll('"', '\\"');
  await writeFile(
    path.join(hookDirectory, "reference-transaction"),
    `#!/bin/sh\nprintf exploited > "${markerForShell}"\n`,
  );
  await git(value.repositoryRoot, ["config", "core.hooksPath", hookDirectory]);

  const owner = new ManagedWorktreeOwner(value);
  const request = {
    task_ref: "9".repeat(64),
    task_id: "TASK-MANAGED-HOOK-HARDENING",
    base_commit: value.baseCommit,
    expected_execution_environment_ref: null,
  };
  const created = await owner.prepare(request);
  const tree = await git(created.worktree_path, ["rev-parse", `${value.baseCommit}^{tree}`]);
  const resultCommit = await git(created.worktree_path, [
    "commit-tree",
    tree,
    "-p",
    value.baseCommit,
    "-m",
    "verified hook-hardened result",
  ]);
  const protectedResult = await owner.protectVerifiedResult({
    ...request,
    attempt: 1,
    writer_fence: 73,
    result_commit: resultCommit,
    expected_baseline_sha256: created.baseline_sha256,
  });

  assert.equal(protectedResult.writer_fence, 73);
  assert.equal(await exists(marker), false, "reference-transaction hook must not execute");
});
