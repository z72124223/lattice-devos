import { createHash } from 'node:crypto';
import { closeSync, existsSync, fstatSync, fsyncSync, lstatSync, openSync, readFileSync, readSync, realpathSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { spawnSync } from 'node:child_process';

const fail = code => { throw new Error(code); };
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const quote = value => '"' + value.replaceAll('"', '""') + '"';
const fold = value => process.platform === 'win32' ? value.toLowerCase() : value;
const identity = stat => `${stat.dev}:${stat.ino}:${stat.birthtimeNs}`;
const digestPattern = /^[a-f0-9]{64}$/u;
const suffixes = ['-wal', '-shm', '-journal'];

// Read the Windows security descriptor without exposing it in receipts or
// changing ACLs. Exact equality is conservative: custom ACLs remain blocked.
// All paths enter through JSON stdin, never executable PowerShell text.
export function sqliteFileAccessDigests(files) {
  if (process.platform !== 'win32') fail('PURGE_SQLITE_ACCESS_AUDIT_UNAVAILABLE');
  for (const file of files) safeFile(file);
  const script = `$ErrorActionPreference='Stop'
[Console]::InputEncoding=[System.Text.UTF8Encoding]::new($false)
Import-Module (Join-Path $PSHOME 'Modules/Microsoft.PowerShell.Security/Microsoft.PowerShell.Security.psd1') -ErrorAction Stop
$paths=ConvertFrom-Json ([Console]::In.ReadToEnd())
$digests=@(foreach($file in $paths) {
  $acl=Get-Acl -LiteralPath $file
  $sections=[System.Security.AccessControl.AccessControlSections]::Access -bor [System.Security.AccessControl.AccessControlSections]::Owner -bor [System.Security.AccessControl.AccessControlSections]::Group
  $sddl=$acl.GetSecurityDescriptorSddlForm($sections)
  $sha=[System.Security.Cryptography.SHA256]::Create()
  try { ([BitConverter]::ToString($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($sddl)))).Replace('-','').ToLowerInvariant() }
  finally { $sha.Dispose() }
})
ConvertTo-Json -InputObject $digests -Compress`;
  const command = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32/WindowsPowerShell/v1.0/powershell.exe');
  const result = spawnSync(command, ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', script], {
    input: JSON.stringify(files), encoding: 'utf8', windowsHide: true, timeout: 15000, maxBuffer: 65536,
  });
  if (result.error || result.status !== 0) fail('PURGE_SQLITE_ACCESS_AUDIT_FAILED');
  let digests; try { digests = JSON.parse(result.stdout); } catch { fail('PURGE_SQLITE_ACCESS_AUDIT_FAILED'); }
  if (!Array.isArray(digests) || digests.length !== files.length || digests.some(value => !digestPattern.test(value))) fail('PURGE_SQLITE_ACCESS_AUDIT_FAILED');
  return digests;
}

export const sqlitePurgeSwapPath = databasePath => `${path.resolve(databasePath)}.purge-swap`;

// This guard is deliberately independent of store.mjs, so normal startup checks
// it before creating/opening a database. It is not a lock against old clients.
export function assertNoSqlitePurgeSwap(databasePath) {
  if (databasePath === ':memory:') return;
  try { lstatSync(sqlitePurgeSwapPath(databasePath)); }
  catch (error) { if (error.code === 'ENOENT') return; throw error; }
  fail('PURGE_SQLITE_MAINTENANCE_PENDING');
}

function safeFile(file, { optional = false } = {}) {
  if (!path.isAbsolute(file)) fail('PURGE_SQLITE_PATH_INVALID');
  for (let parent = path.dirname(file); ; parent = path.dirname(parent)) {
    const stat = lstatSync(parent);
    if (!stat.isDirectory() || stat.isSymbolicLink() || fold(realpathSync(parent)) !== fold(parent)) fail('PURGE_SQLITE_PATH_ALIAS');
    if (parent === path.dirname(parent)) break;
  }
  let stat;
  try { stat = lstatSync(file, { bigint: true }); }
  catch (error) { if (optional && error.code === 'ENOENT') return null; throw error; }
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1n || fold(realpathSync(file)) !== fold(file)) fail('PURGE_SQLITE_FILE_IDENTITY');
  return stat;
}

function fileDigest(file) {
  const before = safeFile(file);
  if (before.size > 512n * 1024n * 1024n) fail('PURGE_SQLITE_FILE_TOO_LARGE');
  const fd = openSync(file, 'r'), digest = createHash('sha256'), buffer = Buffer.alloc(65536);
  try {
    const handle = fstatSync(fd, { bigint: true });
    if (identity(before) !== identity(handle)) fail('PURGE_SQLITE_FILE_IDENTITY');
    for (let count; (count = readSync(fd, buffer)) > 0;) digest.update(buffer.subarray(0, count));
    const after = fstatSync(fd, { bigint: true });
    if (identity(after) !== identity(before) || after.size !== before.size || after.mtimeNs !== before.mtimeNs
      || identity(safeFile(file)) !== identity(before)) fail('PURGE_SQLITE_FILE_CHANGED');
    return digest.digest('hex');
  } finally { closeSync(fd); }
}

function assertNoJournals(file) {
  if (suffixes.some(suffix => existsSync(file + suffix))) fail('PURGE_SQLITE_JOURNAL_REMAINS');
}

export function sqliteRebuildMetadata(db) {
  const manifest = db.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name").all();
  const rowids = {};
  for (const item of manifest.filter(item => item.type === 'table')) {
    rowids[item.name] = db.prepare(`SELECT rowid AS __purge_rowid__, * FROM ${quote(item.name)} ORDER BY rowid`).all();
  }
  const sequence = db.prepare('SELECT name,seq FROM sqlite_sequence ORDER BY name').all();
  const header = Object.fromEntries(['page_size', 'auto_vacuum', 'application_id', 'user_version', 'encoding'].map(name => [name, Object.values(db.prepare(`PRAGMA ${name}`).get())[0]]));
  return { manifest, rowids, sequence, header, digest: hash({ manifest, rowids, sequence, header }) };
}

function writeMarker(file, value, create = false) {
  if (create) {
    const fd = openSync(file, 'wx', 0o600);
    try { writeFileSync(fd, JSON.stringify(value)); fsyncSync(fd); } finally { closeSync(fd); }
  } else {
    safeFile(file);
    const temporary = `${file}.tmp`;
    const fd = openSync(temporary, 'wx', 0o600);
    try { writeFileSync(fd, JSON.stringify(value)); fsyncSync(fd); } finally { closeSync(fd); }
    renameSync(temporary, file);
  }
}

function readMarker(file, binding) {
  const stat = safeFile(file, { optional: true });
  if (!stat) return null;
  if (stat.size > 4096n) fail('PURGE_SQLITE_SWAP_INVALID');
  const value = JSON.parse(readFileSync(file, 'utf8'));
  if (hash(Object.keys(value).sort()) !== hash(['binding', 'nextIdentity', 'nextDigest', 'sourceDigest', 'phase', 'schema'].sort())
    || value.schema !== 'lattice.control.purge-swap.v1' || value.binding !== binding
    || !['RESERVED', 'READY'].includes(value.phase)
    || (value.nextIdentity !== null && !/^\d+:\d+:\d+$/u.test(value.nextIdentity))
    || (value.phase === 'READY' && (!digestPattern.test(value.sourceDigest) || !digestPattern.test(value.nextDigest)))) fail('PURGE_SQLITE_SWAP_BINDING');
  return value;
}

function survivorRowids(metadata, retained) {
  const result = {};
  for (const [table, originals] of Object.entries(metadata.rowids)) {
    const byContent = new Map(originals.map(({ __purge_rowid__, ...row }) => [JSON.stringify(row), __purge_rowid__]));
    let nextId = Math.max(0, ...originals.map(row => row.__purge_rowid__));
    result[table] = retained[table].map(row => {
      let rowid = byContent.get(JSON.stringify(row));
      if (rowid === undefined) {
        if (table === 'decision_state' && originals.length === 1 && retained[table].length === 1
          && row.slot === 'current' && originals[0].slot === row.slot) {
          // Only this global aggregate changes. Surviving decision rows, rowids,
          // request digests and original immutable triggers remain byte-exact.
          rowid = originals[0].__purge_rowid__;
        } else if (['decision_request_tombstones', 'decision_purge_receipts'].includes(table)) {
          rowid = ++nextId;
        } else fail('PURGE_SQLITE_UNEXPECTED_REPLACEMENT_ROW');
      }
      return { __purge_rowid__: rowid, ...row };
    }).sort((a, b) => a.__purge_rowid__ - b.__purge_rowid__);
    if (['decision_request_tombstones', 'decision_purge_receipts'].includes(table)
      && originals.some(({ __purge_rowid__, ...row }) => !retained[table].some(kept => JSON.stringify(kept) === JSON.stringify(row)))) {
      fail('PURGE_SQLITE_ATTESTATION_REMOVED');
    }
  }
  return result;
}

function createSurvivorDatabase(file, metadata, retained, validateSnapshot, expectedDigest) {
  const expectedRowids = survivorRowids(metadata, retained);
  const next = new DatabaseSync(file);
  try {
    next.exec('PRAGMA journal_mode=DELETE; PRAGMA foreign_keys=ON;');
    for (const name of ['page_size', 'auto_vacuum', 'application_id', 'user_version']) {
      const value = metadata.header[name];
      if (!Number.isSafeInteger(value)) fail('PURGE_SQLITE_HEADER_INVALID');
      next.exec(`PRAGMA ${name}=${value};`);
    }
    if (!['UTF-8', 'UTF-16le', 'UTF-16be'].includes(metadata.header.encoding)) fail('PURGE_SQLITE_HEADER_INVALID');
    next.exec(`PRAGMA encoding='${metadata.header.encoding}'; BEGIN; PRAGMA defer_foreign_keys=ON;`);
    for (const item of metadata.manifest.filter(item => item.type === 'table')) next.exec(item.sql);
    for (const [table, rows] of Object.entries(expectedRowids)) {
      for (const original of rows) {
        const { __purge_rowid__, ...row } = original;
        const columns = Object.keys(row);
        next.prepare(`INSERT INTO ${quote(table)} (rowid,${columns.map(quote).join(',')}) VALUES (${Array(columns.length + 1).fill('?').join(',')})`)
          .run(__purge_rowid__, ...Object.values(row));
      }
    }
    // Preserve high-water values even when the greatest event ID was erased.
    next.exec('DELETE FROM sqlite_sequence;');
    for (const row of metadata.sequence) next.prepare('INSERT INTO sqlite_sequence(name,seq) VALUES (?,?)').run(row.name, row.seq);
    for (const item of metadata.manifest.filter(item => item.type !== 'table')) next.exec(item.sql);
    if (hash(validateSnapshot(next)) !== expectedDigest) fail('PURGE_SQLITE_REBUILD_MISMATCH');
    const rebuilt = sqliteRebuildMetadata(next);
    if (hash(rebuilt.manifest) !== hash(metadata.manifest) || hash(rebuilt.header) !== hash(metadata.header)
      || hash(rebuilt.sequence) !== hash(metadata.sequence)) fail('PURGE_SQLITE_REBUILD_PROFILE');
    for (const [table, rows] of Object.entries(rebuilt.rowids)) {
      if (hash(rows) !== hash(expectedRowids[table])) fail('PURGE_SQLITE_ROW_ID_CHANGED');
    }
    const integrity = next.prepare('PRAGMA integrity_check').all();
    if (integrity.length !== 1 || integrity[0].integrity_check !== 'ok') fail('PURGE_SQLITE_REBUILD_INTEGRITY');
    next.exec('COMMIT;');
  } finally { next.close(); }
  assertNoJournals(file);
  const fd = openSync(file, 'r+'); try { fsyncSync(fd); } finally { closeSync(fd); }
}

/** Only for an exact confirmed offline plan. Old clients and hostile filesystem
 * writers are excluded by the maintenance precondition; this is process-crash
 * continuation, not a power-loss, secure-overwrite or OS-wide handle guarantee.
 */
export function beginSqliteSurvivorRebuild(plan, { snapshot, inspect, retainedRows, replace = renameSync }) {
  if (plan.blockers.length) fail('PURGE_SQLITE_BLOCKED');
  const source = path.resolve(plan.databasePath), markerPath = sqlitePurgeSwapPath(source), nextPath = `${source}.purge-next`;
  const binding = hash(plan);
  safeFile(source);
  for (const suffix of suffixes) safeFile(source + suffix, { optional: true });
  if (sqliteFileAccessDigests([source])[0] !== plan.rebuildAccessDigest) fail('PURGE_SQLITE_ACCESS_CHANGED');
  let marker = readMarker(markerPath, binding), db = null;
  if (!marker) {
    if (safeFile(nextPath, { optional: true })) fail('PURGE_SQLITE_STAGING_UNOWNED');
    const read = new DatabaseSync(source, { readOnly: true });
    try {
      const rows = snapshot(read);
      if (hash(rows) === plan.afterDigest && !rows.projects.some(row => row.id === plan.projectId)) {
        return { alreadyApplied: true, catalogWasAbsent: plan.catalogState === 'ABSENT', commit() {}, rollback() {} };
      }
      const current = inspect(read, plan);
      if (current.beforeDigest !== plan.beforeDigest || current.afterDigest !== plan.afterDigest
        || current.blockers.length || sqliteRebuildMetadata(read).digest !== plan.rebuildMetadataDigest) fail('PURGE_SQLITE_STALE_SCOPE');
    } finally { read.close(); }
    marker = { schema: 'lattice.control.purge-swap.v1', binding, phase: 'RESERVED', nextIdentity: null, sourceDigest: null, nextDigest: null };
    writeMarker(markerPath, marker, true);
  }
  const close = () => { if (db) { db.close(); db = null; } };
  try {
    if (marker.phase === 'READY') {
      assertNoJournals(source); assertNoJournals(nextPath);
      if (fileDigest(source) === marker.nextDigest && !safeFile(nextPath, { optional: true })) {
        const read = new DatabaseSync(source, { readOnly: true });
        try { if (hash(snapshot(read)) !== plan.afterDigest) fail('PURGE_SQLITE_READBACK_MISMATCH'); } finally { read.close(); }
        return { alreadyApplied: true, catalogWasAbsent: plan.catalogState === 'ABSENT',
          commit() { if (fileDigest(source) !== marker.nextDigest) fail('PURGE_SQLITE_FILE_CHANGED'); safeFile(markerPath); unlinkSync(markerPath); }, rollback() {} };
      }
      if (fileDigest(source) !== marker.sourceDigest || fileDigest(nextPath) !== marker.nextDigest
        || identity(safeFile(nextPath)) !== marker.nextIdentity) fail('PURGE_SQLITE_SWAP_CHANGED');
      if (sqliteFileAccessDigests([nextPath])[0] !== plan.rebuildAccessDigest) fail('PURGE_SQLITE_ACCESS_CHANGED');
    }
    db = new DatabaseSync(source);
    db.exec('PRAGMA busy_timeout=2000; PRAGMA foreign_keys=ON;');
    const checkpoint = db.prepare('PRAGMA wal_checkpoint(TRUNCATE)').get();
    if (checkpoint.busy !== 0) fail('PURGE_SQLITE_DATABASE_BUSY');
    if (db.prepare('PRAGMA journal_mode=DELETE').get().journal_mode !== 'delete') fail('PURGE_SQLITE_DATABASE_BUSY');
    db.exec('BEGIN EXCLUSIVE;');
    const current = inspect(db, plan), metadata = sqliteRebuildMetadata(db);
    if (current.beforeDigest !== plan.beforeDigest || current.afterDigest !== plan.afterDigest
      || metadata.digest !== plan.rebuildMetadataDigest || current.blockers.length) fail('PURGE_SQLITE_STALE_SCOPE');
    if (marker.phase !== 'READY') {
      const prior = safeFile(nextPath, { optional: true });
      if (prior) {
        if (!marker.nextIdentity || identity(prior) !== marker.nextIdentity) fail('PURGE_SQLITE_STAGING_UNOWNED');
        assertNoJournals(nextPath); unlinkSync(nextPath);
      }
      const fd = openSync(nextPath, 'wx', 0o600);
      try { marker.nextIdentity = identity(fstatSync(fd, { bigint: true })); } finally { closeSync(fd); }
      writeMarker(markerPath, marker);
      if (sqliteFileAccessDigests([nextPath])[0] !== plan.rebuildAccessDigest) fail('PURGE_SQLITE_ACCESS_MISMATCH');
    }
    return {
      alreadyApplied: false, catalogWasAbsent: plan.catalogState === 'ABSENT',
      commit() {
        try {
          if (marker.phase !== 'READY') {
            if (identity(safeFile(nextPath)) !== marker.nextIdentity || safeFile(nextPath).size !== 0n) fail('PURGE_SQLITE_STAGING_UNOWNED');
            if (sqliteFileAccessDigests([source, nextPath]).some(value => value !== plan.rebuildAccessDigest)) fail('PURGE_SQLITE_ACCESS_CHANGED');
            createSurvivorDatabase(nextPath, metadata, retainedRows(db, plan), snapshot, plan.afterDigest);
          }
          db.exec('COMMIT;'); close();
          assertNoJournals(source); assertNoJournals(nextPath);
          if (marker.phase !== 'READY') {
            marker = { ...marker, phase: 'READY', sourceDigest: fileDigest(source), nextDigest: fileDigest(nextPath) };
            writeMarker(markerPath, marker);
          }
          if (fileDigest(source) !== marker.sourceDigest || fileDigest(nextPath) !== marker.nextDigest
            || identity(safeFile(nextPath)) !== marker.nextIdentity) fail('PURGE_SQLITE_SWAP_CHANGED');
          if (sqliteFileAccessDigests([source, nextPath]).some(value => value !== plan.rebuildAccessDigest)) fail('PURGE_SQLITE_ACCESS_CHANGED');
          replace(nextPath, source);
          if (fileDigest(source) !== marker.nextDigest) fail('PURGE_SQLITE_READBACK_MISMATCH');
          assertNoJournals(source); safeFile(markerPath); unlinkSync(markerPath);
        } finally { close(); }
      },
      rollback() { close(); }, // Pending remains bound; normal startup must stay blocked.
    };
  } catch (error) { close(); throw error; }
}
