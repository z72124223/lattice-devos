import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const runtimeSurfaceSchemaVersion = "lattice.control.runtime-surface.v1";
const runtimeIdentitySchemaVersion = "lattice.control.runtime-identity.v1";
const capabilityStatuses = new Set([
  "HEALTHY",
  "NOT_IMPLEMENTED",
  "STOPPED",
  "UNREACHABLE",
  "INCOMPATIBLE",
  "NO_DATA",
]);

function exactKeys(value, keys) {
  return value !== null
    && typeof value === "object"
    && !Array.isArray(value)
    && Object.keys(value).sort().join("\0") === [...keys].sort().join("\0");
}

function loadRuntimeIdentity() {
  const identityPath = fileURLToPath(new URL("../runtime-identity.json", import.meta.url));
  const identity = JSON.parse(readFileSync(identityPath, "utf8"));
  if (
    !exactKeys(identity, ["schema_version", "product", "version"])
    || identity.schema_version !== runtimeIdentitySchemaVersion
    || identity.product !== "LATTICE_CONTROL"
    || typeof identity.version !== "string"
    || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/u.test(identity.version)
  ) {
    throw new Error("CONTROL_RUNTIME_IDENTITY_INVALID");
  }
  return Object.freeze({ ...identity });
}

export const controlRuntimeIdentity = loadRuntimeIdentity();

function capability(id, label, status) {
  if (!capabilityStatuses.has(status)) {
    throw new Error("CONTROL_RUNTIME_CAPABILITY_STATUS_INVALID");
  }
  return { id, label, status };
}

export function createRuntimeSurface(service) {
  const state = service.state();
  const fourCore = service.fourCoreSurface();
  const hasWorkData = Array.isArray(fourCore.work_snapshot?.graph?.nodes)
    && fourCore.work_snapshot.graph.nodes.length > 0;
  const hasDecisionData = Array.isArray(fourCore.decisions?.decisions)
    && fourCore.decisions.decisions.length > 0;

  return {
    schema_version: runtimeSurfaceSchemaVersion,
    identity: { ...controlRuntimeIdentity },
    health: "HEALTHY",
    capabilities: [
      capability("control_sqlite", "Control／SQLite", "HEALTHY"),
      capability(
        "codex_app_server",
        "Codex App Server",
        state.codexConnected ? "HEALTHY" : "STOPPED",
      ),
      capability("work_mcp", "Work MCP", hasWorkData ? "HEALTHY" : "NO_DATA"),
      capability(
        "decision_mcp",
        "Decision MCP",
        hasDecisionData ? "HEALTHY" : "NO_DATA",
      ),
      capability("postgresql", "正式 PostgreSQL", "NOT_IMPLEMENTED"),
    ],
  };
}
