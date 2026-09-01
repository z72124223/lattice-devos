import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { createServer, request as createRequest } from "node:http";

const readyPath = process.env.LATTICE_DESKTOP_GATEWAY_READY;
const modePath = process.env.LATTICE_DESKTOP_GATEWAY_MODE;
const markerPath = process.env.LATTICE_DESKTOP_REDIRECT_MARKER;
const backendPortPath = process.env.LATTICE_DESKTOP_BACKEND_PORT;
const captureReadyPath = process.env.LATTICE_DESKTOP_CAPTURE_READY;
const captureMarkerPath = process.env.LATTICE_DESKTOP_CAPTURE_MARKER;
if (!readyPath || !modePath || !markerPath || !backendPortPath
    || !captureReadyPath || !captureMarkerPath) {
  throw new Error("desktop test gateway paths are required");
}

let capturePort = 0;

function currentMode() {
  return readFileSync(modePath, "utf8").trim();
}

const server = createServer((incoming, outgoing) => {
  const mode = currentMode();
  if (mode === "offline") {
    incoming.socket.destroy();
    return;
  }

  if (mode === "redirect") {
    if (!existsSync(markerPath)) {
      writeFileSync(markerPath, `${incoming.method} ${incoming.url}\n`, {
        encoding: "utf8",
        flag: "wx",
      });
    }
    outgoing.writeHead(302, {
      "Cache-Control": "no-store",
      Location: `http://127.0.0.1:${capturePort}/outside`,
    });
    outgoing.end();
    return;
  }

  if (mode !== "proxy") {
    outgoing.writeHead(503, { "Cache-Control": "no-store" });
    outgoing.end();
    return;
  }

  const backendPort = Number(readFileSync(backendPortPath, "utf8").trim());
  if (!Number.isInteger(backendPort) || backendPort < 1 || backendPort > 65_535) {
    incoming.socket.destroy();
    return;
  }

  const headers = { ...incoming.headers, host: `127.0.0.1:${backendPort}` };
  if (headers.origin) {
    headers.origin = `http://127.0.0.1:${backendPort}`;
  }
  const proxy = createRequest({
    hostname: "127.0.0.1",
    port: backendPort,
    method: incoming.method,
    path: incoming.url,
    headers,
  }, (response) => {
    outgoing.writeHead(response.statusCode ?? 502, response.headers);
    response.pipe(outgoing);
  });
  proxy.on("error", () => incoming.socket.destroy());
  incoming.pipe(proxy);
});

server.on("clientError", (_error, socket) => socket.destroy());
const captureServer = createServer((request, response) => {
  if (!existsSync(captureMarkerPath)) {
    writeFileSync(captureMarkerPath, `${request.method} ${request.url}\n`, {
      encoding: "utf8",
      flag: "wx",
    });
  }
  response.writeHead(204, { "Cache-Control": "no-store" });
  response.end();
});
captureServer.on("clientError", (_error, socket) => socket.destroy());
captureServer.listen(0, "127.0.0.1", () => {
  const captureAddress = captureServer.address();
  if (!captureAddress || typeof captureAddress === "string") {
    throw new Error("desktop test capture server did not bind TCP");
  }
  capturePort = captureAddress.port;
  writeFileSync(captureReadyPath, `${capturePort}\n`, { encoding: "utf8", flag: "wx" });
  server.listen(0, "127.0.0.1", () => {
    const address = server.address();
    if (!address || typeof address === "string") {
      throw new Error("desktop test gateway did not bind TCP");
    }
    writeFileSync(readyPath, `${address.port}\n`, { encoding: "utf8", flag: "wx" });
  });
});
