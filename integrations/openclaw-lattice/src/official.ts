export const OFFICIAL_OPENCLAW_PIN = Object.freeze({
  bin: "openclaw.mjs",
  commit: "0790d9f593ad30c940ed93b5872a8cf6d6f3cf8c",
  integrity:
    "sha512-ycF3yPcbjN6bUPeaUx6Mh6vze1hQWoD3CT/wWcmD7a8xaHHHRUaAlaq+lFxMHf1ssEgODVAwjlzYqp2twkYZ7g==",
  license: "MIT",
  main: "dist/index.js",
  packageName: "openclaw",
  pluginSdkEntrypoint: "openclaw/plugin-sdk/plugin-entry",
  version: "2026.7.1-2",
});

export const LAUNCH_RECORD_SCHEMA = "lattice-openclaw-launch-record-v1" as const;

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

export const LAUNCH_ENVIRONMENT_KEYS = Object.freeze({
  authenticationKey: "LATTICE_OPENCLAW_AUTH_KEY_HEX",
  deadlineMs: "LATTICE_OPENCLAW_DEADLINE_MS",
  gatewayPort: "LATTICE_OPENCLAW_GATEWAY_PORT",
  launchRecordId: "LATTICE_OPENCLAW_LAUNCH_RECORD_ID",
  processStartNonce: "LATTICE_OPENCLAW_PROCESS_START_NONCE",
});

const SAFE_IDENTIFIER = /^[A-Za-z0-9][A-Za-z0-9._:@-]{0,255}$/u;
const HEX_32 = /^[0-9a-f]{32}$/u;
const HEX_64 = /^[0-9a-f]{64}$/u;

export class LatticeLaunchError extends Error {
  public constructor() {
    super("invalid LATTICE-owned OpenClaw launch environment");
    this.name = "LatticeLaunchError";
  }
}

export function readLatticeLaunchEnvironment(
  environment: NodeJS.ProcessEnv = process.env,
): LatticeLaunchEnvironment {
  const launchRecordId = environment[LAUNCH_ENVIRONMENT_KEYS.launchRecordId];
  const processStartNonce = environment[LAUNCH_ENVIRONMENT_KEYS.processStartNonce];
  const authenticationKey = environment[LAUNCH_ENVIRONMENT_KEYS.authenticationKey];
  const port = parseBoundedInteger(environment[LAUNCH_ENVIRONMENT_KEYS.gatewayPort], 65_535);
  const deadlineMs = parseBoundedInteger(
    environment[LAUNCH_ENVIRONMENT_KEYS.deadlineMs],
    30_000,
  );
  if (
    launchRecordId === undefined ||
    processStartNonce === undefined ||
    authenticationKey === undefined ||
    !SAFE_IDENTIFIER.test(launchRecordId) ||
    !HEX_32.test(processStartNonce) ||
    /^0+$/u.test(processStartNonce) ||
    !HEX_64.test(authenticationKey) ||
    /^0+$/u.test(authenticationKey)
  ) {
    throw new LatticeLaunchError();
  }
  return {
    deadlineMs,
    launchRecordId,
    port,
    processStartNonce,
    rootKey: Buffer.from(authenticationKey, "hex"),
  };
}

export function validateLatticeOwnedLaunchRecord(
  record: LatticeOwnedLaunchRecord,
): void {
  if (
    record.schema !== LAUNCH_RECORD_SCHEMA ||
    record.owner !== "lattice" ||
    record.profileMode !== "isolated-temp" ||
    record.gatewayHost !== "127.0.0.1" ||
    !Number.isInteger(record.gatewayPort) ||
    record.gatewayPort < 1 ||
    record.gatewayPort > 65_535 ||
    !SAFE_IDENTIFIER.test(record.launchRecordId) ||
    !HEX_32.test(record.processStartNonce) ||
    /^0+$/u.test(record.processStartNonce) ||
    record.officialPackage.packageName !== OFFICIAL_OPENCLAW_PIN.packageName ||
    record.officialPackage.version !== OFFICIAL_OPENCLAW_PIN.version ||
    record.officialPackage.integrity !== OFFICIAL_OPENCLAW_PIN.integrity ||
    record.officialPackage.commit !== OFFICIAL_OPENCLAW_PIN.commit ||
    record.officialPackage.license !== OFFICIAL_OPENCLAW_PIN.license ||
    record.officialPackage.bin !== OFFICIAL_OPENCLAW_PIN.bin ||
    record.officialPackage.main !== OFFICIAL_OPENCLAW_PIN.main ||
    record.officialPackage.pluginSdkEntrypoint !== OFFICIAL_OPENCLAW_PIN.pluginSdkEntrypoint ||
    record.capabilities.cron ||
    record.capabilities.hooks ||
    record.capabilities.mdns ||
    record.capabilities.memory ||
    record.capabilities.publicListener ||
    record.capabilities.updates
  ) {
    throw new LatticeLaunchError();
  }
}

function parseBoundedInteger(value: string | undefined, maximum: number): number {
  if (value === undefined || !/^[1-9][0-9]*$/u.test(value)) {
    throw new LatticeLaunchError();
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed > maximum) {
    throw new LatticeLaunchError();
  }
  return parsed;
}
