import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";
import {
  defaultControlOrigin,
  normalizeControlOrigin,
  requestJson,
  requireText,
  resolveProject,
} from "./control-client.mjs";

async function sha256File(artifactPath) {
  const before = await stat(artifactPath);
  if (!before.isFile()) throw new TypeError("artifact must be a regular file");
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(artifactPath)) hash.update(chunk);
  const after = await stat(artifactPath);
  if (before.size !== after.size || before.mtimeMs !== after.mtimeMs) {
    throw new Error("artifact changed while its SHA-256 was being calculated");
  }
  return hash.digest("hex");
}

function requireReceiptIdentity(receipt) {
  const id = requireText(receipt?.id, "receipt ID");
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(id)) {
    throw new TypeError("receipt ID must be a lowercase UUID");
  }
  const digest = requireText(receipt?.receipt_digest, "receipt digest");
  if (!/^[a-f0-9]{64}$/u.test(digest)) {
    throw new TypeError("receipt digest must be 64 lowercase hexadecimal characters");
  }
  return { id, digest };
}

export async function recordInstallationObservation({
  controlOrigin = defaultControlOrigin,
  projectId,
  projectName,
  component,
  sourceCommitSha,
  artifactPath,
  fetchImpl = fetch,
  requestTimeoutMs = 5_000,
}) {
  const origin = normalizeControlOrigin(controlOrigin);
  if (!Number.isInteger(requestTimeoutMs) || requestTimeoutMs < 1 || requestTimeoutMs > 60_000) {
    throw new TypeError("request timeout must be an integer from 1 to 60000 milliseconds");
  }
  const absoluteArtifactPath = path.resolve(requireText(artifactPath, "artifact path"));
  const stateResult = await requestJson(fetchImpl, `${origin}/api/state`, {}, requestTimeoutMs);
  const project = resolveProject(stateResult.body.projects, { projectId, projectName });
  const artifactSha256 = await sha256File(absoluteArtifactPath);
  const normalizedComponent = requireText(component, "component").toLowerCase();
  const normalizedSourceCommitSha = requireText(sourceCommitSha, "source commit SHA").toLowerCase();
  const recorded = await requestJson(fetchImpl, `${origin}/api/installation-receipts`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      projectId: project.id,
      component: normalizedComponent,
      sourceCommitSha: normalizedSourceCommitSha,
      artifactPath: absoluteArtifactPath,
      artifactSha256,
    }),
  }, requestTimeoutMs);
  if (![200, 201].includes(recorded.response.status)) {
    throw new Error(`Control returned unexpected receipt status ${recorded.response.status}`);
  }
  const receiptIdentity = requireReceiptIdentity(recorded.body);
  const replayed = await requestJson(
    fetchImpl,
    `${origin}/api/installation-receipts/${encodeURIComponent(receiptIdentity.id)}`,
    {},
    requestTimeoutMs,
  );
  const replay = replayed.body;
  const expected = {
    id: receiptIdentity.id,
    receipt_digest: receiptIdentity.digest,
    schema_version: "lattice.control.installation-receipt.v1",
    observation_kind: "OBSERVED_AFTER_INSTALL",
    authority: "NON_AUTHORITATIVE",
    project_id: project.id,
    component: normalizedComponent,
    source_commit_sha: normalizedSourceCommitSha,
    artifact_path: path.normalize(absoluteArtifactPath),
    artifact_sha256: artifactSha256,
  };
  for (const [field, value] of Object.entries(expected)) {
    if (replay[field] !== value) {
      throw new Error(`replayed installation observation did not match ${field}`);
    }
  }
  return {
    status: "OBSERVATION_RECORDED_AND_REPLAYED",
    created: recorded.response.status === 201,
    receipt: replay,
  };
}

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const name = argv[index];
    if (name === "--help") return { help: true };
    const key = new Map([
      ["--origin", "controlOrigin"],
      ["--project-id", "projectId"],
      ["--project-name", "projectName"],
      ["--component", "component"],
      ["--source-commit", "sourceCommitSha"],
      ["--artifact", "artifactPath"],
    ]).get(name);
    if (!key) throw new TypeError(`unknown option: ${name}`);
    if (options[key] !== undefined) throw new TypeError(`duplicate option: ${name}`);
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new TypeError(`missing value for ${name}`);
    }
    options[key] = value;
    index += 1;
  }
  return options;
}

const usage = [
  "AI-only installation observation recorder",
  "",
  "npm.cmd run control:receipt -- --project-name <name> --component <id> --source-commit <40-hex> --artifact <path>",
  "",
  "Options: --project-id <id> may replace --project-name; --origin defaults to http://127.0.0.1:4317",
].join("\n");

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const options = parseArguments(process.argv.slice(2));
    if (options.help) process.stdout.write(`${usage}\n`);
    else process.stdout.write(`${JSON.stringify(await recordInstallationObservation(options))}\n`);
  } catch (error) {
    process.stderr.write(`${JSON.stringify({ status: "FAILED", error: error.message })}\n`);
    process.exitCode = 1;
  }
}
