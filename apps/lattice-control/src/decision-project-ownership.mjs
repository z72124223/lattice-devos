import { createHash } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';

const fail = code => { throw Object.assign(new Error(code), { code, statusCode: 409 }); };
export const decisionRequestKey = value => createHash('sha256').update(`lattice.control.decision-request.v1\n${value}`).digest('hex');
export const decisionOwnershipSql = `
CREATE TABLE decision_project_ownership (
 decision_id TEXT PRIMARY KEY NOT NULL REFERENCES decisions(id) DEFERRABLE INITIALLY DEFERRED,
 owner_kind TEXT NOT NULL CHECK(owner_kind IN ('PROJECT','GLOBAL')),
 project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT,
 CHECK((owner_kind='PROJECT' AND project_id IS NOT NULL) OR (owner_kind='GLOBAL' AND project_id IS NULL))
);
CREATE TABLE decision_request_tombstones (
 request_digest TEXT PRIMARY KEY NOT NULL CHECK(length(request_digest)=64 AND request_digest NOT GLOB '*[^0-9a-f]*')
);
CREATE TABLE decision_purge_receipts (
 operation_digest TEXT PRIMARY KEY NOT NULL CHECK(length(operation_digest)=64 AND operation_digest NOT GLOB '*[^0-9a-f]*'),
 before_revision INTEGER NOT NULL CHECK(before_revision>=0),
 before_digest TEXT NOT NULL CHECK(length(before_digest)=64 AND before_digest NOT GLOB '*[^0-9a-f]*'),
 after_revision INTEGER NOT NULL CHECK(after_revision>=0),
 after_digest TEXT NOT NULL CHECK(length(after_digest)=64 AND after_digest NOT GLOB '*[^0-9a-f]*'),
 recorded_at TEXT NOT NULL
);
CREATE TRIGGER decision_ownership_insert_required BEFORE INSERT ON decisions
 WHEN NOT EXISTS(SELECT 1 FROM decision_project_ownership WHERE decision_id=NEW.id)
 BEGIN SELECT RAISE(ABORT,'DECISION_OWNERSHIP_REQUIRED'); END;
CREATE TRIGGER decision_ownership_no_update BEFORE UPDATE ON decision_project_ownership
 BEGIN SELECT RAISE(ABORT,'decision ownership is immutable'); END;
CREATE TRIGGER decision_ownership_no_delete BEFORE DELETE ON decision_project_ownership
 BEGIN SELECT RAISE(ABORT,'decision ownership is immutable'); END;
CREATE TRIGGER decision_tombstone_no_update BEFORE UPDATE ON decision_request_tombstones
 BEGIN SELECT RAISE(ABORT,'decision retirement is immutable'); END;
CREATE TRIGGER decision_tombstone_no_delete BEFORE DELETE ON decision_request_tombstones
 BEGIN SELECT RAISE(ABORT,'decision retirement is immutable'); END;
CREATE TRIGGER decision_purge_receipt_no_update BEFORE UPDATE ON decision_purge_receipts
 BEGIN SELECT RAISE(ABORT,'decision purge receipt is immutable'); END;
CREATE TRIGGER decision_purge_receipt_no_delete BEFORE DELETE ON decision_purge_receipts
 BEGIN SELECT RAISE(ABORT,'decision purge receipt is immutable'); END;
`;
export const decisionOwnershipNames = new Set([
  'decision_project_ownership','decision_request_tombstones','decision_purge_receipts',
  'decision_ownership_insert_required','decision_ownership_no_update','decision_ownership_no_delete',
  'decision_tombstone_no_update','decision_tombstone_no_delete','decision_purge_receipt_no_update','decision_purge_receipt_no_delete',
]);
export function decisionOwnershipPresent(database) {
  return Boolean(database.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='decision_project_ownership'").get());
}
export function assertDecisionRequestNotRetired(database, clientRequestId) {
  if (decisionOwnershipPresent(database)
    && database.prepare('SELECT 1 FROM decision_request_tombstones WHERE request_digest=?').get(decisionRequestKey(clientRequestId))) fail('DECISION_REQUEST_RETIRED');
}
export function decisionOwnershipManifest(normalize) {
  const reference = new DatabaseSync(':memory:');
  try {
    reference.exec('CREATE TABLE decisions(id TEXT PRIMARY KEY); CREATE TABLE projects(id TEXT PRIMARY KEY);');
    reference.exec(decisionOwnershipSql);
    return reference.prepare("SELECT type,name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type,name").all()
      .filter(row => decisionOwnershipNames.has(row.name))
      .map(row => ({ type: row.type, name: row.name, sql_sha256: createHash('sha256').update(normalize(row.sql)).digest('hex') }));
  } finally { reference.close(); }
}
export function normalizedDecisionOwner(value) {
  if (value === undefined) return undefined; // Exact legacy replay only; a new write rejects it.
  if (value && Object.keys(value).length === 1 && value.kind === 'GLOBAL') return { kind: 'GLOBAL' };
  if (value && Object.keys(value).sort().join(',') === 'kind,projectId' && value.kind === 'PROJECT'
    && typeof value.projectId === 'string' && /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u.test(value.projectId)) return { kind: 'PROJECT', projectId: value.projectId };
  fail('DECISION_OWNERSHIP_INVALID');
}
export function prepareDecisionOwnership(database, input, id) {
  const owner = input.owner;
  if (!owner) fail('DECISION_OWNERSHIP_REQUIRED');
  if (owner.kind === 'PROJECT' && !database.prepare('SELECT 1 FROM projects WHERE id=?').get(owner.projectId)) fail('DECISION_OWNER_PROJECT_MISSING');
  if (!decisionOwnershipPresent(database)) {
    const names = database.prepare("SELECT name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'").all();
    if (names.some(row => decisionOwnershipNames.has(row.name))) fail('DECISION_OWNERSHIP_SCHEMA_REJECTED');
    database.exec(decisionOwnershipSql);
  }
  assertDecisionRequestNotRetired(database, input.clientRequestId);
  const lineage = database.prepare(`SELECT d.id,o.owner_kind,o.project_id FROM decisions d
    LEFT JOIN decision_project_ownership o ON o.decision_id=d.id WHERE d.scope=?`).all(input.scope);
  if (lineage.some(row => row.owner_kind == null)) fail('DECISION_LEGACY_SCOPE_UNATTRIBUTABLE');
  if (lineage.some(row => row.owner_kind !== owner.kind || row.project_id !== (owner.projectId ?? null))) fail('DECISION_SCOPE_OWNER_CONFLICT');
  database.prepare('INSERT INTO decision_project_ownership(decision_id,owner_kind,project_id) VALUES (?,?,?)')
    .run(id, owner.kind, owner.projectId ?? null);
}

// The sidecar never guesses from scope text. A whole scope has one owner or is
// blocked; partial lineage erasure is forbidden even if a matching ID is found.
export function classifyDecisionOwnership(rows, projectId) {
  const owners = new Map((rows.decision_project_ownership ?? []).map(row => [row.decision_id, row]));
  const scopes = new Map(), selected = new Set(), unknown = [], blockers = [];
  for (const row of rows.decisions) {
    const scope = scopes.get(row.scope) ?? []; scope.push(row); scopes.set(row.scope, scope);
  }
  for (const [scope, decisions] of scopes) {
    const bindings = decisions.map(row => owners.get(row.id));
    if (bindings.some(binding => !binding)) {
      unknown.push({ scopeDigest: createHash('sha256').update(`lattice.decision-scope.v1\n${scope}`).digest('hex'), count: decisions.length });
      blockers.push('SQLITE_DECISION_OWNERSHIP_UNATTRIBUTABLE'); continue;
    }
    const first = bindings[0];
    if (bindings.some(binding => binding.owner_kind !== first.owner_kind || binding.project_id !== first.project_id)) {
      blockers.push('SQLITE_DECISION_SCOPE_MIXED_OWNERS'); continue;
    }
    if (first.owner_kind === 'PROJECT' && first.project_id === projectId) for (const decision of decisions) selected.add(decision.id);
  }
  return { selected, unknown, blockers: [...new Set(blockers)],
    unassignedRows: unknown.reduce((count, entry) => count + entry.count, 0) };
}
