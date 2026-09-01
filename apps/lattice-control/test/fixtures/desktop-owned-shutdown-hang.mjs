import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";

import { controlDataScopeDescriptor } from "../../src/database-path.mjs";
import { controlRuntimeIdentity } from "../../src/runtime-surface.mjs";

const port = Number(process.env.LATTICE_CONTROL_PORT);
const databasePath = process.env.LATTICE_CONTROL_DATABASE_PATH;
const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 60_000)"], {
  stdio: "ignore",
  windowsHide: true,
});
const childPidPath = `${databasePath}.child.pid`;
mkdirSync(path.dirname(childPidPath), { recursive: true });
writeFileSync(childPidPath, String(child.pid), { encoding: "utf8", flag: "wx" });
const surface = {
  schema_version: "lattice.control.runtime-surface.v2",
  identity: { ...controlRuntimeIdentity },
  data_scope: controlDataScopeDescriptor(databasePath),
  reconciliation_required: false,
  health: "HEALTHY",
  capabilities: [
    { id: "control_sqlite", label: "Control／SQLite", status: "HEALTHY", has_data: null },
    { id: "codex_app_server", label: "Codex App Server", status: "STOPPED", has_data: null },
    { id: "work_mcp", label: "Work MCP", status: "HEALTHY", has_data: false },
    { id: "decision_mcp", label: "Decision MCP", status: "HEALTHY", has_data: false },
    { id: "postgresql", label: "正式 PostgreSQL", status: "NOT_IMPLEMENTED", has_data: null },
  ],
};

createServer((request, response) => {
  if (request.method !== "GET" || request.url !== "/api/runtime") {
    response.writeHead(404).end();
    return;
  }
  const body = JSON.stringify(surface);
  response.writeHead(200, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store",
  });
  response.end(body);
}).listen(port, "127.0.0.1");

// Deliberately do not consume the desktop-owned stdin shutdown frame. The
// lifecycle test proves the desktop's default bounded hard-kill fallback stops
// the exact process tree it owns, including this inert child.
