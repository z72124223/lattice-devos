import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

const repositoryRoot = path.resolve();
const runner = path.join(repositoryRoot, "scripts", "run-lattice-delivery.ps1");
const powershell = path.join(
  process.env.SystemRoot ?? "C:\\Windows",
  "System32",
  "WindowsPowerShell",
  "v1.0",
  "powershell.exe",
);
const secret = "task093-secret-value";

async function runRuntimeFixture({
  createDiagnosticCollision = false,
  exitCode = 23,
  malformedJson = false,
} = {}) {
  const targetRoot = path.join(repositoryRoot, "target");
  await mkdir(targetRoot, { recursive: true });
  const root = await mkdtemp(path.join(targetRoot, "task093-runtime-diagnostic-"));
  const evidenceRoot = path.join(root, "evidence");
  const diagnosticPath = path.join(
    evidenceRoot,
    "runtime-run-failure-diagnostic.json",
  );
  const runtime = path.join(root, "failing-runtime.cmd");
  await mkdir(evidenceRoot);
  const collision = createDiagnosticCollision
    ? `echo collision>"${diagnosticPath}"\r\n`
    : "";
  const runtimeSource = malformedJson
    ? ["@echo off", "echo not-json", "exit /b 0", ""].join("\r\n")
    : [
        "@echo off",
        `echo stdout password=${secret} token=${secret}`,
        `echo stderr postgresql://user:${secret}@127.0.0.1:5432/lattice 1>&2`,
        "powershell.exe -NoLogo -NoProfile -NonInteractive -Command \"[Console]::OpenStandardOutput().WriteByte(255)\"",
        "cmd /u /c echo invalid-utf8-probe",
        "for /L %%i in (1,1,3000) do echo stdout-overflow-%%i",
        "for /L %%i in (1,1,3000) do echo stderr-overflow-%%i 1>&2",
        collision.trimEnd(),
        `exit /b ${exitCode}`,
        "",
      ].join("\r\n");
  await writeFile(runtime, runtimeSource, "ascii");

  const result = spawnSync(
    powershell,
    [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      runner,
      "-InternalPhase",
      "DeliveryRun",
    ],
    {
      cwd: repositoryRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        LATTICE_DELIVERY_CODEX_MODE: "SCRIPTED_ACCEPTANCE",
        LATTICE_TASK019_HOST: "127.0.0.1",
        LATTICE_TASK019_PORT: "54321",
        LATTICE_TASK019_RUN_ID: "a".repeat(32),
        LATTICE_TASK019_PASSWORD: secret,
        LATTICE_TASK019_LIVE: "1",
        LATTICE_TASK019_PHASE: "restart",
        LATTICE_DELIVERY_FIXTURE_ROOT: path.join(root, "fixture"),
        LATTICE_DELIVERY_RUNTIME_EXE: runtime,
        LATTICE_DELIVERY_LAUNCHER: runtime,
        LATTICE_DELIVERY_LAUNCHER_VERSION: "codex-cli 0.144.6",
        LATTICE_DELIVERY_LAUNCHER_SHA256: "b".repeat(64),
        LATTICE_DELIVERY_SCHEMA_DIR: path.join(root, "schema"),
        LATTICE_DELIVERY_CODEX_HOME: path.join(root, "codex-home"),
        LATTICE_DELIVERY_ROOT: path.join(root, "delivery"),
        LATTICE_DELIVERY_GIT_EXE: process.env.ComSpec ?? runtime,
        LATTICE_DELIVERY_RUN_EVIDENCE: path.join(evidenceRoot, "delivery-run.json"),
        LATTICE_DELIVERY_STATUS_EVIDENCE: path.join(evidenceRoot, "delivery-status.json"),
        LATTICE_DELIVERY_FINAL_EVIDENCE: path.join(evidenceRoot, "final.json"),
      },
    },
  );

  return {
    root,
    diagnosticPath,
    runEvidencePath: path.join(evidenceRoot, "delivery-run.json"),
    result,
  };
}

test("runtime nonzero records a bounded redacted diagnostic and still fails closed", async () => {
  const fixture = await runRuntimeFixture();
  try {
    assert.equal(fixture.result.error, undefined, fixture.result.error?.message);
    assert.notEqual(fixture.result.status, 0);

    const diagnosticBytes = await readFile(fixture.diagnosticPath);
    assert.ok(diagnosticBytes.length <= 32768);
    const diagnostic = JSON.parse(diagnosticBytes.toString("utf8"));
    assert.equal(diagnostic.kind, "LATTICE_DELIVERY_RUNTIME_FAILURE_V1");
    assert.equal(diagnostic.exit_code, 23);
    assert.equal(diagnostic.stdout_truncated, true);
    assert.equal(diagnostic.stderr_truncated, true);
    assert.doesNotMatch(diagnosticBytes.toString("utf8"), new RegExp(secret, "u"));
    assert.doesNotMatch(diagnosticBytes.toString("utf8"), /postgresql:\/\//u);
    assert.match(diagnosticBytes.toString("utf8"), /\[INVALID_UTF8\]/u);
    assert.match(diagnosticBytes.toString("utf8"), /\[INVALID_ENCODING\]/u);
    await assert.rejects(readFile(fixture.runEvidencePath));
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("runtime diagnostic write collision remains fail-closed and emits no receipt", async () => {
  const fixture = await runRuntimeFixture({ createDiagnosticCollision: true });
  try {
    assert.equal(fixture.result.error, undefined, fixture.result.error?.message);
    assert.notEqual(fixture.result.status, 0);
    await assert.rejects(readFile(fixture.runEvidencePath));
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("runtime timeout exit preserves diagnostics and does not create a receipt", async () => {
  const fixture = await runRuntimeFixture({ exitCode: 124 });
  try {
    assert.equal(fixture.result.error, undefined, fixture.result.error?.message);
    assert.notEqual(fixture.result.status, 0);
    const diagnostic = JSON.parse(await readFile(fixture.diagnosticPath, "utf8"));
    assert.equal(diagnostic.exit_code, 124);
    await assert.rejects(readFile(fixture.runEvidencePath));
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("malformed runtime JSON remains fail-closed without a success receipt", async () => {
  const fixture = await runRuntimeFixture({ malformedJson: true });
  try {
    assert.equal(fixture.result.error, undefined, fixture.result.error?.message);
    assert.notEqual(fixture.result.status, 0);
    await assert.rejects(readFile(fixture.diagnosticPath));
    await assert.rejects(readFile(fixture.runEvidencePath));
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});
