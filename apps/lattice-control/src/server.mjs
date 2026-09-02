import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";
import { CodexAppServer } from "./codex-app-server.mjs";
import {
  controlDataScopeDescriptor,
  defaultControlDatabasePath,
} from "./database-path.mjs";
import { ControlMcpHealthMonitor } from "./mcp-health.mjs";
import { createRuntimeSurface } from "./runtime-surface.mjs";
import { LatticeControlService } from "./service.mjs";
import { LatticeStore } from "./store.mjs";

const sourceDirectory = path.dirname(fileURLToPath(import.meta.url));
const publicDirectory = path.resolve(sourceDirectory, "..", "public");
const maximumDesktopShutdownFrameBytes = 4_096;
const desktopShutdownSchemaVersion = "lattice.control.desktop-shutdown.v1";

class HttpRequestError extends Error {
  constructor(status, code, message) {
    super(message);
    this.name = "HttpRequestError";
    this.status = status;
    this.code = code;
  }
}

function sendJson(response, status, value) {
  const body = JSON.stringify(value);
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store",
  });
  response.end(body);
}

function publicErrorMessage(value) {
  const text = String(value ?? "Control request failed");
  const suffix = " [truncated]";
  return text.length <= 2_048
    ? text
    : `${text.slice(0, 2_048 - suffix.length)}${suffix}`;
}

function publicErrorCode(error) {
  if (error instanceof TypeError) return "INVALID_REQUEST";
  return typeof error?.code === "string" && /^[A-Z][A-Z0-9_]{0,127}$/u.test(error.code)
    ? error.code
    : "CONTROL_REQUEST_FAILED";
}

async function settleWithin(promise, deadline) {
  const remaining = Math.max(0, deadline - Date.now());
  if (remaining === 0) return { settled: false };
  let timer;
  try {
    return await Promise.race([
      Promise.resolve(promise).then(
        (value) => ({ settled: true, value }),
        (error) => ({ settled: true, error }),
      ),
      new Promise((resolve) => {
        timer = setTimeout(() => resolve({ settled: false }), remaining);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

function shutdownDrainTimeoutError() {
  const error = new Error("Control HTTP effects did not drain before shutdown deadline");
  error.code = "CONTROL_SHUTDOWN_DRAIN_TIMEOUT";
  return error;
}

async function readJson(request) {
  const chunks = [];
  let length = 0;
  for await (const chunk of request) {
    length += chunk.length;
    if (length > 1_048_576) throw new Error("request body is too large");
    chunks.push(chunk);
  }
  return chunks.length === 0 ? {} : JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

function validateLoopbackRequest(request) {
  const host = request.headers.host;
  let hostUrl;
  try {
    if (typeof host !== "string" || /[\u0000-\u0020\u007f-\u009f]/u.test(host)) {
      throw new Error("invalid Host");
    }
    hostUrl = new URL(`http://${host}`);
    if (
      hostUrl.hostname !== "127.0.0.1"
      || hostUrl.username
      || hostUrl.password
      || hostUrl.pathname !== "/"
    ) {
      throw new Error("non-loopback Host");
    }
  } catch {
    throw new HttpRequestError(403, "CONTROL_LOOPBACK_REQUIRED", "Control requires its loopback Host");
  }
  const origin = request.headers.origin;
  if (origin != null) {
    let parsedOrigin;
    try {
      parsedOrigin = new URL(origin);
    } catch {
      throw new HttpRequestError(403, "CONTROL_ORIGIN_REJECTED", "Control rejected the request Origin");
    }
    if (parsedOrigin.origin !== hostUrl.origin) {
      throw new HttpRequestError(403, "CONTROL_ORIGIN_REJECTED", "Control rejected the request Origin");
    }
  }
  if (request.method === "POST") {
    const contentType = request.headers["content-type"]?.split(";", 1)[0].trim().toLowerCase();
    if (contentType !== "application/json") {
      throw new HttpRequestError(
        415,
        "CONTROL_JSON_REQUIRED",
        "Control mutations require application/json",
      );
    }
  }
}

function routeId(pathname, action) {
  const match = pathname.match(new RegExp(`^/api/work-items/([^/]+)/${action}$`, "u"));
  return match ? decodeURIComponent(match[1]) : null;
}

function installationReceiptRouteId(pathname) {
  const match = pathname.match(/^\/api\/installation-receipts\/([^/]+)$/u);
  return match ? decodeURIComponent(match[1]) : null;
}

function projectRouteId(pathname, action = null) {
  const suffix = action ? `/${action}` : "";
  const match = pathname.match(new RegExp(`^/api/projects/([^/]+)${suffix}$`, "u"));
  return match ? decodeURIComponent(match[1]) : null;
}

function fourCoreRouteId(pathname, resource) {
  const match = pathname.match(new RegExp(`^/api/four-core/${resource}/([^/]+)$`, "u"));
  return match ? decodeURIComponent(match[1]) : null;
}

export function createLatticeServer({
  databasePath,
  codex = new CodexAppServer(),
  projectInspector,
  mcpHealth,
}) {
  const store = new LatticeStore(databasePath);
  const service = new LatticeControlService({
    store,
    codex,
    ...(projectInspector ? { projectInspector } : {}),
  });
  const resolvedMcpHealth = mcpHealth ?? (typeof databasePath === "string"
    ? new ControlMcpHealthMonitor({ databasePath })
    : {
        current: async () => ({
          work_mcp: "UNREACHABLE",
          decision_mcp: "UNREACHABLE",
        }),
      });
  let acceptingEffects = true;
  let ownedShutdown = false;
  let ownedShutdownPromise = null;
  let serverClosePromise = null;
  const inFlightRequests = new Set();
  const recoveryMutationTarget = (pathname) => {
    if (
      pathname === "/api/conversation/reconnect"
      || pathname === "/api/conversation/interrupt"
    ) return "primary";
    const match = pathname.match(/^\/api\/work-items\/([^/]+)\/(?:interrupt|reconcile)$/u);
    if (!match) return null;
    try {
      return decodeURIComponent(match[1]);
    } catch {
      return null;
    }
  };
  const assertMutationAdmission = (request, url) => {
    if (request.method !== "POST") return;
    if (!acceptingEffects) {
      throw new HttpRequestError(
        503,
        "CONTROL_SHUTTING_DOWN",
        "Control is shutting down and is not accepting new effects",
      );
    }
    if (service.reconciliationRequired()) {
      const recoveryTarget = recoveryMutationTarget(url.pathname);
      if (recoveryTarget === null || !service.reconciliationRequired(recoveryTarget)) {
        throw new HttpRequestError(
          409,
          "CONTROL_RECONCILIATION_REQUIRED",
          "Control must reconcile the inherited active turn before accepting new effects",
        );
      }
    }
  };
  const readMutationJson = async (request, url) => {
    const body = await readJson(request);
    assertMutationAdmission(request, url);
    return body;
  };
  const server = createServer(async (request, response) => {
    let finishTrackedRequest = null;
    let trackedRequest = null;
    try {
      validateLoopbackRequest(request);
      if (ownedShutdown) {
        throw new HttpRequestError(
          503,
          "CONTROL_SHUTTING_DOWN",
          "Control is shutting down",
        );
      }
      trackedRequest = new Promise((resolve) => { finishTrackedRequest = resolve; });
      inFlightRequests.add(trackedRequest);
      const url = new URL(request.url, "http://127.0.0.1");
      assertMutationAdmission(request, url);
      if (request.method === "GET" && url.pathname === "/") {
        const body = await readFile(path.join(publicDirectory, "index.html"));
        response.writeHead(200, {
          "content-type": "text/html; charset=utf-8",
          "content-length": body.length,
          "cache-control": "no-store",
        });
        response.end(body);
        return;
      }
      if (request.method === "GET" && url.pathname === "/api/state") {
        sendJson(response, 200, service.state());
        return;
      }
      if (request.method === "GET" && url.pathname === "/api/runtime") {
        sendJson(response, 200, createRuntimeSurface(service, {
          databasePath,
          mcpHealth: await resolvedMcpHealth.current(),
        }));
        return;
      }
      if (request.method === "GET" && url.pathname === "/api/conversation") {
        sendJson(response, 200, service.primaryConversation());
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/conversation") {
        const body = await readMutationJson(request, url);
        sendJson(response, 200, await service.startPrimaryConversation({
          projectId: body.projectId,
        }));
        return;
      }
      if (request.method === "GET" && url.pathname === "/api/four-core") {
        sendJson(response, 200, service.fourCoreSurface());
        return;
      }
      const fourCoreWorkItemId = fourCoreRouteId(url.pathname, "work");
      if (request.method === "GET" && fourCoreWorkItemId) {
        sendJson(response, 200, service.fourCoreWorkNode({
          workItemId: fourCoreWorkItemId,
          expectedRevision: url.searchParams.get("revision"),
          expectedDigest: url.searchParams.get("digest"),
        }));
        return;
      }
      const fourCoreDecisionId = fourCoreRouteId(url.pathname, "decisions");
      if (request.method === "GET" && fourCoreDecisionId) {
        const revision = url.searchParams.get("revision");
        sendJson(response, 200, service.fourCoreDecisionHistory({
          decisionId: fourCoreDecisionId,
          expectedRevision: revision == null ? null : Number(revision),
          expectedDigest: url.searchParams.get("digest"),
        }));
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/conversation/messages") {
        const body = await readMutationJson(request, url);
        const conversation = await service.sendPrimaryConversationMessage({
          projectId: body.projectId,
          clientMessageId: body.clientMessageId,
          text: body.text,
        });
        sendJson(response, 200, {
          ...conversation,
          acknowledged_client_message_id: body.clientMessageId,
        });
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/conversation/reconnect") {
        await readMutationJson(request, url);
        sendJson(response, 200, await service.reconnectPrimaryConversation());
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/conversation/interrupt") {
        await readMutationJson(request, url);
        sendJson(response, 200, await service.interruptPrimaryConversation());
        return;
      }
      if (request.method === "GET" && url.pathname === "/api/development-radar") {
        sendJson(response, 200, service.developmentRadar());
        return;
      }
      const refreshProjectId = projectRouteId(url.pathname, "refresh");
      if (request.method === "POST" && refreshProjectId) {
        await readMutationJson(request, url);
        sendJson(response, 200, await service.refreshProject(refreshProjectId));
        return;
      }
      const projectId = projectRouteId(url.pathname);
      if (request.method === "GET" && projectId) {
        const project = store.getProjectRegistration(projectId);
        if (!project) {
          sendJson(response, 404, { error: "registered project not found" });
          return;
        }
        sendJson(response, 200, project);
        return;
      }
      if (request.method === "GET" && url.pathname === "/api/installation-receipts") {
        const limit = url.searchParams.has("limit") ? Number(url.searchParams.get("limit")) : 50;
        const offset = url.searchParams.has("offset") ? Number(url.searchParams.get("offset")) : 0;
        sendJson(response, 200, service.installationReceipts({ limit, offset }));
        return;
      }
      const installationReceiptId = installationReceiptRouteId(url.pathname);
      if (request.method === "GET" && installationReceiptId) {
        const receipt = service.installationReceipt(installationReceiptId);
        if (!receipt) {
          sendJson(response, 404, { error: "installation receipt not found" });
          return;
        }
        sendJson(response, 200, receipt);
        return;
      }
      const continuationId = routeId(url.pathname, "continuation");
      if (request.method === "GET" && continuationId) {
        sendJson(response, 200, service.continuation(continuationId));
        return;
      }
      if (request.method === "GET" && url.pathname.startsWith("/api/work-items/")) {
        const id = decodeURIComponent(url.pathname.slice("/api/work-items/".length));
        sendJson(response, 200, service.workItem(id));
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/projects") {
        const body = await readMutationJson(request, url);
        const result = await service.registerProject({ name: body.name, rootPath: body.rootPath });
        sendJson(response, result.created ? 201 : 200, {
          ...result.project,
          created: result.created,
        });
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/work-items") {
        const body = await readMutationJson(request, url);
        sendJson(response, 201, service.createWorkItem({
          projectId: body.projectId,
          title: body.title,
          objective: body.objective,
          priority: body.priority,
        }));
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/installation-receipts") {
        const body = await readMutationJson(request, url);
        const result = service.recordInstallationReceipt({
          projectId: body.projectId,
          component: body.component,
          sourceCommitSha: body.sourceCommitSha,
          artifactPath: body.artifactPath,
          artifactSha256: body.artifactSha256,
        });
        sendJson(response, result.created ? 201 : 200, result.receipt);
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/development-radar") {
        const body = await readMutationJson(request, url);
        sendJson(response, 200, service.replaceDevelopmentRadar(body));
        return;
      }

      for (const action of [
        "start",
        "resume",
        "interrupt",
        "reconcile",
        "approve",
        "verify",
        "archive",
      ]) {
        const id = routeId(url.pathname, action);
        if (!id || request.method !== "POST") continue;
        const body = await readMutationJson(request, url);
        let result;
        if (action === "start") result = await service.start(id);
        else if (action === "resume") result = await service.resume(id, body.prompt);
        else if (action === "interrupt") result = await service.interrupt(id);
        else if (action === "reconcile") result = await service.reconcile(id);
        else if (action === "approve") result = await service.approve(id, body.decision);
        else if (action === "verify") result = service.verify(id, body.notes);
        else result = await service.archive(id);
        sendJson(response, 200, result);
        return;
      }

      sendJson(response, 404, { error: "not found" });
    } catch (error) {
      sendJson(response, Number.isInteger(error?.status) ? error.status : 400, {
        error: publicErrorMessage(error?.message),
        code: publicErrorCode(error),
      });
    } finally {
      if (trackedRequest) inFlightRequests.delete(trackedRequest);
      finishTrackedRequest?.();
    }
  });

  const closeListener = () => {
    if (serverClosePromise) return serverClosePromise;
    if (!server.listening) {
      serverClosePromise = Promise.resolve();
      return serverClosePromise;
    }
    serverClosePromise = new Promise((resolve, reject) => {
      server.close((error) => (error ? reject(error) : resolve()));
    });
    return serverClosePromise;
  };

  const drainRequests = async (deadline) => {
    while (inFlightRequests.size > 0) {
      const snapshot = [...inFlightRequests];
      const result = await settleWithin(Promise.allSettled(snapshot), deadline);
      if (!result.settled) throw shutdownDrainTimeoutError();
    }
  };

  server.on("close", () => {
    if (ownedShutdown) return;
    service.close();
    void codex.close();
    store.close();
  });
  const application = {
    server,
    service,
    store,
    codex,
    stopAcceptingEffects() {
      acceptingEffects = false;
      service.stopAcceptingEffects();
    },
    shutdownOwned({ timeoutMs = 5_000 } = {}) {
      if (ownedShutdownPromise) return ownedShutdownPromise;
      ownedShutdown = true;
      application.stopAcceptingEffects();
      ownedShutdownPromise = (async () => {
        const deadline = Date.now() + timeoutMs;
        const outcome = await service.shutdown({
          timeoutMs: Math.max(1, deadline - Date.now()),
        });
        await drainRequests(deadline);
        service.close();
        const codexResult = await settleWithin(codex.close(), deadline);
        if (!codexResult.settled) throw shutdownDrainTimeoutError();
        if (codexResult.error) throw codexResult.error;
        store.close();
        server.closeIdleConnections?.();
        const listenerResult = await settleWithin(closeListener(), deadline);
        if (!listenerResult.settled || listenerResult.error) throw shutdownDrainTimeoutError();
        return outcome;
      })();
      return ownedShutdownPromise;
    },
  };
  return application;
}

export function attachDesktopShutdownChannel(application, {
  input = process.stdin,
  databasePath,
  timeoutMs = 5_000,
} = {}) {
  const expectedDigest = controlDataScopeDescriptor(databasePath).digest;
  let buffer = Buffer.alloc(0);
  let detached = false;
  const detach = ({ destroy = false } = {}) => {
    if (detached) return;
    detached = true;
    input.off("data", onData);
    input.off("end", onEnd);
    input.off("error", onEnd);
    input.pause();
    if (destroy) input.destroy?.();
  };
  const failClosed = () => detach();
  const accept = (frame) => {
    if (
      frame === null
      || typeof frame !== "object"
      || Array.isArray(frame)
      || Object.keys(frame).sort().join("\0")
        !== ["schema_version", "operation", "data_scope_digest"].sort().join("\0")
      || frame.schema_version !== desktopShutdownSchemaVersion
      || frame.operation !== "shutdown"
      || frame.data_scope_digest !== expectedDigest
    ) {
      failClosed();
      return;
    }
    detach({ destroy: true });
    void application.shutdownOwned({ timeoutMs }).then(
      () => { process.exitCode = 0; },
      () => { process.exitCode = 1; },
    );
  };
  function onData(chunk) {
    if (detached) return;
    if (buffer.length + chunk.length > maximumDesktopShutdownFrameBytes) {
      failClosed();
      return;
    }
    buffer = Buffer.concat([buffer, chunk]);
    const newline = buffer.indexOf(0x0a);
    if (newline < 0) return;
    const frame = buffer.subarray(0, newline);
    if (frame.length === 0 || buffer.subarray(newline + 1).length !== 0) {
      failClosed();
      return;
    }
    try {
      accept(JSON.parse(frame.toString("utf8")));
    } catch {
      failClosed();
    }
  }
  function onEnd() {
    failClosed();
  }
  input.on("data", onData);
  input.on("end", onEnd);
  input.on("error", onEnd);
  return { close: failClosed };
}

export async function startDefaultServer() {
  const port = Number(process.env.LATTICE_CONTROL_PORT || 4317);
  const databasePath = defaultControlDatabasePath();
  const application = createLatticeServer({ databasePath });
  await new Promise((resolve, reject) => {
    application.server.once("error", reject);
    application.server.listen(port, "127.0.0.1", resolve);
  });
  process.stdout.write(`LATTICE Control: http://127.0.0.1:${port}\n`);
  if (process.env.LATTICE_CONTROL_DESKTOP_OWNED === "1") {
    attachDesktopShutdownChannel(application, { databasePath });
  }
  return application;
}

if (import.meta.url === pathToFileURL(process.argv[1] || "").href) {
  await startDefaultServer();
}
