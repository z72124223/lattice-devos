import { randomBytes } from "node:crypto";
import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";

import { CodexAppServer } from "../apps/lattice-control/src/codex-app-server.mjs";
import {
  buildWsl2CodexLaunch,
  executionEnvironmentIdentity,
  validateWsl2ExecutionEnvironment,
} from "../apps/lattice-control/src/wsl2-execution-domain.mjs";

function fail(code) {
  process.stdout.write(`${JSON.stringify({
    schema: "lattice.phase4-wsl2-connector-preflight/1.0",
    status: "FAIL",
    code,
    provider_effect_count: 0,
  })}\n`);
  process.exitCode = 1;
}

const exec = promisify(execFile);
const canonical = (value) => Array.isArray(value)
  ? value.map(canonical)
  : value && typeof value === "object"
    ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]))
    : value;
const typedDigest = (prefix, value) => `${prefix}:sha256:${createHash("sha256")
  .update(JSON.stringify(canonical(value)), "utf8").digest("hex")}`;

async function wsl(distribution, executable, args = []) {
  const { stdout } = await exec("C:\\Windows\\System32\\wsl.exe", [
    "-d", distribution, "--exec", executable, ...args,
  ], { encoding: "utf8", timeout: 30_000, windowsHide: true, maxBuffer: 64 * 1024 });
  return stdout.trimEnd();
}

async function observeEnvironment(descriptor) {
  const linux = descriptor.linux;
  const files = [
    [linux.launcher_path, linux.launcher_sha256],
    [linux.node_path, linux.node_sha256],
    [linux.git_path, linux.git_sha256],
    [linux.supervisor_path, linux.supervisor_sha256],
    [linux.dbus_run_session_path, linux.dbus_run_session_sha256],
    [linux.setsid_path, linux.setsid_sha256],
    [linux.keyring_daemon_path, linux.keyring_daemon_sha256],
  ];
  for (const [file, expected] of files) {
    const output = await wsl(descriptor.distribution, "/usr/bin/sha256sum", [file]);
    if (output.split(/\s+/u)[0] !== expected) throw new Error("WSL2_PREFLIGHT_FILE_DIGEST_MISMATCH");
  }
  if (await wsl(descriptor.distribution, linux.launcher_path, ["--version"]) !== linux.launcher_version) {
    throw new Error("WSL2_PREFLIGHT_LAUNCHER_VERSION_MISMATCH");
  }
  if (await wsl(descriptor.distribution, linux.node_path, ["--version"]) !== linux.node_version) {
    throw new Error("WSL2_PREFLIGHT_NODE_VERSION_MISMATCH");
  }
  const gitVersion = await wsl(descriptor.distribution, linux.git_path, ["--version"]);
  if (gitVersion !== linux.git_version) throw new Error("WSL2_PREFLIGHT_GIT_VERSION_MISMATCH");
  const [topLevel, commonDir, head, status, configSha, linuxHeadSha] = await Promise.all([
    wsl(descriptor.distribution, linux.git_path, ["-C", linux.cwd, "rev-parse", "--show-toplevel"]),
    wsl(descriptor.distribution, linux.git_path, ["-C", linux.cwd, "rev-parse", "--git-common-dir"]),
    wsl(descriptor.distribution, linux.git_path, ["-C", linux.cwd, "rev-parse", "HEAD"]),
    wsl(descriptor.distribution, linux.git_path, ["-C", linux.cwd, "status", "--porcelain=v1"]),
    wsl(descriptor.distribution, "/usr/bin/sha256sum", [`${linux.codex_home}/config.toml`]),
    wsl(descriptor.distribution, "/usr/bin/sha256sum", [`${linux.cwd}/.git/HEAD`]),
  ]);
  if (topLevel !== linux.cwd || configSha.split(/\s+/u)[0] !== linux.config_digest.slice(-64)) {
    throw new Error("WSL2_PREFLIGHT_REPOSITORY_OR_HOME_MISMATCH");
  }
  const windowsHead = await readFile(path.win32.join(descriptor.path_mapping.windows_path, ".git", "HEAD"));
  const windowsHeadSha = createHash("sha256").update(windowsHead).digest("hex");
  if (windowsHeadSha !== linuxHeadSha.split(/\s+/u)[0]) throw new Error("WSL2_PREFLIGHT_PATH_MAPPING_MISMATCH");
  const repositoryFacts = {
    distribution: descriptor.distribution,
    cwd: linux.cwd,
    top_level: topLevel,
    common_git_dir: commonDir,
    head,
    status,
    git_path: linux.git_path,
    git_version: gitVersion,
    git_sha256: linux.git_sha256,
  };
  linux.repository_identity = typedDigest("repository", repositoryFacts);
  descriptor.path_mapping.digest = typedDigest("path-mapping", {
    distribution: descriptor.distribution,
    windows_path: descriptor.path_mapping.windows_path,
    linux_path: descriptor.path_mapping.linux_path,
    shared_git_head_sha256: windowsHeadSha,
  });
  descriptor.identity_digest = executionEnvironmentIdentity(descriptor);
  return repositoryFacts;
}

async function main() {
  const encoded = process.env.LATTICE_WSL2_PROBE_DESCRIPTOR;
  if (typeof encoded !== "string" || Buffer.byteLength(encoded, "utf8") > 16_384) {
    fail("WSL2_PREFLIGHT_DESCRIPTOR_REQUIRED");
    return;
  }
  let environment;
  try {
    const descriptor = JSON.parse(encoded);
    await observeEnvironment(descriptor);
    environment = validateWsl2ExecutionEnvironment(descriptor);
  } catch {
    fail("WSL2_PREFLIGHT_DESCRIPTOR_REJECTED");
    return;
  }
  const launch = buildWsl2CodexLaunch(environment, {
    fence: randomBytes(32).toString("hex"),
  });
  const connector = new CodexAppServer({
    launchSpec: launch,
    requestTimeoutMs: 30_000,
    lifecycleTimeoutMs: 30_000,
  });
  try {
    const readiness = await connector.readAuthReadiness();
    const processIdentity = connector.processDomainIdentity;
    const shutdown = await connector.close();
    process.stdout.write(`${JSON.stringify({
      schema: "lattice.phase4-wsl2-connector-preflight/1.0",
      status: readiness.ready ? "PASS" : "AUTH_REQUIRED",
      execution_environment_ref: environment.identity_digest,
      distribution: environment.distribution,
      launcher_version: environment.linux.launcher_version,
      launcher_sha256: environment.linux.launcher_sha256,
      linux_codex_home: environment.linux.codex_home,
      linux_cwd: environment.linux.cwd,
      auth_mode: readiness.authMode,
      auth_ready: readiness.ready,
      app_server_generation: readiness.appServerGeneration,
      app_server_session_id: readiness.appServerSessionId,
      process_domain_identity: processIdentity,
      subtree_exit_receipt: connector.subtreeExitReceipt,
      connector_exit: shutdown.exited === true,
      provider_effect_count: 0,
    })}\n`);
    if (!readiness.ready) process.exitCode = 2;
  } catch (error) {
    await connector.close().catch(() => {});
    fail(
      typeof error?.code === "string" && /^[A-Z0-9_]{1,96}$/u.test(error.code)
        ? error.code
        : "WSL2_CONNECTOR_PREFLIGHT_FAILED",
    );
  }
}

await main();
