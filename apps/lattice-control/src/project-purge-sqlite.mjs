import { createHash } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import path from 'node:path';
import { validateControlSchemaProfile } from './store.mjs';
import { controlStoreSchemaVersion } from './database-path.mjs';
import { beginSqliteSurvivorRebuild, sqliteRebuildMetadata, sqliteFileAccessDigests, assertNoSqlitePurgeSwap } from './project-purge-sqlite-swap.mjs';

const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const fail = code => { throw new Error(code); };
const fold = value => process.platform === 'win32' ? value.toLowerCase().replaceAll('\\', '/') : value;
const samePath = (a, b) => fold(path.resolve(a)) === fold(path.resolve(b));
const quote = value => '"' + value.replaceAll('"', '""') + '"';

const absolute = value => {
  if (typeof value !== 'string' || !path.isAbsolute(value)
      || (process.platform === 'win32' && !/^[A-Za-z]:[\\/]/u.test(value))) fail('PURGE_SQLITE_PROJECT_IDENTITY');
  return path.resolve(value);
};

// The coordinator must obtain this binding from its validated PostgreSQL preview.
// Catalog absence alone cannot establish project ownership or authorize cleanup.
function authoritativeIdentity({ projectId, canonicalPath, authoritativeProject }) {
  if (typeof projectId !== 'string' || !projectId.trim()) fail('PURGE_SQLITE_PROJECT_IDENTITY');
  absolute(canonicalPath);
  if (authoritativeProject === undefined || authoritativeProject === null) return null;
  if (authoritativeProject.source !== 'POSTGRES_PREVIEW' || authoritativeProject.projectId !== projectId
      || !/^[a-f0-9]{64}$/u.test(authoritativeProject.scopeDigest)
      || !samePath(absolute(authoritativeProject.canonicalPath), canonicalPath)) fail('PURGE_SQLITE_AUTHORITATIVE_IDENTITY');
  return {
    source: 'POSTGRES_PREVIEW', projectId, canonicalPath: absolute(authoritativeProject.canonicalPath),
    scopeDigest: authoritativeProject.scopeDigest,
  };
}

function planIdentity(plan) {
  const identity = authoritativeIdentity(plan);
  // Old v1 plans were created only while the project was present in the catalog.
  const catalogState = plan.catalogState ?? 'PRESENT';
  if (!['PRESENT', 'ABSENT'].includes(catalogState)) fail('PURGE_SQLITE_PLAN_INVALID');
  if (catalogState === 'ABSENT' && !identity) fail('PURGE_SQLITE_AUTHORITATIVE_IDENTITY_REQUIRED');
  return { catalogState, catalogWasAbsent: catalogState === 'ABSENT' };
}

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

function inspect(db, options, { includeRetained = false } = {}) {
  const { databasePath, projectId, canonicalPath } = options;
  const authoritativeProject = authoritativeIdentity(options);
  const rows = snapshot(db);
  const project = rows.projects.find(row => row.id === projectId);
  if (project && !samePath(project.root_path, canonicalPath)) fail('PURGE_SQLITE_PROJECT_IDENTITY');
  if (!project && !authoritativeProject) fail('PURGE_SQLITE_AUTHORITATIVE_IDENTITY_REQUIRED');
  const catalogState = project ? 'PRESENT' : 'ABSENT';
  const registration = rows.project_registration_details.find(row => row.project_id === projectId);
  if (registration && !samePath(registration.canonical_path, canonicalPath)) fail('PURGE_SQLITE_PROJECT_IDENTITY');
  const workIds = new Set(rows.work_items.filter(row => row.project_id === projectId).map(row => row.id));
  const observationIds = new Set(rows.project_observations.filter(row => row.project_id === projectId).map(row => row.id));
  const owned = (table, row) => {
    if (table === 'projects') return row.id === projectId;
    if (['work_items', 'project_registration_details', 'project_observations', 'installation_receipts'].includes(table)) return row.project_id === projectId;
    if (['work_events', 'work_item_relations', 'work_item_dependencies'].includes(table)) return workIds.has(row.work_item_id);
    if (table === 'conversation_writer_leases') return workIds.has(row.conversation_id);
    if (['project_git_remotes', 'project_rule_documents'].includes(table)) return observationIds.has(row.observation_id);
    if (table === 'project_registration_claims') return samePath(row.canonical_path, canonicalPath);
    return false;
  };
  const retained = {}, counts = {}, blockers = [];
  const otherRoots = [
    ...rows.projects.filter(row => row.id !== projectId).map(row => row.root_path),
    ...rows.project_registration_details.filter(row => row.project_id !== projectId).map(row => row.canonical_path),
  ];
  if (otherRoots.some(root => samePath(root, canonicalPath))) blockers.push('SQLITE_SHARED_PROJECT_ROOT');
  const needles = [projectId, canonicalPath, absolute(canonicalPath), ...(project ? [project.root_path] : []), ...workIds, ...observationIds].map(fold);
  const references = value => {
    const walk = (item, depth = 0) => {
      if (depth > 64) fail('PURGE_SQLITE_REFERENCE_DEPTH');
      if (typeof item === 'string') {
        if (needles.some(needle => fold(item).includes(needle))) return true;
        let decoded;
        try { decoded = JSON.parse(item); } catch { return false; }
        return Boolean(decoded !== item && walk(decoded, depth + 1));
      }
      return Boolean(item && typeof item === 'object'
        && Object.entries(item).some(([key, child]) => walk(key, depth + 1) || walk(child, depth + 1)));
    };
    return walk(value);
  };
  for (const [table, values] of Object.entries(rows)) {
    retained[table] = values.filter(row => !owned(table, row));
    counts[table] = values.length - retained[table].length;
    if (retained[table].some(row => Object.values(row).some(references))) blockers.push(`SQLITE_RETAINED_REFERENCE:${table}`);
  }
  if (rows.work_items.some(row => workIds.has(row.id) && ['starting', 'running', 'waiting_approval'].includes(row.status))) blockers.push('SQLITE_ACTIVE_WORK');
  if (rows.conversation_writer_leases.some(row => workIds.has(row.conversation_id))) blockers.push('SQLITE_WRITER_LEASE_PRESENT');
  if (includeRetained) return retained;
  const strategy = counts.installation_receipts > 0 ? 'REBUILD_SURVIVORS_V1' : 'IN_PLACE_V1';
  let rebuildAccessDigest = null;
  if (strategy === 'REBUILD_SURVIVORS_V1') {
    try { [rebuildAccessDigest] = sqliteFileAccessDigests([path.resolve(databasePath)]); }
    catch (error) {
      if (!/^PURGE_SQLITE_ACCESS_/u.test(error.message)) throw error;
      blockers.push(error.message);
    }
  }
  return {
    schema: 'lattice.project-purge-sqlite.v1', databasePath: path.resolve(databasePath),
    projectId, canonicalPath: path.resolve(canonicalPath),
    catalogState, authoritativeProject,
    beforeDigest: hash(rows), afterDigest: hash(retained), counts, strategy,
    retainedDecisionRows: rows.decisions.length,
    ...(strategy === 'REBUILD_SURVIVORS_V1' ? {
      rebuildMetadataDigest: sqliteRebuildMetadata(db).digest,
      rebuildAccessDigest,
    } : {}),
    claimPaths: rows.project_registration_claims.filter(row => samePath(row.canonical_path, canonicalPath)).map(row => row.canonical_path),
    protectedRoots: [...new Set(otherRoots.map(root => path.resolve(root)))],
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
  assertNoSqlitePurgeSwap(options.databasePath);
  const db = open(options.databasePath, true);
  try { db.exec('BEGIN;'); return inspect(db, options); }
  finally { db.close(); }
}

// The caller keeps this write lock through PostgreSQL/filesystem stages. A failed
// later stage rolls back SQLite; it cannot undo the other stores' committed work.
export function beginProjectPurgeSqlite(plan, rebuildOptions = {}) {
  const { catalogState, catalogWasAbsent } = planIdentity(plan);
  if (plan.strategy === 'REBUILD_SURVIVORS_V1') return beginSqliteSurvivorRebuild(plan, {
    ...rebuildOptions, snapshot, inspect, retainedRows: (db, options) => inspect(db, options, { includeRetained: true }),
  });
  if (plan.strategy !== undefined && plan.strategy !== 'IN_PLACE_V1') fail('PURGE_SQLITE_STRATEGY_INVALID');
  assertNoSqlitePurgeSwap(plan.databasePath);
  const db = open(plan.databasePath, false);
  try {
    db.exec('BEGIN IMMEDIATE;');
    const rows = snapshot(db);
    if (plan.blockers?.length) fail('PURGE_SQLITE_BLOCKED');
    if (hash(rows) === plan.afterDigest && !rows.projects.some(row => row.id === plan.projectId)
        && !(catalogWasAbsent && plan.beforeDigest === plan.afterDigest)) {
      if (catalogWasAbsent && inspect(db, plan).blockers.length) fail('PURGE_SQLITE_BLOCKED');
      return { alreadyApplied: true, catalogWasAbsent, commit() { db.exec('COMMIT;'); db.close(); }, rollback() { db.exec('ROLLBACK;'); db.close(); } };
    }
    const current = inspect(db, plan);
    if (current.catalogState !== catalogState || current.beforeDigest !== plan.beforeDigest || current.afterDigest !== plan.afterDigest) fail('PURGE_SQLITE_STALE_SCOPE');
    if (current.blockers.length) fail('PURGE_SQLITE_BLOCKED');
    return {
      alreadyApplied: false,
      catalogWasAbsent,
      commit() {
        try {
          db.prepare('DELETE FROM work_item_dependencies WHERE work_item_id IN (SELECT id FROM work_items WHERE project_id=?)').run(plan.projectId);
          db.prepare('DELETE FROM work_item_relations WHERE work_item_id IN (SELECT id FROM work_items WHERE project_id=?)').run(plan.projectId);
          for (const claimPath of current.claimPaths) db.prepare('DELETE FROM project_registration_claims WHERE canonical_path=? COLLATE NOCASE').run(claimPath);
          if (db.prepare('DELETE FROM projects WHERE id=?').run(plan.projectId).changes !== (catalogWasAbsent ? 0 : 1)) fail('PURGE_SQLITE_DELETE_COUNT');
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
  planIdentity(plan);
  try { assertNoSqlitePurgeSwap(plan.databasePath); }
  catch (error) { if (error.message === 'PURGE_SQLITE_MAINTENANCE_PENDING') return { complete: false, pending: true }; throw error; }
  if (plan.strategy === 'REBUILD_SURVIVORS_V1') {
    try {
      if (sqliteFileAccessDigests([path.resolve(plan.databasePath)])[0] !== plan.rebuildAccessDigest) return { complete: false, accessChanged: true };
    } catch (error) {
      if (/^PURGE_SQLITE_ACCESS_/u.test(error.message)) return { complete: false, accessVerified: false };
      throw error;
    }
  }
  const db = open(plan.databasePath, true);
  try {
    db.exec('BEGIN;');
    const rows = snapshot(db);
    return { complete: !plan.blockers?.length && !rows.projects.some(row => row.id === plan.projectId) && hash(rows) === plan.afterDigest };
  } finally { db.close(); }
}
