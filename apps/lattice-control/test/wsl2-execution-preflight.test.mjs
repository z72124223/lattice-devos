import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import path from "node:path";
import test from "node:test";

import {
  bindWsl2ExecutionWorktree,
  buildLegacyWsl2CodexLaunchFixture,
  buildWsl2CodexLaunch,
  buildWsl2VerifierLaunch,
  canonicalJson,
  codexHomeIdentity,
  credentialAuthorityIdentity,
  distributionIdentity,
  executionEnvironmentIdentity,
  immutableSnapshotIdentity,
  isClosedWsl2ToolVersion,
  linuxCapabilitiesIdentity,
  pathMappingIdentity,
  processFenceAuthorityIdentity,
  privilegeBoundaryIdentity,
  sandboxPolicyIdentity,
  validateWsl2ExecutionEnvironment,
  verificationToolchainIdentity,
  WSL2_SUPERVISOR_BOOTSTRAP_SOURCE,
} from "../src/wsl2-execution-domain.mjs";
import {
  completeWsl2ConnectorPreflight,
  preflightWsl2ExecutionEnvironment,
} from "../src/wsl2-execution-preflight.mjs";
import {
  deriveWsl2GitControlRootIdentity,
  runWsl2VerifierBridge,
  validateWsl2VerifierBridgeRequest,
} from "../src/wsl2-verifier-bridge.mjs";

const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const typed = (kind, value) => `${kind}:sha256:${value.repeat(64)}`;

function seal(pathname, digest, ownerUid, index, extra = {}) {
  return {
    ...extra,
    path: pathname,
    resolved_path: pathname,
    sha256: digest,
    device: "2049",
    inode: String(20000 + index),
    owner_uid: ownerUid,
    mode: 0o500,
    size: 4096,
  };
}

function completeSupervisorExit(environment, role, fields = {}) {
  const toolchain = environment.verification_toolchain;
  const executable = role === "PROVIDER"
    ? { path: environment.linux.launcher_path, sha256: environment.linux.launcher_sha256 }
    : toolchain.sandbox;
  const verifier = role === "NODE" ? toolchain.npm
    : role === "CARGO" ? toolchain.cargo
      : role === "GIT" ? { path: environment.linux.git_path, sha256: environment.linux.git_sha256 }
        : null;
  const nodeRuntime = ["PREFLIGHT", "NODE"].includes(role)
    ? { path: environment.linux.node_path, sha256: environment.linux.node_sha256 }
    : null;
  const rustc = role === "CARGO" ? toolchain.rustc : null;
  const rustdoc = role === "CARGO" ? toolchain.rustdoc : null;
  const emptySha = sha256(Buffer.alloc(0));
  return {
    schema: "lattice.wsl2-subtree-exit/1.2",
    fence: fields.fence,
    unit: fields.unit,
    execution_environment_ref: fields.environmentRef ?? environment.identity_digest,
    credential_seal_digest: fields.credentialSeal,
    cgroup_path: fields.cgroupPath,
    zero_descendants: true,
    credential_seal_intact: true,
    credential_watch_intact: true,
    keyring_daemon_sha256: environment.linux.keyring_daemon_sha256,
    keyring_library_manifest_digest: environment.linux.keyring_library_manifest_digest,
    tool_input_identities: {
      executable: seal(executable.path, executable.sha256, 0, 1),
      verifier_tool: verifier === null ? null : seal(verifier.path, verifier.sha256,
        0, 2),
      sandbox_helper: seal(toolchain.sandbox_helper.path, toolchain.sandbox_helper.sha256, 0, 3),
      node_runtime: nodeRuntime === null ? null
        : seal(nodeRuntime.path, nodeRuntime.sha256, 0, 4),
      rustc: rustc === null ? null : seal(rustc.path, rustc.sha256, 0, 5),
      rustdoc: rustdoc === null ? null : seal(rustdoc.path, rustdoc.sha256, 0, 6),
      keyring_daemon: seal(environment.linux.keyring_daemon_path,
        environment.linux.keyring_daemon_sha256, 0, 7),
      keyring_libraries: ["libgck-1.so.0.0.0", "libgcr-base-3.so.1.0.0"].map(
        (manifestPath, index) => seal(`${environment.linux.keyring_library_path}/${manifestPath}`,
          sha256(Buffer.from(manifestPath, "utf8")), 0, 8 + index,
          { manifest_path: manifestPath }),
      ),
    },
    stdout_bytes: fields.stdoutBytes ?? 0,
    stderr_bytes: fields.stderrBytes ?? 0,
    stdout_limit_bytes: fields.stdoutLimit ?? 262_144,
    stderr_limit_bytes: fields.stderrLimit ?? 262_144,
    output_bound_exceeded: false,
    timeout_ms: fields.timeoutMs ?? 120_000,
    timed_out: false,
    interrupted: false,
    stdin_bytes: fields.stdin?.length ?? 0,
    stdin_sha256: fields.stdin === undefined ? emptySha : sha256(fields.stdin),
    stdin_complete: true,
    attempt: fields.attempt ?? 1,
    retry_of: fields.retryOf ?? null,
    reconnect_of: fields.reconnectOf ?? null,
    exit_code: fields.exitCode ?? 0,
    exit_signal: fields.exitSignal ?? null,
  };
}

function fixture(contextOverrides = {}) {
  const taskRef = "7".repeat(64);
  const fence = "f".repeat(64);
  const attempt = contextOverrides.attempt ?? 1;
  const retryOf = contextOverrides.retryOf ?? null;
  const reconnectOf = contextOverrides.reconnectOf ?? null;
  const taskRoot = "/home/zk/lattice-phase4-wsl2-acceptance-20260828";
  const repository = `${taskRoot}/managed-worktrees/work-${taskRef}`;
  const commonGit = `${taskRoot}/repository/.git`;
  const isolationRoot = `${taskRoot}/verifier-state/${taskRef}`;
  const gatewayBytes = Buffer.from("pinned wsl gateway", "utf8");
  const headBytes = Buffer.from("ref: refs/heads/main\n", "utf8");
  const osRelease = [
    'PRETTY_NAME="Ubuntu 26.04 LTS"',
    'NAME="Ubuntu"',
    'VERSION_ID="26.04"',
    'VERSION="26.04 LTS (Resolute Raccoon)"',
    'VERSION_CODENAME=resolute',
    'ID=ubuntu',
    "",
  ].join("\n");
  const bootId = "11111111-2222-3333-4444-555555555555\n";
  const configDigest = sha256("config");
  const paths = {
    launcher: `${taskRoot}/codex/bin/codex`,
    node: `${taskRoot}/toolchain-node-24.15.0/root/bin/node`,
    bootstrapNode: "/usr/bin/node",
    git: "/usr/bin/git",
    supervisor: `${taskRoot}/runtime-v1/wsl2-codex-supervisor.mjs`,
    dbus: "/usr/bin/dbus-run-session",
    setsid: "/usr/bin/setsid",
    keyring: `${taskRoot}/keyring-static-v1/root/usr/bin/gnome-keyring-daemon`,
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
  const fileDigests = Object.fromEntries(Object.values(paths).map((file) => [file, sha256(file)]));
  const descriptor = {
    schema: "lattice.execution-environment.wsl2-linux/1.1",
    kind: "WSL2_LINUX",
    distribution: "Ubuntu",
    distribution_identity: {
      os_id: "ubuntu",
      os_version_id: "26.04",
      os_version_codename: "resolute",
      os_release_sha256: sha256(osRelease),
      kernel_release: "6.18.33.2-microsoft-standard-WSL2",
      identity_digest: null,
    },
    gateway: {
      windows_path: String.raw`C:\Windows\System32\wsl.exe`,
      version: "2.6.1",
      sha256: sha256(gatewayBytes),
    },
    linux: {
      launcher_path: paths.launcher,
      launcher_version: "codex-cli 0.146.0",
      launcher_sha256: fileDigests[paths.launcher],
      node_path: paths.node,
      node_version: "v24.15.0",
      node_sha256: fileDigests[paths.node],
      git_path: paths.git,
      git_version: "git version 2.53.0",
      git_sha256: fileDigests[paths.git],
      supervisor_path: paths.supervisor,
      supervisor_sha256: fileDigests[paths.supervisor],
      codex_home: `${taskRoot}/codex-home`,
      config_digest: `codex-config:sha256:${configDigest}`,
      cwd: repository,
      repository_head: "0123456789abcdef0123456789abcdef01234567",
      repository_identity: typed("repository", "a"),
      dbus_run_session_path: paths.dbus,
      dbus_run_session_sha256: fileDigests[paths.dbus],
      setsid_path: paths.setsid,
      setsid_sha256: fileDigests[paths.setsid],
      keyring_daemon_path: paths.keyring,
      keyring_daemon_sha256: fileDigests[paths.keyring],
      keyring_library_path: `${taskRoot}/keyring-static-v1/packages`,
      keyring_library_manifest_digest: typed("keyring-library-manifest", "e"),
      xdg_runtime_dir: "/run/user/1000",
    },
    credential_authority: {
      kind: "LINUX_KEYRING",
      authority_digest: null,
    },
    process_fence: {
      schema: "lattice.wsl2-cgroup-v2-fence/1.0",
      kind: "SYSTEMD_USER_SERVICE_CGROUP_V2",
      systemd_run_path: paths.systemdRun,
      systemd_run_version: "systemd 259 (259.5-0ubuntu3.4)",
      systemd_run_sha256: fileDigests[paths.systemdRun],
      systemctl_path: paths.systemctl,
      systemctl_version: "systemd 259 (259.5-0ubuntu3.4)",
      systemctl_sha256: fileDigests[paths.systemctl],
      cgroup_mount: "/sys/fs/cgroup",
      user_runtime_dir: "/run/user/1000",
      unit_prefix: `lattice-wsl2-${taskRef.slice(0, 16)}`,
      supervisor_bootstrap_node: {
        path: paths.bootstrapNode,
        version: "v22.22.1",
        sha256: fileDigests[paths.bootstrapNode],
      },
      immutable_probe_lsattr: {
        path: paths.lsattr,
        version: "lsattr 1.47.2 (1-Jan-2025)",
        sha256: fileDigests[paths.lsattr],
      },
      noninteractive_root_probe: {
        path: paths.sudo,
        version: "sudo-rs 0.2.13-0ubuntu1",
        sha256: fileDigests[paths.sudo],
      },
      identity_digest: null,
    },
    verification_toolchain: {
      schema: "lattice.wsl2-verification-toolchain/1.0",
      task_ref: taskRef,
      task_root: taskRoot,
      isolation_root: isolationRoot,
      owner_uid: 1000,
      home_dir: `${isolationRoot}/home`,
      temp_dir: `${isolationRoot}/tmp`,
      npm_cache: `${isolationRoot}/npm-cache`,
      cargo_home: `${isolationRoot}/cargo-home`,
      cargo_target_dir: `${isolationRoot}/cargo-target`,
      cargo_host: "x86_64-unknown-linux-gnu",
      npm: { path: paths.npm, version: "11.12.1", sha256: fileDigests[paths.npm] },
      cargo: {
        path: paths.cargo,
        version: "cargo 1.97.1 (c980f4866 2026-03-10)",
        sha256: fileDigests[paths.cargo],
      },
      rustc: {
        path: paths.rustc,
        version: "rustc 1.97.1 (8bab26f4f 2026-03-10)",
        sha256: fileDigests[paths.rustc],
      },
      rustdoc: {
        path: paths.rustdoc,
        version: "rustdoc 1.97.1 (8bab26f4f 2026-03-10)",
        sha256: fileDigests[paths.rustdoc],
      },
      sandbox: {
        path: paths.launcher,
        version: "codex-cli 0.146.0",
        sha256: fileDigests[paths.launcher],
      },
      sandbox_helper: {
        path: paths.sandboxHelper,
        version: "bubblewrap 0.11.1",
        sha256: fileDigests[paths.sandboxHelper],
      },
      identity_digest: null,
    },
    path_mapping: {
      windows_path: `\\\\wsl.localhost\\Ubuntu${repository.replaceAll("/", "\\")}`,
      linux_path: repository,
      digest: typed("path-mapping", "b"),
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
        codex: { root: `${taskRoot}/codex`, manifest_digest: typed("immutable-tree-manifest", "1") },
        supervisor_runtime: {
          root: `${taskRoot}/runtime-v1`, manifest_digest: typed("immutable-tree-manifest", "2"),
        },
        node: {
          root: `${taskRoot}/toolchain-node-24.15.0`,
          manifest_digest: typed("immutable-tree-manifest", "3"),
        },
        rust: {
          root: `${taskRoot}/toolchain-rust-1.97.1`,
          manifest_digest: typed("immutable-tree-manifest", "4"),
        },
        keyring: {
          root: `${taskRoot}/keyring-static-v1`,
          manifest_digest: typed("immutable-tree-manifest", "5"),
        },
      },
      snapshot_digest: null,
    },
    sandbox_policy: {
      schema: "lattice.wsl2-sandbox-policy/1.0",
      policy_digest: null,
    },
    privilege_boundary: {
      schema: "lattice.wsl2-privilege-boundary/1.0",
      effective_uid: 1000,
      effective_gid: 1000,
      effective_capabilities_digest: linuxCapabilitiesIdentity({
        effective_uid: 1000,
        effective_gid: 1000,
        proc_status_cap_eff: "0000000000000000",
      }),
      noninteractive_root_unavailable: true,
      boundary_digest: null,
    },
    identity_digest: null,
  };
  descriptor.distribution_identity.identity_digest = distributionIdentity(descriptor);
  descriptor.credential_authority.authority_digest = credentialAuthorityIdentity(descriptor);
  descriptor.process_fence.identity_digest = processFenceAuthorityIdentity(descriptor);
  descriptor.verification_toolchain.identity_digest = verificationToolchainIdentity(descriptor);
  descriptor.immutable_snapshot.snapshot_digest = immutableSnapshotIdentity(descriptor);
  descriptor.sandbox_policy.policy_digest = sandboxPolicyIdentity(descriptor);
  descriptor.privilege_boundary.boundary_digest = privilegeBoundaryIdentity(descriptor);
  descriptor.path_mapping.digest = pathMappingIdentity(descriptor);
  descriptor.identity_digest = executionEnvironmentIdentity(descriptor);

  const credentialFacts = {
    config_regular_file: true,
    config_sha256: configDigest,
    config_identity: {
      device: "2049", inode: "9988", owner_uid: 1000, mode: "100400", size: 6,
    },
    config_owner_matches: true,
    keyring_only: true,
    auth_json_absent: true,
    shell_environment_policy: {
      inherit: "all",
      ignore_default_excludes: false,
      include_only: ["HOME", "PATH", "LANG", "LC_ALL", "TERM", "COLORTERM"],
      experimental_use_profile: false,
      set_keys: [],
      probe_effective_keys: ["HOME", "PATH", "LANG", "LC_ALL", "TERM", "COLORTERM"],
      required_keys_present: true,
      forbidden_keys_absent: true,
    },
  };
  const cgroupUnit = `${descriptor.process_fence.unit_prefix}-preflight-${fence.slice(0, 12)}.service`;
  const cgroupFacts = {
    version: 2,
    path: `/user.slice/user-1000.slice/user@1000.service/app.slice/${cgroupUnit}`,
    type: "domain",
    delegated: false,
    owner_uid: 1000,
  };
  const isolationPaths = [isolationRoot, `${isolationRoot}/home`, `${isolationRoot}/tmp`,
    `${isolationRoot}/npm-cache`, `${isolationRoot}/cargo-home`, `${isolationRoot}/cargo-target`];
  const isolationFacts = {
    observations: Object.fromEntries(isolationPaths.map((candidate, index) => [candidate, {
      realpath: candidate, directory: true, symlink: false, owner_uid: 1000, owner_matches: true,
      mode: "40700", device: "2049", inode: String(12000 + index),
    }])),
  };
  const immutableFacts = {
    schema: "lattice.wsl2-immutable-observation-source/1.0",
    task_root: {
      path: taskRoot, device: "2049", inode: "40001", owner_uid: 0, owner_gid: 0,
      mode: "0555", immutable: true,
    },
    trees: Object.fromEntries(Object.entries(descriptor.immutable_snapshot.trees).map(
      ([name, tree], index) => [name, {
        root: tree.root,
        manifest_digest: tree.manifest_digest,
        entry_count: index + 2,
        file_bytes: 4096 + index,
      }],
    )),
    privilege: {
      effective_uid: 1000,
      effective_gid: 1000,
      effective_capabilities_digest: descriptor.privilege_boundary.effective_capabilities_digest,
      capabilities_empty: true,
      noninteractive_root_unavailable: true,
      sudo_denial_recognized: true,
      sudo_exit_code: 1,
      sudo_stdout_bytes: 0,
      sudo_stderr_bytes: 45,
      sudo_stdout_sha256: sha256(Buffer.alloc(0)),
      sudo_stderr_sha256: sha256(Buffer.from("sudo: interactive authentication is required\n", "utf8")),
    },
    bounds: {
      max_entries_per_tree: 200_000,
      max_file_bytes_per_tree: 8 * 1_073_741_824,
      max_single_file_bytes: 1_073_741_824,
    },
  };
  const calls = [];
  const execFile = async (executable, args) => {
    calls.push([executable, [...args]]);
    assert.equal(executable, descriptor.gateway.windows_path);
    assert.deepEqual(args.slice(0, 4), ["-d", "Ubuntu", "--exec", "/usr/bin/env"]);
    assert.equal(args[4], "-i");
    const programIndex = args.findIndex((arg, index) => index >= 5 && arg.startsWith("/"));
    assert.ok(programIndex >= 5);
    const program = args[programIndex];
    const programArgs = args.slice(programIndex + 1);
    if (program === "/usr/bin/sha256sum") {
      const file = programArgs[0];
      if (file === `${taskRoot}/codex-home/config.toml`) return { stdout: `${configDigest}  ${file}\n` };
      if (file === `${repository}/.git/HEAD`) return { stdout: `${sha256(headBytes)}  ${file}\n` };
      if (file === "/etc/os-release") return { stdout: `${sha256(osRelease)}  ${file}\n` };
      return { stdout: `${fileDigests[file]}  ${file}\n` };
    }
    if (program === "/usr/bin/cat" && programArgs[0] === "/etc/os-release") return { stdout: osRelease };
    if (program === "/usr/bin/cat" && programArgs[0] === "/proc/sys/kernel/random/boot_id") return { stdout: bootId };
    if (program === "/usr/bin/uname") return { stdout: `${descriptor.distribution_identity.kernel_release}\n` };
    if (program === "/usr/bin/realpath") return { stdout: `${isolationRoot}\n` };
    if (program === "/usr/bin/stat") return { stdout: "1048576:12345:1000:41c0:directory\n" };
    if (program === paths.lsattr) {
      return { stdout: `----------------------i------- ${taskRoot}\n`,
        stderr: `${descriptor.process_fence.immutable_probe_lsattr.version}\n` };
    }
    if (program === paths.sudo && programArgs[0] === "-V") {
      return { stdout: `${descriptor.process_fence.noninteractive_root_probe.version}\n` };
    }
    if ([paths.node, paths.bootstrapNode].includes(program) && programArgs[0] === "-e") {
      const source = programArgs[1];
      if (source.includes("lattice.wsl2-immutable-observation-source/1.0")) {
        return { stdout: `${JSON.stringify(immutableFacts)}\n`, stderr: "" };
      }
      if (source.includes("config_regular_file")) return { stdout: `${JSON.stringify(credentialFacts)}\n` };
      if (source.includes("observations")) return { stdout: `${JSON.stringify(isolationFacts)}\n` };
      if (source.includes("lattice.wsl2-keyring-library-manifest/1.0")) return { stdout: `${JSON.stringify({
        schema: "lattice.wsl2-keyring-library-manifest/1.0",
        digest: descriptor.linux.keyring_library_manifest_digest,
      })}\n` };
      if (source.includes("SYSTEMCTL_SHOW_FAILED")) return { stdout: `${JSON.stringify({
        unit: cgroupUnit, active_state: "inactive", sub_state: "dead", result: "success",
        cgroup_path: cgroupFacts.path, delegate: "no", cgroup_exists: true, populated: 0,
      })}\n` };
    }
    if (program === paths.systemdRun && programArgs.some((arg) => arg === `--unit=${cgroupUnit}`)) {
      const dbusIndex = programArgs.indexOf(paths.dbus);
      assert.deepEqual(programArgs.slice(dbusIndex, dbusIndex + 4), [
        paths.dbus, "--", paths.bootstrapNode, "-e",
      ]);
      assert.equal(programArgs[dbusIndex + 4], WSL2_SUPERVISOR_BOOTSTRAP_SOURCE);
      assert.deepEqual(programArgs.slice(dbusIndex + 5, dbusIndex + 7), [
        paths.supervisor, descriptor.linux.supervisor_sha256,
      ]);
      const executionRef = programArgs[programArgs.indexOf("--execution-environment-ref") + 1];
      const seal = programArgs[programArgs.indexOf("--credential-seal-digest") + 1];
      const marker = {
        schema: "lattice.wsl2-process-fence/1.1", fence, unit: cgroupUnit,
        execution_environment_ref: executionRef, credential_seal_digest: seal,
        cgroup_path: cgroupFacts.path, cgroup_version: 2, delegated: false,
      };
      const toolIdentities = {
        controller: { ...descriptor.process_fence.supervisor_bootstrap_node, owner_uid: 0 },
        node: { path: descriptor.linux.node_path, version: descriptor.linux.node_version,
          sha256: descriptor.linux.node_sha256, owner_uid: 0 },
        npm: { ...descriptor.verification_toolchain.npm, owner_uid: 0 },
        cargo: { ...descriptor.verification_toolchain.cargo, owner_uid: 0 },
        rustc: { ...descriptor.verification_toolchain.rustc, owner_uid: 0 },
        rustdoc: { ...descriptor.verification_toolchain.rustdoc, owner_uid: 0 },
        git: { path: descriptor.linux.git_path, version: descriptor.linux.git_version,
          sha256: descriptor.linux.git_sha256, owner_uid: 0 },
        setsid: { path: descriptor.linux.setsid_path, version: null,
          sha256: descriptor.linux.setsid_sha256, owner_uid: 0 },
      };
      const toolInputIdentities = Object.fromEntries(Object.entries(toolIdentities).map(
        ([name, identity], index) => [name, {
          ...identity, sandbox_owner_uid: 65534, sandbox_owner_gid: 65534,
          mode: "100500", device: "2049", inode: String(13000 + index), size: 4096,
        }],
      ));
      const commandLabels = [
        "git-top", "git-dir", "git-common", "git-head", "git-status", "node-version",
        "npm-version", "npm-package", "cargo-version", "cargo-metadata", "rustc-version", "rustdoc-version",
      ];
      const technical = {
        schema: "lattice.wsl2-technical-probe/1.1", status: "PASS", cwd: repository,
        sandbox_namespace: {
          schema: "lattice.wsl2-user-namespace/1.0",
          process_uid: 1000, process_gid: 1000,
          uid_map_sha256: "a".repeat(64), gid_map_sha256: "b".repeat(64),
          root_owner_sandbox_uid: 65534, root_owner_sandbox_gid: 65534,
        },
        write_probe_sha256: sha256(Buffer.from(`${fence}\n`, "utf8")),
        git: {
          top_level: repository,
          git_dir: `${commonGit}/worktrees/work-${taskRef}`,
          common_dir: commonGit,
          head: descriptor.linux.repository_head,
          status: "",
        },
        node: { path: paths.node, version: descriptor.linux.node_version },
        npm: { version: descriptor.verification_toolchain.npm.version, package_name_json: '"lattice-devos"' },
        cargo: { version_verbose: `${descriptor.verification_toolchain.cargo.version}\nrelease: 1.97.1`, metadata_sha256: "c".repeat(64) },
        rustc: { version_verbose: `${descriptor.verification_toolchain.rustc.version}\nrelease: 1.97.1` },
        rustdoc: { version: descriptor.verification_toolchain.rustdoc.version },
        daemon_escape_probe: { attempted: true, mechanism: "setsid-fork" },
        tool_input_identities: toolInputIdentities,
        command_evidence: commandLabels.map((label, index) => ({
          sequence: index + 1, label, stdout_bytes: 16, stderr_bytes: 0,
          stdout_sha256: sha256(Buffer.from(label, "utf8")), stderr_sha256: sha256(Buffer.alloc(0)),
          exit_code: 0, signal: null, timed_out: false, output_bound_exceeded: false,
        })),
        effect_counters: { account_read: 0, thread_start: 0, turn_start: 0, provider_effect_count: 0 },
      };
      const supervisorReceipt = completeSupervisorExit(descriptor, "PREFLIGHT", {
        fence,
        unit: cgroupUnit,
        credentialSeal: seal,
        environmentRef: executionRef,
        cgroupPath: cgroupFacts.path,
        stdoutBytes: Buffer.byteLength(`${JSON.stringify(technical)}\n`, "utf8"),
        timeoutMs: 150_000,
        attempt,
        retryOf,
        reconnectOf,
      });
      return { stdout: `${JSON.stringify(technical)}\n`, stderr: `${JSON.stringify(marker)}\n${JSON.stringify(supervisorReceipt)}\n` };
    }
    if (program === paths.launcher) return { stdout: `${descriptor.linux.launcher_version}\n` };
    if (program === paths.node) return { stdout: `${descriptor.linux.node_version}\n` };
    if (program === paths.bootstrapNode) {
      return { stdout: `${descriptor.process_fence.supervisor_bootstrap_node.version}\n` };
    }
    if (program === paths.git && programArgs[0] === "--version") return { stdout: `${descriptor.linux.git_version}\n` };
    if (program === paths.systemdRun) return { stdout: `${descriptor.process_fence.systemd_run_version}\n` };
    if (program === paths.systemctl && programArgs[0] === "--version") {
      return { stdout: `${descriptor.process_fence.systemctl_version}\n` };
    }
    if (program === paths.systemctl && programArgs[0] === "--user") return { stdout: "" };
    if (program === paths.npm) return { stdout: "11.12.1\n" };
    if (program === paths.cargo) {
      if (programArgs[0] === "-Vv") {
        return { stdout: "cargo 1.97.1 (c980f4866 2026-03-10)\nrelease: 1.97.1\ncommit-hash: c980f4866\nhost: x86_64-unknown-linux-gnu\n" };
      }
      return { stdout: `${descriptor.verification_toolchain.cargo.version}\n` };
    }
    if (program === paths.rustc) {
      if (programArgs[0] === "-Vv") {
        return { stdout: "rustc 1.97.1 (8bab26f4f 2026-03-10)\nbinary: rustc\ncommit-hash: 8bab26f4f\nhost: x86_64-unknown-linux-gnu\nrelease: 1.97.1\n" };
      }
      return { stdout: `${descriptor.verification_toolchain.rustc.version}\n` };
    }
    if (program === paths.rustdoc) return { stdout: `${descriptor.verification_toolchain.rustdoc.version}\n` };
    if (program === paths.sandboxHelper) return { stdout: `${descriptor.verification_toolchain.sandbox_helper.version}\n` };
    if (program === paths.git) {
      const gitCommand = programArgs.at(-1);
      const output = new Map([
        ["--show-toplevel", repository], ["--git-common-dir", commonGit],
        ["--absolute-git-dir", `${commonGit}/worktrees/work-${taskRef}`],
        ["HEAD", descriptor.linux.repository_head], ["HEAD^{commit}", descriptor.linux.repository_head],
        ["--porcelain=v1", ""],
      ]).get(gitCommand);
      return { stdout: `${output ?? ""}\n` };
    }
    throw new Error(`unexpected Linux command: ${program} ${programArgs.join(" ")}`);
  };
  const readFile = async (file) => file === descriptor.gateway.windows_path ? gatewayBytes : headBytes;
  return {
    descriptor, calls, fence, taskRef, taskRoot, isolationRoot, credentialFacts, cgroupFacts,
    context: {
      processFence: fence, taskRef, attempt, worktreeRef: typed("worktree", "9"),
      retryOf, reconnectOf,
    },
    dependencies: { execFile, readFile, observeGatewayVersion: async () => "2.6.1" },
  };
}

async function acceptedFixture(contextOverrides = {}) {
  const value = fixture(contextOverrides);
  const accepted = await preflightWsl2ExecutionEnvironment(
    value.descriptor,
    value.context,
    value.dependencies,
  );
  return { ...value, ...accepted };
}

function withPreflightContinuation(receipt, {
  attempt = 2, retryOf = null, reconnectOf = null,
} = {}) {
  const continued = structuredClone(receipt);
  continued.attempt = attempt;
  continued.continuation = {
    attempt,
    retry_of: retryOf,
    reconnect_of: reconnectOf,
  };
  delete continued.receipt_digest;
  continued.receipt_digest = `wsl2-preflight:sha256:${sha256(Buffer.from(
    canonicalJson(continued), "utf8",
  ))}`;
  return continued;
}

function resealEnvironmentIdentities(descriptor) {
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

test("production preflight binds exact OS, credential seal, cgroup v2, toolchain, repository, and zero model effects", async () => {
  const { environment, receipt, descriptor, calls, fence, isolationRoot } = await acceptedFixture();
  assert.deepEqual(Object.keys(receipt).sort(), [
    "schema", "status", "task_ref", "attempt", "worktree_ref", "execution_environment_ref",
    "descriptor_digest", "distribution_identity_ref", "linux_cwd", "repository_head",
    "repository_identity", "codex_home_digest", "credential_authority_ref",
    "credential_seal_digest", "verification_toolchain_ref", "immutable_snapshot_ref",
    "sandbox_policy_ref", "privilege_boundary_ref", "process_fence", "isolation",
    "probes", "effect_counters", "provider_effect_count", "bounds", "timeout", "continuation",
    "connector_auth_ready", "receipt_digest",
  ].sort());
  assert.equal(environment.identity_digest, executionEnvironmentIdentity(environment));
  assert.equal(receipt.schema, "lattice.wsl2-zero-model-preflight/1.0");
  assert.equal(receipt.status, "PASS");
  assert.equal(receipt.provider_effect_count, 0);
  assert.equal(receipt.task_ref, descriptor.verification_toolchain.task_ref);
  assert.equal(receipt.attempt, 1);
  assert.match(receipt.worktree_ref, /^worktree:sha256:[a-f0-9]{64}$/u);
  assert.equal(receipt.descriptor_digest, environment.identity_digest);
  assert.equal(receipt.codex_home_digest, codexHomeIdentity(environment));
  assert.equal(receipt.process_fence.fence, fence);
  assert.equal(receipt.linux_cwd, descriptor.linux.cwd);
  assert.equal(receipt.repository_head, descriptor.linux.repository_head);
  assert.equal(receipt.credential_authority_ref, descriptor.credential_authority.authority_digest);
  assert.match(receipt.credential_seal_digest, /^credential-seal:sha256:[a-f0-9]{64}$/u);
  assert.equal(receipt.verification_toolchain_ref, descriptor.verification_toolchain.identity_digest);
  assert.equal(receipt.immutable_snapshot_ref, descriptor.immutable_snapshot.snapshot_digest);
  assert.equal(receipt.sandbox_policy_ref, descriptor.sandbox_policy.policy_digest);
  assert.equal(receipt.privilege_boundary_ref, descriptor.privilege_boundary.boundary_digest);
  assert.equal(receipt.process_fence.authority_ref, descriptor.process_fence.identity_digest);
  assert.equal(receipt.process_fence.cgroup_path.endsWith(`/${receipt.process_fence.service_unit}`), true);
  assert.equal(receipt.process_fence.delegated, false);
  assert.equal(receipt.process_fence.outer_post_exit.delegate, "no");
  assert.equal(receipt.process_fence.outer_post_exit.populated, 0);
  assert.equal(receipt.isolation.root, isolationRoot);
  assert.equal(receipt.isolation.owner_uid, 1000);
  assert.match(receipt.process_fence.boot_id_digest, /^wsl-boot:sha256:[a-f0-9]{64}$/u);
  assert.match(receipt.receipt_digest, /^wsl2-preflight:sha256:[a-f0-9]{64}$/u);
  assert.equal(receipt.probes.technical.daemon_escape_probe.attempted, true);
  assert.equal(receipt.probes.technical.effect_counters.provider_effect_count, 0);
  const technicalCall = calls.find(([, args]) => args.includes("--sandbox-state-json"));
  assert.ok(technicalCall);
  const stateIndex = technicalCall[1].indexOf("--sandbox-state-json");
  const sandboxState = JSON.parse(technicalCall[1][stateIndex + 1]);
  assert.equal(sandboxState.permissionProfile.type, "managed");
  assert.equal(sandboxState.permissionProfile.network, "restricted");
  assert.equal(sandboxState.codexLinuxSandboxExe, null);
  assert.equal(sandboxState.sandboxCwd, `file://${environment.linux.cwd}`);
  assert.equal(sandboxState.permissionProfile.file_system.entries.some((entry) => (
    entry.path.path === "/" && entry.access === "read"
  )), false);
  assert.equal(sandboxState.permissionProfile.file_system.entries.some((entry) => (
    entry.path.path === descriptor.verification_toolchain.task_root && entry.access === "read"
  )), true);
  assert.equal(sandboxState.permissionProfile.file_system.entries.some((entry) => (
    entry.path.path === descriptor.linux.codex_home && entry.access === "deny"
  )), true);
  assert.equal(receipt.bounds.stdout_observed_bytes <= receipt.bounds.stdout_limit_bytes, true);
  assert.equal(receipt.timeout.timed_out, false);
  assert.equal(technicalCall[1].some((entry) => entry.startsWith("CODEX_HOME=")), false);
  assert.equal(technicalCall[1].some((entry) => entry.startsWith("--setenv=CODEX_HOME=")), false);
  const closedGitCalls = calls.filter(([, args]) => args.includes("core.fsmonitor=false"));
  assert.equal(closedGitCalls.length, 5);
  for (const [, args] of closedGitCalls) {
    assert.equal(args.includes("GIT_CONFIG_NOSYSTEM=1"), true);
    assert.equal(args.includes("GIT_CONFIG_COUNT=0"), true);
    assert.equal(args.includes("GIT_OPTIONAL_LOCKS=0"), true);
    assert.equal(args.includes(`GIT_CONFIG_GLOBAL=${descriptor.verification_toolchain.home_dir}/.gitconfig`), true);
    assert.equal(args.includes(`core.hooksPath=${descriptor.verification_toolchain.temp_dir}/git-hooks`), true);
    assert.equal(args.includes("protocol.allow=never"), true);
  }
  assert.equal(calls.every(([, args]) => args[3] === "/usr/bin/env" && args[4] === "-i"), true);
  assert.equal(calls.flat(2).includes("bash"), false);
  assert.equal(calls.flat(2).includes("sh"), false);
});

test("preflight accepts none or one continuation while ambiguous lineage stops before spawn", async () => {
  const reconnectOf = typed("attempt-receipt", "6");
  const accepted = await acceptedFixture({ attempt: 1, reconnectOf });
  assert.deepEqual(accepted.receipt.continuation, {
    attempt: 1,
    retry_of: null,
    reconnect_of: reconnectOf,
  });
  assert.equal(accepted.receipt.provider_effect_count, 0);

  const retryOf = typed("attempt-receipt", "7");
  const retried = await acceptedFixture({ attempt: 1, retryOf });
  assert.deepEqual(retried.receipt.continuation, {
    attempt: 1,
    retry_of: retryOf,
    reconnect_of: null,
  });
  assert.equal(retried.receipt.provider_effect_count, 0);

  const laterAttempt = await acceptedFixture({ attempt: 2 });
  assert.deepEqual(laterAttempt.receipt.continuation, {
    attempt: 2,
    retry_of: null,
    reconnect_of: null,
  });
  assert.equal(laterAttempt.receipt.provider_effect_count, 0);

  const finalRepairAttempt = await acceptedFixture({ attempt: 3 });
  assert.equal(finalRepairAttempt.receipt.continuation.attempt, 3);
  assert.equal(finalRepairAttempt.receipt.provider_effect_count, 0);

  const overBudget = fixture({ attempt: 4 });
  await assert.rejects(() => preflightWsl2ExecutionEnvironment(
    overBudget.descriptor, overBudget.context, overBudget.dependencies,
  ), { code: "WSL2_PREFLIGHT_CONTEXT_REJECTED" });
  assert.equal(overBudget.calls.length, 0);

  for (const contextOverrides of [
    {
      attempt: 2,
      retryOf: typed("attempt-receipt", "8"),
      reconnectOf: typed("attempt-receipt", "9"),
    },
  ]) {
    const value = fixture(contextOverrides);
    await assert.rejects(() => preflightWsl2ExecutionEnvironment(
      value.descriptor, value.context, value.dependencies,
    ), { code: "WSL2_PREFLIGHT_CONTEXT_REJECTED" });
    assert.equal(value.calls.length, 0);
  }
});

test("production descriptor rejects exact-key, credential, cgroup, and toolchain substitutions", () => {
  const { descriptor } = fixture();
  assert.doesNotThrow(() => validateWsl2ExecutionEnvironment(descriptor));
  for (const changed of [
    { ...descriptor, extra: true },
    { ...descriptor, distribution_identity: { ...descriptor.distribution_identity, extra: true } },
    { ...descriptor, credential_authority: { ...descriptor.credential_authority, kind: "WINDOWS_KEYRING" } },
    { ...descriptor, linux: { ...descriptor.linux, keyring_library_manifest_digest: typed("keyring-library-manifest", "0") } },
    { ...descriptor, process_fence: { ...descriptor.process_fence, kind: "PROCESS_GROUP" } },
    { ...descriptor, process_fence: {
      ...descriptor.process_fence,
      supervisor_bootstrap_node: { ...descriptor.process_fence.supervisor_bootstrap_node,
        path: `${descriptor.verification_toolchain.task_root}/toolchain-node/bin/node` },
    } },
    { ...descriptor, linux: { ...descriptor.linux, node_version: "v24.14.9" } },
    { ...descriptor, verification_toolchain: { ...descriptor.verification_toolchain, task_root: "/mnt/c/task" } },
    { ...descriptor, verification_toolchain: { ...descriptor.verification_toolchain, npm: { ...descriptor.verification_toolchain.npm, sha256: "0".repeat(64) } } },
  ]) {
    assert.throws(() => validateWsl2ExecutionEnvironment(changed), {
      code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED",
    });
  }
  for (const codexHome of [
    descriptor.linux.cwd,
    `${descriptor.linux.cwd}/.codex`,
    `${descriptor.verification_toolchain.task_root}/managed-worktrees`,
  ]) {
    const changed = structuredClone(descriptor);
    changed.linux.codex_home = codexHome;
    changed.credential_authority.authority_digest = credentialAuthorityIdentity(changed);
    changed.sandbox_policy.policy_digest = sandboxPolicyIdentity(changed);
    changed.identity_digest = executionEnvironmentIdentity(changed);
    assert.throws(() => validateWsl2ExecutionEnvironment(changed), {
      code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED",
    }, `overlapping CODEX_HOME accepted: ${codexHome}`);
  }
});

test("production descriptor rejects credential-shaped payloads in every tool version after exact resealing", () => {
  const { descriptor } = fixture();
  const mutations = [
    ["gateway", (value, suffix) => { value.gateway.version += suffix; }],
    ["launcher", (value, suffix) => {
      value.linux.launcher_version += suffix;
      value.verification_toolchain.sandbox.version = value.linux.launcher_version;
    }],
    ["node", (value, suffix) => { value.linux.node_version += suffix; }],
    ["git", (value, suffix) => { value.linux.git_version += suffix; }],
    ["systemd-run", (value, suffix) => { value.process_fence.systemd_run_version += suffix; }],
    ["systemctl", (value, suffix) => { value.process_fence.systemctl_version += suffix; }],
    ["bootstrap-node", (value, suffix) => {
      value.process_fence.supervisor_bootstrap_node.version += suffix;
    }],
    ["lsattr", (value, suffix) => { value.process_fence.immutable_probe_lsattr.version += suffix; }],
    ["sudo", (value, suffix) => { value.process_fence.noninteractive_root_probe.version += suffix; }],
    ["npm", (value, suffix) => { value.verification_toolchain.npm.version += suffix; }],
    ["cargo", (value, suffix) => { value.verification_toolchain.cargo.version += suffix; }],
    ["rustc", (value, suffix) => { value.verification_toolchain.rustc.version += suffix; }],
    ["rustdoc", (value, suffix) => { value.verification_toolchain.rustdoc.version += suffix; }],
    ["bwrap", (value, suffix) => { value.verification_toolchain.sandbox_helper.version += suffix; }],
  ];
  for (const [label, mutate] of mutations) {
    for (const suffix of [" token=fixture", " password=fixture", " secret=fixture"]) {
      const changed = structuredClone(descriptor);
      mutate(changed, suffix);
      resealEnvironmentIdentities(changed);
      assert.equal(changed.identity_digest, executionEnvironmentIdentity(changed));
      assert.throws(() => validateWsl2ExecutionEnvironment(changed), {
        code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED",
      }, `${label} accepted ${suffix.trim()}`);
    }
  }
});

test("production descriptor rejects seven-digit semantic version components after exact resealing", () => {
  const { descriptor } = fixture();
  const mutations = [
    ["gateway", (value) => { value.gateway.version = "1234567.6.1"; }],
    ["launcher", (value) => {
      value.linux.launcher_version = "codex-cli 1234567.146.0";
      value.verification_toolchain.sandbox.version = value.linux.launcher_version;
    }],
    ["node", (value) => { value.linux.node_version = "v1234567.15.0"; }],
    ["git", (value) => { value.linux.git_version = "git version 1234567.53.0"; }],
    ["systemd", (value) => { value.process_fence.systemd_run_version = "systemd 1234567"; }],
    ["bootstrap-node", (value) => {
      value.process_fence.supervisor_bootstrap_node.version = "v1234567.22.1";
    }],
    ["npm", (value) => { value.verification_toolchain.npm.version = "1234567.12.1"; }],
    ["cargo", (value) => {
      value.verification_toolchain.cargo.version = "cargo 1234567.97.1 (c980f4866 2026-03-10)";
    }],
    ["rustc", (value) => {
      value.verification_toolchain.rustc.version = "rustc 1234567.97.1 (8bab26f4f 2026-03-10)";
    }],
    ["rustdoc", (value) => {
      value.verification_toolchain.rustdoc.version = "rustdoc 1234567.97.1 (8bab26f4f 2026-03-10)";
    }],
    ["bwrap", (value) => { value.verification_toolchain.sandbox_helper.version = "bubblewrap 1234567.11.1"; }],
    ["sudo", (value) => {
      value.process_fence.noninteractive_root_probe.version = "sudo-rs 1234567.2.13-0ubuntu1";
    }],
    ["lsattr", (value) => {
      value.process_fence.immutable_probe_lsattr.version = "lsattr 1234567.47.2 (1-Jan-2025)";
    }],
  ];
  for (const [label, mutate] of mutations) {
    const changed = structuredClone(descriptor);
    mutate(changed);
    resealEnvironmentIdentities(changed);
    assert.throws(() => validateWsl2ExecutionEnvironment(changed), {
      code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED",
    }, `${label} accepted a seven-digit component`);
  }
});

test("closed systemd and sudo version suffix bounds match the durable Rust grammar", () => {
  assert.equal(isClosedWsl2ToolVersion("SYSTEMD", "systemd 12"), true);
  assert.equal(isClosedWsl2ToolVersion("SYSTEMD", "systemd 1234"), true);
  assert.equal(isClosedWsl2ToolVersion("SYSTEMD", "systemd 1"), false);
  assert.equal(isClosedWsl2ToolVersion("SYSTEMD", "systemd 12345"), false);

  const maxSuffix = "a".repeat(64);
  const oversizedSuffix = "a".repeat(65);
  const maxPatch = "1".repeat(64);
  const oversizedPatch = "1".repeat(65);
  assert.equal(isClosedWsl2ToolVersion("SUDO", `Sudo version 1.2.3p${maxPatch}`), true);
  assert.equal(isClosedWsl2ToolVersion("SUDO", `Sudo version 1.2.3p${oversizedPatch}`), false);
  assert.equal(isClosedWsl2ToolVersion("SUDO", `sudo-rs 1.2.3-${maxSuffix}`), true);
  assert.equal(isClosedWsl2ToolVersion("SUDO", `sudo-rs 1.2.3-${oversizedSuffix}`), false);
});

test("production descriptor rejects a substituted path-mapping digest even after top-level resealing", () => {
  const { descriptor } = fixture();
  const changed = structuredClone(descriptor);
  changed.path_mapping.digest = typed("path-mapping", "0");
  assert.notEqual(changed.path_mapping.digest, pathMappingIdentity(changed));
  changed.identity_digest = executionEnvironmentIdentity(changed);

  assert.throws(() => validateWsl2ExecutionEnvironment(changed), {
    code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED",
  });
});

test("production descriptor rejects immutable tree roots below a nested task-root directory", () => {
  const { descriptor } = fixture();
  const changed = structuredClone(descriptor);
  const nestedCodexRoot = `${descriptor.verification_toolchain.task_root}/nested/codex`;
  const nestedLauncher = `${nestedCodexRoot}/bin/codex`;
  changed.immutable_snapshot.trees.codex.root = nestedCodexRoot;
  changed.linux.launcher_path = nestedLauncher;
  changed.verification_toolchain.sandbox.path = nestedLauncher;
  changed.verification_toolchain.identity_digest = verificationToolchainIdentity(changed);
  changed.immutable_snapshot.snapshot_digest = immutableSnapshotIdentity(changed);
  changed.identity_digest = executionEnvironmentIdentity(changed);

  assert.throws(() => validateWsl2ExecutionEnvironment(changed), {
    code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED",
  });
});

test("production descriptor rejects launcher and keyring exact-path substitutions after resealing", () => {
  const { descriptor } = fixture();
  const substitutions = [
    (changed) => {
      const launcher = `${changed.immutable_snapshot.trees.codex.root}/opt/codex`;
      changed.linux.launcher_path = launcher;
      changed.verification_toolchain.sandbox.path = launcher;
      changed.verification_toolchain.identity_digest = verificationToolchainIdentity(changed);
    },
    (changed) => {
      changed.linux.keyring_daemon_path =
        `${changed.immutable_snapshot.trees.keyring.root}/root/usr/local/bin/gnome-keyring-daemon`;
      changed.credential_authority.authority_digest = credentialAuthorityIdentity(changed);
    },
    (changed) => {
      changed.linux.keyring_library_path =
        `${changed.immutable_snapshot.trees.keyring.root}/root/usr/lib`;
      changed.credential_authority.authority_digest = credentialAuthorityIdentity(changed);
    },
  ];

  for (const substitute of substitutions) {
    const changed = structuredClone(descriptor);
    substitute(changed);
    changed.identity_digest = executionEnvironmentIdentity(changed);
    assert.throws(() => validateWsl2ExecutionEnvironment(changed), {
      code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED",
    });
  }
});

test("task worktree binding is pure, native-Linux, identity-bound, and never sends UNC as Linux cwd", () => {
  const { descriptor, taskRoot, taskRef } = fixture();
  const linux = `${taskRoot}/managed-worktrees/work-${taskRef}-retry`;
  const windows = `\\\\wsl.localhost\\Ubuntu${linux.replaceAll("/", "\\")}`;
  const bound = bindWsl2ExecutionWorktree(descriptor, windows, {
    repository_identity: typed("repository", "d"),
    head: "fedcba9876543210fedcba9876543210fedcba98",
  });
  assert.equal(bound.linux.cwd, linux);
  assert.equal(bound.linux.repository_head, "fedcba9876543210fedcba9876543210fedcba98");
  assert.equal(bound.path_mapping.windows_path, windows);
  assert.equal(bound.path_mapping.linux_path, linux);
  assert.equal(bound.identity_digest, executionEnvironmentIdentity(bound));
  assert.throws(() => bindWsl2ExecutionWorktree(descriptor, String.raw`C:\repo`, {
    repository_identity: typed("repository", "d"), head: "f".repeat(40),
  }), { code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED" });
});

test("preflight fails closed on gateway, credential-seal, cgroup, tool, and repository substitutions", async () => {
  const gateway = fixture();
  gateway.descriptor.gateway.sha256 = "0".repeat(64);
  gateway.descriptor.identity_digest = executionEnvironmentIdentity(gateway.descriptor);
  await assert.rejects(
    preflightWsl2ExecutionEnvironment(gateway.descriptor, gateway.context, gateway.dependencies),
    { code: "WSL2_PREFLIGHT_GATEWAY_DIGEST_MISMATCH" },
  );
  assert.equal(gateway.calls.length, 0);

  for (const [label, mutate, code] of [
    ["credential", (value) => {
      const base = value.dependencies.execFile;
      value.dependencies.execFile = async (exe, args) => args.includes("-e")
        && args[args.indexOf("-e") + 1]?.includes("config_regular_file")
        ? { stdout: `${JSON.stringify({ ...value.credentialFacts, auth_json_absent: false })}\n` }
        : base(exe, args);
    }, "WSL2_PREFLIGHT_CREDENTIAL_SEAL_REJECTED"],
    ["shell environment policy", (value) => {
      const base = value.dependencies.execFile;
      value.dependencies.execFile = async (exe, args) => args.includes("-e")
        && args[args.indexOf("-e") + 1]?.includes("config_regular_file")
        ? { stdout: `${JSON.stringify({
          ...value.credentialFacts,
          shell_environment_policy: {
            ...value.credentialFacts.shell_environment_policy,
            required_keys_present: false,
          },
        })}\n` }
        : base(exe, args);
    }, "WSL2_PREFLIGHT_CREDENTIAL_SEAL_REJECTED"],
    ["cgroup", (value) => {
      const base = value.dependencies.execFile;
      value.dependencies.execFile = async (exe, args) => {
        const result = await base(exe, args);
        if (!args.includes(value.cgroupFacts.path.split("/").at(-1)) || !result.stderr) return result;
        const [marker, ...rest] = result.stderr.trimEnd().split("\n").map(JSON.parse);
        return { ...result, stderr: `${JSON.stringify({ ...marker, delegated: true })}\n${rest.map(JSON.stringify).join("\n")}\n` };
      };
    }, "WSL2_PREFLIGHT_CGROUP_V2_FENCE_REJECTED"],
    ["cargo", (value) => {
      const base = value.dependencies.execFile;
      value.dependencies.execFile = async (exe, args) => args.includes(value.descriptor.verification_toolchain.cargo.path)
        && args.includes("/usr/bin/sha256sum")
        ? { stdout: `${"0".repeat(64)}  ${value.descriptor.verification_toolchain.cargo.path}\n` }
        : base(exe, args);
    }, "WSL2_PREFLIGHT_CARGO_DIGEST_MISMATCH"],
    ["world-writable-isolation", (value) => {
      const base = value.dependencies.execFile;
      value.dependencies.execFile = async (exe, args) => {
        const result = await base(exe, args);
        if (!args.includes("-e") || !args[args.indexOf("-e") + 1]?.includes("observations")) return result;
        const observations = JSON.parse(result.stdout);
        observations.observations[value.isolationRoot].mode = "40777";
        return { ...result, stdout: `${JSON.stringify(observations)}\n` };
      };
    }, "WSL2_PREFLIGHT_ISOLATION_REJECTED"],
    ["head", (value) => {
      const base = value.dependencies.execFile;
      value.dependencies.execFile = async (exe, args) => args.includes(value.descriptor.linux.git_path)
        && args.at(-1) === "HEAD^{commit}"
        ? { stdout: `${"f".repeat(40)}\n` }
        : base(exe, args);
    }, "WSL2_PREFLIGHT_REPOSITORY_HEAD_MISMATCH"],
  ]) {
    const value = fixture();
    mutate(value);
    await assert.rejects(
      preflightWsl2ExecutionEnvironment(value.descriptor, value.context, value.dependencies),
      { code },
      label,
    );
  }
});

test("preflight derives effect counts and rejects daemon/cgroup/systemctl substitutions before provider effects", async () => {
  for (const [label, mutate, code] of [
    ["effect", (value) => {
      const base = value.dependencies.execFile;
      value.dependencies.execFile = async (exe, args) => {
        const result = await base(exe, args);
        if (!args.includes(value.cgroupFacts.path.split("/").at(-1)) || !result.stdout) return result;
        const technical = JSON.parse(result.stdout);
        technical.effect_counters.turn_start = 1;
        technical.effect_counters.provider_effect_count = 1;
        return { ...result, stdout: `${JSON.stringify(technical)}\n` };
      };
    }, "WSL2_PREFLIGHT_PROVIDER_EFFECT_DETECTED"],
    ["outer-populated", (value) => {
      const base = value.dependencies.execFile;
      value.dependencies.execFile = async (exe, args) => {
        const result = await base(exe, args);
        if (!args.some((arg) => typeof arg === "string" && arg.includes("SYSTEMCTL_SHOW_FAILED"))) return result;
        return { ...result, stdout: `${JSON.stringify({ ...JSON.parse(result.stdout), populated: 1 })}\n` };
      };
    }, "WSL2_PREFLIGHT_CGROUP_V2_FENCE_REJECTED"],
    ["systemctl", (value) => {
      const base = value.dependencies.execFile;
      value.dependencies.execFile = async (exe, args) => args.includes("/usr/bin/sha256sum")
        && args.includes(value.descriptor.process_fence.systemctl_path)
        ? { stdout: `${"0".repeat(64)}  ${value.descriptor.process_fence.systemctl_path}\n` }
        : base(exe, args);
    }, "WSL2_PREFLIGHT_SYSTEMCTL_DIGEST_MISMATCH"],
  ]) {
    const value = fixture();
    mutate(value);
    await assert.rejects(
      preflightWsl2ExecutionEnvironment(value.descriptor, value.context, value.dependencies),
      { code },
      label,
    );
  }
});

test("connector account/read completion counts actual effects and rejects thread/turn/provider substitutions", async () => {
  const { environment, receipt } = await acceptedFixture();
  const observation = {
    schema: "lattice.wsl2-connector-auth-observation/1.0",
    execution_environment_ref: environment.identity_digest,
    credential_authority_ref: environment.credential_authority.authority_digest,
    credential_seal_digest: receipt.credential_seal_digest,
    process_fence: receipt.process_fence.fence,
    linux_cwd: environment.linux.cwd,
    process_identity_digest: typed("wsl2-process", "7"),
    account_response_digest: typed("codex-account-read", "8"),
    account_read_count: 1,
    refresh_token: false,
    auth_mode: "CHATGPT",
    auth_ready: true,
    thread_start_count: 0,
    turn_start_count: 0,
    provider_effect_count: 0,
    stdout_bytes: 128,
    stderr_bytes: 0,
    timeout_ms: 10_000,
  };
  const completed = completeWsl2ConnectorPreflight(environment, receipt, observation);
  assert.equal(completed.schema, "lattice.wsl2-production-preflight/1.0");
  assert.equal(completed.effect_counters.account_read, 1);
  assert.equal(completed.effect_counters.thread_start, 0);
  assert.equal(completed.effect_counters.turn_start, 0);
  assert.equal(completed.provider_effect_count, 0);
  assert.match(completed.receipt_digest, /^wsl2-production-preflight:sha256:[a-f0-9]{64}$/u);

  for (const changed of [
    { ...observation, refresh_token: true },
    { ...observation, account_read_count: 0 },
    { ...observation, auth_ready: false },
    { ...observation, process_fence: "e".repeat(64) },
    { ...observation, linux_cwd: "/mnt/c/repo" },
    { ...observation, thread_start_count: 1, provider_effect_count: 1 },
    { ...observation, turn_start_count: 1, provider_effect_count: 1 },
    { ...observation, extra: true },
  ]) {
    assert.throws(() => completeWsl2ConnectorPreflight(environment, receipt, changed), {
      code: "WSL2_CONNECTOR_PREFLIGHT_REJECTED",
    });
  }
});

test("production Codex launch cannot bypass a matching zero-model preflight receipt", async () => {
  const value = fixture();
  assert.throws(() => buildWsl2CodexLaunch(value.descriptor, { fence: value.fence }), {
    code: "WSL2_PRODUCTION_PREFLIGHT_REQUIRED",
  });
  const { environment, receipt } = await acceptedFixture();
  assert.throws(() => buildWsl2CodexLaunch(environment, {
    fence: value.fence,
    preflightReceipt: { ...receipt, provider_effect_count: 1 },
  }), { code: "WSL2_PRODUCTION_PREFLIGHT_REQUIRED" });
  const launch = buildWsl2CodexLaunch(environment, {
    fence: value.fence, preflightReceipt: receipt,
  });
  assert.equal(launch.processFence, value.fence);
  assert.equal(launch.codexIdentity.execution_environment_ref, environment.identity_digest);
  assert.equal(launch.codexIdentity.provider_effects_authorized, false);
  assert.equal(launch.args.includes(`HOME=${environment.linux.codex_home}`), true);
  assert.equal(launch.args.includes(`--setenv=HOME=${environment.linux.codex_home}`), true);
  assert.equal(launch.args.includes(`HOME=${environment.verification_toolchain.home_dir}`), false);
  assert.equal(launch.args.includes(`--setenv=HOME=${environment.verification_toolchain.home_dir}`), false);
  assert.equal(
    launch.postExitProbe.user_runtime_dir,
    environment.process_fence.user_runtime_dir,
  );
  assert.equal(launch.args.includes(environment.process_fence.systemd_run_path), true);
  assert.equal(launch.args.includes("--property=TimeoutStopSec=5s"), true);
});

test("provider continuation accepts only attempt receipts before any launch effect", async () => {
  const { environment, receipt, fence } = await acceptedFixture();
  for (const field of ["retryOf", "reconnectOf"]) {
    const continuation = typed("attempt-receipt", field === "retryOf" ? "a" : "b");
    const preflightReceipt = withPreflightContinuation(receipt, { [field]: continuation });
    const launch = buildWsl2CodexLaunch(environment, {
      fence,
      preflightReceipt,
      attempt: 2,
      retryOf: field === "retryOf" ? continuation : null,
      reconnectOf: field === "reconnectOf" ? continuation : null,
    });
    assert.equal(launch.args.includes(continuation), true);
  }
  const initialReconnect = typed("attempt-receipt", "d");
  const initialReconnectReceipt = withPreflightContinuation(receipt, {
    attempt: 1, reconnectOf: initialReconnect,
  });
  const reconnected = buildWsl2CodexLaunch(environment, {
    fence,
    preflightReceipt: initialReconnectReceipt,
    attempt: 1,
    retryOf: null,
    reconnectOf: initialReconnect,
  });
  assert.equal(reconnected.args.includes(initialReconnect), true);

  let launchEffects = 0;
  for (const invalid of [
    {
      attempt: 1,
      retryOf: typed("attempt-receipt", "e"),
      reconnectOf: null,
    },
    {
      attempt: 2,
      retryOf: typed("attempt-receipt", "f"),
      reconnectOf: typed("attempt-receipt", "0"),
    },
    {
      attempt: 2,
      retryOf: null,
      reconnectOf: null,
    },
  ]) {
    const preflightReceipt = withPreflightContinuation(receipt, invalid);
    assert.throws(() => {
      const launch = buildWsl2CodexLaunch(environment, {
        fence,
        preflightReceipt,
        ...invalid,
      });
      launchEffects += 1;
      return launch;
    }, { code: "WSL2_PRODUCTION_PREFLIGHT_REQUIRED" });
  }
  for (const field of ["retryOf", "reconnectOf"]) {
    for (const kind of ["verifier-receipt", "wsl2-preflight"]) {
      const continuation = typed(kind, "c");
      const preflightReceipt = withPreflightContinuation(receipt, { [field]: continuation });
      assert.throws(() => {
        const launch = buildWsl2CodexLaunch(environment, {
          fence,
          preflightReceipt,
          attempt: 2,
          retryOf: field === "retryOf" ? continuation : null,
          reconnectOf: field === "reconnectOf" ? continuation : null,
        });
        launchEffects += 1;
        return launch;
      }, { code: "WSL2_PRODUCTION_PREFLIGHT_REQUIRED" });
    }
  }
  assert.equal(launchEffects, 0);
  assert.equal(receipt.provider_effect_count, 0);
});

test("legacy 1.0 descriptors are fixture-only and fail every production entry", () => {
  const { descriptor, fence } = fixture();
  const legacy = structuredClone(descriptor);
  legacy.schema = "lattice.execution-environment.wsl2-linux/1.0";
  delete legacy.distribution_identity;
  delete legacy.credential_authority;
  delete legacy.process_fence;
  delete legacy.verification_toolchain;
  delete legacy.linux.repository_head;
  legacy.identity_digest = executionEnvironmentIdentity(legacy);
  assert.throws(() => validateWsl2ExecutionEnvironment(legacy), {
    code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED",
  });
  assert.throws(() => buildWsl2CodexLaunch(legacy, { fence }), {
    code: "WSL2_PRODUCTION_PREFLIGHT_REQUIRED",
  });
  assert.equal(buildLegacyWsl2CodexLaunchFixture(legacy, { fence }).processFence, fence);
});

test("verifier launch reuses the same cgroup supervisor, fence, credential seal, sandbox, cwd, and closed NODE/CARGO vectors", async () => {
  for (const [role, args, executableField] of [
    ["NODE", ["run", "verify", "--offline", "--no-audit", "--no-fund"], "npm"],
    ["CARGO", ["test", "--locked", "--offline"], "cargo"],
  ]) {
    const { environment, receipt, fence } = await acceptedFixture();
    const launch = buildWsl2VerifierLaunch(environment, {
      fence, preflightFence: fence, preflightReceipt: receipt, role, cwd: environment.linux.cwd, args,
      timeoutMs: 120_000, stdoutLimitBytes: 262_144, stderrLimitBytes: 262_144,
      attempt: 1, retryOf: null, reconnectOf: null,
    });
    assert.equal(launch.command, environment.gateway.windows_path);
    assert.equal(launch.args.includes(environment.process_fence.systemd_run_path), true);
    assert.equal(launch.args.includes("--property=TimeoutStopSec=5s"), true);
    assert.equal(launch.args.includes(environment.linux.supervisor_path), true);
    const dbusIndex = launch.args.indexOf(environment.linux.dbus_run_session_path);
    assert.deepEqual(launch.args.slice(dbusIndex, dbusIndex + 4), [
      environment.linux.dbus_run_session_path,
      "--",
      environment.process_fence.supervisor_bootstrap_node.path,
      "-e",
    ]);
    assert.equal(launch.args[dbusIndex + 4], WSL2_SUPERVISOR_BOOTSTRAP_SOURCE);
    assert.deepEqual(launch.args.slice(dbusIndex + 5, dbusIndex + 7), [
      environment.linux.supervisor_path, environment.linux.supervisor_sha256,
    ]);
    assert.equal(launch.args.includes(environment.linux.codex_home), true);
    assert.equal(launch.args.includes(environment.verification_toolchain.sandbox.path), true);
    assert.equal(launch.args.includes(environment.verification_toolchain[executableField].path), true);
    assert.equal(launch.args.includes(environment.linux.cwd), true);
    assert.equal(launch.args.includes("bash"), false);
    assert.equal(launch.args.includes("sh"), false);
    assert.equal(launch.verifierIdentity.provider_effect_count, 0);
    assert.equal(launch.verifierIdentity.process_fence, fence);
    assert.equal(launch.verifierIdentity.credential_seal_digest, receipt.credential_seal_digest);
    assert.match(launch.verifierIdentity.command_digest, /^wsl2-verifier-command:sha256:[a-f0-9]{64}$/u);
  }
});

test("verifier launch rejects cwd, args, bounds, receipt, retry, and reconnect substitution", async () => {
  const { environment, receipt, fence } = await acceptedFixture();
  const base = {
    fence, preflightFence: fence, preflightReceipt: receipt, role: "NODE", cwd: environment.linux.cwd,
    args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
    timeoutMs: 120_000, stdoutLimitBytes: 262_144, stderrLimitBytes: 262_144,
    attempt: 1, retryOf: null, reconnectOf: null,
  };
  for (const changed of [
    { ...base, cwd: "/mnt/c/repository" },
    { ...base, args: ["run", "postinstall"] },
    { ...base, timeoutMs: 0 },
    { ...base, stdoutLimitBytes: 8 * 1024 * 1024 },
    { ...base, reconnectOf: typed("attempt-receipt", "a") },
    { ...base, preflightReceipt: {
      ...receipt, process_fence: { ...receipt.process_fence, fence: "e".repeat(64) },
    } },
  ]) {
    assert.throws(() => buildWsl2VerifierLaunch(environment, changed), {
      code: "WSL2_VERIFIER_LAUNCH_REJECTED",
    });
  }
});

test("verifier continuation accepts only verifier receipts before spawn", async () => {
  const { environment, receipt, fence, context } = await acceptedFixture();
  const firstVerifierReceipt = withPreflightContinuation(receipt, { attempt: 2 });
  const firstVerifierLaunch = buildWsl2VerifierLaunch(environment, {
    fence,
    preflightFence: fence,
    preflightReceipt: firstVerifierReceipt,
    role: "NODE",
    cwd: environment.linux.cwd,
    args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
    timeoutMs: 120_000,
    stdoutLimitBytes: 262_144,
    stderrLimitBytes: 262_144,
    attempt: 2,
    retryOf: null,
    reconnectOf: null,
  });
  assert.equal(firstVerifierLaunch.args.includes("--attempt"), true);
  assert.equal(validateWsl2VerifierBridgeRequest({
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: firstVerifierReceipt,
    task_ref: context.taskRef,
    attempt: 2,
    worktree_ref: context.worktreeRef,
    role: "NODE",
    args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
  }).attempt, 2);

  for (const field of ["retryOf", "reconnectOf"]) {
    const continuation = typed("verifier-receipt", field === "retryOf" ? "c" : "d");
    const preflightReceipt = withPreflightContinuation(receipt, {
      [field]: continuation,
    });
    const launch = buildWsl2VerifierLaunch(environment, {
      fence,
      preflightFence: fence,
      preflightReceipt,
      role: "NODE",
      cwd: environment.linux.cwd,
      args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
      timeoutMs: 120_000,
      stdoutLimitBytes: 262_144,
      stderrLimitBytes: 262_144,
      attempt: 2,
      retryOf: field === "retryOf" ? continuation : null,
      reconnectOf: field === "reconnectOf" ? continuation : null,
    });
    assert.equal(launch.args.includes(continuation), true);

    const request = {
      schema: "lattice.wsl2-verifier-request/1.0",
      environment,
      preflight_receipt: preflightReceipt,
      task_ref: context.taskRef,
      attempt: 2,
      worktree_ref: context.worktreeRef,
      role: "NODE",
      args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
    };
    assert.equal(validateWsl2VerifierBridgeRequest(request).attempt, 2);
  }
  const initialReconnect = typed("verifier-receipt", "f");
  const initialReconnectReceipt = withPreflightContinuation(receipt, {
    attempt: 1, reconnectOf: initialReconnect,
  });
  const initialReconnectLaunch = buildWsl2VerifierLaunch(environment, {
    fence,
    preflightFence: fence,
    preflightReceipt: initialReconnectReceipt,
    role: "NODE",
    cwd: environment.linux.cwd,
    args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
    timeoutMs: 120_000,
    stdoutLimitBytes: 262_144,
    stderrLimitBytes: 262_144,
    attempt: 1,
    retryOf: null,
    reconnectOf: initialReconnect,
  });
  assert.equal(initialReconnectLaunch.args.includes(initialReconnect), true);
  assert.equal(validateWsl2VerifierBridgeRequest({
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: initialReconnectReceipt,
    task_ref: context.taskRef,
    attempt: 1,
    worktree_ref: context.worktreeRef,
    role: "NODE",
    args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
  }).attempt, 1);

  const initialRetry = typed("verifier-receipt", "6");
  const initialRetryReceipt = withPreflightContinuation(receipt, {
    attempt: 1, retryOf: initialRetry,
  });
  const initialRetryLaunch = buildWsl2VerifierLaunch(environment, {
    fence,
    preflightFence: fence,
    preflightReceipt: initialRetryReceipt,
    role: "NODE",
    cwd: environment.linux.cwd,
    args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
    timeoutMs: 120_000,
    stdoutLimitBytes: 262_144,
    stderrLimitBytes: 262_144,
    attempt: 1,
    retryOf: initialRetry,
    reconnectOf: null,
  });
  assert.equal(initialRetryLaunch.args.includes(initialRetry), true);
  assert.equal(validateWsl2VerifierBridgeRequest({
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: initialRetryReceipt,
    task_ref: context.taskRef,
    attempt: 1,
    worktree_ref: context.worktreeRef,
    role: "NODE",
    args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
  }).attempt, 1);

  let spawnCount = 0;
  for (const invalid of [
    {
      attempt: 2,
      retryOf: typed("verifier-receipt", "8"),
      reconnectOf: typed("verifier-receipt", "9"),
    },
  ]) {
    const preflightReceipt = withPreflightContinuation(receipt, invalid);
    await assert.rejects(() => runWsl2VerifierBridge({
      schema: "lattice.wsl2-verifier-request/1.0",
      environment,
      preflight_receipt: preflightReceipt,
      task_ref: context.taskRef,
      attempt: invalid.attempt,
      worktree_ref: context.worktreeRef,
      role: "NODE",
      args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
    }, {
      spawnProcess: () => {
        spawnCount += 1;
        throw new Error("spawn must not be reached");
      },
    }), { code: "WSL2_VERIFIER_BRIDGE_REQUEST_REJECTED" });
  }
  for (const field of ["retryOf", "reconnectOf"]) {
    for (const kind of ["attempt-receipt", "wsl2-preflight"]) {
      const continuation = typed(kind, "e");
      const preflightReceipt = withPreflightContinuation(receipt, { [field]: continuation });
      await assert.rejects(() => runWsl2VerifierBridge({
        schema: "lattice.wsl2-verifier-request/1.0",
        environment,
        preflight_receipt: preflightReceipt,
        task_ref: context.taskRef,
        attempt: 2,
        worktree_ref: context.worktreeRef,
        role: "NODE",
        args: ["run", "verify", "--offline", "--no-audit", "--no-fund"],
      }, {
        spawnProcess: () => {
          spawnCount += 1;
          throw new Error("spawn must not be reached");
        },
      }), { code: "WSL2_VERIFIER_BRIDGE_REQUEST_REJECTED" });
    }
  }
  assert.equal(spawnCount, 0);
  assert.equal(receipt.provider_effect_count, 0);
});

test("verifier bridge archives the exact supervised WSL toolchain result and rejects substitutions", async () => {
  const { environment, receipt, context } = await acceptedFixture();
  const firstVerifierReceipt = withPreflightContinuation(receipt, { attempt: 2 });
  const request = {
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: firstVerifierReceipt,
    task_ref: context.taskRef,
    attempt: 2,
    worktree_ref: context.worktreeRef,
    role: "CARGO",
    args: ["test", "--locked", "--offline"],
  };
  const result = await runWsl2VerifierBridge(request, {
    runLaunch: async (launch) => {
      const marker = {
        schema: "lattice.wsl2-process-fence/1.1",
        fence: launch.processFence,
        unit: launch.serviceUnit,
        execution_environment_ref: environment.identity_digest,
        credential_seal_digest: receipt.credential_seal_digest,
        attempt: 2,
        retry_of: null,
        reconnect_of: null,
      };
      const cgroupPath = `/user.slice/user-1000.slice/user@1000.service/app.slice/${launch.serviceUnit}`;
      marker.cgroup_path = cgroupPath;
      const exit = completeSupervisorExit(environment, "CARGO", {
        fence: launch.processFence,
        unit: launch.serviceUnit,
        credentialSeal: receipt.credential_seal_digest,
        cgroupPath,
        stdoutBytes: Buffer.byteLength("test result: ok\n", "utf8"),
        stdoutLimit: receipt.bounds.stdout_limit_bytes,
        stderrLimit: receipt.bounds.stderr_limit_bytes,
        timeoutMs: receipt.timeout.timeout_ms,
        attempt: 2,
        retryOf: null,
        reconnectOf: null,
      });
      return {
        code: 0,
        signal: null,
        stdout: Buffer.from("test result: ok\n", "utf8"),
        stderr: Buffer.from(`${JSON.stringify(marker)}\n${JSON.stringify(exit)}\n`, "utf8"),
      };
    },
    runOuterProbe: async (_request, launch, marker) => ({
      unit: launch.serviceUnit,
      active_state: "inactive",
      sub_state: "dead",
      result: "success",
      cgroup_path: marker.cgroup_path,
      delegate: "no",
      cgroup_exists: false,
      populated: null,
    }),
  });
  assert.equal(result.status, "PASS");
  assert.equal(result.provider_effect_count, 0);
  assert.equal(result.exit_receipt.attempt, 2);
  assert.equal(result.verifier_identity.execution_environment_ref, environment.identity_digest);
  assert.equal(result.exit_receipt.zero_descendants, true);
  assert.equal(result.outer_post_exit.active_state, "inactive");
  assert.match(result.result_digest, /^wsl2-verifier-result:sha256:[a-f0-9]{64}$/u);

  await assert.rejects(() => runWsl2VerifierBridge({
    ...request,
    worktree_ref: typed("worktree", "8"),
  }, { runLaunch: async () => assert.fail("substitution must stop before launch") }), {
    code: "WSL2_VERIFIER_BRIDGE_REQUEST_REJECTED",
  });
});

test("verifier bridge derives exact typed failure, timeout, output-bound, and interrupt outcomes", async () => {
  const { environment, receipt, context } = await acceptedFixture();
  const request = {
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: receipt,
    task_ref: context.taskRef,
    attempt: 1,
    worktree_ref: context.worktreeRef,
    role: "CARGO",
    args: ["test", "--locked", "--offline"],
  };
  const scenarios = [
    { name: "failure", exitCode: 23, exitSignal: null, observedCode: 23,
      timedOut: false, outputBoundExceeded: false, interrupted: false, outcome: "FAILED" },
    { name: "timeout", exitCode: null, exitSignal: "SIGKILL", observedCode: 71,
      timedOut: true, outputBoundExceeded: false, interrupted: false, outcome: "TIMED_OUT" },
    { name: "output bound", exitCode: null, exitSignal: "SIGTERM", observedCode: 71,
      timedOut: false, outputBoundExceeded: true, interrupted: false,
      outcome: "OUTPUT_BOUND_EXCEEDED" },
    { name: "interrupt precedence", exitCode: null, exitSignal: "SIGTERM", observedCode: 1,
      timedOut: true, outputBoundExceeded: true, interrupted: true, outcome: "INTERRUPTED" },
  ];
  for (const scenario of scenarios) {
    const result = await runWsl2VerifierBridge(request, {
      runLaunch: async (launch) => {
        const cgroupPath = `/user.slice/user-1000.slice/user@1000.service/app.slice/${launch.serviceUnit}`;
        const marker = {
          schema: "lattice.wsl2-process-fence/1.1",
          fence: launch.processFence,
          unit: launch.serviceUnit,
          execution_environment_ref: environment.identity_digest,
          credential_seal_digest: receipt.credential_seal_digest,
          cgroup_path: cgroupPath,
          attempt: 1,
          retry_of: null,
          reconnect_of: null,
        };
        const exit = {
          ...completeSupervisorExit(environment, "CARGO", {
            fence: launch.processFence,
            unit: launch.serviceUnit,
            credentialSeal: receipt.credential_seal_digest,
            cgroupPath,
            stdoutLimit: receipt.bounds.stdout_limit_bytes,
            stderrLimit: receipt.bounds.stderr_limit_bytes,
            timeoutMs: receipt.timeout.timeout_ms,
          }),
          timed_out: scenario.timedOut,
          output_bound_exceeded: scenario.outputBoundExceeded,
          interrupted: scenario.interrupted,
          stdout_bytes: scenario.outputBoundExceeded
            ? receipt.bounds.stdout_limit_bytes + 1 : 0,
          exit_code: scenario.exitCode,
          exit_signal: scenario.exitSignal,
        };
        return {
          code: scenario.observedCode,
          signal: null,
          stdout: Buffer.alloc(0),
          stderr: Buffer.from(`${JSON.stringify(marker)}\n${JSON.stringify(exit)}\n`, "utf8"),
        };
      },
      runOuterProbe: async (_request, launch, marker) => ({
        unit: launch.serviceUnit,
        active_state: "inactive",
        sub_state: "dead",
        result: "exit-code",
        cgroup_path: marker.cgroup_path,
        delegate: "no",
        cgroup_exists: true,
        populated: 0,
      }),
    });
    assert.equal(result.status, "FAILED", scenario.name);
    assert.equal(result.outcome, scenario.outcome, scenario.name);
    assert.equal(result.exit_receipt.attempt, 1, scenario.name);
    assert.equal(result.exit_receipt.retry_of, null, scenario.name);
    assert.equal(result.exit_receipt.reconnect_of, null, scenario.name);
  }
});

test("verifier bridge reaps an outer watchdog unit with pinned systemctl before post-exit proof", async () => {
  const { environment, receipt, context } = await acceptedFixture();
  const request = {
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: receipt,
    task_ref: context.taskRef,
    attempt: 1,
    worktree_ref: context.worktreeRef,
    role: "CARGO",
    args: ["test", "--locked", "--offline"],
  };
  const events = [];
  const result = await runWsl2VerifierBridge(request, {
    runLaunch: async (launch) => {
      const cgroupPath = `/user.slice/user-1000.slice/user@1000.service/app.slice/${launch.serviceUnit}`;
      const marker = {
        schema: "lattice.wsl2-process-fence/1.1",
        fence: launch.processFence,
        unit: launch.serviceUnit,
        execution_environment_ref: environment.identity_digest,
        credential_seal_digest: receipt.credential_seal_digest,
        cgroup_path: cgroupPath,
        attempt: 1,
        retry_of: null,
        reconnect_of: null,
      };
      const exit = {
        ...completeSupervisorExit(environment, "CARGO", {
          fence: launch.processFence,
          unit: launch.serviceUnit,
          credentialSeal: receipt.credential_seal_digest,
          cgroupPath,
          stdoutLimit: receipt.bounds.stdout_limit_bytes,
          stderrLimit: receipt.bounds.stderr_limit_bytes,
          timeoutMs: receipt.timeout.timeout_ms,
        }),
        timed_out: true,
        exit_code: null,
        exit_signal: "SIGTERM",
      };
      events.push(["launch", launch.serviceUnit]);
      return {
        code: 71,
        signal: null,
        watchdog: "TIMED_OUT",
        stdout: Buffer.alloc(0),
        stderr: Buffer.from(`${JSON.stringify(marker)}\n${JSON.stringify(exit)}\n`, "utf8"),
      };
    },
    execFile: async (command, args, options) => {
      events.push(["systemctl", command, args, options]);
      if (args.includes("stop")) {
        throw Object.assign(new Error("unit not loaded"), {
          code: 5,
          signal: null,
          stdout: Buffer.alloc(0),
          stderr: Buffer.from("Unit not loaded\n", "utf8"),
        });
      }
      return { stdout: Buffer.alloc(0), stderr: Buffer.alloc(0) };
    },
    runOuterProbe: async (_request, launch, marker) => {
      events.push(["probe", launch.serviceUnit]);
      return {
        unit: launch.serviceUnit,
        active_state: "inactive",
        sub_state: "dead",
        result: "success",
        cgroup_path: marker.cgroup_path,
        delegate: "no",
        cgroup_exists: true,
        populated: 0,
      };
    },
  });
  assert.equal(result.outcome, "TIMED_OUT");
  assert.deepEqual(events.map(([kind]) => kind), [
    "launch", "systemctl", "systemctl", "systemctl", "systemctl", "probe",
  ]);
  const systemctlEvents = events.filter(([kind]) => kind === "systemctl");
  const expectedPrefix = [
    "-d", environment.distribution, "--exec", "/usr/bin/env", "-i",
    `XDG_RUNTIME_DIR=${environment.process_fence.user_runtime_dir}`,
    "LANG=C.UTF-8", "LC_ALL=C.UTF-8", environment.process_fence.systemctl_path,
  ];
  assert.equal(systemctlEvents.every(([, command]) => command === environment.gateway.windows_path), true);
  assert.deepEqual(systemctlEvents[0][2], [
    ...expectedPrefix, "--user", "kill", "--kill-whom=all", "--signal=SIGTERM",
    result.process_marker.unit,
  ]);
  assert.deepEqual(systemctlEvents[1][2], [
    ...expectedPrefix, "--user", "stop", result.process_marker.unit,
  ]);
  assert.deepEqual(systemctlEvents[2][2], [
    ...expectedPrefix, "--user", "kill", "--kill-whom=all", "--signal=SIGKILL",
    result.process_marker.unit,
  ]);
  assert.deepEqual(systemctlEvents[3][2], [
    ...expectedPrefix, "--user", "stop", result.process_marker.unit,
  ]);
  for (const [, , , options] of systemctlEvents) {
    assert.equal(options.windowsHide, true);
    assert.equal(options.timeout, 15_000);
    assert.equal(options.maxBuffer, 65_536);
  }
  assert.equal(result.outer_cleanup.reason, "TIMED_OUT");
  assert.deepEqual(result.outer_cleanup.attempts.map(({ action, result }) => [action, result]), [
    ["TERM_KILL", "SUCCESS"],
    ["STOP", "EXIT_NONZERO"],
    ["KILL", "SUCCESS"],
    ["FORCE_STOP", "EXIT_NONZERO"],
  ]);
  assert.match(result.outer_cleanup.cleanup_digest,
    /^wsl2-verifier-cleanup:sha256:[a-f0-9]{64}$/u);
});

test("verifier bridge closes stdin, spawn, and child transport failures only after cleanup and outer proof", async () => {
  const { environment, receipt, context } = await acceptedFixture();
  const request = {
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: receipt,
    task_ref: context.taskRef,
    attempt: 1,
    worktree_ref: context.worktreeRef,
    role: "CARGO",
    args: ["test", "--locked", "--offline"],
  };
  const makeChild = (source) => {
    const child = new EventEmitter();
    child.stdin = new EventEmitter();
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.stdin.end = () => {};
    child.stdin.destroy = () => {};
    child.kill = (signal) => {
      queueMicrotask(() => child.emit("close", null, signal));
      return true;
    };
    queueMicrotask(() => {
      child.emit("spawn");
      child.stdout.emit("data", Buffer.from("partial stdout\n", "utf8"));
      child.stderr.emit("data", Buffer.from("partial stderr\n", "utf8"));
      const error = Object.assign(new Error(`${source.toLowerCase()} transport failed`), {
        code: source === "STDIN" ? "EACCES" : "ENOENT",
      });
      if (source === "STDIN") child.stdin.emit("error", error);
      else child.emit("error", error);
    });
    return child;
  };
  const makeAsyncSpawnFailure = () => {
    const child = new EventEmitter();
    child.stdin = new EventEmitter();
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.stdin.end = () => {};
    child.stdin.destroy = () => {};
    child.kill = () => true;
    queueMicrotask(() => {
      child.emit("error", Object.assign(new Error("spawn ENOENT"), { code: "ENOENT" }));
      child.emit("close", -4058, null);
    });
    return child;
  };
  const scenarios = [
    {
      source: "SPAWN",
      spawnProcess: () => {
        throw Object.assign(new Error("spawn transport failed"), { code: "ENOENT" });
      },
      spawnObserved: false,
      closeObserved: false,
      output: [Buffer.alloc(0), Buffer.alloc(0)],
    },
    {
      source: "SPAWN",
      spawnProcess: makeAsyncSpawnFailure,
      spawnObserved: false,
      closeObserved: true,
      output: [Buffer.alloc(0), Buffer.alloc(0)],
    },
    {
      source: "CHILD",
      spawnProcess: () => makeChild("CHILD"),
      spawnObserved: true,
      closeObserved: true,
      output: [Buffer.from("partial stdout\n", "utf8"), Buffer.from("partial stderr\n", "utf8")],
    },
    {
      source: "STDIN",
      spawnProcess: () => makeChild("STDIN"),
      spawnObserved: true,
      closeObserved: true,
      output: [Buffer.from("partial stdout\n", "utf8"), Buffer.from("partial stderr\n", "utf8")],
    },
  ];
  for (const scenario of scenarios) {
    const events = [];
    const result = await runWsl2VerifierBridge(request, {
      spawnProcess: scenario.spawnProcess,
      execFile: async (_command, args) => {
        events.push(args.includes("kill") ? "cleanup-kill" : "cleanup-stop");
        return { stdout: Buffer.alloc(0), stderr: Buffer.alloc(0) };
      },
      runOuterProbe: async (_request, launch, marker) => {
        events.push("outer-proof");
        return {
          unit: launch.serviceUnit,
          active_state: "inactive",
          sub_state: "dead",
          result: "success",
          cgroup_path: marker.cgroup_path,
          delegate: "no",
          cgroup_exists: false,
          populated: null,
        };
      },
    });
    assert.equal(result.schema, "lattice.wsl2-verifier-transport-failure/1.0", scenario.source);
    assert.equal(result.status, "FAILED", scenario.source);
    assert.equal(result.outcome, "TRANSPORT_ERROR", scenario.source);
    assert.equal(result.retryable, true, scenario.source);
    assert.equal(result.provider_effect_count, 0, scenario.source);
    assert.equal(result.execution_environment_ref, environment.identity_digest, scenario.source);
    assert.equal(result.unit, result.outer_cleanup.unit, scenario.source);
    assert.equal(result.unit, result.outer_post_exit.unit, scenario.source);
    assert.equal(result.process_fence, result.verifier_identity.process_fence, scenario.source);
    assert.equal(result.outer_cleanup.reason, "TRANSPORT_ERROR", scenario.source);
    assert.equal(result.outer_post_exit.active_state, "inactive", scenario.source);
    assert.equal(result.outer_post_exit.populated, null, scenario.source);
    assert.deepEqual(events, ["cleanup-kill", "cleanup-stop", "outer-proof"], scenario.source);
    assert.equal(result.transport_evidence.error.source, scenario.source, scenario.source);
    assert.equal(result.transport_evidence.process.spawn_observed, scenario.spawnObserved,
      scenario.source);
    assert.equal(result.transport_evidence.process.close_observed, scenario.closeObserved,
      scenario.source);
    assert.equal(result.transport_evidence.process.exit_code, null, scenario.source);
    assert.equal(result.transport_evidence.output.stdout_sha256, sha256(scenario.output[0]),
      scenario.source);
    assert.equal(result.transport_evidence.output.stderr_sha256, sha256(scenario.output[1]),
      scenario.source);
    const errorType = {
      source: scenario.source,
      error_name: "Error",
      error_code: scenario.source === "STDIN" ? "EACCES" : "ENOENT",
    };
    assert.equal(result.transport_evidence.error.error_type_digest,
      `wsl2-verifier-transport-error:sha256:${sha256(canonicalJson(errorType))}`,
      scenario.source);
    const evidenceSubject = structuredClone(result.transport_evidence);
    delete evidenceSubject.evidence_digest;
    assert.equal(result.transport_evidence.evidence_digest,
      `wsl2-verifier-transport-evidence:sha256:${sha256(canonicalJson(evidenceSubject))}`,
      scenario.source);
    const resultSubject = structuredClone(result);
    delete resultSubject.result_digest;
    assert.equal(result.result_digest,
      `wsl2-verifier-transport-failure:sha256:${sha256(canonicalJson(resultSubject))}`,
      scenario.source);
  }
});

test("verifier bridge rejects a transport failure when outer zero-member proof is missing", async () => {
  const { environment, receipt, context } = await acceptedFixture();
  const request = {
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: receipt,
    task_ref: context.taskRef,
    attempt: 1,
    worktree_ref: context.worktreeRef,
    role: "CARGO",
    args: ["test", "--locked", "--offline"],
  };
  const events = [];
  await assert.rejects(() => runWsl2VerifierBridge(request, {
    spawnProcess: () => {
      throw Object.assign(new Error("spawn transport failed"), { code: "ENOENT" });
    },
    execFile: async () => {
      events.push("cleanup");
      return { stdout: Buffer.alloc(0), stderr: Buffer.alloc(0) };
    },
    runOuterProbe: async () => {
      events.push("outer-proof");
      throw new Error("outer probe unavailable");
    },
  }), { code: "WSL2_VERIFIER_BRIDGE_OUTER_EXIT_REJECTED" });
  assert.deepEqual(events, ["cleanup", "cleanup", "outer-proof"]);
});

test("GIT verifier control root is task-owned and binds every durable replay identity", async () => {
  const { environment, receipt, context } = await acceptedFixture();
  const facts = {
    task_ref: context.taskRef,
    attempt: 1,
    worktree_ref: context.worktreeRef,
    execution_environment_ref: environment.identity_digest,
    preflight_receipt_ref: receipt.receipt_digest,
    repository_head: environment.linux.repository_head,
    isolation_root: environment.verification_toolchain.isolation_root,
  };
  const identity = deriveWsl2GitControlRootIdentity(facts);
  assert.equal(identity.locator.startsWith(
    `${environment.verification_toolchain.isolation_root}/git-control/attempt-1-`,
  ), true);
  assert.match(identity.identity_ref, /^wsl2-git-control-root:sha256:[a-f0-9]{64}$/u);
  assert.deepEqual(deriveWsl2GitControlRootIdentity(facts), identity);

  const substitutions = [
    { task_ref: "8".repeat(64) },
    { attempt: 2 },
    { worktree_ref: `worktree:sha256:${"8".repeat(64)}` },
    { execution_environment_ref: `execution-environment:sha256:${"8".repeat(64)}` },
    { preflight_receipt_ref: `wsl2-preflight:sha256:${"8".repeat(64)}` },
    { repository_head: "8".repeat(40) },
    { isolation_root: `${environment.verification_toolchain.isolation_root}-other` },
  ];
  for (const substitution of substitutions) {
    const changed = deriveWsl2GitControlRootIdentity({ ...facts, ...substitution });
    assert.notEqual(changed.locator, identity.locator);
    assert.notEqual(changed.identity_ref, identity.identity_ref);
  }
});

test("GIT verifier bridge binds binary stdin, invocation fence, sandbox command, and output receipt", async () => {
  const { environment, receipt, context } = await acceptedFixture();
  const controlIdentity = deriveWsl2GitControlRootIdentity({
    task_ref: context.taskRef,
    attempt: 1,
    worktree_ref: context.worktreeRef,
    execution_environment_ref: environment.identity_digest,
    preflight_receipt_ref: receipt.receipt_digest,
    repository_head: environment.linux.repository_head,
    isolation_root: environment.verification_toolchain.isolation_root,
  });
  const controlRoot = controlIdentity.locator;
  const commonDirectory = receipt.probes.technical.git.common_dir;
  const gitDirectory = receipt.probes.technical.git.git_dir;
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
    "commit.gpgSign=false", "hash-object", "-w", "--stdin",
  ];
  const stdin = Buffer.from([0, 1, 2, 13, 10, 255, 128, 65]);
  const invocationSubject = {
    schema: "lattice.wsl2-git-invocation/1.0",
    sequence: 2,
    environment: gitEnvironment,
    args,
    stdin: {
      byte_len: stdin.length,
      sha256: sha256(stdin),
      base64: stdin.toString("base64"),
    },
  };
  const invocationDigest = `wsl2-git-invocation:sha256:${sha256(canonicalJson(invocationSubject))}`;
  const processFence = sha256(Buffer.from(
    `${receipt.process_fence.fence}\n${invocationDigest}\n${invocationSubject.sequence}`,
    "utf8",
  ));
  const request = {
    schema: "lattice.wsl2-verifier-request/1.0",
    environment,
    preflight_receipt: receipt,
    task_ref: context.taskRef,
    attempt: 1,
    worktree_ref: context.worktreeRef,
    role: "GIT",
    args,
    git_invocation: {
      ...invocationSubject,
      invocation_digest: invocationDigest,
      process_fence: processFence,
    },
  };
  const gitStdout = Buffer.from("0123456789abcdef0123456789abcdef01234567\n", "utf8");
  let launchIdentity = null;
  const dependencies = {
    runLaunch: async (launch, timeoutMs, observedStdin) => {
      assert.equal(timeoutMs, receipt.timeout.timeout_ms);
      assert.deepEqual(observedStdin, stdin);
      assert.equal(launch.processFence, processFence);
      assert.equal(launch.args.includes(environment.linux.git_path), true);
      assert.equal(launch.args.includes(environment.linux.codex_home), true);
      assert.equal(launch.args.includes("--sandbox-state-disable-network"), true);
      assert.equal(launch.args.includes("/usr/bin/env"), true);
      assert.equal(launch.args.includes("-i"), true);
      const sandboxStateIndex = launch.args.indexOf("--sandbox-state-json");
      const sandboxState = JSON.parse(launch.args[sandboxStateIndex + 1]);
      assert.deepEqual(sandboxState.permissionProfile.file_system.entries
        .filter((entry) => entry.access === "write").map((entry) => entry.path.path), [
        gitEnvironment.HOME, gitEnvironment.TMPDIR, gitEnvironment.GIT_OBJECT_DIRECTORY,
      ]);
      launchIdentity = launch.verifierIdentity;
      const cgroupPath = `/user.slice/user-1000.slice/user@1000.service/app.slice/${launch.serviceUnit}`;
      const marker = {
        schema: "lattice.wsl2-process-fence/1.1",
        fence: launch.processFence,
        unit: launch.serviceUnit,
        execution_environment_ref: environment.identity_digest,
        credential_seal_digest: receipt.credential_seal_digest,
        cgroup_path: cgroupPath,
        cgroup_version: 2,
        delegated: false,
        attempt: 1,
        retry_of: null,
        reconnect_of: null,
      };
      const exit = completeSupervisorExit(environment, "GIT", {
        fence: launch.processFence,
        unit: launch.serviceUnit,
        credentialSeal: receipt.credential_seal_digest,
        cgroupPath,
        stdoutBytes: gitStdout.length,
        stdoutLimit: receipt.bounds.stdout_limit_bytes,
        stderrLimit: receipt.bounds.stderr_limit_bytes,
        timeoutMs: receipt.timeout.timeout_ms,
        stdin,
      });
      return {
        code: 0,
        signal: null,
        stdout: gitStdout,
        stderr: Buffer.from(`${JSON.stringify(marker)}\n${JSON.stringify(exit)}\n`, "utf8"),
      };
    },
    runOuterProbe: async (_request, launch, marker) => ({
      unit: launch.serviceUnit,
      active_state: "inactive",
      sub_state: "dead",
      result: "success",
      cgroup_path: marker.cgroup_path,
      delegate: "no",
      cgroup_exists: true,
      populated: 0,
    }),
  };
  const result = await runWsl2VerifierBridge(request, dependencies);
  assert.equal(result.status, "PASS");
  assert.equal(result.invocation_digest, invocationDigest);
  assert.equal(result.verifier_identity, launchIdentity);
  assert.equal(result.verifier_identity.process_fence, processFence);
  assert.match(result.verifier_identity.command_digest, /^wsl2-verifier-command:sha256:[a-f0-9]{64}$/u);
  assert.equal(result.output.stdout_base64, gitStdout.toString("base64"));
  assert.equal(result.output.stdout_sha256, sha256(gitStdout));
  assert.equal(result.outer_post_exit.populated, 0);
  assert.equal(result.provider_effect_count, 0);

  const substituted = structuredClone(request);
  substituted.git_invocation.stdin.base64 = Buffer.from("substituted", "utf8").toString("base64");
  await assert.rejects(() => runWsl2VerifierBridge(substituted, {
    runLaunch: async () => assert.fail("stdin substitution must stop before launch"),
  }), { code: "WSL2_VERIFIER_BRIDGE_REQUEST_REJECTED" });

  const signInvocation = (subject) => {
    const invocationDigest = `wsl2-git-invocation:sha256:${sha256(canonicalJson(subject))}`;
    return {
      ...subject,
      invocation_digest: invocationDigest,
      process_fence: sha256(Buffer.from(
        `${receipt.process_fence.fence}\n${invocationDigest}\n${subject.sequence}`,
        "utf8",
      )),
    };
  };
  const bootstrapEnvironment = Object.fromEntries(Object.entries(gitEnvironment).filter(([key]) => (
    ["HOME", "TMPDIR", "GIT_CONFIG_GLOBAL", "NO_COLOR", "CI", "GIT_CONFIG_NOSYSTEM",
      "GIT_CONFIG_COUNT", "GIT_TERMINAL_PROMPT", "GIT_OPTIONAL_LOCKS", "GIT_ATTR_NOSYSTEM"].includes(key)
  )));
  const bootstrapArgs = [
    ...args.slice(0, 11), "rev-parse", "--show-toplevel",
  ];
  const bootstrapInvocation = signInvocation({
    schema: "lattice.wsl2-git-invocation/1.0",
    sequence: 3,
    environment: bootstrapEnvironment,
    args: bootstrapArgs,
    stdin: null,
  });
  const bootstrapRequest = {
    ...request,
    args: bootstrapArgs,
    git_invocation: bootstrapInvocation,
  };
  assert.equal(validateWsl2VerifierBridgeRequest(bootstrapRequest).role, "GIT");
  const bootstrapLaunch = buildWsl2VerifierLaunch(environment, {
    role: "GIT",
    args: bootstrapArgs,
    fence: bootstrapInvocation.process_fence,
    preflightFence: receipt.process_fence.fence,
    preflightReceipt: receipt,
    cwd: environment.linux.cwd,
    timeoutMs: receipt.timeout.timeout_ms,
    stdoutLimitBytes: receipt.bounds.stdout_limit_bytes,
    stderrLimitBytes: receipt.bounds.stderr_limit_bytes,
    attempt: 1,
    retryOf: null,
    reconnectOf: null,
    gitInvocation: bootstrapInvocation,
  });
  const sandboxStateIndex = bootstrapLaunch.args.indexOf("--sandbox-state-json");
  const bootstrapSandboxState = JSON.parse(bootstrapLaunch.args[sandboxStateIndex + 1]);
  const bootstrapWrites = bootstrapSandboxState.permissionProfile.file_system.entries
    .filter((entry) => entry.access === "write").map((entry) => entry.path.path);
  assert.deepEqual(bootstrapWrites, [bootstrapEnvironment.HOME, bootstrapEnvironment.TMPDIR]);

  for (const maliciousSubject of [
    {
      ...invocationSubject,
      sequence: 4,
      environment: { ...gitEnvironment, GIT_INDEX_FILE: "/home/zk/.ssh/authorized_keys" },
    },
    {
      ...invocationSubject,
      sequence: 5,
      args: [...args.slice(0, 11), "diff", "--no-index", "/home/zk/.ssh/id_rsa", "/dev/null"],
      stdin: null,
    },
    {
      ...invocationSubject,
      sequence: 6,
      environment: {
        ...gitEnvironment,
        GIT_DIR: `${commonDirectory}/worktrees/${path.posix.basename(gitDirectory)}2`,
      },
    },
    {
      ...invocationSubject,
      sequence: 7,
      args: [...args.slice(0, 11), "update-index", "--add", "--cacheinfo",
        `100644,${"a".repeat(40)},safe.txt,/../home/zk/.ssh/id_rsa`],
      stdin: null,
    },
  ]) {
    const maliciousInvocation = signInvocation(maliciousSubject);
    assert.throws(() => validateWsl2VerifierBridgeRequest({
      ...request,
      args: maliciousSubject.args,
      git_invocation: maliciousInvocation,
    }), { code: "WSL2_VERIFIER_BRIDGE_REQUEST_REJECTED" });
  }
});
