import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { createInterface } from "node:readline";
import { LatticeStore } from "../apps/lattice-control/src/store.mjs";
import { ControlWorkService } from "../apps/lattice-control/src/work-core-service.mjs";

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
    path.resolve(import.meta.dirname, "../apps/lattice-control/src/work-core-mcp.mjs"),
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
  if (!response || response.error || !response.result) {
    throw new Error(`${label} did not return an MCP success response`);
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

async function runAcceptance() {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-work-core-live-"));
  const databasePath = path.join(directory, "control.db");
  const evidencePath = evidencePathFromArguments();
  let store;
  let session;
  try {
    store = new LatticeStore(databasePath);
    const service = new ControlWorkService({ store });
    const project = store.createProject({ name: "Live acceptance", rootPath: directory });
    const goal = store.createWorkItem({
      projectId: project.id,
      title: "Work core",
      objective: "Prove one real Control work snapshot.",
      priority: "high",
    });
    const prerequisite = store.createWorkItem({
      projectId: project.id,
      title: "Migration",
      objective: "Keep the migrated rows readable.",
    });
    const child = store.createWorkItem({
      projectId: project.id,
      title: "MCP adapter",
      objective: "Read tree and graph through direct STDIO.",
    });
    const initial = service.workSnapshot({ projectId: project.id });
    service.setWorkRelations({
      projectId: project.id,
      workItemId: child.id,
      parentId: goal.id,
      dependsOn: [prerequisite.id],
      blocker: { status: "blocked", reason: "WAITING_INDEPENDENT_REVIEW" },
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    });
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
        clientInfo: { name: "lattice-control-live-acceptance", version: "1" },
      },
    }), "initialize");
    session.notify({ jsonrpc: "2.0", method: "notifications/initialized", params: {} });
    const listed = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/list",
      params: {},
    }), "tools/list");
    const snapshotResult = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: {
        name: "lattice_control_work_snapshot",
        arguments: { project_id: project.id, max_nodes: 10, max_edges: 20 },
      },
    }), "snapshot tool");
    if (snapshotResult.isError) throw new Error("snapshot tool returned a tool error");
    const snapshot = snapshotResult.structuredContent;
    const nodeResult = requireSuccess(await session.request({
      jsonrpc: "2.0",
      id: 4,
      method: "tools/call",
      params: {
        name: "lattice_control_work_node",
        arguments: {
          project_id: project.id,
          work_item_id: child.id,
          revision: snapshot.revision,
          digest: snapshot.digest,
          max_nodes: 10,
          max_edges: 20,
        },
      },
    }), "node tool");
    if (nodeResult.isError) throw new Error("node tool returned a tool error");
    const node = nodeResult.structuredContent;

    const toolNames = listed.tools.map(({ name }) => name);
    if (JSON.stringify(toolNames) !== JSON.stringify([
      "lattice_control_work_snapshot",
      "lattice_control_work_node",
    ])) throw new Error("MCP tool catalog changed");
    if (
      snapshot.tree.revision !== snapshot.graph.revision
      || snapshot.tree.digest !== snapshot.graph.digest
      || node.revision !== snapshot.revision
      || node.digest !== snapshot.digest
    ) throw new Error("MCP projection identity mismatch");
    if (
      JSON.stringify(node.graph_node.depends_on) !== JSON.stringify([prerequisite.id])
      || node.graph_node.blocker.status !== "blocked"
      || !snapshot.graph.nodes.find(({ id }) => id === prerequisite.id)
        .reverse_dependents.includes(child.id)
    ) throw new Error("MCP node relations did not replay the seeded Control store");

    session.child.stdin.end();
    const [exitCode] = await once(session.child, "exit");
    const stderrBytes = Buffer.concat(session.stderr).length;
    const metrics = session.metrics();
    if (exitCode !== 0 || stderrBytes !== 0) throw new Error("MCP child did not close cleanly");

    const evidence = {
      schema_version: "lattice.control.work-core-mcp-acceptance.v1",
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
        snapshot_called: true,
        node_called: true,
      },
      shared_projection_identity: {
        revision: snapshot.revision,
        digest: snapshot.digest,
        tree_graph_revision_equal: snapshot.tree.revision === snapshot.graph.revision,
        tree_graph_digest_equal: snapshot.tree.digest === snapshot.graph.digest,
        node_identity_equal: node.revision === snapshot.revision && node.digest === snapshot.digest,
      },
      observed_work: {
        node_count: snapshot.graph.nodes.length,
        root_count: snapshot.tree.roots.length,
        dependency_count: snapshot.graph.nodes.reduce(
          (count, entry) => count + entry.depends_on.length,
          0,
        ),
        reverse_dependency_proven: true,
        blocker_reason_kinds: node.graph_node.blocker.reasons.map(({ kind }) => kind),
      },
      bounds: {
        request_timeout_ms: requestTimeoutMs,
        requested_max_nodes: 10,
        requested_max_edges: 20,
        response_count: metrics.responseCount,
        stdout_bytes: metrics.stdoutBytes,
        maximum_stdout_frame_bytes: metrics.maximumFrameBytes,
        stderr_bytes: stderrBytes,
      },
      process: { exit_code: exitCode },
    };
    const serialized = `${JSON.stringify(evidence, null, 2)}\n`;
    if (
      serialized.includes(databasePath)
      || /(?:api[_-]?key|password|authorization|bearer\s|private[_-]?key)/iu.test(serialized)
      || Buffer.byteLength(serialized, "utf8") > 16_384
    ) throw new Error("acceptance evidence is unsafe or unbounded");
    if (evidencePath) {
      await mkdir(path.dirname(evidencePath), { recursive: true });
      await writeFile(evidencePath, serialized, { encoding: "utf8", flag: "w" });
    }
    process.stdout.write(serialized);
  } finally {
    if (session?.child.exitCode == null) session.child.kill();
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
}

try {
  await runAcceptance();
} catch {
  process.stderr.write("LATTICE_CONTROL_WORK_CORE_MCP_ACCEPTANCE_FAILED\n");
  process.exitCode = 1;
}
