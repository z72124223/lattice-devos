import { writeFileSync } from "node:fs";
import { createServer } from "node:http";

const readyPath = process.env.LATTICE_DESKTOP_INCOMPATIBLE_READY;
const port = Number.parseInt(process.env.LATTICE_DESKTOP_INCOMPATIBLE_PORT ?? "4317", 10);
if (!readyPath || !Number.isInteger(port) || port < 1 || port > 65_535) {
  throw new Error("valid incompatible Control ready path and port are required");
}

const surface = JSON.stringify({
  schema_version: "lattice.control.runtime-surface.v1",
  identity: {
    schema_version: "lattice.control.runtime-identity.v1",
    product: "LATTICE_CONTROL",
    version: "0.0.0-foreign",
  },
  health: "HEALTHY",
  capabilities: [
    { id: "control_sqlite", label: "Control／SQLite", status: "HEALTHY" },
    { id: "codex_app_server", label: "Codex App Server", status: "STOPPED" },
    { id: "work_mcp", label: "Work MCP", status: "NO_DATA" },
    { id: "decision_mcp", label: "Decision MCP", status: "NO_DATA" },
    { id: "postgresql", label: "正式 PostgreSQL", status: "NOT_IMPLEMENTED" },
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
