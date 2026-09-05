import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { readFile, readdir, readlink } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { closedChildEnvironment } from "./lattice-runtime-health.mjs";

const execute = promisify(execFile);
async function ownsLoopbackListener(child, url) {
  const port = Number(new URL(url).port);
  if (!Number.isInteger(child.pid) || !Number.isInteger(port) || port < 1 || port > 65535) return false;
  if (process.platform === "win32") {
    const powershell = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
    const command = `@(Get-NetTCPConnection -State Listen -LocalPort ${port} -ErrorAction SilentlyContinue | Where-Object { $_.OwningProcess -eq ${child.pid} -and $_.LocalAddress -eq '127.0.0.1' }).Count`;
    const result = await execute(powershell, ["-NoProfile", "-NonInteractive", "-Command", command], {
      env: closedChildEnvironment({}), windowsHide: true, timeout: 10000, maxBuffer: 4096,
    });
    return result.stdout.trim() === "1";
  }
  if (process.platform === "linux") {
    const descriptors = await readdir(`/proc/${child.pid}/fd`);
    if (descriptors.length > 1024) return false;
    const links = await Promise.all(descriptors.map((fd) => readlink(`/proc/${child.pid}/fd/${fd}`).catch(() => "")));
    const owned = new Set(links.map((link) => /^socket:\[([0-9]+)\]$/u.exec(link)?.[1]).filter(Boolean));
    const table = await readFile(`/proc/${child.pid}/net/tcp`, "utf8");
    return table.split("\n").some((line) => {
      const fields = line.trim().split(/\s+/u);
      return fields[1] === `0100007F:${port.toString(16).toUpperCase().padStart(4, "0")}` && fields[3] === "0A" && owned.has(fields[9]);
    });
  }
  return false;
}

export async function isOwnedResultPreview(preview) {
  return preview.child.exitCode === null && preview.child.signalCode === null
    && await ownsLoopbackListener(preview.child, preview.url).catch(() => false);
}

export function startResultPreview(artifact) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [fileURLToPath(new URL("./result-preview-worker.mjs", import.meta.url)), artifact.path, artifact.sha256], {
      cwd: path.dirname(artifact.path), env: closedChildEnvironment({}),
      stdio: ["ignore", "pipe", "pipe", "ipc"], windowsHide: true,
    });
    let settled = false, checking = false, bytes = 0;
    const failure = () => {
      if (settled) return;
      settled = true; clearTimeout(timer); child.kill();
      reject(Object.assign(new Error("成果目前無法開啟；已保存的完成紀錄仍保留。"), { code: "CONTROL_RESULT_PREVIEW_UNAVAILABLE" }));
    };
    const timer = setTimeout(failure, 15000);
    child.once("error", failure); child.once("exit", failure);
    const diagnostic = (chunk) => {
      bytes += chunk.length;
      if (bytes > 65536) { child.kill(); failure(); return; }
    };
    child.stderr.on("data", diagnostic); child.stdout.on("data", diagnostic);
    child.on("message", async (packet) => {
      if (settled || checking) return;
      checking = true;
      try {
        if (packet?.kind !== "LATTICE_RESULT_READY" || !/^http:\/\/127\.0\.0\.1:[0-9]{1,5}$/u.test(packet.url)) throw new Error();
        if (!await ownsLoopbackListener(child, packet.url)) throw new Error();
        if (settled) return;
        settled = true; clearTimeout(timer); resolve({ child, url: packet.url });
      } catch { failure(); }
    });
  });
}

export async function closeResultPreview(preview) {
  if (preview.child.exitCode !== null || preview.child.signalCode !== null) return;
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("CONTROL_RESULT_PREVIEW_CLOSE_TIMEOUT")), 5000);
    preview.child.once("exit", () => { clearTimeout(timer); resolve(); });
    preview.child.kill();
  });
}
