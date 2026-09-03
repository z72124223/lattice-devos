import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import {
  request,
  stopControl,
  validateCandidateDirectory,
  validateLifecycleEvidence,
} from "../../../scripts/run-primary-conversation-acceptance.mjs";

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

async function writeCandidate(directory, sourceCommit) {
  const files = new Map([
    ["LATTICE.exe", "desktop"],
    ["LATTICE.dll", "assembly"],
    ["PORTABLE_RELEASE_CANDIDATE.txt", "candidate"],
    ["control-runtime/node.exe", "node"],
    ["control-runtime/apps/lattice-control/src/server.mjs", "server"],
    ["control-runtime/apps/lattice-control/runtime-identity.json", "identity"],
    ["control-runtime/apps/lattice-control/data-scope-contract.json", "scope"],
  ]);
  for (const [relativePath, content] of files) {
    const target = path.join(directory, ...relativePath.split("/"));
    await mkdir(path.dirname(target), { recursive: true });
    await writeFile(target, content, "utf8");
  }
  const manifest = {
    schema_version: "lattice.control.desktop-portable-candidate.v2",
    artifact_type: "PORTABLE_RELEASE_CANDIDATE",
    source_commit: sourceCommit,
    runtime_identifier: "win-x64",
    self_contained: true,
    launch: "LATTICE.exe",
    control_origin: "http://127.0.0.1:4317/",
    executable_sha256: sha256(files.get("LATTICE.exe")),
    control_runtime: {
      identity_schema: "lattice.control.runtime-identity.v1",
      product: "LATTICE_CONTROL",
      version: "1.0.0",
      data_scope_schema: "lattice.control.data-scope.v1",
      store_schema_version: 7,
      node_version: "v24.16.0",
      node_sha256: sha256(files.get("control-runtime/node.exe")),
      executable: "control-runtime/node.exe",
      server: "control-runtime/apps/lattice-control/src/server.mjs",
      database: "%LOCALAPPDATA%\\LATTICE\\control\\lattice-control.db",
    },
    files: [...files].map(([relativePath, content]) => ({
      path: relativePath,
      length: Buffer.byteLength(content),
      sha256: sha256(content),
    })),
  };
  const manifestPath = path.join(directory, "candidate-manifest.json");
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  return { manifest, manifestPath };
}

test("primary conversation acceptance aborts an individually stuck HTTP request", async () => {
  const sockets = new Set();
  const server = createServer(() => {});
  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  try {
    const { port } = server.address();
    await assert.rejects(
      request(`http://127.0.0.1:${port}`, "/stuck", { timeoutMs: 25 }),
      { code: "ACCEPTANCE_REQUEST_TIMEOUT" },
    );
  } finally {
    for (const socket of sockets) socket.destroy();
    await new Promise((resolve) => server.close(resolve));
  }
});

test("primary conversation acceptance requires a stopped IPC receipt and zero Control exit", async () => {
  class FakeChild extends EventEmitter {
    exitCode = null;
    signalCode = null;
    pid = 42_424;

    send(message) {
      assert.equal(message.type, "shutdown");
      setImmediate(() => {
        this.emit("message", { type: "stopped", pid: this.pid });
        this.exitCode = 0;
        this.emit("exit", 0, null);
      });
    }

    kill() {
      this.signalCode = "SIGKILL";
      this.emit("exit", null, this.signalCode);
    }
  }
  const receipt = await stopControl({ child: new FakeChild() });
  assert.deepEqual(receipt, {
    shutdown_requested: true,
    stopped_receipt: true,
    exit_code: 0,
    signal: null,
  });

  const exited = new FakeChild();
  exited.exitCode = 7;
  await assert.rejects(
    stopControl({ child: exited }),
    /exited before acceptance shutdown \(7\)/u,
  );
});

test("primary conversation acceptance validates the exact portable candidate v2 contract", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-live-candidate-contract-"));
  const sourceCommit = "a".repeat(40);
  try {
    const { manifest, manifestPath } = await writeCandidate(directory, sourceCommit);
    const validated = await validateCandidateDirectory(directory, sourceCommit);
    assert.equal(validated.desktop_executable_path, path.join(directory, "LATTICE.exe"));
    assert.equal(validated.verified_package_file_count, manifest.files.length);

    await writeFile(path.join(directory, "undeclared.bin"), "unexpected", "utf8");
    await assert.rejects(
      validateCandidateDirectory(directory, sourceCommit),
      /actual file set does not match/u,
    );
    await rm(path.join(directory, "undeclared.bin"));

    manifest.launch = "LATTICE.dll";
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
    await assert.rejects(
      validateCandidateDirectory(directory, sourceCommit),
      /semantic contract is incompatible/u,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("primary conversation acceptance requires exactly-once message events and rejects unknown turns", () => {
  const clientMessageIds = ["message-1", "message-2", "message-3"];
  const turnIds = ["turn-1", "turn-2", "turn-3"];
  const threadId = "thread-1";
  const events = [
    { event_id: 1, kind: "conversation_message_claimed", clientMessageId: clientMessageIds[0] },
    { event_id: 2, kind: "conversation_turn_dispatch_intended", clientMessageId: clientMessageIds[0] },
    { event_id: 3, kind: "conversation_message_accepted", clientMessageId: clientMessageIds[0], threadId, turnId: turnIds[0] },
    { event_id: 4, kind: "conversation_first_activity", threadId, turnId: turnIds[0], queueDurationMs: 10 },
    { event_id: 5, kind: "conversation_message_claimed", clientMessageId: clientMessageIds[1] },
    { event_id: 6, kind: "turn_completed", threadId, turnId: turnIds[0], status: "completed" },
    { event_id: 7, kind: "conversation_turn_dispatch_intended", clientMessageId: clientMessageIds[1] },
    { event_id: 8, kind: "conversation_message_accepted", clientMessageId: clientMessageIds[1], threadId, turnId: turnIds[1] },
    { event_id: 9, kind: "conversation_first_activity", threadId, turnId: turnIds[1], queueDurationMs: 11 },
    { event_id: 10, kind: "turn_completed", threadId, turnId: turnIds[1], status: "completed" },
    { event_id: 11, kind: "conversation_message_claimed", clientMessageId: clientMessageIds[2] },
    { event_id: 12, kind: "conversation_turn_dispatch_intended", clientMessageId: clientMessageIds[2] },
    { event_id: 13, kind: "conversation_message_accepted", clientMessageId: clientMessageIds[2], threadId, turnId: turnIds[2] },
    { event_id: 14, kind: "conversation_first_activity", threadId, turnId: turnIds[2], queueDurationMs: 12 },
    { event_id: 15, kind: "turn_completed", threadId, turnId: turnIds[2], status: "completed" },
  ];
  const validated = validateLifecycleEvidence(events, clientMessageIds, turnIds);
  assert.deepEqual(validated.acceptances.map(({ turnId }) => turnId), turnIds);

  assert.throws(
    () => validateLifecycleEvidence([
      ...events,
      { event_id: 16, kind: "conversation_turn_dispatch_intended", clientMessageId: clientMessageIds[1] },
    ], clientMessageIds, turnIds),
    /expected exactly one conversation_turn_dispatch_intended event, observed 2/u,
  );
  assert.throws(
    () => validateLifecycleEvidence([
      ...events,
      { event_id: 16, kind: "codex_started", threadId, turnId: "turn-foreign" },
    ], clientMessageIds, turnIds),
    /unexpected turn identity turn-foreign/u,
  );
});
