import { createHash } from 'node:crypto';
import { constants, copyFileSync, closeSync, fstatSync, lstatSync, mkdirSync, openSync, readSync, realpathSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

export const controlRestoreMarker = 'control.restore-quarantine.json';
function reject(code) { throw Object.assign(new Error(code), { code }); }
function statOrAbsent(file) {
  try { return lstatSync(file); } catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}
function ancestors(file, visit) {
  for (let current = file;; current = path.dirname(current)) {
    visit(current);
    if (path.dirname(current) === current) break;
  }
}

// Markers are deliberately outside the historical DB. Their mere presence,
// including malformed files or dangling links, blocks activation. There is no
// release action. Copying a DB elsewhere without its marker is an OS boundary.
export function assertControlDatabaseNotQuarantined(databasePath = ':memory:') {
  if (databasePath === ':memory:') return;
  const absolute = path.resolve(databasePath);
  const check = directory => {
    if (statOrAbsent(path.join(directory, controlRestoreMarker))) reject('CONTROL_RESTORE_QUARANTINED');
  };
  ancestors(path.dirname(absolute), check);
  // Resolve the actual DB (or nearest existing ancestor) to close junction,
  // directory symlink and file symlink aliases into a quarantined tree.
  let existing = absolute;
  while (!statOrAbsent(existing)) {
    const parent = path.dirname(existing);
    if (parent === existing) reject('CONTROL_DATABASE_PATH_UNAVAILABLE');
    existing = parent;
  }
  const actual = realpathSync(existing);
  ancestors(lstatSync(actual).isDirectory() ? actual : path.dirname(actual), check);
}

function regularAbsolute(file, directory = false) {
  if (typeof file !== 'string' || !path.isAbsolute(file)) reject('CONTROL_RESTORE_ABSOLUTE_PATH_REQUIRED');
  ancestors(file, current => {
    const stat = lstatSync(current);
    if (stat.isSymbolicLink()) reject('CONTROL_RESTORE_LINK_REJECTED');
  });
  const stat = lstatSync(file);
  if (directory ? !stat.isDirectory() : (!stat.isFile() || stat.nlink !== 1)) reject('CONTROL_RESTORE_REGULAR_PATH_REQUIRED');
  return realpathSync(file);
}
function digest(file) {
  const fd = openSync(file, 'r'), hash = createHash('sha256'), buffer = Buffer.alloc(1024 * 1024);
  try {
    const before = fstatSync(fd);
    for (let count; (count = readSync(fd, buffer, 0, buffer.length, null));) hash.update(buffer.subarray(0, count));
    const after = fstatSync(fd);
    if (before.size !== after.size || before.mtimeMs !== after.mtimeMs) reject('CONTROL_RESTORE_SOURCE_CHANGED');
    return hash.digest('hex');
  } finally { closeSync(fd); }
}
function noSidecars(source) {
  for (const suffix of ['-wal', '-shm', '-journal']) {
    if (statOrAbsent(source + suffix)) reject('CONTROL_RESTORE_CLOSED_BACKUP_REQUIRED');
  }
}

// Imports an already closed SQLite backup for inspection only. It does not
// connect to a source DB, migrate schemas, contact Runtime/Codex or infer any
// current retirement/activation authority from historical backup contents.
export function importControlSqliteQuarantine({ source, destination, expectedSha256 }) {
  if (!/^[0-9a-f]{64}$/u.test(expectedSha256 ?? '')) reject('CONTROL_RESTORE_DIGEST_REQUIRED');
  source = regularAbsolute(source);
  if (typeof destination !== 'string' || !path.isAbsolute(destination)) reject('CONTROL_RESTORE_ABSOLUTE_PATH_REQUIRED');
  destination = path.join(regularAbsolute(path.dirname(destination), true), path.basename(destination));
  if (statOrAbsent(destination)) reject('CONTROL_RESTORE_FRESH_DESTINATION_REQUIRED');
  noSidecars(source);
  if (digest(source) !== expectedSha256) reject('CONTROL_RESTORE_SOURCE_DIGEST_REJECTED');
  mkdirSync(destination, { mode: 0o700 }); // exclusive; never reuse or clean up
  const databasePath = path.join(destination, 'control.sqlite');
  const marker = path.join(destination, controlRestoreMarker);
  writeFileSync(marker, JSON.stringify({ schema: 'lattice.control-restore-quarantine.v1',
    status: 'QUARANTINED', sourceSha256: expectedSha256,
    activation: 'NOT_AUTHORIZED', authority: 'HISTORICAL_BACKUP_ONLY' }) + '\n', { flag: 'wx', mode: 0o600 });
  // Every failure from here leaves the marker and any partial copy isolated.
  copyFileSync(source, databasePath, constants.COPYFILE_EXCL);
  noSidecars(source);
  if (digest(source) !== expectedSha256 || digest(databasePath) !== expectedSha256) reject('CONTROL_RESTORE_SOURCE_CHANGED');
  const db = new DatabaseSync(databasePath, { readOnly: true });
  try {
    db.exec('PRAGMA query_only=ON');
    const result = db.prepare('PRAGMA quick_check').all();
    if (result.length !== 1 || result[0].quick_check !== 'ok') reject('CONTROL_RESTORE_SQLITE_CHECK_REJECTED');
  } finally { db.close(); }
  if (digest(databasePath) !== expectedSha256) reject('CONTROL_RESTORE_COPY_CHANGED');
  return { status: 'CONTROL_RESTORE_QUARANTINED', databasePath, marker,
    sha256: expectedSha256, sqliteQuickCheck: 'ok', restoreActivation: 'NOT_PERFORMED',
    scope: 'SUPPORTED_IMPORT_AND_CONTROL_ENTRYPOINTS',
    limits: ['NO_CURRENT_RETIREMENT_AUTHORITY', 'NO_REACTIVATION_ENTRYPOINT', 'MANUAL_DB_RELOCATION_OUTSIDE_MARKED_TREE_NOT_DETECTED'] };
}
