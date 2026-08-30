import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmod, copyFile, mkdir, mkdtemp, readFile, realpath, rename, rm, stat, unlink, writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import {
  closeExecutableIdentitySeal,
  createExactStdinTracker,
  credentialSealIdentity,
  explicitChildEnvironment,
  openExecutableIdentitySeal,
  openCredentialSeal,
  openRegularFileIdentitySeal,
  observeBoundedOutput,
  observeChildTerminal,
  parseSupervisorArgs,
  parseManagedShellEnvironmentPolicy,
  parseUnifiedCgroup,
  rewriteSealedVerifierCommandArgs,
  runSealedExecutableOutput,
  runSealedKeyringUnlock,
  runSealedNodeScriptOutput,
  selectKeyringPrivateLibraryRecords,
  startCredentialMutationWatch,
  verifyRegularFileIdentitySeal,
} from "../src/wsl2-codex-supervisor.mjs";

const typed = (kind, value) => `${kind}:sha256:${value.repeat(64)}`;
const managedConfig = `cli_auth_credentials_store = "keyring"
[shell_environment_policy]
inherit = "all"
ignore_default_excludes = false
include_only = ["HOME", "PATH", "LANG", "LC_ALL", "TERM", "COLORTERM"]
experimental_use_profile = false
`;

function argvFixture(role = "PREFLIGHT") {
  const fence = "f".repeat(64);
  const verifierRole = ["NODE", "CARGO", "GIT"].includes(role);
  const verifierPath = role === "NODE"
    ? "/home/zk/task/node/bin/npm"
    : role === "CARGO"
      ? "/home/zk/task/rust/bin/cargo"
      : role === "GIT" ? "/usr/bin/git" : "NONE";
  const verifierVersion = role === "NODE" ? "10.9.4"
    : role === "CARGO" ? "cargo 1.97.1" : role === "GIT" ? "git version 2.53.0" : "NONE";
  const values = {
    role,
    fence,
    unit: `lattice-wsl2-${"a".repeat(16)}-${role.toLowerCase()}-${fence.slice(0, 12)}.service`,
    "execution-environment-ref": typed("execution-environment", "b"),
    "credential-authority-ref": typed("wsl2-credential-authority", "c"),
    "credential-seal-digest": typed("credential-seal", "d"),
    "config-digest": typed("codex-config", "e"),
    "codex-home": "/home/zk/task/codex-home",
    cwd: "/home/zk/task/managed-worktrees/work-a",
    executable: "/home/zk/task/codex/codex",
    "executable-version": "codex-cli 0.146.0",
    "executable-sha256": "1".repeat(64),
    "verifier-tool": verifierPath,
    "verifier-tool-version": verifierVersion,
    "verifier-tool-sha256": verifierRole ? "2".repeat(64) : "NONE",
    "node-runtime": ["PREFLIGHT", "NODE"].includes(role) ? "/usr/bin/node" : "NONE",
    "node-runtime-version": ["PREFLIGHT", "NODE"].includes(role) ? "v24.15.0" : "NONE",
    "node-runtime-sha256": ["PREFLIGHT", "NODE"].includes(role) ? "5".repeat(64) : "NONE",
    rustc: role === "CARGO" ? "/home/zk/task/rust/bin/rustc" : "NONE",
    "rustc-version": role === "CARGO" ? "rustc 1.97.1 (fixture)" : "NONE",
    "rustc-sha256": role === "CARGO" ? "6".repeat(64) : "NONE",
    rustdoc: role === "CARGO" ? "/home/zk/task/rust/bin/rustdoc" : "NONE",
    "rustdoc-version": role === "CARGO" ? "rustdoc 1.97.1 (fixture)" : "NONE",
    "rustdoc-sha256": role === "CARGO" ? "7".repeat(64) : "NONE",
    "keyring-daemon": "/home/zk/task/keyring/gnome-keyring-daemon",
    "keyring-daemon-sha256": "4".repeat(64),
    "keyring-library-path": "/home/zk/task/keyring/lib",
    "keyring-library-manifest-digest": typed("keyring-library-manifest", "f"),
    "sandbox-helper": "/home/zk/task/codex/codex-resources/bwrap",
    "sandbox-helper-version": "bubblewrap built for Codex",
    "sandbox-helper-sha256": "3".repeat(64),
    "timeout-ms": "120000",
    "stdout-limit-bytes": "262144",
    "stderr-limit-bytes": "262144",
    attempt: "1",
    "retry-of": "NONE",
    "reconnect-of": "NONE",
    ...(role === "GIT" ? {
      "stdin-byte-len": "7",
      "stdin-sha256": createHash("sha256").update("managed", "utf8").digest("hex"),
    } : {}),
  };
  return [...Object.entries(values).flatMap(([key, value]) => [`--${key}`, value]), "--", "app-server"];
}

test("supervisor accepts only the closed role/cgroup/credential/bounds vector", () => {
  const parsed = parseSupervisorArgs(argvFixture());
  assert.equal(parsed.options.role, "PREFLIGHT");
  assert.equal(parsed.options.attempt, 1);
  assert.equal(parsed.options["retry-of"], null);
  assert.deepEqual(parsed.commandArgs, ["app-server"]);

  const reconnectArgv = argvFixture();
  reconnectArgv[reconnectArgv.indexOf("--reconnect-of") + 1] = typed("attempt-receipt", "8");
  const reconnect = parseSupervisorArgs(reconnectArgv);
  assert.equal(reconnect.options.attempt, 1);
  assert.equal(reconnect.options["retry-of"], null);
  assert.equal(reconnect.options["reconnect-of"], typed("attempt-receipt", "8"));

  const laterAttemptArgv = argvFixture();
  laterAttemptArgv[laterAttemptArgv.indexOf("--attempt") + 1] = "2";
  const laterAttempt = parseSupervisorArgs(laterAttemptArgv);
  assert.equal(laterAttempt.options.attempt, 2);
  assert.equal(laterAttempt.options["retry-of"], null);
  assert.equal(laterAttempt.options["reconnect-of"], null);

  for (const mutate of [
    (argv) => argv.splice(0, 0, "--extra", "x"),
    (argv) => { argv[argv.indexOf("--role") + 1] = "WINDOWS"; },
    (argv) => { argv[argv.indexOf("--unit") + 1] = "other.service"; },
    (argv) => { argv[argv.indexOf("--stdout-limit-bytes") + 1] = "99999999"; },
    (argv) => { argv[argv.indexOf("--credential-seal-digest") + 1] = typed("codex-home", "d"); },
    (argv) => { argv[argv.indexOf("--keyring-daemon-sha256") + 1] = "not-a-digest"; },
    (argv) => { argv[argv.indexOf("--keyring-library-manifest-digest") + 1] = typed("codex-home", "f"); },
    (argv) => {
      argv[argv.indexOf("--attempt") + 1] = "2";
      argv[argv.indexOf("--retry-of") + 1] = typed("attempt-receipt", "8");
      argv[argv.indexOf("--reconnect-of") + 1] = typed("attempt-receipt", "9");
    },
  ]) {
    const argv = argvFixture();
    mutate(argv);
    assert.throws(() => parseSupervisorArgs(argv), { code: "WSL2_SUPERVISOR_ARGUMENTS_REJECTED" });
  }
});

test("NODE supervisor vector pins a verifier tool and retry lineage is verifier-owned", () => {
  const firstVerifier = argvFixture("NODE");
  firstVerifier[firstVerifier.indexOf("--attempt") + 1] = "2";
  const firstParsed = parseSupervisorArgs(firstVerifier);
  assert.equal(firstParsed.options.attempt, 2);
  assert.equal(firstParsed.options["retry-of"], null);
  assert.equal(firstParsed.options["reconnect-of"], null);

  const argv = argvFixture("NODE");
  argv[argv.indexOf("--attempt") + 1] = "2";
  argv[argv.indexOf("--retry-of") + 1] = typed("verifier-receipt", "9");
  const parsed = parseSupervisorArgs(argv);
  assert.equal(parsed.options["verifier-tool"], "/home/zk/task/node/bin/npm");
  assert.equal(parsed.options["node-runtime"], "/usr/bin/node");
  assert.equal(parsed.options["retry-of"], typed("verifier-receipt", "9"));
  const substituted = argvFixture("NODE");
  substituted[substituted.indexOf("--verifier-tool-sha256") + 1] = "NONE";
  assert.throws(() => parseSupervisorArgs(substituted), { code: "WSL2_SUPERVISOR_ARGUMENTS_REJECTED" });
  const loaderSubstituted = argvFixture("NODE");
  loaderSubstituted[loaderSubstituted.indexOf("--node-runtime-sha256") + 1] = "6".repeat(64);
  loaderSubstituted.splice(loaderSubstituted.indexOf("--node-runtime-version"), 2);
  assert.throws(() => parseSupervisorArgs(loaderSubstituted), {
    code: "WSL2_SUPERVISOR_ARGUMENTS_REJECTED",
  });
  const ambiguousContinuation = argvFixture("NODE");
  ambiguousContinuation[ambiguousContinuation.indexOf("--attempt") + 1] = "2";
  ambiguousContinuation[ambiguousContinuation.indexOf("--retry-of") + 1] = typed("verifier-receipt", "8");
  ambiguousContinuation[ambiguousContinuation.indexOf("--reconnect-of") + 1] = typed("verifier-receipt", "9");
  assert.throws(() => parseSupervisorArgs(ambiguousContinuation), {
    code: "WSL2_SUPERVISOR_ARGUMENTS_REJECTED",
  });
});

test("CARGO supervisor vector pins rustc and rustdoc inputs", () => {
  const parsed = parseSupervisorArgs(argvFixture("CARGO"));
  assert.equal(parsed.options.rustc, "/home/zk/task/rust/bin/rustc");
  assert.equal(parsed.options.rustdoc, "/home/zk/task/rust/bin/rustdoc");
  for (const name of ["rustc-sha256", "rustdoc-sha256"]) {
    const substituted = argvFixture("CARGO");
    substituted[substituted.indexOf(`--${name}`) + 1] = "NONE";
    assert.throws(() => parseSupervisorArgs(substituted), {
      code: "WSL2_SUPERVISOR_ARGUMENTS_REJECTED",
    });
  }
});

test("nested verifier command rewrite pins NODE loader and CARGO compiler fds", () => {
  const preflightOptions = parseSupervisorArgs(argvFixture("PREFLIGHT")).options;
  assert.deepEqual(rewriteSealedVerifierCommandArgs([
    "sandbox", "--", preflightOptions["node-runtime"], "-e", "probe",
  ], preflightOptions), [
    "sandbox", "--", "/proc/self/fd/4", "-e", "probe",
  ]);

  const nodeOptions = parseSupervisorArgs(argvFixture("NODE")).options;
  assert.deepEqual(rewriteSealedVerifierCommandArgs([
    "sandbox", "--", "/usr/bin/env", "-i", "PATH=/usr/bin", nodeOptions["verifier-tool"], "test",
  ], nodeOptions), [
    "sandbox", "--", "/usr/bin/env", "-i", "PATH=/usr/bin",
    "/proc/self/fd/5", "/proc/self/fd/4", "test",
  ]);

  const cargoOptions = parseSupervisorArgs(argvFixture("CARGO")).options;
  assert.deepEqual(rewriteSealedVerifierCommandArgs([
    "sandbox", "--", "/usr/bin/env", "-i", `RUSTC=${cargoOptions.rustc}`,
    `RUSTDOC=${cargoOptions.rustdoc}`, cargoOptions["verifier-tool"], "test",
  ], cargoOptions), [
    "sandbox", "--", "/usr/bin/env", "-i", "RUSTC=/proc/self/fd/5",
    "RUSTDOC=/proc/self/fd/6", "/proc/self/fd/4", "test",
  ]);
});

test("GIT supervisor vector requires an exact bounded stdin length and digest", () => {
  const parsed = parseSupervisorArgs(argvFixture("GIT"));
  assert.equal(parsed.options["stdin-byte-len"], 7);
  assert.equal(parsed.options["stdin-sha256"],
    createHash("sha256").update("managed", "utf8").digest("hex"));

  for (const mutate of [
    (argv) => argv.splice(argv.indexOf("--stdin-byte-len"), 2),
    (argv) => { argv[argv.indexOf("--stdin-byte-len") + 1] = "33554433"; },
    (argv) => { argv[argv.indexOf("--stdin-sha256") + 1] = "not-a-digest"; },
  ]) {
    const argv = argvFixture("GIT");
    mutate(argv);
    assert.throws(() => parseSupervisorArgs(argv), { code: "WSL2_SUPERVISOR_ARGUMENTS_REJECTED" });
  }
});

test("exact stdin tracking rejects extra bytes and records partial or digest-substituted input", () => {
  const expected = Buffer.from("managed", "utf8");
  const digest = createHash("sha256").update(expected).digest("hex");
  const exact = createExactStdinTracker(expected.length, digest);
  exact.observe(expected.subarray(0, 3));
  exact.observe(expected.subarray(3));
  assert.deepEqual(exact.finish(), {
    stdin_bytes: expected.length,
    stdin_sha256: digest,
    stdin_complete: true,
  });

  const partial = createExactStdinTracker(expected.length, digest);
  partial.observe(expected.subarray(0, 3));
  assert.equal(partial.finish().stdin_complete, false);

  const substituted = createExactStdinTracker(expected.length,
    createHash("sha256").update("other", "utf8").digest("hex"));
  substituted.observe(expected);
  assert.equal(substituted.finish().stdin_complete, false);

  const extra = createExactStdinTracker(expected.length, digest);
  assert.throws(() => extra.observe(Buffer.from("managed-extra", "utf8")), {
    code: "WSL2_STDIN_IDENTITY_REJECTED",
  });
});

test("output bounds trip on cumulative bytes at the first overflowing chunk", () => {
  const state = { stdoutBytes: 0, outputBoundExceeded: false };
  assert.equal(observeBoundedOutput(state, "stdoutBytes", Buffer.alloc(700), 1_024), false);
  assert.equal(observeBoundedOutput(state, "stdoutBytes", Buffer.alloc(325), 1_024), true);
  assert.equal(state.stdoutBytes, 1_025);
  assert.equal(state.outputBoundExceeded, true);
  assert.equal(observeBoundedOutput(state, "stdoutBytes", Buffer.alloc(64 * 1_024), 1_024), false,
    "termination is signalled exactly once");
  assert.equal(state.stdoutBytes, 1_025, "the receipt uses one bounded overflow sentinel");
});

test("terminal observation cannot miss a child that exited before listener setup", async () => {
  const child = spawn(process.execPath, ["-e", ""]);
  await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", resolve);
  });
  assert.deepEqual(await observeChildTerminal(child), { code: 0, signal: null });
});

test("keyring private library selection is exact and manifest-bound", () => {
  const records = [
    { path: ".", kind: "DIRECTORY", mode: 0o755, owner_uid: 1000 },
    { path: "libgck-1.so.0", kind: "SYMLINK", owner_uid: 1000, target: "libgck-1.so.0.0.0" },
    {
      path: "libgck-1.so.0.0.0", kind: "FILE", mode: 0o644, owner_uid: 1000,
      byte_len: 8, sha256: "1".repeat(64),
    },
    { path: "libgcr-base-3.so.1", kind: "SYMLINK", owner_uid: 1000, target: "libgcr-base-3.so.1.0.0" },
    {
      path: "libgcr-base-3.so.1.0.0", kind: "FILE", mode: 0o644, owner_uid: 1000,
      byte_len: 9, sha256: "2".repeat(64),
    },
  ];
  assert.deepEqual(selectKeyringPrivateLibraryRecords(records).map((entry) => entry.path), [
    "libgck-1.so.0.0.0",
    "libgcr-base-3.so.1.0.0",
  ]);
  assert.throws(() => selectKeyringPrivateLibraryRecords(records.slice(0, -1)), {
    code: "WSL2_KEYRING_LIBRARY_REJECTED",
  });
  assert.throws(() => selectKeyringPrivateLibraryRecords([...records, {
    path: "libreplacement.so.1", kind: "FILE", mode: 0o644, owner_uid: 1000,
    byte_len: 1, sha256: "3".repeat(64),
  }]), { code: "WSL2_KEYRING_LIBRARY_REJECTED" });
});

test("live keyring loader accepts daemon and both private libraries only through inherited fds", {
  skip: process.platform !== "linux" || process.env.LATTICE_LIVE_KEYRING_PROBE !== "1",
}, async () => {
  const daemonSource = process.env.LATTICE_KEYRING_DAEMON_PATH;
  const librarySourceRoot = process.env.LATTICE_KEYRING_LIBRARY_PATH;
  assert.equal(path.isAbsolute(daemonSource), true);
  assert.equal(path.isAbsolute(librarySourceRoot), true);
  const opened = [];
  const home = await mkdtemp(path.join(process.env.HOME, "lattice-keyring-fd-probe-"));
  try {
    const daemon = path.join(home, "gnome-keyring-daemon");
    await copyFile(daemonSource, daemon);
    await chmod(daemon, 0o755);
    const daemonBytes = await readFile(daemon);
    const daemonSeal = await openExecutableIdentitySeal(daemon,
      createHash("sha256").update(daemonBytes).digest("hex"), "WSL2_TEST_KEYRING_DAEMON_REJECTED");
    opened.push(daemonSeal);
    const libraries = [];
    for (const name of ["libgck-1.so.0.0.0", "libgcr-base-3.so.1.0.0"]) {
      const file = path.join(home, name);
      await copyFile(path.join(librarySourceRoot, name), file);
      await chmod(file, 0o644);
      const bytes = await readFile(file);
      const seal = await openRegularFileIdentitySeal(file,
        createHash("sha256").update(bytes).digest("hex"), "WSL2_TEST_KEYRING_LIBRARY_REJECTED");
      opened.push(seal);
      libraries.push(seal);
      await rename(file, `${file}.sealed`);
      await writeFile(file, "REPLACEMENT", { mode: 0o644 });
    }
    await rename(daemon, `${daemon}.sealed`);
    await writeFile(daemon, "#!/bin/sh\nexit 99\n", { mode: 0o755 });
    await runSealedKeyringUnlock(daemonSeal, libraries, {
      HOME: home,
      PATH: "/usr/bin:/bin",
      LANG: "C.UTF-8",
      LC_ALL: "C.UTF-8",
      XDG_RUNTIME_DIR: process.env.XDG_RUNTIME_DIR,
    });
    const lockedProbe = spawn("/usr/bin/gdbus", [
      "call", "--session", "--dest", "org.freedesktop.secrets",
      "--object-path", "/org/freedesktop/secrets/collection/login",
      "--method", "org.freedesktop.DBus.Properties.Get",
      "org.freedesktop.Secret.Collection", "Locked",
    ], { env: process.env, stdio: ["ignore", "pipe", "pipe"] });
    let lockedOutput = "";
    lockedProbe.stdout.setEncoding("utf8");
    lockedProbe.stdout.on("data", (chunk) => { lockedOutput += chunk; });
    assert.deepEqual(await observeChildTerminal(lockedProbe), { code: 0, signal: null });
    assert.equal(lockedOutput.trim(), "(<false>,)");
  } finally {
    await Promise.allSettled(opened.map(closeExecutableIdentitySeal));
    await rm(home, { recursive: true, force: true });
  }
});

test("only the provider child receives the sealed DBus credential channel", () => {
  const before = process.env.DBUS_SESSION_BUS_ADDRESS;
  const beforeCodexHome = process.env.CODEX_HOME;
  process.env.DBUS_SESSION_BUS_ADDRESS = "unix:path=/run/user/1000/credential-sentinel";
  process.env.CODEX_HOME = "/home/zk/credential-sentinel";
  try {
    assert.equal(explicitChildEnvironment("PROVIDER").DBUS_SESSION_BUS_ADDRESS,
      "unix:path=/run/user/1000/credential-sentinel");
    assert.equal(explicitChildEnvironment("PROVIDER").CODEX_HOME,
      "/home/zk/credential-sentinel");
    for (const role of ["PREFLIGHT", "NODE", "CARGO"]) {
      assert.equal(explicitChildEnvironment(role).DBUS_SESSION_BUS_ADDRESS, undefined);
      assert.equal(explicitChildEnvironment(role).CODEX_HOME, undefined);
    }
  } finally {
    if (before === undefined) delete process.env.DBUS_SESSION_BUS_ADDRESS;
    else process.env.DBUS_SESSION_BUS_ADDRESS = before;
    if (beforeCodexHome === undefined) delete process.env.CODEX_HOME;
    else process.env.CODEX_HOME = beforeCodexHome;
  }
});

test("unified cgroup parser rejects root, sibling, v1, and path escape substitutions", () => {
  const unit = `lattice-wsl2-${"a".repeat(16)}-preflight-${"f".repeat(12)}.service`;
  const expected = `/user.slice/user-1000.slice/user@1000.service/app.slice/${unit}`;
  assert.equal(parseUnifiedCgroup(`0::${expected}\n`, unit, 1000), expected);
  for (const content of [
    "0::/\n", `0::/user.slice/sibling.service\n`, `2:cpu:${expected}\n`,
    `0::/user.slice/../${unit}\n`, `0::${expected}\n0::${expected}\n`,
    `0::/user.slice/foreign.slice/${unit}\n`,
  ]) {
    assert.throws(() => parseUnifiedCgroup(content, unit, 1000), {
      code: "WSL2_CGROUP_V2_FENCE_REJECTED",
    });
  }
});

test("credential seal is a digest of file identity and keyring-only facts, never config bytes", () => {
  const authority = typed("wsl2-credential-authority", "a");
  const facts = {
    config_sha256: "b".repeat(64),
    config_identity: { device: "1", inode: "2", owner_uid: 1000, mode: "100400", size: 18 },
    keyring_only: true,
    auth_json_absent: true,
    shell_environment_policy: parseManagedShellEnvironmentPolicy(managedConfig),
  };
  const seal = credentialSealIdentity(authority, facts);
  assert.match(seal, /^credential-seal:sha256:[a-f0-9]{64}$/u);
  assert.notEqual(seal, `credential-seal:sha256:${createHash("sha256").update("secret").digest("hex")}`);
  assert.notEqual(credentialSealIdentity(authority, { ...facts, config_identity: { ...facts.config_identity, inode: "3" } }), seal);
  assert.notEqual(credentialSealIdentity(authority, { ...facts, auth_json_absent: false }), seal);
});

test("managed shell environment policy keeps HOME and PATH but rejects credential-home or set overrides", () => {
  const policy = parseManagedShellEnvironmentPolicy(managedConfig);
  assert.deepEqual(policy.include_only, ["HOME", "PATH", "LANG", "LC_ALL", "TERM", "COLORTERM"]);
  assert.equal(policy.required_keys_present, true);
  assert.equal(policy.forbidden_keys_absent, true);
  for (const changed of [
    managedConfig.replace('inherit = "all"', 'inherit = "none"'),
    managedConfig.replace('"HOME", "PATH"', '"HOME", "CODEX_HOME", "PATH"'),
    managedConfig.replace("experimental_use_profile = false", 'set = { HOME = "/tmp" }\nexperimental_use_profile = false'),
  ]) {
    assert.throws(() => parseManagedShellEnvironmentPolicy(changed), {
      code: "WSL2_SHELL_ENVIRONMENT_POLICY_REJECTED",
    });
  }
});

test("credential watch is armed before the post-seal ABA hook", {
  skip: process.platform !== "linux",
}, async () => {
  const root = await mkdtemp(path.join(process.env.HOME, "lattice-credential-arm-order-"));
  const config = path.join(root, "config.toml");
  const auth = path.join(root, "auth.json");
  try {
    const bytes = Buffer.from(managedConfig, "utf8");
    await writeFile(config, bytes, { mode: 0o600 });
    const metadata = await stat(config, { bigint: true });
    const authority = typed("wsl2-credential-authority", "a");
    const facts = {
      config_sha256: createHash("sha256").update(bytes).digest("hex"),
      config_identity: {
        device: String(metadata.dev), inode: String(metadata.ino), owner_uid: Number(metadata.uid),
        mode: metadata.mode.toString(8), size: Number(metadata.size),
      },
      keyring_only: true,
      auth_json_absent: true,
      shell_environment_policy: parseManagedShellEnvironmentPolicy(managedConfig),
    };
    let resolveDrift;
    const drift = new Promise((resolve) => { resolveDrift = resolve; });
    await assert.rejects(openCredentialSeal({
      "codex-home": root,
      "config-digest": `codex-config:sha256:${facts.config_sha256}`,
      "credential-authority-ref": authority,
      "credential-seal-digest": credentialSealIdentity(authority, facts),
    }, {
      onDrift: resolveDrift,
      afterInitialObservation: async () => {
        await writeFile(auth, "{}\n", { mode: 0o600 });
        await unlink(auth);
        await Promise.race([
          drift,
          new Promise((_, reject) => setTimeout(() => reject(new Error("watch timeout")), 2_000)),
        ]);
      },
    }), { code: "WSL2_CREDENTIAL_SEAL_REJECTED" });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("Linux credential watch catches transient auth.json creation and deletion", {
  skip: process.platform !== "linux",
}, async () => {
  const root = await mkdtemp(path.join(process.env.HOME, "lattice-credential-watch-auth-"));
  const config = path.join(root, "config.toml");
  const auth = path.join(root, "auth.json");
  let watcher;
  try {
    await writeFile(config, 'cli_auth_credentials_store = "keyring"\n', { mode: 0o600 });
    let resolveDrift;
    const drift = new Promise((resolve) => { resolveDrift = resolve; });
    watcher = startCredentialMutationWatch(root, resolveDrift);
    await writeFile(auth, "{}\n", { mode: 0o600 });
    await unlink(auth);
    assert.match(await Promise.race([
      drift,
      new Promise((_, reject) => setTimeout(() => reject(new Error("watch timeout")), 2_000)),
    ]), /auth\.json/u);
    assert.equal(watcher.drifted, true);
  } finally {
    watcher?.close();
    await rm(root, { recursive: true, force: true });
  }
});

test("Linux credential watch catches config inode replacement", {
  skip: process.platform !== "linux",
}, async () => {
  const root = await mkdtemp(path.join(process.env.HOME, "lattice-credential-watch-config-"));
  const config = path.join(root, "config.toml");
  let watcher;
  try {
    await writeFile(config, 'cli_auth_credentials_store = "keyring"\n', { mode: 0o600 });
    let resolveDrift;
    const drift = new Promise((resolve) => { resolveDrift = resolve; });
    watcher = startCredentialMutationWatch(root, resolveDrift);
    await rename(config, `${config}.old`);
    await writeFile(config, 'cli_auth_credentials_store = "file"\n', { mode: 0o600 });
    assert.match(await Promise.race([
      drift,
      new Promise((_, reject) => setTimeout(() => reject(new Error("watch timeout")), 2_000)),
    ]), /CONFIG|config\.toml/u);
    assert.equal(watcher.drifted, true);
  } finally {
    watcher?.close();
    await rm(root, { recursive: true, force: true });
  }
});

test("Linux credential watch catches whole CODEX_HOME replacement from its parent", {
  skip: process.platform !== "linux",
}, async () => {
  const parent = await mkdtemp(path.join(process.env.HOME, "lattice-credential-watch-home-"));
  const root = path.join(parent, "codex-home");
  let watcher;
  try {
    await mkdir(root);
    await writeFile(path.join(root, "config.toml"),
      'cli_auth_credentials_store = "keyring"\n', { mode: 0o600 });
    let resolveDrift;
    const drift = new Promise((resolve) => { resolveDrift = resolve; });
    watcher = startCredentialMutationWatch(root, resolveDrift);
    await rename(root, `${root}.old`);
    await mkdir(root);
    assert.equal(await Promise.race([
      drift,
      new Promise((_, reject) => setTimeout(() => reject(new Error("watch timeout")), 2_000)),
    ]), "CODEX_HOME_REPLACED");
    assert.equal(watcher.drifted, true);
  } finally {
    watcher?.close();
    await rm(parent, { recursive: true, force: true });
  }
});

test("Linux credential watch allows normal Codex state while retaining credential guards", {
  skip: process.platform !== "linux",
}, async () => {
  const parent = await mkdtemp(path.join(process.env.HOME, "lattice-credential-watch-state-"));
  const root = path.join(parent, "codex-home");
  const sibling = path.join(parent, "task-home");
  const auth = path.join(root, "auth.json");
  let watcher;
  try {
    await mkdir(root);
    await writeFile(path.join(root, "config.toml"),
      'cli_auth_credentials_store = "keyring"\n', { mode: 0o600 });
    let resolveDrift;
    const drift = new Promise((resolve) => { resolveDrift = resolve; });
    watcher = startCredentialMutationWatch(root, resolveDrift);

    await mkdir(sibling);
    await writeFile(path.join(root, "session-state.json"), "{}\n", { mode: 0o600 });
    await new Promise((resolve) => setTimeout(resolve, 100));
    assert.equal(watcher.drifted, false);

    await writeFile(auth, "{}\n", { mode: 0o600 });
    assert.match(await Promise.race([
      drift,
      new Promise((_, reject) => setTimeout(() => reject(new Error("watch timeout")), 2_000)),
    ]), /auth\.json/u);
    assert.equal(watcher.drifted, true);
  } finally {
    watcher?.close();
    await rm(parent, { recursive: true, force: true });
  }
});

test("an opened Linux executable seal runs the pinned inode after its path is replaced", {
  skip: process.platform !== "linux",
}, async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-supervisor-seal-"));
  const executable = path.join(root, "tool");
  const displaced = path.join(root, "tool.sealed");
  try {
    const original = Buffer.from("#!/bin/sh\nprintf OLD\n", "utf8");
    await writeFile(executable, original, { mode: 0o755 });
    const seal = await openExecutableIdentitySeal(
      executable,
      createHash("sha256").update(original).digest("hex"),
      "WSL2_TEST_TOOL_REJECTED",
    );
    try {
      await rename(executable, displaced);
      await writeFile(executable, "#!/bin/sh\nprintf NEW\n", { mode: 0o755 });
      const output = await runSealedExecutableOutput(seal, [], {}, "WSL2_TEST_TOOL_REJECTED");
      assert.equal(output, "OLD");
    } finally {
      await closeExecutableIdentitySeal(seal);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("an opened Linux executable seal fails closed if its pinned inode bytes drift", {
  skip: process.platform !== "linux",
}, async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-supervisor-seal-drift-"));
  const executable = path.join(root, "tool");
  try {
    const original = Buffer.from("#!/bin/sh\nprintf OLD\n", "utf8");
    await writeFile(executable, original, { mode: 0o755 });
    const seal = await openExecutableIdentitySeal(
      executable,
      createHash("sha256").update(original).digest("hex"),
      "WSL2_TEST_TOOL_REJECTED",
    );
    try {
      await chmod(executable, 0o755);
      await writeFile(executable, "#!/bin/sh\nprintf BAD\n", { mode: 0o755 });
      await assert.rejects(
        runSealedExecutableOutput(seal, [], {}, "WSL2_TEST_TOOL_REJECTED"),
        { code: "WSL2_TEST_TOOL_REJECTED" },
      );
    } finally {
      await closeExecutableIdentitySeal(seal);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("an opened Linux library seal keeps the manifest bytes after path replacement", {
  skip: process.platform !== "linux",
}, async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-supervisor-library-seal-"));
  const library = path.join(root, "libgck-1.so.0.0.0");
  const displaced = `${library}.sealed`;
  try {
    const original = Buffer.from("OLD-LIBRARY", "utf8");
    await writeFile(library, original, { mode: 0o644 });
    const seal = await openRegularFileIdentitySeal(
      library,
      createHash("sha256").update(original).digest("hex"),
      "WSL2_TEST_LIBRARY_REJECTED",
    );
    try {
      await rename(library, displaced);
      await writeFile(library, "NEW-LIBRARY", { mode: 0o644 });
      await verifyRegularFileIdentitySeal(seal);
      assert.equal((await seal.handle.readFile({ encoding: "utf8" })), "OLD-LIBRARY");
    } finally {
      await closeExecutableIdentitySeal(seal);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a sealed Node loader reads the pinned npm script inode", {
  skip: process.platform !== "linux",
}, async () => {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-supervisor-node-loader-"));
  const script = path.join(root, "npm-cli.js");
  const displaced = `${script}.sealed`;
  let nodeSeal;
  let scriptSeal;
  try {
    const nodeBytes = await readFile(process.execPath);
    nodeSeal = await openExecutableIdentitySeal(
      process.execPath,
      createHash("sha256").update(nodeBytes).digest("hex"),
      "WSL2_TEST_NODE_REJECTED",
    );
    const original = Buffer.from('process.stdout.write("OLD")\n', "utf8");
    await writeFile(script, original, { mode: 0o644 });
    scriptSeal = await openRegularFileIdentitySeal(
      script,
      createHash("sha256").update(original).digest("hex"),
      "WSL2_TEST_NPM_REJECTED",
    );
    await rename(script, displaced);
    await writeFile(script, 'process.stdout.write("NEW")\n', { mode: 0o644 });
    assert.equal(await runSealedNodeScriptOutput(nodeSeal, scriptSeal, [], {},
      "WSL2_TEST_NODE_REJECTED"), "OLD");
  } finally {
    await closeExecutableIdentitySeal(scriptSeal).catch(() => {});
    await closeExecutableIdentitySeal(nodeSeal).catch(() => {});
    await rm(root, { recursive: true, force: true });
  }
});

test("live sealed Node loader resolves the task-owned npm script", {
  skip: process.platform !== "linux" || process.env.LATTICE_LIVE_NPM_FD_PROBE !== "1",
}, async () => {
  const node = process.env.LATTICE_NODE_PATH;
  const npm = process.env.LATTICE_NPM_PATH;
  const opened = [];
  try {
    const nodeSeal = await openExecutableIdentitySeal(node,
      createHash("sha256").update(await readFile(node)).digest("hex"),
      "WSL2_TEST_NODE_REJECTED");
    opened.push(nodeSeal);
    const npmSeal = await openRegularFileIdentitySeal(npm,
      createHash("sha256").update(await readFile(await realpath(npm))).digest("hex"),
      "WSL2_TEST_NPM_REJECTED");
    opened.push(npmSeal);
    assert.equal((await runSealedNodeScriptOutput(nodeSeal, npmSeal, ["--version"], {
      HOME: process.env.HOME,
      PATH: "/usr/bin:/bin",
      LANG: "C.UTF-8",
      LC_ALL: "C.UTF-8",
    }, "WSL2_TEST_NPM_REJECTED")).trim(), process.env.LATTICE_NPM_VERSION);
  } finally {
    await Promise.allSettled(opened.map(closeExecutableIdentitySeal));
  }
});

test("live Cargo inherits sealed rustc and rustdoc fds", {
  skip: process.platform !== "linux" || process.env.LATTICE_LIVE_CARGO_FD_PROBE !== "1",
  timeout: 30_000,
}, async () => {
  const paths = {
    cargo: process.env.LATTICE_CARGO_PATH,
    rustc: process.env.LATTICE_RUSTC_PATH,
    rustdoc: process.env.LATTICE_RUSTDOC_PATH,
  };
  for (const file of Object.values(paths)) assert.equal(path.isAbsolute(file), true);
  const root = await mkdtemp(path.join(process.env.HOME, "lattice-cargo-fd-probe-"));
  const opened = [];
  try {
    await mkdir(path.join(root, "src"));
    await writeFile(path.join(root, "Cargo.toml"), [
      "[package]", 'name = "lattice_fd_probe"', 'version = "0.0.0"', 'edition = "2024"', "",
    ].join("\n"), { mode: 0o600 });
    await writeFile(path.join(root, "src", "main.rs"), "fn main() {}\n", { mode: 0o600 });
    for (const [name, file] of Object.entries(paths)) {
      const bytes = await readFile(file);
      const seal = await openExecutableIdentitySeal(file,
        createHash("sha256").update(bytes).digest("hex"), `WSL2_TEST_${name.toUpperCase()}_REJECTED`);
      opened.push(seal);
    }
    const [cargo, rustc, rustdoc] = opened;
    const result = await new Promise((resolve, reject) => {
      const child = spawn("/proc/self/fd/3", ["check", "--offline", "--manifest-path",
        path.join(root, "Cargo.toml")], {
        cwd: root,
        env: {
          HOME: root,
          TMPDIR: root,
          CARGO_HOME: path.join(root, "cargo-home"),
          CARGO_TARGET_DIR: path.join(root, "target"),
          CARGO_NET_OFFLINE: "true",
          RUSTC: "/proc/self/fd/4",
          RUSTDOC: "/proc/self/fd/5",
          PATH: "/usr/bin:/bin",
          LANG: "C.UTF-8",
          LC_ALL: "C.UTF-8",
        },
        stdio: ["ignore", "pipe", "pipe", cargo.handle.fd, rustc.handle.fd, rustdoc.handle.fd],
      });
      const stderr = [];
      child.stderr.on("data", (chunk) => stderr.push(chunk));
      child.once("error", reject);
      child.once("exit", (code, signal) => resolve({
        code,
        signal,
        stderr: Buffer.concat(stderr).toString("utf8"),
      }));
    });
    assert.equal(result.code, 0, result.stderr);
    assert.equal(result.signal, null);
  } finally {
    await Promise.allSettled(opened.map(closeExecutableIdentitySeal));
    await rm(root, { recursive: true, force: true });
  }
});

test("supervisor source can be imported from a sealed in-memory data URL", async () => {
  const source = await readFile(new URL("../src/wsl2-codex-supervisor.mjs", import.meta.url), "utf8");
  const loaded = await import(`data:text/javascript;base64,${Buffer.from(source, "utf8").toString("base64")}`);
  assert.equal(typeof loaded.runWsl2CodexSupervisor, "function");
});
