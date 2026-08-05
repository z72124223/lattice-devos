import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

import {
  OFFICIAL_OPENCLAW_PIN,
  readLatticeLaunchEnvironment,
} from "../src/official.js";

test("pins the verified official package, runtime, and SDK entry", () => {
  assert.deepEqual(OFFICIAL_OPENCLAW_PIN, {
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
});

test("package pairs official source/runtime entries and keeps runtime id lattice-devos", async () => {
  const root = new URL("../", import.meta.url);
  const packageJson = JSON.parse(await readFile(new URL("package.json", root), "utf8")) as {
    readonly openclaw: {
      readonly compat: { readonly pluginApi: string };
      readonly extensions: readonly string[];
      readonly plugin: { readonly id: string };
      readonly runtimeExtensions: readonly string[];
    };
    readonly peerDependencies: Record<string, string>;
  };
  const manifest = JSON.parse(
    await readFile(new URL("openclaw.plugin.json", root), "utf8"),
  ) as { readonly id: string };
  assert.deepEqual(packageJson.openclaw.extensions, ["./src/index.ts"]);
  assert.deepEqual(packageJson.openclaw.runtimeExtensions, ["./dist/index.js"]);
  assert.equal(packageJson.openclaw.plugin.id, "lattice-devos");
  assert.equal(packageJson.openclaw.compat.pluginApi, "2026.7.1");
  assert.equal(packageJson.peerDependencies.openclaw, OFFICIAL_OPENCLAW_PIN.version);
  assert.equal(manifest.id, "lattice-devos");
});

test("reads launch identity and root key only from bounded LATTICE environment", () => {
  const launch = readLatticeLaunchEnvironment({
    LATTICE_OPENCLAW_AUTH_KEY_HEX: "1".repeat(64),
    LATTICE_OPENCLAW_DEADLINE_MS: "1000",
    LATTICE_OPENCLAW_GATEWAY_PORT: "49152",
    LATTICE_OPENCLAW_LAUNCH_RECORD_ID: "launch-a",
    LATTICE_OPENCLAW_PROCESS_START_NONCE: "2".repeat(32),
  });
  assert.equal(launch.launchRecordId, "launch-a");
  assert.equal(launch.port, 49_152);
  assert.equal(launch.rootKey.toString("hex"), "1".repeat(64));
  assert.throws(
    () =>
      readLatticeLaunchEnvironment({
        LATTICE_OPENCLAW_AUTH_KEY_HEX: "1".repeat(64),
        LATTICE_OPENCLAW_DEADLINE_MS: "1000",
        LATTICE_OPENCLAW_GATEWAY_PORT: "49152",
        LATTICE_OPENCLAW_LAUNCH_RECORD_ID: "C:/profile",
        LATTICE_OPENCLAW_PROCESS_START_NONCE: "2".repeat(32),
      }),
    { name: "LatticeLaunchError" },
  );
});
