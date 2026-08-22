import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";
import { CodexAppServer } from "./codex-app-server.mjs";
import { LatticeControlService } from "./service.mjs";
import { LatticeStore } from "./store.mjs";

const sourceDirectory = path.dirname(fileURLToPath(import.meta.url));
const publicDirectory = path.resolve(sourceDirectory, "..", "public");

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

function routeId(pathname, action) {
  const match = pathname.match(new RegExp(`^/api/work-items/([^/]+)/${action}$`, "u"));
  return match ? decodeURIComponent(match[1]) : null;
}

export function createLatticeServer({ databasePath, codex = new CodexAppServer() }) {
  const store = new LatticeStore(databasePath);
  const service = new LatticeControlService({ store, codex });
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, "http://127.0.0.1");
    try {
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
        sendJson(response, 201, service.createProject({ name: body.name, rootPath: body.rootPath }));
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

      for (const action of ["start", "resume", "approve", "verify", "archive"]) {
        const id = routeId(url.pathname, action);
        if (!id || request.method !== "POST") continue;
        const body = await readJson(request);
        const result = action === "start"
          ? await service.start(id)
          : action === "resume"
            ? await service.resume(id, body.prompt)
            : action === "approve"
              ? service.approve(id, body.decision)
              : action === "verify"
                ? service.verify(id, body.notes)
                : await service.archive(id);
        sendJson(response, 200, result);
        return;
      }

      sendJson(response, 404, { error: "not found" });
    } catch (error) {
      sendJson(response, 400, { error: error.message });
    }
  });

  server.on("close", () => {
    service.close();
    void codex.close();
    store.close();
  });
  return { server, service, store, codex };
}

function defaultDatabasePath() {
  const base = process.env.LOCALAPPDATA || path.join(process.cwd(), ".lattice");
  return path.join(base, "LATTICE", "control", "lattice-control.db");
}

export async function startDefaultServer() {
  const port = Number(process.env.LATTICE_CONTROL_PORT || 4317);
  const application = createLatticeServer({ databasePath: defaultDatabasePath() });
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
