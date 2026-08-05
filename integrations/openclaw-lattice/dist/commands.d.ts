import { type CanonicalValue } from "./cjson.js";
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
export type LatticeCommand = Readonly<{
    action: "status";
    targetKind: "project";
    projectId: string;
}> | Readonly<{
    action: "status";
    targetKind: "command";
    projectId: string;
    targetCommandId: string;
}> | Readonly<{
    action: "status";
    targetKind: "task";
    target: TaskTargetArguments;
}> | Readonly<{
    action: "stop";
    attemptId: string;
    reason: StopReason;
    target: TaskTargetArguments;
}> | Readonly<{
    action: "submit";
    taskSpecDigest: string;
}>;
export interface CommandIdentities {
    readonly commandId: string;
    readonly correlationId: string;
}
export declare class LatticeInputError extends Error {
    constructor();
}
export declare function parseLatticeArguments(input: string): LatticeCommand;
export declare function deriveCommandIdentities(sessionKey: string, command: LatticeCommand): CommandIdentities;
export declare function canonicalArguments(command: LatticeCommand): CanonicalValue;
//# sourceMappingURL=commands.d.ts.map