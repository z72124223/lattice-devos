// One scope report for every LATTICE project. External discovery is deliberately
// distinct from erasure: an empty or caller-supplied inventory cannot prove absence.
const externalKinds = Object.freeze({
  codex: 'Codex conversations and attachments',
  automations: 'Native schedules and pending runs',
  git: 'Shared Git metadata and remote repositories',
  graphify: 'Graphify derived indexes and caches',
  botLifecycle: 'Separate Bot lifecycle records',
  backups: 'Backups, database WAL and external copies',
  maintenance: 'Purge plans, progress and maintenance receipts',
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
    kind, description, discovery: 'NOT_VERIFIED', resources: resources.filter(item => item.kind === kind),
  }));
}

function blockerCode(blocker) { return typeof blocker === 'string' ? blocker : blocker?.code ?? 'UNKNOWN_BLOCKER'; }
function nextStep(code) {
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
  const blockers = (plan.blockers ?? []).map(item => ({ code: blockerCode(item), detail: item, nextStep: nextStep(blockerCode(item)) }));
  return {
    schema: 'lattice.project-purge.report.v1', projectId: plan.projectId, operationId: plan.operationId,
    planDigest: plan.digest, coreStatus,
    overallStatus: coreStatus === 'BLOCKED' ? 'BLOCKED' : scopedComplete ? 'PARTIAL' : coreStatus === 'READY' ? 'AWAITING_CONFIRMATION' : 'INCOMPLETE',
    complete: false,
    stages: [
      { kind: 'postgres', counts: plan.postgres.counts, status: result?.postgres?.status === 'PURGED' ? 'VERIFIED_ERASED' : 'NOT_VERIFIED' },
      { kind: 'sqlite', counts: plan.sqlite.counts, initialCatalog: plan.sqlite.catalogState ?? 'PRESENT', status: result?.sqlite?.complete ? 'VERIFIED_ERASED' : 'NOT_VERIFIED' },
      { kind: 'files', roots: plan.files.roots, entries: plan.files.entries?.length ?? null,
        status: result?.files?.complete || result?.files?.readback?.complete ? 'VERIFIED_ABSENT' : 'NOT_VERIFIED' },
    ],
    blockers, external,
    remaining: external.map(item => ({ kind: item.kind, reason: 'EXTERNAL_DISCOVERY_AND_ERASURE_NOT_VERIFIED' })),
    explanation: scopedComplete
      ? 'The verified local scope is erased. External resources remain unverified; this is not complete project erasure.'
      : 'Use this operation and its exact plan for continuation. Catalog absence, archive, and process exit are not proof of erasure.',
  };
}
