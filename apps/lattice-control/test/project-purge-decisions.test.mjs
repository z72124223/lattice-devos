import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { mkdtemp, rm } from 'node:fs/promises';
import { renameSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { LatticeStore, decisionStateAfterPurge } from '../src/store.mjs';
import { decisionRequestKey, classifyDecisionOwnership } from '../src/decision-project-ownership.mjs';
import { previewProjectPurgeSqlite, beginProjectPurgeSqlite, readbackProjectPurgeSqlite } from '../src/project-purge-sqlite.mjs';

const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const source = { kind: 'user_confirmation', reference: 'thread:synthetic-decision-purge/turn:1' };
const here = import.meta.dirname;
async function fixture(t) {
  const directory = await mkdtemp(path.join(here, '.project-purge-decisions-'));
  const databasePath = path.join(directory, 'control.db'), handles = [];
  const open = () => {
    const store = new LatticeStore(databasePath), close = store.close.bind(store);
    let closed = false; store.close = () => { if (!closed) { close(); closed = true; } };
    handles.push(store); return store;
  };
  const store = open();
  t.after(async () => {
    for (const handle of handles) handle.close();
    assert.equal(path.dirname(path.resolve(directory)), here);
    assert.ok(path.basename(directory).startsWith('.project-purge-decisions-'));
    await rm(directory, { recursive: true, force: true });
  });
  const a = store.createProject({ name: 'Synthetic A', rootPath: path.join(directory, 'a') });
  const b = store.createProject({ name: 'Synthetic B', rootPath: path.join(directory, 'b') });
  return { store, open, a, b, options: { databasePath, projectId: a.id, canonicalPath: a.root_path, operationDigest: hash(randomUUID()) } };
}
function input(store, owner, overrides = {}) {
  const state = store.decisionStateIdentity();
  return { scope: 'scope-a', subject: 'subject-1', content: 'Synthetic decision', rationale: 'Synthetic verification',
    source, clientRequestId: randomUUID(), expectedRevision: state.revision, expectedDigest: state.digest,
    ...(owner === undefined ? {} : { owner }), ...overrides };
}
function record(store, owner, overrides) { return store.recordDecision(input(store, owner, overrides)); }
const owner = project => ({ kind: 'PROJECT', projectId: project.id });
const globalOwner = { kind: 'GLOBAL' };
function data(store) {
  return Object.fromEntries(store.database.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all()
    .map(({ name }) => [name, store.database.prepare('SELECT rowid,* FROM "' + name + '" ORDER BY rowid').all()]));
}

test('first-write failure after extension installation rolls back to the exact original schema', async t => {
  const f = await fixture(t), before = hash(data(f.store));
  const schema = f.store.database.prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name").all();
  const prepare = f.store.database.prepare;
  f.store.database.prepare = function(sql, ...args) {
    if (/INSERT INTO decisions\s*\(/u.test(sql)) throw new Error('SYNTHETIC_INSERT_FAILURE');
    return prepare.call(this, sql, ...args);
  };
  assert.throws(() => record(f.store,owner(f.a)),/SYNTHETIC_INSERT_FAILURE/);
  f.store.database.prepare = prepare;
  f.store.close();
  const reopened = f.open();
  assert.equal(hash(data(reopened)),before);
  assert.deepEqual(reopened.database.prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name").all(),schema);
});

// A valid pre-extension v7 record, created using the original row/digest format.
// This test helper never drops the original immutable triggers or edits a receipt.
function legacyRecord(store) {
  const request = input(store, undefined, { scope: 'legacy-freeform' });
  const prior = store.database.prepare('SELECT * FROM decision_state').get();
  const createdAt = new Date(Math.max(Date.now(), Date.parse(prior.updated_at) + 1)).toISOString();
  const row = { id: randomUUID(), scope: request.scope, subject: request.subject, content: request.content, rationale: request.rationale,
    source_kind: source.kind, source_reference: source.reference, status: 'current', supersedes_decision_id: null,
    client_request_id: request.clientRequestId,
    request_digest: hash({ operation: 'record', scope: request.scope, subject: request.subject, content: request.content,
      rationale: request.rationale, source, supersedes_decision_id: null, expected_revision: request.expectedRevision, expected_digest: request.expectedDigest }),
    created_at: createdAt };
  store.database.exec('BEGIN IMMEDIATE;');
  store.database.prepare('INSERT INTO decisions (' + Object.keys(row).join(',') + ') VALUES (' + Object.keys(row).map(() => '?').join(',') + ')').run(...Object.values(row));
  const next = decisionStateAfterPurge(store.database.prepare('SELECT * FROM decisions').all(), prior, createdAt);
  store.database.prepare('UPDATE decision_state SET revision=?,digest=?,updated_at=? WHERE slot=?').run(next.revision,next.digest,next.updated_at,next.slot);
  store.database.exec('COMMIT;');
  return request;
}

test('new decisions require explicit valid ownership and one owner for the entire scope', async t => {
  const f = await fixture(t), before = hash(data(f.store));
  assert.throws(() => f.store.recordDecision(input(f.store)), /DECISION_OWNERSHIP_REQUIRED/);
  assert.throws(() => record(f.store, { kind: 'PROJECT', projectId: 'missing-project' }), /DECISION_OWNER_PROJECT_MISSING/);
  assert.equal(hash(data(f.store)), before);
  const first = record(f.store, owner(f.a));
  const afterFirst = hash(data(f.store));
  for (const otherOwner of [owner(f.b), globalOwner]) {
    assert.throws(() => record(f.store, otherOwner, { subject: 'another-subject' }), /DECISION_SCOPE_OWNER_CONFLICT/);
    assert.equal(hash(data(f.store)), afterFirst);
  }
  assert.equal(record(f.store, owner(f.a), { subject: 'another-subject' }).changed, true);
  assert.throws(() => f.store.database.prepare('DELETE FROM projects WHERE id=?').run(f.a.id), /FOREIGN KEY/);
  assert.throws(() => f.store.database.prepare("UPDATE decision_project_ownership SET owner_kind='GLOBAL', project_id=NULL WHERE decision_id=?").run(first.decision.id), /immutable/);
  assert.throws(() => f.store.database.exec("INSERT INTO decisions SELECT 'new-id',scope,subject,content,rationale,source_kind,source_reference,status,NULL,'new-request',request_digest,created_at FROM decisions LIMIT 1"), /DECISION_OWNERSHIP_REQUIRED/);
  assert.throws(() => f.store.database.exec("INSERT INTO decision_project_ownership(decision_id,owner_kind) VALUES ('missing-decision','GLOBAL')"), /FOREIGN KEY/);
  assert.throws(() => f.store.database.exec("INSERT INTO decision_project_ownership(decision_id,owner_kind) VALUES (NULL,'GLOBAL')"), /NOT NULL/);
  assert.throws(() => f.store.database.exec("INSERT INTO decision_request_tombstones(request_digest) VALUES (NULL)"), /NOT NULL/);
});

test('legacy rows remain replayable but cannot be adopted, guessed or silently assigned GLOBAL', async t => {
  const f = await fixture(t), legacy = legacyRecord(f.store);
  const before = hash(data(f.store));
  assert.equal(f.store.recordDecision(legacy).changed, false);
  assert.equal(hash(data(f.store)), before);
  assert.throws(() => record(f.store, owner(f.a), { scope: legacy.scope, subject: 'new-subject' }), /LEGACY_SCOPE_UNATTRIBUTABLE/);
  assert.equal(hash(data(f.store)), before);
  record(f.store, globalOwner, { scope: 'explicit-global' });
  const plan = previewProjectPurgeSqlite(f.options);
  assert.ok(plan.blockers.includes('SQLITE_DECISION_OWNERSHIP_UNATTRIBUTABLE'));
  assert.equal(plan.unattributedDecisionRows, 1);
  assert.equal(plan.unattributedDecisionScopes[0].count, 1);
  assert.match(plan.unattributedDecisionScopes[0].scopeDigest, /^[a-f0-9]{64}$/);
  assert.ok(!JSON.stringify(plan.unattributedDecisionScopes).includes(legacy.scope));
  const beforeApply = hash(data(f.store));
  assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_BLOCKED/);
  assert.equal(hash(data(f.store)), beforeApply);
});

test('partial or mixed ownership blocks a whole scope instead of selecting a subset', () => {
  const rows = { decisions: [{ id:'a', scope:'shared' }, { id:'b', scope:'shared' }],
    decision_project_ownership: [{ decision_id:'a', owner_kind:'PROJECT', project_id:'project-a' }] };
  assert.deepEqual(classifyDecisionOwnership(rows, 'project-a').blockers, ['SQLITE_DECISION_OWNERSHIP_UNATTRIBUTABLE']);
  rows.decision_project_ownership.push({ decision_id:'b', owner_kind:'PROJECT', project_id:'project-b' });
  const mixed = classifyDecisionOwnership(rows, 'project-a');
  assert.equal(mixed.selected.size, 0);
  assert.deepEqual(mixed.blockers, ['SQLITE_DECISION_SCOPE_MIXED_OWNERS']);
});

test('owned scope rebuild preserves B/GLOBAL bytes and rowids, rejects stale global packets and retires request IDs', { skip: process.platform !== 'win32' }, async t => {
  const f = await fixture(t);
  const a1Input = input(f.store, owner(f.a)), a1 = f.store.recordDecision(a1Input);
  record(f.store, owner(f.a), { supersedesDecisionId:a1.decision.id });
  const a2 = record(f.store, owner(f.a), { subject:'subject-2' });
  record(f.store, owner(f.a), { subject:'subject-2', supersedesDecisionId:a2.decision.id });
  const b1 = record(f.store, owner(f.b), { scope:'scope-b' });
  const b2Input = input(f.store, owner(f.b), { scope:'scope-b', supersedesDecisionId:b1.decision.id });
  f.store.recordDecision(b2Input);
  record(f.store, globalOwner, { scope:'global' });
  const before = data(f.store), state = f.store.decisionStateIdentity();
  const schema = f.store.database.prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name").all();
  const plan = previewProjectPurgeSqlite(f.options);
  assert.deepEqual(plan.blockers, []);
  assert.equal(plan.counts.decisions, 4);
  assert.equal(plan.strategy, 'REBUILD_SURVIVORS_V1');
  assert.equal(plan.decisionTransition.beforeRevision, 7);
  assert.equal(plan.decisionTransition.afterRevision, 3);
  assert.equal(plan.unattributedDecisionRows, 0);
  f.store.close();
  beginProjectPurgeSqlite(plan).commit();
  assert.equal(readbackProjectPurgeSqlite(plan).complete, true);
  const after = f.open(), actual = data(after);
  assert.deepEqual(actual.decisions, before.decisions.filter(row => row.scope !== 'scope-a'));
  assert.deepEqual(actual.decision_project_ownership, before.decision_project_ownership.filter(row => row.project_id !== f.a.id));
  assert.deepEqual(after.database.prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name").all(), schema);
  assert.equal(after.getCurrentDecisionsPacket({ scope:'scope-a',limit:10 }).decisions.length,0);
  assert.throws(() => after.readDecision({ decisionId:b1.decision.id,maxDepth:10,expectedRevision:state.revision,expectedDigest:state.digest }), { code:'DECISION_REVISION_MISMATCH' });
  const fresh = after.decisionStateIdentity();
  const read = after.readDecision({ decisionId:b1.decision.id,maxDepth:10,expectedRevision:fresh.revision,expectedDigest:fresh.digest });
  assert.equal(read.decision.id,b1.decision.id);
  const replay = after.recordDecision(b2Input);
  assert.equal(replay.changed,false);
  assert.equal(replay.revision,fresh.revision); assert.equal(replay.digest,fresh.digest);
  assert.deepEqual(Object.keys(replay.decision).sort(),['id','scope','subject','content','rationale','source','status','supersedes_decision_id','created_at'].sort());
  for (const changedOwner of [owner(f.b),globalOwner]) {
    assert.throws(() => record(after, changedOwner, { scope:'new-scope',content:'Changed payload',clientRequestId:a1Input.clientRequestId }), /DECISION_REQUEST_RETIRED/);
  }
  assert.ok(actual.decision_request_tombstones.some(row => row.request_digest === decisionRequestKey(a1Input.clientRequestId)));
  assert.equal(actual.decision_purge_receipts.length,1);
  const receipt = actual.decision_purge_receipts[0];
  assert.equal(receipt.before_digest,state.digest);
  assert.equal(receipt.after_digest,fresh.digest);
  assert.equal(receipt.operation_digest,plan.operationDigest);
  assert.ok(!JSON.stringify(receipt).includes(f.a.id));
  assert.throws(() => after.database.exec('DELETE FROM decisions'), /retained permanently/);
  assert.throws(() => after.database.exec('DELETE FROM decision_purge_receipts'), /immutable/);
  assert.throws(() => after.database.exec('DELETE FROM decision_request_tombstones'), /immutable/);
  after.close();
  const retry = beginProjectPurgeSqlite(plan); assert.equal(retry.alreadyApplied,true); retry.commit();
});

test('an improperly restored retired row cannot bypass tombstone checks through exact replay', async t => {
  const f = await fixture(t), request = input(f.store,owner(f.a));
  f.store.recordDecision(request);
  f.store.database.prepare('INSERT INTO decision_request_tombstones(request_digest) VALUES (?)').run(decisionRequestKey(request.clientRequestId));
  assert.throws(() => f.store.recordDecision(request), /DECISION_REQUEST_RETIRED/);
  assert.throws(() => previewProjectPurgeSqlite(f.options), /PURGE_SQLITE_RETIRED_DECISION_PRESENT/);
});

test('a statement prepared on a pre-extension connection sees the new ownership guard', async t => {
  const f = await fixture(t), oldConnection = f.open();
  const prepared = oldConnection.database.prepare(`INSERT INTO decisions(id,scope,subject,content,rationale,source_kind,source_reference,
    status,supersedes_decision_id,client_request_id,request_digest,created_at) VALUES (?,?,?,?,?,?,?,'current',NULL,?,?,?)`);
  record(f.store,globalOwner);
  const before = hash(data(f.store));
  assert.throws(() => prepared.run(randomUUID(),'legacy-prepared','subject','Synthetic','Verification',source.kind,source.reference,
    randomUUID(),'0'.repeat(64),new Date().toISOString()), /DECISION_OWNERSHIP_REQUIRED/);
  assert.equal(hash(data(f.store)),before);
});

test('surviving project and GLOBAL reverse references to a target decision block before deletion', async t => {
  for (const kind of ['PROJECT','GLOBAL']) {
    const f = await fixture(t), own = record(f.store,owner(f.a));
    record(f.store,kind === 'PROJECT' ? owner(f.b) : globalOwner,{ scope:'survivor',content:'Reference ' + own.decision.id });
    const before = hash(data(f.store)), plan = previewProjectPurgeSqlite(f.options);
    assert.ok(plan.blockers.includes('SQLITE_RETAINED_REFERENCE:decisions'));
    assert.throws(() => beginProjectPurgeSqlite(plan),/PURGE_SQLITE_BLOCKED/);
    assert.equal(hash(data(f.store)),before);
  }
});

test('decision rebuild resumes both sides of replacement with one identical minimal receipt', { skip: process.platform !== 'win32' }, async t => {
  for (const afterReplace of [false,true]) {
    const f = await fixture(t);
    record(f.store,owner(f.a)); record(f.store,owner(f.b),{scope:'scope-b'});
    const plan = previewProjectPurgeSqlite(f.options); f.store.close();
    assert.throws(() => beginProjectPurgeSqlite(plan,{replace(from,to) {
      if (afterReplace) renameSync(from,to);
      throw new Error('SYNTHETIC_INTERRUPTION');
    }}).commit(),/SYNTHETIC_INTERRUPTION/);
    assert.equal(readbackProjectPurgeSqlite(plan).complete,false);
    beginProjectPurgeSqlite(plan).commit();
    assert.equal(readbackProjectPurgeSqlite(plan).complete,true);
    const after = f.open();
    assert.equal(after.database.prepare('SELECT count(*) AS count FROM decision_purge_receipts').get().count,1);
    assert.equal(after.database.prepare('SELECT count(*) AS count FROM decision_request_tombstones').get().count,1);
    assert.equal(after.decisionStateIdentity().revision,1);
  }
});

test('changed ownership schema fails before creating a purge plan or writing decisions', async t => {
  const f = await fixture(t); record(f.store,owner(f.a));
  f.store.database.exec('DROP TRIGGER decision_ownership_insert_required;');
  assert.throws(() => previewProjectPurgeSqlite(f.options), /schema profile/);
  assert.throws(() => record(f.store,owner(f.a),{subject:'new-subject'}), /schema profile/);
});
