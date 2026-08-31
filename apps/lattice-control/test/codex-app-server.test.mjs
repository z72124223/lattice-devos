import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import process from "node:process";
import test from "node:test";
import {
  CodexAppServer,
  resolveCodexAppServerLaunch,
} from "../src/codex-app-server.mjs";
import {
  buildLegacyWsl2CodexLaunchFixture,
  executionEnvironmentIdentity,
  validateWsl2ExecutionEnvironment,
} from "../src/wsl2-execution-domain.mjs";

const wslDigest = (kind, value) => `${kind}:sha256:${value.repeat(64)}`;

test("legacy WSL2 launch is fixture-only and keeps gateway and Linux Codex separate", () => {
  const descriptor = {
    schema: "lattice.execution-environment.wsl2-linux/1.0",
    kind: "WSL2_LINUX",
    distribution: "Ubuntu",
    gateway: {
      windows_path: String.raw`C:\Windows\System32\wsl.exe`,
      version: "10.0.19041.4522",
      sha256: "4".repeat(64),
    },
    linux: {
      launcher_path: "/opt/codex/bin/codex",
      launcher_version: "codex-cli 0.146.0",
      launcher_sha256: "2".repeat(64),
      node_path: "/usr/bin/node",
      node_version: "v22.22.1",
      node_sha256: "d".repeat(64),
      git_path: "/usr/bin/git",
      git_version: "git version 2.53.0",
      git_sha256: "5".repeat(64),
      supervisor_path: "/mnt/c/lattice/wsl2-codex-supervisor.mjs",
      supervisor_sha256: "6".repeat(64),
      codex_home: "/home/zk/lattice/codex-home",
      config_digest: wslDigest("codex-config", "a"),
      cwd: "/home/zk/lattice/repository",
      repository_identity: wslDigest("repository", "b"),
      dbus_run_session_path: "/usr/bin/dbus-run-session",
      dbus_run_session_sha256: "7".repeat(64),
      setsid_path: "/usr/bin/setsid",
      setsid_sha256: "8".repeat(64),
      keyring_daemon_path: "/home/zk/lattice/keyring/gnome-keyring-daemon",
      keyring_daemon_sha256: "9".repeat(64),
      keyring_library_path: "/home/zk/lattice/keyring/lib",
      xdg_runtime_dir: "/home/zk/lattice/keyring/run",
    },
    path_mapping: {
      windows_path: String.raw`\\wsl.localhost\Ubuntu\home\zk\lattice\repository`,
      linux_path: "/home/zk/lattice/repository",
      digest: wslDigest("path-mapping", "c"),
    },
    identity_digest: null,
  };
  descriptor.identity_digest = executionEnvironmentIdentity(descriptor);
  assert.throws(() => validateWsl2ExecutionEnvironment(descriptor), {
    code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED",
  });
  const launch = buildLegacyWsl2CodexLaunchFixture(descriptor, { fence: "f".repeat(64) });
  assert.equal(launch.fixtureOnly, true);
  assert.equal(launch.command, descriptor.gateway.windows_path);
  assert.equal(launch.args.includes(descriptor.linux.launcher_path), true);
  assert.notEqual(descriptor.linux.launcher_path, launch.command);
  assert.throws(
    () => buildLegacyWsl2CodexLaunchFixture({
      ...descriptor,
      linux: { ...descriptor.linux, cwd: "/mnt/c/Users/f7212/repository" },
    }),
    { code: "WSL2_EXECUTION_ENVIRONMENT_REJECTED" },
  );
});

test("Windows command launch is closed to explicit scripted acceptance", () => {
  const environment = {
    SystemRoot: String.raw`C:\Windows`,
    ComSpec: String.raw`C:\Windows\System32\cmd.exe`,
    LATTICE_DELIVERY_CODEX_MODE: "SCRIPTED_ACCEPTANCE",
  };
  assert.deepEqual(
    resolveCodexAppServerLaunch(String.raw`C:\fixture-safe\scripted-codex.cmd`, {
      platform: "win32",
      env: environment,
    }),
    {
      command: environment.ComSpec,
      args: [
        "/d",
        "/s",
        "/c",
        "call",
        String.raw`C:\fixture-safe\scripted-codex.cmd`,
        "app-server",
        "--stdio",
      ],
    },
  );
  assert.throws(
    () => resolveCodexAppServerLaunch(String.raw`C:\fixture-safe\scripted-codex.cmd`, {
      platform: "win32",
      env: { ...environment, LATTICE_DELIVERY_CODEX_MODE: "OFFICIAL_CODEX_APP_SERVER" },
    }),
    { code: "CODEX_APP_SERVER_SCRIPT_REJECTED" },
  );
  assert.throws(
    () => resolveCodexAppServerLaunch(String.raw`C:\fixture&unsafe\scripted-codex.cmd`, {
      platform: "win32",
      env: environment,
    }),
    { code: "CODEX_APP_SERVER_SCRIPT_REJECTED" },
  );
});

class FakeProcess extends EventEmitter {
  stdin = new PassThrough();
  stdout = new PassThrough();
  stderr = new PassThrough();
  exitCode = null;
  killCount = 0;

  kill() {
    this.killCount += 1;
    this.exitCode = 0;
    this.emit("exit", 0, null);
  }
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function observeClientMessages(child, onMessage) {
  let buffered = "";
  child.stdin.on("data", (chunk) => {
    buffered += chunk.toString("utf8");
    for (;;) {
      const newline = buffered.indexOf("\n");
      if (newline < 0) break;
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      if (line) onMessage(JSON.parse(line));
    }
  });
}

function sendServerMessage(child, message) {
  child.stdout.write(`${JSON.stringify(message)}\n`);
}

function nextMacrotask() {
  return new Promise((resolve) => setImmediate(resolve));
}

function productionWslLaunchSpec(fence = "f".repeat(64)) {
  const unit = `lattice-wsl2-${"1".repeat(16)}-provider-${fence.slice(0, 12)}.service`;
  const executionEnvironmentRef = wslDigest("execution-environment", "e");
  const credentialSealDigest = wslDigest("credential-seal", "d");
  return {
    command: String.raw`C:\Windows\System32\wsl.exe`,
    args: [
      "--role", "PROVIDER",
      "--fence", fence,
      "--unit", unit,
      "--execution-environment-ref", executionEnvironmentRef,
      "--credential-seal-digest", credentialSealDigest,
      "--timeout-ms", "1000",
      "--stdout-limit-bytes", "262144",
      "--stderr-limit-bytes", "262144",
      "--attempt", "1",
      "--retry-of", "NONE",
      "--reconnect-of", "NONE",
      "--", "app-server",
    ],
    processFence: fence,
    serviceUnit: unit,
    gracefulClose: true,
    codexIdentity: {
      schema: "lattice.wsl2-codex-launch/1.1",
      execution_environment_ref: executionEnvironmentRef,
      credential_seal_digest: credentialSealDigest,
      process_fence: fence,
    },
  };
}

function replaceWslLaunchOption(launchSpec, name, value) {
  const replaced = structuredClone(launchSpec);
  const index = replaced.args.indexOf(`--${name}`);
  assert.notEqual(index, -1, `missing WSL launch option: ${name}`);
  replaced.args[index + 1] = value;
  return replaced;
}

function wslProcessMarker(launchSpec, overrides = {}) {
  return {
    schema: "lattice.wsl2-process-fence/1.1",
    fence: launchSpec.processFence,
    unit: launchSpec.serviceUnit,
    execution_environment_ref: launchSpec.codexIdentity.execution_environment_ref,
    credential_seal_digest: launchSpec.codexIdentity.credential_seal_digest,
    boot_id_digest: `wsl-boot:sha256:${"a".repeat(64)}`,
    pid: 123,
    process_start_ticks: "456",
    process_group_id: 123,
    cgroup_path: `/user.slice/user-1000.slice/user@1000.service/app.slice/${launchSpec.serviceUnit}`,
    cgroup_version: 2,
    delegated: false,
    attempt: 1,
    retry_of: null,
    reconnect_of: null,
    ...overrides,
  };
}

function wslSubtreeExitReceipt(launchSpec, marker, overrides = {}) {
  const seal = (pathname, index, extra = {}) => ({
    ...extra,
    path: pathname,
    resolved_path: pathname,
    sha256: String(index).repeat(64),
    device: "2049",
    inode: String(30000 + index),
    owner_uid: index < 4 ? 0 : 1000,
    mode: 0o500,
    size: 4096,
  });
  return {
    schema: "lattice.wsl2-subtree-exit/1.2",
    fence: marker.fence,
    unit: marker.unit,
    execution_environment_ref: marker.execution_environment_ref,
    credential_seal_digest: marker.credential_seal_digest,
    cgroup_path: marker.cgroup_path,
    zero_descendants: true,
    credential_seal_intact: true,
    credential_watch_intact: true,
    keyring_daemon_sha256: "7".repeat(64),
    keyring_library_manifest_digest: wslDigest("keyring-library-manifest", "8"),
    tool_input_identities: {
      executable: seal("/home/zk/task/codex", 1),
      verifier_tool: null,
      sandbox_helper: seal("/usr/bin/bwrap", 2),
      node_runtime: null,
      rustc: null,
      rustdoc: null,
      keyring_daemon: seal("/home/zk/task/keyring-daemon", 7),
      keyring_libraries: [
        seal("/home/zk/task/keyring/libgck-1.so.0.0.0", 8,
          { manifest_path: "libgck-1.so.0.0.0" }),
        seal("/home/zk/task/keyring/libgcr-base-3.so.1.0.0", 9,
          { manifest_path: "libgcr-base-3.so.1.0.0" }),
      ],
    },
    stdout_bytes: 128,
    stderr_bytes: 256,
    stdout_limit_bytes: 262144,
    stderr_limit_bytes: 262144,
    output_bound_exceeded: false,
    timeout_ms: 1000,
    timed_out: false,
    interrupted: false,
    stdin_bytes: 0,
    stdin_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    stdin_complete: true,
    attempt: marker.attempt,
    retry_of: marker.retry_of,
    reconnect_of: marker.reconnect_of,
    exit_code: 0,
    exit_signal: null,
    ...overrides,
  };
}

async function runWslPostExitProbe(_launchSpec, marker) {
  return {
    schema: "lattice.wsl2-provider-outer-post-exit/1.0",
    unit: marker.unit,
    fence: marker.fence,
    cgroup_path: marker.cgroup_path,
    boot_id_digest: marker.boot_id_digest,
    active_state: "inactive",
    sub_state: "dead",
    result: "success",
    delegate: "no",
    cgroup_exists: false,
    populated: null,
  };
}

test("WSL2 readiness awaits the exact 1.1 process marker after initialize", async () => {
  const launchSpec = productionWslLaunchSpec();
  const marker = wslProcessMarker(launchSpec);
  const child = new FakeProcess();
  observeClientMessages(child, (message) => {
    if (message.method !== "initialize") return;
    sendServerMessage(child, { id: message.id, result: { platformFamily: "unix" } });
    setTimeout(() => child.stderr.write(`${JSON.stringify(marker)}\n`), 25);
  });
  child.stdin.once("finish", () => {
    child.stderr.write(`${JSON.stringify(wslSubtreeExitReceipt(launchSpec, marker))}\n`);
    child.stderr.end();
    child.exitCode = 0;
    child.emit("exit", 0, null);
  });
  const codex = new CodexAppServer({
    launchSpec,
    spawnProcess: () => child,
    requestTimeoutMs: 1_000,
    lifecycleTimeoutMs: 250,
    runPostExitProbe: runWslPostExitProbe,
  });

  let connected = false;
  const connection = codex.connect().then(() => { connected = true; });
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.equal(connected, false, "initialize alone cannot outrun the WSL2 fence marker");
  await connection;
  assert.equal(codex.connected, true);
  await codex.close();
});

test("WSL2 attempt-one claimed-dispatch reconnect is accepted but retry is rejected before spawn", async () => {
  const reconnectRef = wslDigest("attempt-receipt", "7");
  const reconnectSpec = replaceWslLaunchOption(
    productionWslLaunchSpec(), "reconnect-of", reconnectRef,
  );
  const reconnectMarker = wslProcessMarker(reconnectSpec, { reconnect_of: reconnectRef });
  const child = new FakeProcess();
  observeClientMessages(child, (message) => {
    if (message.method !== "initialize") return;
    child.stderr.write(`${JSON.stringify(reconnectMarker)}\n`);
    sendServerMessage(child, { id: message.id, result: { platformFamily: "unix" } });
  });
  child.stdin.once("finish", () => {
    child.stderr.write(`${JSON.stringify(
      wslSubtreeExitReceipt(reconnectSpec, reconnectMarker),
    )}\n`);
    child.stderr.end();
    child.exitCode = 0;
    child.emit("exit", 0, null);
  });
  const reconnect = new CodexAppServer({
    launchSpec: reconnectSpec,
    spawnProcess: () => child,
    requestTimeoutMs: 1_000,
    lifecycleTimeoutMs: 250,
    runPostExitProbe: runWslPostExitProbe,
  });
  await reconnect.connect();
  await reconnect.close();

  let spawnCount = 0;
  const retrySpec = replaceWslLaunchOption(
    productionWslLaunchSpec(), "retry-of", wslDigest("attempt-receipt", "8"),
  );
  const retry = new CodexAppServer({
    launchSpec: retrySpec,
    spawnProcess: () => {
      spawnCount += 1;
      return new FakeProcess();
    },
  });
  await assert.rejects(retry.connect(), { code: "CODEX_APP_SERVER_LAUNCH_REJECTED" });
  assert.equal(spawnCount, 0);

  const wrongNamespace = new CodexAppServer({
    launchSpec: replaceWslLaunchOption(
      productionWslLaunchSpec(), "reconnect-of", wslDigest("verifier-receipt", "9"),
    ),
    spawnProcess: () => {
      spawnCount += 1;
      return new FakeProcess();
    },
  });
  await assert.rejects(wrongNamespace.connect(), {
    code: "CODEX_APP_SERVER_LAUNCH_REJECTED",
  });
  assert.equal(spawnCount, 0);

  let ambiguousSpec = replaceWslLaunchOption(productionWslLaunchSpec(), "attempt", "2");
  ambiguousSpec = replaceWslLaunchOption(
    ambiguousSpec, "retry-of", wslDigest("attempt-receipt", "8"),
  );
  ambiguousSpec = replaceWslLaunchOption(
    ambiguousSpec, "reconnect-of", wslDigest("attempt-receipt", "9"),
  );
  const ambiguous = new CodexAppServer({
    launchSpec: ambiguousSpec,
    spawnProcess: () => {
      spawnCount += 1;
      return new FakeProcess();
    },
  });
  await assert.rejects(ambiguous.connect(), { code: "CODEX_APP_SERVER_LAUNCH_REJECTED" });
  assert.equal(spawnCount, 0);
});

test("WSL2 readiness rejects stale, substituted, missing, or exited process markers", async () => {
  const launchSpec = productionWslLaunchSpec();
  const invalidMarkers = [
    { schema: "lattice.wsl2-process-fence/1.0" },
    { unit: `${launchSpec.serviceUnit}.substituted` },
    { execution_environment_ref: wslDigest("execution-environment", "9") },
    { credential_seal_digest: wslDigest("credential-seal", "8") },
    { cgroup_path: `/user.slice/../${launchSpec.serviceUnit}` },
    { attempt: 2, retry_of: wslDigest("attempt-receipt", "7") },
    { unexpected: true },
  ];
  for (const overrides of invalidMarkers) {
    const child = new FakeProcess();
    observeClientMessages(child, (message) => {
      if (message.method !== "initialize") return;
      child.stderr.write(`${JSON.stringify(wslProcessMarker(launchSpec, overrides))}\n`);
      sendServerMessage(child, { id: message.id, result: { platformFamily: "unix" } });
    });
    child.stdin.once("finish", () => {
      child.stderr.end();
      child.exitCode = 0;
      child.emit("exit", 0, null);
    });
    const codex = new CodexAppServer({
      launchSpec,
      spawnProcess: () => child,
      requestTimeoutMs: 1_000,
      lifecycleTimeoutMs: 250,
    });
    await assert.rejects(codex.connect(), { code: "CODEX_APP_SERVER_PROCESS_FENCE_REQUIRED" });
  }

  for (const mode of ["timeout", "exit"]) {
    const child = new FakeProcess();
    observeClientMessages(child, (message) => {
      if (message.method !== "initialize") return;
      sendServerMessage(child, { id: message.id, result: { platformFamily: "unix" } });
      if (mode === "exit") {
        setImmediate(() => {
          child.stderr.end();
          child.exitCode = 73;
          child.emit("exit", 73, null);
        });
      }
    });
    child.stdin.once("finish", () => {
      child.stderr.end();
      if (child.exitCode === null) {
        child.exitCode = 0;
        child.emit("exit", 0, null);
      }
    });
    const codex = new CodexAppServer({
      launchSpec,
      spawnProcess: () => child,
      requestTimeoutMs: 1_000,
      lifecycleTimeoutMs: 20,
    });
    await assert.rejects(codex.connect(), {
      code: mode === "exit"
        ? "CODEX_APP_SERVER_PROCESS_EXITED"
        : "CODEX_APP_SERVER_PROCESS_FENCE_REQUIRED",
    });
  }
});

test("WSL2 close accepts only one exact safe 1.1 subtree receipt after stderr drain", async () => {
  const invalidReceipts = [
    null,
    { schema: "lattice.wsl2-subtree-exit/1.0" },
    { fence: "0".repeat(64) },
    { unit: "lattice-wsl2-substituted.service" },
    { execution_environment_ref: wslDigest("execution-environment", "9") },
    { credential_seal_digest: wslDigest("credential-seal", "8") },
    { cgroup_path: "/user.slice/substituted.service" },
    { zero_descendants: false },
    { credential_seal_intact: false },
    { stdout_bytes: 262145 },
    { stderr_limit_bytes: 131072 },
    { output_bound_exceeded: true },
    { timeout_ms: 2000 },
    { timed_out: true },
    { interrupted: true },
    { attempt: 2, retry_of: wslDigest("attempt-receipt", "7") },
    { reconnect_of: wslDigest("attempt-receipt", "6") },
    { exit_code: -1 },
    { unexpected: true },
  ];
  for (const overrides of invalidReceipts) {
    const launchSpec = productionWslLaunchSpec();
    const marker = wslProcessMarker(launchSpec);
    const child = new FakeProcess();
    let launches = 0;
    observeClientMessages(child, (message) => {
      if (message.method !== "initialize") return;
      child.stderr.write(`${JSON.stringify(marker)}\n`);
      sendServerMessage(child, { id: message.id, result: { platformFamily: "unix" } });
    });
    child.stdin.once("finish", () => {
      if (overrides !== null) {
        child.stderr.write(`${JSON.stringify(wslSubtreeExitReceipt(
          launchSpec, marker, overrides,
        ))}\n`);
      }
      child.stderr.end();
      child.exitCode = 0;
      child.emit("exit", 0, null);
    });
    const codex = new CodexAppServer({
      launchSpec,
      spawnProcess: () => {
        launches += 1;
        return child;
      },
      requestTimeoutMs: 1_000,
      lifecycleTimeoutMs: 250,
    });
    await codex.connect();
    await assert.rejects(codex.close(), { code: "CODEX_APP_SERVER_SUBTREE_EXIT_REQUIRED" });
    if (overrides === null) {
      await assert.rejects(codex.close(), { code: "CODEX_APP_SERVER_SUBTREE_EXIT_REQUIRED" });
      await assert.rejects(codex.connect(), { code: "CODEX_APP_SERVER_SUBTREE_EXIT_REQUIRED" });
      assert.equal(launches, 1, "an unresolved subtree receipt must fence replacement spawn");
    }
  }
});

test("WSL2 process-domain control rejects duplicate markers and exit receipts", async () => {
  for (const duplicate of ["marker", "receipt"]) {
    const launchSpec = productionWslLaunchSpec();
    const marker = wslProcessMarker(launchSpec);
    const receipt = wslSubtreeExitReceipt(launchSpec, marker);
    const child = new FakeProcess();
    observeClientMessages(child, (message) => {
      if (message.method !== "initialize") return;
      child.stderr.write(`${JSON.stringify(marker)}\n`);
      if (duplicate === "marker") child.stderr.write(`${JSON.stringify(marker)}\n`);
      sendServerMessage(child, { id: message.id, result: { platformFamily: "unix" } });
    });
    child.stdin.once("finish", () => {
      if (duplicate === "receipt") {
        child.stderr.write(`${JSON.stringify(receipt)}\n`);
        child.stderr.write(`${JSON.stringify(receipt)}\n`);
      }
      child.stderr.end();
      child.exitCode = 0;
      child.emit("exit", 0, null);
    });
    const codex = new CodexAppServer({
      launchSpec,
      spawnProcess: () => child,
      requestTimeoutMs: 1_000,
      lifecycleTimeoutMs: 250,
    });
    if (duplicate === "marker") {
      await assert.rejects(codex.connect(), {
        code: "CODEX_APP_SERVER_PROCESS_FENCE_REQUIRED",
      });
    } else {
      await codex.connect();
      await assert.rejects(codex.close(), {
        code: "CODEX_APP_SERVER_SUBTREE_EXIT_REQUIRED",
      });
    }
  }
});

function createInitializedConnector(onMessage = () => {}, options = {}) {
  const child = new FakeProcess();
  const messages = [];
  observeClientMessages(child, (message) => {
    messages.push(message);
    if (message.method === "initialize") {
      sendServerMessage(child, {
        id: message.id,
        result: { platformFamily: "windows" },
      });
      return;
    }
    onMessage(message, child);
  });
  const codex = new CodexAppServer({
    codexBin: "codex-test",
    spawnProcess: () => child,
    ...options,
  });
  return { child, codex, messages };
}

test("uses the official JSONL handshake and model listing without starting a turn", async () => {
  const child = new FakeProcess();
  const launches = [];
  let buffered = "";
  child.stdin.on("data", (chunk) => {
    buffered += chunk.toString("utf8");
    for (;;) {
      const newline = buffered.indexOf("\n");
      if (newline < 0) break;
      const message = JSON.parse(buffered.slice(0, newline));
      buffered = buffered.slice(newline + 1);
      if (message.method === "initialize") {
        child.stdout.write(`${JSON.stringify({ id: message.id, result: { platformFamily: "windows" } })}\n`);
      } else if (message.method === "model/list") {
        child.stdout.write(`${JSON.stringify({
          id: message.id,
          result: { data: [{ id: "gpt-5.6-terra" }], nextCursor: null },
        })}\n`);
      }
    }
  });

  const codex = new CodexAppServer({
    spawnProcess(command, args, options) {
      launches.push({ command, args, options });
      return child;
    },
  });
  const models = await codex.listModels();
  assert.equal(models.data[0].id, "gpt-5.6-terra");
  assert.equal(launches.length, 1);
  if (process.platform === "win32") {
    assert.equal(launches[0].command, process.execPath);
    assert.match(launches[0].args[0], /@openai[\\/]codex[\\/]bin[\\/]codex\.js$/iu);
    assert.deepEqual(launches[0].args.slice(1), ["app-server", "--stdio"]);
  } else {
    assert.equal(launches[0].command, "codex");
  }
  await codex.close();
});

test("account readiness is sanitized and bound to the exact App Server generation", async () => {
  const { codex, messages } = createInitializedConnector((message, child) => {
    if (message.method === "account/read") {
      sendServerMessage(child, {
        id: message.id,
        result: {
          account: {
            type: "chatgpt",
            email: "must-not-leave-connector@example.invalid",
            planType: "pro",
          },
          requiresOpenaiAuth: true,
          providerPrivateData: "must-not-leave-connector",
        },
      });
    }
  }, { sessionIdentityFactory: () => "app-server-session:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" });

  const readiness = await codex.readAuthReadiness();

  assert.deepEqual(readiness, {
    schema: "lattice.codex-auth-readiness/1.0",
    ready: true,
    authMode: "chatgpt",
    appServerGeneration: 1,
    appServerSessionId: "app-server-session:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  });
  assert.equal(JSON.stringify(readiness).includes("example.invalid"), false);
  assert.equal(JSON.stringify(readiness).includes("providerPrivateData"), false);
  assert.deepEqual(
    messages.find(({ method }) => method === "account/read")?.params,
    { refreshToken: false },
  );
  await codex.close();
});

test("provider effects fail closed when the exact App Server identity changed", async () => {
  const { codex, messages } = createInitializedConnector((message, child) => {
    if (message.method === "account/read") {
      sendServerMessage(child, {
        id: message.id,
        result: { account: { type: "chatgpt" }, requiresOpenaiAuth: true },
      });
    }
  }, { sessionIdentityFactory: () => "app-server-session:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" });

  const readiness = await codex.readAuthReadiness();
  await assert.rejects(
    codex.startThread({
      cwd: String.raw`C:\workspace`,
      effectIdentity: {
        expectedGeneration: readiness.appServerGeneration + 1,
        expectedSessionId: readiness.appServerSessionId,
      },
    }),
    { code: "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED" },
  );
  await assert.rejects(
    codex.startThread({
      cwd: String.raw`C:\workspace`,
      effectIdentity: {
        expectedGeneration: readiness.appServerGeneration,
        expectedSessionId: "app-server-session:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
      },
    }),
    { code: "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED" },
  );
  assert.equal(
    messages.filter(({ method }) => method === "thread/start").length,
    0,
    "no provider effect may be emitted after identity drift",
  );
  await codex.close();
});

test("concurrent starts share one connection and cannot outrun initialized", async () => {
  const child = new FakeProcess();
  const initializeObserved = deferred();
  const messages = [];
  let initializeRequest;
  let initializeReplied = false;
  let threadNumber = 0;

  observeClientMessages(child, (message) => {
    messages.push(message);
    if (message.method === "initialize") {
      initializeRequest = message;
      initializeObserved.resolve();
    } else if (message.method === "thread/start") {
      threadNumber += 1;
      sendServerMessage(child, {
        id: message.id,
        result: { thread: { id: `thread-${threadNumber}` } },
      });
    }
  });

  const launches = [];
  const codex = new CodexAppServer({
    codexBin: "codex-test",
    spawnProcess(command, args, options) {
      launches.push({ command, args, options });
      return child;
    },
  });
  const starts = [
    codex.startThread({ cwd: "C:\\workspace-a" }),
    codex.startThread({ cwd: "C:\\workspace-b" }),
  ];

  try {
    await initializeObserved.promise;
    await nextMacrotask();

    assert.equal(launches.length, 1);
    assert.equal(messages.filter(({ method }) => method === "initialize").length, 1);
    assert.equal(
      messages.filter(({ method }) => method === "thread/start").length,
      0,
      "thread/start must wait until initialize has completed and initialized was sent",
    );

    initializeReplied = true;
    sendServerMessage(child, {
      id: initializeRequest.id,
      result: { platformFamily: "windows" },
    });
    const threads = await Promise.all(starts);
    assert.deepEqual(threads.map(({ id }) => id), ["thread-1", "thread-2"]);

    const initializedIndex = messages.findIndex(({ method }) => method === "initialized");
    const threadStartIndexes = messages
      .map(({ method }, index) => method === "thread/start" ? index : -1)
      .filter((index) => index >= 0);
    assert.ok(initializedIndex >= 0);
    assert.ok(threadStartIndexes.every((index) => index > initializedIndex));
  } finally {
    if (initializeRequest && !initializeReplied) {
      sendServerMessage(child, {
        id: initializeRequest.id,
        result: { platformFamily: "windows" },
      });
    }
    await Promise.allSettled(starts);
    await codex.close();
  }
});

test("a timed out RPC is rejected and removed from the public pending count", async () => {
  const child = new FakeProcess();
  let modelRequest;
  observeClientMessages(child, (message) => {
    if (message.method === "initialize") {
      sendServerMessage(child, {
        id: message.id,
        result: { platformFamily: "windows" },
      });
    } else if (message.method === "model/list") {
      modelRequest = message;
    }
  });

  const codex = new CodexAppServer({
    codexBin: "codex-test",
    requestTimeoutMs: 20,
    spawnProcess: () => child,
  });
  const models = codex.listModels();
  let guardTimer;
  const guard = new Promise((resolve, reject) => {
    guardTimer = setTimeout(
      () => reject(new Error("test guard expired before the connector rejected the RPC")),
      250,
    );
  });

  try {
    await assert.rejects(
      Promise.race([models, guard]),
      /(?:model\/list.*timed out|timed out.*model\/list)/iu,
    );
    clearTimeout(guardTimer);
    assert.equal(codex.pendingRequestCount, 0);

    sendServerMessage(child, {
      id: modelRequest.id,
      result: { data: [], nextCursor: null },
    });
    await nextMacrotask();
    assert.equal(codex.pendingRequestCount, 0, "a late reply must not restore timed-out state");
  } finally {
    clearTimeout(guardTimer);
    await codex.close();
    await Promise.allSettled([models]);
  }
});

test("a numeric App Server RPC rejection keeps a safe connector code and exact method", async () => {
  const { child, codex } = createInitializedConnector((message, server) => {
    if (message.method === "turn/start") {
      sendServerMessage(server, {
        id: message.id,
        error: {
          code: -32602,
          message: "UNTRUSTED_PROVIDER_MESSAGE_SENTINEL",
          data: { unsafe: "UNTRUSTED_PROVIDER_DATA_SENTINEL" },
        },
      });
    }
  });

  try {
    await assert.rejects(
      codex.startTurn("thread-rpc-rejected", "UNTRUSTED_PROMPT_SENTINEL"),
      (error) => {
        assert.equal(error.code, "CODEX_APP_SERVER_RPC_REJECTED");
        assert.equal(error.rpcCode, -32602);
        assert.equal(error.method, "turn/start");
        assert.equal(error.requestId, 2);
        return true;
      },
    );
    assert.equal(codex.pendingRequestCount, 0);
  } finally {
    await codex.close();
    child.destroy?.();
  }
});

test("accepted starts correlate exact started notifications before or after the RPC reply", async () => {
  const child = new FakeProcess();
  let turnRequest;
  observeClientMessages(child, (message) => {
    if (message.method === "initialize") {
      sendServerMessage(child, {
        id: message.id,
        result: { platformFamily: "windows" },
      });
    } else if (message.method === "thread/start") {
      sendServerMessage(child, {
        method: "thread/started",
        params: { thread: { id: "thread-ready" } },
      });
      sendServerMessage(child, {
        id: message.id,
        result: { thread: { id: "thread-ready" } },
      });
    } else if (message.method === "turn/start") {
      turnRequest = message;
      sendServerMessage(child, {
        id: message.id,
        result: { turn: { id: "turn-ready", status: "inProgress" } },
      });
    }
  });

  const codex = new CodexAppServer({
    codexBin: "codex-test",
    spawnProcess: () => child,
  });

  try {
    const thread = await codex.startThread({ cwd: "C:\\workspace" });
    await codex.waitForThreadStarted(thread.id, { timeoutMs: 200 });

    const turn = await codex.startTurn(thread.id, "Run the focused check.");
    assert.equal(turnRequest.params.threadId, thread.id);
    let turnStartedSettled = false;
    const turnStarted = codex.waitForTurnStarted(thread.id, turn.id, { timeoutMs: 200 });
    turnStarted.then(
      () => { turnStartedSettled = true; },
      () => { turnStartedSettled = true; },
    );

    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "other-thread", turn: { id: turn.id, status: "inProgress" } },
    });
    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: thread.id, turn: { id: "other-turn", status: "inProgress" } },
    });
    await nextMacrotask();
    assert.equal(turnStartedSettled, false, "unrelated started notifications cannot release the waiter");

    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: thread.id, turn: { id: turn.id, status: "inProgress" } },
    });
    await turnStarted;
  } finally {
    await codex.close();
  }
});

test("interrupt is fail-closed until the exact turn reports inProgress", async () => {
  const { child, codex, messages } = createInitializedConnector();
  try {
    await codex.connect();
    assert.equal(codex.isTurnActive("thread-active", "turn-active"), false);

    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "thread-active", turn: { id: "turn-active", status: "completed" } },
    });
    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "other-thread", turn: { id: "turn-active", status: "inProgress" } },
    });
    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "thread-active", turn: { id: "other-turn", status: "inProgress" } },
    });
    await nextMacrotask();
    assert.equal(codex.isTurnActive("thread-active", "turn-active"), false);
    await assert.rejects(
      codex.interruptTurn("thread-active", "turn-active", { timeoutMs: 100 }),
      /turn.*not active|no active turn/iu,
    );
    assert.equal(messages.some(({ method }) => method === "turn/interrupt"), false);

    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "thread-active", turn: { id: "turn-active", status: "inProgress" } },
    });
    await nextMacrotask();
    assert.equal(codex.isTurnActive("thread-active", "turn-active"), true);
  } finally {
    await codex.close();
  }
});

test("interrupt waits past RPC acceptance for the exact interrupted or failed terminal", async () => {
  const interruptRequests = [];
  const { child, codex } = createInitializedConnector((message, server) => {
    if (message.method !== "turn/interrupt") return;
    interruptRequests.push(message);
    sendServerMessage(server, { id: message.id, result: {} });
  });

  try {
    await codex.connect();
    for (const [index, terminalStatus] of ["interrupted", "failed"].entries()) {
      const threadId = `thread-${index}`;
      const turnId = `turn-${index}`;
      sendServerMessage(child, {
        method: "turn/started",
        params: { threadId, turn: { id: turnId, status: "inProgress" } },
      });
      await nextMacrotask();
      assert.equal(codex.isTurnActive(threadId, turnId), true);

      let settled = false;
      const interrupted = codex.interruptTurn(threadId, turnId, { timeoutMs: 200 });
      interrupted.then(
        () => { settled = true; },
        () => { settled = true; },
      );
      await nextMacrotask();
      assert.equal(settled, false, "the turn/interrupt RPC result is not a terminal event");

      if (index === 0) {
        sendServerMessage(child, {
          method: "turn/completed",
          params: { threadId: "other-thread", turn: { id: turnId, status: terminalStatus } },
        });
        sendServerMessage(child, {
          method: "turn/completed",
          params: { threadId, turn: { id: "other-turn", status: terminalStatus } },
        });
        sendServerMessage(child, {
          method: "turn/completed",
          params: { threadId, turn: { id: turnId, status: "completed" } },
        });
        await nextMacrotask();
        assert.equal(settled, false, "wrong IDs or a non-interrupt terminal cannot release the waiter");
      }

      sendServerMessage(child, {
        method: "turn/completed",
        params: { threadId, turn: { id: turnId, status: terminalStatus } },
      });
      await interrupted;
      assert.equal(codex.isTurnActive(threadId, turnId), false);
      assert.equal(codex.pendingNotificationCount, 0);
      assert.equal(child.killCount, 0, "a correlated terminal must not kill the App Server");
    }

    assert.deepEqual(
      interruptRequests.map(({ params }) => params),
      [
        { threadId: "thread-0", turnId: "turn-0" },
        { threadId: "thread-1", turnId: "turn-1" },
      ],
    );
  } finally {
    await codex.close();
  }
});

test("interrupt timeout clears its waiter and only then kills the owned process", async () => {
  const interruptObserved = deferred();
  const { child, codex } = createInitializedConnector((message, server) => {
    if (message.method !== "turn/interrupt") return;
    sendServerMessage(server, { id: message.id, result: {} });
    interruptObserved.resolve();
  });
  let interrupted;
  let guardTimer;

  try {
    await codex.connect();
    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "thread-timeout", turn: { id: "turn-timeout", status: "inProgress" } },
    });
    await nextMacrotask();

    interrupted = codex.interruptTurn("thread-timeout", "turn-timeout", { timeoutMs: 20 });
    await interruptObserved.promise;
    await nextMacrotask();
    assert.equal(child.killCount, 0, "RPC acceptance alone must not kill the App Server");

    const guard = new Promise((resolve, reject) => {
      guardTimer = setTimeout(
        () => reject(new Error("test guard expired before interrupt timeout cleanup")),
        250,
      );
    });
    await assert.rejects(
      Promise.race([interrupted, guard]),
      /(?:interrupt|turn\/completed).*timed out/iu,
    );
    clearTimeout(guardTimer);
    assert.equal(codex.pendingNotificationCount, 0);
    assert.equal(codex.isTurnActive("thread-timeout", "turn-timeout"), false);
    assert.equal(child.killCount, 1);
  } finally {
    clearTimeout(guardTimer);
    await codex.close();
    if (interrupted) await Promise.allSettled([interrupted]);
  }
});

test("an exact interrupt terminal wins over a lost RPC acknowledgement without killing peers", async () => {
  const interruptObserved = deferred();
  const { child, codex } = createInitializedConnector((message) => {
    if (message.method === "turn/interrupt") interruptObserved.resolve();
  });
  try {
    await codex.connect();
    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "thread-terminal-first", turn: { id: "turn-terminal-first", status: "inProgress" } },
    });
    await nextMacrotask();

    const interrupting = codex.interruptTurn(
      "thread-terminal-first",
      "turn-terminal-first",
      { timeoutMs: 30 },
    );
    await interruptObserved.promise;
    sendServerMessage(child, {
      method: "turn/completed",
      params: {
        threadId: "thread-terminal-first",
        turn: { id: "turn-terminal-first", status: "interrupted" },
      },
    });

    const terminal = await interrupting;
    assert.equal(terminal.status, "interrupted");
    assert.equal(codex.pendingRequestCount, 0);
    assert.equal(codex.pendingNotificationCount, 0);
    assert.equal(child.killCount, 0);
  } finally {
    await codex.close();
  }
});

test("readThread returns only an exact thread with non-empty turns", async () => {
  const { codex } = createInitializedConnector((message, server) => {
    if (message.method !== "thread/read") return;
    const { threadId, includeTurns } = message.params;
    assert.equal(includeTurns, true);
    const responses = {
      "thread-valid": { id: "thread-valid", turns: [{ id: "turn-done", status: "completed" }] },
      "thread-mismatch": { id: "other-thread", turns: [{ id: "turn-done", status: "completed" }] },
      "thread-missing-turns": { id: "thread-missing-turns" },
      "thread-empty": { id: "thread-empty", turns: [] },
    };
    sendServerMessage(server, { id: message.id, result: { thread: responses[threadId] } });
  });

  try {
    assert.deepEqual(
      await codex.readThread("thread-valid", { includeTurns: true }),
      { id: "thread-valid", turns: [{ id: "turn-done", status: "completed" }] },
    );
    for (const threadId of ["thread-mismatch", "thread-missing-turns", "thread-empty"]) {
      await assert.rejects(
        codex.readThread(threadId, { includeTurns: true }),
        /not recoverable|reconciliation|empty rollout/iu,
      );
    }
  } finally {
    await codex.close();
  }
});

test("thread/list uses the closed dispatch filters and validates its bounded page", async () => {
  const cwd = "C:\\disposable\\managed";
  const { codex, messages } = createInitializedConnector((message, server) => {
    if (message.method !== "thread/list") return;
    sendServerMessage(server, {
      id: message.id,
      result: {
        data: [{ id: "thread-listed", cwd, createdAt: 1_787_714_825, turns: [] }],
        nextCursor: null,
      },
    });
  });

  try {
    assert.deepEqual(await codex.listThreads({
      cwd,
      sourceKinds: ["appServer"],
      archived: false,
      sortKey: "created_at",
      sortDirection: "desc",
      limit: 100,
      useStateDbOnly: true,
    }), {
      data: [{ id: "thread-listed", cwd, createdAt: 1_787_714_825, turns: [] }],
      nextCursor: null,
    });
    assert.deepEqual(messages.find(({ method }) => method === "thread/list")?.params, {
      cwd,
      cursor: null,
      limit: 100,
      sortKey: "created_at",
      sortDirection: "desc",
      archived: false,
      useStateDbOnly: true,
      sourceKinds: ["appServer"],
    });
  } finally {
    await codex.close();
  }
});

test("empty thread recovery requires the same exact empty rollout twice", async () => {
  const { codex, messages } = createInitializedConnector((message, server) => {
    if (message.method === "thread/resume" || message.method === "thread/read") {
      sendServerMessage(server, {
        id: message.id,
        result: { thread: { id: message.params.threadId, turns: [] } },
      });
    }
  });

  try {
    assert.deepEqual(await codex.resumeEmptyThread("thread-empty-recoverable"), {
      id: "thread-empty-recoverable",
      turns: [],
    });
    assert.deepEqual(
      messages.filter(({ method }) => method === "thread/resume" || method === "thread/read")
        .map(({ method }) => method),
      ["thread/resume", "thread/read"],
    );
  } finally {
    await codex.close();
  }
});

test("resume rejects an empty loaded rollout before reconciliation read", async () => {
  const { codex, messages } = createInitializedConnector((message, server) => {
    if (message.method === "thread/resume") {
      const turns = [];
      sendServerMessage(server, {
        id: message.id,
        result: { thread: { id: message.params.threadId, turns } },
      });
    } else if (message.method === "thread/read") {
      sendServerMessage(server, {
        id: message.id,
        result: { thread: { id: message.params.threadId, turns: [] } },
      });
    }
  });

  try {
    await assert.rejects(
      codex.resumeThread("thread-empty"),
      /not recoverable|reconciliation|empty rollout/iu,
    );
    assert.equal(messages.filter(({ method }) => method === "thread/resume").length, 1);
    assert.equal(messages.some(({ method }) => method === "thread/read"), false);
  } finally {
    await codex.close();
  }
});

test("fresh-process resume reconciles and restores one exact active turn", async () => {
  const persistedThread = {
    id: "thread-active",
    turns: [{ id: "turn-active", status: "inProgress" }],
  };
  const { codex, messages } = createInitializedConnector((message, server) => {
    if (message.method === "thread/resume" || message.method === "thread/read") {
      sendServerMessage(server, { id: message.id, result: { thread: persistedThread } });
    }
  });

  try {
    assert.deepEqual(
      await codex.resumeThread(persistedThread.id, { expectedTurnId: "turn-active" }),
      persistedThread,
    );
    assert.equal(codex.isTurnActive("thread-active", "turn-active"), true);
    assert.equal(messages.filter(({ method }) => method === "thread/resume").length, 1);
    assert.equal(messages.filter(({ method }) => method === "thread/read").length, 1);
  } finally {
    await codex.close();
  }
});

test("fresh-process resume loads terminal history before an exact reconciliation read", async () => {
  const persistedThread = {
    id: "thread-resumable",
    turns: [{ id: "turn-completed", status: "completed" }],
  };
  const { codex, messages } = createInitializedConnector((message, server) => {
    if (message.method === "thread/resume") {
      sendServerMessage(server, { id: message.id, result: { thread: persistedThread } });
    } else if (message.method === "thread/read") {
      sendServerMessage(server, { id: message.id, result: { thread: persistedThread } });
    }
  });

  try {
    assert.deepEqual(await codex.resumeThread(persistedThread.id), persistedThread);
    assert.deepEqual(
      messages.map(({ method }) => method),
      ["initialize", "initialized", "thread/resume", "thread/read"],
    );
    assert.deepEqual(messages[2].params, {
      threadId: persistedThread.id,
    });
    assert.deepEqual(messages[3].params, {
      threadId: persistedThread.id,
      includeTurns: true,
    });
  } finally {
    await codex.close();
  }
});

test("an unhandled server request receives an immediate method-not-found response", async () => {
  const { child, codex, messages } = createInitializedConnector();
  try {
    await codex.connect();
    sendServerMessage(child, {
      id: 701,
      method: "unknown/server/request",
      params: { value: "untrusted" },
    });
    await nextMacrotask();

    const response = messages.find((message) => message.id === 701 && !message.method);
    assert.equal(response?.error?.code, -32601);
    assert.match(response?.error?.message ?? "", /method not found|unsupported|unknown/iu);
    assert.equal(codex.pendingServerRequestCount, 0);
  } finally {
    await codex.close();
  }
});

test("a handler can explicitly defer and then resolve a server request", async () => {
  const { child, codex, messages } = createInitializedConnector();
  codex.on("serverRequest", (request) => {
    codex.deferServerRequest(request.id, { timeoutMs: 200 });
  });

  try {
    await codex.connect();
    sendServerMessage(child, {
      id: 702,
      method: "item/commandExecution/requestApproval",
      params: { reason: "Run focused tests" },
    });
    await nextMacrotask();
    assert.equal(codex.pendingServerRequestCount, 1);
    assert.equal(messages.some((message) => message.id === 702 && !message.method), false);

    codex.respond(702, { decision: "accept" });
    await nextMacrotask();
    assert.deepEqual(
      messages.find((message) => message.id === 702 && !message.method),
      { id: 702, result: { decision: "accept" } },
    );
    assert.equal(codex.pendingServerRequestCount, 0);
  } finally {
    await codex.close();
  }
});

test("a deferred server request times out with an explicit error and no leaked state", async () => {
  const timedOut = deferred();
  const { child, codex } = createInitializedConnector((message) => {
    if (message.id === 703 && message.error) timedOut.resolve(message);
  });
  codex.on("serverRequest", (request) => {
    codex.deferServerRequest(request.id, { timeoutMs: 20 });
  });
  let guardTimer;

  try {
    await codex.connect();
    sendServerMessage(child, {
      id: 703,
      method: "item/fileChange/requestApproval",
      params: {},
    });
    const guard = new Promise((resolve, reject) => {
      guardTimer = setTimeout(
        () => reject(new Error("test guard expired before deferred request timeout")),
        250,
      );
    });
    const response = await Promise.race([timedOut.promise, guard]);
    clearTimeout(guardTimer);

    assert.equal(response.id, 703);
    assert.ok(Number.isInteger(response.error.code));
    assert.match(response.error.message, /timed out|timeout/iu);
    assert.equal(codex.pendingServerRequestCount, 0);
    assert.equal(codex.listenerCount("serverRequest"), 1);
  } finally {
    clearTimeout(guardTimer);
    await codex.close();
  }
});

test("closing the connector clears deferred server requests", async () => {
  const { child, codex } = createInitializedConnector();
  codex.on("serverRequest", (request) => {
    codex.deferServerRequest(request.id, { timeoutMs: 5_000 });
  });

  await codex.connect();
  sendServerMessage(child, {
    id: 704,
    method: "item/tool/requestUserInput",
    params: {},
  });
  await nextMacrotask();
  assert.equal(codex.pendingServerRequestCount, 1);

  await codex.close();
  assert.equal(codex.pendingServerRequestCount, 0);
});

test("owned close emits one disconnect so shared work can fail closed", async () => {
  const { codex } = createInitializedConnector();
  const disconnects = [];
  codex.on("disconnect", (details) => disconnects.push(details));

  await codex.connect();
  await codex.close();
  await nextMacrotask();

  assert.deepEqual(disconnects, [{ code: null, signal: "client-close" }]);
});

test("unexpected App Server exit and transport error reject active waits with typed transport codes", async () => {
  for (const failure of [
    { kind: "exit", expected: "CODEX_APP_SERVER_PROCESS_EXITED" },
    { kind: "error", expected: "CODEX_APP_SERVER_TRANSPORT_ERROR" },
  ]) {
    const { child, codex } = createInitializedConnector();
    await codex.connect();
    const terminal = codex.waitForTurnCompleted("thread-exact", "turn-exact", {
      timeoutMs: 1_000,
    });
    if (failure.kind === "exit") {
      child.exitCode = 91;
      child.emit("exit", 91, null);
    } else {
      const cause = new Error("simulated stdio transport failure");
      cause.code = "EPIPE";
      child.emit("error", cause);
    }
    await assert.rejects(terminal, (error) => error.code === failure.expected);
    assert.equal(codex.connected, false);
  }
});

test("owned close resolves only after the exact App Server process exits", async () => {
  const child = new FakeProcess();
  child.pid = 4242;
  child.kill = function kill() {
    this.killCount += 1;
    setTimeout(() => {
      this.exitCode = 0;
      this.emit("exit", 0, null);
    }, 25);
    return true;
  };
  const codex = new CodexAppServer({
    codexBin: "codex-test",
    lifecycleTimeoutMs: 250,
    spawnProcess: () => child,
  });
  observeClientMessages(child, (message) => {
    if (message.method === "initialize") {
      sendServerMessage(child, { id: message.id, result: { platformFamily: "windows" } });
    }
  });

  await codex.connect();
  let resolved = false;
  const closing = codex.close().then((receipt) => {
    resolved = true;
    return receipt;
  });
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.equal(resolved, false);
  assert.equal(child.exitCode, null);
  const receipt = await closing;
  assert.deepEqual(receipt, {
    exited: true,
    processId: 4242,
    code: 0,
    signal: null,
  });
  assert.equal(child.killCount, 1);
  assert.equal(codex.connected, false);
});

test("owned close fails closed when the App Server exit cannot be observed", async () => {
  const child = new FakeProcess();
  let launches = 0;
  child.pid = 4343;
  child.kill = function kill() {
    this.killCount += 1;
    return true;
  };
  const codex = new CodexAppServer({
    codexBin: "codex-test",
    lifecycleTimeoutMs: 20,
    spawnProcess: () => {
      launches += 1;
      return child;
    },
  });
  observeClientMessages(child, (message) => {
    if (message.method === "initialize") {
      sendServerMessage(child, { id: message.id, result: { platformFamily: "windows" } });
    }
  });

  await codex.connect();
  await assert.rejects(
    codex.close(),
    (error) => error.code === "CODEX_APP_SERVER_CLOSE_TIMEOUT",
  );
  assert.equal(child.exitCode, null);
  assert.equal(child.killCount, 1);
  await assert.rejects(
    codex.connect(),
    (error) => error.code === "CODEX_APP_SERVER_CLOSE_PENDING",
  );
  assert.equal(launches, 1, "a live unclosed child must fence every replacement spawn");
  child.exitCode = 0;
  child.emit("exit", 0, null);
  await nextMacrotask();
});

test("transport error without exact exit cannot turn a repeated close into a false receipt", async () => {
  const child = new FakeProcess();
  let launches = 0;
  child.pid = 4444;
  child.kill = function kill() {
    this.killCount += 1;
    const cause = new Error("simulated kill transport error");
    cause.code = "EIO";
    queueMicrotask(() => this.emit("error", cause));
    return false;
  };
  const codex = new CodexAppServer({
    codexBin: "codex-test",
    lifecycleTimeoutMs: 20,
    spawnProcess: () => {
      launches += 1;
      return child;
    },
  });
  observeClientMessages(child, (message) => {
    if (message.method === "initialize") {
      sendServerMessage(child, { id: message.id, result: { platformFamily: "windows" } });
    }
  });

  await codex.connect();
  await assert.rejects(codex.close(), (error) => error.code === "CODEX_APP_SERVER_CLOSE_TIMEOUT");
  await assert.rejects(codex.close(), (error) => error.code === "CODEX_APP_SERVER_CLOSE_TIMEOUT");
  await assert.rejects(codex.connect(), (error) => error.code === "CODEX_APP_SERVER_CLOSE_PENDING");
  assert.equal(child.exitCode, null);
  assert.equal(launches, 1);

  child.exitCode = 0;
  child.emit("exit", 0, null);
  await nextMacrotask();
});

test("late output from an exited generation cannot settle or activate a reconnect", async () => {
  const children = [new FakeProcess(), new FakeProcess()];
  for (const child of children) {
    observeClientMessages(child, (message) => {
      if (message.method === "initialize") {
        sendServerMessage(child, { id: message.id, result: { platformFamily: "windows" } });
      } else if (message.method === "model/list") {
        sendServerMessage(child, { id: message.id, result: { data: [], nextCursor: null } });
      }
    });
  }
  let launch = 0;
  const codex = new CodexAppServer({
    codexBin: "codex-test",
    spawnProcess: () => children[launch++],
  });

  try {
    await codex.connect();
    children[0].exitCode = 1;
    children[0].emit("exit", 1, null);
    await nextMacrotask();
    await codex.connect();

    sendServerMessage(children[0], {
      method: "turn/started",
      params: {
        threadId: "stale-thread",
        turn: { id: "stale-turn", status: "inProgress" },
      },
    });
    await codex.listModels();
    await nextMacrotask();

    assert.equal(launch, 2);
    assert.equal(codex.connected, true);
    assert.equal(codex.isTurnActive("stale-thread", "stale-turn"), false);
    assert.equal(codex.notificationSnapshot({ threadId: "stale-thread" }).length, 0);
  } finally {
    await codex.close();
  }
});
