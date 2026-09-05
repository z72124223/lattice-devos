import { execFileSync, fork } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { DatabaseSync } from "node:sqlite";
import { fileURLToPath, pathToFileURL } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const repositoryRoot = path.resolve(path.dirname(scriptPath), "..");
const childMode = process.argv[2] === "--control-child";

async function runControlChild() {
  const databasePath = process.env.LATTICE_CONVERSATION_ACCEPTANCE_DB;
  const port = Number(process.env.LATTICE_CONVERSATION_ACCEPTANCE_PORT);
  const serverModulePath = process.env.LATTICE_CONVERSATION_ACCEPTANCE_SERVER_MODULE
    ?? path.join(repositoryRoot, "apps", "lattice-control", "src", "server.mjs");
  if (!databasePath || !Number.isInteger(port) || port < 1 || port > 65_535) {
    throw new Error("acceptance child configuration is invalid");
  }
  const { createLatticeServer } = await import(pathToFileURL(serverModulePath).href);
  const application = createLatticeServer({ databasePath });
  const adapterKind = application.codex?.constructor?.name;
  if (adapterKind !== "CodexAppServer") {
    throw new Error(`acceptance child used unexpected adapter ${adapterKind ?? "unknown"}`);
  }
  await new Promise((resolve, reject) => {
    application.server.once("error", reject);
    application.server.listen(port, "127.0.0.1", resolve);
  });
  process.send?.({ type: "ready", pid: process.pid, port, adapterKind });
  let closing = false;
  process.on("message", async (message) => {
    if (message?.type === "identity" && !closing) {
      process.send?.({
        type: "command-result",
        requestId: message.requestId,
        result: {
          connected: Boolean(application.codex.connected),
          generation: Number.isSafeInteger(application.codex.connectionGeneration)
            ? application.codex.connectionGeneration
            : null,
          session_id: application.codex.appServerSessionId ?? null,
          process_id: Number.isSafeInteger(application.codex.process?.pid)
            ? application.codex.process.pid
            : null,
        },
      });
      return;
    }
    if (message?.type === "disconnect-codex" && !closing) {
      try {
        const receipt = await application.codex.close();
        process.send?.({
          type: "command-result",
          requestId: message.requestId,
          result: { exited: receipt.exited, process_id: receipt.processId },
        });
      } catch (error) {
        process.send?.({
          type: "command-failed",
          requestId: message.requestId,
          message: boundedErrorText(error.message),
          code: error.code ?? null,
        });
      }
      return;
    }
    if (message?.type !== "shutdown" || closing) return;
    closing = true;
    try {
      await new Promise((resolve) => application.server.close(resolve));
      await application.codex.close();
      await new Promise((resolve) => {
        if (process.send) process.send({ type: "stopped", pid: process.pid }, resolve);
        else resolve();
      });
      process.disconnect?.();
      process.exit(0);
    } catch (error) {
      await new Promise((resolve) => {
        if (process.send) process.send({ type: "stop-failed", message: error.message }, resolve);
        else resolve();
      });
      process.disconnect?.();
      process.exit(1);
    }
  });
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function freeLoopbackPort() {
  const { createServer } = await import("node:net");
  const probe = createServer();
  await new Promise((resolve, reject) => {
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", resolve);
  });
  const port = probe.address().port;
  await new Promise((resolve) => probe.close(resolve));
  return port;
}

function boundedErrorText(value, limit = 4_096) {
  const text = String(value ?? "");
  return text.length <= limit ? text : `${text.slice(0, limit)} [truncated]`;
}

async function startControl({ databasePath, port, runtime }) {
  const child = fork(scriptPath, ["--control-child"], {
    cwd: repositoryRoot,
    execPath: runtime.node_path,
    env: {
      ...process.env,
      LATTICE_CONVERSATION_ACCEPTANCE_DB: databasePath,
      LATTICE_CONVERSATION_ACCEPTANCE_PORT: String(port),
      LATTICE_CONVERSATION_ACCEPTANCE_SERVER_MODULE: runtime.server_module_path,
    },
    stdio: ["ignore", "pipe", "pipe", "ipc"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr = boundedErrorText(`${stderr}${chunk.toString("utf8")}`);
  });
  let ready;
  try {
    ready = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`Control child readiness timed out: ${stderr}`));
      }, 30_000);
      child.once("error", (error) => {
        clearTimeout(timer);
        reject(error);
      });
      child.once("exit", (code, signal) => {
        clearTimeout(timer);
        reject(new Error(`Control child exited before ready (${code ?? signal}): ${stderr}`));
      });
      child.on("message", (message) => {
        if (message?.type !== "ready") return;
        clearTimeout(timer);
        resolve(message);
      });
    });
  } catch (error) {
    if (child.exitCode === null) child.kill("SIGKILL");
    if (child.exitCode === null) {
      await new Promise((resolve) => {
        const timer = setTimeout(resolve, 5_000);
        child.once("exit", () => {
          clearTimeout(timer);
          resolve();
        });
      });
    }
    throw error;
  }
  return { child, ready, stderr: () => stderr };
}

async function controlCommand(control, type, timeoutMs = 30_000) {
  const requestId = randomUUID();
  return new Promise((resolve, reject) => {
    const finish = () => {
      clearTimeout(timer);
      control.child.off("message", onMessage);
    };
    const onMessage = (message) => {
      if (message?.requestId !== requestId) return;
      finish();
      if (message.type === "command-result") resolve(message.result);
      else {
        const error = new Error(message.message ?? `${type} failed`);
        error.code = message.code ?? null;
        reject(error);
      }
    };
    const timer = setTimeout(() => {
      control.child.off("message", onMessage);
      reject(new Error(`${type} timed out`));
    }, timeoutMs);
    control.child.on("message", onMessage);
    try {
      control.child.send({ type, requestId });
    } catch (error) {
      finish();
      reject(error);
    }
  });
}

async function waitForChildExit(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) return true;
  return new Promise((resolve) => {
    const onExit = () => {
      clearTimeout(timer);
      resolve(true);
    };
    const timer = setTimeout(() => {
      child.off("exit", onExit);
      resolve(false);
    }, timeoutMs);
    child.once("exit", onExit);
  });
}

async function stopControl(control) {
  if (!control?.child) {
    return {
      shutdown_requested: false,
      stopped_receipt: false,
      exit_code: null,
      signal: null,
    };
  }
  if (control.child.exitCode !== null || control.child.signalCode !== null) {
    throw new Error(
      `Control child exited before acceptance shutdown (${control.child.exitCode ?? control.child.signalCode})`,
    );
  }
  let shutdownError = null;
  let shutdownRequested = false;
  let stoppedReceipt = false;
  await new Promise((resolve) => {
    const finish = () => {
      clearTimeout(timer);
      control.child.off("message", onMessage);
      control.child.off("exit", onExit);
      resolve();
    };
    const onMessage = (message) => {
      if (message?.type === "stopped" && message.pid === control.child.pid) {
        stoppedReceipt = true;
        finish();
      } else if (message?.type === "stop-failed") {
        shutdownError = new Error(message.message);
        control.child.kill("SIGKILL");
        finish();
      }
    };
    const onExit = (code, signal) => {
      if (!stoppedReceipt) {
        shutdownError = new Error(
          `Control child exited before its stopped receipt (${code ?? signal})`,
        );
      } else if (code !== 0) {
        shutdownError = new Error(`Control child exited during shutdown (${code ?? signal})`);
      }
      finish();
    };
    const timer = setTimeout(() => {
      shutdownError = new Error("Control child graceful shutdown timed out");
      control.child.kill("SIGKILL");
      finish();
    }, 30_000);
    control.child.on("message", onMessage);
    control.child.once("exit", onExit);
    try {
      control.child.send({ type: "shutdown" });
      shutdownRequested = true;
    } catch (error) {
      shutdownError = error;
      control.child.kill("SIGKILL");
      finish();
    }
  });
  let reaped = await waitForChildExit(control.child, 5_000);
  if (!reaped) {
    control.child.kill("SIGKILL");
    reaped = await waitForChildExit(control.child, 5_000);
  }
  if (!reaped) throw new Error("Control child could not be reaped after shutdown");
  if (shutdownError) throw shutdownError;
  if (!shutdownRequested || !stoppedReceipt) {
    throw new Error("Control child shutdown did not produce an exact stopped receipt");
  }
  if (control.child.exitCode !== 0) {
    throw new Error(`Control child shutdown exited ${control.child.exitCode ?? control.child.signalCode}`);
  }
  return {
    shutdown_requested: true,
    stopped_receipt: true,
    exit_code: control.child.exitCode,
    signal: control.child.signalCode,
  };
}

async function request(origin, pathname, {
  method = "GET",
  body,
  timeoutMs = 30_000,
} = {}) {
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 180_000) {
    throw new TypeError("acceptance request timeout must be between 1 and 180000ms");
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  timer.unref?.();
  try {
    const response = await fetch(`${origin}${pathname}`, {
      method,
      signal: controller.signal,
      ...(body === undefined ? {} : {
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      }),
    });
    const payload = await response.json();
    if (!response.ok) {
      const error = new Error(payload.error || `HTTP ${response.status}`);
      error.code = payload.code;
      error.status = response.status;
      throw error;
    }
    return payload;
  } catch (error) {
    if (controller.signal.aborted) {
      const timeoutError = new Error(
        `${method} ${pathname} exceeded its ${timeoutMs}ms acceptance deadline`,
      );
      timeoutError.code = "ACCEPTANCE_REQUEST_TIMEOUT";
      throw timeoutError;
    }
    throw error;
  } finally {
    clearTimeout(timer);
  }
}

async function waitForConversation(origin, predicate, label, timeoutMs = 180_000) {
  const deadline = Date.now() + timeoutMs;
  let latest = null;
  while (Date.now() < deadline) {
    const remaining = deadline - Date.now();
    latest = await request(origin, "/api/conversation", {
      timeoutMs: Math.max(1, Math.min(10_000, remaining)),
    });
    if (predicate(latest)) return latest;
    if (latest.status === "failed") {
      throw new Error(`${label} failed: ${latest.last_error ?? latest.status_text}`);
    }
    await delay(Math.min(500, Math.max(1, deadline - Date.now())));
  }
  throw new Error(`${label} timed out from state ${latest?.status ?? "unknown"}`);
}

function latestAssistant(conversation) {
  return conversation.messages.filter(({ role }) => role === "assistant").at(-1) ?? null;
}

function latestAssistantForTurn(conversation, turnId) {
  return conversation.messages.filter(
    ({ role, turn_id: messageTurnId }) => role === "assistant" && messageTurnId === turnId,
  ).at(-1) ?? null;
}

function sha256File(filePath) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    const input = createReadStream(filePath);
    input.once("error", reject);
    input.on("data", (chunk) => hash.update(chunk));
    input.once("end", () => resolve(hash.digest("hex")));
  });
}

function currentHead() {
  const revision = execFileSync(
    "git",
    ["rev-parse", "HEAD"],
    { cwd: repositoryRoot, encoding: "utf8", windowsHide: true },
  ).trim();
  if (!/^[0-9a-f]{40}$/u.test(revision)) {
    throw new Error("the repository HEAD is not a full Git revision");
  }
  return revision;
}

function candidateFile(candidateDirectory, relativePath, label) {
  if (
    typeof relativePath !== "string"
    || !relativePath
    || path.isAbsolute(relativePath)
  ) throw new Error(`${label} path is invalid`);
  const target = path.resolve(candidateDirectory, relativePath.replaceAll("/", path.sep));
  const relative = path.relative(candidateDirectory, target);
  if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error(`${label} escaped the candidate directory`);
  }
  return target;
}

function requiredManifestProperty(value, name) {
  if (
    value === null
    || typeof value !== "object"
    || !Object.prototype.hasOwnProperty.call(value, name)
  ) throw new Error(`candidate manifest property is missing: ${name}`);
  return value[name];
}

async function validateCandidateDirectory(candidateDirectory, sourceCommit) {
  const manifestPath = path.join(candidateDirectory, "candidate-manifest.json");
  const manifestDetails = await lstat(manifestPath);
  if (!manifestDetails.isFile() || manifestDetails.isSymbolicLink()) {
    throw new Error("candidate manifest is not an immutable regular file");
  }
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  const controlRuntime = requiredManifestProperty(manifest, "control_runtime");
  const artifactType = requiredManifestProperty(manifest, "artifact_type");
  const runtimeIdentifier = requiredManifestProperty(manifest, "runtime_identifier");
  const selfContained = requiredManifestProperty(manifest, "self_contained");
  const launch = requiredManifestProperty(manifest, "launch");
  const controlOrigin = requiredManifestProperty(manifest, "control_origin");
  const executableSha256 = requiredManifestProperty(manifest, "executable_sha256");
  const runtimeVersion = requiredManifestProperty(controlRuntime, "version");
  const nodeVersion = requiredManifestProperty(controlRuntime, "node_version");
  const nodeSha256Claim = requiredManifestProperty(controlRuntime, "node_sha256");
  const nodeRelativePath = requiredManifestProperty(controlRuntime, "executable");
  const serverRelativePath = requiredManifestProperty(controlRuntime, "server");
  if (
    manifest.schema_version !== "lattice.control.desktop-portable-candidate.v2"
    || artifactType !== "PORTABLE_RELEASE_CANDIDATE"
    || manifest.source_commit !== sourceCommit
    || runtimeIdentifier !== "win-x64"
    || selfContained !== true
    || launch !== "LATTICE.exe"
    || controlOrigin !== "http://127.0.0.1:4317/"
    || !/^[0-9a-f]{64}$/u.test(executableSha256)
    || controlRuntime.identity_schema !== "lattice.control.runtime-identity.v1"
    || controlRuntime.product !== "LATTICE_CONTROL"
    || typeof runtimeVersion !== "string"
    || !runtimeVersion.trim()
    || controlRuntime.data_scope_schema !== "lattice.control.data-scope.v1"
    || controlRuntime.store_schema_version !== 7
    || typeof nodeVersion !== "string"
    || !/^v[0-9]+\.[0-9]+\.[0-9]+$/u.test(nodeVersion)
    || typeof nodeSha256Claim !== "string"
    || !/^[0-9a-f]{64}$/u.test(nodeSha256Claim)
    || nodeRelativePath !== "control-runtime/node.exe"
    || serverRelativePath !== "control-runtime/apps/lattice-control/src/server.mjs"
    || controlRuntime.database !== "%LOCALAPPDATA%\\LATTICE\\control\\lattice-control.db"
  ) throw new Error("candidate manifest semantic contract is incompatible");

  const manifestFiles = requiredManifestProperty(manifest, "files");
  if (!Array.isArray(manifestFiles) || manifestFiles.length < 1 || manifestFiles.length > 4_096) {
    throw new Error("candidate manifest file set is invalid");
  }
  const fileEntries = new Map();
  for (const entry of manifestFiles) {
    const entryPath = requiredManifestProperty(entry, "path");
    const length = requiredManifestProperty(entry, "length");
    const sha256 = requiredManifestProperty(entry, "sha256");
    const segments = typeof entryPath === "string" ? entryPath.split("/") : [];
    if (
      typeof entryPath !== "string"
      || !entryPath
      || path.isAbsolute(entryPath)
      || entryPath.includes("\\")
      || segments.some((segment) => !segment || segment === "." || segment === "..")
      || entryPath === "candidate-manifest.json"
      || !Number.isSafeInteger(length)
      || length < 0
      || typeof sha256 !== "string"
      || !/^[0-9a-f]{64}$/u.test(sha256)
      || fileEntries.has(entryPath)
    ) throw new Error("candidate manifest file set is invalid");
    fileEntries.set(entryPath, { path: entryPath, length, sha256 });
  }
  for (const requiredPath of [
    "LATTICE.exe",
    "LATTICE.dll",
    "PORTABLE_RELEASE_CANDIDATE.txt",
    nodeRelativePath,
    serverRelativePath,
    "control-runtime/apps/lattice-control/runtime-identity.json",
    "control-runtime/apps/lattice-control/data-scope-contract.json",
  ]) {
    if (!fileEntries.has(requiredPath)) {
      throw new Error(`candidate manifest core file is missing: ${requiredPath}`);
    }
  }

  const listFiles = async (directory) => {
    const collected = [];
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const target = path.join(directory, entry.name);
      if (entry.isSymbolicLink()) {
        throw new Error(`candidate package contains a link: ${entry.name}`);
      }
      if (entry.isDirectory()) {
        collected.push(...await listFiles(target));
      } else if (entry.isFile()) {
        const relativePath = path.relative(candidateDirectory, target).replaceAll(path.sep, "/");
        if (relativePath !== "candidate-manifest.json") collected.push(relativePath);
      } else {
        throw new Error(`candidate package contains an unsupported entry: ${entry.name}`);
      }
    }
    return collected;
  };
  const actualPaths = (await listFiles(candidateDirectory)).sort();
  const declaredPaths = [...fileEntries.keys()].sort();
  if (JSON.stringify(actualPaths) !== JSON.stringify(declaredPaths)) {
    throw new Error("candidate package actual file set does not match its manifest");
  }
  for (const entry of fileEntries.values()) {
    const target = candidateFile(candidateDirectory, entry.path, "candidate package file");
    const details = await lstat(target);
    const actualSha256 = await sha256File(target);
    if (
      !details.isFile()
      || details.isSymbolicLink()
      || details.size !== entry.length
      || actualSha256 !== entry.sha256
    ) throw new Error(`candidate package file hash mismatch: ${entry.path}`);
  }

  const desktopEntry = fileEntries.get(launch);
  const nodeEntry = fileEntries.get(nodeRelativePath);
  const serverEntry = fileEntries.get(serverRelativePath);
  if (executableSha256 !== desktopEntry.sha256) {
    throw new Error("candidate desktop executable hash does not match its manifest");
  }
  if (nodeSha256Claim !== nodeEntry.sha256) {
    throw new Error("candidate Node runtime hash does not match its manifest");
  }
  const desktopPath = candidateFile(candidateDirectory, launch, "candidate desktop executable");
  const nodePath = candidateFile(candidateDirectory, nodeRelativePath, "candidate Node runtime");
  const serverModulePath = candidateFile(
    candidateDirectory,
    serverRelativePath,
    "candidate Control server",
  );
  const controlRuntimeEntries = manifestFiles.filter(({ path: entryPath }) => (
    entryPath === nodeRelativePath
    || entryPath.startsWith("control-runtime/apps/lattice-control/")
  ));
  return Object.freeze({
    mode: "portable_candidate",
    source_commit: sourceCommit,
    candidate_directory: candidateDirectory,
    manifest_path: manifestPath,
    manifest_sha256: await sha256File(manifestPath),
    desktop_executable_path: desktopPath,
    desktop_executable_sha256: desktopEntry.sha256,
    node_path: nodePath,
    node_sha256: nodeEntry.sha256,
    server_module_path: serverModulePath,
    server_sha256: serverEntry.sha256,
    verified_package_file_count: fileEntries.size,
    verified_runtime_file_count: controlRuntimeEntries.length,
  });
}

async function resolveControlRuntime() {
  const sourceCommit = currentHead();
  const requestedCandidate = process.env.LATTICE_CONVERSATION_ACCEPTANCE_CANDIDATE_DIR?.trim();
  if (!requestedCandidate) {
    if (!process.argv.includes("--repository")) {
      throw new Error(
        "an exact candidate directory is required; use --repository only for an explicit source-tree diagnostic",
      );
    }
    const serverModulePath = path.join(
      repositoryRoot,
      "apps",
      "lattice-control",
      "src",
      "server.mjs",
    );
    return Object.freeze({
      mode: "repository",
      source_commit: sourceCommit,
      candidate_directory: null,
      manifest_path: null,
      manifest_sha256: null,
      desktop_executable_sha256: null,
      node_path: process.execPath,
      node_sha256: await sha256File(process.execPath),
      server_module_path: serverModulePath,
      server_sha256: await sha256File(serverModulePath),
    });
  }

  return validateCandidateDirectory(path.resolve(requestedCandidate), sourceCommit);
}

function readConversationEvidence(databasePath, clientMessageIds) {
  const database = new DatabaseSync(databasePath, { readOnly: true });
  try {
    const rows = database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = 'primary'
      ORDER BY id ASC
    `).all();
    const relevantKinds = new Set([
      "conversation_message_claimed",
      "conversation_turn_dispatch_intended",
      "conversation_message_accepted",
      "codex_started",
      "conversation_first_activity",
      "conversation_assistant_message",
      "turn_completed",
      "conversation_reconnected",
      "conversation_start_timeout",
      "conversation_message_failed",
    ]);
    const knownIds = new Set(clientMessageIds);
    return rows.flatMap((row) => {
      if (!relevantKinds.has(row.kind)) return [];
      const payload = JSON.parse(row.payload_json);
      if (
        payload.clientMessageId
        && !knownIds.has(payload.clientMessageId)
      ) return [];
      const selected = {};
      for (const key of [
        "clientMessageId",
        "threadId",
        "turnId",
        "status",
        "confirmedBy",
        "type",
        "queueDurationMs",
        "terminalStatus",
        "timeoutMs",
        "queueAfterThreadId",
        "queueAfterTurnId",
      ]) {
        if (payload[key] !== undefined) selected[key] = payload[key];
      }
      return [{
        event_id: Number(row.id),
        kind: row.kind,
        created_at: row.created_at,
        ...selected,
      }];
    });
  } finally {
    database.close();
  }
}

function exactEventFor(events, kind, { clientMessageId = null, turnId = null } = {}) {
  const matches = events.filter((event) => event.kind === kind
    && (clientMessageId === null || event.clientMessageId === clientMessageId)
    && (turnId === null || event.turnId === turnId));
  if (matches.length !== 1) {
    throw new Error(
      `SQLite lifecycle evidence expected exactly one ${kind} event, observed ${matches.length}`,
    );
  }
  return matches[0];
}

function validateLifecycleEvidence(events, clientMessageIds, turnIds) {
  if (
    clientMessageIds.length !== 3
    || turnIds.length !== 3
    || new Set(clientMessageIds).size !== 3
    || new Set(turnIds).size !== 3
  ) throw new Error("acceptance lifecycle identities are not exactly three unique messages and turns");
  const claims = clientMessageIds.map((clientMessageId) => exactEventFor(
    events,
    "conversation_message_claimed",
    { clientMessageId },
  ));
  const dispatches = clientMessageIds.map((clientMessageId) => exactEventFor(
    events,
    "conversation_turn_dispatch_intended",
    { clientMessageId },
  ));
  const acceptances = clientMessageIds.map((clientMessageId) => exactEventFor(
    events,
    "conversation_message_accepted",
    { clientMessageId },
  ));
  const terminals = turnIds.map(
    (turnId) => exactEventFor(events, "turn_completed", { turnId }),
  );
  const firstActivities = turnIds.map(
    (turnId) => exactEventFor(events, "conversation_first_activity", { turnId }),
  );
  const knownTurnIds = new Set(turnIds);
  const turnIdentityKinds = new Set([
    "conversation_message_accepted",
    "codex_started",
    "conversation_first_activity",
    "conversation_assistant_message",
    "turn_completed",
    "conversation_start_timeout",
    "conversation_message_failed",
  ]);
  const unknownTurnEvent = events.find(
    ({ kind, turnId }) => turnIdentityKinds.has(kind) && turnId && !knownTurnIds.has(turnId),
  );
  if (unknownTurnEvent) {
    throw new Error(
      `SQLite lifecycle evidence contains an unexpected turn identity ${unknownTurnEvent.turnId}`,
    );
  }
  if (
    terminals.some(({ status }) => status !== "completed")
    || new Set(acceptances.map(({ threadId }) => threadId)).size !== 1
    || acceptances.some(({ turnId }, index) => turnId !== turnIds[index])
    || firstActivities.some(({ turnId }, index) => turnId !== turnIds[index])
    || !(claims[0].event_id < dispatches[0].event_id
      && dispatches[0].event_id < acceptances[0].event_id
      && acceptances[0].event_id < claims[1].event_id
      && claims[1].event_id < terminals[0].event_id
      && terminals[0].event_id < dispatches[1].event_id
      && dispatches[1].event_id < acceptances[1].event_id
      && acceptances[1].event_id < terminals[1].event_id
      && terminals[1].event_id < claims[2].event_id
      && claims[2].event_id < dispatches[2].event_id
      && dispatches[2].event_id < acceptances[2].event_id
      && acceptances[2].event_id < terminals[2].event_id)
    || events.some(({ kind }) => kind === "conversation_start_timeout")
  ) throw new Error("SQLite lifecycle evidence did not prove exact queue and terminal ordering");
  return { claims, dispatches, acceptances, terminals, firstActivities };
}

function messageFor(conversation, clientMessageId) {
  return conversation.messages.find(({ id }) => id === clientMessageId) ?? null;
}

async function runAcceptance() {
  const runId = `${new Date().toISOString().replace(/[:.]/gu, "-")}-${randomUUID().slice(0, 8)}`;
  const artifactRoot = path.join(repositoryRoot, ".lattice", "acceptance", runId);
  const evidencePath = path.join(artifactRoot, "primary-conversation.json");
  const temporaryRoot = await mkdtemp(path.join(tmpdir(), "lattice-primary-live-"));
  const projectRoot = path.join(temporaryRoot, "project");
  const databasePath = path.join(temporaryRoot, "control.db");
  await mkdir(projectRoot, { recursive: true });
  await mkdir(artifactRoot, { recursive: true });
  const port = await freeLoopbackPort();
  const origin = `http://127.0.0.1:${port}`;
  const evidence = {
    schema_version: "lattice.control.primary-conversation-acceptance.v6",
    run_id: runId,
    started_at: new Date().toISOString(),
    status: "FAIL",
    transport: "official Codex App Server through LATTICE Control",
    mock_used: false,
    loopback_origin: origin,
    runtime: null,
    control: null,
    queued_followup: null,
    app_server_reconnect: null,
    post_reconnect_turn: null,
    queue_deadline: null,
    event_sequence: null,
    teardown: null,
    failure: null,
  };
  let control = null;
  let runtime = null;
  const appServerPids = new Set();
  const clientMessageIds = [];
  try {
    runtime = await resolveControlRuntime();
    evidence.runtime = runtime;
    control = await startControl({ databasePath, port, runtime });
    evidence.control = {
      pid: control.ready.pid,
      adapter_kind: control.ready.adapterKind,
    };
    const project = await request(origin, "/api/projects", {
      method: "POST",
      body: { name: "Primary conversation lifecycle acceptance", rootPath: projectRoot },
    });

    const firstMessage = {
      projectId: project.id,
      clientMessageId: `live-active-${randomUUID()}`,
      text: [
        "This is a bounded live queue acceptance turn.",
        "Use the shell exactly once to run this read-only command:",
        "powershell -NoProfile -Command \"Start-Sleep -Seconds 12\"",
        "After it finishes, reply with exactly LATTICE_PRIMARY_ACTIVE_DONE.",
        "Do not inspect, create, modify, or delete files.",
      ].join("\n"),
    };
    const queuedMessage = {
      projectId: project.id,
      clientMessageId: `live-queued-${randomUUID()}`,
      text: "Reply with exactly LATTICE_PRIMARY_QUEUED_DONE. Do not inspect or modify files. Do not call tools.",
    };
    const reconnectMessage = {
      projectId: project.id,
      clientMessageId: `live-reconnected-${randomUUID()}`,
      text: "Reply with exactly LATTICE_PRIMARY_RECONNECTED_DONE. Do not inspect or modify files. Do not call tools.",
    };
    clientMessageIds.push(
      firstMessage.clientMessageId,
      queuedMessage.clientMessageId,
      reconnectMessage.clientMessageId,
    );

    const firstRequestedAtMs = Date.now();
    const firstAccepted = await request(origin, "/api/conversation/messages", {
      method: "POST",
      body: firstMessage,
    });
    if (!firstAccepted.codex_thread_id || !firstAccepted.codex_turn_id) {
      throw new Error("the first live turn did not expose exact thread/turn identity");
    }
    const firstRunning = await waitForConversation(
      origin,
      (conversation) => conversation.id === firstAccepted.id
        && conversation.codex_thread_id === firstAccepted.codex_thread_id
        && conversation.codex_turn_id === firstAccepted.codex_turn_id
        && conversation.status === "running"
        && conversation.can_send === true
        && !conversation.messages.some(
          ({ role, turn_id: turnId }) => role === "assistant" && turnId === firstAccepted.codex_turn_id,
        ),
      "first live turn running before follow-up",
      30_000,
    );
    const firstRunningObservedAtMs = Date.now();
    const queuedRequestedAtMs = Date.now();
    const queued = await request(origin, "/api/conversation/messages", {
      method: "POST",
      body: queuedMessage,
    });
    const queuedAcknowledgedAtMs = Date.now();
    const queuedUser = messageFor(queued, queuedMessage.clientMessageId);
    if (
      queued.acknowledged_client_message_id !== queuedMessage.clientMessageId
      || queuedUser?.delivery_status !== "saved"
      || queuedUser.queue_status !== "queued"
      || queued.codex_thread_id !== firstRunning.codex_thread_id
      || queued.codex_turn_id !== firstRunning.codex_turn_id
    ) throw new Error("the active follow-up was not explicitly queued behind the exact live turn");
    const queuedReplay = await request(origin, "/api/conversation/messages", {
      method: "POST",
      body: queuedMessage,
    });
    if (
      queuedReplay.messages.filter(({ id }) => id === queuedMessage.clientMessageId).length !== 1
      || queuedReplay.codex_turn_id !== firstRunning.codex_turn_id
    ) throw new Error("queued client-message replay was not idempotent");

    const queuedTerminal = await waitForConversation(
      origin,
      (conversation) => {
        const queuedAccepted = messageFor(conversation, queuedMessage.clientMessageId);
        return conversation.id === firstRunning.id
          && conversation.codex_thread_id === firstRunning.codex_thread_id
          && conversation.status === "codex_done"
          && Boolean(queuedAccepted?.turn_id)
          && conversation.codex_turn_id === queuedAccepted.turn_id
          && conversation.messages.some(
            ({ role, turn_id: turnId, text }) => role === "assistant"
              && turnId === queuedAccepted.turn_id
              && text === "LATTICE_PRIMARY_QUEUED_DONE",
          );
      },
      "automatically dispatched queued follow-up",
    );
    const queuedTerminalObservedAtMs = Date.now();
    const firstUser = messageFor(queuedTerminal, firstMessage.clientMessageId);
    const secondUser = messageFor(queuedTerminal, queuedMessage.clientMessageId);
    const firstReply = latestAssistantForTurn(queuedTerminal, firstUser?.turn_id);
    const secondReply = latestAssistantForTurn(queuedTerminal, secondUser?.turn_id);
    if (
      firstUser?.turn_id !== firstRunning.codex_turn_id
      || !secondUser?.turn_id
      || secondUser.turn_id === firstUser.turn_id
      || firstReply?.text !== "LATTICE_PRIMARY_ACTIVE_DONE"
      || secondReply?.text !== "LATTICE_PRIMARY_QUEUED_DONE"
      || secondUser.delivery_status !== "accepted"
    ) throw new Error("the queued follow-up did not execute exactly once after the first terminal");
    evidence.queued_followup = {
      conversation_id: queuedTerminal.id,
      thread_id: queuedTerminal.codex_thread_id,
      first: {
        client_message_id: firstMessage.clientMessageId,
        turn_id: firstUser.turn_id,
        requested_at: new Date(firstRequestedAtMs).toISOString(),
        running_observed_at: new Date(firstRunningObservedAtMs).toISOString(),
        final_response: firstReply.text,
      },
      second: {
        client_message_id: queuedMessage.clientMessageId,
        turn_id: secondUser.turn_id,
        requested_at: new Date(queuedRequestedAtMs).toISOString(),
        acknowledged_at: new Date(queuedAcknowledgedAtMs).toISOString(),
        acknowledgment_latency_ms: queuedAcknowledgedAtMs - queuedRequestedAtMs,
        requested_after_running_observed_ms: queuedRequestedAtMs - firstRunningObservedAtMs,
        initial_delivery_status: queuedUser.delivery_status,
        initial_queue_status: queuedUser.queue_status,
        final_delivery_status: secondUser.delivery_status,
        final_response: secondReply.text,
        terminal_observed_at: new Date(queuedTerminalObservedAtMs).toISOString(),
      },
      automatic_order_verified: true,
      duplicate_replay_message_count: queuedReplay.messages.filter(
        ({ id }) => id === queuedMessage.clientMessageId,
      ).length,
    };

    const beforeDisconnect = await controlCommand(control, "identity");
    if (!beforeDisconnect.connected || !beforeDisconnect.process_id || !beforeDisconnect.session_id) {
      throw new Error("the official App Server identity was unavailable before disconnect");
    }
    appServerPids.add(beforeDisconnect.process_id);
    const disconnectedAtMs = Date.now();
    const disconnectReceipt = await controlCommand(control, "disconnect-codex");
    const disconnectedIdentity = await controlCommand(control, "identity");
    if (
      disconnectReceipt.exited !== true
      || disconnectReceipt.process_id !== beforeDisconnect.process_id
      || disconnectedIdentity.connected
      || disconnectedIdentity.process_id !== null
      || disconnectedIdentity.session_id !== null
    ) throw new Error("the owned App Server generation did not close cleanly");

    const reconnectRequestedAtMs = Date.now();
    const reconnectedConversation = await request(origin, "/api/conversation/reconnect", {
      method: "POST",
      body: {},
    });
    const reconnectCompletedAtMs = Date.now();
    const afterReconnect = await controlCommand(control, "identity");
    if (afterReconnect.process_id) appServerPids.add(afterReconnect.process_id);
    if (
      !afterReconnect.connected
      || !afterReconnect.process_id
      || !afterReconnect.session_id
      || afterReconnect.generation <= beforeDisconnect.generation
      || afterReconnect.session_id === beforeDisconnect.session_id
      || reconnectedConversation.id !== queuedTerminal.id
      || reconnectedConversation.codex_thread_id !== queuedTerminal.codex_thread_id
      || reconnectedConversation.codex_turn_id !== secondUser.turn_id
      || reconnectedConversation.status !== "codex_done"
    ) throw new Error("App Server reconnect did not retain the exact completed conversation");
    evidence.app_server_reconnect = {
      disconnected_at: new Date(disconnectedAtMs).toISOString(),
      reconnect_requested_at: new Date(reconnectRequestedAtMs).toISOString(),
      reconnect_completed_at: new Date(reconnectCompletedAtMs).toISOString(),
      reconnect_latency_ms: reconnectCompletedAtMs - reconnectRequestedAtMs,
      before: beforeDisconnect,
      disconnect_receipt: disconnectReceipt,
      disconnected: disconnectedIdentity,
      after: afterReconnect,
      retained: {
        conversation_id: reconnectedConversation.id,
        thread_id: reconnectedConversation.codex_thread_id,
        turn_id: reconnectedConversation.codex_turn_id,
        status: reconnectedConversation.status,
      },
    };

    const postReconnectRequestedAtMs = Date.now();
    const postReconnectAccepted = await request(origin, "/api/conversation/messages", {
      method: "POST",
      body: reconnectMessage,
    });
    const postReconnectAcceptedAtMs = Date.now();
    if (
      postReconnectAccepted.id !== queuedTerminal.id
      || postReconnectAccepted.codex_thread_id !== queuedTerminal.codex_thread_id
      || !postReconnectAccepted.codex_turn_id
      || postReconnectAccepted.codex_turn_id === secondUser.turn_id
    ) throw new Error("the post-reconnect message did not receive a new exact turn identity");
    const postReconnectTerminal = await waitForConversation(
      origin,
      (conversation) => conversation.id === queuedTerminal.id
        && conversation.codex_thread_id === queuedTerminal.codex_thread_id
        && conversation.codex_turn_id === postReconnectAccepted.codex_turn_id
        && conversation.status === "codex_done"
        && latestAssistant(conversation)?.text === "LATTICE_PRIMARY_RECONNECTED_DONE",
      "post-reconnect real Codex turn",
    );
    const postReconnectTerminalAtMs = Date.now();
    const thirdUser = messageFor(postReconnectTerminal, reconnectMessage.clientMessageId);
    if (
      thirdUser?.turn_id !== postReconnectAccepted.codex_turn_id
      || thirdUser.delivery_status !== "accepted"
    ) throw new Error("the post-reconnect message was not durably accepted by its exact turn");
    evidence.post_reconnect_turn = {
      conversation_id: postReconnectTerminal.id,
      client_message_id: reconnectMessage.clientMessageId,
      thread_id: postReconnectTerminal.codex_thread_id,
      turn_id: postReconnectTerminal.codex_turn_id,
      requested_at: new Date(postReconnectRequestedAtMs).toISOString(),
      accepted_at: new Date(postReconnectAcceptedAtMs).toISOString(),
      terminal_observed_at: new Date(postReconnectTerminalAtMs).toISOString(),
      accepted_latency_ms: postReconnectAcceptedAtMs - postReconnectRequestedAtMs,
      terminal_latency_ms: postReconnectTerminalAtMs - postReconnectRequestedAtMs,
      terminal_deadline_ms: 180_000,
      terminal_status: postReconnectTerminal.status,
      final_response: latestAssistant(postReconnectTerminal).text,
    };

    const events = readConversationEvidence(databasePath, clientMessageIds);
    const turnIds = [firstUser.turn_id, secondUser.turn_id, thirdUser.turn_id];
    const { firstActivities } = validateLifecycleEvidence(
      events,
      clientMessageIds,
      turnIds,
    );
    const queueDurations = firstActivities.map(({ queueDurationMs }) => queueDurationMs);
    if (queueDurations.some((duration) => !Number.isInteger(duration) || duration >= 30_000)) {
      throw new Error("a real Codex turn missed the preserved 30 second first-activity deadline");
    }
    evidence.queue_deadline = {
      first_activity_deadline_ms: 30_000,
      first_activity_queue_duration_ms: {
        first: queueDurations[0],
        queued: queueDurations[1],
        post_reconnect: queueDurations[2],
      },
      timeout_event_count: events.filter(
        ({ kind }) => kind === "conversation_start_timeout",
      ).length,
      live_post_reconnect_first_activity_within_deadline: true,
      live_post_reconnect_terminal_within_acceptance_deadline: true,
    };
    evidence.event_sequence = events;
    evidence.status = "PASS";
  } catch (error) {
    evidence.failure = {
      message: boundedErrorText(error.message),
      code: error.code ?? null,
      http_status: error.status ?? null,
    };
  } finally {
    if (!evidence.event_sequence) {
      try {
        evidence.event_sequence = readConversationEvidence(databasePath, clientMessageIds);
      } catch {}
    }
    let shutdownFailure = null;
    let shutdownReceipt = {
      shutdown_requested: false,
      stopped_receipt: false,
      exit_code: control?.child?.exitCode ?? null,
      signal: control?.child?.signalCode ?? null,
    };
    try {
      shutdownReceipt = await stopControl(control);
    } catch (error) {
      shutdownFailure = boundedErrorText(error.message);
      evidence.status = "FAIL";
    }
    const appServerResidue = [];
    for (const processId of appServerPids) {
      try {
        process.kill(processId, 0);
        appServerResidue.push(processId);
      } catch {}
    }
    let listenerReleased = false;
    try {
      const probe = await import("node:net").then(({ createServer }) => createServer());
      await new Promise((resolve, reject) => {
        probe.once("error", reject);
        probe.listen(port, "127.0.0.1", resolve);
      });
      await new Promise((resolve) => probe.close(resolve));
      listenerReleased = true;
    } catch {}
    let temporaryRootRemoved = false;
    try {
      await rm(temporaryRoot, {
        recursive: true,
        force: true,
        maxRetries: 20,
        retryDelay: 250,
      });
      temporaryRootRemoved = true;
    } catch (error) {
      evidence.cleanup_failure = boundedErrorText(error.message);
      evidence.status = "FAIL";
    }
    evidence.teardown = {
      control_pid: control?.ready.pid ?? null,
      control_reaped: !control?.child || control.child.exitCode !== null || control.child.signalCode !== null,
      app_server_pids: [...appServerPids],
      app_server_residue: appServerResidue,
      loopback_listener_released: listenerReleased,
      temporary_root_removed: temporaryRootRemoved,
      shutdown_failure: shutdownFailure,
      shutdown_requested: shutdownReceipt.shutdown_requested,
      stopped_ipc_receipt: shutdownReceipt.stopped_receipt,
      exit_code: shutdownReceipt.exit_code,
      exit_signal: shutdownReceipt.signal,
    };
    if (
      appServerResidue.length > 0
      || !listenerReleased
      || !temporaryRootRemoved
      || shutdownFailure
      || evidence.teardown.control_reaped !== true
      || evidence.teardown.shutdown_requested !== true
      || evidence.teardown.stopped_ipc_receipt !== true
      || evidence.teardown.exit_code !== 0
      || evidence.teardown.exit_signal !== null
    ) evidence.status = "FAIL";
    evidence.completed_at = new Date().toISOString();
    await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");
  }
  process.stdout.write(`${JSON.stringify({ status: evidence.status, evidence_path: evidencePath })}\n`);
  if (evidence.status !== "PASS") {
    const error = new Error(
      evidence.failure?.message
      || evidence.teardown?.shutdown_failure
      || "acceptance failed",
    );
    error.code = evidence.failure?.code;
    throw error;
  }
  return evidencePath;
}

const launchedAsMain = process.argv[1]
  && path.resolve(process.argv[1]) === scriptPath;
if (launchedAsMain) {
  if (childMode) await runControlChild();
  else await runAcceptance();
}

export {
  exactEventFor,
  request,
  stopControl,
  validateCandidateDirectory,
  validateLifecycleEvidence,
  waitForConversation,
};
