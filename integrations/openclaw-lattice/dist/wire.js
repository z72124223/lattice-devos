import { createHash } from "node:crypto";
import { canonicalJson } from "./cjson.js";
export const INBOUND_PROTOCOL = "lattice-openclaw-inbound";
export const CLIENT_HELLO_PROTOCOL = "lattice-openclaw-client-hello";
export const WIRE_VERSION = "1";
const SAFE_IDENTIFIER = /^[A-Za-z0-9][A-Za-z0-9._:@-]{0,255}$/u;
const HEX_32 = /^[0-9a-f]{32}$/u;
const DIGEST = /^[0-9a-f]{64}$/u;
const HASH_FRAME_ID = "lattice-hash-1";
const HASH_DIGEST_ID = "sha256";
const HASH_ALGORITHM_ID = "lattice-cjson-1";
const GATEWAY_REQUEST_SCHEMA = "lattice.gateway-request";
const HASH_SCHEMA_VERSION = "1.0";
export class LatticeWireError extends Error {
    constructor() {
        super("invalid closed LATTICE wire value");
        this.name = "LatticeWireError";
    }
}
export function encodeClientHello(launchRecordId, processStartNonce) {
    if (!SAFE_IDENTIFIER.test(launchRecordId) ||
        !HEX_32.test(processStartNonce) ||
        /^0+$/u.test(processStartNonce)) {
        throw new LatticeWireError();
    }
    return encode({
        launch_record_id: launchRecordId,
        process_start_nonce: processStartNonce,
        protocol: CLIENT_HELLO_PROTOCOL,
        version: WIRE_VERSION,
    });
}
export function encodeCommandRequest(command, identities) {
    validateIdentity(identities.commandId);
    validateIdentity(identities.correlationId);
    return encode({
        action: command.action,
        body: requestBody(command),
        command_id: identities.commandId,
        correlation_id: identities.correlationId,
        protocol: INBOUND_PROTOCOL,
        version: WIRE_VERSION,
    });
}
export function decodeCommandReply(payload, command, identities) {
    let parsed;
    try {
        const source = payload.toString("utf8");
        parsed = JSON.parse(source);
        if (canonicalJson(parsed) !== source) {
            throw new LatticeWireError();
        }
    }
    catch {
        throw new LatticeWireError();
    }
    const frame = exactRecord(parsed, command.action === "submit"
        ? [
            "action",
            "body",
            "command_id",
            "correlation_id",
            "gateway_reply_digest",
            "protocol",
            "version",
        ]
        : [
            "action",
            "body",
            "command_id",
            "correlation_id",
            "protocol",
            "reply_digest",
            "request_digest",
            "version",
        ]);
    if (frame.action !== command.action ||
        frame.command_id !== identities.commandId ||
        frame.correlation_id !== identities.correlationId ||
        frame.version !== WIRE_VERSION) {
        throw new LatticeWireError();
    }
    if (command.action === "submit") {
        if (frame.protocol !== "lattice-openclaw-inbound-reply" ||
            !validDigest(frame.gateway_reply_digest)) {
            throw new LatticeWireError();
        }
        return decodeSubmitBody(frame.body, command.taskSpecDigest);
    }
    const requestDigest = frame.request_digest;
    if (frame.protocol !== "lattice-gateway-ipc" ||
        !validDigest(frame.reply_digest) ||
        !validDigest(requestDigest) ||
        requestDigest !== gatewayControlRequestDigest(command, identities)) {
        throw new LatticeWireError();
    }
    return decodeControlBody(frame.body, command);
}
function gatewayControlRequestDigest(command, identities) {
    const canonical = Buffer.from(canonicalJson({
        action: command.action,
        body: gatewayControlRequestBody(command),
        command_id: identities.commandId,
        correlation_id: identities.correlationId,
        protocol: "lattice-gateway-ipc",
        version: WIRE_VERSION,
    }), "utf8");
    const payloadLength = Buffer.alloc(8);
    payloadLength.writeBigUInt64BE(BigInt(canonical.length));
    return createHash("sha256")
        .update(HASH_FRAME_ID, "utf8")
        .update(Buffer.from([0]))
        .update(lengthPrefixed(HASH_DIGEST_ID))
        .update(lengthPrefixed(HASH_ALGORITHM_ID))
        .update(lengthPrefixed(GATEWAY_REQUEST_SCHEMA))
        .update(lengthPrefixed(HASH_SCHEMA_VERSION))
        .update(payloadLength)
        .update(canonical)
        .digest("hex");
}
function gatewayControlRequestBody(command) {
    if (command.action === "stop") {
        return {
            attempt_id: command.attemptId,
            reason: command.reason,
            target: taskTarget(command.target),
        };
    }
    switch (command.targetKind) {
        case "project":
            return {
                cursor: null,
                kind: "project",
                page_size: "10",
                project_id: command.projectId,
            };
        case "command":
            return {
                kind: "command",
                original_command_id: command.targetCommandId,
                project_id: command.projectId,
            };
        case "task":
            return { kind: "task", target: taskTarget(command.target) };
    }
}
function lengthPrefixed(value) {
    const encoded = Buffer.from(value, "utf8");
    if (encoded.length > 65_535) {
        throw new LatticeWireError();
    }
    const length = Buffer.alloc(2);
    length.writeUInt16BE(encoded.length);
    return Buffer.concat([length, encoded]);
}
function requestBody(command) {
    switch (command.action) {
        case "submit":
            return { task_spec_digest: command.taskSpecDigest };
        case "status":
            switch (command.targetKind) {
                case "project":
                    return {
                        cursor: null,
                        kind: "project",
                        page_size: "10",
                        project_id: command.projectId,
                    };
                case "command":
                    return {
                        kind: "command",
                        project_id: command.projectId,
                        target_command_id: command.targetCommandId,
                    };
                case "task":
                    return { kind: "task", target: taskTarget(command.target) };
            }
            throw new LatticeWireError();
        case "stop":
            return {
                attempt_id: command.attemptId,
                reason: command.reason,
                target: taskTarget(command.target),
            };
    }
}
function taskTarget(target) {
    return {
        binding: {
            project_id: target.projectId,
            project_snapshot_id: target.projectSnapshotId,
            task_id: target.taskId,
            task_revision: target.taskRevision,
            task_spec_digest: target.taskSpecDigest,
        },
        expected_ledger_head_digest: target.expectedLedgerHeadDigest,
    };
}
function decodeSubmitBody(value, taskSpecDigest) {
    const body = record(value);
    if (body.outcome === "accepted") {
        exactKeys(body, ["binding", "command_receipt_digest", "outcome"]);
        const binding = validateBinding(body.binding);
        if (binding.task_spec_digest !== taskSpecDigest || !validDigest(body.command_receipt_digest)) {
            throw new LatticeWireError();
        }
        return { kind: "accepted", summary: `LATTICE submit accepted for ${binding.task_id}` };
    }
    if (body.outcome === "denied") {
        exactKeys(body, ["code", "outcome"]);
        return { kind: "denied", summary: `LATTICE denied: ${denialCode(body.code)}` };
    }
    if (body.outcome === "unknown_outcome") {
        exactKeys(body, ["code", "outcome"]);
        return { kind: "unknown", summary: `LATTICE outcome unknown: ${unknownCode(body.code)}` };
    }
    throw new LatticeWireError();
}
function decodeControlBody(value, command) {
    const body = record(value);
    if (body.kind === "denied") {
        exactKeys(body, ["code", "kind"]);
        return { kind: "denied", summary: `LATTICE denied: ${denialCode(body.code)}` };
    }
    if (body.kind === "unknown_outcome") {
        exactKeys(body, ["code", "kind"]);
        return { kind: "unknown", summary: `LATTICE outcome unknown: ${unknownCode(body.code)}` };
    }
    if (command.action === "status" && body.kind === "status_observed") {
        exactKeys(body, ["kind", "observation"]);
        return decodeObservation(body.observation, command);
    }
    if (command.action === "stop" && body.kind === "stop_routed") {
        exactKeys(body, ["disposition", "kind", "routing_receipt_digest", "target"]);
        if (body.disposition !== "REQUESTED" &&
            body.disposition !== "ALREADY_TERMINAL" &&
            body.disposition !== "RECONCILIATION_REQUIRED") {
            throw new LatticeWireError();
        }
        if (!validDigest(body.routing_receipt_digest)) {
            throw new LatticeWireError();
        }
        validateStopTarget(body.target, command);
        return { kind: "routed", summary: `LATTICE stop: ${body.disposition}` };
    }
    throw new LatticeWireError();
}
function decodeObservation(value, command) {
    const observation = record(value);
    if (observation.kind !== command.targetKind) {
        throw new LatticeWireError();
    }
    if (command.targetKind === "project") {
        exactKeys(observation, ["kind", "next_cursor", "project_id", "tasks"]);
        if (!Array.isArray(observation.tasks) || observation.tasks.length > 100) {
            throw new LatticeWireError();
        }
        for (const task of observation.tasks) {
            const projection = validateTaskProjection(task);
            if (projection.binding.project_id !== command.projectId) {
                throw new LatticeWireError();
            }
        }
        if ((observation.next_cursor !== null && typeof observation.next_cursor !== "string") ||
            observation.project_id !== command.projectId) {
            throw new LatticeWireError();
        }
        return {
            kind: "observed",
            summary: `LATTICE project ${observation.project_id}: ${observation.tasks.length.toString()} task(s)`,
        };
    }
    if (command.targetKind === "task") {
        exactKeys(observation, ["kind", "task"]);
        const task = validateTaskProjection(observation.task);
        requireExactTaskTarget(task.binding, task.ledgerHeadDigest, command.target);
        return { kind: "observed", summary: `LATTICE task ${task.taskId}: ${task.state}` };
    }
    exactKeys(observation, ["kind", "original_command_id", "project_id", "terminal_reply_digest"]);
    if (observation.original_command_id !== command.targetCommandId ||
        observation.project_id !== command.projectId ||
        !validDigest(observation.terminal_reply_digest)) {
        throw new LatticeWireError();
    }
    return {
        kind: "observed",
        summary: `LATTICE command ${observation.original_command_id}: terminal`,
    };
}
function validateTaskProjection(value) {
    const projection = exactRecord(value, [
        "binding",
        "ledger_head_digest",
        "observation_receipt_digest",
        "state",
    ]);
    const binding = validateBinding(projection.binding);
    if (!validDigest(projection.ledger_head_digest) ||
        !validDigest(projection.observation_receipt_digest) ||
        typeof projection.state !== "string" ||
        !TASK_STATES.has(projection.state)) {
        throw new LatticeWireError();
    }
    return {
        binding,
        ledgerHeadDigest: projection.ledger_head_digest,
        state: projection.state,
        taskId: binding.task_id,
    };
}
function validateStopTarget(value, command) {
    const target = exactRecord(value, ["attempt_id", "reason", "target"]);
    if (target.attempt_id !== command.attemptId ||
        target.reason !== command.reason) {
        throw new LatticeWireError();
    }
    const task = exactRecord(target.target, ["binding", "expected_ledger_head_digest"]);
    const binding = validateBinding(task.binding);
    requireExactTaskTarget(binding, task.expected_ledger_head_digest, command.target);
}
function validateBinding(value) {
    const binding = exactRecord(value, [
        "project_id",
        "project_snapshot_id",
        "task_id",
        "task_revision",
        "task_spec_digest",
    ]);
    for (const key of ["project_id", "project_snapshot_id", "task_id", "task_revision"]) {
        if (typeof binding[key] !== "string") {
            throw new LatticeWireError();
        }
    }
    if (!validDigest(binding.task_spec_digest)) {
        throw new LatticeWireError();
    }
    return binding;
}
function requireExactTaskTarget(binding, ledgerHeadDigest, target) {
    if (binding.project_id !== target.projectId ||
        binding.project_snapshot_id !== target.projectSnapshotId ||
        binding.task_id !== target.taskId ||
        binding.task_revision !== target.taskRevision ||
        binding.task_spec_digest !== target.taskSpecDigest ||
        ledgerHeadDigest !== target.expectedLedgerHeadDigest ||
        !validDigest(ledgerHeadDigest)) {
        throw new LatticeWireError();
    }
}
function denialCode(value) {
    if (typeof value !== "string" || !DENIAL_CODES.has(value)) {
        throw new LatticeWireError();
    }
    return value;
}
function unknownCode(value) {
    if (typeof value !== "string" || !UNKNOWN_CODES.has(value)) {
        throw new LatticeWireError();
    }
    return value;
}
function validateIdentity(value) {
    if (!SAFE_IDENTIFIER.test(value)) {
        throw new LatticeWireError();
    }
}
function validDigest(value) {
    return typeof value === "string" && DIGEST.test(value) && !/^0+$/u.test(value);
}
function encode(value) {
    return Buffer.from(canonicalJson(value), "utf8");
}
function exactRecord(value, keys) {
    const output = record(value);
    exactKeys(output, keys);
    return output;
}
function record(value) {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
        throw new LatticeWireError();
    }
    return value;
}
function exactKeys(value, keys) {
    const actual = Object.keys(value).sort();
    const expected = [...keys].sort();
    if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
        throw new LatticeWireError();
    }
}
const DENIAL_CODES = new Set([
    "SCOPE_DENIED",
    "SESSION_NOT_CURRENT",
    "ROLE_DENIED",
    "PROTECTED_SURFACE_REQUIRED",
    "COMMAND_SUBSTITUTION",
    "MALFORMED_SUBJECT",
    "DOWNSTREAM_DENIED",
]);
const UNKNOWN_CODES = new Set(["DOWNSTREAM_AMBIGUOUS", "RECONCILIATION_REQUIRED"]);
const TASK_STATES = new Set([
    "DRAFT",
    "AWAITING_EXECUTION_APPROVAL",
    "PREPARING",
    "EXECUTING",
    "VERIFYING",
    "REVIEWING",
    "AWAITING_MERGE_APPROVAL",
    "MERGING",
    "COMPLETED",
    "REJECTED",
    "BLOCKED",
    "FAILED",
    "STOPPING",
    "CANCELLED",
]);
//# sourceMappingURL=wire.js.map