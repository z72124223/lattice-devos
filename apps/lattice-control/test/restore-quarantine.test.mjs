import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, existsSync, symlinkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { DatabaseSync } from 'node:sqlite';
import { fileURLToPath } from 'node:url';
import { LatticeStore } from '../src/store.mjs';
import { LatticeControlService } from '../src/service.mjs';
import { createLatticeServer } from '../src/server.mjs';
import { controlRestoreMarker, importControlSqliteQuarantine } from '../src/restore-quarantine.mjs';

const hash = file => createHash('sha256').update(readFileSync(file)).digest('hex');
function fixture(t) {
  const root = mkdtempSync(path.join(tmpdir(), 'lattice-quarantine-'));
  // Only this test-created directory is removed; no paths from backup metadata.
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const source = path.join(root, 'mixed.sqlite'), store = new LatticeStore(source);
  const projects = ['old-target', 'survivor'].map(name => store.createProject({ name, rootPath: path.join(root, name) }));
  const work = projects.map(project => store.createWorkItem({ projectId: project.id, title: 'Retained work', objective: 'Historical intent is not current authority' }));
  store.close();
  return { root, source, projects, work, expectedSha256: hash(source), destination: path.join(root, 'isolated') };
}
test('quarantine CLI preserves mixed SQLite and blocks all ordinary Store/server entries', t => {
  const f = fixture(t), cli = fileURLToPath(new URL('../../../scripts/control-quarantine-import.mjs', import.meta.url));
  const result = spawnSync(process.execPath, [cli, '--source', f.source, '--destination', f.destination, '--sha256', f.expectedSha256], { encoding: 'utf8', windowsHide: true });
  assert.equal(result.status, 0, result.stderr);
  const receipt = JSON.parse(result.stdout);
  assert.equal(receipt.status, 'CONTROL_RESTORE_QUARANTINED');
  assert.equal(hash(f.source), f.expectedSha256);
  assert.equal(hash(receipt.databasePath), f.expectedSha256);
  const read = new DatabaseSync(receipt.databasePath, { readOnly: true });
  try {
    assert.equal(read.prepare('SELECT count(*) n FROM projects').get().n, 2);
    assert.equal(read.prepare('SELECT count(*) n FROM work_items').get().n, 2);
    assert.throws(() => read.exec("DELETE FROM projects"), /readonly/u);
  } finally { read.close(); }
  const codex = new EventEmitter(); let effects = 0;
  codex.startThread = codex.startTurn = () => { effects++; throw new Error('Unexpected Codex'); };
  for (const autoRestore of [undefined, false, true]) {
    assert.throws(() => createLatticeServer({ databasePath: receipt.databasePath, codex, autoRestore }), /CONTROL_RESTORE_QUARANTINED/u);
  }
  assert.throws(() => new LatticeStore(receipt.databasePath), /CONTROL_RESTORE_QUARANTINED/u);
  assert.equal(effects, 0);
  assert.equal(hash(receipt.databasePath), f.expectedSha256);
});
test('quarantine rejects existing destination, incorrect digest and source sidecars without changes', t => {
  const f = fixture(t);
  assert.throws(() => importControlSqliteQuarantine({ ...f, expectedSha256: '0'.repeat(64) }), /SOURCE_DIGEST_REJECTED/u);
  assert.equal(existsSync(f.destination), false);
  writeFileSync(f.source + '-wal', 'uncheckpointed');
  assert.throws(() => importControlSqliteQuarantine(f), /CLOSED_BACKUP_REQUIRED/u);
  assert.equal(existsSync(f.destination), false);
  const existing = path.join(f.root, 'existing'); mkdirSync(existing); writeFileSync(path.join(existing, 'keep'), 'survivor');
  assert.throws(() => importControlSqliteQuarantine({ ...f, destination: existing }), /FRESH_DESTINATION_REQUIRED/u);
  assert.equal(readFileSync(path.join(existing, 'keep'), 'utf8'), 'survivor');
  assert.equal(hash(f.source), f.expectedSha256);
});
test('failed SQLite validation leaves its copy quarantined and cannot be retried over', t => {
  const f = fixture(t); writeFileSync(f.source, 'not a sqlite database'); f.expectedSha256 = hash(f.source);
  assert.throws(() => importControlSqliteQuarantine(f));
  assert.equal(existsSync(path.join(f.destination, controlRestoreMarker)), true);
  const copied = path.join(f.destination, 'control.sqlite');
  assert.equal(hash(copied), f.expectedSha256);
  assert.throws(() => new LatticeStore(copied), /CONTROL_RESTORE_QUARANTINED/u);
  assert.throws(() => importControlSqliteQuarantine(f), /FRESH_DESTINATION_REQUIRED/u);
});
test('quarantine follows actual ancestor paths through junction aliases and blocks malformed markers', t => {
  const f = fixture(t), receipt = importControlSqliteQuarantine(f);
  const alias = path.join(f.root, 'alias');
  symlinkSync(f.destination, alias, process.platform === 'win32' ? 'junction' : 'dir');
  assert.throws(() => new LatticeStore(path.join(alias, 'control.sqlite')), /CONTROL_RESTORE_QUARANTINED/u);
  const nested = path.join(f.destination, 'nested'); mkdirSync(nested);
  assert.throws(() => new LatticeStore(path.join(alias, 'nested', 'new.sqlite')), /CONTROL_RESTORE_QUARANTINED/u);
  assert.equal(existsSync(path.join(nested, 'new.sqlite')), false);
  writeFileSync(receipt.marker, '{broken');
  assert.throws(() => new LatticeStore(receipt.databasePath), /CONTROL_RESTORE_QUARANTINED/u);
  assert.throws(() => importControlSqliteQuarantine({ ...f, destination: path.join(alias, 'another') }), /LINK_REJECTED/u);
});
test('quarantine added to an open Store blocks service construction and later dispatcher effects', t => {
  const f = fixture(t), store = new LatticeStore(f.source), codex = new EventEmitter();
  const service = new LatticeControlService({ store, codex, autoRestore: false });
  try {
    writeFileSync(path.join(f.root, controlRestoreMarker), '{}');
    assert.throws(() => new LatticeControlService({ store, codex, autoRestore: true }), /CONTROL_RESTORE_QUARANTINED/u);
    assert.throws(() => service.start(f.work[0].id), /CONTROL_RESTORE_QUARANTINED/u);
  } finally { service.close(); store.close(); }
});
