import { execFile } from "node:child_process";
import { lstat, readdir } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";

const runFile = promisify(execFile);

async function installedVersion(command) {
  const { stdout } = await runFile(command, ["--version"], {
    timeout: 5000,
    maxBuffer: 4096,
    windowsHide: true,
  });
  return stdout.trim();
}

async function ordinaryPath(file, kind) {
  try {
    const stat = await lstat(file);
    return !stat.isSymbolicLink() && (kind === "directory" ? stat.isDirectory() : stat.isFile());
  } catch {
    return false;
  }
}

function parseCodexVersion(value) {
  if (typeof value !== "string") return null;
  const match = /^codex-cli (0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/u.exec(value);
  if (!match || match[0] !== value) return null;
  const prerelease = match[4]?.split(".") ?? [];
  if (prerelease.some(identifier => /^0[0-9]+$/u.test(identifier))) return null;
  // SemVer integers have no precision bound; build metadata has no precedence.
  return { core: match.slice(1, 4).map(BigInt), prerelease };
}

const compare = (left, right) => left === right ? 0 : left < right ? -1 : 1;
function compareVersions(left, right) {
  for (let index = 0; index < 3; index += 1) {
    const order = compare(left.core[index], right.core[index]);
    if (order) return order;
  }
  const a = left.prerelease, b = right.prerelease;
  if (!a.length || !b.length) return compare(!a.length, !b.length);
  for (let index = 0; index < Math.min(a.length, b.length); index += 1) {
    if (a[index] === b[index]) continue;
    const numericA = /^[0-9]+$/u.test(a[index]), numericB = /^[0-9]+$/u.test(b[index]);
    if (numericA && numericB) return compare(BigInt(a[index]), BigInt(b[index]));
    if (numericA !== numericB) return numericA ? -1 : 1;
    return compare(a[index], b[index]); // ASCII identifier order, not locale order.
  }
  return compare(a.length, b.length);
}

// Explicit codexBin/launchSpec overrides are resolved by the caller first.
// The desktop application bundles its own CLI; an older npm installation can
// otherwise hide models already supported by the installed desktop application.
export async function resolveWindowsCodexRuntime({
  env = process.env,
  nodeExecutable = process.execPath,
  probeVersion = installedVersion,
} = {}) {
  const candidates = [];
  if (env.LOCALAPPDATA && path.isAbsolute(env.LOCALAPPDATA)) {
    const root = path.join(env.LOCALAPPDATA, "OpenAI", "Codex", "bin");
    if (await ordinaryPath(root, "directory")) {
      const entries = await readdir(root, { withFileTypes: true });
      for (const entry of entries) {
        if (!entry.isDirectory() || !/^[a-f0-9]{16}$/u.test(entry.name)) continue;
        const command = path.join(root, entry.name, "codex.exe");
        if (!await ordinaryPath(command, "file")) continue;
        try {
          const version = parseCodexVersion(await probeVersion(command));
          if (version) candidates.push({ command, version });
        } catch {
          // An incomplete desktop update is not an executable installation.
        }
      }
    }
  }
  candidates.sort((left, right) => compareVersions(right.version, left.version)
    || left.command.localeCompare(right.command));
  if (candidates.length) return { command: candidates[0].command, args: ["app-server", "--stdio"] };

  if (env.APPDATA && path.isAbsolute(env.APPDATA)) {
    const script = path.join(env.APPDATA, "npm", "node_modules", "@openai", "codex", "bin", "codex.js");
    if (await ordinaryPath(script, "file")) {
      return { command: nodeExecutable, args: [script, "app-server", "--stdio"] };
    }
  }
  throw new Error("Codex runtime was not found; install Codex or set codexBin to an exact trusted path");
}
