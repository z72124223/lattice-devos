import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";
import { Server } from "node:http";

let activeServer;
process.on("disconnect", () => {
  if (!activeServer) process.exit(0);
  activeServer.close(() => process.exit(0));
  setTimeout(() => process.exit(0), 1000).unref();
});

// A completed web artifact exposes startServer; no Runtime credentials or
// execution harness are supplied to this ordinary application process.
try {
  const [artifactPath, expectedHash] = process.argv.slice(2);
  if (createHash("sha256").update(await readFile(artifactPath)).digest("hex") !== expectedHash) throw new Error("RESULT_CHANGED");
  const artifact = await import(pathToFileURL(artifactPath).href);
  if (typeof artifact.startServer !== "function") throw new Error("RESULT_PREVIEW_UNSUPPORTED");
  const server = await artifact.startServer({ port: 0, host: "127.0.0.1" });
  activeServer = server;
  const address = server instanceof Server && server.address();
  if (!address || typeof address === "string" || address.address !== "127.0.0.1") {
    server?.close?.(); throw new Error("RESULT_LOOPBACK_REQUIRED");
  }
  if (!process.connected) throw new Error("RESULT_PARENT_GONE");
  process.send({ kind: "LATTICE_RESULT_READY", url: `http://127.0.0.1:${address.port}` });
  process.on("SIGTERM", () => server.close(() => process.exit(0)));
} catch {
  process.stderr.write("RESULT_PREVIEW_UNAVAILABLE\n"); process.exit(2);
}
