import { existsSync, statSync } from "node:fs";
import path from "node:path";
import { performance } from "node:perf_hooks";
import process from "node:process";
import { pathToFileURL } from "node:url";
import { defaultControlDatabasePath } from "./database-path.mjs";
import { LatticeStore } from "./store.mjs";
import { ControlWorkService } from "./work-core-service.mjs";

const protocolVersion = "2025-11-25";
const supportedProtocolVersions = new Set([protocolVersion, "2025-06-18"]);
const maximumFrameBytes = 65_536;
const maximumPendingInputBytes = 131_072;
const maximumResponseBytes = 1_048_576;
const toolCallWindowMs = 1_000;
const maximumToolCallsPerWindow = 16;

const snapshotTool = Object.freeze({
  name: "lattice_control_work_snapshot",
  title: "Read bounded Control work snapshot",
  description: "Read one project-scoped tree and dependency graph from the same LATTICE Control SQLite snapshot.",
  inputSchema: {
    type: "object",
    additionalProperties: false,
    required: ["project_id"],
    properties: {
      project_id: { type: "string", minLength: 1, maxLength: 128 },
      max_nodes: { type: "integer", minimum: 1, maximum: 256 },
      max_edges: { type: "integer", minimum: 1, maximum: 1_024 },
    },
  },
});

const nodeTool = Object.freeze({
  name: "lattice_control_work_node",
  title: "Read one Control work node",
  description: "Read one work node with its parent, children, dependencies, reverse dependents, and blocker reasons at an exact snapshot identity.",
  inputSchema: {
    type: "object",
    additionalProperties: false,
    required: ["project_id", "work_item_id", "revision", "digest"],
    properties: {
      project_id: { type: "string", minLength: 1, maxLength: 128 },
      work_item_id: { type: "string", minLength: 1, maxLength: 128 },
      revision: { type: "string", pattern: "^[a-f0-9]{64}$" },
      digest: { type: "string", pattern: "^[a-f0-9]{64}$" },
      max_nodes: { type: "integer", minimum: 1, maximum: 256 },
      max_edges: { type: "integer", minimum: 1, maximum: 1_024 },
    },
  },
});

const tools = Object.freeze([snapshotTool, nodeTool]);

function exactObject(value, requiredKeys, optionalKeys = []) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const keys = Object.keys(value).sort();
  const allowed = new Set([...requiredKeys, ...optionalKeys]);
  return requiredKeys.every((key) => Object.hasOwn(value, key))
    && keys.every((key) => allowed.has(key));
}

function boundedInteger(value, minimum, maximum) {
  return Number.isSafeInteger(value) && value >= minimum && value <= maximum;
}

function boundedPlainObject(value, { maximumKeys, maximumBytes }) {
  if (
    value === null
    || typeof value !== "object"
    || Array.isArray(value)
    || Object.getPrototypeOf(value) !== Object.prototype
    || Object.keys(value).length > maximumKeys
  ) return false;
  try {
    return Buffer.byteLength(JSON.stringify(value), "utf8") <= maximumBytes;
  } catch {
    return false;
  }
}

function boundedMeta(value) {
  return value === undefined || boundedPlainObject(value, {
    maximumKeys: 16,
    maximumBytes: 4_096,
  });
}

function boundedProtocolText(value, maximumBytes) {
  return typeof value === "string"
    && value.length > 0
    && Buffer.byteLength(value, "utf8") <= maximumBytes
    && !/[\u0000-\u001f\u007f-\u009f]/u.test(value);
}

function validInitializeParams(value) {
  return exactObject(
    value,
    ["protocolVersion", "capabilities", "clientInfo"],
    ["_meta"],
  )
    && boundedProtocolText(value.protocolVersion, 64)
    && boundedPlainObject(value.capabilities, { maximumKeys: 32, maximumBytes: 16_384 })
    && boundedPlainObject(value.clientInfo, { maximumKeys: 16, maximumBytes: 8_192 })
    && boundedProtocolText(value.clientInfo.name, 128)
    && boundedProtocolText(value.clientInfo.version, 64)
    && boundedMeta(value._meta);
}

function validRequestId(value) {
  return Number.isSafeInteger(value)
    || (typeof value === "string" && Buffer.byteLength(value, "utf8") <= 128);
}

function optionalRequestParams(value) {
  return value === undefined ? {} : value;
}

function safeId(value) {
  return typeof value === "string"
    && value.length >= 1
    && value.length <= 128
    && /^[A-Za-z0-9._:-]+$/u.test(value);
}

function safeDigest(value) {
  return typeof value === "string" && /^[a-f0-9]{64}$/u.test(value);
}

function validateSnapshotArguments(value) {
  if (!exactObject(value, ["project_id"], ["max_nodes", "max_edges"])) return false;
  return safeId(value.project_id)
    && (value.max_nodes == null || boundedInteger(value.max_nodes, 1, 256))
    && (value.max_edges == null || boundedInteger(value.max_edges, 1, 1_024));
}

function validateNodeArguments(value) {
  if (!exactObject(
    value,
    ["project_id", "work_item_id", "revision", "digest"],
    ["max_nodes", "max_edges"],
  )) return false;
  return safeId(value.project_id)
    && safeId(value.work_item_id)
    && safeDigest(value.revision)
    && safeDigest(value.digest)
    && (value.max_nodes == null || boundedInteger(value.max_nodes, 1, 256))
    && (value.max_edges == null || boundedInteger(value.max_edges, 1, 1_024));
}

function configuredDatabasePath() {
  const databasePath = process.env.LATTICE_CONTROL_DATABASE_PATH
    || defaultControlDatabasePath();
  if (
    !path.isAbsolute(databasePath)
    || databasePath.length > 2_048
    || /[\u0000-\u001f\u007f-\u009f]/u.test(databasePath)
    || !existsSync(databasePath)
    || !statSync(databasePath).isFile()
  ) {
    throw new Error("LATTICE_CONTROL_DATABASE_UNAVAILABLE");
  }
  return path.normalize(databasePath);
}

function safeError(error) {
  const code = typeof error?.code === "string"
    && /^[A-Z0-9_]{1,128}$/u.test(error.code)
    ? error.code
    : "CONTROL_WORK_TOOL_FAILED";
  const message = typeof error?.message === "string"
    ? error.message.slice(0, 512)
    : "Control work tool failed";
  return { code, message };
}

function toolTextSummary(name, structuredContent) {
  if (name === snapshotTool.name) {
    return JSON.stringify({
      schema_version: structuredContent.schema_version,
      project_id: structuredContent.project_id,
      revision: structuredContent.revision,
      digest: structuredContent.digest,
      node_count: structuredContent.graph.nodes.length,
      dependency_count: structuredContent.graph.nodes.reduce(
        (count, node) => count + node.depends_on.length,
        0,
      ),
    });
  }
  return JSON.stringify({
    schema_version: structuredContent.schema_version,
    project_id: structuredContent.project_id,
    work_item_id: structuredContent.graph_node.id,
    revision: structuredContent.revision,
    digest: structuredContent.digest,
  });
}

export function runControlWorkMcp({
  input = process.stdin,
  output = process.stdout,
  databasePath = configuredDatabasePath(),
} = {}) {
  const store = new LatticeStore(databasePath);
  const service = new ControlWorkService({ store });
  let initialized = false;
  let initializeSeen = false;
  let buffer = Buffer.alloc(0);
  let closed = false;
  let backpressured = false;
  let inputEnded = false;
  const toolCallTimestamps = [];

  const reserveToolCall = () => {
    const timestamp = performance.now();
    while (
      toolCallTimestamps.length > 0
      && toolCallTimestamps[0] <= timestamp - toolCallWindowMs
    ) toolCallTimestamps.shift();
    if (toolCallTimestamps.length >= maximumToolCallsPerWindow) return false;
    toolCallTimestamps.push(timestamp);
    return true;
  };

  const onDrain = () => {
    if (closed) return;
    backpressured = false;
    processBufferedFrames();
    if (closed || backpressured) return;
    if (inputEnded) finishInput();
    else input.resume();
  };

  const write = (message) => {
    const line = `${JSON.stringify(message)}\n`;
    if (Buffer.byteLength(line, "utf8") > maximumResponseBytes) {
      throw new Error("CONTROL_WORK_MCP_RESPONSE_LIMIT_EXCEEDED");
    }
    if (!output.write(line) && !backpressured) {
      backpressured = true;
      input.pause();
      output.once("drain", onDrain);
    }
  };
  const success = (id, result) => write({ jsonrpc: "2.0", id, result });
  const failure = (id, code, message) => write({
    jsonrpc: "2.0",
    id: id ?? null,
    error: { code, message },
  });
  const toolFailure = (id, code, message) => {
    const structuredContent = { error: { code, message } };
    success(id, {
      content: [{ type: "text", text: JSON.stringify(structuredContent) }],
      structuredContent,
      isError: true,
    });
  };
  const requireReady = (id) => {
    if (!initializeSeen || !initialized) {
      failure(id, -32002, "MCP session is not initialized");
      return false;
    }
    return true;
  };

  const handle = (line) => {
    let request;
    try {
      request = JSON.parse(line.toString("utf8"));
    } catch {
      failure(null, -32700, "Parse error");
      return;
    }
    if (!request || request.jsonrpc !== "2.0" || typeof request.method !== "string") {
      failure(request?.id, -32600, "Invalid Request");
      return;
    }
    const hasRequestId = Object.hasOwn(request, "id");
    if (!hasRequestId) {
      if (
        request.method === "notifications/initialized"
        && initializeSeen
        && exactObject(optionalRequestParams(request.params), [], ["_meta"])
        && boundedMeta(request.params?._meta)
      ) initialized = true;
      return;
    }
    if (!validRequestId(request.id)) {
      failure(null, -32600, "Invalid Request ID");
      return;
    }
    if (request.method === "notifications/initialized") {
      failure(request.id, -32602, "Invalid initialized notification");
      return;
    }
    if (request.method === "ping") {
      if (
        !exactObject(optionalRequestParams(request.params), [], ["_meta"])
        || !boundedMeta(request.params?._meta)
      ) {
        failure(request.id, -32602, "Invalid ping params");
        return;
      }
      success(request.id, {});
      return;
    }
    if (request.method === "initialize") {
      if (
        initializeSeen
        || !validInitializeParams(request.params)
      ) {
        failure(request.id, -32602, "Invalid initialize params");
        return;
      }
      initializeSeen = true;
      success(request.id, {
        protocolVersion: supportedProtocolVersions.has(request.params.protocolVersion)
          ? request.params.protocolVersion
          : protocolVersion,
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: "lattice-control-work-core", version: "1.0.0" },
      });
      return;
    }
    if (!requireReady(request.id)) return;
    if (request.method === "tools/list") {
      if (
        !exactObject(optionalRequestParams(request.params), [], ["_meta"])
        || !boundedMeta(request.params?._meta)
      ) {
        failure(request.id, -32602, "Invalid tools/list params");
        return;
      }
      success(request.id, { tools });
      return;
    }
    if (request.method !== "tools/call") {
      failure(request.id, -32601, "Method not found");
      return;
    }
    if (
      !exactObject(request.params, ["name"], ["arguments", "_meta"])
      || typeof request.params.name !== "string"
      || !boundedMeta(request.params._meta)
    ) {
      failure(request.id, -32602, "Invalid tools/call params");
      return;
    }
    const { name, arguments: providedArguments } = request.params;
    const args = providedArguments ?? {};
    if (!tools.some((tool) => tool.name === name)) {
      failure(request.id, -32602, "Invalid tool name or arguments");
      return;
    }
    if (
      (name === snapshotTool.name && !validateSnapshotArguments(args))
      || (name === nodeTool.name && !validateNodeArguments(args))
    ) {
      toolFailure(
        request.id,
        "CONTROL_WORK_TOOL_ARGUMENTS_REJECTED",
        "Tool arguments failed validation",
      );
      return;
    }
    if (!reserveToolCall()) {
      toolFailure(
        request.id,
        "CONTROL_WORK_TOOL_RATE_LIMITED",
        "Tool call rate limit exceeded; retry after the rolling window",
      );
      return;
    }
    try {
      const structuredContent = name === snapshotTool.name
        ? service.workSnapshot({
            projectId: args.project_id,
            maxNodes: args.max_nodes,
            maxEdges: args.max_edges,
          })
        : service.workNode({
            projectId: args.project_id,
            workItemId: args.work_item_id,
            expectedRevision: args.revision,
            expectedDigest: args.digest,
            maxNodes: args.max_nodes,
            maxEdges: args.max_edges,
          });
      success(request.id, {
        content: [{ type: "text", text: toolTextSummary(name, structuredContent) }],
        structuredContent,
        isError: false,
      });
    } catch (error) {
      const { code, message } = safeError(error);
      toolFailure(request.id, code, message);
    }
  };

  const close = () => {
    if (closed) return;
    closed = true;
    output.off("drain", onDrain);
    input.off("data", onData);
    input.off("end", onEnd);
    input.off("error", onError);
    input.pause();
    buffer = Buffer.alloc(0);
    toolCallTimestamps.length = 0;
    store.close();
  };

  function processBufferedFrames() {
    if (closed || backpressured) return;
    while (true) {
      const newline = buffer.indexOf(0x0a);
      if (newline < 0) break;
      const frame = buffer.subarray(0, newline);
      buffer = buffer.subarray(newline + 1);
      if (frame.length === 0 || frame.length > maximumFrameBytes) {
        failure(null, -32600, "MCP frame size rejected");
        process.exitCode = 1;
        input.destroy();
        close();
        return;
      }
      handle(frame);
      if (closed || backpressured) return;
    }
    if (buffer.length > maximumFrameBytes) {
      failure(null, -32600, "MCP frame size rejected");
      process.exitCode = 1;
      input.destroy();
      close();
    }
  }

  function finishInput() {
    if (closed || backpressured || !inputEnded) return;
    processBufferedFrames();
    if (closed || backpressured) return;
    if (buffer.length > 0) {
      failure(null, -32600, "Incomplete MCP frame rejected");
      process.exitCode = 1;
      buffer = Buffer.alloc(0);
    }
    close();
  }

  function onData(chunk) {
    if (closed) return;
    if (buffer.length + chunk.length > maximumPendingInputBytes) {
      if (!backpressured) failure(null, -32600, "MCP pending input limit rejected");
      process.exitCode = 1;
      input.destroy();
      close();
      return;
    }
    buffer = Buffer.concat([buffer, chunk]);
    processBufferedFrames();
  }

  function onEnd() {
    inputEnded = true;
    finishInput();
  }

  function onError() {
    process.exitCode = 1;
    close();
  }

  input.on("data", onData);
  input.on("end", onEnd);
  input.on("error", onError);
  return { close };
}

if (import.meta.url === pathToFileURL(process.argv[1] || "").href) {
  try {
    runControlWorkMcp();
  } catch {
    process.stderr.write("LATTICE_CONTROL_WORK_MCP_START_FAILED\n");
    process.exitCode = 1;
  }
}
