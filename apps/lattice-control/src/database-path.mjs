import path from "node:path";
import process from "node:process";

export function defaultControlDatabasePath({ env = process.env, cwd = process.cwd() } = {}) {
  const base = env.LOCALAPPDATA || path.join(cwd, ".lattice");
  return path.join(base, "LATTICE", "control", "lattice-control.db");
}
