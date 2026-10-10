import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, writeFile, readFile, link, rm } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { LatticeStore } from '../src/store.mjs';
import { previewProjectPurge, applyProjectPurge, statusProjectPurge } from '../src/project-purge.mjs';
import { runProjectPurgeCli } from '../src/project-purge-client.mjs';
import { externalPurgeInventory, projectPurgeReport } from '../src/project-purge-report.mjs';

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
  const survivorProject = store.createProject({ name: 'survivor', rootPath: survivor });
  store.database.close();
  const statePath = path.join(root, 'progress.json');
  const codeGraphCacheDirectory = path.join(root, 'code-graphs');
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
  const preview = () => previewProjectPurge({ projectId: project.id, databasePath, nativeBinary: path.join(root, 'native.exe'), statePath, codeGraphCacheDirectory }, { native });
  return { root, target, survivor, databasePath, statePath, project, survivorProject, codeGraphCacheDirectory, native, preview,
    get applied() { return applied; }, loseReply() { loseReply = true; }, changeReadback() { readbackChanged = true; }, block() { blockers = ['CROSS_SCOPE_RETAINED_REFERENCE']; } };
}

async function cache(f, projectId, sourceRoot) {
  const graph = { schema_version: 'lattice.control.code-graph.v1', source: 'GRAPHIFY', authority: 'DERIVED',
    commit: '1'.repeat(40), nodes: [], edges: [], project_id: projectId, source_root: sourceRoot,
    generated_at: '2026-10-10T00:00:00.000Z' };
  const digest = value => createHash('sha256').update(value).digest('hex');
  const root = path.join(f.codeGraphCacheDirectory, digest(`${projectId}\0${sourceRoot.toLocaleLowerCase()}`));
  await mkdir(path.join(root, 'runs'), { recursive: true });
  await writeFile(path.join(root, 'graph.json'), JSON.stringify({ cache_digest: digest(JSON.stringify(graph)), ...graph }));
  await writeFile(path.join(root, 'runs', 'analysis.txt'), `Derived cache for ${projectId}`);
  return root;
}

test('attestation policy and seal stay bound through preview, apply and readback', async t => {
  const f=await fixture(t), calls=[];
  const history={assurance:'ATTESTED_FROM_SEAL',epoch:1,sealDigest:'b'.repeat(64),identifiersAreAnonymous:false};
  const native=async(binary,request)=>{
    calls.push(request);
    const response=await f.native(binary,request);
    return ['READY','PURGED'].includes(response.status)?{...response,registryStrategy:'ATTESTED_EPOCH_V1',history,registrySealDigest:history.sealDigest}:response;
  };
  const plan=await previewProjectPurge({projectId:f.project.id,databasePath:f.databasePath,nativeBinary:path.join(f.root,'native.exe'),statePath:f.statePath,codeGraphCacheDirectory:f.codeGraphCacheDirectory,registryPolicy:'MINIMAL_ATTESTATION'},{native});
  const result=await applyProjectPurge(plan,{confirmDigest:plan.digest,maintenanceOffline:true,native});
  assert.equal(result.status,'SCOPED_PURGED');assert.deepEqual(result.report.history,history);
  assert.ok(calls.every(request=>request.registryPolicy==='MINIMAL_ATTESTATION'));
  assert.ok(calls.slice(1).every(request=>request.expectedScopeDigest===plan.postgres.scopeDigest));
  const readback=await statusProjectPurge(plan,{native:async(binary,request)=>({...await native(binary,request),registrySealDigest:'c'.repeat(64)})});
  assert.equal(readback.status,'INCOMPLETE');
});

test('coordinator resumes a lost PostgreSQL reply before rebuilding immutable Control receipts', { skip: process.platform !== 'win32' }, async t => {
  const f = await fixture(t), store = new LatticeStore(f.databasePath);
  const record = project => store.createInstallationReceipt({ projectId: project.id, component: 'synthetic', sourceCommitSha: 'a'.repeat(40), artifactPath: path.join(project.root_path, 'synthetic.exe'), artifactSha256: 'b'.repeat(64) });
  record(f.project);
  const retained = record(f.survivorProject).receipt;
  store.close();
  const plan = await f.preview();
  assert.equal(plan.status, 'READY');
  assert.equal(plan.sqlite.strategy, 'REBUILD_SURVIVORS_V1');
  f.loseReply();
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /TIMEOUT_OUTCOME_UNKNOWN/);
  assert.throws(() => new LatticeStore(f.databasePath), /MAINTENANCE_PENDING/);
  const result = await applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native });
  assert.equal(result.status, 'SCOPED_PURGED');
  assert.equal(f.applied, 1);
  const after = new LatticeStore(f.databasePath);
  try {
    assert.equal(after.getProject(f.project.id), null);
    assert.deepEqual(after.getInstallationReceipt(retained.id), retained);
    assert.throws(() => after.database.exec('DELETE FROM installation_receipts'), /append-only/);
  } finally { after.close(); }
  assert.equal((await statusProjectPurge(plan, { native: f.native })).status, 'SCOPED_PURGED');
});

test('coordinator purges all owned Control graph caches and preserves another project cache', async t => {
  const f = await fixture(t);
  const first = await cache(f, f.project.id, f.target);
  const second = await cache(f, f.project.id, path.join(f.root, 'other-checkout'));
  const retained = await cache(f, f.survivorProject.id, f.survivor);
  const original = await readFile(path.join(retained, 'graph.json'));
  const plan = await f.preview();
  assert.equal(plan.status, 'READY'); assert.equal(plan.codeGraph.roots.length, 2);
  const result = await applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native });
  assert.equal(result.status, 'SCOPED_PURGED'); assert.equal(result.codeGraph.complete, true);
  for (const root of [first, second]) await assert.rejects(readFile(path.join(root, 'graph.json')), { code: 'ENOENT' });
  assert.deepEqual(await readFile(path.join(retained, 'graph.json')), original);
  assert.equal(result.report.stages.find(stage => stage.kind === 'controlCodeGraph').status, 'VERIFIED_ABSENT');
  assert.equal(result.report.complete, false);
  await cache(f, f.project.id, path.join(f.root, 'new-checkout'));
  const readback = await statusProjectPurge(plan, { native: f.native });
  assert.equal(readback.status, 'INCOMPLETE'); assert.equal(readback.codeGraph.complete, false);
});

test('new graph cache after preview is rejected before PostgreSQL or SQLite deletion', async t => {
  const f = await fixture(t); await cache(f, f.project.id, f.target);
  const plan = await f.preview();
  await cache(f, f.project.id, path.join(f.root, 'new-checkout'));
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /PURGE_CODE_GRAPH_SCOPE_CHANGED/);
  assert.equal(f.applied, 0);
  const db = new LatticeStore(f.databasePath); assert.ok(db.getProject(f.project.id)); db.close();
});

test('unowned Control cache blocks the workflow instead of claiming discovery complete', async t => {
  const f = await fixture(t);
  await mkdir(path.join(f.codeGraphCacheDirectory, 'unknown'), { recursive: true });
  const plan = await f.preview();
  assert.equal(plan.status, 'BLOCKED'); assert.equal(plan.codeGraph.discovery, 'BLOCKED');
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /PURGE_BLOCKED/);
  assert.equal(f.applied, 0);
});

test('a shared cache located inside a project deletion root blocks before any deletion', async t => {
  const f = await fixture(t), codeGraphCacheDirectory = path.join(f.target, 'shared-cache');
  const retained = await cache({ ...f, codeGraphCacheDirectory }, f.survivorProject.id, f.survivor);
  const original = await readFile(path.join(retained, 'graph.json'));
  const plan = await previewProjectPurge({ projectId: f.project.id, databasePath: f.databasePath,
    nativeBinary: path.join(f.root, 'native.exe'), statePath: f.statePath, codeGraphCacheDirectory }, { native: f.native });
  assert.equal(plan.status, 'BLOCKED');
  assert.ok(plan.blockers.some(blocker => blocker.code === 'PURGE_CODE_GRAPH_SHARED_ROOT_OVERLAP'));
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /PURGE_BLOCKED/);
  assert.equal(f.applied, 0); assert.deepEqual(await readFile(path.join(retained, 'graph.json')), original);
});

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

test('online inventory preserves maintenance blockers and counts without allowing mutation', async t => {
  const f = await fixture(t), calls = [];
  const native = async (binary, request) => {
    calls.push(request.action);
    assert.equal(request.action, 'preview');
    return { ...await f.native(binary, request), status: 'BLOCKED', inventoryMode: 'READ_ONLY_SNAPSHOT',
      maintenanceExtensionInstalled: false, maintenanceStopped: false,
      blockers: [{ code: 'MAINTENANCE_EXTENSION_REQUIRED' }, { code: 'MAINTENANCE_OFFLINE_REQUIRED' }] };
  };
  const plan = await previewProjectPurge({ projectId: f.project.id, databasePath: f.databasePath,
    nativeBinary: path.join(f.root, 'native.exe'), statePath: f.statePath, codeGraphCacheDirectory: f.codeGraphCacheDirectory }, { native });
  assert.equal(plan.postgres.inventoryMode, 'READ_ONLY_SNAPSHOT');
  assert.equal(plan.postgres.maintenanceExtensionInstalled, false);
  assert.equal(plan.postgres.maintenanceStopped, false);
  assert.deepEqual(plan.postgres.counts, { projects: 1 });
  const result = await statusProjectPurge(plan, { native });
  assert.equal(result.status, 'BLOCKED');
  assert.deepEqual(result.report.blockers.map(item => item.code), ['MAINTENANCE_EXTENSION_REQUIRED', 'MAINTENANCE_OFFLINE_REQUIRED']);
  assert.ok(result.report.blockers.every(item => item.nextStep.includes('fresh preview')));
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native }), /PURGE_BLOCKED/);
  assert.deepEqual(calls, ['preview']);
  const db = new LatticeStore(f.databasePath); assert.ok(db.getProject(f.project.id)); db.close();
  assert.equal(await readFile(path.join(f.target, 'target.txt'), 'utf8'), 'target content');
  await assert.rejects(readFile(f.statePath), { code: 'ENOENT' });
});

test('filesystem drift is rejected before PostgreSQL deletion', async t => {
  const f = await fixture(t), plan = await f.preview();
  await writeFile(path.join(f.target, 'unapproved.txt'), 'new');
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /SCOPE_CHANGED/);
  assert.equal(f.applied, 0);
  const db = new LatticeStore(f.databasePath); assert.ok(db.getProject(f.project.id)); db.database.close();
});

test('hard links block inventory and post-preview apply before PostgreSQL or SQLite deletion', async t => {
  const f = await fixture(t), originalPlan = await f.preview();
  const original = path.join(f.target, 'target.txt');
  const alias = path.join(f.survivor, 'target-link.txt');
  await link(original, alias);
  const blocked = await f.preview();
  assert.equal(blocked.status, 'BLOCKED');
  assert.ok(blocked.blockers.includes('PURGE_FILE_HARDLINK_UNSUPPORTED'));
  await assert.rejects(applyProjectPurge(blocked, { confirmDigest: blocked.digest, maintenanceOffline: true, native: f.native }), /PURGE_BLOCKED/);
  await assert.rejects(applyProjectPurge(originalPlan, { confirmDigest: originalPlan.digest, maintenanceOffline: true, native: f.native }), /PURGE_FILE_HARDLINK_UNSUPPORTED/);
  assert.equal(f.applied, 0);
  const db = new LatticeStore(f.databasePath); assert.ok(db.getProject(f.project.id)); db.database.close();
  assert.equal(await readFile(original, 'utf8'), 'target content');
  assert.equal(await readFile(alias, 'utf8'), 'target content');
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

test('PostgreSQL-owned project can finish the local scope after an earlier catalog-only removal', async t => {
  const f = await fixture(t);
  const db = new LatticeStore(f.databasePath);
  db.database.prepare('DELETE FROM projects WHERE id=?').run(f.project.id); db.close();
  const plan = await f.preview();
  assert.equal(plan.sqlite.catalogState, 'ABSENT');
  assert.equal(plan.sqlite.authoritativeProject.scopeDigest, plan.postgres.scopeDigest);
  const result = await applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native });
  assert.equal(result.status, 'SCOPED_PURGED'); assert.equal(f.applied, 1);
  assert.equal(result.report.overallStatus, 'PARTIAL'); assert.equal(result.report.complete, false);
  assert.equal((await applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native })).status, 'SCOPED_PURGED');
  assert.equal(f.applied, 1);
});

test('unsafe filesystem scope yields an inspectable blocked plan without deleting PostgreSQL', async t => {
  const f = await fixture(t);
  const plan = await previewProjectPurge({ projectId: f.project.id, databasePath: f.databasePath,
    nativeBinary: path.join(f.root, 'native.exe'), statePath: f.statePath, codeGraphCacheDirectory: f.codeGraphCacheDirectory, protectedRoots: [f.target] }, { native: f.native });
  assert.equal(plan.status, 'BLOCKED');
  assert.ok(plan.blockers.includes('PURGE_FILE_PROTECTED_ROOT'));
  const readback = await statusProjectPurge(plan, { native: f.native });
  assert.equal(readback.report.overallStatus, 'BLOCKED');
  assert.ok(readback.report.blockers[0].nextStep);
  await assert.rejects(applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native: f.native }), /PURGE_BLOCKED/);
  assert.equal(f.applied, 0);
});

test('external resource inventory is bounded and never accepts claimed deletion or absence', async t => {
  const f = await fixture(t), plan = await f.preview();
  assert.throws(() => externalPurgeInventory([{ kind: 'codex', reference: 'thread-1', source: 'native list', status: 'DELETED' }]), /INVENTORY_INVALID/);
  const declared = { kind: 'codex', reference: 'thread-1', source: 'native list' };
  assert.throws(() => externalPurgeInventory([declared, declared]), /INVENTORY_DUPLICATE/);
  plan.externalInventory = externalPurgeInventory([declared]);
  plan.externalInventory[0].discovery = 'COMPLETE';
  plan.externalInventory[0].resources[0].status = 'DELETED';
  const report = projectPurgeReport(plan, { status: 'SCOPED_PURGED' });
  assert.equal(report.complete, false); assert.equal(report.overallStatus, 'PARTIAL');
  assert.equal(report.external[0].discovery, 'NOT_VERIFIED');
  assert.equal(report.external[0].resources[0].status, 'NOT_VERIFIED');
  assert.equal(report.remaining.length, 7);
  assert.equal(report.latticeScopeComplete, false);
  for (const kind of ['graphify', 'botLifecycle', 'maintenance']) {
    const scope = report.external.find(item => item.kind === kind);
    assert.equal(scope.ownership, 'LATTICE');
    assert.equal(scope.disposition, 'PRODUCT_WORK_REQUIRED');
    assert.equal(report.remaining.find(item => item.kind === kind).reason, 'LATTICE_OWNED_CLEANUP_NOT_IMPLEMENTED');
  }
  assert.equal(report.external.find(item => item.kind === 'backups').ownership, 'MIXED');
  assert.match(report.explanation, /Additional LATTICE-owned data/);
});

test('native lifecycle inventory distinguishes absent, unreachable, observed and unproven ownership', async t => {
  const f = await fixture(t), plan = await f.preview();
  const absent = { schema: 'lattice.project-purge.bot-inventory.v1', ownership: 'LATTICE',
    discovery: 'VERIFIED_ABSENT', databaseCommitment: 'd'.repeat(64), databasePresent: false,
    identityBinding: 'NOT_APPLICABLE', scope: 'VERIFIED_DEDICATED_BOT_SERVICE', counts: { roles: 0, events: 0 } };
  const report = observation => projectPurgeReport(plan, { status: 'SCOPED_PURGED', postgres: { relatedStores: { botLifecycle: observation } } });
  assert.equal(report(absent).external.find(scope => scope.kind === 'botLifecycle').disposition, 'NO_DATA_IN_CONFIGURED_STORE');
  assert.equal(report(absent).remaining.some(scope => scope.kind === 'botLifecycle'), false);
  assert.equal(report(absent).latticeScopeComplete, false);
  for (const invalid of [{ ...absent, counts: { roles: 1, events: 0 } }, { ...absent, databasePresent: true }, { ...absent, scope: 'CALLER_CLAIM' }]) {
    assert.equal(report(invalid).remaining.some(scope => scope.kind === 'botLifecycle'), true);
  }
  const unknown = { ...absent, discovery: 'NOT_VERIFIED', reason: 'BOT_LIFECYCLE_DATABASE_UNAVAILABLE' };
  assert.equal(report(unknown).external.find(scope => scope.kind === 'botLifecycle').observation.reason, unknown.reason);
  const observed = { ...absent, discovery: 'OBSERVED', databasePresent: true, snapshotDigest: 'e'.repeat(64),
    counts: { roles: 1, events: 4 }, exactKeyMatches: { roles: 0, events: 0 }, identityBinding: 'TEXT_KEY_ONLY_REGISTRY_BINDING_NOT_PROVEN' };
  assert.equal(report(observed).external.find(scope => scope.kind === 'botLifecycle').disposition, 'PRODUCT_WORK_REQUIRED');
});

test('Runtime source cache is purged with an empty Git sentinel and survivor files unchanged', async t => {
  const f=await fixture(t), work=path.join(f.root,'runtime-graph'), sourceKey='c'.repeat(64);
  const a=path.join(work,'sources',sourceKey), b=path.join(work,'sources','d'.repeat(64));
  const snapshot=path.join(a,'1'.repeat(40),'snapshots','synthetic');
  await mkdir(path.join(snapshot,'.git'),{recursive:true}); await mkdir(b,{recursive:true});
  await writeFile(path.join(snapshot,'source.txt'),'target code snapshot'); await writeFile(path.join(b,'keep.txt'),'survivor graph');
  const native=async(binary,request)=>({...await f.native(binary,request),
    ...(request.action==='preview'?{runtimeGraphSource:{sourceKey,binding:'REGISTRY_CANONICAL_PATH'}}:{}),
    relatedStores:{runtimeGraph:{schema:'lattice.project-purge.graph-inventory.v1',ownership:'LATTICE',
      discovery:'VERIFIED_EMPTY',scope:'VERIFIED_MAIN_STORE_MEMORY',snapshotDigest:'e'.repeat(64),counts:{}}}});
  const options={projectId:f.project.id,databasePath:f.databasePath,nativeBinary:path.join(f.root,'native.exe'),
    statePath:f.statePath,codeGraphCacheDirectory:f.codeGraphCacheDirectory,runtimeGraphWorkDirectory:work,botService:null};
  const plan=await previewProjectPurge(options,{native});
  assert.equal(plan.status,'READY',JSON.stringify(plan.blockers)); assert.deepEqual(plan.runtimeGraph.roots,[a]);
  const result=await applyProjectPurge(plan,{confirmDigest:plan.digest,maintenanceOffline:true,native});
  assert.equal(result.runtimeGraph.complete,true);
  assert.equal(result.report.remaining.some(item=>item.kind==='graphify'),false);
  await assert.rejects(readFile(path.join(snapshot,'source.txt')),/ENOENT/);
  assert.equal(await readFile(path.join(b,'keep.txt'),'utf8'),'survivor graph');
  assert.equal((await statusProjectPurge(plan,{native})).runtimeGraph.complete,true);
});

test('unknown legacy Runtime directories and nested real Git metadata block before erasure', async t => {
  const f=await fixture(t), work=path.join(f.root,'runtime-graph'), sourceKey='c'.repeat(64);
  const sentinel=path.join(work,'sources',sourceKey,'snapshot','.git');
  await mkdir(sentinel,{recursive:true}); await writeFile(path.join(sentinel,'config'),'real metadata');
  const native=async(binary,request)=>({...await f.native(binary,request),runtimeGraphSource:{sourceKey,binding:'REGISTRY_CANONICAL_PATH'}});
  const options={projectId:f.project.id,databasePath:f.databasePath,nativeBinary:path.join(f.root,'native.exe'),
    statePath:f.statePath,codeGraphCacheDirectory:f.codeGraphCacheDirectory,runtimeGraphWorkDirectory:work,botService:null};
  const plan=await previewProjectPurge(options,{native});
  assert.equal(plan.status,'BLOCKED'); assert.ok(plan.blockers.includes('PURGE_FILE_NESTED_REPOSITORY'));
  await mkdir(path.join(work,'legacy-layout'));
  const legacy=await previewProjectPurge(options,{native});
  assert.ok(legacy.blockers.some(item=>item.code==='PURGE_CODE_GRAPH_LEGACY_WORK_ROOT_UNCLASSIFIED'));
  assert.equal(f.applied,0);
});

test('standard CLI rejects unknown, duplicate and incomplete deletion arguments before side effects', async () => {
  const never = async () => assert.fail('unexpected side effect');
  for (const args of [
    ['apply', '--plan', 'missing'], ['resume', '--plan', 'missing', '--confirm', 'a'.repeat(64)],
    ['apply', '--plan', 'missing', '--confirm', 'a'.repeat(64), '--maintenance-offline', '--force'],
    ['status', '--plan', 'one', '--plan', 'two'], ['status', '--plan'], ['purge-all'],
  ]) assert.equal((await runProjectPurgeCli(args, { preview: never, apply: never, status: never })).exitCode, 1);
});

test('standard CLI inventory saves one plan; resume delegates exact plan and verify rejects partial completion', async t => {
  const f = await fixture(t), input = path.join(f.root, 'input.json'), planPath = path.join(f.root, 'plan.json');
  await writeFile(input, JSON.stringify({ projectId: f.project.id, databasePath: f.databasePath,
    nativeBinary: path.join(f.root, 'native.exe'), statePath: f.statePath, codeGraphCacheDirectory: f.codeGraphCacheDirectory }));
  const services = { preview: options => previewProjectPurge(options, { native: f.native }),
    apply: (plan, options) => applyProjectPurge(plan, { ...options, native: f.native }),
    status: plan => statusProjectPurge(plan, { native: f.native }) };
  assert.equal((await runProjectPurgeCli(['inventory', '--input', input, '--plan', planPath], services)).exitCode, 0);
  assert.equal((await runProjectPurgeCli(['preview', '--input', input, '--plan', planPath], services)).exitCode, 1);
  const plan = JSON.parse(await readFile(planPath, 'utf8'));
  const applied = await runProjectPurgeCli(['resume', '--plan', planPath, '--confirm', plan.digest, '--maintenance-offline'], services);
  assert.equal(applied.exitCode, 0);
  const verified = await runProjectPurgeCli(['verify', '--plan', planPath], services);
  assert.equal(verified.exitCode, 2); assert.equal(JSON.parse(verified.output).report.complete, false);
  assert.equal(JSON.parse(verified.output).status, 'SCOPED_PURGED'); assert.equal(f.applied, 1);
});
