import type { CommandIdentities, LatticeCommand } from "./commands.js";
export declare const INBOUND_PROTOCOL: "lattice-openclaw-inbound";
export declare const CLIENT_HELLO_PROTOCOL: "lattice-openclaw-client-hello";
export declare const WIRE_VERSION: "1";
export declare class LatticeWireError extends Error {
    constructor();
}
export interface DecodedCommandReply {
    readonly kind: "accepted" | "denied" | "observed" | "routed" | "unknown";
    readonly summary: string;
}
export declare function encodeClientHello(launchRecordId: string, processStartNonce: string): Buffer;
export declare function encodeCommandRequest(command: LatticeCommand, identities: CommandIdentities): Buffer;
export declare function decodeCommandReply(payload: Buffer, command: LatticeCommand, identities: CommandIdentities): DecodedCommandReply;
//# sourceMappingURL=wire.d.ts.map