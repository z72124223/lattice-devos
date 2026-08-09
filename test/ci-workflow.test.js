import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const workflowUrl = new URL("../.github/workflows/ci.yml", import.meta.url);
const checkoutAction =
  "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1";
const setupNodeAction =
  "actions/setup-node@820762786026740c76f36085b0efc47a31fe5020";

function topLevelBlock(source, name) {
  const lines = source.split(/\r?\n/u);
  const start = lines.indexOf(`${name}:`);
  assert.notEqual(start, -1, `missing top-level '${name}' block`);
  const end = lines.findIndex(
    (line, index) => index > start && /^[a-zA-Z0-9_-]+:\s*$/u.test(line),
  );
  return lines.slice(start, end === -1 ? lines.length : end).join("\n");
}

function jobBlock(source, name) {
  const lines = source.split(/\r?\n/u);
  const start = lines.indexOf(`  ${name}:`);
  assert.notEqual(start, -1, `missing '${name}' job`);
  const end = lines.findIndex(
    (line, index) => index > start && /^  [a-zA-Z0-9_-]+:\s*$/u.test(line),
  );
  return lines.slice(start, end === -1 ? lines.length : end).join("\n");
}

function eventBlock(source, name) {
  const triggers = topLevelBlock(source, "on");
  const lines = triggers.split(/\r?\n/u);
  const start = lines.indexOf(`  ${name}:`);
  assert.notEqual(start, -1, `missing '${name}' trigger`);
  const end = lines.findIndex(
    (line, index) =>
      index > start && /^  [a-zA-Z0-9_-]+:\s*(?:#.*)?$/u.test(line),
  );
  return lines.slice(start, end === -1 ? lines.length : end).join("\n");
}

function actionUses(job) {
  return [...job.matchAll(/^\s+- uses:\s*(\S+)\s*(?:#.*)?$/gmu)].map(
    (match) => match[1],
  );
}

function runCommands(job) {
  return [...job.matchAll(/^\s+(?:-\s+)?run:\s*(\S.*)\s*$/gmu)].map(
    (match) => match[1],
  );
}

test("CI enforces Node and Rust verification on PRs and every branch push", async () => {
  const workflow = await readFile(workflowUrl, "utf8");
  const triggers = topLevelBlock(workflow, "on");

  assert.match(triggers, /^  pull_request:\s*$/mu);
  assert.equal(
    eventBlock(workflow, "push").trim(),
    "push:",
    "push must not contain branch, tag, or path filters",
  );
  assert.equal(
    topLevelBlock(workflow, "permissions").trim(),
    "permissions:\n  contents: read",
  );
  assert.equal(
    [...workflow.matchAll(/^\s*permissions\s*:/gmu)].length,
    1,
    "jobs must not override the read-only workflow permissions",
  );

  const nodeJob = jobBlock(workflow, "verify");
  assert.deepEqual(actionUses(nodeJob), [checkoutAction, setupNodeAction]);
  assert.match(nodeJob, /^\s+node-version: ["']?24["']?\s*$/mu);
  assert.deepEqual(runCommands(nodeJob), [
    "npm ci --ignore-scripts",
    "npm run verify",
  ]);

  const rustJob = jobBlock(workflow, "rust");
  assert.deepEqual(actionUses(rustJob), [checkoutAction]);
  assert.deepEqual(runCommands(rustJob), [
    "rustup toolchain install 1.97.1 --profile minimal --component clippy --component rustfmt",
    "cargo +1.97.1 fmt --all -- --check",
    "cargo +1.97.1 clippy --workspace --all-targets --all-features --locked -- -D warnings",
    "cargo +1.97.1 test --workspace --all-targets --all-features --locked",
  ]);
});
