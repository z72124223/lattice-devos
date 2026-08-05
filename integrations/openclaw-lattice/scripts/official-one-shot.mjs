import { createHash, createHmac, randomBytes } from "node:crypto";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer, createConnection } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const OFFICIAL = Object.freeze({
  bin: "openclaw.mjs",
  commit: "0790d9f593ad30c940ed93b5872a8cf6d6f3cf8c",
  integrity:
    "sha512-ycF3yPcbjN6bUPeaUx6Mh6vze1hQWoD3CT/wWcmD7a8xaHHHRUaAlaq+lFxMHf1ssEgODVAwjlzYqp2twkYZ7g==",
  license: "MIT",
  main: "dist/index.js",
  packageName: "openclaw",
  sdkExport: "./plugin-sdk/plugin-entry",
  version: "2026.7.1-2",
});
const LAUNCH_ATTESTATION_DOMAIN = "lattice-openclaw-launch-attestation-v1";
const PLUGIN_ID = "lattice-devos";
const SESSION_KEY = "agent:main:main";
const MAX_CAPTURE_BYTES = 4 * 1024 * 1024;

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const pluginRoot = path.resolve(scriptDirectory, "..");
const repositoryRoot = path.resolve(pluginRoot, "..", "..");
const temporaryRoot = await mkdtemp(path.join(tmpdir(), "lattice-openclaw-live-"));
const evidence = {
  officialPackage: undefined,
  pluginInspect: undefined,
  rustComplete: undefined,
  runtimeStatus: undefined,
  temporaryRoot,
};
let gatewayChild;
let gatewayPort;
let latticePort;
let rustChild;
let succeeded = false;

try {
  const tarball = await obtainAndVerifyTarball(temporaryRoot);
  const officialPackageRoot =
    process.env.LATTICE_OPENCLAW_OFFICIAL_PACKAGE_ROOT === undefined
      ? await installOfficialPackage(temporaryRoot, tarball.path)
      : path.resolve(process.env.LATTICE_OPENCLAW_OFFICIAL_PACKAGE_ROOT);
  const packageEvidence = await verifyOfficialPackage(officialPackageRoot, tarball.sha256);
  evidence.officialPackage = packageEvidence;

  gatewayPort = await freeLoopbackPort();
  latticePort = await freeLoopbackPort();
  while (latticePort === gatewayPort) {
    latticePort = await freeLoopbackPort();
  }
  const gatewayToken = randomBytes(32).toString("hex");
  const transportKey = randomBytes(32).toString("hex");
  const processStartNonce = randomBytes(16).toString("hex");
  const launchRecordId = `lattice-${randomBytes(12).toString("hex")}`;
  const attestationKey = randomBytes(32);

  const profileRoot = path.join(temporaryRoot, "profile");
  const configPath = path.join(profileRoot, "openclaw.json");
  await mkdir(path.join(profileRoot, "appdata"), { recursive: true });
  await mkdir(path.join(profileRoot, "localappdata"), { recursive: true });
  const profileBytes = Buffer.from(
    `${JSON.stringify(
      {
        agents: {
          defaults: {
            heartbeat: { every: "0m" },
            memorySearch: { enabled: false },
          },
        },
        cron: { enabled: false },
        discovery: { mdns: { mode: "off" } },
        gateway: {
          auth: { mode: "token" },
          bind: "loopback",
          mode: "local",
          tailscale: { mode: "off" },
          terminal: { enabled: false },
        },
        hooks: { enabled: false },
        plugins: {
          allow: [PLUGIN_ID],
          entries: { [PLUGIN_ID]: { enabled: true } },
          load: { paths: [pluginRoot] },
          slots: { memory: "none" },
        },
        update: { auto: { enabled: false }, checkOnStart: false },
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  await writeFile(configPath, profileBytes, { flag: "wx" });
  const profileSha256 = sha256(profileBytes);

  const officialEnvironment = isolatedOpenClawEnvironment({
    configPath,
    gatewayPort: latticePort,
    gatewayToken,
    launchRecordId,
    processStartNonce,
    profileRoot,
    transportKey,
  });
  const cliPath = path.join(officialPackageRoot, OFFICIAL.bin);
  await run(process.execPath, [cliPath, "config", "validate"], {
    cwd: officialPackageRoot,
    env: officialEnvironment,
    timeoutMs: 20_000,
  });

  gatewayChild = spawnCaptured(
    process.execPath,
    [
      cliPath,
      "gateway",
      "run",
      "--bind",
      "loopback",
      "--auth",
      "token",
      "--token",
      gatewayToken,
      "--port",
      String(gatewayPort),
      "--ws-log",
      "compact",
    ],
    { cwd: officialPackageRoot, env: officialEnvironment, name: "official-openclaw" },
  );
  if (gatewayChild.child.pid === undefined || gatewayChild.child.pid === 0) {
    throw new Error("official OpenClaw process did not expose a PID");
  }

  const runtimeStatus = await waitForGatewayStatus({
    cliPath,
    env: officialEnvironment,
    gatewayPort,
    gatewayToken,
    packageRoot: officialPackageRoot,
  });
  if (runtimeStatus.runtimeVersion !== OFFICIAL.version) {
    throw new Error(`official gateway reported unexpected runtimeVersion ${String(runtimeStatus.runtimeVersion)}`);
  }
  evidence.runtimeStatus = runtimeStatus;

  const attestationTag = launchAttestationTag(attestationKey, {
    entrypointDigest: packageEvidence.entrypointSha256,
    isolatedProfileDigest: profileSha256,
    launchRecordId,
    packageTarballDigest: tarball.sha256,
    processId: gatewayChild.child.pid,
    processStartNonce,
  });
  await buildRustCanary();
  const rustExecutable = await rustExampleExecutable();
  rustChild = spawnCaptured(rustExecutable, [], {
    cwd: repositoryRoot,
    env: {
      ...process.env,
      LATTICE_OPENCLAW_AUTH_KEY_HEX: transportKey,
      LATTICE_OPENCLAW_DEADLINE_MS: "10000",
      LATTICE_OPENCLAW_ENTRYPOINT_SHA256: packageEvidence.entrypointSha256,
      LATTICE_OPENCLAW_GATEWAY_PORT: String(latticePort),
      LATTICE_OPENCLAW_LAUNCH_ATTESTATION_KEY_HEX: attestationKey.toString("hex"),
      LATTICE_OPENCLAW_LAUNCH_ATTESTATION_TAG_HEX: attestationTag,
      LATTICE_OPENCLAW_LAUNCH_RECORD_ID: launchRecordId,
      LATTICE_OPENCLAW_PACKAGE_TARBALL_SHA256: tarball.sha256,
      LATTICE_OPENCLAW_PROCESS_ID: String(gatewayChild.child.pid),
      LATTICE_OPENCLAW_PROCESS_START_NONCE: processStartNonce,
      LATTICE_OPENCLAW_PROFILE_SHA256: profileSha256,
    },
    name: "lattice-rust-canary",
  });
  const rustReady = await rustChild.waitForJson(
    (value) => value.event === "ready",
    15_000,
  );
  if (
    rustReady.runtime_kind !== "Fake" ||
    rustReady.durability !== "process-memory" ||
    rustReady.launch_record_id !== launchRecordId ||
    typeof rustReady.submit_digest !== "string"
  ) {
    throw new Error("Rust canary emitted an invalid bounded ready record");
  }

  const inspectResult = await run(
    process.execPath,
    [cliPath, "plugins", "inspect", PLUGIN_ID, "--runtime", "--json"],
    {
      cwd: officialPackageRoot,
      env: officialEnvironment,
      timeoutMs: 30_000,
    },
  );
  if (!inspectResult.stdout.includes(PLUGIN_ID) || !inspectResult.stdout.includes("lattice")) {
    throw new Error("official runtime inspect did not expose the lattice plugin command");
  }
  evidence.pluginInspect = parseJsonOutput(inspectResult.stdout);

  const statusReply = await sendSlashCommandAndWait({
    cliPath,
    env: officialEnvironment,
    gatewayPort,
    gatewayToken,
    idempotencyKey: `lattice-status-${randomBytes(12).toString("hex")}`,
    message: "/lattice status project project-a",
    packageRoot: officialPackageRoot,
    responseText: "LATTICE project project-a: 0 task(s)",
  });
  const submitReply = await sendSlashCommandAndWait({
    cliPath,
    env: officialEnvironment,
    gatewayPort,
    gatewayToken,
    idempotencyKey: `lattice-submit-${randomBytes(12).toString("hex")}`,
    message: `/lattice submit ${rustReady.submit_digest}`,
    packageRoot: officialPackageRoot,
    responseText: "LATTICE submit accepted for task-a",
  });

  const rustComplete = await rustChild.waitForJson(
    (value) => value.event === "complete",
    15_000,
  );
  const expectedObservations = [
    "status:project-a",
    `submit:${rustReady.submit_digest}`,
  ];
  if (
    rustComplete.runtime_kind !== "Fake" ||
    rustComplete.durability !== "process-memory" ||
    JSON.stringify(rustComplete.observations) !== JSON.stringify(expectedObservations)
  ) {
    throw new Error("Rust canary did not record the exact official status and submit sequence");
  }
  evidence.rustComplete = rustComplete;
  await rustChild.waitForExit(5_000, true);

  succeeded = true;
  process.stdout.write(
    `${JSON.stringify({
      classification: "official-openclaw-lattice-one-shot-live",
      durability: "process-memory",
      event: "green",
      gateway_rpc_runtime_version: runtimeStatus.runtimeVersion,
      official_package: `${OFFICIAL.packageName}@${OFFICIAL.version}`,
      official_process_id: gatewayChild.child.pid,
      plugin_id: PLUGIN_ID,
      restart_safe: false,
      runtime_kind: "Fake",
      rust_observations: rustComplete.observations,
      status_reply_observed: statusReply,
      submit_reply_observed: submitReply,
      transport_live: true,
    })}\n`,
  );
} catch (error) {
  process.stderr.write(
    `${JSON.stringify({
      classification: "official-openclaw-lattice-one-shot-failed",
      error: error instanceof Error ? error.message : String(error),
      evidence,
      gateway_stderr: gatewayChild?.stderrText(),
      gateway_stdout: gatewayChild?.stdoutText(),
      rust_stderr: rustChild?.stderrText(),
      rust_stdout: rustChild?.stdoutText(),
      temporaryRoot,
    })}\n`,
  );
  process.exitCode = 1;
} finally {
  await stopCaptured(rustChild);
  await stopCaptured(gatewayChild);
  if (succeeded) {
    await assertPortClosed(latticePort);
    await assertPortClosed(gatewayPort);
    await rm(temporaryRoot, { force: true, recursive: true });
  }
}

async function obtainAndVerifyTarball(root) {
  const supplied = process.env.LATTICE_OPENCLAW_OFFICIAL_TARBALL_PATH;
  let tarballPath;
  if (supplied !== undefined) {
    tarballPath = path.resolve(supplied);
  } else {
    const npmCli = process.env.npm_execpath;
    if (npmCli === undefined) {
      throw new Error("npm_execpath is required when no verified official tarball is supplied");
    }
    const packed = await run(
      process.execPath,
      [
        npmCli,
        "pack",
        `${OFFICIAL.packageName}@${OFFICIAL.version}`,
        "--ignore-scripts",
        "--json",
        "--pack-destination",
        root,
      ],
      { cwd: root, env: process.env, timeoutMs: 120_000 },
    );
    const report = parseJsonOutput(packed.stdout);
    const entry = Array.isArray(report) ? report[0] : undefined;
    if (entry === undefined || entry.integrity !== OFFICIAL.integrity || typeof entry.filename !== "string") {
      throw new Error("npm pack did not return the exact official integrity pin");
    }
    tarballPath = path.join(root, entry.filename);
  }
  const bytes = await readFile(tarballPath);
  const integrity = `sha512-${createHash("sha512").update(bytes).digest("base64")}`;
  if (integrity !== OFFICIAL.integrity) {
    throw new Error("official OpenClaw tarball integrity mismatch");
  }
  return { path: tarballPath, sha256: sha256(bytes) };
}

async function installOfficialPackage(root, tarballPath) {
  const npmCli = process.env.npm_execpath;
  if (npmCli === undefined) {
    throw new Error("npm_execpath is required for isolated official package installation");
  }
  const installRoot = path.join(root, "install");
  await mkdir(installRoot, { recursive: true });
  await writeFile(
    path.join(installRoot, "package.json"),
    '{"name":"lattice-openclaw-isolated-live","private":true,"version":"0.0.0"}\n',
    { flag: "wx" },
  );
  await run(
    process.execPath,
    [
      npmCli,
      "install",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      "--omit=dev",
      "--save-exact",
      tarballPath,
    ],
    { cwd: installRoot, env: process.env, timeoutMs: 240_000 },
  );
  return path.join(installRoot, "node_modules", OFFICIAL.packageName);
}

async function verifyOfficialPackage(packageRoot, packageTarballSha256) {
  const metadata = JSON.parse(await readFile(path.join(packageRoot, "package.json"), "utf8"));
  const bin = typeof metadata.bin === "string" ? metadata.bin : metadata.bin?.openclaw;
  if (
    metadata.name !== OFFICIAL.packageName ||
    metadata.version !== OFFICIAL.version ||
    metadata.license !== OFFICIAL.license ||
    metadata.main !== OFFICIAL.main ||
    bin !== OFFICIAL.bin ||
    metadata.exports === undefined ||
    !Object.hasOwn(metadata.exports, OFFICIAL.sdkExport)
  ) {
    throw new Error("isolated official package metadata does not match the frozen pin");
  }
  const entrypointPath = path.resolve(packageRoot, OFFICIAL.bin);
  if (!entrypointPath.startsWith(`${path.resolve(packageRoot)}${path.sep}`)) {
    throw new Error("official entrypoint escaped the isolated package root");
  }
  const entrypointSha256 = sha256(await readFile(entrypointPath));
  const version = await run(process.execPath, [entrypointPath, "--version"], {
    cwd: packageRoot,
    env: process.env,
    timeoutMs: 20_000,
  });
  if (!version.stdout.includes(`OpenClaw ${OFFICIAL.version} (${OFFICIAL.commit.slice(0, 7)})`)) {
    throw new Error("official CLI version/commit output does not match the frozen pin");
  }
  return {
    entrypoint: OFFICIAL.bin,
    entrypointSha256,
    integrity: OFFICIAL.integrity,
    license: OFFICIAL.license,
    packageTarballSha256,
    sourceCommit: OFFICIAL.commit,
    version: OFFICIAL.version,
  };
}

function isolatedOpenClawEnvironment({
  configPath,
  gatewayPort,
  gatewayToken,
  launchRecordId,
  processStartNonce,
  profileRoot,
  transportKey,
}) {
  const environment = {};
  for (const key of ["COMSPEC", "NUMBER_OF_PROCESSORS", "OS", "PATHEXT", "PATH", "SystemRoot", "WINDIR"]) {
    const value = process.env[key] ?? process.env[key.toLowerCase()] ?? process.env[key === "PATH" ? "Path" : key];
    if (value !== undefined) {
      environment[key] = value;
    }
  }
  return {
    ...environment,
    APPDATA: path.join(profileRoot, "appdata"),
    CI: "1",
    HOME: profileRoot,
    LATTICE_OPENCLAW_AUTH_KEY_HEX: transportKey,
    LATTICE_OPENCLAW_DEADLINE_MS: "10000",
    LATTICE_OPENCLAW_GATEWAY_PORT: String(gatewayPort),
    LATTICE_OPENCLAW_LAUNCH_RECORD_ID: launchRecordId,
    LATTICE_OPENCLAW_PROCESS_START_NONCE: processStartNonce,
    LOCALAPPDATA: path.join(profileRoot, "localappdata"),
    NO_COLOR: "1",
    OPENCLAW_CONFIG_PATH: configPath,
    OPENCLAW_DISABLE_BONJOUR: "1",
    OPENCLAW_GATEWAY_TOKEN: gatewayToken,
    OPENCLAW_SKIP_GMAIL_WATCHER: "1",
    OPENCLAW_STATE_DIR: profileRoot,
    USERPROFILE: profileRoot,
  };
}

function launchAttestationTag(key, value) {
  const chunks = [Buffer.from(`${LAUNCH_ATTESTATION_DOMAIN}\0`, "utf8")];
  const fields = [
    ["launch_record_id", Buffer.from(value.launchRecordId, "utf8")],
    ["process_id", u32(value.processId)],
    ["process_start_nonce", Buffer.from(value.processStartNonce, "hex")],
    ["package_name", Buffer.from(OFFICIAL.packageName, "utf8")],
    ["package_version", Buffer.from(OFFICIAL.version, "utf8")],
    ["source_commit", Buffer.from(OFFICIAL.commit, "utf8")],
    ["package_license", Buffer.from(OFFICIAL.license, "utf8")],
    ["package_integrity", Buffer.from(OFFICIAL.integrity, "utf8")],
    ["entrypoint", Buffer.from(OFFICIAL.bin, "utf8")],
    ["package_tarball_digest", Buffer.from(value.packageTarballDigest, "utf8")],
    ["entrypoint_digest", Buffer.from(value.entrypointDigest, "utf8")],
    ["isolated_profile_digest", Buffer.from(value.isolatedProfileDigest, "utf8")],
  ];
  for (const [name, fieldValue] of fields) {
    const nameBytes = Buffer.from(name, "utf8");
    chunks.push(u32(nameBytes.length), nameBytes, u32(fieldValue.length), fieldValue);
  }
  return createHmac("sha256", key).update(Buffer.concat(chunks)).digest("hex");
}

async function buildRustCanary() {
  await run(
    "cargo",
    ["build", "-p", "lattice-openclaw-adapter", "--example", "openclaw_live_preflight", "--locked"],
    { cwd: repositoryRoot, env: process.env, timeoutMs: 120_000 },
  );
}

async function rustExampleExecutable() {
  const metadata = await run(
    "cargo",
    ["metadata", "--no-deps", "--format-version", "1", "--locked"],
    { cwd: repositoryRoot, env: process.env, timeoutMs: 30_000 },
  );
  const targetDirectory = parseJsonOutput(metadata.stdout).target_directory;
  if (typeof targetDirectory !== "string") {
    throw new Error("cargo metadata omitted target_directory");
  }
  return path.join(
    targetDirectory,
    "debug",
    "examples",
    process.platform === "win32" ? "openclaw_live_preflight.exe" : "openclaw_live_preflight",
  );
}

async function waitForGatewayStatus({ cliPath, env, gatewayPort, gatewayToken, packageRoot }) {
  await waitForPortOpen(gatewayPort, 30_000);
  await delay(2_000);
  const result = await run(
    process.execPath,
    [
      cliPath,
      "gateway",
      "call",
      "status",
      "--url",
      `ws://127.0.0.1:${gatewayPort}`,
      "--token",
      gatewayToken,
      "--json",
      "--timeout",
      "60000",
    ],
    { cwd: packageRoot, env, timeoutMs: 90_000 },
  );
  return parseJsonOutput(result.stdout);
}

async function sendSlashCommandAndWait({
  cliPath,
  env,
  gatewayPort,
  gatewayToken,
  idempotencyKey,
  message,
  packageRoot,
  responseText,
}) {
  const connection = [
    "--url",
    `ws://127.0.0.1:${gatewayPort}`,
    "--token",
    gatewayToken,
    "--json",
    "--timeout",
    "30000",
  ];
  await run(
    process.execPath,
    [
      cliPath,
      "gateway",
      "call",
      "chat.send",
      ...connection,
      "--params",
      JSON.stringify({
        deliver: false,
        idempotencyKey,
        message,
        sessionKey: SESSION_KEY,
        timeoutMs: 8_000,
      }),
    ],
    { cwd: packageRoot, env, timeoutMs: 45_000 },
  );
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    const history = await run(
      process.execPath,
      [
        cliPath,
        "gateway",
        "call",
        "chat.history",
        ...connection,
        "--params",
        JSON.stringify({ limit: 20, maxChars: 40_000, sessionKey: SESSION_KEY }),
      ],
      { cwd: packageRoot, env, timeoutMs: 45_000 },
    );
    const historyValue = parseJsonOutput(history.stdout);
    if (JSON.stringify(historyValue).includes(responseText)) {
      return responseText;
    }
    await delay(250);
  }
  throw new Error(`official chat history did not contain ${responseText}`);
}

function spawnCaptured(executable, args, { cwd, env, name }) {
  const child = spawn(executable, args, { cwd, env, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
  let stdout = "";
  let stderr = "";
  let stdoutPending = "";
  const jsonValues = [];
  const jsonWaiters = new Set();
  const append = (current, chunk) => {
    const next = current + chunk.toString("utf8");
    return next.length <= MAX_CAPTURE_BYTES ? next : next.slice(next.length - MAX_CAPTURE_BYTES);
  };
  child.stdout.on("data", (chunk) => {
    stdout = append(stdout, chunk);
    stdoutPending += chunk.toString("utf8");
    for (;;) {
      const newline = stdoutPending.indexOf("\n");
      if (newline < 0) break;
      const line = stdoutPending.slice(0, newline).trim();
      stdoutPending = stdoutPending.slice(newline + 1);
      if (line.length === 0) continue;
      try {
        const value = JSON.parse(line);
        jsonValues.push(value);
        for (const waiter of [...jsonWaiters]) {
          if (waiter.predicate(value)) {
            jsonWaiters.delete(waiter);
            clearTimeout(waiter.timer);
            waiter.resolve(value);
          }
        }
      } catch {
        // Non-JSON runtime logs remain captured for exact failure evidence.
      }
    }
  });
  child.stderr.on("data", (chunk) => {
    stderr = append(stderr, chunk);
  });
  const waitForJson = (predicate, timeoutMs) =>
    new Promise((resolve, reject) => {
      const existing = jsonValues.find(predicate);
      if (existing !== undefined) {
        resolve(existing);
        return;
      }
      const waiter = {
        predicate,
        reject,
        resolve,
        timer: setTimeout(() => {
          jsonWaiters.delete(waiter);
          reject(new Error(`${name} did not emit the required JSON event`));
        }, timeoutMs),
      };
      jsonWaiters.add(waiter);
      if (child.exitCode !== null) {
        clearTimeout(waiter.timer);
        jsonWaiters.delete(waiter);
        reject(new Error(`${name} exited before the required JSON event`));
      }
    });
  child.once("exit", (code, signal) => {
    for (const waiter of jsonWaiters) {
      clearTimeout(waiter.timer);
      waiter.reject(new Error(`${name} exited (${String(code)}/${String(signal)})`));
    }
    jsonWaiters.clear();
  });
  return {
    child,
    stderrText: () => stderr,
    stdoutText: () => stdout,
    waitForExit: (timeoutMs, requireSuccess = false) => waitForExit(child, name, timeoutMs, requireSuccess),
    waitForJson,
  };
}

async function run(executable, args, { cwd, env, timeoutMs }) {
  const captured = spawnCaptured(executable, args, { cwd, env, name: path.basename(executable) });
  await captured.waitForExit(timeoutMs, true);
  return { stderr: captured.stderrText(), stdout: captured.stdoutText() };
}

async function waitForExit(child, name, timeoutMs, requireSuccess) {
  if (child.exitCode === null) {
    let timer;
    await Promise.race([
      once(child, "exit"),
      new Promise((_, reject) => {
        timer = setTimeout(() => {
          child.kill();
          reject(new Error(`${name} timed out`));
        }, timeoutMs);
      }),
    ]).finally(() => clearTimeout(timer));
  }
  if (requireSuccess && child.exitCode !== 0) {
    throw new Error(`${name} exited ${String(child.exitCode)}`);
  }
}

async function stopCaptured(captured) {
  if (captured === undefined || captured.child.exitCode !== null) return;
  captured.child.kill();
  try {
    await captured.waitForExit(5_000, false);
  } catch {
    // The exact child was already asked to terminate; caller retains logs.
  }
}

async function freeLoopbackPort() {
  const server = createServer();
  server.unref();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error("failed to reserve a loopback port");
  }
  await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  return address.port;
}

async function waitForPortOpen(port, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const open = await new Promise((resolve) => {
      const socket = createConnection({ host: "127.0.0.1", port });
      const timer = setTimeout(() => {
        socket.destroy();
        resolve(false);
      }, 1_000);
      socket.once("connect", () => {
        clearTimeout(timer);
        socket.destroy();
        resolve(true);
      });
      socket.once("error", () => {
        clearTimeout(timer);
        resolve(false);
      });
    });
    if (open) return;
    await delay(250);
  }
  throw new Error(`official gateway loopback listener ${port} did not open`);
}

async function assertPortClosed(port) {
  if (!Number.isInteger(port) || port < 1) return;
  await new Promise((resolve, reject) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    const timer = setTimeout(() => {
      socket.destroy();
      reject(new Error(`loopback port ${port} did not close`));
    }, 2_000);
    socket.once("connect", () => {
      clearTimeout(timer);
      socket.destroy();
      reject(new Error(`loopback port ${port} remained open`));
    });
    socket.once("error", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

function parseJsonOutput(stdout) {
  const trimmed = stdout.trim();
  try {
    return JSON.parse(trimmed);
  } catch {
    const start = trimmed.indexOf("{");
    const end = trimmed.lastIndexOf("}");
    if (start >= 0 && end > start) {
      return JSON.parse(trimmed.slice(start, end + 1));
    }
    throw new Error("command did not emit JSON");
  }
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function u32(value) {
  if (!Number.isInteger(value) || value < 0 || value > 0xffff_ffff) {
    throw new Error("value does not fit u32");
  }
  const bytes = Buffer.alloc(4);
  bytes.writeUInt32BE(value);
  return bytes;
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
