// One scope report for every LATTICE project. External discovery is deliberately
// distinct from erasure: an empty or caller-supplied inventory cannot prove absence.
const externalKinds = Object.freeze({
  codex: 'Codex conversations and attachments',
  automations: 'Native schedules and pending runs',
  git: 'Shared Git metadata and remote repositories',
  graphify: 'LATTICE Runtime Graphify derived indexes and caches',
  botLifecycle: 'LATTICE separate Bot lifecycle records',
  backups: 'Backups, database WAL and external copies',
  maintenance: 'Purge plans, progress and maintenance receipts',
});
// Compatibility keeps the original inventory keys. Ownership is independent of
// whether an adapter exists: LATTICE-owned data must never be called external
// merely because this version cannot yet erase it.
const scopeOwnership = Object.freeze({
  codex: 'CODEX', automations: 'CODEX', git: 'MIXED', graphify: 'LATTICE',
  botLifecycle: 'LATTICE', backups: 'MIXED', maintenance: 'LATTICE',
});
const scopeActions = Object.freeze({
  codex: 'Read the exact bound conversation and attachment inventory. Archival is not permanent deletion; the exposed Codex tools do not provide permanent conversation erasure.',
  automations: 'Identify exact project-owned native schedules and pending runs, remove them with the native automation tool under the confirmed scope, and read each result back.',
  git: 'Inspect worktree/common-directory ownership and exact local and remote references. Shared or remotely published history requires its own explicit scope; do not erase another checkout.',
  graphify: 'Inventory Runtime-owned source snapshots, staging and PostgreSQL derived-memory rows; remove or rebuild the target-owned graph and verify queries against surviving projects.',
  botLifecycle: 'Inspect the dedicated LATTICE lifecycle database. Bind its project key to Registry identity, retire active owners, and erase target roles and events without changing survivor generations.',
  backups: 'Separate LATTICE-created backups and journal files from user/third-party copies. Shared backups need an explicit retention/rebuild decision; a checkpoint alone does not erase historical media blocks.',
  maintenance: 'After all owned stages verify, finalize known plans, progress and staging files, retaining only the approved content-free attestation. Do not remove recovery evidence while an owned stage remains incomplete.',
});

export function externalPurgeInventory(input = []) {
  if (!Array.isArray(input) || input.length > 1024) throw new Error('PURGE_RESOURCE_INVENTORY_INVALID');
  const seen = new Set();
  const resources = input.map(value => {
    if (!value || Object.keys(value).some(key => !['kind', 'reference', 'source'].includes(key))
      || !Object.hasOwn(externalKinds, value.kind)
      || ['reference', 'source'].some(key => typeof value[key] !== 'string' || !value[key].trim()
        || value[key].length > 2048 || /[\x00-\x1f\x7f]/u.test(value[key]))) throw new Error('PURGE_RESOURCE_INVENTORY_INVALID');
    const key = `${value.kind}\n${value.reference}`;
    if (seen.has(key)) throw new Error('PURGE_RESOURCE_INVENTORY_DUPLICATE');
    seen.add(key);
    return { kind: value.kind, reference: value.reference, source: value.source, status: 'NOT_VERIFIED' };
  });
  return Object.entries(externalKinds).map(([kind, description]) => ({
    kind, description, ownership: scopeOwnership[kind], discovery: 'NOT_VERIFIED',
    disposition: scopeOwnership[kind] === 'LATTICE' ? 'PRODUCT_WORK_REQUIRED' : 'NATIVE_OR_OWNER_ACTION_REQUIRED',
    nextStep: scopeActions[kind], resources: resources.filter(item => item.kind === kind),
  }));
}

function blockerCode(blocker) { return typeof blocker === 'string' ? blocker : blocker?.code ?? 'UNKNOWN_BLOCKER'; }
function nativeBotInventory(value) {
  if (!value || value.schema !== 'lattice.project-purge.bot-inventory.v1' || value.ownership !== 'LATTICE') return null;
  if (value.discovery === 'NOT_VERIFIED') return {
    discovery: 'NOT_VERIFIED', reason: typeof value.reason === 'string' ? value.reason : 'NATIVE_BOT_READ_FAILED',
  };
  if (!['VERIFIED_ABSENT', 'VERIFIED_EMPTY', 'OBSERVED'].includes(value.discovery)
    || value.scope !== 'VERIFIED_DEDICATED_BOT_SERVICE' || !/^[a-f0-9]{64}$/u.test(value.databaseCommitment ?? '')
    || !value.counts || ['roles', 'events'].some(key => !Number.isSafeInteger(value.counts[key]) || value.counts[key] < 0)) return null;
  if (value.discovery === 'VERIFIED_ABSENT' && value.databasePresent !== false) return null;
  if (value.discovery !== 'VERIFIED_ABSENT' && (value.databasePresent !== true || !/^[a-f0-9]{64}$/u.test(value.snapshotDigest ?? ''))) return null;
  if (value.discovery !== 'OBSERVED' && (value.counts.roles !== 0 || value.counts.events !== 0)) return null;
  return { ...value, source: 'NATIVE_PURGE_READER' };
}
function nextStep(code) {
  if (code.startsWith('PURGE_SQLITE_ACCESS_')) return 'Windows source and staging owner/group/DACL must match exactly; unsupported or unreadable access descriptors remain blocked without changing permissions.';
  if (code === 'REGISTRY_EPOCH_EXTENSION_REQUIRED') return 'Install the compatible Registry epoch extension while stopped, then review a fresh preview with minimal historical attestation.';
  if (code === 'REGISTRY_CURRENT_SURVIVOR_REFERENCE') return 'Another project still uses this identity in its current state; reconcile that project through its normal commands before generating a new purge preview.';
  if (code === 'MAINTENANCE_EXTENSION_REQUIRED') return 'Use a compatible Runtime and install the purge maintenance schema while stopped, then create a fresh preview.';
  if (code === 'MAINTENANCE_OFFLINE_REQUIRED') return 'Enter the existing offline maintenance lifecycle, then create and confirm a fresh preview before erasure.';
  if (code === 'REGISTRY_SURVIVOR_REFERENCE') return 'A retained project history contains this project identity or path; erasure requires an explicit retention or redaction decision, not rewriting another project receipt.';
  if (code.startsWith('PURGE_CODE_GRAPH_')) return 'Inspect the exact Control code-graph cache ownership and stop its writer; unknown or changed cache entries cannot be treated as absent.';
  if (code.includes('HARDLINK')) return 'Hard-link ownership and recovery are not supported; keep this scope blocked without changing shared link metadata to force acceptance.';
  if (code.includes('INTERLEAVED')) return 'Registry history requires a compatible maintenance migration; do not rewrite survivor receipts.';
  if (code.includes('IMMUTABLE') || code.includes('RETAINED_REFERENCE') || code.includes('UNSUPPORTED_REFERENCE')) return 'Resolve the owning module retention and reference contract before erasing; never disable its protection.';
  if (code.includes('ACTIVE') || code.includes('LEASE')) return 'Finish or stop the identified owner through its normal lifecycle, then create a fresh preview.';
  if (code.includes('GIT') || code.includes('PROTECTED') || code.includes('OVERLAP')) return 'Resolve exact project ownership; shared resources are not included in automatic deletion.';
  if (code.includes('POLICY')) return 'Keep this operation blocked until the platform restriction is resolved; do not switch tools to retry.';
  return 'Inspect the specific blocker and create a fresh preview only after its cause is resolved.';
}

export function projectPurgeReport(plan, result = null) {
  const coreStatus = result?.status ?? plan.status;
  const scopedComplete = coreStatus === 'SCOPED_PURGED';
  const external = externalPurgeInventory((plan.externalInventory ?? []).flatMap(group =>
    (group.resources ?? []).map(({ kind, reference, source }) => ({ kind, reference, source }))));
  const bot = nativeBotInventory((result ? result.postgres?.relatedStores : plan.postgres.relatedStores)?.botLifecycle);
  if (bot) {
    const scope = external.find(item => item.kind === 'botLifecycle');
    Object.assign(scope, { discovery: bot.discovery, observation: bot,
      disposition: ['VERIFIED_ABSENT', 'VERIFIED_EMPTY'].includes(bot.discovery) ? 'NO_DATA_IN_CONFIGURED_STORE' : 'PRODUCT_WORK_REQUIRED' });
  }
  const graphPg=(result?result.postgres?.relatedStores:plan.postgres.relatedStores)?.runtimeGraph;
  const graph=external.find(item=>item.kind==='graphify');
  if(graphPg?.schema==='lattice.project-purge.graph-inventory.v1'&&graphPg.ownership==='LATTICE') {
    graph.observation={postgres:graphPg,files:result?.runtimeGraph??plan.runtimeGraph??null};
    graph.discovery=graphPg.discovery;
    if(result?.runtimeGraph?.complete===true&&graphPg.discovery==='VERIFIED_EMPTY'
      &&graphPg.scope==='VERIFIED_MAIN_STORE_MEMORY'&&/^[a-f0-9]{64}$/u.test(graphPg.snapshotDigest??'')
      &&graphPg.counts&&Object.values(graphPg.counts).every(count=>count===0)) {
      graph.disposition='NO_DATA_IN_CONFIGURED_STORE';
    }
  }
  const blockers = (plan.blockers ?? []).map(item => ({ code: blockerCode(item), detail: item, nextStep: nextStep(blockerCode(item)) }));
  return {
    schema: 'lattice.project-purge.report.v1', projectId: plan.projectId, operationId: plan.operationId,
    planDigest: plan.digest, coreStatus,
    overallStatus: coreStatus === 'BLOCKED' ? 'BLOCKED' : scopedComplete ? 'PARTIAL' : coreStatus === 'READY' ? 'AWAITING_CONFIRMATION' : 'INCOMPLETE',
    complete: false,
    latticeScopeComplete: false,
    latticeScopeStatus: 'INCOMPLETE',
    stages: [
      { kind: 'postgres', counts: plan.postgres.counts, status: result?.postgres?.status === 'PURGED' ? 'VERIFIED_ERASED' : 'NOT_VERIFIED' },
      { kind: 'sqlite', counts: plan.sqlite.counts, strategy: plan.sqlite.strategy ?? 'IN_PLACE_V1',
        decisionOwnership: plan.sqlite.retainedDecisionRows === 0 ? 'NO_DECISIONS_PRESENT' : 'LEGACY_SCOPE_OWNERSHIP_NOT_PROVEN',
        ...(plan.sqlite.strategy === 'REBUILD_SURVIVORS_V1' ? {
          requiredAccessCheck: 'WINDOWS_OWNER_GROUP_DACL_EXACT_MATCH', saclAudit: 'NOT_VERIFIED',
          handleExclusion: 'REQUIRES_OFFLINE_MAINTENANCE', powerLossRecoveryGuaranteed: false,
        } : {}),
        initialCatalog: plan.sqlite.catalogState ?? 'PRESENT', status: result?.sqlite?.complete ? 'VERIFIED_ERASED' : 'NOT_VERIFIED' },
      { kind: 'files', roots: plan.files.roots, entries: plan.files.entries?.length ?? null,
        status: result?.files?.complete || result?.files?.readback?.complete ? 'VERIFIED_ABSENT' : 'NOT_VERIFIED' },
      ...(plan.codeGraph ? [{ kind: 'controlCodeGraph', cacheDirectory: plan.codeGraph.cacheDirectory,
        roots: plan.codeGraph.roots, discovery: plan.codeGraph.discovery,
        status: result?.codeGraph?.complete === true ? 'VERIFIED_ABSENT' : 'NOT_VERIFIED' }] : []),
      ...(plan.runtimeGraph?.discovery==='COMPLETE'?[{kind:'runtimeGraphFiles',roots:plan.runtimeGraph.roots,
        status:result?.runtimeGraph?.complete===true?'VERIFIED_ABSENT':'NOT_VERIFIED'}]:[]),
    ],
    blockers, external,
    history: plan.postgres.history ?? null,
    remaining: [
      ...(plan.sqlite.retainedDecisionRows > 0 ? [{ kind: 'controlDecisions', reason: 'FREEFORM_SCOPE_HAS_NO_STRUCTURAL_PROJECT_OWNERSHIP' }] : []),
      ...external.filter(item => item.disposition !== 'NO_DATA_IN_CONFIGURED_STORE').map(item => ({ kind: item.kind, ownership: item.ownership,
        reason: item.ownership === 'LATTICE' ? 'LATTICE_OWNED_CLEANUP_NOT_IMPLEMENTED' : 'DISCOVERY_AND_ERASURE_NOT_VERIFIED',
        nextStep: item.nextStep })),
    ],
    explanation: scopedComplete
      ? 'The implemented local scope is erased. Additional LATTICE-owned data and external resources remain unresolved; neither complete LATTICE cleanup nor full project erasure is verified.'
      : 'Use this operation and its exact plan for continuation. Catalog absence, archive, and process exit are not proof of erasure.',
  };
}
