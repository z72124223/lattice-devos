import { createHash } from 'node:crypto';
import { lstat, open, readFile, realpath, rename, unlink } from 'node:fs/promises';
import path from 'node:path';
import { statusProjectPurge } from './project-purge.mjs';
import { previewProjectPurgeFiles, applyProjectPurgeFiles, readbackProjectPurgeFiles } from './project-purge-files.mjs';

const MANIFEST = 'lattice.project-purge.finalization.v1';
const ATTESTATION = 'lattice.project-purge.attestation.v1';
const hash = value => createHash('sha256').update(JSON.stringify(value ?? null)).digest('hex');
const bytesHash = value => createHash('sha256').update(value).digest('hex');
const fail = code => { throw new Error(code); };
const payload = record => { const { digest, ...body } = record; return body; };
const validDigest = value => typeof value === 'string' && /^[a-f0-9]{64}$/u.test(value);
const fileKey = value => process.platform === 'win32' ? value.toLowerCase() : value;

async function safeFile(file, missing = false) {
  if (!path.isAbsolute(file) || path.resolve(file) !== file) fail('PURGE_FINALIZATION_PATH_INVALID');
  for (let parent = path.dirname(file); ; parent = path.dirname(parent)) {
    const stat = await lstat(parent);
    if (!stat.isDirectory() || stat.isSymbolicLink() || fileKey(await realpath(parent)) !== fileKey(parent)) fail('PURGE_FINALIZATION_UNSAFE_ANCESTOR');
    if (path.dirname(parent) === parent) break;
  }
  let stat;
  try { stat = await lstat(file); } catch (error) { if (missing && error.code === 'ENOENT') return null; throw error; }
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || stat.size > 32 * 1024 * 1024) fail('PURGE_FINALIZATION_UNSAFE_FILE');
  return readFile(file);
}
async function readEnvelope(file, missing = false) {
  const bytes = await safeFile(file, missing);
  if (!bytes) return null;
  let record; try { record = JSON.parse(bytes); } catch { fail('PURGE_FINALIZATION_RECORD_INVALID'); }
  if (!record || ![MANIFEST, ATTESTATION].includes(record.schema) || !validDigest(record.planDigest)
    || !validDigest(record.digest) || record.digest !== hash(payload(record))) fail('PURGE_FINALIZATION_RECORD_INVALID');
  if (record.schema === ATTESTATION && (Object.keys(record).some(key => ![
    'schema','status','planDigest','operationDigest','verifiedAt','scope','stageDigests','retainedArtifacts',
    'limitations','complete','latticeScopeComplete','externalCleanup','predecessor','digest',
  ].includes(key)) || record.status !== 'LOGICAL_SCOPE_COMPLETE' || record.complete !== false
    || record.latticeScopeComplete !== true || !validDigest(record.predecessor))) fail('PURGE_FINALIZATION_RECORD_INVALID');
  if (record.schema === ATTESTATION && (!validDigest(record.operationDigest)
    || record.scope !== 'CONFIGURED_ACTIVE_LATTICE_LOGICAL_STORES' || record.externalCleanup !== 'NOT_VERIFIED'
    || !record.stageDigests || Object.keys(record.stageDigests).sort().join(',') !== 'bot,files,graph,postgres,sqlite'
    || !Object.values(record.stageDigests).every(validDigest))) fail('PURGE_FINALIZATION_RECORD_INVALID');
  return record;
}
async function publish(file, body, predecessor, fault) {
  const record = { ...body, predecessor, digest: hash({ ...body, predecessor }) };
  const temporary = `${file}.next`;
  if (await safeFile(temporary, true)) fail('PURGE_FINALIZATION_PENDING_WRITE');
  const output = await open(temporary, 'wx', 0o600);
  try { await output.writeFile(JSON.stringify(record, null, 2)); await output.sync(); } finally { await output.close(); }
  await fault?.('after-write', record);
  await rename(temporary, file);
  return record;
}
async function recover(file, confirmDigest) {
  let current = await readEnvelope(file, true);
  const pending = await readEnvelope(`${file}.next`, true);
  if (pending) {
    if (pending.planDigest !== confirmDigest || pending.predecessor !== (current?.digest ?? null)
      || (current && current.schema !== MANIFEST)) fail('PURGE_FINALIZATION_PENDING_MISMATCH');
    await rename(`${file}.next`, file); current = pending;
  }
  if (!current || current.planDigest !== confirmDigest) fail('PURGE_FINALIZATION_RECORD_MISSING');
  return current;
}

function scope(plan) {
  const owned = plan.maintenance;
  if (!owned || owned.schema !== 'lattice.project-purge.maintenance-scope.v1'
    || owned.statePath !== plan.statePath || !validDigest(owned.configDigest)) fail('PURGE_FINALIZATION_NOT_PLANNED');
  const roots = [owned.planPath, owned.configPath, owned.statePath, owned.boundaryPath].filter(Boolean);
  return { projectId: plan.projectId, roots,
    protectedRoots: [owned.finalizationPath, `${owned.finalizationPath}.next`, `${owned.finalizationPath}.lock`,
      `${plan.statePath}.lock`, plan.sqlite.databasePath, plan.nativeBinary].filter(Boolean) };
}
function logicalDataReady(result) {
  return result.status === 'SCOPED_PURGED' && result.report.stages.every(stage => ['VERIFIED_ERASED','VERIFIED_ABSENT'].includes(stage.status))
    && !result.report.remaining.some(item => item.kind === 'controlDecisions' || (item.ownership === 'LATTICE' && item.kind !== 'maintenance'));
}

/// Freeze only the exact metadata paths authorized by the original preview.
/// The file manifest adds their current bytes after core erasure has verified.
export async function finalizeProjectPurge(plan, { confirmDigest, maintenanceOffline, status = statusProjectPurge, fault } = {}) {
  if (confirmDigest !== plan.digest || !validDigest(confirmDigest) || hash(payload(plan)) !== confirmDigest
    || maintenanceOffline !== true) fail('PURGE_EXACT_CONFIRMATION_REQUIRED');
  const owned = scope(plan), file = plan.maintenance.finalizationPath;
  await safeFile(file, true);
  const lock = await open(`${file}.lock`, 'wx', 0o600);
  const operationLock = await open(`${plan.statePath}.lock`, 'wx', 0o600).catch(async error => { await lock.close(); await unlink(`${file}.lock`); throw error; });
  try {
    if (await safeFile(file, true) || await safeFile(`${file}.next`, true)) fail('PURGE_FINALIZATION_ALREADY_STARTED');
    const result = await status(plan);
    if (!logicalDataReady(result)) fail('PURGE_FINALIZATION_OWNED_SCOPE_INCOMPLETE');
    const config = await safeFile(plan.maintenance.configPath);
    if (bytesHash(config) !== plan.maintenance.configDigest) fail('PURGE_FINALIZATION_CONFIG_CHANGED');
    const original = JSON.parse(await safeFile(plan.maintenance.planPath));
    if (hash(original) !== hash(plan)) fail('PURGE_FINALIZATION_PLAN_CHANGED');
    const progress = JSON.parse(await safeFile(plan.statePath));
    if (progress.planDigest !== plan.digest || progress.operationId !== plan.operationId) fail('PURGE_FINALIZATION_PROGRESS_CHANGED');
    for (const suffix of ['.tmp']) if (await safeFile(`${plan.statePath}${suffix}`, true)) fail('PURGE_FINALIZATION_RECOVERY_PENDING');
    for (const target of owned.roots) await safeFile(target);
    const files = await previewProjectPurgeFiles(owned);
    if (files.entries.some(entry => entry.type !== 'file') || files.entries.length !== owned.roots.length) fail('PURGE_FINALIZATION_SCOPE_INVALID');
    let record = await publish(file, { schema: MANIFEST, planDigest: plan.digest, plan, files, progress: null,
      attestation: { schema: ATTESTATION, status: 'LOGICAL_SCOPE_COMPLETE', planDigest: plan.digest,
        operationDigest: hash(plan.operationId), verifiedAt: new Date().toISOString(), scope: 'CONFIGURED_ACTIVE_LATTICE_LOGICAL_STORES',
        stageDigests: { postgres: result.postgres.afterDigest, sqlite: hash(result.sqlite), files: hash(result.files),
          bot: result.bot?.afterDigest ?? hash(result.postgres.relatedStores?.botLifecycle), graph: hash(result.postgres.relatedStores?.runtimeGraph) },
        retainedArtifacts: ['REGISTRY_VERIFICATION_SEALS','OPERATION_RECEIPTS','RETIREMENT_TOMBSTONES','FINALIZATION_ATTESTATION'],
        limitations: ['LOCAL_TRUST_NOT_USER_TAMPER_PROOF','COMMITMENTS_ALLOW_OFFLINE_GUESSING','PROCESS_CRASH_RECOVERY_ONLY',
          'PHYSICAL_MEDIA_WAL_BACKUPS_EXTERNAL_COPIES_NOT_ERASED','CODEX_PERMANENT_CONVERSATION_ERASURE_NOT_VERIFIED','VERIFIED_AT_FINALIZATION_NOT_CONTINUOUS_MONITORING'],
        complete: false, latticeScopeComplete: true, externalCleanup: 'NOT_VERIFIED' } }, null, fault);
    return await finish(file, record, fault);
  } finally { await operationLock.close(); await unlink(`${plan.statePath}.lock`); await lock.close(); await unlink(`${file}.lock`); }
}
async function finish(file, initial, fault) {
  let record = initial;
  if (record.schema === ATTESTATION) return record;
  const plan = record.plan, owned = scope(plan);
  if (file !== plan.maintenance.finalizationPath || plan.digest !== record.planDigest || hash(payload(plan)) !== plan.digest
    || hash([...record.files.roots].sort()) !== hash([...owned.roots].sort())) fail('PURGE_FINALIZATION_SCOPE_INVALID');
  const options = { ...owned, planDigest: record.files.planDigest, previousResult: record.progress,
    onProgress: async progress => {
      record = await publish(file, { ...payload(record), progress }, record.digest, fault);
      await fault?.('after-progress', record);
    } };
  const result = await applyProjectPurgeFiles(record.files, options);
  if (result.status !== 'completed' || !(await readbackProjectPurgeFiles(record.files, options)).complete) fail('PURGE_FINALIZATION_FILES_INCOMPLETE');
  return publish(file, record.attestation, record.digest, fault);
}
export async function resumeProjectPurgeFinalization(file, { confirmDigest, maintenanceOffline, fault } = {}) {
  if (!validDigest(confirmDigest) || maintenanceOffline !== true) fail('PURGE_EXACT_CONFIRMATION_REQUIRED');
  await safeFile(file, true);
  const lock = await open(`${file}.lock`, 'wx', 0o600);
  let operationLock, operationPath;
  try {
    const record = await recover(file, confirmDigest);
    if (record.schema === ATTESTATION) return record;
    scope(record.plan); operationPath = `${record.plan.statePath}.lock`;
    operationLock = await open(operationPath, 'wx', 0o600);
    return await finish(file, record, fault);
  } finally { if (operationLock) { await operationLock.close(); await unlink(operationPath); } await lock.close(); await unlink(`${file}.lock`); }
}
export async function readProjectPurgeAttestation(file) {
  const record = await readEnvelope(file);
  if (record.schema !== ATTESTATION || await safeFile(`${file}.next`, true)
    || await safeFile(`${file}.lock`, true)) fail('PURGE_FINALIZATION_INCOMPLETE');
  return record;
}
