import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { LatticeStore, validateControlSchemaProfile } from '../src/store.mjs';
import { previewProjectPurgeSqlite, beginProjectPurgeSqlite, readbackProjectPurgeSqlite } from '../src/project-purge-sqlite.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const timestamp = '2026-10-09T00:00:00.000Z';
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
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
  t.after(async () => {
    store.close();
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
  return { store, own, other, workA, workB, otherWork, observations, options: { databasePath, projectId: own.id, canonicalPath: own.root_path } };
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

test('real LatticeStore deletion cascades owned rows, handles internal RESTRICT edges, and preserves other-project hash', async t => {
  const f = await fixture(t), beforeOther = otherSnapshot(f);
  const plan = previewProjectPurgeSqlite(f.options);
  assert.deepEqual(plan.blockers, []);
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

test('immutable installation receipts block without changing triggers or data', async t => {
  const f = await fixture(t);
  f.store.createInstallationReceipt({ projectId: f.own.id, component: 'synthetic', sourceCommitSha: 'a'.repeat(40), artifactPath: path.join(f.own.root_path, 'synthetic.exe'), artifactSha256: 'b'.repeat(64) });
  const before = hash(rows(f.store.database)), plan = previewProjectPurgeSqlite(f.options);
  assert.ok(plan.blockers.includes('SQLITE_IMMUTABLE_INSTALLATION_RECEIPT'));
  assert.throws(() => beginProjectPurgeSqlite(plan), /PURGE_SQLITE_BLOCKED/);
  assert.throws(() => f.store.database.exec('DELETE FROM installation_receipts'), /append-only/);
  assert.equal(hash(rows(f.store.database)), before);
  assert.doesNotThrow(() => validateControlSchemaProfile(f.store.database));
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
