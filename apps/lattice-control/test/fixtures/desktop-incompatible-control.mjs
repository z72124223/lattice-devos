import { writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { controlDataScopeDescriptor } from "../../src/database-path.mjs";

const readyPath = process.env.LATTICE_DESKTOP_INCOMPATIBLE_READY;
const port = Number.parseInt(process.env.LATTICE_DESKTOP_INCOMPATIBLE_PORT ?? "4317", 10);
const databasePath = process.env.LATTICE_CONTROL_DATABASE_PATH;
if (!readyPath || !databasePath || !Number.isInteger(port) || port < 1 || port > 65_535) {
  throw new Error("valid incompatible Control ready path and port are required");
}

const surface = JSON.stringify({
  schema_version: "lattice.control.runtime-surface.v2",
  identity: {
    schema_version: "lattice.control.runtime-identity.v1",
    product: "LATTICE_CONTROL",
    version: "0.0.0-foreign",
  },
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
});

const server = createServer((request, response) => {
  if (request.url === "/api/runtime") {
    response.writeHead(200, {
      "content-type": "application/json; charset=utf-8",
      "content-length": Buffer.byteLength(surface),
      "cache-control": "no-store",
    });
    response.end(surface);
    return;
  }
  response.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
  response.end("foreign listener");
});

await new Promise((resolve, reject) => {
  server.once("error", reject);
  server.listen(port, "127.0.0.1", resolve);
});
writeFileSync(readyPath, `${port}\n`, { encoding: "utf8", flag: "wx" });
