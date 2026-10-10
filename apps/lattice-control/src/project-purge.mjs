import { createHash, randomUUID } from 'node:crypto';
import { spawn } from 'node:child_process';
import { open, readFile, writeFile, rename, unlink } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { previewProjectPurgeSqlite, beginProjectPurgeSqlite, readbackProjectPurgeSqlite } from './project-purge-sqlite.mjs';
import { previewProjectPurgeFiles, validateProjectPurgeFiles, applyProjectPurgeFiles, readbackProjectPurgeFiles } from './project-purge-files.mjs';
import { externalPurgeInventory, projectPurgeReport } from './project-purge-report.mjs';
import { previewControlCodeGraphPurge, validateControlCodeGraphPurge, readbackControlCodeGraphPurge } from './project-purge-code-graph.mjs';

const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const fail = code => { throw new Error(code); };
const schema = 'lattice.project-purge.workflow.v1';
const absolute = value => {
  if (typeof value !== 'string' || !path.isAbsolute(value)
    || (process.platform === 'win32' && !/^[A-Za-z]:[\\/]/u.test(value))) fail('PURGE_PATH_INVALID');
  return path.resolve(value);
};
const planDigest = plan => { const { digest, ...body } = plan; return hash(body); };
const containsPath = (parent, child) => {
  const relative = path.relative(parent, child);
  return relative === '' || (!path.isAbsolute(relative) && relative !== '..' && !relative.startsWith(`..${path.sep}`));
};
const request = (plan, action) => ({ schema: 'lattice.project-purge.request.v1', action, projectId: plan.projectId, operationId: plan.operationId,
  ...(plan.registryPolicy ? { registryPolicy: plan.registryPolicy } : {}),
  ...(plan.postgres?.scopeDigest ? { expectedScopeDigest: plan.postgres.scopeDigest } : {}) });
const fileOptions = plan => ({ projectId: plan.projectId, planDigest: plan.files.planDigest, roots: plan.files.roots, protectedRoots: plan.files.protectedRoots });

// No shell, credentials in argv, service lifecycle actions, or policy fallback.
export function nativeProjectPurge(binary, input) {
  absolute(binary);
  return new Promise((resolve, reject) => {
    const child = spawn(binary, [], { shell: false, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    let output = '', errorOutput = '', bytes = 0, settled = false;
    const finish = (error, value) => { if (settled) return; settled = true; clearTimeout(timer); error ? reject(error) : resolve(value); };
    const timer = setTimeout(() => { child.kill(); finish(new Error('PURGE_NATIVE_TIMEOUT_OUTCOME_UNKNOWN')); }, 120000);
    child.on('error', () => finish(new Error('PURGE_NATIVE_START_FAILED')));
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', data => { bytes += Buffer.byteLength(data); if (bytes > 1048576) { child.kill(); finish(new Error('PURGE_NATIVE_OUTPUT_LIMIT')); } else output += data; });
    child.stderr.on('data', data => { bytes += data.length; if (bytes > 1048576) { child.kill(); finish(new Error('PURGE_NATIVE_OUTPUT_LIMIT')); } else errorOutput += data.toString(); });
    child.on('close', code => {
      if (code !== 0) return finish(new Error(/^(?:PROJECT_PURGE|REGISTRY)_[A-Z_]{1,100}$/.test(errorOutput.trim()) ? errorOutput.trim() : 'PURGE_NATIVE_FAILED'));
      try { finish(null, JSON.parse(output)); } catch { finish(new Error('PURGE_NATIVE_RESPONSE_INVALID')); }
    });
    child.stdin.on('error', () => {});
    child.stdin.end(JSON.stringify(input));
  });
}

export async function previewProjectPurge(options, { native = nativeProjectPurge } = {}) {
  const databasePath = absolute(options.databasePath), nativeBinary = absolute(options.nativeBinary), statePath = absolute(options.statePath);
  if (options.registryPolicy !== undefined && options.registryPolicy !== 'MINIMAL_ATTESTATION') fail('PURGE_REGISTRY_POLICY_INVALID');
  const base = { projectId: options.projectId, operationId: options.operationId ?? randomUUID(),
    ...(options.registryPolicy ? { registryPolicy: options.registryPolicy } : {}) };
  const pg = await native(nativeBinary, request(base, 'preview'));
  if (!['READY', 'BLOCKED'].includes(pg.status) || pg.project?.id !== options.projectId || !Array.isArray(pg.blockers)
      || !Array.isArray(pg.protectedRoots) || !Array.isArray(pg.filesystemRoots) || !/^[a-f0-9]{64}$/.test(pg.scopeDigest)) fail('PURGE_NATIVE_RESPONSE_INVALID');
  const canonicalPath = absolute(pg.project.canonicalPath);
  const sqlite = previewProjectPurgeSqlite({ databasePath, projectId: base.projectId, canonicalPath,
    authoritativeProject: { source: 'POSTGRES_PREVIEW', projectId: base.projectId, canonicalPath, scopeDigest: pg.scopeDigest } });
  const codeGraph = await previewControlCodeGraphPurge({ projectId: base.projectId,
    cacheDirectory: absolute(options.codeGraphCacheDirectory ?? path.join(process.env.LOCALAPPDATA || process.cwd(), 'LATTICE/control/code-graphs')) });
  const projectRoots = [...new Set([sqlite.canonicalPath, ...pg.filesystemRoots.map(entry => absolute(entry.path))])];
  // The cache directory is shared by all projects. Even an empty or currently
  // target-only cache cannot authorize recursive deletion of that shared root.
  if (projectRoots.some(root => containsPath(root, codeGraph.cacheDirectory) || containsPath(codeGraph.cacheDirectory, root))) {
    codeGraph.blockers.push({ code: 'PURGE_CODE_GRAPH_SHARED_ROOT_OVERLAP', path: codeGraph.cacheDirectory });
    codeGraph.discovery = 'BLOCKED';
  }
  const roots = [...new Set([...projectRoots, ...codeGraph.roots])];
  const protectedRoots = [...new Set([...sqlite.protectedRoots, ...pg.protectedRoots.map(absolute), databasePath,
    nativeBinary, statePath, `${statePath}.lock`, `${statePath}.tmp`, fileURLToPath(import.meta.url), ...(options.protectedRoots ?? []).map(absolute)])];
  let files;
  const fileBlockers = [];
  try { files = await previewProjectPurgeFiles({ projectId: base.projectId, roots, protectedRoots }); }
  catch (error) {
    if (!/^PURGE_FILE_[A-Z_]+$/.test(error.message)) throw error;
    fileBlockers.push(error.message);
    files = { projectId: base.projectId, roots, protectedRoots, status: 'BLOCKED', blockers: fileBlockers };
  }
  const blockers = [...pg.blockers, ...sqlite.blockers, ...codeGraph.blockers, ...fileBlockers];
  const plan = { schema, ...base, nativeBinary, statePath, sqlite, files, codeGraph,
    postgres: { scopeDigest: pg.scopeDigest, counts: pg.counts, registryStrategy: pg.registryStrategy, history: pg.history ?? null,
      inventoryMode: pg.inventoryMode ?? 'LEGACY_MAINTENANCE_PREVIEW',
      maintenanceExtensionInstalled: pg.maintenanceExtensionInstalled ?? null, maintenanceStopped: pg.maintenanceStopped ?? null },
    blockers,
    externalScope: ['Codex conversations and attachments', 'automations', 'remote repositories and published artifacts', 'backups and database WAL', 'separate Bot lifecycle database', 'Graphify external caches', 'this maintenance plan, progress, and receipts'],
    externalInventory: externalPurgeInventory(options.externalResources),
    status: pg.status === 'BLOCKED' || blockers.length ? 'BLOCKED' : 'READY',
  };
  plan.digest = planDigest(plan);
  return plan;
}

function validatePlan(plan) {
  if (plan.schema !== schema || plan.digest !== planDigest(plan) || plan.projectId !== plan.sqlite.projectId || plan.projectId !== plan.files.projectId
    || !plan.operationId || !/^[a-f0-9]{64}$/.test(plan.postgres.scopeDigest)) fail('PURGE_PLAN_INVALID');
  if (plan.postgres.registryStrategy === 'ATTESTED_EPOCH_V1' && plan.registryPolicy !== 'MINIMAL_ATTESTATION') fail('PURGE_PLAN_INVALID');
  absolute(plan.statePath); absolute(plan.nativeBinary);
  if (plan.codeGraph && (plan.codeGraph.projectId !== plan.projectId
    || plan.codeGraph.roots.some(root => !plan.files.roots.includes(root)))) fail('PURGE_PLAN_INVALID');
  if (plan.sqlite.catalogState === 'ABSENT') {
    const authority = plan.sqlite.authoritativeProject;
    if (authority?.source !== 'POSTGRES_PREVIEW' || authority.projectId !== plan.projectId
      || authority.scopeDigest !== plan.postgres.scopeDigest || authority.canonicalPath !== plan.sqlite.canonicalPath) fail('PURGE_PLAN_INVALID');
  }
}

async function loadState(plan) {
  try {
    const state = JSON.parse(await readFile(plan.statePath, 'utf8'));
    if (state.operationId !== plan.operationId || state.planDigest !== plan.digest) fail('PURGE_OPERATION_ID_REUSED');
    return state;
  } catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}
async function saveState(plan, state) {
  // File sync plus rename supports process-crash continuation, not a verified
  // power-loss ordering guarantee for directory entries (especially on Windows).
  const temporary = `${plan.statePath}.${randomUUID()}.tmp`;
  const file = await open(temporary, 'wx', 0o600);
  try { await file.writeFile(JSON.stringify(state, null, 2)); await file.sync(); }
  finally { await file.close(); }
  await rename(temporary, plan.statePath);
}
function matchingReceipt(plan, receipt) {
  if (plan.postgres.registryStrategy === 'ATTESTED_EPOCH_V1'
    && (receipt?.registryStrategy !== 'ATTESTED_EPOCH_V1'
      || receipt.registrySealDigest !== plan.postgres.history?.sealDigest
      || receipt.history?.epoch !== plan.postgres.history?.epoch
      || receipt.history?.assurance !== 'ATTESTED_FROM_SEAL')) return false;
  return receipt?.status === 'PURGED' && receipt.phase === 'POSTGRES_ONLY' && receipt.operationId === plan.operationId && receipt.scopeDigest === plan.postgres.scopeDigest;
}

export async function applyProjectPurge(plan, { confirmDigest, maintenanceOffline, native = nativeProjectPurge } = {}) {
  validatePlan(plan);
  if (confirmDigest !== plan.digest || maintenanceOffline !== true) fail('PURGE_EXACT_CONFIRMATION_REQUIRED');
  if (plan.status !== 'READY' || plan.blockers.length) fail('PURGE_BLOCKED');
  const lock = await open(`${plan.statePath}.lock`, 'wx', 0o600);
  let transaction = null, state;
  try {
    state = await loadState(plan) ?? { schema: 'lattice.project-purge.progress.v1', operationId: plan.operationId, planDigest: plan.digest,
      status: 'PREPARED', postgres: null, files: null, sqlite: null, externalCleanup: 'NOT_VERIFIED' };
    await saveState(plan, state);
    transaction = beginProjectPurgeSqlite(plan.sqlite);
    await validateProjectPurgeFiles(plan.files, { ...fileOptions(plan), previousResult: state.files });
    if (plan.codeGraph) await validateControlCodeGraphPurge(plan.codeGraph, { allowPlannedPartial: Boolean(state.files) });
    let receipt = await native(plan.nativeBinary, request(plan, 'status'));
    if (!matchingReceipt(plan, receipt)) {
      if (!['NOT_FOUND', 'UNKNOWN_OPERATION'].includes(receipt.status)) fail('PURGE_NATIVE_RECEIPT_MISMATCH');
      if (transaction.alreadyApplied && plan.sqlite.catalogState !== 'ABSENT') fail('PURGE_SQLITE_ABSENT_WITHOUT_NATIVE_RECEIPT');
      const current = await native(plan.nativeBinary, request(plan, 'preview'));
      if (current.scopeDigest !== plan.postgres.scopeDigest || current.status !== 'READY' || current.blockers?.length) fail('PURGE_NATIVE_STALE_SCOPE');
      receipt = await native(plan.nativeBinary, { ...request(plan, 'apply'), expectedScopeDigest: plan.postgres.scopeDigest, authorization: 'ERASE_PROJECT_DATA' });
      if (!matchingReceipt(plan, receipt)) fail('PURGE_NATIVE_RECEIPT_MISMATCH');
    }
    state.postgres = receipt; state.status = 'PARTIAL'; await saveState(plan, state);
    state.files = await applyProjectPurgeFiles(plan.files, { ...fileOptions(plan), previousResult: state.files,
      onProgress: async progress => { state.files = progress; await saveState(plan, state); } });
    await saveState(plan, state);
    if (state.files.status !== 'completed') fail('PURGE_FILES_INCOMPLETE');
    const committing = transaction; transaction = null; committing.commit();
    state.sqlite = readbackProjectPurgeSqlite(plan.sqlite);
    const fileReadback = await readbackProjectPurgeFiles(plan.files, fileOptions(plan));
    state.codeGraph = plan.codeGraph ? await readbackControlCodeGraphPurge(plan.codeGraph) : null;
    const finalReceipt = await native(plan.nativeBinary, request(plan, 'status'));
    if (!state.sqlite.complete || !fileReadback.complete || (plan.codeGraph && !state.codeGraph.complete)
      || !matchingReceipt(plan, finalReceipt)) fail('PURGE_READBACK_INCOMPLETE');
    state.postgres = finalReceipt;
    state.status = 'SCOPED_PURGED'; state.files.readback = fileReadback;
    state.report = projectPurgeReport(plan, state);
    await saveState(plan, state); return state;
  } catch (error) {
    transaction?.rollback(); transaction = null;
    if (state) {
      state.status = 'INCOMPLETE'; state.error = /^[A-Z_]+$/.test(error.message) ? error.message : 'PURGE_STAGE_FAILED';
      // A timeout may have committed PostgreSQL. Never label this as untouched.
      await saveState(plan, state);
    }
    throw error;
  } finally { await lock.close(); await unlink(`${plan.statePath}.lock`); }
}

export async function statusProjectPurge(plan, { native = nativeProjectPurge } = {}) {
  validatePlan(plan);
  if (plan.status === 'BLOCKED') return { status: 'BLOCKED', report: projectPurgeReport(plan) };
  const state = await loadState(plan);
  const receipt = await native(plan.nativeBinary, request(plan, 'status'));
  const sqlite = readbackProjectPurgeSqlite(plan.sqlite);
  const files = await readbackProjectPurgeFiles(plan.files, fileOptions(plan));
  const codeGraph = plan.codeGraph ? await readbackControlCodeGraphPurge(plan.codeGraph) : null;
  const result = { operationId: plan.operationId, planDigest: plan.digest,
    status: state && matchingReceipt(plan, receipt) && sqlite.complete && files.complete && (!plan.codeGraph || codeGraph.complete) ? 'SCOPED_PURGED' : 'INCOMPLETE',
    postgres: receipt, sqlite, files, codeGraph, externalCleanup: 'NOT_VERIFIED', externalScope: plan.externalScope };
  return { ...result, report: projectPurgeReport(plan, result) };
}
