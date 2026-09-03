import { spawn } from "node:child_process";
import { readFile, stat } from "node:fs/promises";
import { homedir } from "node:os";
import path from "node:path";
import process from "node:process";

const protocolVersion = "2025-11-25";
const maximumProbeOutputBytes = 65_536;
const defaultProbeTimeoutMs = 30_000;
const defaultCacheTtlMs = 60_000;

export const latticeRuntimeTools = Object.freeze([
  "lattice_delivery_reconcile",
  "lattice_delivery_run",
  "lattice_delivery_status",
  "lattice_foreman_checkpoint",
  "lattice_runtime_status",
  "lattice_task_status",
  "lattice_task_submit",
]);

const healthy = Object.freeze({
  postgresql: "HEALTHY",
  detail: "LATTICE_RUNTIME_VERIFIED",
});
const unreachable = Object.freeze({
  postgresql: "UNREACHABLE",
  detail: "LATTICE_RUNTIME_UNREACHABLE",
});
const incompatible = Object.freeze({
  postgresql: "INCOMPATIBLE",
  detail: "LATTICE_RUNTIME_INCOMPATIBLE",
});
const stopped = Object.freeze({
  postgresql: "STOPPED",
  detail: "LATTICE_RUNTIME_NOT_CONFIGURED",
});
const checking = Object.freeze({
  postgresql: "NO_DATA",
  detail: "LATTICE_RUNTIME_CHECKING",
});

function configurationError(code) {
  const error = new Error(code);
  error.code = code;
  return error;
}

function parseTomlString(rawValue) {
  const source = rawValue.trim();
  if (source.startsWith("\"")) {
    let escaped = false;
    for (let index = 1; index < source.length; index += 1) {
      const character = source[index];
      if (escaped) {
        escaped = false;
        continue;
      }
      if (character === "\\") {
        escaped = true;
        continue;
      }
      if (character !== "\"") continue;
      const literal = source.slice(0, index + 1);
      const remainder = source.slice(index + 1).trim();
      if (remainder !== "" && !remainder.startsWith("#")) {
        throw configurationError("LATTICE_RUNTIME_CONFIGURATION_INCOMPATIBLE");
      }
      try {
        const value = JSON.parse(literal);
        if (typeof value === "string") return value;
      } catch {
        break;
      }
    }
  } else if (source.startsWith("'")) {
    const closing = source.indexOf("'", 1);
    if (closing > 0) {
      const remainder = source.slice(closing + 1).trim();
      if ((remainder === "" || remainder.startsWith("#")) && !source.slice(1, closing).includes("'")) {
        return source.slice(1, closing);
      }
    }
  }
  throw configurationError("LATTICE_RUNTIME_CONFIGURATION_INCOMPATIBLE");
}

function parseLatticeConfiguration(text) {
  if (typeof text !== "string" || text.length > 2_000_000) {
    throw configurationError("LATTICE_RUNTIME_CONFIGURATION_INCOMPATIBLE");
  }
  let section = "";
  let command = null;
  let latticeSectionSeen = false;
  let environmentSectionSeen = false;
  const environment = Object.create(null);
  for (const sourceLine of text.replaceAll("\r\n", "\n").split("\n")) {
    const line = sourceLine.trim();
    if (line === "" || line.startsWith("#")) continue;
    const sectionMatch = /^\[([^\]]+)\](?:\s*#.*)?$/u.exec(line);
    if (sectionMatch) {
      section = sectionMatch[1];
      if (section === "mcp_servers.lattice") {
        if (latticeSectionSeen) {
          throw configurationError("LATTICE_RUNTIME_CONFIGURATION_INCOMPATIBLE");
        }
        latticeSectionSeen = true;
      } else if (section === "mcp_servers.lattice.env") {
        if (environmentSectionSeen) {
          throw configurationError("LATTICE_RUNTIME_CONFIGURATION_INCOMPATIBLE");
        }
        environmentSectionSeen = true;
      }
      continue;
    }
    if (section !== "mcp_servers.lattice" && section !== "mcp_servers.lattice.env") {
      continue;
    }
    const assignment = /^([A-Za-z0-9_-]+)\s*=\s*(.+)$/u.exec(line);
    if (!assignment) {
      throw configurationError("LATTICE_RUNTIME_CONFIGURATION_INCOMPATIBLE");
    }
    const [, key, rawValue] = assignment;
    if (section === "mcp_servers.lattice") {
      if (key !== "command" || command !== null) {
        throw configurationError("LATTICE_RUNTIME_CONFIGURATION_INCOMPATIBLE");
      }
      command = parseTomlString(rawValue);
      continue;
    }
    if (!/^LATTICE_[A-Z0-9_]+$/u.test(key) || Object.hasOwn(environment, key)) {
      throw configurationError("LATTICE_RUNTIME_CONFIGURATION_INCOMPATIBLE");
    }
    environment[key] = parseTomlString(rawValue);
  }
  if (!latticeSectionSeen || !environmentSectionSeen || !command) {
    throw configurationError("LATTICE_RUNTIME_NOT_CONFIGURED");
  }
  return { command, environment };
}

async function verifyExecutableFile(executablePath) {
  const metadata = await stat(executablePath);
  if (!metadata.isFile()) {
    throw configurationError("LATTICE_RUNTIME_NOT_CONFIGURED");
  }
}

export async function loadLatticeRuntimeConfiguration({
  configPath = path.join(homedir(), ".codex", "config.toml"),
  readText = (target) => readFile(target, "utf8"),
  verifyExecutable = verifyExecutableFile,
} = {}) {
  if (
    typeof configPath !== "string"
    || !path.isAbsolute(configPath)
    || typeof readText !== "function"
    || typeof verifyExecutable !== "function"
  ) throw new TypeError("LATTICE_RUNTIME_CONFIGURATION_LOADER_INVALID");

  const parsed = parseLatticeConfiguration(await readText(configPath));
  if (!path.isAbsolute(parsed.command)) {
    throw configurationError("LATTICE_RUNTIME_NOT_CONFIGURED");
  }
  await verifyExecutable(parsed.command);
  const environment = { ...parsed.environment };
  if (!environment.LATTICE_DELIVERY_LAUNCHER) {
    if (!path.isAbsolute(environment.LATTICE_HERMES_CODEX_LAUNCHER ?? "")) {
      throw configurationError("LATTICE_RUNTIME_CONFIGURATION_INCOMPATIBLE");
    }
    environment.LATTICE_DELIVERY_LAUNCHER = environment.LATTICE_HERMES_CODEX_LAUNCHER;
  }
  if (!environment.LATTICE_DELIVERY_SCHEMA_DIR) {
    if (!path.isAbsolute(environment.LATTICE_DELIVERY_ROOT ?? "")) {
      throw configurationError("LATTICE_RUNTIME_CONFIGURATION_INCOMPATIBLE");
    }
    environment.LATTICE_DELIVERY_SCHEMA_DIR = path.join(
      environment.LATTICE_DELIVERY_ROOT,
      "schema",
    );
  }
  return Object.freeze({
    executablePath: parsed.command,
    environment: Object.freeze(environment),
  });
}

function exactKeys(value, keys) {
  return value !== null
    && typeof value === "object"
    && !Array.isArray(value)
    && Object.keys(value).sort().join("\0") === [...keys].sort().join("\0");
}

function validInitializeResponse(message) {
  const result = message?.result;
  return message?.jsonrpc === "2.0"
    && message.id === 1
    && exactKeys(result, ["protocolVersion", "capabilities", "serverInfo", "instructions"])
    && result.protocolVersion === protocolVersion
    && exactKeys(result.capabilities, ["tools"])
    && exactKeys(result.capabilities.tools, [])
    && exactKeys(result.serverInfo, ["name", "title", "version"])
    && result.serverInfo.name === "latticed"
    && result.serverInfo.title === "LATTICE DevOS"
    && result.serverInfo.version === "1.0.0"
    && typeof result.instructions === "string"
    && result.instructions.length > 0
    && result.instructions.length <= 8_192;
}

function validToolsResponse(message) {
  if (
    message?.jsonrpc !== "2.0"
    || message.id !== 2
    || !Array.isArray(message?.result?.tools)
    || message.result.tools.length !== latticeRuntimeTools.length
  ) return false;
  const names = message.result.tools.map((tool) => tool?.name);
  return names.every((name) => typeof name === "string")
    && new Set(names).size === names.length
    && names.sort().join("\0") === [...latticeRuntimeTools].sort().join("\0");
}

function validRuntimeStatus(value) {
  if (!exactKeys(value, [
    "component",
    "foreman",
    "graphify_runtime_status",
    "hermes_activation_status",
    "hermes_runtime_status",
    "runtime_integration",
    "scope",
    "status",
  ])) return false;
  if (
    value.component !== "delivery-receipt"
    || typeof value.status !== "string"
    || typeof value.scope !== "string"
    || typeof value.runtime_integration !== "string"
    || typeof value.graphify_runtime_status !== "string"
    || typeof value.hermes_runtime_status !== "string"
    || typeof value.hermes_activation_status !== "string"
  ) return false;
  const foreman = value.foreman;
  if (!exactKeys(foreman, [
    "active_count",
    "blocked_count",
    "checkpoint_digest",
    "checkpoint_status",
    "completed_count",
    "degraded_code",
    "dependency",
    "latest_generation",
    "ledger_digest",
    "next_action",
    "replay_status",
    "schema",
  ])) return false;
  return foreman.schema === "lattice.foreman-runtime-projection/1.1"
    && foreman.replay_status === "VERIFIED"
    && typeof foreman.checkpoint_status === "string"
    && typeof foreman.next_action === "string"
    && ["active_count", "blocked_count", "completed_count", "latest_generation"]
      .every((key) => Number.isSafeInteger(foreman[key]) && foreman[key] >= 0)
    && ["ledger_digest", "checkpoint_digest"]
      .every((key) => foreman[key] === null || /^[0-9a-f]{64}$/u.test(foreman[key]))
    && (foreman.degraded_code === null || typeof foreman.degraded_code === "string")
    && (foreman.dependency === null
      || (typeof foreman.dependency === "object" && !Array.isArray(foreman.dependency)));
}

function runtimeStatusFromMessage(message) {
  if (
    message?.jsonrpc !== "2.0"
    || message.id !== 3
    || message.result === null
    || typeof message.result !== "object"
    || Array.isArray(message.result)
  ) return incompatible;
  if (message.result.isError === true) return unreachable;
  return message.result.isError === false
    && validRuntimeStatus(message.result.structuredContent)
    ? healthy
    : incompatible;
}

function requestLine(value) {
  return `${JSON.stringify(value)}\n`;
}

export function probeLatticeRuntimeEndpoint({
  executablePath,
  environment,
  timeoutMs = defaultProbeTimeoutMs,
  spawnProcess = spawn,
  signal,
  onOwnedChild,
}) {
  if (
    typeof executablePath !== "string"
    || !path.isAbsolute(executablePath)
    || environment === null
    || typeof environment !== "object"
    || Array.isArray(environment)
    || !Number.isInteger(timeoutMs)
    || timeoutMs < 100
    || timeoutMs > 30_000
    || typeof spawnProcess !== "function"
    || (onOwnedChild !== undefined && typeof onOwnedChild !== "function")
    || (signal !== undefined
      && (signal === null
        || typeof signal.aborted !== "boolean"
        || typeof signal.addEventListener !== "function"
        || typeof signal.removeEventListener !== "function"))
  ) throw new TypeError("LATTICE_RUNTIME_HEALTH_PROBE_INVALID");

  if (signal?.aborted) return Promise.resolve({ ...unreachable });

  return new Promise((resolve) => {
    let child;
    try {
      child = spawnProcess(executablePath, [], {
        cwd: path.dirname(executablePath),
        env: { ...process.env, ...environment },
        shell: false,
        stdio: ["pipe", "pipe", "ignore"],
        windowsHide: true,
      });
    } catch {
      resolve({ ...unreachable });
      return;
    }
    try {
      onOwnedChild?.(child);
    } catch {
      try { child.kill(); } catch { /* failed spawn cleanup */ }
      resolve({ ...unreachable });
      return;
    }

    let settled = false;
    let output = Buffer.alloc(0);
    let observedOutputBytes = 0;
    let phase = "initialize";
    const settle = (status) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      if (child.exitCode === null && child.signalCode === null) {
        try { child.kill(); } catch { /* already exited */ }
      }
      resolve({ ...status });
    };
    const onAbort = () => settle(unreachable);
    const timer = setTimeout(() => settle(unreachable), timeoutMs);
    timer.unref?.();
    signal?.addEventListener("abort", onAbort, { once: true });
    if (signal?.aborted) {
      onAbort();
      return;
    }

    child.stdin.on("error", () => settle(unreachable));
    child.on("error", () => settle(unreachable));
    child.on("exit", () => {
      if (!settled) settle(unreachable);
    });
    child.stdout.on("data", (chunk) => {
      if (settled) return;
      observedOutputBytes += chunk.length;
      if (observedOutputBytes > maximumProbeOutputBytes) {
        settle(incompatible);
        return;
      }
      output = Buffer.concat([output, chunk]);
      while (!settled) {
        const newline = output.indexOf(0x0a);
        if (newline < 0) break;
        const frame = output.subarray(0, newline);
        output = output.subarray(newline + 1);
        if (frame.length === 0) {
          settle(incompatible);
          return;
        }
        let message;
        try {
          message = JSON.parse(frame.toString("utf8"));
        } catch {
          settle(incompatible);
          return;
        }
        if (phase === "initialize") {
          if (!validInitializeResponse(message)) {
            settle(incompatible);
            return;
          }
          phase = "tools";
          child.stdin.write(requestLine({
            jsonrpc: "2.0",
            method: "notifications/initialized",
            params: {},
          }));
          child.stdin.write(requestLine({
            jsonrpc: "2.0",
            id: 2,
            method: "tools/list",
            params: {},
          }));
          continue;
        }
        if (phase === "tools") {
          if (!validToolsResponse(message)) {
            settle(incompatible);
            return;
          }
          phase = "status";
          child.stdin.write(requestLine({
            jsonrpc: "2.0",
            id: 3,
            method: "tools/call",
            params: { name: "lattice_runtime_status", arguments: {} },
          }));
          continue;
        }
        settle(runtimeStatusFromMessage(message));
      }
    });

    child.stdin.write(requestLine({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion,
        capabilities: {},
        clientInfo: { name: "lattice-control-runtime-health", version: "1.0.0" },
      },
    }));
  });
}

export async function probeConfiguredLatticeRuntime({
  loadConfiguration = loadLatticeRuntimeConfiguration,
  probeEndpoint = probeLatticeRuntimeEndpoint,
  signal,
  onOwnedChild,
} = {}) {
  if (typeof loadConfiguration !== "function" || typeof probeEndpoint !== "function") {
    throw new TypeError("LATTICE_RUNTIME_CONFIGURED_PROBE_INVALID");
  }
  let configuration;
  try {
    configuration = await loadConfiguration();
  } catch (error) {
    if (error?.code === "ENOENT" || error?.code === "LATTICE_RUNTIME_NOT_CONFIGURED") {
      return { ...stopped };
    }
    return { ...incompatible };
  }
  if (signal?.aborted) return { ...unreachable };
  try {
    return await probeEndpoint({ ...configuration, signal, onOwnedChild });
  } catch {
    return { ...unreachable };
  }
}

function normalizeHealth(value) {
  if (value?.postgresql === "HEALTHY") return { ...healthy };
  if (value?.postgresql === "STOPPED") return { ...stopped };
  if (value?.postgresql === "INCOMPATIBLE") return { ...incompatible };
  return { ...unreachable };
}

export class LatticeRuntimeHealthMonitor {
  constructor({
    ttlMs = defaultCacheTtlMs,
    probe = probeConfiguredLatticeRuntime,
    now = () => Date.now(),
  } = {}) {
    if (
      !Number.isInteger(ttlMs)
      || ttlMs < 0
      || ttlMs > 300_000
      || typeof probe !== "function"
      || typeof now !== "function"
    ) throw new TypeError("LATTICE_RUNTIME_HEALTH_MONITOR_INVALID");
    this.ttlMs = ttlMs;
    this.probe = probe;
    this.now = now;
    this.cached = null;
    this.pending = null;
    this.probeController = null;
    this.ownedChildren = new Map();
    this.closed = false;
  }

  trackOwnedChild(child) {
    if (
      child === null
      || typeof child !== "object"
      || typeof child.once !== "function"
      || typeof child.kill !== "function"
      || child.exitCode !== null
      || child.signalCode !== null
    ) return;
    let finish;
    const exited = new Promise((resolve) => { finish = resolve; });
    const done = () => {
      if (!this.ownedChildren.has(child)) return;
      this.ownedChildren.delete(child);
      finish();
    };
    this.ownedChildren.set(child, exited);
    child.once("exit", done);
    child.once("error", done);
  }

  current({ waitForProbe = true } = {}) {
    if (typeof waitForProbe !== "boolean") {
      throw new TypeError("LATTICE_RUNTIME_HEALTH_MONITOR_INVALID");
    }
    if (this.closed) return Promise.resolve({ ...stopped });
    if (this.cached && this.now() < this.cached.expiresAt) {
      return Promise.resolve(this.cached.health);
    }
    if (!this.pending) {
      const probeController = new AbortController();
      this.probeController = probeController;
      const pending = Promise.resolve()
        .then(() => this.probe({
          signal: probeController.signal,
          onOwnedChild: (child) => this.trackOwnedChild(child),
        }))
        .then(normalizeHealth)
        .catch(() => ({ ...unreachable }))
        .then((health) => {
          if (!this.closed) {
            this.cached = { health, expiresAt: this.now() + this.ttlMs };
          }
          return health;
        })
        .finally(() => {
          if (this.pending === pending) this.pending = null;
          if (this.probeController === probeController) this.probeController = null;
        });
      this.pending = pending;
    }
    return waitForProbe ? this.pending : Promise.resolve({ ...checking });
  }

  async close() {
    if (this.closed) return;
    this.closed = true;
    this.cached = { health: { ...stopped }, expiresAt: Number.POSITIVE_INFINITY };
    this.probeController?.abort();
    if (this.pending) await this.pending;
    const children = [...this.ownedChildren.keys()];
    for (const child of children) {
      if (child.exitCode === null && child.signalCode === null) {
        try { child.kill(); } catch { /* already exited */ }
      }
    }
    if (this.ownedChildren.size > 0) {
      let timer;
      try {
        await Promise.race([
          Promise.allSettled([...this.ownedChildren.values()]),
          new Promise((resolve) => {
            timer = setTimeout(resolve, 2_000);
            timer.unref?.();
          }),
        ]);
      } finally {
        clearTimeout(timer);
      }
    }
  }
}
