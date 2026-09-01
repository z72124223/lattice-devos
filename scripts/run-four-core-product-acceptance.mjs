import { createHash, randomUUID } from "node:crypto";
import { fork, spawn, spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { StringDecoder } from "node:string_decoder";
import { fileURLToPath } from "node:url";
import { createLatticeServer } from "../apps/lattice-control/src/server.mjs";
import { LatticeStore } from "../apps/lattice-control/src/store.mjs";
import { ControlWorkService } from "../apps/lattice-control/src/work-core-service.mjs";

const scriptPath = fileURLToPath(import.meta.url);
const repositoryRoot = path.resolve(path.dirname(scriptPath), "..");
const childMode = process.argv[2] === "--control-child";
const browserMode = process.argv.includes("--serve-browser");
const automaticMode = process.argv.includes("--automatic");
const requestTimeoutMs = 10_000;
const mutationTimeoutMs = 120_000;
const maximumMcpOutputBytes = 2_097_152;
const maximumMcpStderrBytes = 65_536;
const sourcePaths = [
  "apps/lattice-control/data-scope-contract.json",
  "apps/lattice-control/public/index.html",
  "apps/lattice-control/runtime-identity.json",
  "apps/lattice-control/test/control-plane.test.mjs",
  "package.json",
  "package-lock.json",
  "scripts/run-four-core-product-acceptance.mjs",
];

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function boundedText(value, limit = 4_096) {
  const text = String(value ?? "");
  return text.length <= limit ? text : `${text.slice(0, limit)} [truncated]`;
}

async function sourceBinding() {
  const head = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: repositoryRoot,
    encoding: "utf8",
    windowsHide: true,
  });
  const gitHead = head.status === 0 ? head.stdout.trim() : "";
  if (!/^[a-f0-9]{40}$/u.test(gitHead)) {
    throw new Error("could not bind acceptance to the current Git HEAD");
  }
  const runtimeDirectory = path.join(repositoryRoot, "apps", "lattice-control", "src");
  const runtimePaths = (await readdir(runtimeDirectory, { withFileTypes: true }))
    .filter((entry) => entry.isFile() && entry.name.endsWith(".mjs"))
    .map((entry) => path.posix.join("apps/lattice-control/src", entry.name));
  const boundPaths = [...new Set([...sourcePaths, ...runtimePaths])].sort();
  const files = [];
  for (const relativePath of boundPaths) {
    const bytes = await readFile(path.join(repositoryRoot, relativePath));
    files.push({
      path: relativePath,
      sha256: createHash("sha256").update(bytes).digest("hex"),
    });
  }
  return {
    git_head: gitHead,
    files,
    digest: createHash("sha256")
      .update(JSON.stringify({ git_head: gitHead, files }))
      .digest("hex"),
  };
}

function assertSameSourceBinding(start, end) {
  if (start.git_head !== end.git_head || start.digest !== end.digest) {
    throw new Error("acceptance source bytes changed while the run was active");
  }
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

async function runControlChild() {
  const databasePath = process.env.LATTICE_FOUR_CORE_ACCEPTANCE_DB;
  const port = Number(process.env.LATTICE_FOUR_CORE_ACCEPTANCE_PORT);
  if (!databasePath || !Number.isInteger(port) || port < 1 || port > 65_535) {
    throw new Error("four-core acceptance child configuration is invalid");
  }
  const application = createLatticeServer({ databasePath });
  const adapterKind = application.codex?.constructor?.name;
  if (adapterKind !== "CodexAppServer") {
    throw new Error(`unexpected conversation adapter ${adapterKind ?? "unknown"}`);
  }
  await new Promise((resolve, reject) => {
    application.server.once("error", reject);
    application.server.listen(port, "127.0.0.1", resolve);
  });
  process.send?.({ type: "ready", pid: process.pid, port, adapterKind });
  let closing = false;
  const closeApplication = async () => {
    if (closing) return;
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
      process.send?.({ type: "stop-failed", message: boundedText(error.message) });
      process.disconnect?.();
      process.exit(1);
    }
  };
  process.on("message", (message) => {
    if (message?.type === "shutdown") void closeApplication();
  });
  process.once("disconnect", () => void closeApplication());
  process.once("SIGINT", () => void closeApplication());
  process.once("SIGTERM", () => void closeApplication());
}

async function startControl({ databasePath, port }) {
  const child = fork(scriptPath, ["--control-child"], {
    cwd: repositoryRoot,
    env: {
      ...process.env,
      LATTICE_FOUR_CORE_ACCEPTANCE_DB: databasePath,
      LATTICE_FOUR_CORE_ACCEPTANCE_PORT: String(port),
    },
    stdio: ["ignore", "pipe", "pipe", "ipc"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr = boundedText(`${stderr}${chunk.toString("utf8")}`);
  });
  try {
    const ready = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`Control readiness timed out: ${stderr}`));
      }, 30_000);
      const fail = (error) => {
        clearTimeout(timer);
        reject(error);
      };
      child.once("error", fail);
      child.once("exit", (code, signal) => {
        fail(new Error(`Control exited before ready (${code ?? signal}): ${stderr}`));
      });
      child.on("message", (message) => {
        if (message?.type !== "ready") return;
        clearTimeout(timer);
        resolve(message);
      });
    });
    return { child, ready, stderr: () => stderr };
  } catch (error) {
    if (child.exitCode === null) child.kill("SIGKILL");
    const closed = await waitForChildClose(child, 5_000);
    if (!closed) throw new AggregateError([error], "Control startup failed and child did not close");
    throw error;
  }
}

async function waitForChildClose(child, timeoutMs) {
  const streams = [child.stdout, child.stderr].filter(Boolean);
  if (
    (child.exitCode !== null || child.signalCode !== null)
    && streams.every((stream) => stream.closed || stream.readableEnded)
  ) return true;
  return new Promise((resolve) => {
    const onClose = () => {
      clearTimeout(timer);
      resolve(true);
    };
    const timer = setTimeout(() => {
      child.off("close", onClose);
      resolve(false);
    }, timeoutMs);
    child.once("close", onClose);
  });
}

async function stopControl(control) {
  if (!control?.child) return;
  if (control.child.exitCode !== null || control.child.signalCode !== null) {
    const closed = await waitForChildClose(control.child, 5_000);
    if (!closed) throw new Error("Control exited without closing its stdio");
    if (
      control.gracefulStopAcknowledged !== true
      || control.child.exitCode !== 0
      || control.child.signalCode !== null
    ) throw new Error(`Control exited without a clean shutdown acknowledgement (${control.child.exitCode ?? control.child.signalCode})`);
    return;
  }
  let shutdownError = null;
  await new Promise((resolve) => {
    const finish = () => {
      clearTimeout(timer);
      control.child.off("message", onMessage);
      control.child.off("exit", onExit);
      resolve();
    };
    const onMessage = (message) => {
      if (message?.type === "stopped") {
        control.gracefulStopAcknowledged = true;
        finish();
      }
      else if (message?.type === "stop-failed") {
        shutdownError = new Error(message.message);
        finish();
      }
    };
    const onExit = (code, signal) => {
      if (code !== 0 || signal !== null || control.gracefulStopAcknowledged !== true) {
        shutdownError = new Error(`Control stopped without acknowledgement (${code ?? signal})`);
      }
      finish();
    };
    const timer = setTimeout(() => {
      shutdownError = new Error("Control graceful shutdown timed out");
      control.child.kill("SIGKILL");
      finish();
    }, 30_000);
    control.child.on("message", onMessage);
    control.child.once("exit", onExit);
    control.child.send({ type: "shutdown" });
  });
  let reaped = await waitForChildClose(control.child, 5_000);
  if (!reaped) {
    control.child.kill("SIGKILL");
    reaped = await waitForChildClose(control.child, 5_000);
  }
  if (!reaped) throw new Error("Control child could not be reaped");
  if (
    !shutdownError
    && (
      control.gracefulStopAcknowledged !== true
      || control.child.exitCode !== 0
      || control.child.signalCode !== null
    )
  ) {
    shutdownError = new Error(
      `Control did not finish its acknowledged shutdown cleanly (${control.child.exitCode ?? control.child.signalCode})`,
    );
  }
  if (shutdownError) throw shutdownError;
}

function mcpEnvironment(databasePath) {
  const environment = { LATTICE_CONTROL_DATABASE_PATH: databasePath };
  for (const key of ["SystemRoot", "WINDIR", "COMSPEC", "PATH", "PATHEXT", "TEMP", "TMP"]) {
    if (process.env[key]) environment[key] = process.env[key];
  }
  return environment;
}

function startMcpSession(adapterPath, databasePath) {
  const child = spawn(process.execPath, [adapterPath], {
    cwd: repositoryRoot,
    env: mcpEnvironment(databasePath),
    stdio: ["pipe", "pipe", "pipe"],
    windowsHide: true,
  });
  const pending = new Map();
  let stderrBytes = 0;
  let stderrExceeded = false;
  let stdoutBytes = 0;
  let stdoutExceeded = false;
  let stdoutBuffer = "";
  let protocolError = null;
  let nextId = 1;
  const decoder = new StringDecoder("utf8");
  const rejectPending = (error) => {
    for (const waiter of pending.values()) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
    pending.clear();
  };
  const failProtocol = (error, { kill = false } = {}) => {
    if (!protocolError) protocolError = error;
    rejectPending(error);
    if (kill && child.exitCode === null) child.kill("SIGKILL");
  };
  const handleLine = (line) => {
    if (!line) return;
    let response;
    try {
      response = JSON.parse(line);
    } catch {
      failProtocol(new Error("MCP emitted non-JSON stdout"), { kill: true });
      return;
    }
    const waiter = pending.get(response.id);
    if (!waiter) {
      failProtocol(new Error("MCP returned an unexpected response ID"), { kill: true });
      return;
    }
    clearTimeout(waiter.timer);
    pending.delete(response.id);
    waiter.resolve(response);
  };
  child.stderr.on("data", (chunk) => {
    stderrBytes += chunk.length;
    if (stderrBytes <= maximumMcpStderrBytes || stderrExceeded) return;
    stderrExceeded = true;
    failProtocol(new Error("MCP stderr exceeded the acceptance bound"), { kill: true });
  });
  child.stdout.on("data", (chunk) => {
    stdoutBytes += chunk.length;
    if (stdoutBytes > maximumMcpOutputBytes) {
      if (!stdoutExceeded) {
        stdoutExceeded = true;
        failProtocol(new Error("MCP stdout exceeded the acceptance bound"), { kill: true });
      }
      return;
    }
    stdoutBuffer += decoder.write(chunk);
    for (let newline = stdoutBuffer.indexOf("\n"); newline >= 0;
      newline = stdoutBuffer.indexOf("\n")) {
      const line = stdoutBuffer.slice(0, newline).replace(/\r$/u, "");
      stdoutBuffer = stdoutBuffer.slice(newline + 1);
      handleLine(line);
      if (protocolError) break;
    }
  });
  child.stdout.once("end", () => {
    if (stdoutExceeded) return;
    stdoutBuffer += decoder.end();
    if (stdoutBuffer.length > 0) {
      failProtocol(new Error("MCP emitted unterminated stdout"));
    }
  });
  child.once("error", (error) => failProtocol(error));
  child.once("exit", (code) => {
    if (pending.size > 0) rejectPending(new Error(`MCP exited early (${code})`));
  });
  return {
    child,
    protocolError: () => protocolError,
    stderrBytes: () => stderrBytes,
    stdoutBytes: () => stdoutBytes,
    request(method, params = {}) {
      const id = nextId;
      nextId += 1;
      const result = new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          pending.delete(id);
          reject(new Error(`MCP ${method} timed out`));
        }, requestTimeoutMs);
        pending.set(id, { resolve, reject, timer });
      });
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
      return result;
    },
    notify(method, params = {}) {
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
    },
  };
}

function rpcResult(response, label) {
  if (!response || response.error || !response.result) {
    throw new Error(`${label} did not return an MCP success response`);
  }
  return response.result;
}

async function initializeMcp(session) {
  const initialize = rpcResult(await session.request("initialize", {
    protocolVersion: "2025-11-25",
    capabilities: {},
    clientInfo: { name: "lattice-four-core-product-acceptance", version: "1" },
  }), "initialize");
  session.notify("notifications/initialized");
  const listed = rpcResult(await session.request("tools/list"), "tools/list");
  return { initialize, listed };
}

async function callTool(session, name, args) {
  const result = rpcResult(await session.request("tools/call", {
    name,
    arguments: args,
  }), name);
  if (result.isError || !result.structuredContent) {
    throw new Error(`${name} returned an MCP tool error`);
  }
  return result.structuredContent;
}

async function closeMcp(session) {
  session.child.stdin.end();
  let reaped = await waitForChildClose(session.child, 5_000);
  if (!reaped) {
    session.child.kill("SIGKILL");
    reaped = await waitForChildClose(session.child, 5_000);
    throw new Error(reaped
      ? "MCP child did not exit after EOF before the deadline"
      : "MCP child could not be reaped after the deadline");
  }
  const exitCode = session.child.exitCode;
  const stderrBytes = session.stderrBytes();
  const protocolError = session.protocolError();
  if (protocolError) throw protocolError;
  if (exitCode !== 0 || stderrBytes !== 0) {
    throw new Error(`MCP child did not close cleanly (${exitCode}, ${stderrBytes} stderr bytes)`);
  }
  return { exitCode, stderrBytes, stdoutBytes: session.stdoutBytes() };
}

function sameStringArray(actual, expected) {
  return JSON.stringify(actual) === JSON.stringify(expected);
}

async function seedWork(databasePath, projectRoot) {
  const store = new LatticeStore(databasePath);
  try {
    const work = new ControlWorkService({ store });
    const project = store.createProject({ name: "四核心驗收", rootPath: projectRoot });
    const goal = store.createWorkItem({
      projectId: project.id,
      title: "交付四核心介面",
      objective: "讓四個核心在同一個本機產品畫面可操作。",
      priority: "high",
    });
    const prerequisite = store.createWorkItem({
      projectId: project.id,
      title: "核對共用資料快照",
      objective: "證明工作圖譜與工作樹使用相同 identity。",
    });
    const child = store.createWorkItem({
      projectId: project.id,
      title: "完成使用者視覺驗收",
      objective: "檢查桌面與手機四核心互動。",
      priority: "high",
    });
    const initial = work.workSnapshot({ projectId: project.id });
    work.setWorkRelations({
      projectId: project.id,
      workItemId: child.id,
      parentId: goal.id,
      dependsOn: [prerequisite.id],
      blocker: { status: "blocked", reason: "等待使用者視覺驗收" },
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    });
    store.updateWorkItem(prerequisite.id, {
      status: "failed",
      progress: "需要重新核對 snapshot identity",
    });
    store.updateWorkItem(child.id, {
      status: "codex_done",
      progress: "等待桌面與手機畫面驗收",
    });
    return {
      project,
      goal,
      prerequisite,
      child,
      schemaVersion: store.database.prepare("PRAGMA user_version").get().user_version,
    };
  } finally {
    store.close();
  }
}

async function readWorkThroughMcp(databasePath, seed) {
  const session = startMcpSession(
    path.join(repositoryRoot, "apps/lattice-control/src/work-core-mcp.mjs"),
    databasePath,
  );
  try {
    const { initialize, listed } = await initializeMcp(session);
    const tools = listed.tools.map(({ name }) => name);
    if (!sameStringArray(tools, [
      "lattice_control_work_snapshot",
      "lattice_control_work_node",
    ])) throw new Error("work MCP tool catalog changed");
    const snapshot = await callTool(session, "lattice_control_work_snapshot", {
      project_id: seed.project.id,
      max_nodes: 32,
      max_edges: 64,
    });
    const node = await callTool(session, "lattice_control_work_node", {
      project_id: seed.project.id,
      work_item_id: seed.child.id,
      revision: snapshot.revision,
      digest: snapshot.digest,
      max_nodes: 32,
      max_edges: 64,
    });
    const prerequisite = snapshot.graph.nodes.find(({ id }) => id === seed.prerequisite.id);
    if (
      snapshot.revision !== snapshot.tree.revision
      || snapshot.revision !== snapshot.graph.revision
      || snapshot.digest !== snapshot.tree.digest
      || snapshot.digest !== snapshot.graph.digest
      || node.revision !== snapshot.revision
      || node.digest !== snapshot.digest
      || node.tree_node.parent_id !== seed.goal.id
      || !snapshot.tree.nodes.find(({ id }) => id === seed.goal.id)?.children.includes(seed.child.id)
      || !sameStringArray(node.graph_node.depends_on, [seed.prerequisite.id])
      || !prerequisite?.reverse_dependents.includes(seed.child.id)
      || node.graph_node.blocker.status !== "blocked"
      || !node.graph_node.blocker.reasons.some(({ kind }) => kind === "explicit")
      || !node.graph_node.blocker.reasons.some(({ kind }) => kind === "dependency")
    ) throw new Error("work MCP did not replay the seeded same-snapshot relations");
    const processEvidence = await closeMcp(session);
    return {
      snapshot,
      node,
      tools,
      serverName: initialize.serverInfo.name,
      processEvidence,
    };
  } catch (error) {
    if (session.child.exitCode === null && session.child.signalCode === null) {
      session.child.kill("SIGKILL");
    }
    const closed = await waitForChildClose(session.child, 5_000);
    if (!closed) throw new AggregateError([error], "work MCP failed and child did not close");
    throw error;
  }
}

async function writeDecisionsThroughMcp(databasePath, scope) {
  const session = startMcpSession(
    path.join(repositoryRoot, "apps/lattice-control/src/decision-core-mcp.mjs"),
    databasePath,
  );
  try {
    const { initialize, listed } = await initializeMcp(session);
    const tools = listed.tools.map(({ name }) => name);
    if (!sameStringArray(tools, [
      "lattice_control_decision_record",
      "lattice_control_decision_current",
      "lattice_control_decision_read",
      "lattice_control_decision_search",
    ])) throw new Error("decision MCP tool catalog changed");
    const initial = await callTool(session, "lattice_control_decision_current", {
      scope,
      limit: 10,
    });
    const source = {
      kind: "user_confirmation",
      reference: "thread:01a039e4-6ef8-7252-84fa-45c2ea8e731d/delegation:input",
    };
    const first = await callTool(session, "lattice_control_decision_record", {
      scope,
      subject: "four-core.navigation",
      content: "主要產品畫面只保留對話、工作圖譜、工作樹與決策記憶。",
      rationale: "使用者明確限定四個主要核心。",
      source,
      client_request_id: "four-core-product-decision-1",
      revision: initial.revision,
      digest: initial.digest,
    });
    const replacement = await callTool(session, "lattice_control_decision_record", {
      scope,
      subject: "four-core.navigation",
      content: "桌面與手機都完整提供對話、工作圖譜、工作樹與決策記憶。",
      rationale: "手機不可刪減任何核心，並沿用同一資料與對話脈絡。",
      source,
      supersedes_decision_id: first.decision.id,
      client_request_id: "four-core-product-decision-2",
      revision: first.revision,
      digest: first.digest,
    });
    const current = await callTool(session, "lattice_control_decision_current", {
      scope,
      limit: 10,
    });
    const read = await callTool(session, "lattice_control_decision_read", {
      decision_id: replacement.decision.id,
      max_depth: 10,
      revision: current.revision,
      digest: current.digest,
    });
    if (
      first.changed !== true
      || replacement.changed !== true
      || replacement.decision.supersedes_decision_id !== first.decision.id
      || current.decisions.length !== 1
      || current.decisions[0].id !== replacement.decision.id
      || read.lineage.length !== 2
      || read.revision !== current.revision
      || read.digest !== current.digest
    ) throw new Error("decision MCP did not retain current and superseded lineage");
    const processEvidence = await closeMcp(session);
    return {
      first,
      replacement,
      current,
      read,
      tools,
      serverName: initialize.serverInfo.name,
      processEvidence,
    };
  } catch (error) {
    if (session.child.exitCode === null && session.child.signalCode === null) {
      session.child.kill("SIGKILL");
    }
    const closed = await waitForChildClose(session.child, 5_000);
    if (!closed) throw new AggregateError([error], "decision MCP failed and child did not close");
    throw error;
  }
}

async function requestJson(
  origin,
  pathname,
  { method = "GET", body, timeoutMs = requestTimeoutMs } = {},
) {
  const response = await fetch(`${origin}${pathname}`, {
    method,
    signal: AbortSignal.timeout(timeoutMs),
    ...(body === undefined ? {} : {
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),
  });
  const payload = await response.json();
  if (!response.ok) {
    const error = new Error(payload.error || `HTTP ${response.status}`);
    error.code = payload.code;
    throw error;
  }
  return payload;
}

async function waitForConversation(origin, predicate, timeoutMs = 180_000) {
  const deadline = Date.now() + timeoutMs;
  let latest = null;
  while (Date.now() < deadline) {
    latest = await requestJson(origin, "/api/conversation");
    if (predicate(latest)) return latest;
    if (latest.status === "failed") {
      throw new Error(`real Codex turn failed: ${latest.last_error ?? latest.status_text}`);
    }
    await delay(500);
  }
  throw new Error(`real Codex turn timed out from ${latest?.status ?? "unknown"}`);
}

function latestAssistant(conversation) {
  return conversation.messages.filter(({ role }) => role === "assistant").at(-1) ?? null;
}

function assertUiSurface(surface, seed, workMcp, decisionMcp) {
  if (
    surface.context.status !== "ready"
    || surface.context.project_id !== seed.project.id
    || surface.work_snapshot.revision !== workMcp.snapshot.revision
    || surface.work_snapshot.digest !== workMcp.snapshot.digest
    || surface.work_snapshot.tree.revision !== surface.work_snapshot.graph.revision
    || surface.work_snapshot.tree.digest !== surface.work_snapshot.graph.digest
    || surface.decisions.revision !== decisionMcp.current.revision
    || surface.decisions.digest !== decisionMcp.current.digest
    || surface.decisions.decisions[0]?.id !== decisionMcp.replacement.decision.id
    || JSON.stringify(surface.decisions).includes("rationale")
  ) throw new Error("four-core HTTP surface did not read the exact MCP-backed store state");
}

async function inspectHttpSurface(origin, seed, workMcp, decisionMcp) {
  const pageResponse = await fetch(`${origin}/`, {
    signal: AbortSignal.timeout(requestTimeoutMs),
  });
  const page = await pageResponse.text();
  if (
    !pageResponse.ok
    || [...page.matchAll(/data-core-target=/gu)].length !== 4
    || [...page.matchAll(/<form\b/gu)].length !== 1
    || !page.includes('id="conversation-form"')
  ) throw new Error("served product shell does not expose exactly four cores and one form");
  const surface = await requestJson(origin, "/api/four-core");
  assertUiSurface(surface, seed, workMcp, decisionMcp);
  const workDetail = await requestJson(
    origin,
    `/api/four-core/work/${encodeURIComponent(seed.child.id)}`
      + `?revision=${encodeURIComponent(surface.work_snapshot.revision)}`
      + `&digest=${encodeURIComponent(surface.work_snapshot.digest)}`,
  );
  const decisionDetail = await requestJson(
    origin,
    `/api/four-core/decisions/${encodeURIComponent(decisionMcp.replacement.decision.id)}`
      + `?revision=${surface.decisions.revision}`
      + `&digest=${encodeURIComponent(surface.decisions.digest)}`,
  );
  if (
    workDetail.tree_node.parent_id !== seed.goal.id
    || !workDetail.graph_node.depends_on.includes(seed.prerequisite.id)
    || workDetail.graph_node.blocker.status !== "blocked"
    || decisionDetail.lineage.length !== 2
    || JSON.stringify(decisionDetail).includes("rationale")
  ) throw new Error("four-core detail endpoints did not retain relations or safe lineage");
  return { surface, workDetail, decisionDetail };
}

async function prepareScenario({ databasePath, projectRoot }) {
  await mkdir(projectRoot, { recursive: true });
  const seed = await seedWork(databasePath, projectRoot);
  const workMcp = await readWorkThroughMcp(databasePath, seed);
  const decisionMcp = await writeDecisionsThroughMcp(databasePath, seed.project.id);
  return { seed, workMcp, decisionMcp };
}

function evidenceSummary(scenario) {
  const { seed, workMcp, decisionMcp } = scenario;
  return {
    control_schema_version: seed.schemaVersion,
    project_id: seed.project.id,
    work: {
      transport: "DIRECT_STDIO_REAL_CHILD",
      server_name: workMcp.serverName,
      tools: workMcp.tools,
      revision: workMcp.snapshot.revision,
      digest: workMcp.snapshot.digest,
      node_count: workMcp.snapshot.graph.nodes.length,
      parent: true,
      dependency: true,
      reverse_dependency: true,
      blocker: true,
      process: workMcp.processEvidence,
    },
    decisions: {
      transport: "DIRECT_STDIO_REAL_CHILD",
      server_name: decisionMcp.serverName,
      tools: decisionMcp.tools,
      revision: decisionMcp.current.revision,
      digest: decisionMcp.current.digest,
      current_count: decisionMcp.current.decisions.length,
      lineage_count: decisionMcp.read.lineage.length,
      superseded: true,
      process: decisionMcp.processEvidence,
    },
  };
}

async function writeEvidence(evidencePath, evidence) {
  const serialized = `${JSON.stringify(evidence, null, 2)}\n`;
  const forbidden = [
    /-----BEGIN [A-Z ]*PRIVATE KEY-----/u,
    /\bBearer\s+[A-Za-z0-9._~-]{12,}\b/iu,
    /\b(?:password|passwd|pwd|api[_-]?key|access[_-]?token|refresh[_-]?token|otp)\s*[:=]/iu,
  ];
  if (forbidden.some((pattern) => pattern.test(serialized))) {
    throw new Error("four-core acceptance evidence contains a forbidden secret pattern");
  }
  if (Buffer.byteLength(serialized, "utf8") > 16_384) {
    throw new Error("four-core acceptance evidence exceeded 16 KiB");
  }
  await writeFile(evidencePath, serialized, "utf8");
}

async function fileSha256(filePath) {
  return createHash("sha256").update(await readFile(filePath)).digest("hex");
}

async function pngDimensions(filePath) {
  const bytes = await readFile(filePath);
  if (
    bytes.length < 24
    || bytes.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a"
    || bytes.subarray(12, 16).toString("ascii") !== "IHDR"
  ) throw new Error("browser screenshot is not a PNG with an IHDR header");
  return {
    width: bytes.readUInt32BE(16),
    height: bytes.readUInt32BE(20),
  };
}

async function verifiedBrowserRender({
  artifactRoot,
  runId,
  sourceDigest,
  expectedWorkRevision,
  expectedWorkDigest,
  expectedDecisionRevision,
  expectedDecisionDigest,
}) {
  const renderPath = path.join(artifactRoot, "browser-render.json");
  const raw = await readFile(renderPath, "utf8");
  if (Buffer.byteLength(raw, "utf8") > 16_384) {
    throw new Error("browser render evidence exceeded 16 KiB");
  }
  const render = JSON.parse(raw);
  const desktop = render.desktop;
  const mobile = render.mobile;
  const expectedWorkIdentity = `rev ${expectedWorkRevision} · ${expectedWorkDigest}`;
  const expectedDecisionIdentity = `rev ${expectedDecisionRevision} · ${expectedDecisionDigest}`;
  const valid = (
    render.schema_version === "lattice.control.four-core-browser-render.v1"
    && render.run_id === runId
    && render.source_binding_digest === sourceDigest
    && render.result === "PASS"
    && desktop?.requested_viewport?.width === 1_600
    && desktop.requested_viewport.height === 900
    && desktop.observed?.viewport?.width === 1_600
    && desktop.observed.viewport.height === 900
    && desktop.observed.document?.scrollWidth === desktop.observed.document?.clientWidth
    && desktop.observed.document.scrollHeight === desktop.observed.document.clientHeight
    && desktop.observed.horizontalOverflow === false
    && desktop.observed.tabCount === 4
    && desktop.observed.formCount === 1
    && desktop.observed.graphIdentity === expectedWorkIdentity
    && desktop.observed.treeIdentity === expectedWorkIdentity
    && desktop.observed.decisionIdentity === expectedDecisionIdentity
    && mobile?.requested_viewport?.width === 450
    && mobile.requested_viewport.height === 800
    && mobile.composer?.viewport?.width === 450
    && mobile.composer.viewport.height === 800
    && mobile.composer.horizontalOverflow === false
    && mobile.composer.noIntersection === true
    && mobile.composer.tabCount === 4
    && mobile.composer.formCount === 1
    && mobile.chat?.userMessages === 1
    && mobile.chat.assistantMessages === 1
    && mobile.chat.horizontalOverflow === false
    && mobile.chat.canSend === true
    && mobile.graph?.visiblePanel === "core-work-graph"
    && mobile.graph.horizontalOverflow === false
    && mobile.graph.graphIdentity === expectedWorkIdentity
    && mobile.graph.treeIdentity === expectedWorkIdentity
    && mobile.tree?.visiblePanel === "core-work-tree"
    && mobile.tree.horizontalOverflow === false
    && mobile.work_dialog?.withinViewport === true
    && mobile.work_dialog.horizontalOverflow === false
    && mobile.work_dialog.fields === 10
    && mobile.decisions?.visiblePanel === "core-decisions"
    && mobile.decisions.horizontalOverflow === false
    && mobile.decisions.decisionIdentity === expectedDecisionIdentity
    && mobile.decision_dialog?.withinViewport === true
    && mobile.decision_dialog.horizontalOverflow === false
    && mobile.decision_dialog.lineageCount === 2
    && sameStringArray(mobile.decision_dialog.states, ["superseded", "current"])
    && sameStringArray(render.interactions?.reached, [
      "core-conversation",
      "core-work-graph",
      "core-work-tree",
      "core-decisions",
    ])
    && render.interactions.one_user_message === true
    && render.interactions.one_assistant_message === true
    && render.interactions.work_detail_viewable === true
    && render.interactions.decision_history_viewable === true
    && render.console_error_or_warning_count === 0
  );
  if (!valid) throw new Error("browser render evidence did not satisfy the four-core gates");
  const screenshots = {
    desktop: { file: "desktop-1600x900.png", width: 1_600, height: 900 },
    mobile_chat: { file: "mobile-chat-450x800.png", width: 450, height: 800 },
    mobile_decisions: { file: "mobile-decisions-450x800.png", width: 450, height: 800 },
  };
  for (const [key, expected] of Object.entries(screenshots)) {
    const proof = render.screenshots?.[key];
    if (proof?.file !== expected.file || !/^[a-f0-9]{64}$/u.test(proof.sha256)) {
      throw new Error(`browser screenshot proof is invalid for ${key}`);
    }
    const absolutePath = path.join(artifactRoot, expected.file);
    const metadata = await stat(absolutePath);
    if (!metadata.isFile() || metadata.size < 1 || metadata.size > 5_242_880) {
      throw new Error(`browser screenshot is missing or unbounded for ${key}`);
    }
    if (await fileSha256(absolutePath) !== proof.sha256) {
      throw new Error(`browser screenshot digest changed for ${key}`);
    }
    const dimensions = await pngDimensions(absolutePath);
    if (dimensions.width !== expected.width || dimensions.height !== expected.height) {
      throw new Error(`browser screenshot dimensions are invalid for ${key}`);
    }
  }
  return {
    schema_version: render.schema_version,
    source_binding_digest: render.source_binding_digest,
    desktop_viewport: desktop.requested_viewport,
    mobile_viewport: mobile.requested_viewport,
    reached: render.interactions.reached,
    screenshots: render.screenshots,
  };
}

async function runAutomaticAcceptance() {
  const runId = `${new Date().toISOString().replace(/[:.]/gu, "-")}-${randomUUID().slice(0, 8)}`;
  const artifactRoot = path.join(repositoryRoot, ".lattice", "acceptance", runId);
  const evidencePath = path.join(artifactRoot, "four-core-product.json");
  const temporaryRoot = await mkdtemp(path.join(tmpdir(), "lattice-four-core-live-"));
  const databasePath = path.join(temporaryRoot, "control.db");
  const projectRoot = path.join(temporaryRoot, "project");
  const port = await freeLoopbackPort();
  const origin = `http://127.0.0.1:${port}`;
  await mkdir(artifactRoot, { recursive: true });
  const sourceStart = await sourceBinding();
  const evidence = {
    schema_version: "lattice.control.four-core-product-acceptance.v1",
    run_id: runId,
    started_at: new Date().toISOString(),
    result: "FAIL",
    source: "TEMP_CONTROL_SQLITE_REAL_STORE",
    authority: "CONTROL_LOCAL_PRODUCT_STATE",
    loopback_origin: origin,
    mock_used: false,
    source_binding_start: sourceStart,
    failure: null,
  };
  let control = null;
  try {
    const scenario = await prepareScenario({ databasePath, projectRoot });
    evidence.scenario = evidenceSummary(scenario);
    control = await startControl({ databasePath, port });
    evidence.first_control = {
      pid: control.ready.pid,
      adapter_kind: control.ready.adapterKind,
    };
    const beforeTurn = await inspectHttpSurface(
      origin,
      scenario.seed,
      scenario.workMcp,
      scenario.decisionMcp,
    );
    const message = {
      projectId: scenario.seed.project.id,
      clientMessageId: `four-core-${randomUUID()}`,
      text: "Reply with exactly LATTICE_FOUR_CORE_READY. Do not inspect or modify files. Do not call tools.",
    };
    await requestJson(origin, "/api/conversation/messages", {
      method: "POST",
      body: message,
      timeoutMs: mutationTimeoutMs,
    });
    const completed = await waitForConversation(
      origin,
      (conversation) => conversation.status === "codex_done"
        && latestAssistant(conversation)?.text === "LATTICE_FOUR_CORE_READY",
    );
    const duplicate = await requestJson(origin, "/api/conversation/messages", {
      method: "POST",
      body: message,
      timeoutMs: mutationTimeoutMs,
    });
    await delay(750);
    const afterDuplicate = await requestJson(origin, "/api/conversation");
    const duplicateCount = afterDuplicate.messages.filter(({ id }) => (
      id === message.clientMessageId
    )).length;
    if (duplicate.codex_turn_id !== completed.codex_turn_id || duplicateCount !== 1) {
      throw new Error("browser-facing chat duplicate protection did not retain one turn");
    }
    evidence.real_conversation = {
      adapter_kind: control.ready.adapterKind,
      conversation_id: completed.id,
      thread_id: completed.codex_thread_id,
      turn_id: completed.codex_turn_id,
      final_response: latestAssistant(completed).text,
      duplicate_user_message_count: duplicateCount,
      duplicate_turn_unchanged: duplicate.codex_turn_id === completed.codex_turn_id,
    };
    await stopControl(control);
    control = await startControl({ databasePath, port });
    evidence.second_control = {
      pid: control.ready.pid,
      adapter_kind: control.ready.adapterKind,
    };
    const afterRestart = await inspectHttpSurface(
      origin,
      scenario.seed,
      scenario.workMcp,
      scenario.decisionMcp,
    );
    const restartedConversation = afterRestart.surface.conversation;
    if (
      evidence.first_control.pid === evidence.second_control.pid
      || restartedConversation.id !== completed.id
      || restartedConversation.codex_thread_id !== completed.codex_thread_id
      || latestAssistant(restartedConversation)?.text !== "LATTICE_FOUR_CORE_READY"
      || afterRestart.surface.work_snapshot.revision
        !== beforeTurn.surface.work_snapshot.revision
      || afterRestart.surface.work_snapshot.digest
        !== beforeTurn.surface.work_snapshot.digest
      || afterRestart.surface.decisions.revision !== beforeTurn.surface.decisions.revision
      || afterRestart.surface.decisions.digest !== beforeTurn.surface.decisions.digest
      || afterRestart.decisionDetail.lineage.length !== 2
    ) throw new Error("Control restart changed a four-core durable identity");
    const replayAfterRestart = await requestJson(origin, "/api/conversation/messages", {
      method: "POST",
      body: message,
      timeoutMs: mutationTimeoutMs,
    });
    await delay(750);
    const afterRestartReplay = await requestJson(origin, "/api/conversation");
    const restartReplayCount = afterRestartReplay.messages.filter(({ id }) => (
      id === message.clientMessageId
    )).length;
    if (
      replayAfterRestart.codex_turn_id !== completed.codex_turn_id
      || afterRestartReplay.codex_turn_id !== completed.codex_turn_id
      || restartReplayCount !== 1
    ) throw new Error("Control restart replayed a duplicate chat message or turn");
    evidence.restart = {
      conversation_same: true,
      codex_thread_same: true,
      work_snapshot_same: true,
      decisions_same: true,
      decision_history_retained: true,
      duplicate_turn_unchanged: true,
      duplicate_user_message_count: restartReplayCount,
    };
    evidence.result = "PASS";
  } catch (error) {
    evidence.failure = { message: boundedText(error.message), code: error.code ?? null };
  } finally {
    try {
      await stopControl(control);
    } catch (error) {
      evidence.shutdown_failure = boundedText(error.message);
      evidence.result = "FAIL";
    }
    try {
      await rm(temporaryRoot, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
    } catch (error) {
      evidence.cleanup_failure = boundedText(error.message);
      evidence.result = "FAIL";
    }
    try {
      const sourceEnd = await sourceBinding();
      evidence.source_binding_end = sourceEnd;
      assertSameSourceBinding(sourceStart, sourceEnd);
    } catch (error) {
      evidence.source_binding_failure = boundedText(error.message);
      evidence.failure ??= { message: boundedText(error.message), code: error.code ?? null };
      evidence.result = "FAIL";
    }
    evidence.completed_at = new Date().toISOString();
    await writeEvidence(evidencePath, evidence);
  }
  process.stdout.write(`${JSON.stringify({ result: evidence.result, evidence_path: evidencePath })}\n`);
  if (evidence.result !== "PASS") throw new Error(evidence.failure?.message ?? "acceptance failed");
}

async function runBrowserServer() {
  const runId = `${new Date().toISOString().replace(/[:.]/gu, "-")}-${randomUUID().slice(0, 8)}`;
  const artifactRoot = path.join(repositoryRoot, ".lattice", "acceptance", runId);
  const evidencePath = path.join(artifactRoot, "four-core-browser-session.json");
  const databasePath = path.join(artifactRoot, "control.db");
  const projectRoot = path.join(artifactRoot, "project");
  const port = await freeLoopbackPort();
  const origin = `http://127.0.0.1:${port}`;
  await mkdir(artifactRoot, { recursive: true });
  const sourceStart = await sourceBinding();
  const scenario = await prepareScenario({ databasePath, projectRoot });
  const evidence = {
    schema_version: "lattice.control.four-core-browser-session.v1",
    run_id: runId,
    started_at: new Date().toISOString(),
    result: "RUNNING",
    loopback_origin: origin,
    mock_used: false,
    source_binding_start: sourceStart,
    scenario: evidenceSummary(scenario),
  };
  let control = null;
  try {
    control = await startControl({ databasePath, port });
    const initial = await inspectHttpSurface(
      origin,
      scenario.seed,
      scenario.workMcp,
      scenario.decisionMcp,
    );
    evidence.control = { pid: control.ready.pid, adapter_kind: control.ready.adapterKind };
    evidence.initial_http = {
      project_id: initial.surface.context.project_id,
      work_revision: initial.surface.work_snapshot.revision,
      work_digest: initial.surface.work_snapshot.digest,
      decision_revision: initial.surface.decisions.revision,
      decision_digest: initial.surface.decisions.digest,
      lineage_count: initial.decisionDetail.lineage.length,
    };
    await writeEvidence(evidencePath, evidence);
    process.stdout.write(`${JSON.stringify({
      result: "READY",
      origin,
      artifact_root: artifactRoot,
      evidence_path: evidencePath,
      run_id: runId,
      source_binding_digest: sourceStart.digest,
    })}\n`);
    await new Promise((resolve, reject) => {
      const finish = (error = null) => {
        clearTimeout(timer);
        process.off("SIGINT", onSignal);
        process.off("SIGTERM", onSignal);
        process.stdin.off("data", onData);
        process.stdin.off("end", onEnd);
        process.stdin.pause();
        if (error) reject(error);
        else resolve();
      };
      const onData = () => finish();
      const onEnd = () => finish(new Error("browser proof input ended before completion"));
      const onSignal = () => finish(new Error("browser proof was interrupted"));
      const timer = setTimeout(
        () => finish(new Error("browser proof timed out after 15 minutes")),
        900_000,
      );
      process.once("SIGINT", onSignal);
      process.once("SIGTERM", onSignal);
      process.stdin.resume();
      process.stdin.once("data", onData);
      process.stdin.once("end", onEnd);
    });
    evidence.browser_render = await verifiedBrowserRender({
      artifactRoot,
      runId,
      sourceDigest: sourceStart.digest,
      expectedWorkRevision: scenario.workMcp.snapshot.revision,
      expectedWorkDigest: scenario.workMcp.snapshot.digest,
      expectedDecisionRevision: scenario.decisionMcp.current.revision,
      expectedDecisionDigest: scenario.decisionMcp.current.digest,
    });
    const final = await requestJson(origin, "/api/four-core");
    const userMessages = final.conversation.messages.filter(({ role }) => role === "user");
    const assistantMessages = final.conversation.messages.filter(({ role }) => role === "assistant");
    const finalAssistant = latestAssistant(final.conversation);
    if (
      userMessages.length !== 1
      || assistantMessages.length !== 1
      || finalAssistant?.text !== "LATTICE_BROWSER_FOUR_CORE_READY"
      || final.work_snapshot.revision !== initial.surface.work_snapshot.revision
      || final.work_snapshot.digest !== initial.surface.work_snapshot.digest
      || final.decisions.revision !== initial.surface.decisions.revision
      || final.decisions.digest !== initial.surface.decisions.digest
    ) throw new Error("browser session did not retain the exact four-core interaction state");
    evidence.browser_observation = {
      conversation_id: final.conversation.id,
      thread_id: final.conversation.codex_thread_id,
      status: final.conversation.status,
      message_count: final.conversation.messages.length,
      final_response: finalAssistant.text,
      work_revision_unchanged: final.work_snapshot.revision
        === initial.surface.work_snapshot.revision,
      work_digest_unchanged: final.work_snapshot.digest
        === initial.surface.work_snapshot.digest,
      decisions_unchanged: final.decisions.revision === initial.surface.decisions.revision
        && final.decisions.digest === initial.surface.decisions.digest,
    };
    evidence.result = "PASS";
  } catch (error) {
    evidence.result = "FAIL";
    evidence.failure = { message: boundedText(error.message), code: error.code ?? null };
  } finally {
    try {
      await stopControl(control);
    } catch (error) {
      evidence.shutdown_failure = boundedText(error.message);
      evidence.result = "FAIL";
    }
    try {
      const sourceEnd = await sourceBinding();
      evidence.source_binding_end = sourceEnd;
      assertSameSourceBinding(sourceStart, sourceEnd);
    } catch (error) {
      evidence.source_binding_failure = boundedText(error.message);
      evidence.failure ??= { message: boundedText(error.message), code: error.code ?? null };
      evidence.result = "FAIL";
    }
    evidence.completed_at = new Date().toISOString();
    await writeEvidence(evidencePath, evidence);
  }
  if (evidence.result !== "PASS") throw new Error(evidence.failure?.message ?? "browser session failed");
}

async function readCurrentAcceptanceEvidence(fileName, schemaVersion, sourceDigest) {
  const acceptanceRoot = path.join(repositoryRoot, ".lattice", "acceptance");
  const directories = (await readdir(acceptanceRoot, { withFileTypes: true }))
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort()
    .reverse()
    .slice(0, 128);
  for (const directory of directories) {
    const evidencePath = path.join(acceptanceRoot, directory, fileName);
    let raw;
    try {
      raw = await readFile(evidencePath, "utf8");
    } catch {
      continue;
    }
    if (Buffer.byteLength(raw, "utf8") > 16_384) continue;
    const evidence = JSON.parse(raw);
    if (
      evidence.schema_version === schemaVersion
      && evidence.result === "PASS"
      && evidence.mock_used === false
      && evidence.source_binding_start?.digest === sourceDigest
      && evidence.source_binding_end?.digest === sourceDigest
    ) return {
      evidence,
      evidencePath,
      evidenceSha256: createHash("sha256").update(raw, "utf8").digest("hex"),
      artifactRoot: path.dirname(evidencePath),
    };
  }
  throw new Error(`no current PASS evidence found for ${fileName}`);
}

async function runProductGate() {
  const runId = `${new Date().toISOString().replace(/[:.]/gu, "-")}-${randomUUID().slice(0, 8)}`;
  const artifactRoot = path.join(repositoryRoot, ".lattice", "acceptance", runId);
  const evidencePath = path.join(artifactRoot, "four-core-product-gate.json");
  const sourceStart = await sourceBinding();
  const automatic = await readCurrentAcceptanceEvidence(
    "four-core-product.json",
    "lattice.control.four-core-product-acceptance.v1",
    sourceStart.digest,
  );
  const browser = await readCurrentAcceptanceEvidence(
    "four-core-browser-session.json",
    "lattice.control.four-core-browser-session.v1",
    sourceStart.digest,
  );
  if (
    automatic.evidence.real_conversation?.final_response !== "LATTICE_FOUR_CORE_READY"
    || automatic.evidence.restart?.conversation_same !== true
    || automatic.evidence.restart?.work_snapshot_same !== true
    || automatic.evidence.restart?.decisions_same !== true
    || browser.evidence.browser_observation?.final_response !== "LATTICE_BROWSER_FOUR_CORE_READY"
    || browser.evidence.browser_render?.source_binding_digest !== sourceStart.digest
  ) throw new Error("four-core product evidence is incomplete");
  await verifiedBrowserRender({
    artifactRoot: browser.artifactRoot,
    runId: browser.evidence.run_id,
    sourceDigest: sourceStart.digest,
    expectedWorkRevision: browser.evidence.initial_http.work_revision,
    expectedWorkDigest: browser.evidence.initial_http.work_digest,
    expectedDecisionRevision: browser.evidence.initial_http.decision_revision,
    expectedDecisionDigest: browser.evidence.initial_http.decision_digest,
  });
  const sourceEnd = await sourceBinding();
  assertSameSourceBinding(sourceStart, sourceEnd);
  if (
    await fileSha256(automatic.evidencePath) !== automatic.evidenceSha256
    || await fileSha256(browser.evidencePath) !== browser.evidenceSha256
  ) throw new Error("four-core evidence changed during aggregate verification");
  const result = {
    result: "PASS",
    schema_version: "lattice.control.four-core-product-gate.v1",
    run_id: runId,
    completed_at: new Date().toISOString(),
    source_binding_digest: sourceStart.digest,
    automatic_evidence: {
      path: automatic.evidencePath,
      sha256: automatic.evidenceSha256,
    },
    browser_evidence: {
      path: browser.evidencePath,
      sha256: browser.evidenceSha256,
    },
  };
  await mkdir(artifactRoot, { recursive: true });
  await writeEvidence(evidencePath, result);
  process.stdout.write(`${JSON.stringify({ ...result, evidence_path: evidencePath })}\n`);
}

if (childMode) await runControlChild();
else if (browserMode) await runBrowserServer();
else if (automaticMode) await runAutomaticAcceptance();
else await runProductGate();
