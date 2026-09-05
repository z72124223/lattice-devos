import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import { createServer } from "node:http";
import path from "node:path";
import os from "node:os";
import { FormalTaskService } from "../src/formal-task-service.mjs";

const sha = (value) => createHash("sha256").update(value).digest("hex");
async function fixture(t, body) {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-result-preview-"));
  const workspace = path.join(root, "workspace"); await mkdir(workspace);
  const taskRef = "a".repeat(64), artifactPath = path.join(workspace, "web.mjs");
  await writeFile(artifactPath, body ?? `import { createServer } from 'node:http';
export async function startServer({port,host}) {
 console.log('ordinary artifact startup message');
 const server=createServer((req,res)=>res.end(JSON.stringify({working:true,secret:process.env.LATTICE_TASK019_PASSWORD??null})));
 await new Promise(resolve=>server.listen(port,host,resolve));return server;
}`);
  const receipts = path.join(root, "control-results", taskRef, "verified-turn"); await mkdir(receipts, { recursive: true });
  async function retain(value) { const bytes = Buffer.from(JSON.stringify(value)); const hash = sha(bytes); await writeFile(path.join(receipts, `${hash}.json`), bytes); return `evidence:sha256:${hash}`; }
  const artifactRef = await retain({ schema: "lattice.local-result.artifact.v1", task_ref: taskRef, path: artifactPath, sha256: sha(await readFile(artifactPath)) });
  const acceptanceRef = await retain({ schema: "lattice.local-result.acceptance.v1", task_ref: taskRef, artifact_ref: artifactRef });
  const detail = { id: taskRef, project_id: "project", completion_verified: true, result_digest: "b".repeat(64), claims: [
    { phase: "EXECUTION", worktree_path: workspace, archived: true },
    { phase: "VERIFICATION", turn_id: "verified-turn", archived: true, verification_outcome: { kind: "VERIFICATION_PASSED", evidence_ref: acceptanceRef } },
  ] };
  const codex = new EventEmitter(); codex.close = async () => {};
  const service = new FormalTaskService({ store: { detail: async () => structuredClone(detail) }, codex,
    configurationLoader: async () => ({ environment: { LATTICE_DELIVERY_ROOT: root, LATTICE_TASK019_PASSWORD: "fixture-secret-must-not-inherit" } }) });
  t.after(async () => { await service.close(); assert.equal(path.dirname(path.resolve(root)), path.resolve(os.tmpdir())); assert.ok(path.basename(root).startsWith("lattice-result-preview-")); await rm(root, { recursive: true }); });
  return { root, detail, service, taskRef, artifactPath, receipts, artifactRef, retain };
}

test("verified archived result opens locally, reuses one process, excludes credentials and closes with service", async (t) => {
  const { service, taskRef, detail } = await fixture(t);
  const first = await service.action("project", taskRef, "preview");
  assert.equal(first.result_digest, detail.result_digest);
  assert.deepEqual(await (await fetch(first.url)).json(), { working: true, secret: null });
  assert.deepEqual(await service.action("project", taskRef, "preview"), first);
  assert.equal(service.previews.size, 1);
  assert.equal((await service.resultPreviewIdentity(first.url + '/')).task_ref, taskRef);
  await assert.rejects(service.resultPreviewIdentity('http://127.0.0.1:4317/'), { code: 'CONTROL_RESULT_PREVIEW_UNAVAILABLE' });
  const child = service.previews.get(taskRef).child;
  await service.close(); assert.ok(child.exitCode !== null || child.signalCode !== null);
  await assert.rejects(fetch(first.url));
});

test("losing the parent IPC connection closes the result listener without another supervisor", async (t) => {
  const { service, taskRef } = await fixture(t);
  const ready = await service.action("project", taskRef, "preview");
  const child = service.previews.get(taskRef).child;
  const exited = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('result child did not exit')), 5000);
    child.once('exit', () => { clearTimeout(timer); resolve(); });
  });
  child.disconnect(); await exited;
  await assert.rejects(fetch(ready.url));
  await assert.rejects(service.resultPreviewIdentity(ready.url + '/'), { code: 'CONTROL_RESULT_PREVIEW_UNAVAILABLE' });
});

test("changed artifact or changed acceptance receipt is rejected before a result process starts", async (t) => {
  const { service, taskRef, artifactPath, receipts, detail } = await fixture(t);
  const original = await readFile(artifactPath);
  await writeFile(artifactPath, "throw new Error('changed');");
  await assert.rejects(service.action("project", taskRef, "preview"), { code: "CONTROL_RESULT_CHANGED" });
  await writeFile(artifactPath, original);
  await writeFile(path.join(receipts, `${detail.claims[1].verification_outcome.evidence_ref.slice(-64)}.json`), "{}");
  await assert.rejects(service.action("project", taskRef, "preview"), { code: "CONTROL_RESULT_CHANGED" });
  assert.equal(service.previews.size, 0);
});

test("unverified work and receipt paths outside its workspace cannot start a preview", async (t) => {
  const { service, taskRef, detail, retain, root } = await fixture(t);
  detail.completion_verified = false;
  await assert.rejects(service.action("project", taskRef, "preview"), { code: "CONTROL_RESULT_NOT_VERIFIED" });
  detail.completion_verified = true;
  const outside = path.join(root, "outside.mjs"); await writeFile(outside, "export const untouched=true;");
  const artifactRef = await retain({ schema: "lattice.local-result.artifact.v1", task_ref: taskRef, path: outside, sha256: sha(await readFile(outside)) });
  detail.claims[1].verification_outcome.evidence_ref = await retain({ schema: "lattice.local-result.acceptance.v1", task_ref: taskRef, artifact_ref: artifactRef });
  await assert.rejects(service.action("project", taskRef, "preview"), { code: "CONTROL_RESULT_PATH_REJECTED" });
  assert.equal(service.previews.size, 0);
});

test("unsupported web interface fails clearly and leaves no retained preview process", async (t) => {
  const { service, taskRef } = await fixture(t, "export const result='file-only';");
  await assert.rejects(service.action("project", taskRef, "preview"), { code: "CONTROL_RESULT_PREVIEW_UNAVAILABLE" });
  assert.equal(service.previews.size, 0);
});

test("artifact cannot present a different process listener as its retained result", async (t) => {
  const unrelated = createServer((request, response) => response.end('unrelated local service'));
  await new Promise((resolve) => unrelated.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise((resolve) => unrelated.close(resolve)));
  const url = `http://127.0.0.1:${unrelated.address().port}`;
  const forged = JSON.stringify({ kind: 'LATTICE_RESULT_READY', url });
  const { service, taskRef } = await fixture(t, `process.send(${forged}); await new Promise(()=>{});`);
  await assert.rejects(service.action('project', taskRef, 'preview'), { code: 'CONTROL_RESULT_PREVIEW_UNAVAILABLE' });
  assert.equal(service.previews.size, 0);
  assert.equal(await (await fetch(url)).text(), 'unrelated local service');
});
