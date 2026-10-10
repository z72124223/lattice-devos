import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { mkdtemp, rm, readFile, writeFile, link } from 'node:fs/promises';
import { renameSync, existsSync } from 'node:fs';
import { DatabaseSync } from 'node:sqlite';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { LatticeStore, validateControlSchemaProfile } from '../src/store.mjs';
import { previewProjectPurgeSqlite, beginProjectPurgeSqlite, readbackProjectPurgeSqlite } from '../src/project-purge-sqlite.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const timestamp = '2026-10-09T00:00:00.000Z';
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
function databaseRowsDigest(databasePath) {
  const db = new DatabaseSync(databasePath, { readOnly: true });
  try { return hash(rows(db)); } finally { db.close(); }
}
function rows(db) {
  const data = {};
  for (const { name } of db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all()) {
    data[name] = db.prepare(`SELECT * FROM "${name.replaceAll('"', '""')}"`).all()
      .sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b), 'en'));
  }
  return data;
}

async function fixture(t) {
  const directory = await mkdtemp(path.join(here, '.project-purge-sqlite-fixture-'));
  const databasePath = path.join(directory, 'synthetic-control.db');
  const store = new LatticeStore(databasePath);
  const originalClose = store.close.bind(store);
  let closed = false;
  store.close = () => { if (!closed) { originalClose(); closed = true; } };
  const stores = [store];
  t.after(async () => {
    for (const handle of stores) handle.close();
    assert.equal(path.dirname(path.resolve(directory)), here);
    assert.ok(path.basename(directory).startsWith('.project-purge-sqlite-fixture-'));
    await rm(directory, { recursive: true, force: true });
  });
  const own = store.createProject({ name: 'Synthetic A', rootPath: path.join(directory, 'project-a') });
  const other = store.createProject({ name: 'Synthetic B', rootPath: path.join(directory, 'project-b') });
  const workA = store.createWorkItem({ projectId: own.id, title: 'Own parent', objective: 'Synthetic parent' });
  const workB = store.createWorkItem({ projectId: own.id, title: 'Own child', objective: 'Synthetic child' });
  const otherWork = store.createWorkItem({ projectId: other.id, title: 'Other work', objective: 'Keep this work' });
  store.database.prepare("INSERT INTO work_item_relations(work_item_id,parent_work_item_id,blocker_status) VALUES (?,?,'clear')").run(workB.id, workA.id);
  store.database.prepare('INSERT INTO work_item_dependencies(work_item_id,depends_on_work_item_id,created_at) VALUES (?,?,?)').run(workB.id, workA.id, timestamp);
  const observations = [];
  for (const project of [own, other]) {
    const id = randomUUID(); observations.push(id);
    store.database.prepare('INSERT INTO project_registration_details(project_id,canonical_path,registered_at,refreshed_at) VALUES (?,?,?,?)').run(project.id, project.root_path, timestamp, timestamp);
    store.database.prepare('INSERT INTO project_registration_claims(canonical_path,generation,claimed_at) VALUES (?,1,?)').run(project.root_path, timestamp);
    store.database.prepare(`INSERT INTO project_observations(id,project_id,git_status,git_observed_at,git_failures_json,rule_status,rules_observed_at,missing_rules_json,rule_failures_json)
      VALUES (?,?,'complete',?,'[]','complete',?,'[]','[]')`).run(id, project.id, timestamp, timestamp);
    store.database.prepare("INSERT INTO project_git_remotes(observation_id,name,direction,url_sanitized,credentials_redacted) VALUES (?,'origin','fetch','https://example.invalid/synthetic',0)").run(id);
    store.database.prepare("INSERT INTO project_rule_documents(observation_id,relative_path,sha256,purpose,observed_at) VALUES (?,'AGENTS.md',?,'synthetic',?)").run(id, 'a'.repeat(64), timestamp);
  }
  return { store, stores, own, other, workA, workB, otherWork, observations, options: { databasePath, projectId: own.id, canonicalPath: own.root_path } };
}

function otherSnapshot(f) {
  const all = rows(f.store.database), observationId = f.observations[1];
  return hash({
    projects: all.projects.filter(row => row.id === f.other.id),
    work: all.work_items.filter(row => row.project_id === f.other.id),
    events: all.work_events.filter(row => row.work_item_id === f.otherWork.id),
    registrations: all.project_registration_details.filter(row => row.project_id === f.other.id),
    claims: all.project_registration_claims.filter(row => row.canonical_path === f.other.root_path),
    observations: all.project_observations.filter(row => row.project_id === f.other.id),
    remotes: all.project_git_remotes.filter(row => row.observation_id === observationId),
    rules: all.project_rule_documents.filter(row => row.observation_id === observationId),
    decisions: all.decisions,
    decisionState: all.decision_state,
  });
}

function removeCatalogProject(f, { retainClaim = false } = {}) {
  const db = f.store.database;
  db.prepare('DELETE FROM work_item_dependencies WHERE work_item_id IN (SELECT id FROM work_items WHERE project_id=?)').run(f.own.id);
  db.prepare('DELETE FROM work_item_relations WHERE work_item_id IN (SELECT id FROM work_items WHERE project_id=?)').run(f.own.id);
  db.prepare('DELETE FROM projects WHERE id=?').run(f.own.id);
  if (!retainClaim) db.prepare('DELETE FROM project_registration_claims WHERE canonical_path=?').run(f.own.root_path);
}

function authoritativeOptions(f) {
  return { ...f.options, authoritativeProject: {
    source: 'POSTGRES_PREVIEW', projectId: f.own.id, canonicalPath: f.own.root_path, scopeDigest: 'c'.repeat(64),
  } };
}

test('real LatticeStore deletion cascades owned rows, handles internal RESTRICT edges, and preserves other-project hash', async t => {
  const f = await fixture(t), beforeOther = otherSnapshot(f);
  const plan = previewProjectPurgeSqlite(f.options);
  assert.deepEqual(plan.blockers, []);
  assert.equal(plan.catalogState, 'PRESENT');
  assert.equal(plan.authoritativeProject, null);
  assert.equal(plan.counts.projects, 1);
  assert.equal(plan.counts.work_items, 2);
  assert.equal(plan.counts.work_events, 2);
  assert.equal(plan.counts.project_observations, 1);
  assert.equal(plan.counts.project_git_remotes, 1);
  assert.equal(plan.counts.project_rule_documents, 1);
  assert.deepEqual(plan.protectedRoots, [f.other.root_path]);
  const transaction = beginProjectPurgeSqlite(plan);
  assert.equal(transaction.alreadyApplied, false);
  assert.ok(f.store.getProject(f.own.id));
  transaction.commit();
  assert.equal(f.store.getProject(f.own.id), null);
  assert.equal(otherSnapshot(f), beforeOther);
  assert.deepEqual(f.store.database.prepare('PRAGMA foreign_key_check').all(), []);
  assert.doesNotThrow(() => validateControlSchemaProfile(f.store.database));
  assert.equal(readbackProjectPurgeSqlite(plan).complete, true);
  const retry = beginProjectPurgeSqlite(plan);
  assert.equal(retry.alreadyApplied, true); retry.commit();
  assert.equal(readbackProjectPurgeSqlite(plan).complete, true);
});

test('rolling back the held transaction leaves every row untouched', async t => {
  const f = await fixture(t), before = hash(rows(f.store.database));
  beginProjectPurgeSqlite(previewProjectPurgeSqlite(f.options)).rollback();
  assert.equal(hash(rows(f.store.database)), before);
});

test('project ID/path mismatch and any post-preview mutation fail before deletion', async t => {
  const f = await fixture(t);
  assert.throws(() => previewProjectPurgeSqlite({ ...f.options, projectId: f.other.id }), /PURGE_SQLITE_PROJECT_IDENTITY/);
  assert.throws(() => previewProjectPurgeSqlite({ ...f.options, canonicalPath: f.other.root_path }), /PURGE_SQLITE_PROJECT_IDENTITY/);
  const plan = previewProjectPurgeSqlite(f.options);
  f.store.appendEvent(f.otherWork.id, 'synthetic-change', { text: 'new unrelated event' });
  const before = hash(rows(f.store.database));
  assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_STALE_SCOPE/);
  assert.equal(hash(rows(f.store.database)), before);
});

test('incoming cross-project parent and dependency references block; outgoing owned edges can be removed', async t => {
  const f = await fixture(t);
  f.store.database.prepare("INSERT INTO work_item_relations(work_item_id,parent_work_item_id,blocker_status) VALUES (?,?,'clear')").run(f.otherWork.id, f.workA.id);
  f.store.database.prepare('INSERT INTO work_item_dependencies(work_item_id,depends_on_work_item_id,created_at) VALUES (?,?,?)').run(f.otherWork.id, f.workA.id, timestamp);
  const before = hash(rows(f.store.database)), plan = previewProjectPurgeSqlite(f.options);
  assert.ok(plan.blockers.includes('SQLITE_RETAINED_REFERENCE:work_item_relations'));
  assert.ok(plan.blockers.includes('SQLITE_RETAINED_REFERENCE:work_item_dependencies'));
  assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_BLOCKED/);
  assert.equal(hash(rows(f.store.database)), before);
  const outgoing = await fixture(t), otherBefore = otherSnapshot(outgoing);
  outgoing.store.database.prepare('UPDATE work_item_relations SET parent_work_item_id=? WHERE work_item_id=?').run(outgoing.otherWork.id, outgoing.workB.id);
  outgoing.store.database.prepare('INSERT INTO work_item_dependencies(work_item_id,depends_on_work_item_id,created_at) VALUES (?,?,?)').run(outgoing.workA.id, outgoing.otherWork.id, timestamp);
  const outgoingPlan = previewProjectPurgeSqlite(outgoing.options);
  assert.deepEqual(outgoingPlan.blockers, []);
  beginProjectPurgeSqlite(outgoingPlan).commit();
  assert.equal(otherSnapshot(outgoing), otherBefore);
});

test('offline rebuild removes only owned receipts and preserves original triggers, rowids, decisions and sequence', { skip: process.platform !== 'win32' }, async t => {
  const f = await fixture(t);
  f.store.createInstallationReceipt({ projectId: f.own.id, component: 'synthetic', sourceCommitSha: 'a'.repeat(40), artifactPath: path.join(f.own.root_path, 'synthetic.exe'), artifactSha256: 'b'.repeat(64) });
  f.store.createInstallationReceipt({ projectId: f.other.id, component: 'synthetic', sourceCommitSha: 'c'.repeat(40), artifactPath: path.join(f.other.root_path, 'synthetic.exe'), artifactSha256: 'd'.repeat(64) });
  f.store.appendEvent(f.workA.id, 'highest-id-removed', { synthetic: true });
  const current = f.store.decisionStateIdentity();
  f.store.recordDecision({
    scope: 'shared', subject: 'unrelated-retained-decision', content: 'Preserve original bytes', rationale: 'Synthetic',
    source: { kind: 'user_confirmation', reference: 'thread:synthetic-survivor/turn:1' },
    clientRequestId: 'synthetic-survivor-decision', expectedRevision: current.revision, expectedDigest: current.digest,
  });
  const beforeOther = otherSnapshot(f);
  const schemaBefore = f.store.database.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name").all();
  const sequenceBefore = f.store.database.prepare('SELECT * FROM sqlite_sequence ORDER BY name').all();
  const receiptsBefore = f.store.database.prepare('SELECT rowid,* FROM installation_receipts WHERE project_id=?').all(f.other.id);
  const plan = previewProjectPurgeSqlite(f.options);
  assert.equal(plan.strategy, 'REBUILD_SURVIVORS_V1');
  assert.deepEqual(plan.blockers, []);
  assert.equal(plan.counts.installation_receipts, 1);
  assert.throws(() => f.store.database.exec('DELETE FROM installation_receipts'), /append-only/);
  f.store.close();
  const transaction = beginProjectPurgeSqlite(plan);
  assert.throws(() => new LatticeStore(f.options.databasePath), /PURGE_SQLITE_MAINTENANCE_PENDING/);
  assert.equal(readbackProjectPurgeSqlite(plan).complete, false);
  transaction.commit();
  assert.equal(readbackProjectPurgeSqlite(plan).complete, true);
  const after = new LatticeStore(f.options.databasePath); f.stores.push(after);
  assert.equal(otherSnapshot({ ...f, store: after }), beforeOther);
  assert.deepEqual(after.database.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name").all(), schemaBefore);
  assert.deepEqual(after.database.prepare('SELECT * FROM sqlite_sequence ORDER BY name').all(), sequenceBefore);
  assert.deepEqual(after.database.prepare('SELECT rowid,* FROM installation_receipts WHERE project_id=?').all(f.other.id), receiptsBefore);
  assert.doesNotThrow(() => after.decisionStateIdentity());
  assert.throws(() => after.database.exec('DELETE FROM installation_receipts'), /append-only/);
  assert.throws(() => after.database.exec("UPDATE installation_receipts SET component='changed'"), /append-only/);
  assert.throws(() => after.createInstallationReceipt({ projectId: f.own.id }), /project not found/);
  assert.equal(beginProjectPurgeSqlite(plan).alreadyApplied, true);
  assert.equal(existsSync(f.options.databasePath + '.purge-next'), false);
  assert.equal(existsSync(f.options.databasePath + '.purge-swap'), false);
});

async function rebuildFixture(t) {
  const f = await fixture(t);
  f.store.createInstallationReceipt({ projectId: f.own.id, component: 'synthetic', sourceCommitSha: 'a'.repeat(40), artifactPath: path.join(f.own.root_path, 'synthetic.exe'), artifactSha256: 'b'.repeat(64) });
  const plan = previewProjectPurgeSqlite(f.options);
  f.store.close();
  return { ...f, plan, marker: f.options.databasePath + '.purge-swap', next: f.options.databasePath + '.purge-next' };
}

test('rebuild resumes both sides of atomic replacement from the same bound plan', { skip: process.platform !== 'win32' }, async t => {
  for (const afterReplace of [false, true]) {
    const f = await rebuildFixture(t);
    const before = databaseRowsDigest(f.options.databasePath);
    const transaction = beginProjectPurgeSqlite(f.plan, { replace(source, destination) {
      if (afterReplace) renameSync(source, destination);
      throw new Error('SYNTHETIC_INTERRUPTION');
    } });
    assert.throws(() => transaction.commit(), /SYNTHETIC_INTERRUPTION/);
    const pending = JSON.parse(await readFile(f.marker, 'utf8'));
    assert.equal(pending.phase, 'READY');
    assert.equal(readbackProjectPurgeSqlite(f.plan).complete, false);
    assert.throws(() => new LatticeStore(f.options.databasePath), /MAINTENANCE_PENDING/);
    assert.throws(() => beginProjectPurgeSqlite({ ...f.plan, rebuildMetadataDigest: 'f'.repeat(64) }), /SWAP_BINDING/);
    if (!afterReplace) assert.equal(databaseRowsDigest(f.options.databasePath), before);
    const resumed = beginProjectPurgeSqlite(f.plan);
    assert.equal(resumed.alreadyApplied, afterReplace);
    resumed.commit();
    assert.equal(readbackProjectPurgeSqlite(f.plan).complete, true);
    assert.equal(existsSync(f.marker), false);
    assert.equal(existsSync(f.next), false);
  }
});

test('rebuild rollback keeps maintenance binding and later resumes without changing source rows', { skip: process.platform !== 'win32' }, async t => {
  const f = await rebuildFixture(t), before = databaseRowsDigest(f.options.databasePath);
  beginProjectPurgeSqlite(f.plan).rollback();
  assert.equal(databaseRowsDigest(f.options.databasePath), before);
  assert.throws(() => previewProjectPurgeSqlite(f.options), /MAINTENANCE_PENDING/);
  beginProjectPurgeSqlite(f.plan).commit();
  assert.equal(readbackProjectPurgeSqlite(f.plan).complete, true);
});

test('changed staging bytes, sidecar journals and hard-linked sources fail closed', { skip: process.platform !== 'win32' }, async t => {
  for (const corruption of ['bytes', 'journal', 'hardlink']) {
    const f = await rebuildFixture(t);
    if (corruption === 'hardlink') {
      const alias = path.join(path.dirname(f.options.databasePath), 'synthetic-alias.db');
      await link(f.options.databasePath, alias);
      assert.throws(() => beginProjectPurgeSqlite(f.plan), /FILE_IDENTITY/);
      assert.equal(existsSync(f.marker), false);
      continue;
    }
    assert.throws(() => beginProjectPurgeSqlite(f.plan, { replace() { throw new Error('SYNTHETIC_INTERRUPTION'); } }).commit(), /SYNTHETIC_INTERRUPTION/);
    const before = await readFile(f.options.databasePath);
    if (corruption === 'bytes') await writeFile(f.next, 'changed synthetic staging');
    else await writeFile(f.options.databasePath + '-wal', 'stale synthetic wal');
    assert.throws(() => beginProjectPurgeSqlite(f.plan), /SWAP_CHANGED|JOURNAL_REMAINS/);
    assert.deepEqual(await readFile(f.options.databasePath), before);
    assert.equal(readbackProjectPurgeSqlite(f.plan).complete, false);
  }
});

test('a live WAL reader prevents offline rebuilding before any replacement', { skip: process.platform !== 'win32' }, async t => {
  const f = await fixture(t);
  f.store.createInstallationReceipt({ projectId: f.own.id, component: 'synthetic', sourceCommitSha: 'a'.repeat(40), artifactPath: path.join(f.own.root_path, 'synthetic.exe'), artifactSha256: 'b'.repeat(64) });
  const plan = previewProjectPurgeSqlite(f.options), before = hash(rows(f.store.database));
  f.store.database.exec('BEGIN;');
  f.store.database.prepare('SELECT * FROM projects').all();
  assert.throws(() => beginProjectPurgeSqlite(plan), /locked|busy|DATABASE_BUSY/iu);
  assert.equal(hash(rows(f.store.database)), before);
  assert.equal(existsSync(f.options.databasePath + '.purge-next'), false);
  f.store.database.exec('ROLLBACK;'); f.store.close();
  beginProjectPurgeSqlite(plan).commit();
  assert.equal(readbackProjectPurgeSqlite(plan).complete, true);
});

test('a custom source DACL blocks staging before copying any retained content', { skip: process.platform !== 'win32' }, async t => {
  const f = await rebuildFixture(t);
  // Preserve every existing ACE; only protect this disposable fixture from
  // inheritance, making its descriptor differ from the sibling staging file.
  const script = `$ErrorActionPreference='Stop'
[Console]::InputEncoding=[System.Text.UTF8Encoding]::new($false)
Import-Module (Join-Path $PSHOME 'Modules/Microsoft.PowerShell.Security/Microsoft.PowerShell.Security.psd1') -ErrorAction Stop
$file=ConvertFrom-Json ([Console]::In.ReadToEnd())
$acl=Get-Acl -LiteralPath $file
$acl.SetAccessRuleProtection($true,$true)
Set-Acl -LiteralPath $file -AclObject $acl`;
  const result = spawnSync(path.join(process.env.SystemRoot, 'System32/WindowsPowerShell/v1.0/powershell.exe'),
    ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', script],
    { input: JSON.stringify(f.options.databasePath), encoding: 'utf8', windowsHide: true, timeout: 15000 });
  assert.equal(result.status, 0);
  assert.equal(readbackProjectPurgeSqlite(f.plan).accessChanged, true);
  assert.throws(() => beginProjectPurgeSqlite(f.plan), /ACCESS_CHANGED/);
  assert.equal(existsSync(f.marker), false);
  const changedPlan = previewProjectPurgeSqlite(f.options), before = databaseRowsDigest(f.options.databasePath);
  assert.throws(() => beginProjectPurgeSqlite(changedPlan), /ACCESS_MISMATCH/);
  assert.equal(databaseRowsDigest(f.options.databasePath), before);
  assert.equal((await readFile(f.next)).length, 0);
  assert.equal(readbackProjectPurgeSqlite(changedPlan).complete, false);
});

test('project references in immutable decisions block without modifying the decision ledger', async t => {
  const f = await fixture(t), current = f.store.decisionStateIdentity();
  f.store.recordDecision({
    scope: `project:${f.own.id}`, subject: 'synthetic-retention', content: 'Synthetic project decision', rationale: 'Synthetic test',
    source: { kind: 'user_confirmation', reference: 'thread:synthetic-purge-test/turn:1' },
    clientRequestId: 'synthetic-purge-decision', expectedRevision: current.revision, expectedDigest: current.digest,
  });
  const before = hash(rows(f.store.database)), plan = previewProjectPurgeSqlite(f.options);
  assert.ok(plan.blockers.includes('SQLITE_RETAINED_REFERENCE:decisions'));
  assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_BLOCKED/);
  assert.throws(() => f.store.database.exec('DELETE FROM decisions'), /retained permanently/);
  assert.equal(hash(rows(f.store.database)), before);
  assert.doesNotThrow(() => f.store.decisionStateIdentity());
});

test('active work and writer leases block an otherwise owned project', async t => {
  const f = await fixture(t);
  f.store.database.prepare("UPDATE work_items SET status='running' WHERE id=?").run(f.workA.id);
  const plan = previewProjectPurgeSqlite(f.options);
  assert.ok(plan.blockers.includes('SQLITE_ACTIVE_WORK'));
  assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_BLOCKED/);
  f.store.database.prepare("UPDATE work_items SET status='draft' WHERE id=?").run(f.workA.id);
  f.store.database.prepare("INSERT INTO work_items(id,project_id,title,objective,priority,status,created_at,updated_at) VALUES ('primary',?,'synthetic','synthetic','normal','draft',?,?)").run(f.own.id, timestamp, timestamp);
  f.store.database.prepare("INSERT INTO conversation_writer_leases(conversation_id,owner_id,owner_pid,lease_expires_at,updated_at,generation) VALUES ('primary','synthetic',123,?,?,1)").run(timestamp, timestamp);
  const leased = previewProjectPurgeSqlite(f.options);
  assert.ok(leased.blockers.includes('SQLITE_WRITER_LEASE_PRESENT'));
  assert.throws(() => beginProjectPurgeSqlite(leased), /PURGE_SQLITE_BLOCKED/);
});

test('schema drift or an altered immutable trigger fails closed', async t => {
  const f = await fixture(t);
  f.store.database.exec('DROP TRIGGER installation_receipts_no_delete');
  assert.throws(() => previewProjectPurgeSqlite(f.options), /schema|profile/iu);
  assert.ok(f.store.getProject(f.own.id));
});

test('readback reports false if retained project rows change after a successful purge', async t => {
  const f = await fixture(t), plan = previewProjectPurgeSqlite(f.options);
  beginProjectPurgeSqlite(plan).commit();
  assert.equal(readbackProjectPurgeSqlite(plan).complete, true);
  f.store.appendEvent(f.otherWork.id, 'synthetic-change', { after: true });
  assert.equal(readbackProjectPurgeSqlite(plan).complete, false);
});

test('Windows path aliases in registration claims use the same ownership semantics during deletion', { skip: process.platform !== 'win32' }, async t => {
  const f = await fixture(t);
  f.store.database.prepare('UPDATE project_registration_claims SET canonical_path=? WHERE canonical_path=?').run(f.own.root_path.replaceAll('\\', '/'), f.own.root_path);
  const plan = previewProjectPurgeSqlite(f.options);
  assert.deepEqual(plan.blockers, []);
  assert.equal(plan.counts.project_registration_claims, 1);
  beginProjectPurgeSqlite(plan).commit();
  assert.equal(readbackProjectPurgeSqlite(plan).complete, true);
});

test('escaped Windows paths in retained JSON payloads block orphaning related records', { skip: process.platform !== 'win32' }, async t => {
  const f = await fixture(t);
  f.store.appendEvent(f.otherWork.id, 'synthetic-cross-project-reference', { relatedRoot: f.own.root_path });
  const plan = previewProjectPurgeSqlite(f.options);
  assert.ok(plan.blockers.includes('SQLITE_RETAINED_REFERENCE:work_events'));
  assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_BLOCKED/);
});

test('an absent catalog requires an explicit matching PostgreSQL identity and is not evidence of a completed purge', async t => {
  const f = await fixture(t);
  removeCatalogProject(f);
  const before = hash(rows(f.store.database));
  assert.throws(() => previewProjectPurgeSqlite(f.options), /PURGE_SQLITE_AUTHORITATIVE_IDENTITY_REQUIRED/);
  const options = authoritativeOptions(f);
  for (const mismatch of [
    { source: 'USER_INPUT' }, { projectId: f.other.id }, { canonicalPath: f.other.root_path }, { scopeDigest: 'not-a-digest' },
  ]) {
    assert.throws(() => previewProjectPurgeSqlite({ ...options, authoritativeProject: { ...options.authoritativeProject, ...mismatch } }), /PURGE_SQLITE_AUTHORITATIVE_IDENTITY/);
  }
  const plan = previewProjectPurgeSqlite(options);
  assert.equal(plan.catalogState, 'ABSENT');
  assert.deepEqual(plan.authoritativeProject, options.authoritativeProject);
  assert.deepEqual(plan.blockers, []);
  assert.equal(plan.beforeDigest, plan.afterDigest);
  assert.ok(Object.values(plan.counts).every(count => count === 0));
  const transaction = beginProjectPurgeSqlite(plan);
  assert.equal(transaction.alreadyApplied, false);
  assert.equal(transaction.catalogWasAbsent, true);
  transaction.commit();
  assert.equal(readbackProjectPurgeSqlite(plan).complete, true);
  assert.equal(hash(rows(f.store.database)), before);
  assert.throws(() => beginProjectPurgeSqlite({ ...plan, authoritativeProject: null }), /PURGE_SQLITE_AUTHORITATIVE_IDENTITY_REQUIRED/);
  assert.throws(() => readbackProjectPurgeSqlite({ ...plan, authoritativeProject: null }), /PURGE_SQLITE_AUTHORITATIVE_IDENTITY_REQUIRED/);
});

test('an absent catalog deletes only its authority-bound orphan registration claim and supports exact retry', async t => {
  const f = await fixture(t), beforeOther = otherSnapshot(f);
  removeCatalogProject(f, { retainClaim: true });
  const plan = previewProjectPurgeSqlite(authoritativeOptions(f));
  assert.equal(plan.catalogState, 'ABSENT');
  assert.equal(plan.counts.projects, 0);
  assert.equal(plan.counts.project_registration_claims, 1);
  assert.equal(readbackProjectPurgeSqlite(plan).complete, false);
  const transaction = beginProjectPurgeSqlite(plan);
  assert.equal(transaction.alreadyApplied, false);
  assert.equal(transaction.catalogWasAbsent, true);
  transaction.commit();
  assert.equal(readbackProjectPurgeSqlite(plan).complete, true);
  assert.equal(otherSnapshot(f), beforeOther);
  assert.deepEqual(f.store.database.prepare('PRAGMA foreign_key_check').all(), []);
  const retry = beginProjectPurgeSqlite(plan);
  assert.equal(retry.alreadyApplied, true);
  assert.equal(retry.catalogWasAbsent, true);
  retry.commit();
});

test('a surviving project sharing an absent target root or registered canonical path blocks cleanup', async t => {
  for (const location of ['root', 'registration']) {
    const f = await fixture(t);
    removeCatalogProject(f, { retainClaim: true });
    if (location === 'root') f.store.database.prepare('UPDATE projects SET root_path=? WHERE id=?').run(f.own.root_path, f.other.id);
    else f.store.database.prepare('UPDATE project_registration_details SET canonical_path=? WHERE project_id=?').run(f.own.root_path, f.other.id);
    const before = hash(rows(f.store.database)), plan = previewProjectPurgeSqlite(authoritativeOptions(f));
    assert.ok(plan.blockers.includes('SQLITE_SHARED_PROJECT_ROOT'));
    assert.ok(plan.protectedRoots.includes(f.own.root_path));
    assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_BLOCKED/);
    assert.equal(readbackProjectPurgeSqlite(plan).complete, false);
    assert.equal(hash(rows(f.store.database)), before);
  }
});

test('an absent catalog still blocks retained project references and immutable decisions', async t => {
  const f = await fixture(t), current = f.store.decisionStateIdentity();
  f.store.recordDecision({
    scope: `project:${f.own.id}`, subject: 'synthetic-retained-after-catalog-removal', content: 'Synthetic retained decision', rationale: 'Synthetic test',
    source: { kind: 'user_confirmation', reference: 'thread:synthetic-purge-test/turn:2' },
    clientRequestId: 'synthetic-absent-purge-decision', expectedRevision: current.revision, expectedDigest: current.digest,
  });
  removeCatalogProject(f);
  f.store.appendEvent(f.otherWork.id, 'synthetic-absent-reference', { previousProjectId: f.own.id, previousRoot: f.own.root_path });
  const before = hash(rows(f.store.database)), plan = previewProjectPurgeSqlite(authoritativeOptions(f));
  assert.ok(plan.blockers.includes('SQLITE_RETAINED_REFERENCE:decisions'));
  assert.ok(plan.blockers.includes('SQLITE_RETAINED_REFERENCE:work_events'));
  assert.equal(plan.beforeDigest, plan.afterDigest);
  assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_BLOCKED/);
  assert.equal(readbackProjectPurgeSqlite(plan).complete, false);
  assert.throws(() => f.store.database.exec('DELETE FROM decisions'), /retained permanently/);
  assert.equal(hash(rows(f.store.database)), before);
  assert.doesNotThrow(() => f.store.decisionStateIdentity());
});

test('an absent catalog still requires the exact schema and relational integrity', async t => {
  const f = await fixture(t);
  removeCatalogProject(f);
  f.store.database.exec('DROP TRIGGER installation_receipts_no_delete');
  assert.throws(() => previewProjectPurgeSqlite(authoritativeOptions(f)), /schema|profile/iu);
  const orphan = await fixture(t);
  removeCatalogProject(orphan);
  // An invalid legacy orphan must not be silently treated as an owned row.
  orphan.store.database.exec('PRAGMA foreign_keys=OFF');
  orphan.store.database.prepare('INSERT INTO project_registration_details(project_id,canonical_path,registered_at,refreshed_at) VALUES (?,?,?,?)')
    .run(orphan.own.id, orphan.own.root_path, timestamp, timestamp);
  orphan.store.database.exec('PRAGMA foreign_keys=ON');
  assert.throws(() => previewProjectPurgeSqlite(authoritativeOptions(orphan)), /foreign key integrity/iu);
});

test('an absent catalog still discovers nested encoded Windows path references', { skip: process.platform !== 'win32' }, async t => {
  const f = await fixture(t);
  removeCatalogProject(f);
  f.store.appendEvent(f.otherWork.id, 'synthetic-nested-reference', { legacyPayload: JSON.stringify({ root: f.own.root_path }) });
  const plan = previewProjectPurgeSqlite(authoritativeOptions(f));
  assert.ok(plan.blockers.includes('SQLITE_RETAINED_REFERENCE:work_events'));
  assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_BLOCKED/);
});

test('absent-catalog plans reject unrelated mutation, changed orphan claims, and reappearing catalog identity', async t => {
  for (const mutation of ['other-row', 'claim', 'reappeared']) {
    const f = await fixture(t);
    removeCatalogProject(f, { retainClaim: true });
    const plan = previewProjectPurgeSqlite(authoritativeOptions(f));
    if (mutation === 'other-row') f.store.appendEvent(f.otherWork.id, 'synthetic-change', { text: 'changed after absent preview' });
    else if (mutation === 'claim') f.store.database.prepare('UPDATE project_registration_claims SET generation=generation+1 WHERE canonical_path=?').run(f.own.root_path);
    else f.store.database.prepare('INSERT INTO projects(id,name,root_path,created_at,updated_at) VALUES (?,?,?,?,?)')
      .run(f.own.id, 'Reappeared synthetic A', f.own.root_path, timestamp, timestamp);
    const before = hash(rows(f.store.database));
    assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_STALE_SCOPE/);
    assert.equal(readbackProjectPurgeSqlite(plan).complete, false);
    assert.equal(hash(rows(f.store.database)), before);
  }
});

test('old v1 present-catalog plans retain completion and exact retry behavior', async t => {
  const f = await fixture(t), plan = previewProjectPurgeSqlite(f.options);
  delete plan.catalogState;
  delete plan.authoritativeProject;
  const transaction = beginProjectPurgeSqlite(plan);
  assert.equal(transaction.alreadyApplied, false);
  assert.equal(transaction.catalogWasAbsent, false);
  transaction.commit();
  assert.equal(readbackProjectPurgeSqlite(plan).complete, true);
  const retry = beginProjectPurgeSqlite(plan);
  assert.equal(retry.alreadyApplied, true);
  assert.equal(retry.catalogWasAbsent, false);
  retry.commit();
});
