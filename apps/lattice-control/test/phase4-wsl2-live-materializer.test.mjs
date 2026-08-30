import assert from "node:assert/strict";
import test from "node:test";

import {
  assertReviewedSupervisorDigest,
  hashPhase4Wsl2ToolsAfterVersionValidation,
  phase4Wsl2MaterializationFailureEnvelope,
} from "../../../scripts/materialize-phase4-wsl2-live-environment.mjs";

const VALID_VERSIONS = Object.freeze({
  gateway: "2.6.1",
  launcher: "codex-cli 0.146.0",
  node: "v24.15.0",
  git: "git version 2.53.0",
  systemd_run: "systemd 259 (259.5-0ubuntu3.4)",
  systemctl: "systemd 259 (259.5-0ubuntu3.4)",
  bootstrap_node: "v22.22.1",
  lsattr: "lsattr 1.47.2 (1-Jan-2025)",
  sudo: "sudo-rs 0.2.13-0ubuntu1",
  bwrap: "bubblewrap 0.11.1",
  npm: "11.12.1",
  cargo: "cargo 1.97.1 (c980f4866 2026-03-10)",
  rustc: "rustc 1.97.1 (8bab26f4f 2026-03-10)",
  rustdoc: "rustdoc 1.97.1 (8bab26f4f 2026-03-10)",
});

test("materializer rejects every credential-shaped tool version before hashing", async () => {
  let hashCalls = 0;
  for (const name of Object.keys(VALID_VERSIONS)) {
    for (const suffix of [" token=fixture", " password=fixture", " secret=fixture"]) {
      const versions = { ...VALID_VERSIONS, [name]: `${VALID_VERSIONS[name]}${suffix}` };
      await assert.rejects(() => hashPhase4Wsl2ToolsAfterVersionValidation(
        versions,
        async () => {
          hashCalls += 1;
          return { unreachable: true };
        },
      ), { code: "PHASE4_WSL2_TOOL_VERSION_REJECTED" }, `${name} accepted ${suffix.trim()}`);
    }
  }
  assert.equal(hashCalls, 0);

  const hashes = await hashPhase4Wsl2ToolsAfterVersionValidation(
    VALID_VERSIONS,
    async () => {
      hashCalls += 1;
      return { exact: true };
    },
  );
  assert.deepEqual(hashes, { exact: true });
  assert.equal(hashCalls, 1);
});

test("materializer rejects an unreviewed supervisor digest before any live preflight", () => {
  const supervisor = "/home/zk/lattice-phase4/runtime-v4/wsl2-codex-supervisor.mjs";
  const expected = "a".repeat(64);
  assert.equal(assertReviewedSupervisorDigest({ [supervisor]: expected }, supervisor, expected), expected);
  for (const [digests, path, digest] of [
    [{ [supervisor]: "b".repeat(64) }, supervisor, expected],
    [{}, supervisor, expected],
    [{ [supervisor]: expected }, supervisor, "A".repeat(64)],
    [{ [supervisor]: expected }, "/tmp/wsl2-codex-supervisor.mjs", expected],
  ]) {
    assert.throws(() => assertReviewedSupervisorDigest(digests, path, digest), {
      code: "PHASE4_WSL2_REVIEWED_SUPERVISOR_REJECTED",
    });
  }
});

test("materializer failure envelope never serializes message stdout or stderr secrets", () => {
  const sentinel = "PHASE4_VERSION_SECRET_SENTINEL_7f3c";
  const error = new Error(`message password=${sentinel}`);
  error.code = "PHASE4_WSL2_TOOL_VERSION_REJECTED";
  error.stdout = `token=${sentinel}`;
  error.stderr = Buffer.from(`secret=${sentinel}`, "utf8");
  error.signal = "SIGTERM";
  error.killed = true;

  const envelope = phase4Wsl2MaterializationFailureEnvelope(error);
  const serialized = JSON.stringify(envelope);
  assert.equal(serialized.includes(sentinel), false);
  assert.equal(serialized.includes("password="), false);
  assert.equal(serialized.includes("token="), false);
  assert.equal(serialized.includes("secret="), false);
  assert.deepEqual(Object.keys(envelope).sort(), [
    "code", "provider_effect_count", "schema", "stage", "status", "stderr", "stdout",
    "transport_exit_code", "transport_killed", "transport_signal",
  ]);
  assert.equal(envelope.code, "PHASE4_WSL2_TOOL_VERSION_REJECTED");
  assert.equal(envelope.stage, "TOOL_VERSION_VALIDATION");
  assert.equal(envelope.transport_signal, "SIGTERM");
  assert.equal(envelope.transport_killed, true);
  assert.equal(envelope.stdout.byte_len, Buffer.byteLength(error.stdout));
  assert.equal(envelope.stderr.byte_len, error.stderr.length);
  assert.match(envelope.stdout.sha256, /^[a-f0-9]{64}$/u);
  assert.match(envelope.stderr.sha256, /^[a-f0-9]{64}$/u);

  const reviewed = new Error("PHASE4_WSL2_REVIEWED_SUPERVISOR_REJECTED");
  reviewed.code = "PHASE4_WSL2_REVIEWED_SUPERVISOR_REJECTED";
  assert.equal(
    phase4Wsl2MaterializationFailureEnvelope(reviewed).stage,
    "REVIEWED_SUPERVISOR_VALIDATION",
  );
});
