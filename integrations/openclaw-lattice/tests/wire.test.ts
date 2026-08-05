import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { createServer, type Server, type Socket } from "node:net";
import { test } from "node:test";

import { canonicalJson, type CanonicalValue } from "../src/cjson.js";
import {
  parseLatticeArguments,
  type CommandIdentities,
  type LatticeCommand,
} from "../src/commands.js";
import {
  exchangeAuthenticatedFrames,
  type NonceSource,
  TRANSPORT_WIRE,
} from "../src/transport.js";
import {
  decodeCommandReply,
  encodeClientHello,
  encodeCommandRequest,
} from "../src/wire.js";

const IDENTITIES: CommandIdentities = {
  commandId: "command-a",
  correlationId: "correlation-a",
};

test("encodes exact closed ClientHello and command envelopes", () => {
  assert.equal(
    encodeClientHello("launch-a", "1".repeat(32)).toString("utf8"),
    `{"launch_record_id":"launch-a","process_start_nonce":"${"1".repeat(32)}","protocol":"lattice-openclaw-client-hello","version":"1"}`,
  );
  assert.equal(
    encodeCommandRequest(parseLatticeArguments(`submit ${"a".repeat(64)}`), IDENTITIES).toString(
      "utf8",
    ),
    `{"action":"submit","body":{"task_spec_digest":"${"a".repeat(64)}"},"command_id":"command-a","correlation_id":"correlation-a","protocol":"lattice-openclaw-inbound","version":"1"}`,
  );
  assert.equal(
    encodeCommandRequest(parseLatticeArguments("status project project-a"), IDENTITIES).toString(
      "utf8",
    ),
    '{"action":"status","body":{"cursor":null,"kind":"project","page_size":"10","project_id":"project-a"},"command_id":"command-a","correlation_id":"correlation-a","protocol":"lattice-openclaw-inbound","version":"1"}',
  );
  assert.equal(
    encodeCommandRequest(
      parseLatticeArguments("status command project-a target-command-a"),
      IDENTITIES,
    ).toString("utf8"),
    '{"action":"status","body":{"kind":"command","project_id":"project-a","target_command_id":"target-command-a"},"command_id":"command-a","correlation_id":"correlation-a","protocol":"lattice-openclaw-inbound","version":"1"}',
  );
  const taskTail = `project-a snapshot-a task-a 1 ${"a".repeat(64)} ${"b".repeat(64)}`;
  const taskTarget = `{"binding":{"project_id":"project-a","project_snapshot_id":"snapshot-a","task_id":"task-a","task_revision":"1","task_spec_digest":"${"a".repeat(64)}"},"expected_ledger_head_digest":"${"b".repeat(64)}"}`;
  assert.equal(
    encodeCommandRequest(parseLatticeArguments(`status task ${taskTail}`), IDENTITIES).toString(
      "utf8",
    ),
    `{"action":"status","body":{"kind":"task","target":${taskTarget}},"command_id":"command-a","correlation_id":"correlation-a","protocol":"lattice-openclaw-inbound","version":"1"}`,
  );
  assert.equal(
    encodeCommandRequest(
      parseLatticeArguments(`stop ${taskTail} attempt-a USER_REQUESTED`),
      IDENTITIES,
    ).toString("utf8"),
    `{"action":"stop","body":{"attempt_id":"attempt-a","reason":"USER_REQUESTED","target":${taskTarget}},"command_id":"command-a","correlation_id":"correlation-a","protocol":"lattice-openclaw-inbound","version":"1"}`,
  );
});

test("matches the Rust greeting, request, and response golden packet fixtures", async () => {
  const fixturePath = new URL(
    "../../../crates/lattice-openclaw-adapter/tests/fixtures/openclaw_wire_parity.json",
    import.meta.url,
  );
  const fixture = JSON.parse(await readFile(fixturePath, "utf8")) as GoldenFixture;
  assert.equal(fixture.session_greeting.magic_ascii, TRANSPORT_WIRE.sessionMagic);
  assert.equal(fixture.request_packet.magic_ascii, TRANSPORT_WIRE.requestMagic);
  assert.equal(fixture.response_packet.magic_ascii, TRANSPORT_WIRE.responseMagic);

  let capturedCommand: Buffer | undefined;
  const server = createServer((socket) => {
    void (async () => {
      socket.write(Buffer.from(fixture.session_greeting.bytes_hex, "hex"));
      await readPacket(socket);
      capturedCommand = await readPacket(socket);
      socket.end(Buffer.from(fixture.response_packet.bytes_hex, "hex"));
    })().catch(() => socket.destroy());
  });
  const port = await listen(server);
  const nonceSource = fixedNonces(
    Buffer.alloc(16, 0x01),
    Buffer.from(fixture.request_packet.nonce_hex, "hex"),
  );
  try {
    const reply = await exchangeAuthenticatedFrames({
      commandPayload: Buffer.from(fixture.request_packet.payload_utf8, "utf8"),
      deadlineMs: 1_000,
      helloPayload: encodeClientHello("launch-fixture", "2".repeat(32)),
      nonceSource,
      port,
      rootKey: Buffer.from(fixture.authentication.root_key_hex, "hex"),
    });
    assert.equal(capturedCommand?.toString("hex"), fixture.request_packet.bytes_hex);
    assert.equal(reply.toString("utf8"), fixture.response_packet.payload_utf8);
    assert.deepEqual(
      decodeCommandReply(
        reply,
        parseLatticeArguments("status command project-a target-command-a"),
        {
          commandId: "command-status-a",
          correlationId: "correlation-status-a",
        },
      ),
      { kind: "denied", summary: "LATTICE denied: DOWNSTREAM_DENIED" },
    );
  } finally {
    await close(server);
  }
});

test("rejects authenticated control replies substituted across exact targets", () => {
  const project = parseControlCommand("status project project-a");
  assert.throws(() =>
    decodeCommandReply(
      controlReply(project, {
        kind: "status_observed",
        observation: {
          kind: "project",
          next_cursor: null,
          project_id: "project-b",
          tasks: [],
        },
      }),
      project,
      IDENTITIES,
    ),
  );

  const command = parseControlCommand("status command project-a target-command-a");
  assert.throws(() =>
    decodeCommandReply(
      controlReply(command, {
        kind: "status_observed",
        observation: {
          kind: "command",
          original_command_id: "target-command-b",
          project_id: "project-a",
          terminal_reply_digest: "c".repeat(64),
        },
      }),
      command,
      IDENTITIES,
    ),
  );

  const taskTail = `project-a snapshot-a task-a 1 ${"a".repeat(64)} ${"b".repeat(64)}`;
  const task = parseControlCommand(`status task ${taskTail}`);
  assert.throws(() =>
    decodeCommandReply(
      controlReply(task, {
        kind: "status_observed",
        observation: {
          kind: "task",
          task: taskProjection("project-a", "task-b", "b".repeat(64)),
        },
      }),
      task,
      IDENTITIES,
    ),
  );

  const stop = parseControlCommand(`stop ${taskTail} attempt-a USER_REQUESTED`);
  assert.throws(() =>
    decodeCommandReply(
      controlReply(stop, {
        disposition: "REQUESTED",
        kind: "stop_routed",
        routing_receipt_digest: "c".repeat(64),
        target: {
          attempt_id: "attempt-b",
          reason: "USER_REQUESTED",
          target: taskTarget("project-a", "task-a", "b".repeat(64)),
        },
      }),
      stop,
      IDENTITIES,
    ),
  );
});

test("accepts control replies only when every typed target field matches", () => {
  const taskTail = `project-a snapshot-a task-a 1 ${"a".repeat(64)} ${"b".repeat(64)}`;
  const task = parseControlCommand(`status task ${taskTail}`);
  assert.deepEqual(
    decodeCommandReply(
      controlReply(task, {
        kind: "status_observed",
        observation: {
          kind: "task",
          task: taskProjection("project-a", "task-a", "b".repeat(64)),
        },
      }),
      task,
      IDENTITIES,
    ),
    { kind: "observed", summary: "LATTICE task task-a: EXECUTING" },
  );

  const stop = parseControlCommand(`stop ${taskTail} attempt-a USER_REQUESTED`);
  assert.deepEqual(
    decodeCommandReply(
      controlReply(stop, {
        disposition: "REQUESTED",
        kind: "stop_routed",
        routing_receipt_digest: "c".repeat(64),
        target: {
          attempt_id: "attempt-a",
          reason: "USER_REQUESTED",
          target: taskTarget("project-a", "task-a", "b".repeat(64)),
        },
      }),
      stop,
      IDENTITIES,
    ),
    { kind: "routed", summary: "LATTICE stop: REQUESTED" },
  );
});

function controlReply(
  command: Exclude<LatticeCommand, { readonly action: "submit" }>,
  body: CanonicalValue,
): Buffer {
  return Buffer.from(
    canonicalJson({
      action: command.action,
      body,
      command_id: IDENTITIES.commandId,
      correlation_id: IDENTITIES.correlationId,
      protocol: "lattice-gateway-ipc",
      reply_digest: "c".repeat(64),
      request_digest: testGatewayRequestDigest(command),
      version: "1",
    }),
    "utf8",
  );
}

function parseControlCommand(
  input: string,
): Exclude<LatticeCommand, { readonly action: "submit" }> {
  const command = parseLatticeArguments(input);
  if (command.action === "submit") {
    throw new Error("expected control command");
  }
  return command;
}

function testGatewayRequestDigest(
  command: Exclude<LatticeCommand, { readonly action: "submit" }>,
): string {
  const body = (() => {
    if (command.action === "stop") {
      return {
        attempt_id: command.attemptId,
        reason: command.reason,
        target: taskTarget(
          command.target.projectId,
          command.target.taskId,
          command.target.expectedLedgerHeadDigest,
        ),
      };
    }
    if (command.targetKind === "project") {
      return {
        cursor: null,
        kind: "project",
        page_size: "10",
        project_id: command.projectId,
      };
    }
    if (command.targetKind === "command") {
      return {
        kind: "command",
        original_command_id: command.targetCommandId,
        project_id: command.projectId,
      };
    }
    return {
      kind: "task",
      target: taskTarget(
        command.target.projectId,
        command.target.taskId,
        command.target.expectedLedgerHeadDigest,
      ),
    };
  })();
  const canonical = Buffer.from(
    canonicalJson({
      action: command.action,
      body,
      command_id: IDENTITIES.commandId,
      correlation_id: IDENTITIES.correlationId,
      protocol: "lattice-gateway-ipc",
      version: "1",
    }),
    "utf8",
  );
  const length = Buffer.alloc(8);
  length.writeBigUInt64BE(BigInt(canonical.length));
  return createHash("sha256")
    .update("lattice-hash-1\0", "utf8")
    .update(testLengthPrefixed("sha256"))
    .update(testLengthPrefixed("lattice-cjson-1"))
    .update(testLengthPrefixed("lattice.gateway-request"))
    .update(testLengthPrefixed("1.0"))
    .update(length)
    .update(canonical)
    .digest("hex");
}

function testLengthPrefixed(value: string): Buffer {
  const encoded = Buffer.from(value, "utf8");
  const length = Buffer.alloc(2);
  length.writeUInt16BE(encoded.length);
  return Buffer.concat([length, encoded]);
}

function taskProjection(
  projectId: string,
  taskId: string,
  ledgerHeadDigest: string,
): CanonicalValue {
  return {
    binding: taskBinding(projectId, taskId),
    ledger_head_digest: ledgerHeadDigest,
    observation_receipt_digest: "c".repeat(64),
    state: "EXECUTING",
  };
}

function taskTarget(
  projectId: string,
  taskId: string,
  expectedLedgerHeadDigest: string,
): CanonicalValue {
  return {
    binding: taskBinding(projectId, taskId),
    expected_ledger_head_digest: expectedLedgerHeadDigest,
  };
}

function taskBinding(projectId: string, taskId: string): CanonicalValue {
  return {
    project_id: projectId,
    project_snapshot_id: "snapshot-a",
    task_id: taskId,
    task_revision: "1",
    task_spec_digest: "a".repeat(64),
  };
}

interface GoldenFixture {
  readonly authentication: { readonly root_key_hex: string };
  readonly request_packet: {
    readonly bytes_hex: string;
    readonly magic_ascii: string;
    readonly nonce_hex: string;
    readonly payload_utf8: string;
  };
  readonly response_packet: {
    readonly bytes_hex: string;
    readonly magic_ascii: string;
    readonly payload_utf8: string;
  };
  readonly session_greeting: {
    readonly bytes_hex: string;
    readonly magic_ascii: string;
  };
}

function fixedNonces(...values: readonly Buffer[]): NonceSource {
  let index = 0;
  return () => {
    const value = values[index];
    index += 1;
    assert.ok(value);
    return Buffer.from(value);
  };
}

async function readPacket(socket: Socket): Promise<Buffer> {
  const header = await readExactly(socket, TRANSPORT_WIRE.headerBytes);
  const payload = await readExactly(socket, header.readUInt32BE(40));
  return Buffer.concat([header, payload]);
}

async function readExactly(socket: Socket, length: number): Promise<Buffer> {
  let buffer = Buffer.alloc(0);
  while (buffer.length < length) {
    const chunk = await new Promise<Buffer>((resolve, reject) => {
      const cleanup = (): void => {
        socket.off("data", onData);
        socket.off("error", onError);
        socket.off("end", onEnd);
      };
      const onData = (value: Buffer): void => {
        cleanup();
        resolve(value);
      };
      const onError = (error: Error): void => {
        cleanup();
        reject(error);
      };
      const onEnd = (): void => {
        cleanup();
        reject(new Error("unexpected disconnect"));
      };
      socket.once("data", onData);
      socket.once("error", onError);
      socket.once("end", onEnd);
    });
    buffer = Buffer.concat([buffer, chunk]);
  }
  const output = Buffer.from(buffer.subarray(0, length));
  const remainder = buffer.subarray(length);
  if (remainder.length > 0) {
    socket.unshift(remainder);
  }
  return output;
}

async function listen(server: Server): Promise<number> {
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  assert.ok(address && typeof address === "object");
  return address.port;
}

async function close(server: Server): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}
