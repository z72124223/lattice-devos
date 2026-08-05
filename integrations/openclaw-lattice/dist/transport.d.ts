export declare const LOOPBACK_HOST: "127.0.0.1";
export declare const TRANSPORT_WIRE: Readonly<{
    headerBytes: 76;
    maxFrameBytes: 1048576;
    nonceBytes: 16;
    requestMagic: "LATGW001";
    responseMagic: "LATGR001";
    sessionEpochBytes: 16;
    sessionMagic: "LATSN001";
    tagBytes: 32;
}>;
export type LatticeTransportErrorCode = "AUTHENTICATION" | "CONFIGURATION" | "DISCONNECTED" | "MALFORMED" | "TIMEOUT" | "UNAVAILABLE";
export declare class LatticeTransportError extends Error {
    readonly code: LatticeTransportErrorCode;
    constructor(code: LatticeTransportErrorCode);
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
export declare function exchangeAuthenticatedFrames(options: AuthenticatedExchangeOptions): Promise<Buffer>;
//# sourceMappingURL=transport.d.ts.map