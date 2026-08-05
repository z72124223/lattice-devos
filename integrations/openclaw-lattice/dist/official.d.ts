export declare const OFFICIAL_OPENCLAW_PIN: Readonly<{
    bin: "openclaw.mjs";
    commit: "0790d9f593ad30c940ed93b5872a8cf6d6f3cf8c";
    integrity: "sha512-ycF3yPcbjN6bUPeaUx6Mh6vze1hQWoD3CT/wWcmD7a8xaHHHRUaAlaq+lFxMHf1ssEgODVAwjlzYqp2twkYZ7g==";
    license: "MIT";
    main: "dist/index.js";
    packageName: "openclaw";
    pluginSdkEntrypoint: "openclaw/plugin-sdk/plugin-entry";
    version: "2026.7.1-2";
}>;
export declare const LAUNCH_RECORD_SCHEMA: "lattice-openclaw-launch-record-v1";
export interface LatticeOwnedLaunchRecord {
    readonly capabilities: Readonly<{
        readonly cron: boolean;
        readonly hooks: boolean;
        readonly mdns: boolean;
        readonly memory: boolean;
        readonly publicListener: boolean;
        readonly updates: boolean;
    }>;
    readonly gatewayHost: string;
    readonly gatewayPort: number;
    readonly launchRecordId: string;
    readonly officialPackage: Readonly<{
        readonly bin: string;
        readonly commit: string;
        readonly integrity: string;
        readonly license: string;
        readonly main: string;
        readonly packageName: string;
        readonly pluginSdkEntrypoint: string;
        readonly version: string;
    }>;
    readonly owner: string;
    readonly processStartNonce: string;
    readonly profileMode: string;
    readonly schema: string;
}
export interface LatticeLaunchEnvironment {
    readonly deadlineMs: number;
    readonly launchRecordId: string;
    readonly port: number;
    readonly processStartNonce: string;
    readonly rootKey: Buffer;
}
export declare const LAUNCH_ENVIRONMENT_KEYS: Readonly<{
    authenticationKey: "LATTICE_OPENCLAW_AUTH_KEY_HEX";
    deadlineMs: "LATTICE_OPENCLAW_DEADLINE_MS";
    gatewayPort: "LATTICE_OPENCLAW_GATEWAY_PORT";
    launchRecordId: "LATTICE_OPENCLAW_LAUNCH_RECORD_ID";
    processStartNonce: "LATTICE_OPENCLAW_PROCESS_START_NONCE";
}>;
export declare class LatticeLaunchError extends Error {
    constructor();
}
export declare function readLatticeLaunchEnvironment(environment?: NodeJS.ProcessEnv): LatticeLaunchEnvironment;
export declare function validateLatticeOwnedLaunchRecord(record: LatticeOwnedLaunchRecord): void;
//# sourceMappingURL=official.d.ts.map