import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { createInterface } from "node:readline";
import { ControlDecisionService } from "../apps/lattice-control/src/decision-core-service.mjs";
import { LatticeStore } from "../apps/lattice-control/src/store.mjs";

const requestTimeoutMs = 5_000;
const maximumObservedOutputBytes = 2_097_152;

function childEnvironment(databasePath) {
  const environment = { LATTICE_CONTROL_DATABASE_PATH: databasePath };
  for (const key of ["SystemRoot", "WINDIR", "COMSPEC", "PATH", "PATHEXT", "TEMP", "TMP"]) {
    if (process.env[key]) environment[key] = process.env[key];
  }
  return environment;
}

function startSession(databasePath) {
  const child = spawn(process.execPath, [
    path.resolve(import.meta.dirname, "../apps/lattice-control/src/decision-core-mcp.mjs"),
  ], {
    env: childEnvironment(databasePath),
    stdio: ["pipe", "pipe", "pipe"],
    windowsHide: true,
  });
  const pending = new Map();
  const stderr = [];
  let stdoutBytes = 0;
  let maximumFrameBytes = 0;
  let responseCount = 0;
  child.stderr.on("data", (chunk) => stderr.push(chunk));
  const rejectPending = (error) => {
    for (const waiter of pending.values()) {
      clearTimeout(waiter.timeout);
      waiter.reject(error);
    }
    pending.clear();
  };
  createInterface({ input: child.stdout, crlfDelay: Infinity }).on("line", (line) => {
    const frameBytes = Buffer.byteLength(line, "utf8");
    stdoutBytes += frameBytes + 1;
    maximumFrameBytes = Math.max(maximumFrameBytes, frameBytes);
    if (stdoutBytes > maximumObservedOutputBytes) {
      child.kill();
      rejectPending(new Error("MCP acceptance output exceeded its bound"));
      return;
    }
    let response;
    try {
      response = JSON.parse(line);
    } catch {
      rejectPending(new Error("MCP acceptance received non-JSON stdout"));
      return;
    }
    responseCount += 1;
    const waiter = pending.get(response.id);
    if (!waiter) {
      rejectPending(new Error("MCP acceptance received an unexpected response ID"));
      return;
    }
    clearTimeout(waiter.timeout);
    pending.delete(response.id);
    waiter.resolve(response);
  });
  child.once("error", rejectPending);
  child.once("exit", (code) => {
    if (pending.size > 0) rejectPending(new Error(`MCP child exited early (${code})`));
  });
  return {
    child,
    stderr,
    metrics() { return { stdoutBytes, maximumFrameBytes, responseCount }; },
    request(message) {
      const response = new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          pending.delete(message.id);
          reject(new Error(`MCP request ${message.id} timed out`));
        }, requestTimeoutMs);
        pending.set(message.id, { resolve, reject, timeout });
      });
      child.stdin.write(`${JSON.stringify(message)}\n`);
      return response;
    },
    notify(message) {
      child.stdin.write(`${JSON.stringify(message)}\n`);
    },
  };
}

function requireSuccess(response, label) {
  if (!response || response.error || !response.result || response.result.isError) {
    throw new Error(`${label} did not return an MCP tool success response`);
  }
  return response.result;
}

function evidencePathFromArguments() {
  const index = process.argv.indexOf("--evidence");
  if (index < 0) return null;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error("--evidence requires a path");
  return path.resolve(value);
}

function recordArguments(state, overrides = {}) {
  return {
    scope: "product:lattice",
    subject: "decision.adapter",
    content: "Decision memory uses a standalone bounded MCP adapter.",
    rationale: "The Control store remains the only local product-state authority.",
    source: {
      kind: "user_confirmation",
      reference: "thread:01a039e4-6ef8-7252-84fa-45c2ea8e731d/delegation:input",
    },
    client_request_id: "decision-live-record-1",
    revision: state.revision,
    digest: state.digest,
    ...overrides,
  };
}

async function runAcceptance() {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-decision-core-live-"));
  const databasePath = path.join(directory, "control.db");
  const evidencePath = evidencePathFromArguments();
  let store;
  let session;
  try {
    store = new LatticeStore(databasePath);
    const seedService = new ControlDecisionService({ store });
    const initial = seedService.current({ scope: "product:lattice", limit: 10 });
    const schemaVersion = store.database.prepare("PRAGMA user_version").get().user_version;
    store.close();
    store = null;

    session = startSession(databasePath);
    const initialize = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        capabilities: {},
        clientInfo: { name: "lattice-control-decision-live-acceptance", version: "1" },
      },
    }), "initialize");
    session.notify({ jsonrpc: "2.0", method: "notifications/initialized", params: {} });
    const listed = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/list",
      params: {},
    }), "tools/list");

    const first = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_record",
        arguments: recordArguments(initial),
      },
    }), "record").structuredContent;
    const replacementArguments = recordArguments(first, {
      content: "Decision memory uses one standalone bounded MCP adapter supplied to Codex.",
      rationale: "The adapter is a connector and never a second source of truth.",
      supersedes_decision_id: first.decision.id,
      client_request_id: "decision-live-record-2",
    });
    const replacement = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 4,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_record",
        arguments: replacementArguments,
      },
    }), "supersede").structuredContent;
    const idempotentReplay = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 5,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_record",
        arguments: replacementArguments,
      },
    }), "idempotent replay").structuredContent;
    const current = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 6,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_current",
        arguments: { scope: "product:lattice", limit: 10 },
      },
    }), "current").structuredContent;
    const search = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 7,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_search",
        arguments: {
          scope: "product:lattice",
          query: "standalone",
          limit: 10,
          revision: current.revision,
          digest: current.digest,
        },
      },
    }), "search").structuredContent;
    const read = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 8,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_read",
        arguments: {
          decision_id: first.decision.id,
          max_depth: 10,
          revision: current.revision,
          digest: current.digest,
        },
      },
    }), "read").structuredContent;

    const toolNames = listed.tools.map(({ name }) => name);
    if (JSON.stringify(toolNames) !== JSON.stringify([
      "lattice_control_decision_record",
      "lattice_control_decision_current",
      "lattice_control_decision_read",
      "lattice_control_decision_search",
    ])) throw new Error("MCP decision tool catalog changed");
    if (
      first.changed !== true
      || replacement.changed !== true
      || idempotentReplay.changed !== false
      || idempotentReplay.decision.id !== replacement.decision.id
      || replacement.decision.supersedes_decision_id !== first.decision.id
      || current.decisions.length !== 1
      || current.decisions[0].id !== replacement.decision.id
      || search.decisions.length !== 2
      || read.lineage.length !== 2
      || read.lineage[0].id !== first.decision.id
      || read.lineage[1].id !== replacement.decision.id
      || read.revision !== current.revision
      || read.digest !== current.digest
      || search.revision !== current.revision
      || search.digest !== current.digest
    ) throw new Error("MCP decision state did not replay the seeded Control store");

    session.child.stdin.end();
    const [exitCode] = await once(session.child, "exit");
    const stderrBytes = Buffer.concat(session.stderr).length;
    const metrics = session.metrics();
    if (exitCode !== 0 || stderrBytes !== 0) throw new Error("MCP child did not close cleanly");
    session = null;

    store = new LatticeStore(databasePath);
    const replay = new ControlDecisionService({ store }).current({
      scope: "product:lattice",
      limit: 10,
    });
    if (
      replay.revision !== current.revision
      || replay.digest !== current.digest
      || replay.decisions.length !== 1
      || replay.decisions[0].id !== replacement.decision.id
    ) throw new Error("fresh-process Control decision replay changed identity");
    store.close();
    store = null;

    const evidence = {
      schema_version: "lattice.control.decision-core-mcp-acceptance.v1",
      generated_at: new Date().toISOString(),
      result: "PASS",
      transport: "DIRECT_STDIO_REAL_CHILD",
      source: {
        kind: "TEMP_CONTROL_SQLITE_REAL_STORE",
        control_schema_version: schemaVersion,
        authority: "CONTROL_LOCAL_PRODUCT_STATE",
        database_path_included: false,
      },
      protocol: {
        initialized: initialize.protocolVersion === "2025-11-25",
        server_name: initialize.serverInfo.name,
        tools_listed: toolNames,
        record_called: true,
        supersede_called: true,
        current_called: true,
        search_called: true,
        read_called: true,
      },
      mutation: {
        record_changed: first.changed,
        supersede_changed: replacement.changed,
        replay_changed: idempotentReplay.changed,
        retained_history_count: read.lineage.length,
        current_count: current.decisions.length,
        search_result_count: search.decisions.length,
      },
      shared_state_identity: {
        revision: current.revision,
        digest: current.digest,
        current_search_equal: (
          search.revision === current.revision && search.digest === current.digest
        ),
        current_read_equal: read.revision === current.revision && read.digest === current.digest,
        fresh_process_equal: replay.revision === current.revision && replay.digest === current.digest,
      },
      bounds: {
        request_timeout_ms: requestTimeoutMs,
        requested_current_limit: 10,
        requested_search_limit: 10,
        requested_lineage_depth: 10,
        response_count: metrics.responseCount,
        stdout_bytes: metrics.stdoutBytes,
        maximum_stdout_frame_bytes: metrics.maximumFrameBytes,
        stderr_bytes: stderrBytes,
      },
      process: { exit_code: exitCode },
    };
    const serialized = `${JSON.stringify(evidence, null, 2)}\n`;
    const forbiddenEvidence = [
      /-----BEGIN [A-Z ]*PRIVATE KEY-----/u,
      /\bBearer\s+[A-Za-z0-9._~-]{12,}\b/iu,
      /\b(?:password|passwd|pwd|api[_-]?key|access[_-]?token|refresh[_-]?token|otp)\s*[:=]/iu,
      /(?:^|\s)[A-Z][A-Z0-9_]{2,}\s*=\s*[^\s]+/mu,
    ];
    if (
      serialized.includes(databasePath)
      || forbiddenEvidence.some((pattern) => pattern.test(serialized))
      || Buffer.byteLength(serialized, "utf8") > 16_384
    ) throw new Error("acceptance evidence is unsafe or unbounded");
    if (evidencePath) {
      await mkdir(path.dirname(evidencePath), { recursive: true });
      await writeFile(evidencePath, serialized, { encoding: "utf8", flag: "w" });
    }
    process.stdout.write(serialized);
  } finally {
    if (session && session.child.exitCode == null) session.child.kill();
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
}

try {
  await runAcceptance();
} catch {
  process.stderr.write("LATTICE_CONTROL_DECISION_CORE_MCP_ACCEPTANCE_FAILED\n");
  process.exitCode = 1;
}
