import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import test from "node:test";
import { ControlDecisionService } from "../src/decision-core-service.mjs";
import { LatticeStore } from "../src/store.mjs";

function startMcp(databasePath) {
  const child = spawn(process.execPath, [
    "--input-type=module", "-e",
    "import {runControlDecisionMcp} from './apps/lattice-control/src/decision-core-mcp.mjs'; runControlDecisionMcp({databasePath:process.env.LATTICE_CONTROL_DATABASE_PATH});",
  ], {
    cwd: path.resolve(import.meta.dirname, "../../.."),
    env: { ...process.env, LATTICE_CONTROL_DATABASE_PATH: databasePath },
    stdio: ["pipe", "pipe", "pipe"],
    windowsHide: true,
  });
  const pending = new Map();
  const stderr = [];
  createInterface({ input: child.stdout, crlfDelay: Infinity }).on("line", (line) => {
    const message = JSON.parse(line);
    const waiter = pending.get(message.id);
    if (!waiter) throw new Error(`unexpected MCP response ${line}`);
    pending.delete(message.id);
    waiter.resolve(message);
  });
  child.stderr.on("data", (chunk) => stderr.push(chunk));
  const rejectPending = (error) => {
    for (const waiter of pending.values()) waiter.reject(error);
    pending.clear();
  };
  child.once("error", rejectPending);
  child.once("exit", (code) => {
    if (pending.size > 0) rejectPending(new Error(`MCP child exited before response (${code})`));
  });
  return {
    child,
    stderr,
    request(message) {
      const response = new Promise((resolve, reject) => pending.set(message.id, { resolve, reject }));
      child.stdin.write(`${JSON.stringify(message)}\n`);
      return response;
    },
    notify(message) {
      child.stdin.write(`${JSON.stringify(message)}\n`);
    },
  };
}

function decisionArguments(overrides = {}) {
  return {
    scope: "product:lattice",
    subject: "execution.adapter",
    content: "Ordinary work uses disposable execution workers.",
    rationale: "The Control store retains decisions while adapters stay replaceable.",
    source: {
      kind: "user_confirmation",
      reference: "thread:decision-mcp/turn:confirmed-1",
    },
    client_request_id: "decision-mcp-record-1",
    revision: 0,
    digest: "0".repeat(64),
    ...overrides,
  };
}

test("Control decision MCP records, supersedes, reads, and searches a real store over direct STDIO", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-decision-mcp-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  let session;
  try {
    store = new LatticeStore(databasePath);
    const service = new ControlDecisionService({ store });
    const initial = service.current({ scope: "product:lattice", limit: 10 });
    store.close();
    store = null;

    session = startMcp(databasePath);
    const initialize = await session.request({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        capabilities: {},
        clientInfo: { name: "decision-core-test", version: "1" },
      },
    });
    assert.equal(initialize.result.serverInfo.name, "lattice-control-decision-core");
    session.notify({ jsonrpc: "2.0", method: "notifications/initialized", params: {} });

    const listed = await session.request({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/list",
      params: {},
    });
    assert.deepEqual(
      listed.result.tools.map(({ name }) => name),
      [
        "lattice_control_decision_record",
        "lattice_control_decision_current",
        "lattice_control_decision_read",
        "lattice_control_decision_search",
      ],
    );
    const recordSourceSchema = listed.result.tools.find(
      ({ name }) => name === "lattice_control_decision_record",
    ).inputSchema.properties.source;
    assert.deepEqual(
      recordSourceSchema.oneOf.map((entry) => entry.properties.kind.const),
      ["user_confirmation", "approved_document"],
    );
    assert.match(
      decisionArguments().source.reference,
      new RegExp(recordSourceSchema.oneOf[0].properties.reference.pattern, "u"),
    );
    assert.doesNotMatch(
      "thread:x",
      new RegExp(recordSourceSchema.oneOf[0].properties.reference.pattern, "u"),
    );
    const approvedReferenceSchema = recordSourceSchema.oneOf[1].properties.reference;
    const patternValidButTooLong = `file:${"a".repeat(384)}#${"b".repeat(123)}`;
    assert.equal(approvedReferenceSchema.minLength, 1);
    assert.equal(approvedReferenceSchema.maxLength, 512);
    assert.equal(patternValidButTooLong.length, 513);
    assert.match(
      patternValidButTooLong,
      new RegExp(approvedReferenceSchema.pattern, "u"),
    );
    assert.equal(patternValidButTooLong.length <= approvedReferenceSchema.maxLength, false);
    const extraEnvelopeField = await session.request({
      jsonrpc: "2.0",
      id: 20,
      method: "tools/list",
      params: {},
      unexpected_top_level: true,
    });
    assert.equal(extraEnvelopeField.error.code, -32600);

    const firstResponse = await session.request({
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_record",
        arguments: decisionArguments({ revision: initial.revision, digest: initial.digest }),
      },
    });
    assert.equal(firstResponse.result.isError, false);
    const first = firstResponse.result.structuredContent;
    assert.equal(first.changed, true);
    assert.equal(first.revision, 1);
    const conflictingReplay = await session.request({
      jsonrpc: "2.0",
      id: 21,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_record",
        arguments: decisionArguments({
          revision: Number.MAX_SAFE_INTEGER,
          digest: "f".repeat(64),
        }),
      },
    });
    assert.equal(conflictingReplay.result.isError, true);
    assert.equal(
      conflictingReplay.result.structuredContent.error.code,
      "DECISION_IDEMPOTENCY_CONFLICT",
    );

    const replacementResponse = await session.request({
      jsonrpc: "2.0",
      id: 4,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_record",
        arguments: decisionArguments({
          content: "Ordinary work uses disposable workers selected by current configuration.",
          rationale: "Do not bind a durable decision to one replaceable adapter release.",
          supersedes_decision_id: first.decision.id,
          client_request_id: "decision-mcp-record-2",
          revision: first.revision,
          digest: first.digest,
        }),
      },
    });
    assert.equal(replacementResponse.result.isError, false);
    const replacement = replacementResponse.result.structuredContent;
    assert.equal(replacement.decision.supersedes_decision_id, first.decision.id);

    const currentResponse = await session.request({
      jsonrpc: "2.0",
      id: 5,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_current",
        arguments: { scope: "product:lattice", limit: 10 },
      },
    });
    const current = currentResponse.result.structuredContent;
    assert.equal(current.schema_version, "lattice.control.current-decisions-packet.v1");
    assert.deepEqual(current.decisions.map(({ id }) => id), [replacement.decision.id]);

    const searchResponse = await session.request({
      jsonrpc: "2.0",
      id: 6,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_search",
        arguments: {
          scope: "product:lattice",
          query: "disposable",
          limit: 10,
          revision: current.revision,
          digest: current.digest,
        },
      },
    });
    assert.equal(searchResponse.result.isError, false);
    assert.equal(searchResponse.result.structuredContent.decisions.length, 2);

    const readResponse = await session.request({
      jsonrpc: "2.0",
      id: 7,
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
    });
    assert.equal(readResponse.result.isError, false);
    assert.deepEqual(
      readResponse.result.structuredContent.lineage.map(({ id }) => id),
      [first.decision.id, replacement.decision.id],
    );

    const transcriptRejected = await session.request({
      jsonrpc: "2.0",
      id: 8,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_record",
        arguments: decisionArguments({
          client_request_id: "decision-mcp-record-3",
          revision: current.revision,
          digest: current.digest,
          transcript: "complete chat text",
        }),
      },
    });
    assert.equal(transcriptRejected.result.isError, true);
    assert.equal(
      transcriptRejected.result.structuredContent.error.code,
      "CONTROL_DECISION_TOOL_ARGUMENTS_REJECTED",
    );

    const unboundedSearch = await session.request({
      jsonrpc: "2.0",
      id: 9,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_search",
        arguments: {
          scope: "product:lattice",
          query: "worker",
          revision: current.revision,
          digest: current.digest,
        },
      },
    });
    assert.equal(unboundedSearch.result.isError, true);
    assert.equal(
      unboundedSearch.result.structuredContent.error.code,
      "CONTROL_DECISION_TOOL_ARGUMENTS_REJECTED",
    );

    const staleRead = await session.request({
      jsonrpc: "2.0",
      id: 10,
      method: "tools/call",
      params: {
        name: "lattice_control_decision_read",
        arguments: {
          decision_id: first.decision.id,
          max_depth: 10,
          revision: current.revision - 1,
          digest: current.digest,
        },
      },
    });
    assert.equal(staleRead.result.isError, true);
    assert.equal(staleRead.result.structuredContent.error.code, "DECISION_REVISION_MISMATCH");

    session.child.stdin.end();
    const [exitCode] = await once(session.child, "exit");
    assert.equal(exitCode, 0);
    assert.equal(Buffer.concat(session.stderr).toString("utf8"), "");
    session = null;

    store = new LatticeStore(databasePath);
    const replay = new ControlDecisionService({ store }).current({
      scope: "product:lattice",
      limit: 10,
    });
    assert.equal(replay.revision, current.revision);
    assert.equal(replay.digest, current.digest);
    assert.deepEqual(replay.decisions.map(({ id }) => id), [replacement.decision.id]);
  } finally {
    if (session && session.child.exitCode == null) session.child.kill();
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});
