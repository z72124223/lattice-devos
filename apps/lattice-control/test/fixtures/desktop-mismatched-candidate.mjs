import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

const role = process.env.LATTICE_DESKTOP_MISMATCH_ROLE ?? "root";
const readyPath = process.env.LATTICE_DESKTOP_MISMATCH_READY;
const storePath = process.env.LATTICE_DESKTOP_MISMATCH_STORE;

if (!readyPath || !["root", "child", "external"].includes(role)) {
  throw new Error("valid mismatch fixture role and ready path are required");
}

if (role === "external") {
  const server = createServer((socket) => socket.end());
  server.listen(0, "127.0.0.1", () => {
    const address = server.address();
    writeFileSync(readyPath, `${JSON.stringify({
      role,
      pid: process.pid,
      port: address.port,
    })}\n`, { encoding: "utf8", flag: "wx" });
  });
} else if (role === "child") {
  const rootPid = Number.parseInt(
    process.env.LATTICE_DESKTOP_MISMATCH_ROOT_PID ?? "",
    10,
  );
  if (!storePath || !Number.isInteger(rootPid) || rootPid < 1) {
    throw new Error("mismatch child requires its owned root and store path");
  }

  mkdirSync(dirname(storePath), { recursive: true });
  writeFileSync(storePath, `owned by ${process.pid}\n`, {
    encoding: "utf8",
    flag: "wx",
  });
  const server = createServer((socket) => socket.end());
  server.listen(4317, "127.0.0.1", () => {
    writeFileSync(readyPath, `${JSON.stringify({
      role,
      root_pid: rootPid,
      child_pid: process.pid,
      port: 4317,
      store_path: storePath,
    })}\n`, { encoding: "utf8", flag: "wx" });
  });
} else {
  if (!storePath) {
    throw new Error("mismatch root requires a store path");
  }
  const child = spawn(process.execPath, [fileURLToPath(import.meta.url)], {
    env: {
      ...process.env,
      LATTICE_DESKTOP_MISMATCH_ROLE: "child",
      LATTICE_DESKTOP_MISMATCH_ROOT_PID: String(process.pid),
    },
    stdio: "ignore",
    windowsHide: true,
  });
  child.unref();
  setInterval(() => {}, 1_000);
}
