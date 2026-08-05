import { createHash } from "node:crypto";

import { canonicalJson, type CanonicalValue } from "./cjson.js";

const MAX_ARGUMENT_BYTES = 1_024;
const MAX_IDENTIFIER_BYTES = 256;
const IDENTIFIER = /^[A-Za-z0-9][A-Za-z0-9._:@-]*$/u;
const DIGEST = /^[0-9a-f]{64}$/u;
const COMMAND_ID_DOMAIN = "lattice-openclaw-command-id-v1";
const CORRELATION_ID_DOMAIN = "lattice-openclaw-correlation-id-v1";

export type StatusTargetKind = "project" | "command" | "task";
export type StopReason = "USER_REQUESTED" | "SUPERSEDED" | "SAFETY_CONCERN";

export interface TaskTargetArguments {
  readonly expectedLedgerHeadDigest: string;
  readonly projectId: string;
  readonly projectSnapshotId: string;
  readonly taskId: string;
  readonly taskRevision: string;
  readonly taskSpecDigest: string;
}

export type LatticeCommand =
  | Readonly<{
      action: "status";
      targetKind: "project";
      projectId: string;
    }>
  | Readonly<{
      action: "status";
      targetKind: "command";
      projectId: string;
      targetCommandId: string;
    }>
  | Readonly<{
      action: "status";
      targetKind: "task";
      target: TaskTargetArguments;
    }>
  | Readonly<{
      action: "stop";
      attemptId: string;
      reason: StopReason;
      target: TaskTargetArguments;
    }>
  | Readonly<{
      action: "submit";
      taskSpecDigest: string;
    }>;

export interface CommandIdentities {
  readonly commandId: string;
  readonly correlationId: string;
}

export class LatticeInputError extends Error {
  public constructor() {
    super("invalid closed LATTICE command");
    this.name = "LatticeInputError";
  }
}

export function parseLatticeArguments(input: string): LatticeCommand {
  if (
    Buffer.byteLength(input, "utf8") > MAX_ARGUMENT_BYTES ||
    containsAsciiControl(input)
  ) {
    throw new LatticeInputError();
  }
  const normalized = input.trim();
  if (normalized.length === 0) {
    throw new LatticeInputError();
  }
  const tokens = normalized.split(/[ \t]+/u);
  const action = tokens[0];
  if (action === "status" && tokens[1] === "project" && tokens.length === 3) {
    const projectId = tokens[2];
    if (projectId !== undefined && validIdentifier(projectId)) {
      return { action, projectId, targetKind: "project" };
    }
  } else if (action === "status" && tokens[1] === "command" && tokens.length === 4) {
    const projectId = tokens[2];
    const targetCommandId = tokens[3];
    if (
      projectId !== undefined &&
      targetCommandId !== undefined &&
      validIdentifier(projectId) &&
      validIdentifier(targetCommandId)
    ) {
      return { action, projectId, targetCommandId, targetKind: "command" };
    }
  } else if (action === "status" && tokens[1] === "task" && tokens.length === 8) {
    const target = parseTaskTarget(tokens, 2);
    if (target !== undefined) {
      return { action, target, targetKind: "task" };
    }
  } else if (action === "stop" && tokens.length === 9) {
    const target = parseTaskTarget(tokens, 1);
    const attemptId = tokens[7];
    const reason = tokens[8];
    if (
      target !== undefined &&
      attemptId !== undefined &&
      validIdentifier(attemptId) &&
      isStopReason(reason)
    ) {
      return { action, attemptId, reason, target };
    }
  } else if (action === "submit" && tokens.length === 2) {
    const taskSpecDigest = tokens[1];
    if (taskSpecDigest !== undefined && validDigest(taskSpecDigest)) {
      return { action, taskSpecDigest };
    }
  }
  throw new LatticeInputError();
}

function parseTaskTarget(
  tokens: readonly string[],
  offset: number,
): TaskTargetArguments | undefined {
  const projectId = tokens[offset];
  const projectSnapshotId = tokens[offset + 1];
  const taskId = tokens[offset + 2];
  const taskRevision = tokens[offset + 3];
  const taskSpecDigest = tokens[offset + 4];
  const expectedLedgerHeadDigest = tokens[offset + 5];
  if (
    projectId === undefined ||
    projectSnapshotId === undefined ||
    taskId === undefined ||
    taskRevision === undefined ||
    taskSpecDigest === undefined ||
    expectedLedgerHeadDigest === undefined ||
    !validIdentifier(projectId) ||
    !validIdentifier(projectSnapshotId) ||
    !validIdentifier(taskId) ||
    !validRevision(taskRevision) ||
    !validDigest(taskSpecDigest) ||
    !validDigest(expectedLedgerHeadDigest)
  ) {
    return undefined;
  }
  return {
    expectedLedgerHeadDigest,
    projectId,
    projectSnapshotId,
    taskId,
    taskRevision,
    taskSpecDigest,
  };
}

function isStopReason(value: string | undefined): value is StopReason {
  return value === "USER_REQUESTED" || value === "SUPERSEDED" || value === "SAFETY_CONCERN";
}

function validRevision(value: string): boolean {
  return /^[1-9][0-9]*$/u.test(value) && BigInt(value) <= 18_446_744_073_709_551_615n;
}

function validDigest(value: string): boolean {
  return DIGEST.test(value) && !/^0+$/u.test(value);
}

export function deriveCommandIdentities(
  sessionKey: string,
  command: LatticeCommand,
): CommandIdentities {
  const normalizedSessionKey = sessionKey.normalize("NFC");
  if (
    normalizedSessionKey.length === 0 ||
    Buffer.byteLength(normalizedSessionKey, "utf8") > MAX_ARGUMENT_BYTES ||
    containsAsciiControl(normalizedSessionKey)
  ) {
    throw new LatticeInputError();
  }
  const canonical = Buffer.from(canonicalJson(canonicalArguments(command)), "utf8");
  return {
    commandId: deriveIdentity(COMMAND_ID_DOMAIN, normalizedSessionKey, canonical),
    correlationId: deriveIdentity(CORRELATION_ID_DOMAIN, normalizedSessionKey, canonical),
  };
}

export function canonicalArguments(command: LatticeCommand): CanonicalValue {
  switch (command.action) {
    case "status":
      switch (command.targetKind) {
        case "project":
          return {
            action: command.action,
            project_id: command.projectId,
            target_kind: command.targetKind,
          };
        case "command":
          return {
            action: command.action,
            project_id: command.projectId,
            target_command_id: command.targetCommandId,
            target_kind: command.targetKind,
          };
        case "task":
          return {
            action: command.action,
            target: canonicalTaskTarget(command.target),
            target_kind: command.targetKind,
          };
      }
      throw new LatticeInputError();
    case "stop":
      return {
        action: command.action,
        attempt_id: command.attemptId,
        reason: command.reason,
        target: canonicalTaskTarget(command.target),
      };
    case "submit":
      return {
        action: command.action,
        task_spec_digest: command.taskSpecDigest,
      };
  }
}

function containsAsciiControl(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 0x1f || code === 0x7f) {
      return true;
    }
  }
  return false;
}

function canonicalTaskTarget(target: TaskTargetArguments): CanonicalValue {
  return {
    expected_ledger_head_digest: target.expectedLedgerHeadDigest,
    project_id: target.projectId,
    project_snapshot_id: target.projectSnapshotId,
    task_id: target.taskId,
    task_revision: target.taskRevision,
    task_spec_digest: target.taskSpecDigest,
  };
}

function validIdentifier(value: string): boolean {
  const bytes = Buffer.byteLength(value, "utf8");
  return (
    bytes > 0 &&
    bytes <= MAX_IDENTIFIER_BYTES &&
    value !== "." &&
    value !== ".." &&
    IDENTIFIER.test(value)
  );
}

function deriveIdentity(domain: string, sessionKey: string, canonical: Buffer): string {
  return createHash("sha256")
    .update(domain, "utf8")
    .update(Buffer.from([0]))
    .update(sessionKey, "utf8")
    .update(Buffer.from([0]))
    .update(canonical)
    .digest("hex");
}
