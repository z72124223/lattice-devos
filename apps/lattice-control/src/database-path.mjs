import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

function exactKeys(value, keys) {
  return value !== null
    && typeof value === "object"
    && !Array.isArray(value)
    && Object.keys(value).sort().join("\0") === [...keys].sort().join("\0");
}

function loadDataScopeContract() {
  const contractPath = fileURLToPath(new URL("../data-scope-contract.json", import.meta.url));
  const contract = JSON.parse(readFileSync(contractPath, "utf8"));
  if (
    !exactKeys(contract, [
      "schema_version",
      "store",
      "store_schema_version",
      "authority_class",
      "registry_authority",
    ])
    || contract.schema_version !== "lattice.control.data-scope.v1"
    || contract.store !== "CONTROL_SQLITE"
    || !Number.isInteger(contract.store_schema_version)
    || contract.store_schema_version < 1
    || contract.authority_class !== "CONTROL_LOCAL_PRODUCT_STATE"
    || contract.registry_authority !== "NONE"
  ) throw new Error("CONTROL_DATA_SCOPE_CONTRACT_INVALID");
  return Object.freeze({ ...contract });
}

export const controlDataScopeContract = loadDataScopeContract();
export const controlDataScopeSchemaVersion = controlDataScopeContract.schema_version;
export const controlStoreSchemaVersion = controlDataScopeContract.store_schema_version;
export const controlAuthorityClass = controlDataScopeContract.authority_class;
export const controlRegistryAuthority = controlDataScopeContract.registry_authority;

function asciiFoldWindowsPath(value) {
  return value.replace(/[A-Z]/gu, (character) => character.toLowerCase());
}

function normalizedScopePath(databasePath) {
  if (typeof databasePath !== "string" || databasePath.length === 0 || databasePath.includes("\0")) {
    throw new TypeError("CONTROL_DATABASE_PATH_INVALID");
  }
  const resolved = path.resolve(databasePath).replaceAll("\\", "/");
  return process.platform === "win32" ? asciiFoldWindowsPath(resolved) : resolved;
}

function scopePreimage(values) {
  const chunks = [];
  for (const value of values) {
    const encoded = Buffer.from(String(value), "utf8");
    const length = Buffer.allocUnsafe(4);
    length.writeUInt32BE(encoded.length);
    chunks.push(length, encoded);
  }
  return Buffer.concat(chunks);
}

export function controlDataScopeDescriptor(databasePath) {
  const preimage = scopePreimage([
    controlDataScopeSchemaVersion,
    controlDataScopeContract.store,
    controlStoreSchemaVersion,
    controlAuthorityClass,
    controlRegistryAuthority,
    normalizedScopePath(databasePath),
  ]);
  return Object.freeze({
    schema_version: controlDataScopeSchemaVersion,
    store: controlDataScopeContract.store,
    store_schema_version: controlStoreSchemaVersion,
    authority_class: controlAuthorityClass,
    registry_authority: controlRegistryAuthority,
    digest: createHash("sha256").update(preimage).digest("hex"),
  });
}

export function defaultControlDatabasePath({ env = process.env, cwd = process.cwd() } = {}) {
  if (typeof env.LATTICE_CONTROL_DATABASE_PATH === "string"
    && env.LATTICE_CONTROL_DATABASE_PATH.length > 0) {
    return path.resolve(cwd, env.LATTICE_CONTROL_DATABASE_PATH);
  }
  const base = env.LOCALAPPDATA || path.join(cwd, ".lattice");
  return path.join(base, "LATTICE", "control", "lattice-control.db");
}
