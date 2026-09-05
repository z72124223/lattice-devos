import { spawn } from "node:child_process";
import { StringDecoder } from "node:string_decoder";
import {
  closedChildEnvironment,
  loadLatticeRuntimeConfiguration,
} from "./lattice-runtime-health.mjs";

const allowedTools = new Set([
  "lattice_task_submit", "lattice_task_status",
  "lattice_runtime_status",
  "lattice_control_snapshot", "lattice_control_update",
]);
const protocolVersion = "2025-11-25";
const maximumFrameBytes = 1_048_576;

function failure(code) {
  const error = new Error("LATTICE 暫時無法完成這項操作（" + code + "）。");
  error.code = code;
  return error;
}

function toolValue(result) {
  if (!result || !Array.isArray(result.content)) throw failure("LATTICE_RUNTIME_RESPONSE_REJECTED");
  let value = result.structuredContent;
  const texts = result.content.filter((item) => item.type === "text");
  if (value === undefined && texts.length === 1) {
    try { value = JSON.parse(texts[0].text); } catch { /* handled below */ }
  }
  if (result.isError) {
    const candidate = value?.code ?? value?.error ?? texts[0]?.text;
    throw failure(typeof candidate === "string" && /^[A-Z][A-Z0-9_]{0,127}$/u.test(candidate)
      ? candidate : "LATTICE_RUNTIME_TOOL_FAILED");
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw failure("LATTICE_RUNTIME_RESPONSE_REJECTED");
  }
  return value;
}

// One connection to the configured Runtime. Requests stay ordered, and an
// uncertain request is never automatically sent again.
export class LatticeRuntimeClient {
  constructor({
    configurationLoader = loadLatticeRuntimeConfiguration,
    spawnProcess = spawn,
    startupTimeoutMs = 120_000,
    requestTimeoutMs = 60_000,
    cleanupTimeoutMs = 2_000,
    callsPerSession = 48,
  } = {}) {
    if (typeof configurationLoader !== "function" || typeof spawnProcess !== "function"
      || ![startupTimeoutMs, requestTimeoutMs, cleanupTimeoutMs].every((v) => Number.isInteger(v) && v > 0)
      || !Number.isInteger(callsPerSession) || callsPerSession < 1 || callsPerSession > 48) {
      throw new TypeError("Invalid Runtime client configuration");
    }
    Object.assign(this, { configurationLoader, spawnProcess, startupTimeoutMs,
      requestTimeoutMs, cleanupTimeoutMs, callsPerSession });
    this.connection = null;
    this.queue = Promise.resolve();
    this.closed = false;
    this.closedSignal = new Promise((resolve) => { this.signalClosed = resolve; });
    this.epoch = 0;
    this.nextId = 1;
  }

  call(name, args) {
    if (!allowedTools.has(name) || !args || typeof args !== "object" || Array.isArray(args)) {
      return Promise.reject(new TypeError("Unsupported Runtime operation"));
    }
    // Capture bytes before queuing; callers cannot change an accepted request.
    const captured = JSON.parse(JSON.stringify(args));
    if (Buffer.byteLength(JSON.stringify(captured)) > 60_000) {
      return Promise.reject(new TypeError("Runtime request is too large"));
    }
    const operation = this.queue.then(async () => {
      if (this.closed) throw failure("LATTICE_RUNTIME_CLIENT_CLOSED");
      if (this.connection?.calls >= this.callsPerSession) await this.#disconnect();
      const connection = await this.#connect();
      if (!connection.tools.has(name)) throw failure("LATTICE_RUNTIME_UPGRADE_REQUIRED");
      connection.calls += 1;
      const result = await this.#request(connection, "tools/call",
        { name, arguments: captured }, this.requestTimeoutMs);
      return toolValue(result);
    });
    this.queue = operation.catch(() => {});
    return operation;
  }

  async #connect() {
    if (this.connection?.ready && !this.connection.ended) return this.connection;
    if (this.connection?.ended) await this.#waitForExit(this.connection);
    const epoch = this.epoch;
    let configuration;
    let loadingTimer;
    try {
      configuration = await Promise.race([
        this.configurationLoader(),
        this.closedSignal.then(() => { throw failure("LATTICE_RUNTIME_CLIENT_CLOSED"); }),
        new Promise((_, reject) => {
          loadingTimer = setTimeout(() => reject(failure("LATTICE_RUNTIME_STARTUP_TIMEOUT")),
            this.startupTimeoutMs);
        }),
      ]);
    } finally { clearTimeout(loadingTimer); }
    if (this.closed || epoch !== this.epoch) throw failure("LATTICE_RUNTIME_CLIENT_CLOSED");
    const child = this.spawnProcess(configuration.executablePath, [], {
      env: closedChildEnvironment(configuration.environment),
      windowsHide: true,
      stdio: ["pipe", "pipe", "ignore"],
    });
    const connection = {
      child, pending: new Map(), decoder: new StringDecoder("utf8"),
      buffer: "", ready: false, calls: 0, tools: new Set(), ended: false,
    };
    connection.exited = false;
    connection.exit = new Promise((resolve) => { connection.resolveExit = resolve; });
    this.connection = connection;
    const abort = (code) => {
      if (connection.ended) return;
      connection.ended = true;
      for (const request of connection.pending.values()) {
        clearTimeout(request.timer);
        request.reject(failure(code));
      }
      connection.pending.clear();
      if (child.exitCode == null && child.signalCode == null) child.kill();
    };
    connection.abort = abort;
    child.once("error", () => abort("LATTICE_RUNTIME_UNAVAILABLE"));
    child.once("close", () => {
      connection.exited = true;
      connection.resolveExit();
      abort("LATTICE_RUNTIME_CONNECTION_CLOSED");
      if (this.connection === connection) this.connection = null;
    });
    child.stdin.on("error", () => abort("LATTICE_RUNTIME_CONNECTION_CLOSED"));
    child.stdout.on("data", (chunk) => {
      connection.buffer += connection.decoder.write(chunk);
      if (Buffer.byteLength(connection.buffer) > maximumFrameBytes) {
        abort("LATTICE_RUNTIME_RESPONSE_TOO_LARGE");
        return;
      }
      let newline;
      while ((newline = connection.buffer.indexOf("\n")) >= 0) {
        const line = connection.buffer.slice(0, newline);
        connection.buffer = connection.buffer.slice(newline + 1);
        let message;
        try { message = JSON.parse(line); } catch {
          abort("LATTICE_RUNTIME_RESPONSE_REJECTED"); return;
        }
        if (message?.jsonrpc !== "2.0") { abort("LATTICE_RUNTIME_RESPONSE_REJECTED"); return; }
        if (!Object.hasOwn(message, "id") && typeof message.method === "string") continue;
        const request = connection.pending.get(message.id);
        if (!request) { abort("LATTICE_RUNTIME_RESPONSE_REJECTED"); return; }
        clearTimeout(request.timer);
        connection.pending.delete(message.id);
        if (message.error) request.reject(failure("LATTICE_RUNTIME_REQUEST_REJECTED"));
        else if (Object.hasOwn(message, "result")) request.resolve(message.result);
        else {
          abort("LATTICE_RUNTIME_RESPONSE_REJECTED");
          request.reject(failure("LATTICE_RUNTIME_RESPONSE_REJECTED"));
          return;
        }
      }
    });
    try {
      const initialized = await this.#request(connection, "initialize", {
        protocolVersion, capabilities: {},
        clientInfo: { name: "lattice-control", version: "1.0.0" },
      }, this.startupTimeoutMs);
      if (initialized?.protocolVersion !== protocolVersion || initialized?.serverInfo?.name !== "latticed") {
        throw failure("LATTICE_RUNTIME_INCOMPATIBLE");
      }
      child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
      const catalog = await this.#request(connection, "tools/list", {}, this.requestTimeoutMs);
      if (!Array.isArray(catalog?.tools)) throw failure("LATTICE_RUNTIME_INCOMPATIBLE");
      connection.tools = new Set(catalog.tools.map((tool) => tool.name));
      connection.ready = true;
      return connection;
    } catch (error) {
      abort("LATTICE_RUNTIME_CONNECTION_CLOSED");
      throw error;
    }
  }

  #request(connection, method, params, timeoutMs) {
    if (this.closed || connection.ended) return Promise.reject(failure("LATTICE_RUNTIME_CLIENT_CLOSED"));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => connection.abort("LATTICE_RUNTIME_OUTCOME_UNKNOWN"), timeoutMs);
      connection.pending.set(id, { resolve, reject, timer });
      connection.child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
        (error) => { if (error) connection.abort("LATTICE_RUNTIME_OUTCOME_UNKNOWN"); });
    });
  }

  async #disconnect() {
    const connection = this.connection;
    if (!connection) return;
    const child = connection.child;
    if (!connection.ended) child.stdin.end();
    try { await this.#waitForExit(connection); } catch {
      connection.abort("LATTICE_RUNTIME_CONNECTION_CLOSED");
      await this.#waitForExit(connection);
    }
  }

  async #waitForExit(connection) {
    if (connection.exited) return;
    let timer;
    try {
      await Promise.race([
        connection.exit,
        new Promise((_, reject) => {
          timer = setTimeout(() => reject(failure("LATTICE_RUNTIME_PROCESS_STILL_CLOSING")),
            this.cleanupTimeoutMs);
        }),
      ]);
    } finally { clearTimeout(timer); }
  }

  async close() {
    if (this.closed) return;
    this.closed = true;
    this.signalClosed();
    this.epoch += 1;
    const connection = this.connection;
    for (const request of connection?.pending.values() ?? []) {
      clearTimeout(request.timer);
      request.reject(failure("LATTICE_RUNTIME_CLIENT_CLOSED"));
    }
    connection?.pending.clear();
    await this.#disconnect();
  }
}
