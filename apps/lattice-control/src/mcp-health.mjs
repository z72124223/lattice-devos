import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { controlDecisionMcpTools } from "./decision-core-mcp.mjs";
import { controlWorkMcpTools } from "./work-core-mcp.mjs";

const maximumProbeOutputBytes = 65_536;
const defaultProbeTimeoutMs = 1_200;
const defaultCacheTtlMs = 5_000;
const protocolVersion = "2025-11-25";
const healthy = "HEALTHY";
const unreachable = "UNREACHABLE";
const incompatible = "INCOMPATIBLE";

const endpoints = Object.freeze({
  work_mcp: Object.freeze({
    scriptPath: fileURLToPath(new URL("./work-core-mcp.mjs", import.meta.url)),
    expectedName: "lattice-control-work-core",
    expectedTools: controlWorkMcpTools,
  }),
  decision_mcp: Object.freeze({
    scriptPath: fileURLToPath(new URL("./decision-core-mcp.mjs", import.meta.url)),
    expectedName: "lattice-control-decision-core",
    expectedTools: controlDecisionMcpTools,
  }),
});

function validInitializeResponse(message, expectedName) {
  const result = message?.result;
  return message?.jsonrpc === "2.0"
    && message.id === 1
    && result !== null
    && typeof result === "object"
    && !Array.isArray(result)
    && Object.keys(result).sort().join("\0")
      === ["protocolVersion", "capabilities", "serverInfo"].sort().join("\0")
    && result.protocolVersion === protocolVersion
    && result.capabilities !== null
    && typeof result.capabilities === "object"
    && !Array.isArray(result.capabilities)
    && Object.keys(result.capabilities).sort().join("\0") === "tools"
    && result.capabilities.tools !== null
    && typeof result.capabilities.tools === "object"
    && !Array.isArray(result.capabilities.tools)
    && Object.keys(result.capabilities.tools).sort().join("\0") === "listChanged"
    && result.capabilities.tools.listChanged === false
    && result.serverInfo !== null
    && typeof result.serverInfo === "object"
    && !Array.isArray(result.serverInfo)
    && Object.keys(result.serverInfo).sort().join("\0") === "name\0version"
    && result.serverInfo.name === expectedName
    && result.serverInfo.version === "1.0.0";
}

function canonicalJson(value) {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  return `{${Object.keys(value).sort().map(
    (key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`,
  ).join(",")}}`;
}

function inputSchemaDigest(value) {
  return createHash("sha256").update(canonicalJson(value), "utf8").digest("hex");
}

function validToolsResponse(message, expectedTools) {
  if (
    message?.jsonrpc !== "2.0"
    || message.id !== 2
    || !Array.isArray(message?.result?.tools)
  ) return false;
  if (message.result.tools.length !== expectedTools.length) return false;
  const expectedByName = new Map(expectedTools.map((tool) => [tool.name, tool]));
  const observedNames = new Set();
  for (const tool of message.result.tools) {
    if (
      tool === null
      || typeof tool !== "object"
      || Array.isArray(tool)
      || Object.keys(tool).sort().join("\0")
        !== ["name", "title", "description", "inputSchema"].sort().join("\0")
      || typeof tool.name !== "string"
      || observedNames.has(tool.name)
    ) return false;
    observedNames.add(tool.name);
    const expected = expectedByName.get(tool.name);
    if (
      !expected
      || tool.title !== expected.title
      || tool.description !== expected.description
      || inputSchemaDigest(tool.inputSchema) !== inputSchemaDigest(expected.inputSchema)
    ) return false;
  }
  return observedNames.size === expectedByName.size;
}

function requestLine(value) {
  return `${JSON.stringify(value)}\n`;
}

export function probeControlMcpEndpoint({
  scriptPath,
  databasePath,
  expectedName,
  expectedTools,
  timeoutMs = defaultProbeTimeoutMs,
  spawnProcess = spawn,
}) {
  if (
    typeof scriptPath !== "string"
    || typeof databasePath !== "string"
    || typeof expectedName !== "string"
    || !Array.isArray(expectedTools)
    || !Number.isInteger(timeoutMs)
    || timeoutMs < 100
    || timeoutMs > 30_000
  ) throw new TypeError("CONTROL_MCP_HEALTH_PROBE_INVALID");

  return new Promise((resolve) => {
    let child;
    try {
      child = spawnProcess(process.execPath, [scriptPath], {
        env: {
          ...process.env,
          LATTICE_CONTROL_DATABASE_PATH: databasePath,
        },
        stdio: ["pipe", "pipe", "ignore"],
        windowsHide: true,
      });
    } catch {
      resolve(unreachable);
      return;
    }

    let settled = false;
    let desiredStatus = null;
    let output = Buffer.alloc(0);
    let phase = "initialize";

    const settle = (status, terminate = false) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (terminate && child.exitCode === null && child.signalCode === null) {
        try { child.kill(); } catch { /* already exited */ }
      }
      resolve(status);
    };
    const closeAfter = (status) => {
      if (desiredStatus !== null) return;
      desiredStatus = status;
      try { child.stdin.end(); } catch { settle(status, true); }
    };
    const rejectProtocol = () => closeAfter(incompatible);
    const timer = setTimeout(() => {
      settle(desiredStatus === incompatible ? incompatible : unreachable, true);
    }, timeoutMs);
    timer.unref?.();

    child.stdin.on("error", () => {
      if (desiredStatus === null) settle(unreachable, true);
    });
    child.on("error", () => settle(unreachable, true));
    child.on("exit", (code) => {
      if (desiredStatus === incompatible) {
        settle(incompatible);
        return;
      }
      settle(desiredStatus === healthy && code === 0 ? healthy : unreachable);
    });
    child.stdout.on("data", (chunk) => {
      if (settled || desiredStatus !== null) return;
      if (output.length + chunk.length > maximumProbeOutputBytes) {
        rejectProtocol();
        return;
      }
      output = Buffer.concat([output, chunk]);
      while (true) {
        const newline = output.indexOf(0x0a);
        if (newline < 0) break;
        const frame = output.subarray(0, newline);
        output = output.subarray(newline + 1);
        if (frame.length === 0) {
          rejectProtocol();
          return;
        }
        let message;
        try {
          message = JSON.parse(frame.toString("utf8"));
        } catch {
          rejectProtocol();
          return;
        }
        if (phase === "initialize") {
          if (!validInitializeResponse(message, expectedName)) {
            rejectProtocol();
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
        if (!validToolsResponse(message, expectedTools)) {
          rejectProtocol();
          return;
        }
        phase = "complete";
        closeAfter(healthy);
        return;
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

export async function probeBundledControlMcps({
  databasePath,
  timeoutMs = defaultProbeTimeoutMs,
  probeEndpoint = probeControlMcpEndpoint,
} = {}) {
  if (typeof probeEndpoint !== "function") {
    throw new TypeError("CONTROL_MCP_HEALTH_PROBE_INVALID");
  }
  const [work_mcp, decision_mcp] = await Promise.all([
    probeEndpoint({
      ...endpoints.work_mcp,
      databasePath,
      timeoutMs,
    }),
    probeEndpoint({
      ...endpoints.decision_mcp,
      databasePath,
      timeoutMs,
    }),
  ]);
  return { work_mcp, decision_mcp };
}

export class ControlMcpHealthMonitor {
  constructor({
    databasePath,
    ttlMs = defaultCacheTtlMs,
    probe = probeBundledControlMcps,
    now = () => Date.now(),
  }) {
    if (
      typeof databasePath !== "string"
      || !Number.isInteger(ttlMs)
      || ttlMs < 0
      || ttlMs > 60_000
      || typeof probe !== "function"
      || typeof now !== "function"
    ) throw new TypeError("CONTROL_MCP_HEALTH_MONITOR_INVALID");
    this.databasePath = databasePath;
    this.ttlMs = ttlMs;
    this.probe = probe;
    this.now = now;
    this.cached = null;
    this.pending = null;
  }

  current() {
    if (this.cached && this.now() < this.cached.expiresAt) {
      return Promise.resolve(this.cached.health);
    }
    if (this.pending) return this.pending;
    const pending = Promise.resolve()
      .then(() => this.probe({ databasePath: this.databasePath }))
      .then((health) => {
        const normalized = {
          work_mcp: [healthy, unreachable, incompatible].includes(health?.work_mcp)
            ? health.work_mcp
            : incompatible,
          decision_mcp: [healthy, unreachable, incompatible].includes(health?.decision_mcp)
            ? health.decision_mcp
            : incompatible,
        };
        this.cached = { health: normalized, expiresAt: this.now() + this.ttlMs };
        return normalized;
      })
      .catch(() => {
        const health = { work_mcp: unreachable, decision_mcp: unreachable };
        this.cached = { health, expiresAt: this.now() + this.ttlMs };
        return health;
      })
      .finally(() => {
        if (this.pending === pending) this.pending = null;
      });
    this.pending = pending;
    return pending;
  }
}
