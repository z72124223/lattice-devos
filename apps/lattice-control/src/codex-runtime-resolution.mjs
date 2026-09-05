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
          const version = /^codex-cli (\d+)\.(\d+)\.(\d+)$/u.exec(await probeVersion(command));
          if (version) candidates.push({ command, version: version.slice(1).map(Number) });
        } catch {
          // An incomplete desktop update is not an executable installation.
        }
      }
    }
  }
  candidates.sort((left, right) => {
    for (let index = 0; index < 3; index += 1) {
      const order = right.version[index] - left.version[index];
      if (order) return order;
    }
    return left.command.localeCompare(right.command);
  });
  if (candidates.length) return { command: candidates[0].command, args: ["app-server", "--stdio"] };

  if (env.APPDATA && path.isAbsolute(env.APPDATA)) {
    const script = path.join(env.APPDATA, "npm", "node_modules", "@openai", "codex", "bin", "codex.js");
    if (await ordinaryPath(script, "file")) {
      return { command: nodeExecutable, args: [script, "app-server", "--stdio"] };
    }
  }
  throw new Error("Codex runtime was not found; install Codex or set codexBin to an exact trusted path");
}
