import { randomUUID } from "node:crypto";
import {
  mkdir,
  open,
  readFile,
  rename,
  rm,
} from "node:fs/promises";
import path from "node:path";

import {
  deepFreeze,
  sha256Canonical,
} from "../domain/canonical-json.js";
import { LedgerError, ledgerFailure } from "./errors.js";
import { sanitizeForAudit } from "./sanitize.js";
import { projectTaskPacketFromEvents } from "./projection.js";

const ZERO_HASH = "0".repeat(64);
const TASK_ID_PATTERN = /^TASK-[A-Z0-9][A-Z0-9_-]{2,63}$/;
const HASH_PATTERN = /^[a-f0-9]{64}$/;
const OUTCOMES = new Set([
  "recorded",
  "allow",
  "deny",
  "pass",
  "fail",
  "blocked",
  "cancelled",
]);

function requiredString(value, field) {
  if (typeof value !== "string" || value.trim().length === 0 || value.includes("\0")) {
    ledgerFailure("INVALID_LEDGER_EVENT", `${field} must be a non-empty string.`, {
      field,
    });
  }
  return value.trim();
}

function validateTaskId(taskId) {
  const normalized = requiredString(taskId, "task_id");
  if (!TASK_ID_PATTERN.test(normalized)) {
    ledgerFailure("INVALID_TASK_ID", "task_id is unsafe for a ledger path.");
  }
  return normalized;
}

function normalizeAppendInput(input) {
  if (
    input === null ||
    typeof input !== "object" ||
    Array.isArray(input) ||
    Object.getPrototypeOf(input) !== Object.prototype
  ) {
    ledgerFailure("INVALID_LEDGER_EVENT", "Ledger append input must be a plain object.");
  }
  const taskId = validateTaskId(input.task_id);
  if (!Number.isInteger(input.expected_sequence) || input.expected_sequence < 0) {
    ledgerFailure(
      "INVALID_LEDGER_EVENT",
      "expected_sequence must be a non-negative integer.",
    );
  }
  const subjectHash = requiredString(input.subject_hash, "subject_hash").toLowerCase();
  if (!HASH_PATTERN.test(subjectHash)) {
    ledgerFailure("INVALID_LEDGER_EVENT", "subject_hash must be a SHA-256 hash.");
  }
  const outcome = requiredString(input.outcome, "outcome");
  if (!OUTCOMES.has(outcome)) {
    ledgerFailure("INVALID_LEDGER_EVENT", `Unknown event outcome '${outcome}'.`);
  }
  return {
    task_id: taskId,
    expected_sequence: input.expected_sequence,
    command: {
      event_version: 1,
      task_id: taskId,
      command_id: requiredString(input.command_id, "command_id"),
      correlation_id: requiredString(input.correlation_id, "correlation_id"),
      type: requiredString(input.type, "type"),
      actor_id: requiredString(input.actor_id, "actor_id"),
      role: requiredString(input.role, "role"),
      action: requiredString(input.action, "action"),
      outcome,
      reason_code: requiredString(input.reason_code, "reason_code"),
      subject_hash: subjectHash,
      payload: sanitizeForAudit(input.payload ?? {}),
    },
  };
}

async function readOptional(file) {
  try {
    return await readFile(file, "utf8");
  } catch (error) {
    if (error?.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

async function writeFileDurably(file, content, flag) {
  const handle = await open(file, flag);
  try {
    await handle.writeFile(content, "utf8");
    await handle.sync();
  } finally {
    await handle.close();
  }
}

async function writeHeadAtomically(file, head) {
  const temporary = `${file}.${process.pid}.${randomUUID()}.tmp`;
  try {
    await writeFileDurably(temporary, `${JSON.stringify(head)}\n`, "wx");
    await rename(temporary, file);
  } finally {
    await rm(temporary, { force: true });
  }
}

function verifyEventShape(event, expectedTaskId, expectedSequence, previousHash) {
  if (
    event === null ||
    typeof event !== "object" ||
    Array.isArray(event) ||
    event.event_version !== 1 ||
    event.task_id !== expectedTaskId ||
    event.sequence !== expectedSequence ||
    event.previous_hash !== previousHash ||
    typeof event.hash !== "string"
  ) {
    ledgerFailure("LEDGER_EVENT_INVALID", "Ledger event shape or sequence is invalid.", {
      task_id: expectedTaskId,
      expected_sequence: expectedSequence,
    });
  }
  const { hash, ...unsigned } = event;
  const actualHash = sha256Canonical(unsigned);
  if (hash !== actualHash) {
    ledgerFailure("LEDGER_HASH_MISMATCH", "Ledger event hash does not match content.", {
      task_id: expectedTaskId,
      sequence: expectedSequence,
      expected_hash: hash,
      actual_hash: actualHash,
    });
  }
}

export class TaskLedger {
  #root;
  #clock;
  #idFactory;
  #queues = new Map();

  constructor({
    root,
    clock = () => new Date(),
    idFactory = () => randomUUID(),
  }) {
    if (typeof root !== "string" || root.trim().length === 0) {
      ledgerFailure("INVALID_LEDGER_ROOT", "Task Ledger root is required.");
    }
    if (typeof clock !== "function" || typeof idFactory !== "function") {
      ledgerFailure("INVALID_LEDGER_DEPENDENCY", "Clock and ID factory must be functions.");
    }
    this.#root = path.resolve(root);
    this.#clock = clock;
    this.#idFactory = idFactory;
  }

  taskLogPath(taskId) {
    return path.join(this.#root, `${validateTaskId(taskId)}.jsonl`);
  }

  taskHeadPath(taskId) {
    return path.join(this.#root, `${validateTaskId(taskId)}.head.json`);
  }

  async #readVerified(taskId) {
    const normalizedTaskId = validateTaskId(taskId);
    const logFile = this.taskLogPath(normalizedTaskId);
    const headFile = this.taskHeadPath(normalizedTaskId);
    const [rawLog, rawHead] = await Promise.all([
      readOptional(logFile),
      readOptional(headFile),
    ]);

    if (rawLog === null && rawHead === null) {
      return [];
    }
    if (rawLog === null || rawHead === null) {
      ledgerFailure(
        "LEDGER_HEAD_MISMATCH",
        "Ledger log and integrity head must both exist.",
        { task_id: normalizedTaskId },
      );
    }
    if (!rawLog.endsWith("\n")) {
      ledgerFailure("LEDGER_TRUNCATED", "Ledger log is not newline-terminated.", {
        task_id: normalizedTaskId,
      });
    }
    let head;
    try {
      head = JSON.parse(rawHead);
    } catch {
      ledgerFailure("LEDGER_HEAD_INVALID", "Ledger integrity head is invalid JSON.", {
        task_id: normalizedTaskId,
      });
    }
    const lines = rawLog.slice(0, -1).split("\n");
    if (lines.some((line) => line.length === 0)) {
      ledgerFailure("LEDGER_TRUNCATED", "Ledger contains an empty event line.", {
        task_id: normalizedTaskId,
      });
    }
    const events = [];
    let previousHash = ZERO_HASH;
    for (let index = 0; index < lines.length; index += 1) {
      let event;
      try {
        event = JSON.parse(lines[index]);
      } catch {
        ledgerFailure("LEDGER_EVENT_INVALID", "Ledger event is invalid JSON.", {
          task_id: normalizedTaskId,
          sequence: index + 1,
        });
      }
      verifyEventShape(event, normalizedTaskId, index + 1, previousHash);
      events.push(deepFreeze(event));
      previousHash = event.hash;
    }
    if (
      head?.version !== 1 ||
      head.task_id !== normalizedTaskId ||
      head.sequence !== events.length ||
      head.hash !== previousHash
    ) {
      ledgerFailure("LEDGER_HEAD_MISMATCH", "Ledger integrity head does not match the log.", {
        task_id: normalizedTaskId,
        log_sequence: events.length,
        head_sequence: head?.sequence,
      });
    }
    return events;
  }

  async verify(taskId) {
    return deepFreeze([...(await this.#readVerified(taskId))]);
  }

  async readTaskPacket(taskId) {
    return projectTaskPacketFromEvents(await this.#readVerified(taskId));
  }

  async #appendNow(input) {
    const normalized = normalizeAppendInput(input);
    await mkdir(this.#root, { recursive: true });
    const events = await this.#readVerified(normalized.task_id);
    const fingerprint = sha256Canonical(normalized.command);
    const duplicate = events.find(
      (event) => event.command_id === normalized.command.command_id,
    );
    if (duplicate) {
      if (duplicate.command_fingerprint !== fingerprint) {
        ledgerFailure(
          "COMMAND_ID_REUSE",
          "command_id was already used for different content.",
          { command_id: normalized.command.command_id },
        );
      }
      return deepFreeze({ event: duplicate, idempotent: true });
    }
    if (events.length !== normalized.expected_sequence) {
      ledgerFailure(
        "LEDGER_SEQUENCE_CONFLICT",
        "expected_sequence does not match the verified ledger head.",
        {
          expected_sequence: normalized.expected_sequence,
          actual_sequence: events.length,
        },
      );
    }

    const timestampValue = this.#clock();
    const timestamp =
      timestampValue instanceof Date
        ? timestampValue.toISOString()
        : new Date(timestampValue).toISOString();
    const previousHash = events.at(-1)?.hash ?? ZERO_HASH;
    const unsigned = {
      ...normalized.command,
      event_id: requiredString(this.#idFactory(), "event_id"),
      sequence: events.length + 1,
      timestamp,
      command_fingerprint: fingerprint,
      previous_hash: previousHash,
    };
    const event = deepFreeze({
      ...unsigned,
      hash: sha256Canonical(unsigned),
    });

    await writeFileDurably(
      this.taskLogPath(normalized.task_id),
      `${JSON.stringify(event)}\n`,
      "a",
    );
    await writeHeadAtomically(this.taskHeadPath(normalized.task_id), {
      version: 1,
      task_id: normalized.task_id,
      sequence: event.sequence,
      hash: event.hash,
    });
    return deepFreeze({ event, idempotent: false });
  }

  async append(input) {
    const taskId = validateTaskId(input?.task_id);
    const previous = this.#queues.get(taskId) ?? Promise.resolve();
    const operation = previous.catch(() => undefined).then(() => this.#appendNow(input));
    this.#queues.set(taskId, operation);
    try {
      return await operation;
    } finally {
      if (this.#queues.get(taskId) === operation) {
        this.#queues.delete(taskId);
      }
    }
  }
}

export { LedgerError, ZERO_HASH };
