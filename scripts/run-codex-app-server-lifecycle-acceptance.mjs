import { execFileSync, spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { CodexAppServer } from "../apps/lattice-control/src/codex-app-server.mjs";
import { LatticeControlService } from "../apps/lattice-control/src/service.mjs";
import { LatticeStore } from "../apps/lattice-control/src/store.mjs";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const evidencePath = path.join(
  repositoryRoot,
  "docs",
  "reviews",
  "CODEX_APP_SERVER_LIFECYCLE_ACCEPTANCE_2026-08-26.json",
);
const runId = `codex-lifecycle-${new Date().toISOString().replaceAll(":", "-")}-${randomUUID().slice(0, 8)}`;
const artifactRoot = path.join(repositoryRoot, ".lattice", "acceptance", runId);
const databasePath = path.join(artifactRoot, "control.db");
const schemaPath = path.join(artifactRoot, "app-server-schema");
const restartChildPath = path.join(repositoryRoot, "scripts", "codex-app-server-reconcile-child.mjs");
const restartInputPath = path.join(artifactRoot, "restart-input.json");
const restartOutputPath = path.join(artifactRoot, "restart-output.json");
const supersededEvidencePath = path.join(
  repositoryRoot,
  "docs",
  "reviews",
  "CODEX_APP_SERVER_LIFECYCLE_ACCEPTANCE_2026-08-26_IN_PROCESS_ATTEMPT.json",
);
const model = "gpt-5.6-luna";
const lifecycleTimeoutMs = 120_000;
const startedAt = new Date().toISOString();
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

const notifications = [];
const diagnostics = [];
const requestSettlements = [];
const gates = ["A", "B", "C", "D", "E", "F"].map((id) => ({
  id,
  status: "NOT RUN",
  evidence: {},
}));
let store;
let service;
let codex;
let finalSnapshot = null;
let topLevelError = null;
let priorCanonicalEvidence = null;

function gate(id) {
  return gates.find((candidate) => candidate.id === id);
}

function mark(id, status, evidence = {}) {
  Object.assign(gate(id), { status, evidence });
}

function serializeError(error) {
  return {
    name: error?.name ?? "Error",
    message: String(error?.message ?? error),
    code: error?.code ?? null,
    method: error?.method ?? null,
    threadId: error?.threadId ?? null,
    turnId: error?.turnId ?? null,
    pid: error?.pid ?? null,
    cleanup: error?.cleanup ?? null,
    stack: typeof error?.stack === "string" ? error.stack.split(/\r?\n/u).slice(0, 12) : [],
  };
}

function assertGate(condition, message) {
  if (!condition) throw new Error(message);
}

async function bounded(promise, label, timeoutMs = lifecycleTimeoutMs) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((resolve, reject) => {
        timer = setTimeout(() => {
          const error = new Error(`${label} timed out after ${timeoutMs}ms`);
          error.code = "CODEX_ACCEPTANCE_TIMEOUT";
          reject(error);
        }, timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

async function sha256File(filePath) {
  return createHash("sha256").update(await readFile(filePath)).digest("hex");
}

async function optionalSha256File(filePath) {
  try {
    return await sha256File(filePath);
  } catch {
    return null;
  }
}

async function preserveCanonicalEvidence() {
  const canonicalSha256 = await optionalSha256File(evidencePath);
  if (!canonicalSha256) return { status: "NONE", path: null, sha256: null };
  const supersededSha256 = await optionalSha256File(supersededEvidencePath);
  if (supersededSha256 === canonicalSha256) {
    return {
      status: "ALREADY_PRESERVED",
      path: supersededEvidencePath,
      sha256: supersededSha256,
    };
  }
  const backupPath = path.join(
    path.dirname(evidencePath),
    "CODEX_APP_SERVER_LIFECYCLE_ACCEPTANCE_2026-08-26_PRIOR_" + runId + ".json",
  );
  await copyFile(evidencePath, backupPath, fsConstants.COPYFILE_EXCL);
  const backupSha256 = await sha256File(backupPath);
  if (backupSha256 !== canonicalSha256) throw new Error("canonical evidence backup hash mismatch");
  return { status: "COPIED", path: backupPath, sha256: backupSha256 };
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

function commandText(command, args = []) {
  return execFileSync(command, args, {
    cwd: repositoryRoot,
    encoding: "utf8",
    windowsHide: true,
  }).trim();
}

function attachCapture(client, generation) {
  client.on("notification", (message, entry) => notifications.push({
    generation,
    sequence: entry.sequence,
    observedAt: entry.observedAt,
    message: structuredClone(message),
  }));
  client.on("diagnostic", (text) => diagnostics.push({
    generation,
    observedAt: new Date().toISOString(),
    text,
  }));
  client.on("serverRequestSettled", (settlement) => requestSettlements.push({
    generation,
    observedAt: new Date().toISOString(),
    ...structuredClone(settlement),
  }));
}

function createRuntime(generation) {
  const client = new CodexAppServer({
    codexBin: codexBinary,
    requestTimeoutMs: 30_000,
    lifecycleTimeoutMs,
  });
  attachCapture(client, generation);
  const latticeService = new LatticeControlService({
    store,
    codex: client,
    model,
    lifecycleTimeoutMs,
    approvalTimeoutMs: 30_000,
    threadOptions: {
      approvalPolicy: "never",
      sandbox: "read-only",
      ephemeral: false,
      serviceName: "lattice_control_lifecycle_acceptance",
      developerInstructions: [
        "This is a bounded read-only lifecycle acceptance.",
        "Follow the work objective exactly, do not modify files, and do not ask for user input.",
      ].join(" "),
      config: { model_reasoning_effort: "low" },
    },
  });
  return { client, latticeService };
}

function waitForProcessExit(child, label, timeoutMs = 15_000) {
  if (!child || child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve({ code: child?.exitCode ?? null, signal: child?.signalCode ?? null });
  }
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timer);
      child.removeListener("exit", onExit);
    };
    const onExit = (code, signal) => {
      cleanup();
      resolve({ code, signal });
    };
    const timer = setTimeout(() => {
      cleanup();
      const error = new Error(label + " did not exit within " + timeoutMs + "ms");
      error.code = "CODEX_ACCEPTANCE_PROCESS_EXIT_TIMEOUT";
      error.pid = child.pid;
      reject(error);
    }, timeoutMs);
    child.once("exit", onExit);
  });
}

function terminateOwnedProcessTree(pid) {
  if (!Number.isInteger(pid) || pid <= 0) {
    return { attempted: false, succeeded: false, error: "invalid owned PID" };
  }
  try {
    if (process.platform === "win32") {
      execFileSync("taskkill.exe", ["/PID", String(pid), "/T", "/F"], {
        stdio: "ignore",
        windowsHide: true,
      });
    } else {
      process.kill(pid, "SIGKILL");
    }
    return { attempted: true, succeeded: true, error: null };
  } catch (error) {
    return {
      attempted: true,
      succeeded: !isProcessAlive(pid),
      error: serializeError(error),
    };
  }
}

function isProcessAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function closeRuntime() {
  const ownedAppServer = codex?.process ?? null;
  const ownedAppServerPid = ownedAppServer?.pid ?? null;
  let closeError = null;
  service?.close();
  service = null;
  if (codex) await codex.close();
  codex = null;
  if (ownedAppServer && ownedAppServer.exitCode === null && ownedAppServer.signalCode === null) {
    try {
      await waitForProcessExit(ownedAppServer, "owned Codex App Server");
    } catch (error) {
      const termination = terminateOwnedProcessTree(ownedAppServerPid);
      let forcedExitError = null;
      try {
        await waitForProcessExit(ownedAppServer, "forced Codex App Server cleanup", 5_000);
      } catch (cleanupFailure) {
        forcedExitError = serializeError(cleanupFailure);
      }
      error.cleanup = {
        termination,
        forcedExitError,
        processAlive: isProcessAlive(ownedAppServerPid),
      };
      closeError = error;
    }
  }
  if (store) {
    store.database.exec("PRAGMA wal_checkpoint(TRUNCATE);");
    store.close();
  }
  store = null;
  if (closeError) throw closeError;
  return {
    appServerPid: ownedAppServerPid,
    appServerExitedAt: ownedAppServerPid ? new Date().toISOString() : null,
  };
}

async function runFreshControlProcess(payload) {
  await writeFile(restartInputPath, JSON.stringify(payload, null, 2) + "\n", "utf8");
  const child = spawn(process.execPath, [restartChildPath, restartInputPath, restartOutputPath], {
    cwd: repositoryRoot,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  const spawnedPid = child.pid;
  return new Promise((resolve, reject) => {
    let stdout = "";
    let stderr = "";
    let settled = false;
    const append = (current, chunk) => (current + String(chunk)).slice(-65_536);
    child.stdout.on("data", (chunk) => {
      stdout = append(stdout, chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderr = append(stderr, chunk);
    });
    const cleanup = () => {
      clearTimeout(timer);
      child.removeListener("error", onError);
      child.removeListener("exit", onExit);
    };
    const finish = (callback) => {
      if (settled) return;
      settled = true;
      cleanup();
      callback();
    };
    const onError = (error) => finish(() => reject(error));
    const onExit = (code, signal) => finish(async () => {
      try {
        const result = JSON.parse(await readFile(restartOutputPath, "utf8"));
        resolve({ spawnedPid, exitCode: code, signal, stdout, stderr, ...result });
      } catch (error) {
        error.message = "fresh Control result could not be read: " + error.message;
        error.child = { spawnedPid, exitCode: code, signal, stdout, stderr };
        reject(error);
      }
    });
    const timer = setTimeout(async () => {
      if (settled) return;
      settled = true;
      cleanup();
      const termination = terminateOwnedProcessTree(spawnedPid);
      let exitError = null;
      try {
        await waitForProcessExit(child, "timed-out fresh Control cleanup", 5_000);
      } catch (cleanupFailure) {
        exitError = serializeError(cleanupFailure);
      }
      const processAlive = isProcessAlive(spawnedPid);
      const error = new Error("fresh Control process timed out after " + (lifecycleTimeoutMs + 15_000) + "ms");
      error.code = termination.succeeded && !exitError && !processAlive
        ? "CODEX_ACCEPTANCE_PROCESS_TIMEOUT"
        : "CODEX_ACCEPTANCE_PROCESS_CLEANUP_FAILED";
      error.pid = spawnedPid;
      error.cleanup = { termination, exitError, processAlive };
      reject(error);
    }, lifecycleTimeoutMs + 15_000);
    child.once("error", onError);
    child.once("exit", onExit);
  });
}

function exactNotification(method, threadId, turnId, generation = null) {
  return notifications.filter((entry) => (
    (generation === null || entry.generation === generation)
    && entry.message.method === method
    && entry.message.params?.threadId === threadId
    && entry.message.params?.turn?.id === turnId
  ));
}

function itemEvidence(item) {
  return {
    workItemId: item.id,
    status: item.status,
    threadId: item.codex_thread_id,
    turnId: item.codex_turn_id,
  };
}

await mkdir(path.join(artifactRoot, "workspace-fast"), { recursive: true });
await mkdir(path.join(artifactRoot, "workspace-interrupt"), { recursive: true });
await mkdir(path.dirname(evidencePath), { recursive: true });
priorCanonicalEvidence = await preserveCanonicalEvidence();

const codexPackageRoot = path.join(
  process.env.APPDATA || "",
  "npm",
  "node_modules",
  "@openai",
  "codex",
);
const codexScript = path.join(codexPackageRoot, "bin", "codex.js");
const codexBinary = path.join(
  codexPackageRoot,
  "node_modules",
  "@openai",
  "codex-win32-x64",
  "vendor",
  "x86_64-pc-windows-msvc",
  "bin",
  "codex.exe",
);
let runtimeIdentity = {};

try {
  const codexPackage = JSON.parse(await readFile(path.join(codexPackageRoot, "package.json"), "utf8"));
  execFileSync(process.execPath, [
    codexScript,
    "app-server",
    "generate-json-schema",
    "--out",
    schemaPath,
  ], { cwd: repositoryRoot, stdio: "pipe", windowsHide: true });
  const schemaFiles = [
    "ServerRequest.json",
    "v2/ThreadReadParams.json",
    "v2/TurnStartedNotification.json",
    "v2/TurnCompletedNotification.json",
    "v2/McpServerStatusUpdatedNotification.json",
  ];
  runtimeIdentity = {
    packageVersion: codexPackage.version,
    codexScript,
    codexBinary,
    codexBinarySha256: await sha256File(codexBinary),
    schemaPath,
    schemaSha256: Object.fromEntries(await Promise.all(schemaFiles.map(async (relative) => [
      relative,
      await sha256File(path.join(schemaPath, relative)),
    ]))),
  };

  store = new LatticeStore(databasePath);
  ({ client: codex, latticeService: service } = createRuntime(1));
  const fastProject = service.createProject({
    name: "Codex lifecycle fast agent",
    rootPath: path.join(artifactRoot, "workspace-fast"),
  });
  const interruptProject = service.createProject({
    name: "Codex lifecycle interrupt agent",
    rootPath: path.join(artifactRoot, "workspace-interrupt"),
  });
  const fastWork = service.createWorkItem({
    projectId: fastProject.id,
    title: "Real fast lifecycle agent",
    objective: [
      "Execute exactly one PowerShell command: Start-Sleep -Seconds 12.",
      "After it exits, reply with exactly LATTICE_FAST_COMPLETED.",
      "Do not inspect or modify files and do not perform any other action.",
    ].join(" "),
  });
  const interruptWork = service.createWorkItem({
    projectId: interruptProject.id,
    title: "Real interrupt lifecycle agent",
    objective: [
      "Execute exactly one PowerShell command: Start-Sleep -Seconds 90.",
      "After it exits, reply with exactly LATTICE_LONG_COMPLETED.",
      "Do not inspect or modify files and do not perform any other action.",
    ].join(" "),
  });

  let fastActive;
  let interruptActive;
  try {
    [fastActive, interruptActive] = await bounded(Promise.all([
      service.start(fastWork.id),
      service.start(interruptWork.id),
    ]), "two real agents reaching turn/started");
    const exactStarts = [fastActive, interruptActive].map((item) => exactNotification(
      "turn/started",
      item.codex_thread_id,
      item.codex_turn_id,
      1,
    ).at(-1));
    assertGate(exactStarts.every(Boolean), "both exact turn/started notifications were not observed");
    assertGate(
      fastActive.codex_thread_id !== interruptActive.codex_thread_id,
      "the two work items did not receive independent threads",
    );
    assertGate(
      codex.isTurnActive(fastActive.codex_thread_id, fastActive.codex_turn_id)
        && codex.isTurnActive(interruptActive.codex_thread_id, interruptActive.codex_turn_id),
      "both exact turns were not simultaneously active",
    );
    mark("A", "PASS", {
      agents: [itemEvidence(fastActive), itemEvidence(interruptActive)],
      turnStarted: exactStarts,
    });
  } catch (error) {
    mark("A", "FAIL", { error: serializeError(error) });
  }

  if (gate("A").status === "PASS") {
    try {
      const fastTerminal = await bounded(codex.waitForTurnCompleted(
        fastActive.codex_thread_id,
        fastActive.codex_turn_id,
        { timeoutMs: lifecycleTimeoutMs, statuses: ["completed"] },
      ), "normal real agent completion");
      const startEntries = [fastActive, interruptActive].map((item) => exactNotification(
        "turn/started",
        item.codex_thread_id,
        item.codex_turn_id,
        1,
      ).at(-1));
      const completion = exactNotification(
        "turn/completed",
        fastActive.codex_thread_id,
        fastActive.codex_turn_id,
        1,
      ).at(-1);
      assertGate(fastTerminal.status === "completed" && completion, "normal turn did not complete");
      assertGate(
        startEntries.every((entry) => entry.sequence < completion.sequence),
        "both turns were not active before the normal completion",
      );
      assertGate(
        codex.isTurnActive(interruptActive.codex_thread_id, interruptActive.codex_turn_id),
        "the interrupt target was no longer active after the normal peer completed",
      );
      mark("B", "PASS", { fastTerminal, startEntries, completion });
    } catch (error) {
      mark("B", "FAIL", { error: serializeError(error) });
    }
  }

  if (gate("B").status === "PASS") {
    try {
      const interrupted = await bounded(
        service.interrupt(interruptWork.id),
        "exact active turn interruption",
      );
      const terminal = exactNotification(
        "turn/completed",
        interrupted.codex_thread_id,
        interrupted.codex_turn_id,
        1,
      ).at(-1);
      assertGate(
        terminal && ["interrupted", "failed"].includes(terminal.message.params.turn.status),
        "interrupt did not produce the exact correlated interrupted/failed terminal",
      );
      assertGate(interrupted.status === "failed", "interrupted LATTICE work did not fail closed");
      mark("C", "PASS", { item: itemEvidence(interrupted), terminal });
    } catch (error) {
      mark("C", "FAIL", { error: serializeError(error) });
    }
  }

  if (gate("C").status === "PASS") {
    try {
      const fastBefore = await bounded(codex.readThread(fastActive.codex_thread_id), "read fast rollout");
      const interruptedBefore = service.workItem(interruptWork.id).item;
      const retryActive = await bounded(service.resume(
        interruptWork.id,
        "Reply with exactly LATTICE_RETRY_COMPLETED. Do not use tools or modify files.",
      ), "bounded interrupted retry");
      assertGate(
        retryActive.codex_turn_id !== interruptedBefore.codex_turn_id,
        "retry reused the interrupted turn ID",
      );
      const retryTerminal = await bounded(codex.waitForTurnCompleted(
        retryActive.codex_thread_id,
        retryActive.codex_turn_id,
        { timeoutMs: lifecycleTimeoutMs, statuses: ["completed"] },
      ), "retry completion");
      await bounded(service.resume(fastWork.id), "completed fast reconciliation");
      await bounded(service.resume(interruptWork.id), "completed retry reconciliation");
      const fastAfter = await bounded(codex.readThread(fastActive.codex_thread_id), "reread fast rollout");
      const retriedAfter = await bounded(codex.readThread(retryActive.codex_thread_id), "reread retry rollout");
      const fastEvents = service.workItem(fastWork.id).events;
      const retryEvents = service.workItem(interruptWork.id).events;
      assertGate(retryTerminal.status === "completed", "retry did not complete normally");
      assertGate(fastAfter.turns.length === fastBefore.turns.length, "completed fast work was duplicated");
      assertGate(
        retryEvents.filter(({ kind }) => kind === "codex_retry_claimed").length === 1,
        "interrupted work did not use exactly one durable retry claim",
      );
      assertGate(
        fastEvents.filter(({ kind }) => kind === "turn_completed").length === 1,
        "completed fast turn was recorded more than once",
      );
      mark("D", "PASS", {
        retry: itemEvidence(service.workItem(interruptWork.id).item),
        retryTerminal,
        fastTurnCountBefore: fastBefore.turns.length,
        fastTurnCountAfter: fastAfter.turns.length,
        retryTurnCount: retriedAfter.turns.length,
      });
    } catch (error) {
      mark("D", "FAIL", { error: serializeError(error) });
    }
  }

  if (gate("D").status === "PASS") {
    let restartAttempt = null;
    try {
      const saved = {
        fast: itemEvidence(service.workItem(fastWork.id).item),
        retry: itemEvidence(service.workItem(interruptWork.id).item),
      };
      const savedItems = [saved.fast, saved.retry];
      const savedThreads = await bounded(Promise.all(savedItems.map(({ threadId }) => (
        codex.readThread(threadId)
      ))), "pre-restart rollout snapshot");
      const restartItems = savedItems.map((item, index) => ({
        ...item,
        turnIds: savedThreads[index].turns.map(({ id }) => id),
        eventCounts: eventCounts(service.workItem(item.workItemId).events),
      }));
      const phaseOneAppServerPid = codex.process?.pid ?? null;
      assertGate(Number.isInteger(phaseOneAppServerPid), "phase-one App Server PID was unavailable");
      const phaseOneClose = await closeRuntime();
      assertGate(
        phaseOneClose.appServerPid === phaseOneAppServerPid && phaseOneClose.appServerExitedAt,
        "phase-one App Server exit was not confirmed",
      );
      restartAttempt = await runFreshControlProcess({
        databasePath,
        model,
        lifecycleTimeoutMs,
        supervisorPid: process.pid,
        previousAppServerPid: phaseOneAppServerPid,
        previousAppServerExitedAt: phaseOneClose.appServerExitedAt,
        runtimeIdentity,
        items: restartItems,
      });
      const childNotifications = restartAttempt.notifications ?? [];
      const childDiagnostics = restartAttempt.diagnostics ?? [];
      const childSettlements = restartAttempt.requestSettlements ?? [];
      notifications.push(...childNotifications);
      diagnostics.push(...childDiagnostics);
      requestSettlements.push(...childSettlements);
      assertGate(restartAttempt.exitCode === 0 && restartAttempt.status === "PASS", "fresh Control process failed");
      assertGate(
        restartAttempt.spawnedPid === restartAttempt.controlPid
          && restartAttempt.controlPid !== process.pid
          && restartAttempt.parentPid === process.pid,
        "fresh Control PID boundary was not proven",
      );
      assertGate(
        restartAttempt.previousAppServerPid === phaseOneAppServerPid
          && Number.isInteger(restartAttempt.appServerPid)
          && restartAttempt.appServerPid !== phaseOneAppServerPid,
        "fresh Control did not own a different App Server process",
      );
      assertGate(
        !isProcessAlive(restartAttempt.controlPid) && !isProcessAlive(restartAttempt.appServerPid),
        "fresh Control or its App Server remained alive after reconciliation",
      );
      assertGate(
        restartAttempt.cleanup?.pendingRpc === 0
          && restartAttempt.cleanup?.pendingNotifications === 0
          && restartAttempt.cleanup?.pendingServerRequests === 0
          && !restartAttempt.cleanup?.error,
        "fresh Control left a pending request, waiter, or cleanup error",
      );
      finalSnapshot = restartAttempt.finalSnapshot;
      const {
        notifications: ignoredNotifications,
        diagnostics: ignoredDiagnostics,
        requestSettlements: ignoredSettlements,
        finalSnapshot: ignoredSnapshot,
        ...restartSummary
      } = restartAttempt;
      mark("E", "PASS", {
        beforeRestart: saved,
        phaseOne: {
          supervisorPid: process.pid,
          appServerPid: phaseOneAppServerPid,
          appServerExitedAt: phaseOneClose.appServerExitedAt,
        },
        phaseTwo: restartSummary,
        phaseTwoProcessesExited: true,
        afterRestart: restartAttempt.after,
        generation2Notifications: childNotifications.length,
      });
    } catch (error) {
      mark("E", "FAIL", {
        error: serializeError(error),
        restartAttempt,
      });
    }
  }
} catch (error) {
  topLevelError = serializeError(error);
  const firstNotRun = gates.find(({ id, status }) => id !== "F" && status === "NOT RUN");
  if (firstNotRun) mark(firstNotRun.id, "FAIL", { error: topLevelError });
} finally {
  if (store) {
    finalSnapshot = {
      workItems: store.listWorkItems(),
      events: Object.fromEntries(store.listWorkItems().map((item) => [
        item.id,
        store.listEvents(item.id),
      ])),
    };
  }
  try {
    await closeRuntime();
  } catch (error) {
    topLevelError ??= serializeError(error);
  }
}

let databaseSha256 = null;
try {
  databaseSha256 = await sha256File(databasePath);
} catch (error) {
  topLevelError ??= serializeError(error);
}
const restartInputSha256 = await optionalSha256File(restartInputPath);
const restartOutputSha256 = await optionalSha256File(restartOutputPath);
const supersededEvidenceSha256 = await optionalSha256File(supersededEvidencePath);

const implementationFiles = [
  "apps/lattice-control/src/codex-app-server.mjs",
  "apps/lattice-control/src/service.mjs",
  "apps/lattice-control/src/store.mjs",
  "apps/lattice-control/src/server.mjs",
  "scripts/run-codex-app-server-lifecycle-acceptance.mjs",
  "scripts/codex-app-server-reconcile-child.mjs",
];
const implementationSha256 = Object.fromEntries(await Promise.all(
  implementationFiles.map(async (relative) => [relative, await sha256File(path.join(repositoryRoot, relative))]),
));

const sourceDiff = commandText("git", ["diff", "--binary"]);
const evidence = {
  schemaVersion: "lattice.control.codex-app-server-lifecycle-acceptance.v3",
  runId,
  startedAt,
  completedAt: new Date().toISOString(),
  source: {
    repositoryRoot,
    branch: commandText("git", ["branch", "--show-current"]),
    head: commandText("git", ["rev-parse", "HEAD"]),
    workingTree: commandText("git", ["status", "--short"]),
    workingTreeDiffSha256: createHash("sha256").update(sourceDiff).digest("hex"),
    implementationSha256,
  },
  durableTaskBinding: {
    status: "UNAVAILABLE_FOR_GENERAL_REPAIR",
    availableSubmitIntent: "CONTROLLED_CODEX_CANARY",
    action: "NOT_REPLAYED",
    reason: "No available LATTICE task tool can bind this general connector repair.",
  },
  executionProfile: {
    model,
    reasoningEffort: "low",
    sandbox: "read-only",
    approvalPolicy: "never",
    maxRetryAttempts: 1,
    lifecycleTimeoutMs,
    appServerLaunch: "direct hashed codex.exe",
  },
  runtimeIdentity,
  artifacts: {
    artifactRoot,
    databasePath,
    databaseSha256,
    evidencePath,
    restartInputPath,
    restartInputSha256,
    restartOutputPath,
    restartOutputSha256,
    supersededEvidencePath,
    supersededEvidenceSha256,
    priorCanonicalEvidence,
  },
  gates,
  notifications,
  diagnostics,
  requestSettlements,
  finalSnapshot,
  error: topLevelError,
};

mark(
  "F",
  databaseSha256
    && restartInputSha256
    && restartOutputSha256
    && (
      priorCanonicalEvidence?.status === "NONE"
      || Boolean(priorCanonicalEvidence?.path && priorCanonicalEvidence?.sha256)
    )
    ? "PASS"
    : "FAIL",
  {
  evidencePath,
  databasePath,
  databaseSha256,
  restartInputPath,
  restartInputSha256,
  restartOutputPath,
  restartOutputSha256,
  supersededEvidencePath,
  supersededEvidenceSha256,
  priorCanonicalEvidence,
  gateStatuses: gates.filter(({ id }) => id !== "F").map(({ id, status }) => ({ id, status })),
  },
);
await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");

const allPassed = gates.every(({ status }) => status === "PASS");
process.stdout.write(`${JSON.stringify({ evidencePath, runId, gates }, null, 2)}\n`);
process.exitCode = allPassed ? 0 : 1;
