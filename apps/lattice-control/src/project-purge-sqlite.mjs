import { createHash } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import path from 'node:path';
import { validateControlSchemaProfile } from './store.mjs';
import { controlStoreSchemaVersion } from './database-path.mjs';

const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const fail = code => { throw new Error(code); };
const fold = value => process.platform === 'win32' ? value.toLowerCase().replaceAll('\\', '/') : value;
const samePath = (a, b) => fold(path.resolve(a)) === fold(path.resolve(b));
const quote = value => '"' + value.replaceAll('"', '""') + '"';

function snapshot(db) {
  validateControlSchemaProfile(db);
  if (db.prepare('PRAGMA user_version').get().user_version !== controlStoreSchemaVersion) fail('PURGE_SQLITE_SCHEMA_VERSION');
  const rows = {};
  for (const { name } of db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all()) {
    const values = db.prepare(`SELECT * FROM ${quote(name)} LIMIT 10001`).all();
    if (values.length > 10000) fail('PURGE_SQLITE_SCOPE_TOO_LARGE');
    rows[name] = values.sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b), 'en'));
  }
  if (Buffer.byteLength(JSON.stringify(rows)) > 32 * 1024 * 1024) fail('PURGE_SQLITE_SCOPE_TOO_LARGE');
  return rows;
}

function inspect(db, { databasePath, projectId, canonicalPath }) {
  const rows = snapshot(db);
  const project = rows.projects.find(row => row.id === projectId);
  if (!project || !samePath(project.root_path, canonicalPath)) fail('PURGE_SQLITE_PROJECT_IDENTITY');
  const registration = rows.project_registration_details.find(row => row.project_id === projectId);
  if (registration && !samePath(registration.canonical_path, canonicalPath)) fail('PURGE_SQLITE_PROJECT_IDENTITY');
  const workIds = new Set(rows.work_items.filter(row => row.project_id === projectId).map(row => row.id));
  const observationIds = new Set(rows.project_observations.filter(row => row.project_id === projectId).map(row => row.id));
  const owned = (table, row) => {
    if (table === 'projects') return row.id === projectId;
    if (['work_items', 'project_registration_details', 'project_observations'].includes(table)) return row.project_id === projectId;
    if (['work_events', 'work_item_relations', 'work_item_dependencies'].includes(table)) return workIds.has(row.work_item_id);
    if (table === 'conversation_writer_leases') return workIds.has(row.conversation_id);
    if (['project_git_remotes', 'project_rule_documents'].includes(table)) return observationIds.has(row.observation_id);
    if (table === 'project_registration_claims') return samePath(row.canonical_path, canonicalPath);
    return false;
  };
  const retained = {}, counts = {}, blockers = [];
  const needles = [projectId, canonicalPath, project.root_path, ...workIds, ...observationIds].map(fold);
  const references = value => {
    if (typeof value !== 'string') return false;
    if (needles.some(needle => fold(value).includes(needle))) return true;
    try {
      const walk = item => typeof item === 'string' ? needles.some(needle => fold(item).includes(needle))
        : item && typeof item === 'object' && Object.entries(item).some(([key, child]) => walk(key) || walk(child));
      return Boolean(walk(JSON.parse(value)));
    } catch { return false; }
  };
  for (const [table, values] of Object.entries(rows)) {
    retained[table] = values.filter(row => !owned(table, row));
    counts[table] = values.length - retained[table].length;
    if (retained[table].some(row => Object.values(row).some(references))) blockers.push(`SQLITE_RETAINED_REFERENCE:${table}`);
  }
  if (rows.installation_receipts.some(row => row.project_id === projectId)) blockers.push('SQLITE_IMMUTABLE_INSTALLATION_RECEIPT');
  if (rows.work_items.some(row => workIds.has(row.id) && ['starting', 'running', 'waiting_approval'].includes(row.status))) blockers.push('SQLITE_ACTIVE_WORK');
  if (rows.conversation_writer_leases.some(row => workIds.has(row.conversation_id))) blockers.push('SQLITE_WRITER_LEASE_PRESENT');
  return {
    schema: 'lattice.project-purge-sqlite.v1', databasePath: path.resolve(databasePath),
    projectId, canonicalPath: path.resolve(canonicalPath),
    beforeDigest: hash(rows), afterDigest: hash(retained), counts,
    claimPaths: rows.project_registration_claims.filter(row => samePath(row.canonical_path, canonicalPath)).map(row => row.canonical_path),
    protectedRoots: rows.projects.filter(row => row.id !== projectId).map(row => path.resolve(row.root_path)),
    blockers: [...new Set(blockers)].sort(),
  };
}

function open(databasePath, readOnly) {
  if (!path.isAbsolute(databasePath)) fail('PURGE_SQLITE_PATH_INVALID');
  const db = new DatabaseSync(databasePath, { readOnly });
  db.exec('PRAGMA foreign_keys=ON; PRAGMA busy_timeout=2000;');
  return db;
}

export function previewProjectPurgeSqlite(options) {
  const db = open(options.databasePath, true);
  try { db.exec('BEGIN;'); return inspect(db, options); }
  finally { db.close(); }
}

// The caller keeps this write lock through PostgreSQL/filesystem stages. A failed
// later stage rolls back SQLite; it cannot undo the other stores' committed work.
export function beginProjectPurgeSqlite(plan) {
  const db = open(plan.databasePath, false);
  try {
    db.exec('BEGIN IMMEDIATE;');
    const rows = snapshot(db);
    if (hash(rows) === plan.afterDigest && !rows.projects.some(row => row.id === plan.projectId)) {
      return { alreadyApplied: true, commit() { db.exec('COMMIT;'); db.close(); }, rollback() { db.exec('ROLLBACK;'); db.close(); } };
    }
    const current = inspect(db, plan);
    if (current.beforeDigest !== plan.beforeDigest || current.afterDigest !== plan.afterDigest) fail('PURGE_SQLITE_STALE_SCOPE');
    if (current.blockers.length) fail('PURGE_SQLITE_BLOCKED');
    return {
      alreadyApplied: false,
      commit() {
        try {
          db.prepare('DELETE FROM work_item_dependencies WHERE work_item_id IN (SELECT id FROM work_items WHERE project_id=?)').run(plan.projectId);
          db.prepare('DELETE FROM work_item_relations WHERE work_item_id IN (SELECT id FROM work_items WHERE project_id=?)').run(plan.projectId);
          for (const claimPath of current.claimPaths) db.prepare('DELETE FROM project_registration_claims WHERE canonical_path=? COLLATE NOCASE').run(claimPath);
          if (db.prepare('DELETE FROM projects WHERE id=?').run(plan.projectId).changes !== 1) fail('PURGE_SQLITE_DELETE_COUNT');
          if (hash(snapshot(db)) !== plan.afterDigest) fail('PURGE_SQLITE_READBACK_MISMATCH');
          db.exec('COMMIT;');
        } catch (error) { db.exec('ROLLBACK;'); throw error; }
        finally { db.close(); }
      },
      rollback() { db.exec('ROLLBACK;'); db.close(); },
    };
  } catch (error) { db.close(); throw error; }
}

export function readbackProjectPurgeSqlite(plan) {
  const db = open(plan.databasePath, true);
  try {
    db.exec('BEGIN;');
    const rows = snapshot(db);
    return { complete: !rows.projects.some(row => row.id === plan.projectId) && hash(rows) === plan.afterDigest };
  } finally { db.close(); }
}
