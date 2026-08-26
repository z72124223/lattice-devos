import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { CodexAppServer } from "../apps/lattice-control/src/codex-app-server.mjs";
import { LatticeControlService } from "../apps/lattice-control/src/service.mjs";
import { LatticeStore } from "../apps/lattice-control/src/store.mjs";

const [inputArgument, outputArgument] = process.argv.slice(2);
if (!inputArgument || !outputArgument) {
  throw new Error("usage: codex-app-server-reconcile-child <input.json> <output.json>");
}

const inputPath = path.resolve(inputArgument);
const outputPath = path.resolve(outputArgument);
const startedAt = new Date().toISOString();
const notifications = [];
const diagnostics = [];
const requestSettlements = [];
const rpcTrace = [];
const idempotencyEventKinds = new Set([
  "codex_thread_accepted",
  "codex_thread_started",
  "codex_turn_accepted",
  "codex_started",
  "turn_completed",
  "codex_retry_claimed",
  "codex_retry_accepted",
  "codex_retry_started",
  "codex_reconciled",
]);
let store = null;
let service = null;
let codex = null;
let outcome = null;
let cleanupError = null;

function serializeError(error) {
  return {
    name: error?.name ?? "Error",
    message: String(error?.message ?? error),
    code: error?.code ?? null,
    method: error?.method ?? null,
    threadId: error?.threadId ?? null,
    turnId: error?.turnId ?? null,
    stack: typeof error?.stack === "string" ? error.stack.split(/\r?\n/u).slice(0, 12) : [],
  };
}

function assertRestart(condition, message) {
  if (!condition) throw new Error(message);
}

function eventCounts(events) {
  const byKind = {};
  for (const event of events) {
    if (idempotencyEventKinds.has(event.kind)) {
      byKind[event.kind] = (byKind[event.kind] ?? 0) + 1;
    }
  }
  return { total: Object.values(byKind).reduce((sum, count) => sum + count, 0), byKind };
}

function sameJson(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

async function sha256File(filePath) {
  return createHash("sha256").update(await readFile(filePath)).digest("hex");
}

try {
  const input = JSON.parse(await readFile(inputPath, "utf8"));
  assertRestart(path.isAbsolute(input.databasePath), "database path must be absolute");
  assertRestart(Number.isInteger(input.supervisorPid), "supervisor PID is missing");
  assertRestart(process.pid !== input.supervisorPid, "fresh Control reused the supervisor OS PID");
  assertRestart(Array.isArray(input.items) && input.items.length === 2, "two saved work items are required");
  const expectedRuntime = input.runtimeIdentity;
  const packagePath = path.join(path.dirname(path.dirname(expectedRuntime.codexScript)), "package.json");
  const packageJson = JSON.parse(await readFile(packagePath, "utf8"));
  const observedSchemaSha256 = Object.fromEntries(await Promise.all(
    Object.keys(expectedRuntime.schemaSha256).map(async (relative) => [
      relative,
      await sha256File(path.join(expectedRuntime.schemaPath, relative)),
    ]),
  ));
  const runtimeVerification = {
    packageVersion: packageJson.version,
    codexScript: path.join(
      process.env.APPDATA || "",
      "npm",
      "node_modules",
      "@openai",
      "codex",
      "bin",
      "codex.js",
    ),
    codexBinarySha256: await sha256File(expectedRuntime.codexBinary),
    appServerCommand: expectedRuntime.codexBinary,
    schemaSha256: observedSchemaSha256,
  };
  assertRestart(runtimeVerification.packageVersion === expectedRuntime.packageVersion, "Codex package version changed");
  assertRestart(runtimeVerification.codexScript === expectedRuntime.codexScript, "Codex script path changed");
  assertRestart(
    runtimeVerification.codexBinarySha256 === expectedRuntime.codexBinarySha256,
    "Codex binary hash changed",
  );
  assertRestart(
    sameJson(runtimeVerification.schemaSha256, expectedRuntime.schemaSha256),
    "Codex App Server schema hashes changed",
  );

  store = new LatticeStore(input.databasePath);
  codex = new CodexAppServer({
    codexBin: expectedRuntime.codexBinary,
    requestTimeoutMs: 30_000,
    lifecycleTimeoutMs: input.lifecycleTimeoutMs,
  });
  codex.on("notification", (message, entry) => notifications.push({
    generation: 2,
    sequence: entry.sequence,
    observedAt: entry.observedAt,
    controlPid: process.pid,
    message: structuredClone(message),
  }));
  codex.on("diagnostic", (text) => diagnostics.push({
    generation: 2,
    observedAt: new Date().toISOString(),
    controlPid: process.pid,
    text,
  }));
  codex.on("serverRequestSettled", (settlement) => requestSettlements.push({
    generation: 2,
    observedAt: new Date().toISOString(),
    controlPid: process.pid,
    ...structuredClone(settlement),
  }));
  const request = codex.request.bind(codex);
  codex.request = async (method, params = {}, options = {}) => {
    const trace = {
      method,
      threadId: params?.threadId ?? null,
      startedAt: new Date().toISOString(),
      status: "pending",
    };
    rpcTrace.push(trace);
    try {
      const result = await request(method, params, options);
      Object.assign(trace, { status: "accepted", completedAt: new Date().toISOString() });
      return result;
    } catch (error) {
      Object.assign(trace, {
        status: "failed",
        completedAt: new Date().toISOString(),
        error: serializeError(error),
      });
      throw error;
    }
  };

  service = new LatticeControlService({
    store,
    codex,
    model: input.model,
    lifecycleTimeoutMs: input.lifecycleTimeoutMs,
    approvalTimeoutMs: 30_000,
    threadOptions: {
      approvalPolicy: "never",
      sandbox: "read-only",
      ephemeral: false,
      serviceName: "lattice_control_lifecycle_acceptance_restart",
      developerInstructions: [
        "This process only reconciles completed lifecycle acceptance threads.",
        "Do not start a turn, modify files, or ask for user input.",
      ].join(" "),
      config: { model_reasoning_effort: "low" },
    },
  });

  const before = input.items.map((expected) => {
    const item = service.workItem(expected.workItemId);
    assertRestart(item.item.status === "codex_done", expected.workItemId + " was not durable codex_done");
    assertRestart(item.item.codex_thread_id === expected.threadId, "saved thread ID changed before restart");
    assertRestart(item.item.codex_turn_id === expected.turnId, "saved turn ID changed before restart");
    const counts = eventCounts(item.events);
    assertRestart(sameJson(counts, expected.eventCounts), "durable events changed before reconciliation");
    return {
      workItemId: item.item.id,
      status: item.item.status,
      threadId: item.item.codex_thread_id,
      turnId: item.item.codex_turn_id,
      eventCounts: counts,
    };
  });

  const reconciled = await Promise.all(input.items.map(({ workItemId }) => service.reconcile(workItemId)));
  const threads = await Promise.all(input.items.map(({ threadId }) => codex.readThread(threadId)));
  const after = input.items.map((expected, index) => {
    const item = service.workItem(expected.workItemId);
    const thread = threads[index];
    const turnIds = thread.turns.map(({ id }) => id);
    assertRestart(reconciled[index].status === "codex_done", "fresh Control changed completed status");
    assertRestart(item.item.codex_thread_id === expected.threadId, "fresh Control changed saved thread ID");
    assertRestart(item.item.codex_turn_id === expected.turnId, "fresh Control changed saved turn ID");
    assertRestart(thread.turns.at(-1)?.id === expected.turnId, "fresh rollout latest turn did not match saved turn");
    assertRestart(thread.turns.at(-1)?.status === "completed", "fresh rollout latest turn was not completed");
    assertRestart(sameJson(turnIds, expected.turnIds), "fresh Control added or removed a turn");
    const counts = eventCounts(item.events);
    assertRestart(sameJson(counts, expected.eventCounts), "fresh Control duplicated a durable event");
    return {
      workItemId: item.item.id,
      status: item.item.status,
      threadId: item.item.codex_thread_id,
      turnId: item.item.codex_turn_id,
      turnIds,
      eventCounts: counts,
    };
  });

  assertRestart(!rpcTrace.some(({ method }) => method === "turn/start"), "fresh Control replayed a completed turn");
  for (const expected of input.items) {
    assertRestart(
      rpcTrace.some(({ method, threadId }) => method === "thread/resume" && threadId === expected.threadId),
      "fresh Control did not resume " + expected.threadId,
    );
    assertRestart(
      rpcTrace.some(({ method, threadId }) => method === "thread/read" && threadId === expected.threadId),
      "fresh Control did not read " + expected.threadId,
    );
    assertRestart(
      !notifications.some(({ message }) => (
        message.method === "turn/started"
        && message.params?.threadId === expected.threadId
      )),
      "fresh Control observed a replayed turn/started for " + expected.threadId,
    );
  }

  outcome = {
    status: "PASS",
    supervisorPid: input.supervisorPid,
    controlPid: process.pid,
    parentPid: process.ppid,
    previousAppServerPid: input.previousAppServerPid,
    appServerPid: codex.process?.pid ?? null,
    startedAt,
    before,
    after,
    rpcTrace,
    notifications,
    diagnostics,
    requestSettlements,
    runtimeVerification,
    finalSnapshot: {
      workItems: store.listWorkItems(),
      events: Object.fromEntries(store.listWorkItems().map((item) => [
        item.id,
        store.listEvents(item.id),
      ])),
    },
  };
  assertRestart(
    Number.isInteger(outcome.appServerPid) && outcome.appServerPid !== input.previousAppServerPid,
    "fresh Control did not own a different App Server PID",
  );
} catch (error) {
  outcome = {
    status: "FAIL",
    supervisorPid: null,
    controlPid: process.pid,
    parentPid: process.ppid,
    startedAt,
    rpcTrace,
    notifications,
    diagnostics,
    requestSettlements,
    error: serializeError(error),
  };
} finally {
  try {
    service?.close();
    if (codex) await codex.close();
    if (store) {
      store.database.exec("PRAGMA wal_checkpoint(TRUNCATE);");
      store.close();
    }
  } catch (error) {
    cleanupError = serializeError(error);
  }
}

outcome.completedAt = new Date().toISOString();
outcome.cleanup = {
  error: cleanupError,
  pendingRpc: codex?.pendingRequestCount ?? 0,
  pendingNotifications: codex?.pendingNotificationCount ?? 0,
  pendingServerRequests: codex?.pendingServerRequestCount ?? 0,
};
if (cleanupError) outcome.status = "FAIL";
await writeFile(outputPath, JSON.stringify(outcome, null, 2) + "\n", "utf8");
process.exitCode = outcome.status === "PASS" ? 0 : 1;
