import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import {
  buildWsl2SandboxPolicyTemplate,
  buildWsl2SandboxState,
  canonicalJson,
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
import {
  materializeWsl2ImmutableExecutionFacts,
  observeWsl2ImmutableExecutionState,
} from "../src/wsl2-execution-preflight.mjs";

const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const typed = (domain, marker) => `${domain}:sha256:${marker.repeat(64)}`;

function rehashDescriptor(descriptor) {
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

function replaceLinuxPrefix(value, from, to) {
  if (Array.isArray(value)) {
    for (const entry of value) replaceLinuxPrefix(entry, from, to);
    return;
  }
  if (value === null || typeof value !== "object") return;
  for (const [key, entry] of Object.entries(value)) {
    if (typeof entry === "string" && entry.startsWith(from)) {
      value[key] = `${to}${entry.slice(from.length)}`;
    } else {
      replaceLinuxPrefix(entry, from, to);
    }
  }
}

function fixture() {
  const taskRef = "7".repeat(64);
  const taskRoot = "/home/zk/lattice-phase4-wsl2-acceptance-20260828";
  const repo = `${taskRoot}/managed-worktrees/work-${taskRef}`;
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
    bwrap: "/usr/bin/bwrap",
    systemdRun: "/usr/bin/systemd-run",
    systemctl: "/usr/bin/systemctl",
    bootstrapNode: "/usr/bin/node",
    lsattr: "/usr/bin/lsattr",
    sudo: "/usr/bin/sudo",
  };
  const descriptor = {
    schema: "lattice.execution-environment.wsl2-linux/1.1",
    kind: "WSL2_LINUX",
    distribution: "Ubuntu",
    distribution_identity: {
      os_id: "ubuntu", os_version_id: "24.04", os_version_codename: "noble",
      os_release_sha256: sha256("os-release"),
      kernel_release: "6.6.87.2-microsoft-standard-WSL2", identity_digest: null,
    },
    gateway: {
      windows_path: String.raw`C:\Windows\System32\wsl.exe`, version: "2.6.1",
      sha256: sha256("wsl.exe"),
    },
    linux: {
      launcher_path: paths.launcher, launcher_version: "codex-cli 0.146.0",
      launcher_sha256: sha256(paths.launcher), node_path: paths.node, node_version: "v24.15.0",
      node_sha256: sha256(paths.node), git_path: paths.git, git_version: "git version 2.43.0",
      git_sha256: sha256(paths.git), supervisor_path: paths.supervisor,
      supervisor_sha256: sha256(paths.supervisor), codex_home: `${taskRoot}/codex-home`,
      config_digest: typed("codex-config", "1"), cwd: repo,
      repository_head: "0123456789abcdef0123456789abcdef01234567",
      repository_identity: typed("repository", "2"), dbus_run_session_path: paths.dbus,
      dbus_run_session_sha256: sha256(paths.dbus), setsid_path: paths.setsid,
      setsid_sha256: sha256(paths.setsid), keyring_daemon_path: paths.keyring,
      keyring_daemon_sha256: sha256(paths.keyring),
      keyring_library_path: `${taskRoot}/keyring-static-v1/packages`,
      keyring_library_manifest_digest: typed("keyring-library-manifest", "3"),
      xdg_runtime_dir: "/run/user/1000",
    },
    credential_authority: { kind: "LINUX_KEYRING", authority_digest: null },
    process_fence: {
      schema: "lattice.wsl2-cgroup-v2-fence/1.0", kind: "SYSTEMD_USER_SERVICE_CGROUP_V2",
      systemd_run_path: paths.systemdRun, systemd_run_version: "systemd 255 (255.4-1ubuntu8.11)",
      systemd_run_sha256: sha256(paths.systemdRun), systemctl_path: paths.systemctl,
      systemctl_version: "systemd 255 (255.4-1ubuntu8.11)", systemctl_sha256: sha256(paths.systemctl),
      cgroup_mount: "/sys/fs/cgroup", user_runtime_dir: "/run/user/1000",
      unit_prefix: `lattice-wsl2-${taskRef.slice(0, 16)}`,
      supervisor_bootstrap_node: { path: paths.bootstrapNode, version: "v22.22.1", sha256: sha256(paths.bootstrapNode) },
      immutable_probe_lsattr: { path: paths.lsattr, version: "lsattr 1.47.0 (5-Feb-2023)", sha256: sha256(paths.lsattr) },
      noninteractive_root_probe: { path: paths.sudo, version: "Sudo version 1.9.15p5", sha256: sha256(paths.sudo) },
      identity_digest: null,
    },
    verification_toolchain: {
      schema: "lattice.wsl2-verification-toolchain/1.0", task_ref: taskRef, task_root: taskRoot,
      isolation_root: isolation, owner_uid: 1000, home_dir: `${isolation}/home`,
      temp_dir: `${isolation}/tmp`, npm_cache: `${isolation}/npm-cache`,
      cargo_home: `${isolation}/cargo-home`, cargo_target_dir: `${isolation}/cargo-target`,
      cargo_host: "x86_64-unknown-linux-gnu",
      npm: { path: paths.npm, version: "11.12.1", sha256: sha256(paths.npm) },
      cargo: { path: paths.cargo, version: "cargo 1.97.1 (c980f4866 2026-03-10)", sha256: sha256(paths.cargo) },
      rustc: { path: paths.rustc, version: "rustc 1.97.1 (8bab26f4f 2026-03-10)", sha256: sha256(paths.rustc) },
      rustdoc: { path: paths.rustdoc, version: "rustdoc 1.97.1 (8bab26f4f 2026-03-10)", sha256: sha256(paths.rustdoc) },
      sandbox: { path: paths.launcher, version: "codex-cli 0.146.0", sha256: sha256(paths.launcher) },
      sandbox_helper: { path: paths.bwrap, version: "bubblewrap 0.11.0", sha256: sha256(paths.bwrap) },
      identity_digest: null,
    },
    immutable_snapshot: {
      schema: "lattice.wsl2-immutable-snapshot/1.0", task_root_path: taskRoot,
      task_root_device: "2049", task_root_inode: "40001", task_root_owner_uid: 0,
      task_root_owner_gid: 0, task_root_mode: "0555", task_root_immutable: true,
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
      schema: "lattice.wsl2-privilege-boundary/1.0", effective_uid: 1000, effective_gid: 1000,
      effective_capabilities_digest: typed("linux-capabilities", "0"),
      noninteractive_root_unavailable: true, boundary_digest: null,
    },
    path_mapping: {
      windows_path: `\\\\wsl.localhost\\Ubuntu${repo.replaceAll("/", "\\")}`,
      linux_path: repo, digest: typed("path-mapping", "4"),
    },
    identity_digest: null,
  };
  return rehashDescriptor(descriptor);
}

test("descriptor binds exact immutable snapshot, sandbox template, and privilege boundary", () => {
  const descriptor = fixture();
  assert.deepEqual(validateWsl2ExecutionEnvironment(descriptor), descriptor);
  const template = buildWsl2SandboxPolicyTemplate(descriptor);
  assert.deepEqual(template.base_entries, [
    { path: { type: "special", value: { kind: "minimal" } }, access: "read" },
    { path: { type: "path", path: descriptor.verification_toolchain.task_root }, access: "read" },
  ]);
  assert.ok(template.role_writes.PREFLIGHT.includes(descriptor.linux.cwd));
  assert.ok(template.deny_entries.every((entry) => entry.missing_path_behavior === "skip"));

  for (const mutation of [
    (value) => { value.immutable_snapshot.trees.node.manifest_digest = typed("immutable-tree-manifest", "f"); },
    (value) => { value.sandbox_policy.policy_digest = typed("wsl2-sandbox-policy", "f"); },
    (value) => { value.privilege_boundary.effective_uid = 1001; },
    (value) => { value.immutable_snapshot.extra = true; },
    (value) => { value.process_fence.immutable_probe_lsattr.sha256 = "f".repeat(64); },
  ]) {
    const changed = structuredClone(descriptor);
    mutation(changed);
    assert.throws(() => validateWsl2ExecutionEnvironment(changed), /WSL2_EXECUTION_ENVIRONMENT_REJECTED/u);
  }
  const nestedTree = structuredClone(descriptor);
  nestedTree.immutable_snapshot.trees.node.root = `${nestedTree.immutable_snapshot.trees.node.root}/root`;
  nestedTree.immutable_snapshot.snapshot_digest = immutableSnapshotIdentity(nestedTree);
  nestedTree.identity_digest = executionEnvironmentIdentity(nestedTree);
  assert.throws(() => validateWsl2ExecutionEnvironment(nestedTree),
    /WSL2_EXECUTION_ENVIRONMENT_REJECTED/u);
});

test("digest-valid descriptor rejects credential-shaped strings in every leaf class", () => {
  const cases = [
    ["kernel password", "password=phase4-password-sentinel", (value, sentinel) => {
      value.distribution_identity.kernel_release =
        `${sentinel}-6.6.87.2-microsoft-standard-WSL2`;
    }],
    ["kernel token", "token=phase4-token-sentinel", (value, sentinel) => {
      value.distribution_identity.kernel_release =
        `${sentinel}-6.6.87.2-microsoft-standard-WSL2`;
    }],
    ["kernel secret", "secret=phase4-secret-sentinel", (value, sentinel) => {
      value.distribution_identity.kernel_release =
        `${sentinel}-6.6.87.2-microsoft-standard-WSL2`;
    }],
    ["kernel API key", "api key=phase4-api-key-sentinel", (value, sentinel) => {
      value.distribution_identity.kernel_release =
        `${sentinel}-6.6.87.2-microsoft-standard-WSL2`;
    }],
    ["kernel bearer", "Bearer phase4-bearer-sentinel", (value, sentinel) => {
      value.distribution_identity.kernel_release =
        `${sentinel}-6.6.87.2-microsoft-standard-WSL2`;
    }],
    ["task root", "ghp_phase4taskrootsentinel", (value, sentinel) => {
      const prior = value.verification_toolchain.task_root;
      const replacement = `/home/zk/${sentinel}`;
      replaceLinuxPrefix(value, prior, replacement);
      value.path_mapping.windows_path =
        `\\\\wsl.localhost\\Ubuntu${value.linux.cwd.replaceAll("/", "\\")}`;
    }],
    ["isolated home", "github_pat_phase4homesentinel", (value, sentinel) => {
      value.verification_toolchain.home_dir =
        `${value.verification_toolchain.isolation_root}/${sentinel}`;
    }],
    ["repository cwd", "sk-phase4repositorysentinel", (value, sentinel) => {
      const cwd = `${value.verification_toolchain.task_root}/managed-worktrees/${sentinel}`;
      value.linux.cwd = cwd;
      value.path_mapping.linux_path = cwd;
      value.path_mapping.windows_path =
        `\\\\wsl.localhost\\Ubuntu${cwd.replaceAll("/", "\\")}`;
    }],
    ["tool path", "gho_phase4toolpathsentinel", (value, sentinel) => {
      value.linux.node_path =
        `${value.immutable_snapshot.trees.node.root}/root/bin/${sentinel}`;
    }],
  ];

  for (const [label, sentinel, mutate] of cases) {
    const descriptor = fixture();
    mutate(descriptor, sentinel);
    rehashDescriptor(descriptor);
    let failure;
    try {
      validateWsl2ExecutionEnvironment(descriptor);
    } catch (error) {
      failure = error;
    }
    assert.ok(failure, `${label} credential-shaped leaf was accepted`);
    assert.equal(String(failure).includes(sentinel), false, `${label} leaked rejected input`);
    assert.match(String(failure), /WSL2_EXECUTION_ENVIRONMENT_REJECTED/u);
  }
});

test("sandbox state uses minimal/task-root reads and explicit skip-on-missing denies", () => {
  const descriptor = fixture();
  const state = buildWsl2SandboxState(descriptor, {
    role: "NODE",
    cwd: descriptor.linux.cwd,
    writableRoots: [descriptor.verification_toolchain.home_dir,
      descriptor.verification_toolchain.temp_dir, descriptor.verification_toolchain.npm_cache],
    deniedRoots: [descriptor.linux.codex_home],
  });
  const entries = state.permissionProfile.file_system.entries;
  assert.deepEqual(entries.slice(0, 2), buildWsl2SandboxPolicyTemplate(descriptor).base_entries);
  assert.ok(entries.filter((entry) => entry.access === "deny")
    .every((entry) => entry.missing_path_behavior === "skip"));
  assert.equal(entries.some((entry) => entry.path?.path === "/"), false);
});

test("immutable tree roots are direct children and pairwise non-overlapping", () => {
  const descriptor = fixture();
  const roots = Object.values(descriptor.immutable_snapshot.trees).map((tree) => tree.root);
  assert.equal(new Set(roots).size, roots.length);
  assert.ok(roots.every((root) => root.slice(descriptor.verification_toolchain.task_root.length + 1)
    .includes("/") === false));
  assert.ok(roots.every((root, index) => roots.every((other, otherIndex) => index === otherIndex
    || (!root.startsWith(`${other}/`) && !other.startsWith(`${root}/`)))));

  const overlap = structuredClone(descriptor);
  overlap.immutable_snapshot.trees.keyring.root = overlap.immutable_snapshot.trees.codex.root;
  overlap.immutable_snapshot.snapshot_digest = immutableSnapshotIdentity(overlap);
  overlap.identity_digest = executionEnvironmentIdentity(overlap);
  assert.throws(() => validateWsl2ExecutionEnvironment(overlap),
    /WSL2_EXECUTION_ENVIRONMENT_REJECTED/u);
});

test("live observation binds all five manifests, +i task root, and privilege boundary", async () => {
  const descriptor = fixture();
  const sourceFacts = {
    schema: "lattice.wsl2-immutable-observation-source/1.0",
    task_root: {
      path: descriptor.immutable_snapshot.task_root_path, device: "2049", inode: "40001",
      owner_uid: 0, owner_gid: 0, mode: "0555", immutable: true,
    },
    trees: Object.fromEntries(Object.entries(descriptor.immutable_snapshot.trees).map(
      ([name, tree], index) => [name, {
        root: tree.root, manifest_digest: tree.manifest_digest,
        entry_count: index + 1, file_bytes: 1024 + index,
      }],
    )),
    privilege: {
      effective_uid: 1000, effective_gid: 1000,
      effective_capabilities_digest: descriptor.privilege_boundary.effective_capabilities_digest,
      capabilities_empty: true, noninteractive_root_unavailable: true,
      sudo_denial_recognized: true,
      sudo_exit_code: 1, sudo_stdout_bytes: 0, sudo_stderr_bytes: 29,
      sudo_stdout_sha256: sha256(Buffer.alloc(0)), sudo_stderr_sha256: sha256("sudo denied"),
    },
    bounds: { max_entries_per_tree: 200000, max_file_bytes_per_tree: 8589934592, max_single_file_bytes: 1073741824 },
  };
  const run = async (program, args) => {
    if (program === "/usr/bin/sha256sum") {
      const tool = [descriptor.process_fence.supervisor_bootstrap_node,
        descriptor.process_fence.immutable_probe_lsattr,
        descriptor.process_fence.noninteractive_root_probe].find((candidate) => candidate.path === args[0]);
      return { stdout: `${tool.sha256}  ${tool.path}\n`, stderr: "" };
    }
    if (program === descriptor.process_fence.immutable_probe_lsattr.path) {
      return { stdout: `----i---------e------- ${descriptor.verification_toolchain.task_root}\n`,
        stderr: `${descriptor.process_fence.immutable_probe_lsattr.version}\n` };
    }
    if (program === descriptor.process_fence.noninteractive_root_probe.path) {
      return { stdout: `${descriptor.process_fence.noninteractive_root_probe.version}\n`, stderr: "" };
    }
    if (program === descriptor.process_fence.supervisor_bootstrap_node.path
        && args[0] === "--version") {
      return { stdout: `${descriptor.process_fence.supervisor_bootstrap_node.version}\n`, stderr: "" };
    }
    if (program === descriptor.process_fence.supervisor_bootstrap_node.path && args[0] === "-e") {
      return { stdout: `${JSON.stringify(sourceFacts)}\n`, stderr: "" };
    }
    throw new Error(`unexpected ${program} ${args.join(" ")}`);
  };
  const observed = await observeWsl2ImmutableExecutionState(descriptor, { run });
  assert.equal(observed.schema, "lattice.wsl2-immutable-observation/1.0");
  assert.equal(observed.immutable_snapshot_ref, descriptor.immutable_snapshot.snapshot_digest);
  assert.equal(observed.privilege_boundary_ref, descriptor.privilege_boundary.boundary_digest);
  assert.equal(observed.probe_tools.lsattr.sha256,
    descriptor.process_fence.immutable_probe_lsattr.sha256);
  assert.match(observed.observation_digest, /^wsl2-immutable-observation:sha256:[a-f0-9]{64}$/u);
});

test("live observation fails closed on manifest, inode, +i, privilege, and probe substitution", async () => {
  const base = fixture();
  const facts = {
    schema: "lattice.wsl2-immutable-observation-source/1.0",
    task_root: { path: base.immutable_snapshot.task_root_path, device: "2049", inode: "40001",
      owner_uid: 0, owner_gid: 0, mode: "0555", immutable: true },
    trees: Object.fromEntries(Object.entries(base.immutable_snapshot.trees).map(([name, tree]) => [name, {
      root: tree.root, manifest_digest: tree.manifest_digest, entry_count: 1, file_bytes: 1,
    }])),
    privilege: { effective_uid: 1000, effective_gid: 1000,
      effective_capabilities_digest: base.privilege_boundary.effective_capabilities_digest,
      capabilities_empty: true, noninteractive_root_unavailable: true, sudo_exit_code: 1,
      sudo_denial_recognized: true,
      sudo_stdout_bytes: 0, sudo_stderr_bytes: 1, sudo_stdout_sha256: sha256(Buffer.alloc(0)),
      sudo_stderr_sha256: sha256("x") },
    bounds: { max_entries_per_tree: 200000, max_file_bytes_per_tree: 8589934592, max_single_file_bytes: 1073741824 },
  };
  for (const mutate of [
    (value) => { value.trees.rust.manifest_digest = typed("immutable-tree-manifest", "f"); },
    (value) => { value.task_root.inode = "40002"; },
    (value) => { value.task_root.immutable = false; },
    (value) => { value.privilege.capabilities_empty = false; },
  ]) {
    const changed = structuredClone(facts);
    mutate(changed);
    await assert.rejects(observeWsl2ImmutableExecutionState(base, { run: async (program, args) => {
      if (program === "/usr/bin/sha256sum") {
        const tool = [base.process_fence.supervisor_bootstrap_node,
          base.process_fence.immutable_probe_lsattr,
          base.process_fence.noninteractive_root_probe].find((candidate) => candidate.path === args[0]);
        return { stdout: `${tool.sha256}  ${tool.path}\n`, stderr: "" };
      }
      if (program === base.process_fence.immutable_probe_lsattr.path) return {
        stdout: `----i---------e------- ${base.verification_toolchain.task_root}\n`,
        stderr: `${base.process_fence.immutable_probe_lsattr.version}\n`,
      };
      if (program === base.process_fence.noninteractive_root_probe.path) return { stdout: `${base.process_fence.noninteractive_root_probe.version}\n`, stderr: "" };
      if (program === base.process_fence.supervisor_bootstrap_node.path && args[0] === "--version") {
        return { stdout: `${base.process_fence.supervisor_bootstrap_node.version}\n`, stderr: "" };
      }
      return { stdout: `${JSON.stringify(changed)}\n`, stderr: "" };
    } }), /WSL2_PREFLIGHT_IMMUTABLE_(?:SNAPSHOT|PRIVILEGE)_REJECTED/u);
  }
});

test("live materialization ignores supplied manifests and returns sealable typed facts", async () => {
  const descriptor = fixture();
  const draft = structuredClone(descriptor);
  for (const tree of Object.values(draft.immutable_snapshot.trees)) tree.manifest_digest = null;
  const facts = {
    schema: "lattice.wsl2-immutable-observation-source/1.0",
    task_root: { path: descriptor.immutable_snapshot.task_root_path, device: "2049", inode: "40001",
      owner_uid: 0, owner_gid: 0, mode: "0555", immutable: true },
    trees: Object.fromEntries(Object.entries(descriptor.immutable_snapshot.trees).map(([name, tree]) => [name, {
      root: tree.root, manifest_digest: tree.manifest_digest, entry_count: 2, file_bytes: 4096,
    }])),
    privilege: { effective_uid: 1000, effective_gid: 1000,
      effective_capabilities_digest: descriptor.privilege_boundary.effective_capabilities_digest,
      capabilities_empty: true, noninteractive_root_unavailable: true, sudo_denial_recognized: true,
      sudo_exit_code: 1, sudo_stdout_bytes: 0, sudo_stderr_bytes: 10,
      sudo_stdout_sha256: sha256(Buffer.alloc(0)), sudo_stderr_sha256: sha256("auth denied") },
    bounds: { max_entries_per_tree: 200000, max_file_bytes_per_tree: 8589934592,
      max_single_file_bytes: 1073741824 },
  };
  const tools = [descriptor.process_fence.supervisor_bootstrap_node,
    descriptor.process_fence.immutable_probe_lsattr, descriptor.process_fence.noninteractive_root_probe];
  const result = await materializeWsl2ImmutableExecutionFacts(draft, { run: async (program, args) => {
    if (program === "/usr/bin/sha256sum") {
      const tool = tools.find((candidate) => candidate.path === args[0]);
      return { stdout: `${tool.sha256}  ${tool.path}\n`, stderr: "" };
    }
    if (program === descriptor.process_fence.immutable_probe_lsattr.path) return {
      stdout: `----i---------e------- ${descriptor.verification_toolchain.task_root}\n`,
      stderr: `${descriptor.process_fence.immutable_probe_lsattr.version}\n`,
    };
    if (program === descriptor.process_fence.noninteractive_root_probe.path) {
      return { stdout: `${descriptor.process_fence.noninteractive_root_probe.version}\n`, stderr: "" };
    }
    if (program === descriptor.process_fence.supervisor_bootstrap_node.path
        && args[0] === "--version") {
      return { stdout: `${descriptor.process_fence.supervisor_bootstrap_node.version}\n`, stderr: "" };
    }
    return { stdout: `${JSON.stringify(facts)}\n`, stderr: "" };
  } });
  assert.equal(result.immutable_snapshot.trees.node.manifest_digest,
    descriptor.immutable_snapshot.trees.node.manifest_digest);
  assert.match(result.immutable_snapshot.snapshot_digest,
    /^wsl2-immutable-snapshot:sha256:[a-f0-9]{64}$/u);
  assert.match(result.privilege_boundary.boundary_digest,
    /^wsl2-privilege-boundary:sha256:[a-f0-9]{64}$/u);
  assert.match(result.evidence.materialization_digest,
    /^wsl2-immutable-materialization:sha256:[a-f0-9]{64}$/u);
});
