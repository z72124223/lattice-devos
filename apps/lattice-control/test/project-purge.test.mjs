import assert from 'node:assert/strict';
import { mkdir, mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { LatticeStore } from '../src/store.mjs';
import { previewProjectPurge, applyProjectPurge, statusProjectPurge } from '../src/project-purge.mjs';

async function fixture(t) {
  const parent = path.resolve('.lattice/project-purge-workflow-tests');
  await mkdir(parent, { recursive: true });
  const root = await mkdtemp(path.join(parent, 'case-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const target = path.join(root, 'target'), survivor = path.join(root, 'survivor');
  await mkdir(target); await mkdir(survivor);
  await writeFile(path.join(target, 'target.txt'), 'target content');
  await writeFile(path.join(survivor, 'keep.txt'), 'retained content');
  const databasePath = path.join(root, 'control.db');
  const store = new LatticeStore(databasePath);
  const project = store.createProject({ name: 'target', rootPath: target });
  store.createProject({ name: 'survivor', rootPath: survivor });
  store.database.close();
  const statePath = path.join(root, 'progress.json');
  let applied = 0, receipt, loseReply = false, readbackChanged = false, blockers = [];
  const native = async (_binary, request) => {
    if (request.action === 'status') {
      if (receipt && readbackChanged) throw new Error('PROJECT_PURGE_READBACK_CHANGED');
      return receipt ?? { status: 'UNKNOWN_OPERATION' };
    }
    if (request.action === 'preview') return { status: blockers.length ? 'BLOCKED' : 'READY',
      project: { id: project.id, canonicalPath: target }, filesystemRoots: [{ path: target, source: 'registry' }],
      protectedRoots: [survivor], counts: { projects: 1 }, scopeDigest: 'a'.repeat(64), blockers };
    applied++;
    receipt = { status: 'PURGED', phase: 'POSTGRES_ONLY', operationId: request.operationId, scopeDigest: 'a'.repeat(64) };
    if (loseReply) { loseReply = false; throw new Error('PURGE_NATIVE_TIMEOUT_OUTCOME_UNKNOWN'); }
    return receipt;
  };
  const preview = () => previewProjectPurge({ projectId: project.id, databasePath, nativeBinary: path.join(root, 'native.exe'), statePath }, { native });
  return { root, target, survivor, databasePath, statePath, project, native, preview,
    get applied() { return applied; }, loseReply() { loseReply = true; }, changeReadback() { readbackChanged = true; }, block() { blockers = ['CROSS_SCOPE_RETAINED_REFERENCE']; } };
}

test('scoped real SQLite and files purge keeps survivor, verifies receipt, and replays idempotently', async t => {
  const f = await fixture(t), plan = await f.preview();
  const result = await applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native });
  assert.equal(result.status, 'SCOPED_PURGED');
  assert.equal(result.externalCleanup, 'NOT_VERIFIED');
  assert.equal(await readFile(path.join(f.survivor, 'keep.txt'), 'utf8'), 'retained content');
  assert.equal((await statusProjectPurge(plan, { native: f.native })).status, 'SCOPED_PURGED');
  assert.equal((await applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native })).status, 'SCOPED_PURGED');
  assert.equal(f.applied, 1);
});

test('exact approval and offline maintenance are required before any mutation', async t => {
  const f = await fixture(t), plan = await f.preview();
  await assert.rejects(applyProjectPurge(plan, { native: f.native }), /EXACT_CONFIRMATION/);
  await assert.rejects(applyProjectPurge(plan, { native: f.native, confirmDigest: plan.digest }), /EXACT_CONFIRMATION/);
  assert.equal(f.applied, 0);
});

test('blocked preview cannot apply', async t => {
  const f = await fixture(t); f.block(); const plan = await f.preview();
  assert.equal(plan.status, 'BLOCKED');
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /PURGE_BLOCKED/);
  assert.equal(f.applied, 0);
});

test('filesystem drift is rejected before PostgreSQL deletion', async t => {
  const f = await fixture(t), plan = await f.preview();
  await writeFile(path.join(f.target, 'unapproved.txt'), 'new');
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /SCOPE_CHANGED/);
  assert.equal(f.applied, 0);
  const db = new LatticeStore(f.databasePath); assert.ok(db.getProject(f.project.id)); db.database.close();
});

test('SQLite drift is rejected before PostgreSQL deletion', async t => {
  const f = await fixture(t), plan = await f.preview();
  const db = new LatticeStore(f.databasePath); db.createWorkItem({ projectId: f.project.id, title: 'new', objective: 'unapproved' }); db.database.close();
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /STALE_SCOPE/);
  assert.equal(f.applied, 0);
});

test('lost PostgreSQL reply records unknown outcome and resumes from operation-bound receipt', async t => {
  const f = await fixture(t), plan = await f.preview(); f.loseReply();
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /TIMEOUT_OUTCOME_UNKNOWN/);
  const state = JSON.parse(await readFile(f.statePath, 'utf8'));
  assert.equal(state.status, 'INCOMPLETE');
  assert.equal(state.error, 'PURGE_NATIVE_TIMEOUT_OUTCOME_UNKNOWN');
  assert.equal((await applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native })).status, 'SCOPED_PURGED');
  assert.equal(f.applied, 1);
});

test('scope digest tampering and concurrent operation lock reject', async t => {
  const f = await fixture(t), plan = await f.preview();
  await assert.rejects(applyProjectPurge({ ...plan, projectId: 'changed' }, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /PLAN_INVALID/);
  await writeFile(`${f.statePath}.lock`, 'another writer');
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /EEXIST/);
  assert.equal(f.applied, 0);
});

test('PostgreSQL is read again after the other stages; historical receipt alone cannot finish', async t => {
  const f = await fixture(t), plan = await f.preview(); f.changeReadback();
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /READBACK_CHANGED/);
  assert.equal(JSON.parse(await readFile(f.statePath, 'utf8')).status, 'INCOMPLETE');
  assert.equal(f.applied, 1);
});
