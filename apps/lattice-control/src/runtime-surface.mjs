import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { controlDataScopeDescriptor } from "./database-path.mjs";

const runtimeSurfaceSchemaVersion = "lattice.control.runtime-surface.v2";
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

function capability(id, label, status, hasData = null) {
  if (!capabilityStatuses.has(status)) {
    throw new Error("CONTROL_RUNTIME_CAPABILITY_STATUS_INVALID");
  }
  if (hasData !== null && typeof hasData !== "boolean") {
    throw new Error("CONTROL_RUNTIME_CAPABILITY_DATA_INVALID");
  }
  return { id, label, status, has_data: hasData };
}

export function createRuntimeSurface(service, { databasePath, mcpHealth, runtimeHealth, formalRuntime = null }) {
  const state = service.state();
  const dataPresence = service.runtimeDataPresence();

  return {
    schema_version: runtimeSurfaceSchemaVersion,
    identity: { ...controlRuntimeIdentity },
    data_scope: controlDataScopeDescriptor(databasePath),
    reconciliation_required: service.reconciliationRequired(),
    health: "HEALTHY",
    capabilities: [
      capability("control_sqlite", "Control／SQLite", "HEALTHY"),
      capability(
        "codex_app_server",
        "Codex App Server",
        state.codexConnected || formalRuntime?.codexConnected ? "HEALTHY" : "STOPPED",
      ),
      capability("work_mcp", "Work MCP", mcpHealth.work_mcp, formalRuntime ? formalRuntime.work : dataPresence.work),
      capability(
        "decision_mcp",
        "Decision MCP",
        mcpHealth.decision_mcp,
        formalRuntime ? formalRuntime.decisions : dataPresence.decisions,
      ),
      capability("postgresql", "正式 PostgreSQL", runtimeHealth.postgresql),
    ],
  };
}
