import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";
import { CodexAppServer } from "./codex-app-server.mjs";
import { defaultControlDatabasePath } from "./database-path.mjs";
import { LatticeControlService } from "./service.mjs";
import { LatticeStore } from "./store.mjs";

const sourceDirectory = path.dirname(fileURLToPath(import.meta.url));
const publicDirectory = path.resolve(sourceDirectory, "..", "public");

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

export function createLatticeServer({
  databasePath,
  codex = new CodexAppServer(),
  projectInspector,
}) {
  const store = new LatticeStore(databasePath);
  const service = new LatticeControlService({
    store,
    codex,
    ...(projectInspector ? { projectInspector } : {}),
  });
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, "http://127.0.0.1");
    try {
      validateLoopbackRequest(request);
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
      if (request.method === "GET" && url.pathname === "/api/conversation") {
        sendJson(response, 200, service.primaryConversation());
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/conversation/messages") {
        const body = await readJson(request);
        sendJson(response, 200, await service.sendPrimaryConversationMessage({
          projectId: body.projectId,
          clientMessageId: body.clientMessageId,
          text: body.text,
        }));
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/conversation/reconnect") {
        await readJson(request);
        sendJson(response, 200, await service.reconnectPrimaryConversation());
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/conversation/interrupt") {
        await readJson(request);
        sendJson(response, 200, await service.interruptPrimaryConversation());
        return;
      }
      if (request.method === "GET" && url.pathname === "/api/development-radar") {
        sendJson(response, 200, service.developmentRadar());
        return;
      }
      const refreshProjectId = projectRouteId(url.pathname, "refresh");
      if (request.method === "POST" && refreshProjectId) {
        await readJson(request);
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
        const body = await readJson(request);
        const result = await service.registerProject({ name: body.name, rootPath: body.rootPath });
        sendJson(response, result.created ? 201 : 200, {
          ...result.project,
          created: result.created,
        });
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/work-items") {
        const body = await readJson(request);
        sendJson(response, 201, service.createWorkItem({
          projectId: body.projectId,
          title: body.title,
          objective: body.objective,
          priority: body.priority,
        }));
        return;
      }
      if (request.method === "POST" && url.pathname === "/api/installation-receipts") {
        const body = await readJson(request);
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
        const body = await readJson(request);
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
        const body = await readJson(request);
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
        error: error.message,
        code: typeof error.code === "string"
          ? error.code
          : error instanceof TypeError
            ? "INVALID_REQUEST"
            : "CONTROL_REQUEST_FAILED",
      });
    }
  });

  server.on("close", () => {
    service.close();
    void codex.close();
    store.close();
  });
  return { server, service, store, codex };
}

export async function startDefaultServer() {
  const port = Number(process.env.LATTICE_CONTROL_PORT || 4317);
  const application = createLatticeServer({ databasePath: defaultControlDatabasePath() });
  await new Promise((resolve, reject) => {
    application.server.once("error", reject);
    application.server.listen(port, "127.0.0.1", resolve);
  });
  process.stdout.write(`LATTICE Control: http://127.0.0.1:${port}\n`);
  return application;
}

if (import.meta.url === pathToFileURL(process.argv[1] || "").href) {
  await startDefaultServer();
}
