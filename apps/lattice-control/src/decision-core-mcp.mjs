import { existsSync, statSync } from "node:fs";
import path from "node:path";
import { performance } from "node:perf_hooks";
import process from "node:process";
import { pathToFileURL } from "node:url";
import { defaultControlDatabasePath } from "./database-path.mjs";
import { ControlDecisionService } from "./decision-core-service.mjs";
import { LatticeStore } from "./store.mjs";

const protocolVersion = "2025-11-25";
const supportedProtocolVersions = new Set([protocolVersion, "2025-06-18"]);
const maximumFrameBytes = 65_536;
const maximumPendingInputBytes = 131_072;
const maximumResponseBytes = 1_048_576;
const toolCallWindowMs = 1_000;
const maximumToolCallsPerWindow = 16;
const sourceKinds = ["user_confirmation", "approved_document"];

const recordTool = Object.freeze({
  name: "lattice_control_decision_record",
  title: "Record or supersede one confirmed Control decision",
  description: "Record one explicit decision in the LATTICE Control store, optionally superseding the exact current decision for the same scope and subject.",
  inputSchema: {
    type: "object",
    additionalProperties: false,
    required: [
      "scope",
      "subject",
      "content",
      "rationale",
      "source",
      "client_request_id",
      "revision",
      "digest",
    ],
    properties: {
      scope: { type: "string", minLength: 1, maxLength: 128 },
      subject: { type: "string", minLength: 1, maxLength: 256 },
      content: { type: "string", minLength: 1, maxLength: 4_096 },
      rationale: { type: "string", minLength: 1, maxLength: 4_096 },
      source: {
        oneOf: [
          {
            type: "object",
            additionalProperties: false,
            required: ["kind", "reference"],
            properties: {
              kind: { const: "user_confirmation" },
              reference: {
                type: "string",
                minLength: 1,
                maxLength: 512,
                pattern: "^thread:[A-Za-z0-9][A-Za-z0-9._-]{0,127}/(?:turn|delegation):[A-Za-z0-9][A-Za-z0-9._:-]{0,127}(?:#[A-Za-z0-9][A-Za-z0-9._:-]{0,127})?$",
              },
            },
          },
          {
            type: "object",
            additionalProperties: false,
            required: ["kind", "reference"],
            properties: {
              kind: { const: "approved_document" },
              reference: {
                type: "string",
                minLength: 1,
                maxLength: 512,
                pattern: "^(?:file:[A-Za-z0-9][A-Za-z0-9._/-]{0,383}|document:[A-Za-z0-9][A-Za-z0-9._:/-]{0,383})#[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$",
              },
            },
          },
        ],
      },
      supersedes_decision_id: { type: "string", minLength: 1, maxLength: 128 },
      client_request_id: { type: "string", minLength: 1, maxLength: 128 },
      revision: { type: "integer", minimum: 0 },
      digest: { type: "string", pattern: "^[a-f0-9]{64}$" },
    },
  },
});

const currentTool = Object.freeze({
  name: "lattice_control_decision_current",
  title: "Read bounded current Control decisions",
  description: "Read a bounded current-decisions packet from one Control scope with a verifiable revision and digest.",
  inputSchema: {
    type: "object",
    additionalProperties: false,
    required: ["scope", "limit"],
    properties: {
      scope: { type: "string", minLength: 1, maxLength: 128 },
      subject: { type: "string", minLength: 1, maxLength: 256 },
      limit: { type: "integer", minimum: 1, maximum: 32 },
    },
  },
});

const readTool = Object.freeze({
  name: "lattice_control_decision_read",
  title: "Read one Control decision and bounded lineage",
  description: "Read one retained decision and a bounded lineage window at an exact Control decision revision and digest.",
  inputSchema: {
    type: "object",
    additionalProperties: false,
    required: ["decision_id", "max_depth", "revision", "digest"],
    properties: {
      decision_id: { type: "string", minLength: 1, maxLength: 128 },
      max_depth: { type: "integer", minimum: 1, maximum: 64 },
      revision: { type: "integer", minimum: 0 },
      digest: { type: "string", pattern: "^[a-f0-9]{64}$" },
    },
  },
});

const searchTool = Object.freeze({
  name: "lattice_control_decision_search",
  title: "Search bounded Control decision history",
  description: "Search retained decisions directly in one Control scope with a required query, limit, revision, and digest.",
  inputSchema: {
    type: "object",
    additionalProperties: false,
    required: ["scope", "query", "limit", "revision", "digest"],
    properties: {
      scope: { type: "string", minLength: 1, maxLength: 128 },
      query: { type: "string", minLength: 1, maxLength: 128 },
      limit: { type: "integer", minimum: 1, maximum: 20 },
      revision: { type: "integer", minimum: 0 },
      digest: { type: "string", pattern: "^[a-f0-9]{64}$" },
    },
  },
});

const tools = Object.freeze([recordTool, currentTool, readTool, searchTool]);

function exactObject(value, requiredKeys, optionalKeys = []) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const allowed = new Set([...requiredKeys, ...optionalKeys]);
  const keys = Object.keys(value);
  return requiredKeys.every((key) => Object.hasOwn(value, key))
    && keys.every((key) => allowed.has(key));
}

function boundedInteger(value, minimum, maximum = Number.MAX_SAFE_INTEGER) {
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
  return exactObject(value, ["protocolVersion", "capabilities", "clientInfo"], ["_meta"])
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

function safeIdentifier(value, maximumLength) {
  return typeof value === "string"
    && value.length >= 1
    && value.length <= maximumLength
    && /^[A-Za-z0-9][A-Za-z0-9._:/-]*$/u.test(value);
}

function safeRequestId(value) {
  return typeof value === "string"
    && value.length >= 1
    && value.length <= 128
    && /^[A-Za-z0-9._:-]+$/u.test(value);
}

function safeDigest(value) {
  return typeof value === "string" && /^[a-f0-9]{64}$/u.test(value);
}

function decisionText(value, maximumBytes) {
  return typeof value === "string"
    && value.trim().length > 0
    && Buffer.byteLength(value.trim(), "utf8") <= maximumBytes
    && !/[\u0000-\u001f\u007f-\u009f]/u.test(value);
}

function decisionSource(value) {
  return exactObject(value, ["kind", "reference"])
    && sourceKinds.includes(value.kind)
    && typeof value.reference === "string"
    && value.reference.length >= 1
    && value.reference.length <= 512
    && /^[A-Za-z0-9][A-Za-z0-9._:/#@+-]*$/u.test(value.reference)
    && (
      (
        value.kind === "user_confirmation"
        && /^thread:[A-Za-z0-9][A-Za-z0-9._-]{0,127}\/(?:turn|delegation):[A-Za-z0-9][A-Za-z0-9._:-]{0,127}(?:#[A-Za-z0-9][A-Za-z0-9._:-]{0,127})?$/u.test(value.reference)
      )
      || (
        value.kind === "approved_document"
        && /^(?:file:[A-Za-z0-9][A-Za-z0-9._/-]{0,383}|document:[A-Za-z0-9][A-Za-z0-9._:/-]{0,383})#[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u.test(value.reference)
      )
    );
}

function validateRecordArguments(value) {
  if (!exactObject(
    value,
    [
      "scope",
      "subject",
      "content",
      "rationale",
      "source",
      "client_request_id",
      "revision",
      "digest",
    ],
    ["supersedes_decision_id"],
  )) return false;
  return safeIdentifier(value.scope, 128)
    && safeIdentifier(value.subject, 256)
    && decisionText(value.content, 4_096)
    && decisionText(value.rationale, 4_096)
    && decisionSource(value.source)
    && safeRequestId(value.client_request_id)
    && boundedInteger(value.revision, 0)
    && safeDigest(value.digest)
    && (
      value.supersedes_decision_id == null
      || safeIdentifier(value.supersedes_decision_id, 128)
    );
}

function validateCurrentArguments(value) {
  return exactObject(value, ["scope", "limit"], ["subject"])
    && safeIdentifier(value.scope, 128)
    && (value.subject == null || safeIdentifier(value.subject, 256))
    && boundedInteger(value.limit, 1, 32);
}

function validateReadArguments(value) {
  return exactObject(value, ["decision_id", "max_depth", "revision", "digest"])
    && safeIdentifier(value.decision_id, 128)
    && boundedInteger(value.max_depth, 1, 64)
    && boundedInteger(value.revision, 0)
    && safeDigest(value.digest);
}

function validateSearchArguments(value) {
  return exactObject(value, ["scope", "query", "limit", "revision", "digest"])
    && safeIdentifier(value.scope, 128)
    && decisionText(value.query, 128)
    && boundedInteger(value.limit, 1, 20)
    && boundedInteger(value.revision, 0)
    && safeDigest(value.digest);
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
    : "CONTROL_DECISION_TOOL_FAILED";
  const message = typeof error?.message === "string"
    ? error.message.slice(0, 512)
    : "Control decision tool failed";
  return { code, message };
}

function toolTextSummary(name, structuredContent) {
  if (name === recordTool.name) {
    return JSON.stringify({
      schema_version: structuredContent.schema_version,
      decision_id: structuredContent.decision.id,
      status: structuredContent.decision.status,
      changed: structuredContent.changed,
      revision: structuredContent.revision,
      digest: structuredContent.digest,
    });
  }
  if (name === currentTool.name) {
    return JSON.stringify({
      schema_version: structuredContent.schema_version,
      scope: structuredContent.scope,
      decision_count: structuredContent.decisions.length,
      truncated: structuredContent.truncated,
      revision: structuredContent.revision,
      digest: structuredContent.digest,
    });
  }
  if (name === readTool.name) {
    return JSON.stringify({
      schema_version: structuredContent.schema_version,
      decision_id: structuredContent.decision.id,
      lineage_count: structuredContent.lineage.length,
      revision: structuredContent.revision,
      digest: structuredContent.digest,
    });
  }
  return JSON.stringify({
    schema_version: structuredContent.schema_version,
    scope: structuredContent.scope,
    match_count: structuredContent.decisions.length,
    truncated: structuredContent.truncated,
    revision: structuredContent.revision,
    digest: structuredContent.digest,
  });
}

export function runControlDecisionMcp({
  input = process.stdin,
  output = process.stdout,
  databasePath = configuredDatabasePath(),
} = {}) {
  const store = new LatticeStore(databasePath);
  const service = new ControlDecisionService({ store });
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
      throw new Error("CONTROL_DECISION_MCP_RESPONSE_LIMIT_EXCEEDED");
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
    if (
      !exactObject(request, ["jsonrpc", "method"], ["id", "params"])
      || request.jsonrpc !== "2.0"
      || typeof request.method !== "string"
    ) {
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
      if (initializeSeen || !validInitializeParams(request.params)) {
        failure(request.id, -32602, "Invalid initialize params");
        return;
      }
      initializeSeen = true;
      success(request.id, {
        protocolVersion: supportedProtocolVersions.has(request.params.protocolVersion)
          ? request.params.protocolVersion
          : protocolVersion,
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: "lattice-control-decision-core", version: "1.0.0" },
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
    const valid = (name === recordTool.name && validateRecordArguments(args))
      || (name === currentTool.name && validateCurrentArguments(args))
      || (name === readTool.name && validateReadArguments(args))
      || (name === searchTool.name && validateSearchArguments(args));
    if (!valid) {
      toolFailure(
        request.id,
        "CONTROL_DECISION_TOOL_ARGUMENTS_REJECTED",
        "Tool arguments failed validation",
      );
      return;
    }
    if (!reserveToolCall()) {
      toolFailure(
        request.id,
        "CONTROL_DECISION_TOOL_RATE_LIMITED",
        "Tool call rate limit exceeded; retry after the rolling window",
      );
      return;
    }
    try {
      let structuredContent;
      if (name === recordTool.name) {
        structuredContent = service.record({
          scope: args.scope,
          subject: args.subject,
          content: args.content,
          rationale: args.rationale,
          source: args.source,
          supersedesDecisionId: args.supersedes_decision_id,
          clientRequestId: args.client_request_id,
          expectedRevision: args.revision,
          expectedDigest: args.digest,
        });
      } else if (name === currentTool.name) {
        structuredContent = service.current({
          scope: args.scope,
          subject: args.subject,
          limit: args.limit,
        });
      } else if (name === readTool.name) {
        structuredContent = service.read({
          decisionId: args.decision_id,
          maxDepth: args.max_depth,
          expectedRevision: args.revision,
          expectedDigest: args.digest,
        });
      } else {
        structuredContent = service.search({
          scope: args.scope,
          query: args.query,
          limit: args.limit,
          expectedRevision: args.revision,
          expectedDigest: args.digest,
        });
      }
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
    runControlDecisionMcp();
  } catch {
    process.stderr.write("LATTICE_CONTROL_DECISION_MCP_START_FAILED\n");
    process.exitCode = 1;
  }
}
