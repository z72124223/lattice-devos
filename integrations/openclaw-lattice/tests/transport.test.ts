import assert from "node:assert/strict";
import {
  createHash,
  createHmac,
  timingSafeEqual,
} from "node:crypto";
import {
  createServer,
  type Server,
  type Socket,
} from "node:net";
import { performance } from "node:perf_hooks";
import { test } from "node:test";

import {
  exchangeAuthenticatedFrames,
  type NonceSource,
} from "../src/transport.js";

const REQUEST_MAGIC = Buffer.from("LATGW001", "ascii");
const RESPONSE_MAGIC = Buffer.from("LATGR001", "ascii");
const SESSION_MAGIC = Buffer.from("LATSN001", "ascii");
const ROOT_KEY = Buffer.alloc(32, 0x31);
const EPOCH = Buffer.from("00112233445566778899aabbccddeeff", "hex");
const HEADER_BYTES = 76;

test("uses a buffered two-frame authenticated loopback exchange", async () => {
  const hello = Buffer.from('{"action":"client_hello"}', "utf8");
  const command = Buffer.from(
    `{"action":"submit","body":{"task_spec_digest":"${"a".repeat(64)}"}}`,
    "utf8",
  );
  const response = Buffer.from('{"outcome":"accepted"}', "utf8");
  const observed: Buffer[] = [];
  const nonceSource = fixedNonces(
    Buffer.alloc(16, 0x41),
    Buffer.alloc(16, 0x42),
  );
  const fixture = await listen(async (socket) => {
    assert.equal(socket.remoteAddress, "127.0.0.1");
    const greeting = Buffer.concat([SESSION_MAGIC, EPOCH]);
    socket.write(greeting.subarray(0, 3));
    socket.write(greeting.subarray(3));

    const sessionKey = deriveSessionKey(ROOT_KEY, EPOCH);
    observed.push(await readAuthenticatedFrame(socket, REQUEST_MAGIC, sessionKey));
    observed.push(await readAuthenticatedFrame(socket, REQUEST_MAGIC, sessionKey));
    const packet = encodeAuthenticatedFrame(
      RESPONSE_MAGIC,
      EPOCH,
      Buffer.alloc(16, 0x42),
      response,
      sessionKey,
    );
    socket.write(packet.subarray(0, HEADER_BYTES + 2));
    socket.write(packet.subarray(HEADER_BYTES + 2));
    socket.end();
  });

  try {
    const actual = await exchangeAuthenticatedFrames({
      commandPayload: command,
      deadlineMs: 1_000,
      helloPayload: hello,
      nonceSource,
      port: fixture.port,
      rootKey: ROOT_KEY,
    });
    assert.deepEqual(actual, response);
    assert.deepEqual(observed, [hello, command]);
  } finally {
    await fixture.close();
  }
});

test("one absolute deadline defeats a slow-drip greeting", async () => {
  let interval: NodeJS.Timeout | undefined;
  const fixture = await listen((socket) => {
    const greeting = Buffer.concat([SESSION_MAGIC, EPOCH]);
    let offset = 0;
    interval = setInterval(() => {
      if (offset < greeting.length && !socket.destroyed) {
        socket.write(greeting.subarray(offset, offset + 1));
        offset += 1;
      }
    }, 15);
  });
  const started = performance.now();
  try {
    await assert.rejects(
      exchangeAuthenticatedFrames({
        commandPayload: Buffer.from('{"action":"status"}', "utf8"),
        deadlineMs: 60,
        helloPayload: Buffer.from('{"action":"client_hello"}', "utf8"),
        nonceSource: fixedNonces(Buffer.alloc(16, 1), Buffer.alloc(16, 2)),
        port: fixture.port,
        rootKey: ROOT_KEY,
      }),
      { code: "TIMEOUT", name: "LatticeTransportError" },
    );
    assert.ok(performance.now() - started < 250);
  } finally {
    if (interval !== undefined) {
      clearInterval(interval);
    }
    await fixture.close();
  }
});

test("disconnect after submit is surfaced without an automatic retry", async () => {
  let connections = 0;
  let frames = 0;
  const fixture = await listen(async (socket) => {
    connections += 1;
    socket.write(Buffer.concat([SESSION_MAGIC, EPOCH]));
    const sessionKey = deriveSessionKey(ROOT_KEY, EPOCH);
    await readAuthenticatedFrame(socket, REQUEST_MAGIC, sessionKey);
    frames += 1;
    await readAuthenticatedFrame(socket, REQUEST_MAGIC, sessionKey);
    frames += 1;
    socket.destroy();
  });

  try {
    await assert.rejects(
      exchangeAuthenticatedFrames({
        commandPayload: Buffer.from('{"action":"submit"}', "utf8"),
        deadlineMs: 500,
        helloPayload: Buffer.from('{"action":"client_hello"}', "utf8"),
        nonceSource: fixedNonces(Buffer.alloc(16, 3), Buffer.alloc(16, 4)),
        port: fixture.port,
        rootKey: ROOT_KEY,
      }),
      { code: "DISCONNECTED", name: "LatticeTransportError" },
    );
    assert.equal(connections, 1);
    assert.equal(frames, 2);
  } finally {
    await fixture.close();
  }
});

function fixedNonces(...nonces: readonly Buffer[]): NonceSource {
  let index = 0;
  return () => {
    const nonce = nonces[index];
    index += 1;
    assert.ok(nonce);
    return Buffer.from(nonce);
  };
}

function deriveSessionKey(rootKey: Buffer, epoch: Buffer): Buffer {
  return createHash("sha256")
    .update("lattice-openclaw-session-key-v1\0", "utf8")
    .update(rootKey)
    .update(epoch)
    .digest();
}

async function readAuthenticatedFrame(
  socket: Socket,
  magic: Buffer,
  sessionKey: Buffer,
): Promise<Buffer> {
  const header = await readExactly(socket, HEADER_BYTES);
  assert.deepEqual(header.subarray(0, 8), magic);
  assert.deepEqual(header.subarray(8, 24), EPOCH);
  const length = header.readUInt32BE(40);
  const payload = await readExactly(socket, length);
  const expected = createHmac("sha256", sessionKey)
    .update(header.subarray(0, 44))
    .update(payload)
    .digest();
  assert.ok(timingSafeEqual(header.subarray(44, 76), expected));
  return payload;
}

function encodeAuthenticatedFrame(
  magic: Buffer,
  epoch: Buffer,
  nonce: Buffer,
  payload: Buffer,
  sessionKey: Buffer,
): Buffer {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(payload.length);
  const tag = createHmac("sha256", sessionKey)
    .update(magic)
    .update(epoch)
    .update(nonce)
    .update(length)
    .update(payload)
    .digest();
  return Buffer.concat([magic, epoch, nonce, length, tag, payload]);
}

async function readExactly(socket: Socket, length: number): Promise<Buffer> {
  let buffered = Buffer.alloc(0);
  while (buffered.length < length) {
    const chunk = await new Promise<Buffer>((resolve, reject) => {
      socket.once("data", resolve);
      socket.once("error", reject);
      socket.once("end", () => reject(new Error("unexpected disconnect")));
    });
    buffered = Buffer.concat([buffered, chunk]);
  }
  const output = buffered.subarray(0, length);
  const remainder = buffered.subarray(length);
  if (remainder.length > 0) {
    socket.unshift(remainder);
  }
  return output;
}

async function listen(
  handler: (socket: Socket) => void | Promise<void>,
): Promise<{ readonly port: number; readonly close: () => Promise<void> }> {
  const sockets = new Set<Socket>();
  const server = createServer((socket) => {
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
    void Promise.resolve(handler(socket)).catch(() => socket.destroy());
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  assert.ok(address && typeof address === "object");
  return {
    port: address.port,
    close: async () => {
      for (const socket of sockets) {
        socket.destroy();
      }
      await closeServer(server);
    },
  };
}

async function closeServer(server: Server): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    server.close((error) => {
      if (error) {
        reject(error);
      } else {
        resolve();
      }
    });
  });
}
