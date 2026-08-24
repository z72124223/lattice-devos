import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { promisify } from "node:util";
import { createLatticeServer } from "../src/server.mjs";
import { recordInstallationObservation } from "../src/installation-receipt-client.mjs";

const execFileAsync = promisify(execFile);
const projectRoot = path.resolve(import.meta.dirname, "../../..");
const clientEntrypoint = path.join(
  projectRoot,
  "apps",
  "lattice-control",
  "src",
  "installation-receipt-client.mjs",
);

class FakeCodex extends EventEmitter {
  connected = false;

  async close() {}
}

async function listen(application) {
  await new Promise((resolve, reject) => {
    application.server.once("error", reject);
    application.server.listen(0, "127.0.0.1", resolve);
  });
  const address = application.server.address();
  return `http://127.0.0.1:${address.port}`;
}

test("AI client computes, records, and re-reads an installation observation", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-control-ai-receipt-"));
  const databasePath = path.join(directory, "control.db");
  const artifactPath = path.join(directory, "lattice.exe");
  const artifact = Buffer.from("verified lattice artifact\n", "utf8");
  const application = createLatticeServer({ databasePath, codex: new FakeCodex() });
  try {
    await writeFile(artifactPath, artifact);
    application.store.createProject({ name: "LATTICE DevOS", rootPath: directory });
    const controlOrigin = await listen(application);
    const input = {
      controlOrigin,
      projectName: "LATTICE DevOS",
      component: "lattice-cli",
      sourceCommitSha: "a".repeat(40),
      artifactPath,
    };

    const first = await recordInstallationObservation(input);
    assert.equal(first.status, "OBSERVATION_RECORDED_AND_REPLAYED");
    assert.equal(first.created, true);
    assert.equal(first.receipt.artifact_sha256, createHash("sha256").update(artifact).digest("hex"));
    assert.equal(first.receipt.authority, "NON_AUTHORITATIVE");

    const retry = await recordInstallationObservation(input);
    assert.equal(retry.created, false);
    assert.equal(retry.receipt.id, first.receipt.id);
    assert.equal(application.store.countInstallationReceipts(), 1);
  } finally {
    if (application.server.listening) {
      await new Promise((resolve) => application.server.close(resolve));
    } else {
      application.service.close();
      await application.codex.close();
      application.store.close();
    }
    await rm(directory, { recursive: true, force: true });
  }
});

test("AI client refuses non-loopback Control endpoints", async () => {
  let called = false;
  await assert.rejects(
    recordInstallationObservation({
      controlOrigin: "https://example.com",
      projectName: "LATTICE DevOS",
      component: "lattice-cli",
      sourceCommitSha: "a".repeat(40),
      artifactPath: path.resolve("lattice.exe"),
      fetchImpl: async () => {
        called = true;
        throw new Error("unexpected fetch");
      },
    }),
    /loopback/u,
  );
  assert.equal(called, false);
});

test("the packaged CLI entrypoint records and replays an observation", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-control-ai-cli-"));
  const artifactPath = path.join(directory, "lattice.exe");
  const application = createLatticeServer({
    databasePath: path.join(directory, "control.db"),
    codex: new FakeCodex(),
  });
  try {
    await writeFile(artifactPath, "cli artifact\n", "utf8");
    application.store.createProject({ name: "LATTICE CLI Test", rootPath: directory });
    const controlOrigin = await listen(application);
    const packageJson = JSON.parse(await readFile(path.join(projectRoot, "package.json"), "utf8"));
    assert.equal(
      packageJson.scripts["control:receipt"],
      "node apps/lattice-control/src/installation-receipt-client.mjs",
    );
    const { stdout, stderr } = await execFileAsync(process.execPath, [
      clientEntrypoint,
      "--origin", controlOrigin,
      "--project-name", "LATTICE CLI Test",
      "--component", "lattice-cli",
      "--source-commit", "a".repeat(40),
      "--artifact", artifactPath,
    ], { cwd: projectRoot });
    assert.equal(stderr, "");
    const result = JSON.parse(stdout);
    assert.equal(result.status, "OBSERVATION_RECORDED_AND_REPLAYED");
    assert.equal(result.created, true);
    assert.equal(application.store.countInstallationReceipts(), 1);
  } finally {
    if (application.server.listening) {
      await new Promise((resolve) => application.server.close(resolve));
    } else {
      application.service.close();
      await application.codex.close();
      application.store.close();
    }
    await rm(directory, { recursive: true, force: true });
  }
});

test("AI client never follows a redirect from the loopback Control endpoint", async () => {
  let redirectedRequests = 0;
  const captureServer = createServer((_request, response) => {
    redirectedRequests += 1;
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ projects: [{ id: "captured", name: "LATTICE DevOS" }] }));
  });
  const redirectServer = createServer((_request, response) => {
    const captureAddress = captureServer.address();
    response.writeHead(307, {
      location: `http://127.0.0.1:${captureAddress.port}/capture`,
    });
    response.end();
  });
  try {
    const captureOrigin = await listen({ server: captureServer });
    assert.match(captureOrigin, /^http:\/\/127\.0\.0\.1:/u);
    const redirectOrigin = await listen({ server: redirectServer });
    await assert.rejects(
      recordInstallationObservation({
        controlOrigin: redirectOrigin,
        projectName: "LATTICE DevOS",
        component: "lattice-cli",
        sourceCommitSha: "a".repeat(40),
        artifactPath: path.resolve("lattice.exe"),
      }),
    );
    assert.equal(redirectedRequests, 0);
  } finally {
    await Promise.all([captureServer, redirectServer].map((server) => (
      server.listening ? new Promise((resolve) => server.close(resolve)) : Promise.resolve()
    )));
  }
});

test("AI client rejects a replay whose receipt fields do not match the observation", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-control-ai-tamper-"));
  const artifactPath = path.join(directory, "lattice.exe");
  const artifact = Buffer.from("artifact", "utf8");
  const artifactSha256 = createHash("sha256").update(artifact).digest("hex");
  const receipt = {
    id: "11111111-1111-4111-8111-111111111111",
    receipt_digest: "d".repeat(64),
    schema_version: "lattice.control.installation-receipt.v1",
    observation_kind: "OBSERVED_AFTER_INSTALL",
    authority: "NON_AUTHORITATIVE",
    project_id: "project-1",
    component: "lattice-cli",
    source_commit_sha: "a".repeat(40),
    artifact_path: path.normalize(artifactPath),
    artifact_sha256: artifactSha256,
  };
  let requestNumber = 0;
  const fetchImpl = async () => {
    requestNumber += 1;
    const responses = [
      { status: 200, body: { projects: [{ id: "project-1", name: "LATTICE DevOS" }] } },
      { status: 201, body: receipt },
      { status: 200, body: { ...receipt, authority: "AUTHORITATIVE" } },
    ];
    const selected = responses[requestNumber - 1];
    return new Response(JSON.stringify(selected.body), {
      status: selected.status,
      headers: { "content-type": "application/json" },
    });
  };
  try {
    await writeFile(artifactPath, artifact);
    await assert.rejects(
      recordInstallationObservation({
        projectName: "LATTICE DevOS",
        component: "lattice-cli",
        sourceCommitSha: "a".repeat(40),
        artifactPath,
        fetchImpl,
      }),
      /authority/u,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("AI client rejects a recorded response without a receipt identity", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-control-ai-malformed-"));
  const artifactPath = path.join(directory, "lattice.exe");
  const artifact = Buffer.from("artifact", "utf8");
  const artifactSha256 = createHash("sha256").update(artifact).digest("hex");
  const receiptWithoutIdentity = {
    schema_version: "lattice.control.installation-receipt.v1",
    observation_kind: "OBSERVED_AFTER_INSTALL",
    authority: "NON_AUTHORITATIVE",
    project_id: "project-1",
    component: "lattice-cli",
    source_commit_sha: "a".repeat(40),
    artifact_path: path.normalize(artifactPath),
    artifact_sha256: artifactSha256,
  };
  let requestNumber = 0;
  const fetchImpl = async () => {
    requestNumber += 1;
    const responses = [
      { status: 200, body: { projects: [{ id: "project-1", name: "LATTICE DevOS" }] } },
      { status: 201, body: receiptWithoutIdentity },
      { status: 200, body: receiptWithoutIdentity },
    ];
    const selected = responses[requestNumber - 1];
    return new Response(JSON.stringify(selected.body), {
      status: selected.status,
      headers: { "content-type": "application/json" },
    });
  };
  try {
    await writeFile(artifactPath, artifact);
    await assert.rejects(
      recordInstallationObservation({
        projectName: "LATTICE DevOS",
        component: "lattice-cli",
        sourceCommitSha: "a".repeat(40),
        artifactPath,
        fetchImpl,
      }),
      /receipt ID|receipt digest/u,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
