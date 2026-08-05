import {
  createHash,
  createHmac,
  randomBytes,
  timingSafeEqual,
} from "node:crypto";
import { Socket } from "node:net";

export const LOOPBACK_HOST = "127.0.0.1" as const;
export const TRANSPORT_WIRE = Object.freeze({
  headerBytes: 76,
  maxFrameBytes: 1_048_576,
  nonceBytes: 16,
  requestMagic: "LATGW001",
  responseMagic: "LATGR001",
  sessionEpochBytes: 16,
  sessionMagic: "LATSN001",
  tagBytes: 32,
});

const REQUEST_MAGIC = Buffer.from(TRANSPORT_WIRE.requestMagic, "ascii");
const RESPONSE_MAGIC = Buffer.from(TRANSPORT_WIRE.responseMagic, "ascii");
const SESSION_MAGIC = Buffer.from(TRANSPORT_WIRE.sessionMagic, "ascii");
const SESSION_GREETING_BYTES = SESSION_MAGIC.length + TRANSPORT_WIRE.sessionEpochBytes;
const ROOT_KEY_BYTES = 32;
const LENGTH_OFFSET = 40;
const TAG_OFFSET = 44;
const MAX_DEADLINE_MS = 30_000;

export type LatticeTransportErrorCode =
  | "AUTHENTICATION"
  | "CONFIGURATION"
  | "DISCONNECTED"
  | "MALFORMED"
  | "TIMEOUT"
  | "UNAVAILABLE";

export class LatticeTransportError extends Error {
  public readonly code: LatticeTransportErrorCode;

  public constructor(code: LatticeTransportErrorCode) {
    super(`LATTICE transport failed: ${code}`);
    this.name = "LatticeTransportError";
    this.code = code;
  }
}

export type NonceSource = () => Buffer;

export interface AuthenticatedExchangeOptions {
  readonly commandPayload: Buffer;
  readonly deadlineMs: number;
  readonly helloPayload: Buffer;
  readonly nonceSource?: NonceSource;
  readonly port: number;
  readonly rootKey: Buffer;
}

/**
 * Performs exactly one loopback connection with one ClientHello frame and one
 * command frame. There is deliberately no retry loop: a disconnect or timeout
 * after Submit remains an ambiguous outcome for higher-level reconciliation.
 */
export async function exchangeAuthenticatedFrames(
  options: AuthenticatedExchangeOptions,
): Promise<Buffer> {
  validateOptions(options);
  const socket = new Socket();
  const reader = new BufferedSocketReader(socket);
  const deadlineError = new LatticeTransportError("TIMEOUT");
  const deadlineTimer = setTimeout(() => socket.destroy(deadlineError), options.deadlineMs);
  const nonceSource = options.nonceSource ?? (() => randomBytes(TRANSPORT_WIRE.nonceBytes));

  try {
    await connectLoopback(socket, options.port);
    socket.setNoDelay(true);

    const greeting = await reader.readExactly(SESSION_GREETING_BYTES);
    if (!greeting.subarray(0, SESSION_MAGIC.length).equals(SESSION_MAGIC)) {
      throw new LatticeTransportError("AUTHENTICATION");
    }
    const epoch = greeting.subarray(SESSION_MAGIC.length);
    if (allZero(epoch)) {
      throw new LatticeTransportError("AUTHENTICATION");
    }
    const sessionKey = deriveSessionKey(options.rootKey, epoch);
    const helloNonce = readNonce(nonceSource);
    const commandNonce = readNonce(nonceSource);
    if (helloNonce.equals(commandNonce)) {
      throw new LatticeTransportError("CONFIGURATION");
    }

    await writeAll(
      socket,
      encodeAuthenticatedPacket(
        REQUEST_MAGIC,
        epoch,
        helloNonce,
        options.helloPayload,
        sessionKey,
      ),
    );
    await writeAll(
      socket,
      encodeAuthenticatedPacket(
        REQUEST_MAGIC,
        epoch,
        commandNonce,
        options.commandPayload,
        sessionKey,
      ),
    );

    const header = await reader.readExactly(TRANSPORT_WIRE.headerBytes);
    validateResponseHeader(header, epoch, commandNonce);
    const payloadLength = header.readUInt32BE(LENGTH_OFFSET);
    if (payloadLength === 0 || payloadLength > TRANSPORT_WIRE.maxFrameBytes) {
      throw new LatticeTransportError("MALFORMED");
    }
    const payload = await reader.readExactly(payloadLength);
    const expectedTag = authenticate(
      sessionKey,
      RESPONSE_MAGIC,
      epoch,
      commandNonce,
      header.subarray(LENGTH_OFFSET, TAG_OFFSET),
      payload,
    );
    const claimedTag = header.subarray(TAG_OFFSET, TRANSPORT_WIRE.headerBytes);
    if (!timingSafeEqual(claimedTag, expectedTag)) {
      throw new LatticeTransportError("AUTHENTICATION");
    }
    return payload;
  } catch (error: unknown) {
    throw normalizeTransportError(error);
  } finally {
    clearTimeout(deadlineTimer);
    reader.dispose();
    socket.destroy();
  }
}

function validateOptions(options: AuthenticatedExchangeOptions): void {
  if (
    !Number.isInteger(options.port) ||
    options.port < 1 ||
    options.port > 65_535 ||
    !Number.isInteger(options.deadlineMs) ||
    options.deadlineMs < 1 ||
    options.deadlineMs > MAX_DEADLINE_MS ||
    options.rootKey.length !== ROOT_KEY_BYTES ||
    allZero(options.rootKey)
  ) {
    throw new LatticeTransportError("CONFIGURATION");
  }
  validatePayload(options.helloPayload);
  validatePayload(options.commandPayload);
}

function validatePayload(payload: Buffer): void {
  if (payload.length === 0 || payload.length > TRANSPORT_WIRE.maxFrameBytes) {
    throw new LatticeTransportError("MALFORMED");
  }
}

function deriveSessionKey(rootKey: Buffer, epoch: Buffer): Buffer {
  return createHash("sha256")
    .update("lattice-openclaw-session-key-v1\0", "utf8")
    .update(rootKey)
    .update(epoch)
    .digest();
}

function readNonce(source: NonceSource): Buffer {
  const nonce = source();
  if (!Buffer.isBuffer(nonce) || nonce.length !== TRANSPORT_WIRE.nonceBytes || allZero(nonce)) {
    throw new LatticeTransportError("CONFIGURATION");
  }
  return Buffer.from(nonce);
}

function encodeAuthenticatedPacket(
  magic: Buffer,
  epoch: Buffer,
  nonce: Buffer,
  payload: Buffer,
  sessionKey: Buffer,
): Buffer {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(payload.length);
  const tag = authenticate(sessionKey, magic, epoch, nonce, length, payload);
  return Buffer.concat([magic, epoch, nonce, length, tag, payload]);
}

function authenticate(
  sessionKey: Buffer,
  magic: Buffer,
  epoch: Buffer,
  nonce: Buffer,
  length: Buffer,
  payload: Buffer,
): Buffer {
  return createHmac("sha256", sessionKey)
    .update(magic)
    .update(epoch)
    .update(nonce)
    .update(length)
    .update(payload)
    .digest();
}

function validateResponseHeader(header: Buffer, epoch: Buffer, nonce: Buffer): void {
  if (!header.subarray(0, RESPONSE_MAGIC.length).equals(RESPONSE_MAGIC)) {
    throw new LatticeTransportError("MALFORMED");
  }
  if (!header.subarray(8, 24).equals(epoch)) {
    throw new LatticeTransportError("AUTHENTICATION");
  }
  if (!header.subarray(24, 40).equals(nonce)) {
    throw new LatticeTransportError("AUTHENTICATION");
  }
}

function connectLoopback(socket: Socket, port: number): Promise<void> {
  return new Promise((resolve, reject) => {
    const onConnect = (): void => {
      cleanup();
      resolve();
    };
    const onError = (error: Error): void => {
      cleanup();
      reject(error);
    };
    const cleanup = (): void => {
      socket.off("connect", onConnect);
      socket.off("error", onError);
    };
    socket.once("connect", onConnect);
    socket.once("error", onError);
    socket.connect({ host: LOOPBACK_HOST, port });
  });
}

function writeAll(socket: Socket, payload: Buffer): Promise<void> {
  return new Promise((resolve, reject) => {
    socket.write(payload, (error) => {
      if (error) {
        reject(error);
      } else {
        resolve();
      }
    });
  });
}

function allZero(value: Buffer): boolean {
  return value.every((byte) => byte === 0);
}

function normalizeTransportError(error: unknown): LatticeTransportError {
  if (error instanceof LatticeTransportError) {
    return error;
  }
  if (error instanceof Error) {
    const code = "code" in error && typeof error.code === "string" ? error.code : "";
    if (code === "ETIMEDOUT") {
      return new LatticeTransportError("TIMEOUT");
    }
    if (code === "ECONNRESET" || code === "EPIPE") {
      return new LatticeTransportError("DISCONNECTED");
    }
  }
  return new LatticeTransportError("UNAVAILABLE");
}

class BufferedSocketReader {
  private buffer = Buffer.alloc(0);
  private terminalError: LatticeTransportError | undefined;
  private pending:
    | {
        readonly length: number;
        readonly reject: (error: LatticeTransportError) => void;
        readonly resolve: (value: Buffer) => void;
      }
    | undefined;

  public constructor(private readonly socket: Socket) {
    socket.on("data", this.onData);
    socket.on("end", this.onEnd);
    socket.on("error", this.onError);
    socket.on("close", this.onClose);
  }

  public readExactly(length: number): Promise<Buffer> {
    if (
      !Number.isInteger(length) ||
      length < 1 ||
      length > TRANSPORT_WIRE.maxFrameBytes
    ) {
      return Promise.reject(new LatticeTransportError("MALFORMED"));
    }
    if (this.buffer.length >= length) {
      return Promise.resolve(this.take(length));
    }
    if (this.terminalError !== undefined) {
      return Promise.reject(this.terminalError);
    }
    if (this.pending !== undefined) {
      return Promise.reject(new LatticeTransportError("CONFIGURATION"));
    }
    return new Promise((resolve, reject) => {
      this.pending = { length, reject, resolve };
    });
  }

  public dispose(): void {
    this.socket.off("data", this.onData);
    this.socket.off("end", this.onEnd);
    this.socket.off("error", this.onError);
    this.socket.off("close", this.onClose);
    this.fail(new LatticeTransportError("DISCONNECTED"));
  }

  private readonly onData = (chunk: Buffer): void => {
    if (this.terminalError !== undefined) {
      return;
    }
    if (this.buffer.length + chunk.length > TRANSPORT_WIRE.maxFrameBytes) {
      const error = new LatticeTransportError("MALFORMED");
      this.fail(error);
      this.socket.destroy(error);
      return;
    }
    this.buffer = Buffer.concat([this.buffer, chunk]);
    this.resolvePending();
  };

  private readonly onEnd = (): void => {
    this.fail(new LatticeTransportError("DISCONNECTED"));
  };

  private readonly onClose = (): void => {
    this.fail(new LatticeTransportError("DISCONNECTED"));
  };

  private readonly onError = (error: Error): void => {
    this.fail(normalizeTransportError(error));
  };

  private resolvePending(): void {
    const pending = this.pending;
    if (pending !== undefined && this.buffer.length >= pending.length) {
      this.pending = undefined;
      pending.resolve(this.take(pending.length));
    }
  }

  private take(length: number): Buffer {
    const value = Buffer.from(this.buffer.subarray(0, length));
    this.buffer = this.buffer.subarray(length);
    return value;
  }

  private fail(error: LatticeTransportError): void {
    if (this.terminalError === undefined) {
      this.terminalError = error;
    }
    const pending = this.pending;
    if (pending !== undefined) {
      this.pending = undefined;
      pending.reject(this.terminalError);
    }
  }
}
